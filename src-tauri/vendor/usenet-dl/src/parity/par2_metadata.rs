//! Pure Rust parser for PAR2 file metadata (File Description packets).
//!
//! Extracts filenames and their 16KB MD5 hashes from PAR2 files, enabling
//! DirectRename to match obfuscated files to their real names.
//!
//! ## PAR2 Packet Structure
//!
//! Every PAR2 packet starts with:
//! - 8 bytes: magic "PAR2\0PKT"
//! - 8 bytes: packet length (little-endian u64, includes header)
//! - 16 bytes: packet hash (MD5 of body)
//! - 16 bytes: recovery set ID
//! - 16 bytes: packet type signature
//! - variable: packet body
//!
//! File Description packets (type `PAR 2.0\0FileDesc`) contain:
//! - 16 bytes: file ID
//! - 16 bytes: MD5 hash of entire file
//! - 16 bytes: MD5 hash of first 16KB of file
//! - 8 bytes: file length (little-endian u64)
//! - variable: filename (UTF-8, null-padded to 4-byte boundary)

use std::path::Path;

/// A file entry parsed from a PAR2 File Description packet.
#[derive(Debug, Clone)]
pub struct Par2FileEntry {
    /// The real filename from the PAR2 metadata
    pub filename: String,
    /// MD5 hash of the first 16KB of the file
    pub hash_16k: [u8; 16],
}

/// PAR2 packet header magic bytes
const PAR2_MAGIC: &[u8; 8] = b"PAR2\0PKT";

/// Packet type signature for File Description packets
const FILE_DESC_TYPE: &[u8; 16] = b"PAR 2.0\0FileDesc";

/// Size of the fixed packet header (magic + length + hash + set_id + type)
const HEADER_SIZE: usize = 8 + 8 + 16 + 16 + 16; // 64 bytes

/// Offset of the packet type field within the header
const TYPE_OFFSET: usize = 8 + 8 + 16 + 16; // 48 bytes

/// Size of the File Description packet body before the filename
/// (file_id + md5_full + md5_16k + file_length)
const FILE_DESC_FIXED_BODY: usize = 16 + 16 + 16 + 8; // 56 bytes

/// Offset of the MD5-16K hash within the File Description body
const MD5_16K_OFFSET: usize = 16 + 16; // 32 bytes (after file_id + md5_full)

/// Parse all File Description packets from a PAR2 file.
///
/// Returns a list of file entries with their filenames and 16KB MD5 hashes.
/// Returns an empty vec (not an error) if the file contains no File Description packets.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn parse_par2_file_entries(par2_path: &Path) -> crate::Result<Vec<Par2FileEntry>> {
    let data = std::fs::read(par2_path)?;
    Ok(parse_par2_file_entries_from_bytes(&data))
}

/// Parse File Description packets from raw PAR2 bytes.
///
/// This is the core parsing function, separated for testability.
pub(crate) fn parse_par2_file_entries_from_bytes(data: &[u8]) -> Vec<Par2FileEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;

    while pos + HEADER_SIZE <= data.len() {
        // Find next PAR2 magic
        match find_magic(data, pos) {
            Some(magic_pos) => pos = magic_pos,
            None => break,
        }

        // Need at least the header to read packet length
        if pos + HEADER_SIZE > data.len() {
            break;
        }

        // Read packet length (total packet size including header)
        let packet_len =
            u64::from_le_bytes(data[pos + 8..pos + 16].try_into().unwrap_or([0; 8])) as usize;

        // Sanity check: packet must be at least header size and not exceed remaining data
        if packet_len < HEADER_SIZE || pos + packet_len > data.len() {
            pos += 8; // Skip past this magic and try to find another
            continue;
        }

        // Check if this is a File Description packet
        let type_sig = &data[pos + TYPE_OFFSET..pos + TYPE_OFFSET + 16];
        if type_sig == FILE_DESC_TYPE {
            let body_start = pos + HEADER_SIZE;
            let body_len = packet_len - HEADER_SIZE;

            if body_len >= FILE_DESC_FIXED_BODY {
                // Extract MD5 of first 16KB
                let md5_start = body_start + MD5_16K_OFFSET;
                let mut hash_16k = [0u8; 16];
                hash_16k.copy_from_slice(&data[md5_start..md5_start + 16]);

                // Extract filename (after the fixed fields, null-terminated/padded)
                let name_start = body_start + FILE_DESC_FIXED_BODY;
                let name_end = pos + packet_len;
                if name_start < name_end {
                    let name_bytes = &data[name_start..name_end];
                    // Filename is null-padded to 4-byte boundary
                    let filename = extract_filename(name_bytes);
                    if !filename.is_empty() {
                        entries.push(Par2FileEntry { filename, hash_16k });
                    }
                }
            }
        }

        // Advance to next packet
        pos += packet_len;
    }

    entries
}

/// Find the next occurrence of PAR2 magic bytes starting from `pos`.
fn find_magic(data: &[u8], start: usize) -> Option<usize> {
    if start + PAR2_MAGIC.len() > data.len() {
        return None;
    }
    data[start..]
        .windows(PAR2_MAGIC.len())
        .position(|w| w == PAR2_MAGIC)
        .map(|offset| start + offset)
}

/// Extract a filename from null-padded bytes.
fn extract_filename(bytes: &[u8]) -> String {
    // Find the first null byte (filename is null-terminated, then padded)
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Compute the MD5 hash of the first 16KB of a file.
///
/// Used by DirectRename to match completed files against PAR2 metadata.
///
/// # Errors
///
/// Returns an error if the file cannot be read.
pub fn compute_16k_md5(file_path: &Path) -> crate::Result<[u8; 16]> {
    use std::io::Read;

    let mut file = std::fs::File::open(file_path)?;
    let mut buffer = [0u8; 16384]; // 16KB
    let bytes_read = file.read(&mut buffer)?;

    let digest = md5::compute(&buffer[..bytes_read]);
    Ok(digest.0)
}

// unwrap/expect are acceptable in tests for concise failure-on-error assertions
#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal PAR2 File Description packet for testing.
    fn build_file_desc_packet(filename: &str, hash_16k: [u8; 16]) -> Vec<u8> {
        // Pad filename to 4-byte boundary
        let name_bytes = filename.as_bytes();
        let padded_len = (name_bytes.len() + 3) & !3; // Round up to 4-byte boundary
        let mut padded_name = vec![0u8; padded_len];
        padded_name[..name_bytes.len()].copy_from_slice(name_bytes);

        let body_len = FILE_DESC_FIXED_BODY + padded_len;
        let packet_len = (HEADER_SIZE + body_len) as u64;

        let mut packet = Vec::with_capacity(packet_len as usize);

        // Magic
        packet.extend_from_slice(PAR2_MAGIC);
        // Packet length
        packet.extend_from_slice(&packet_len.to_le_bytes());
        // Packet hash (16 bytes, zeroed for test)
        packet.extend_from_slice(&[0u8; 16]);
        // Recovery set ID (16 bytes, zeroed for test)
        packet.extend_from_slice(&[0u8; 16]);
        // Packet type
        packet.extend_from_slice(FILE_DESC_TYPE);

        // Body: file_id (16 bytes)
        packet.extend_from_slice(&[0u8; 16]);
        // Body: md5_full (16 bytes)
        packet.extend_from_slice(&[0u8; 16]);
        // Body: md5_16k (16 bytes)
        packet.extend_from_slice(&hash_16k);
        // Body: file_length (8 bytes)
        packet.extend_from_slice(&1024u64.to_le_bytes());
        // Body: filename (padded)
        packet.extend_from_slice(&padded_name);

        packet
    }

    #[test]
    fn parse_single_file_desc_packet() {
        let hash = [1u8; 16];
        let data = build_file_desc_packet("movie.mkv", hash);

        let entries = parse_par2_file_entries_from_bytes(&data);

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "movie.mkv");
        assert_eq!(entries[0].hash_16k, hash);
    }

    #[test]
    fn parse_multiple_file_desc_packets() {
        let hash1 = [1u8; 16];
        let hash2 = [2u8; 16];

        let mut data = build_file_desc_packet("file1.rar", hash1);
        data.extend_from_slice(&build_file_desc_packet("file2.rar", hash2));

        let entries = parse_par2_file_entries_from_bytes(&data);

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].filename, "file1.rar");
        assert_eq!(entries[0].hash_16k, hash1);
        assert_eq!(entries[1].filename, "file2.rar");
        assert_eq!(entries[1].hash_16k, hash2);
    }

    #[test]
    fn parse_empty_data_returns_no_entries() {
        let entries = parse_par2_file_entries_from_bytes(&[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_garbage_data_returns_no_entries() {
        let garbage = vec![0xFFu8; 1024];
        let entries = parse_par2_file_entries_from_bytes(&garbage);
        assert!(entries.is_empty());
    }

    #[test]
    fn parse_truncated_packet_returns_no_entries() {
        let full = build_file_desc_packet("test.bin", [3u8; 16]);
        // Truncate to just the header
        let truncated = &full[..HEADER_SIZE];
        let entries = parse_par2_file_entries_from_bytes(truncated);
        assert!(entries.is_empty());
    }

    #[test]
    fn extract_filename_handles_null_padding() {
        let bytes = b"hello.txt\0\0\0"; // padded to 12 bytes
        assert_eq!(extract_filename(bytes), "hello.txt");
    }

    #[test]
    fn extract_filename_handles_no_null() {
        let bytes = b"hello.txt";
        assert_eq!(extract_filename(bytes), "hello.txt");
    }

    #[test]
    fn non_file_desc_packets_are_skipped() {
        let mut data = Vec::new();

        // Build a non-FileDesc packet (e.g., Main packet)
        const BODY_LEN: usize = 16; // minimal body
        let packet_len = (HEADER_SIZE + BODY_LEN) as u64;
        data.extend_from_slice(PAR2_MAGIC);
        data.extend_from_slice(&packet_len.to_le_bytes());
        data.extend_from_slice(&[0u8; 16]); // hash
        data.extend_from_slice(&[0u8; 16]); // set_id
        data.extend_from_slice(b"PAR 2.0\0Main\0\0\0\0"); // different type
        data.extend_from_slice(&[0u8; BODY_LEN]);

        // Then a real FileDesc packet
        data.extend_from_slice(&build_file_desc_packet("real.rar", [5u8; 16]));

        let entries = parse_par2_file_entries_from_bytes(&data);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].filename, "real.rar");
    }
}

//! RAR volume naming conventions and first-volume detection.
//!
//! DirectUnpack only attempts extraction from the first RAR volume, since `unrar`
//! and other extractors automatically process subsequent volumes from there.

/// Check if a filename is the first volume of a RAR archive set.
///
/// Recognizes these naming conventions:
/// - `archive.rar` (single file or first volume of old-style naming)
/// - `archive.part01.rar`, `archive.part001.rar` (new-style multi-volume)
/// - `archive.r00` (old-style split: .r00, .r01, ... but .rar is the first)
///
/// Returns `false` for non-first volumes like `.r01`, `.part02.rar`, etc.
pub(crate) fn is_first_rar_volume(filename: &str) -> bool {
    let lower = filename.to_lowercase();

    // New-style: .part01.rar, .part001.rar, .part0001.rar
    if lower.ends_with(".rar") {
        // Check for .partNNN.rar pattern
        if let Some(stem) = lower.strip_suffix(".rar")
            && let Some(part_idx) = stem.rfind(".part")
        {
            let num_str = &stem[part_idx + 5..]; // after ".part"
            if !num_str.is_empty() && num_str.chars().all(|c| c.is_ascii_digit()) {
                // It's a .partNNN.rar file — first volume has part number 1 or 01 or 001
                let num: u32 = num_str.parse().unwrap_or(0);
                return num == 1;
            }
        }
        // Plain .rar with no .partNNN — this IS the first volume
        return true;
    }

    // Old-style: .r00 is NOT the first volume — .rar is.
    // So .r00, .r01, etc. are never first volumes.
    false
}

/// Check if a filename is any part of a RAR archive (first or subsequent volume).
#[allow(dead_code)]
pub(crate) fn is_rar_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();

    // .rar, .partNNN.rar
    if lower.ends_with(".rar") {
        return true;
    }

    // Old-style split: .r00, .r01, .r02, ...
    if lower.len() >= 4 {
        let ext = &lower[lower.len() - 4..];
        if ext.starts_with(".r") && ext[2..].chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
    }

    false
}

/// Check if a filename is a PAR2 file.
pub(crate) fn is_par2_file(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    lower.ends_with(".par2")
}

#[allow(clippy::unwrap_used, clippy::expect_used)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_rar_is_first_volume() {
        assert!(is_first_rar_volume("movie.rar"));
        assert!(is_first_rar_volume("Movie.RAR"));
        assert!(is_first_rar_volume("some.file.name.rar"));
    }

    #[test]
    fn part01_rar_is_first_volume() {
        assert!(is_first_rar_volume("movie.part01.rar"));
        assert!(is_first_rar_volume("movie.part001.rar"));
        assert!(is_first_rar_volume("movie.part0001.rar"));
        assert!(is_first_rar_volume("movie.part1.rar"));
        assert!(is_first_rar_volume("Movie.Part01.RAR"));
    }

    #[test]
    fn part02_rar_is_not_first_volume() {
        assert!(!is_first_rar_volume("movie.part02.rar"));
        assert!(!is_first_rar_volume("movie.part002.rar"));
        assert!(!is_first_rar_volume("movie.part10.rar"));
        assert!(!is_first_rar_volume("movie.part2.rar"));
    }

    #[test]
    fn old_style_split_is_not_first_volume() {
        assert!(!is_first_rar_volume("movie.r00"));
        assert!(!is_first_rar_volume("movie.r01"));
        assert!(!is_first_rar_volume("movie.r99"));
    }

    #[test]
    fn non_rar_files_are_not_first_volume() {
        assert!(!is_first_rar_volume("movie.mkv"));
        assert!(!is_first_rar_volume("movie.par2"));
        assert!(!is_first_rar_volume("movie.zip"));
        assert!(!is_first_rar_volume("movie.7z"));
    }

    #[test]
    fn is_rar_file_detects_all_rar_extensions() {
        assert!(is_rar_file("movie.rar"));
        assert!(is_rar_file("movie.part01.rar"));
        assert!(is_rar_file("movie.r00"));
        assert!(is_rar_file("movie.r01"));
        assert!(is_rar_file("movie.r99"));
        assert!(is_rar_file("Movie.RAR"));
    }

    #[test]
    fn is_rar_file_rejects_non_rar() {
        assert!(!is_rar_file("movie.mkv"));
        assert!(!is_rar_file("movie.par2"));
        assert!(!is_rar_file("movie.zip"));
    }

    #[test]
    fn is_par2_file_works() {
        assert!(is_par2_file("movie.par2"));
        assert!(is_par2_file("movie.vol00+01.PAR2"));
        assert!(!is_par2_file("movie.rar"));
        assert!(!is_par2_file("movie.par"));
    }
}

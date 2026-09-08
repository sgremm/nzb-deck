//! PAR2 parity file handling
//!
//! This module provides a trait-based architecture for handling PAR2 verification
//! and repair operations. It supports both CLI-based implementations (using external
//! par2 binaries) and stub implementations for graceful degradation when PAR2
//! support is unavailable.
//!
//! ## Architecture
//!
//! The core abstraction is the [`ParityHandler`] trait, which defines the interface
//! for PAR2 operations. Multiple implementations are provided:
//!
//! - [`CliParityHandler`]: Uses external `par2` binary for full functionality
//! - [`NoOpParityHandler`]: Stub implementation when PAR2 is unavailable
//!
//! ## Usage
//!
//! ```no_run
//! use usenet_dl::parity::{CliParityHandler, ParityHandler};
//! use std::path::Path;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     // Try to find par2 in PATH
//!     let handler = CliParityHandler::from_path()
//!         .expect("par2 binary not found");
//!
//!     // Verify files
//!     let result = handler.verify(Path::new("download.par2")).await?;
//!     if !result.is_complete {
//!         println!("Found {} damaged blocks", result.damaged_blocks);
//!
//!         // Attempt repair
//!         let repair = handler.repair(Path::new("download.par2")).await?;
//!         if repair.success {
//!             println!("Repaired: {:?}", repair.repaired_files);
//!         }
//!     }
//!
//!     Ok(())
//! }
//! ```

mod cli;
mod noop;
pub(crate) mod par2_metadata;
mod parser;
mod traits;

pub use cli::CliParityHandler;
pub use noop::NoOpParityHandler;
pub use par2_metadata::{Par2FileEntry, compute_16k_md5, parse_par2_file_entries};
pub use traits::{ParityCapabilities, ParityHandler, RepairResult, VerifyResult};

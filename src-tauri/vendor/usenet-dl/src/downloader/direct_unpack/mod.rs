//! DirectUnpack — extract archives while download is still in progress.
//!
//! This module contains:
//! - [`DirectUnpackCoordinator`]: Background task that polls for completed files
//!   and dispatches extraction
//! - [`DirectRenameState`]: Uses PAR2 metadata to fix obfuscated filenames mid-download
//! - RAR volume detection helpers for identifying first volumes

pub(crate) mod coordinator;
pub(crate) mod rar_detection;
pub(crate) mod rename;

pub(crate) use coordinator::{DirectUnpackCoordinator, state as direct_unpack_state};

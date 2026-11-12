//! dng-sdk — pure Rust port of the Adobe DNG SDK 1.7.1.
//!
//! Module `dng_foo` is a faithful port of `dng_foo.cpp/.h` from
//! `vendor/extracted/dng_sdk_1_7_1/dng_sdk/source/`.  Naming conventions,
//! error model and porting rules are defined in the workspace
//! `ARCHITECTURE.md`; deviations are recorded in `PORTING_NOTES.md`.

// Constants keep their C++ spelling (tcNewSubFileType, ccLossyJPEG, ptRGB, ...).
#![allow(non_upper_case_globals)]

// ---------------------------------------------------------------------------
// Agent B modules (phase 2 batch 1) — see NEGATIVE_API.md ownership table.
// Agent A modules are appended as they land (integration phase assembles the
// final re-export list).
// ---------------------------------------------------------------------------

pub mod dng_bmff;
pub mod dng_big_table;
pub mod dng_jpeg_image;
pub mod dng_lossless_jpeg;
pub mod dng_lossless_jpeg_shared;
pub mod dng_preview;

pub use dng_core::{DngError, DngResult};

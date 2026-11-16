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

// ---------------------------------------------------------------------------
// Agent A modules (phase 2) — metadata / negative side.
// ---------------------------------------------------------------------------

pub mod dng_exif;
pub mod dng_iptc;
pub mod dng_xmp_sdk;
pub mod dng_xmp;
pub mod dng_hue_sat_map;
pub mod dng_tone_curve;
pub mod dng_gain_map;
pub mod dng_camera_profile;
pub mod dng_color_spec;

pub use dng_core::{DngError, DngResult};

// ---------------------------------------------------------------------------
// Agent B modules (phase 2 batch 2) — read_image / update_meta / writer /
// render.  Appended by Agent B; see crates/dng-sdk/INTEGRATION_NOTES.md.
// ---------------------------------------------------------------------------

pub mod dng_read_image;
pub mod dng_update_meta;

// Agent A's negative/opcode/info modules (landed after B's batch-2 started;
// B declared dng_opcode_list/dng_opcodes/dng_misc_opcodes as build
// dependencies of dng_bad_pixels during the seam-stub window — see
// INTEGRATION_NOTES.md).
pub mod dng_negative;
pub mod dng_opcode_list;
pub mod dng_opcodes;
pub mod dng_misc_opcodes;
pub mod dng_noise_profile;
pub mod dng_linearization_info;
pub mod dng_mosaic_info;
pub mod dng_info;
pub mod dng_lens_correction;

pub mod dng_bad_pixels;
pub mod dng_render;
pub mod dng_image_writer;

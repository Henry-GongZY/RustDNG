//! dng-core — pure Rust port of the Adobe DNG SDK 1.7.1 foundation layer.
//!
//! Module `dng_foo` is a faithful port of `dng_foo.cpp/.h` from
//! `vendor/extracted/dng_sdk_1_7_1/dng_sdk/source/`.  Naming conventions,
//! error model and porting rules are defined in the workspace
//! `ARCHITECTURE.md`; deviations are recorded in `PORTING_NOTES.md`.

// Constants keep their C++ spelling (tcNewSubFileType, ccDeflate, ptRGB, ...).
#![allow(non_upper_case_globals)]
// C++-style type aliases: int32, uint32, real64, ...
#![allow(non_camel_case_types)]

pub mod dng_flags;
pub mod dng_types;
pub mod dng_errors;
pub mod dng_exceptions;
pub mod dng_memory;
pub mod dng_globals;
pub mod dng_safe_arithmetic;
pub mod dng_utils;
pub mod dng_point;
pub mod dng_rect;
pub mod dng_string;
pub mod dng_string_list;
pub mod dng_local_string;
pub mod dng_date_time;
pub mod dng_rational;
pub mod dng_tag_types;
pub mod dng_tag_values;
pub mod dng_tags;
pub mod dng_sdk_limits;
pub mod dng_stream;
pub mod dng_memory_stream;
pub mod dng_file_stream;
pub mod zlib;
pub mod dng_pixel_buffer;
pub mod dng_image;
pub mod dng_simple_image;
pub mod dng_tile_iterator;
pub mod dng_area_task;
pub mod dng_filter_task;
pub mod dng_resample;
pub mod dng_bottlenecks;
pub mod dng_1d_function;
pub mod dng_1d_table;
pub mod dng_spline;
pub mod dng_matrix;
pub mod dng_xy_coord;
pub mod dng_temperature;
pub mod dng_color_space;
pub mod dng_color_space_profiles;
pub mod dng_orientation;
pub mod dng_fingerprint;
pub mod dng_abort_sniffer;
pub mod dng_host;
pub mod dng_tiff_directory;
pub mod dng_ifd;
pub mod dng_parse_utils;
pub mod dng_shared;

pub use dng_errors::{DngError, DngResult};

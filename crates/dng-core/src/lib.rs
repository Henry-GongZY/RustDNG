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

// ---------------------------------------------------------------------------
// Root re-exports: the commonly used foundation types, so downstream crates
// can write `dng_core::DngString` (the C++ treats these as root names too).
// ---------------------------------------------------------------------------

pub use dng_date_time::{
    DngDateTime, DngDateTimeFormat, DngDateTimeInfo, DngDateTimeStorageInfo, DngTimeZone,
};
pub use dng_exceptions::{
    bad_format, eof_failure, memory_full, not_yet_implemented, program_error, read_failure,
    user_cancel, write_failure,
};
pub use dng_fingerprint::{DngFingerprint, DngMd5Printer};
pub use dng_image::{DngImage, ImageRef};
pub use dng_matrix::{DngMatrix, DngMatrix3by3, DngMatrix4by3, DngMatrix4by4, DngVector};
pub use dng_memory::DngMemoryBlock;
pub use dng_memory_stream::DngMemoryStream;
pub use dng_parse_utils::*;
pub use dng_point::{DngPoint, DngPointReal64};
pub use dng_rational::{DngRational, DngSRational, DngURational};
pub use dng_rect::{DngRect, DngRectReal64};
pub use dng_sdk_limits::*;
pub use dng_shared::{DngCameraProfileDynamicRange, DngCameraProfileInfo, DngShared};
pub use dng_stream::{
    DngStream, StreamIO, kBigBufferSize, kDNGStreamInvalidOffset, kDefaultBufferSize,
    kSmallBufferSize,
};
pub use dng_string::{DngString, DngStringTable};
pub use dng_local_string::{DngLocalString};
pub use dng_string_list::DngStringList;
pub use dng_tag_types::*;
pub use dng_tag_values::*;
pub use dng_tags::*;
pub use dng_temperature::DngTemperature;
pub use dng_xy_coord::{
    calculate_triple_illuminant_weights, d50_xy_coord, d65_xy_coord, pcs_to_xy, pcs_to_xyz,
    xy_to_xyz, xyz_to_xy, DngIlluminantData, DngXyCoord,
};
pub use dng_matrix::{invert, invert_with_hint, dot as matrix_dot};
pub use dng_utils::{abs_real64, max_real64, min_real64, pin_real64, pin_real64_01, smooth_step};
pub use dng_types::*;
pub use dng_utils::{force_uppercase, pin_int32, pin_uint16, pin_uint32, round_int32_f64, round_uint32_f64, compute_buffer_size, PaddingType};
pub use dng_safe_arithmetic::convert_double_to_int32;

pub use dng_host::DngHost;
pub use dng_orientation::DngOrientation;
pub use dng_1d_function::Dng1dFunction;
pub use dng_1d_table::Dng1dTable;
pub use dng_resample::{
    DngResampleBicubic, DngResampleFunction, DngResampleWeights, DngResampleWeights2d,
    kResampleSubsampleBits2D, kResampleSubsampleCount2D, kResampleSubsampleMask2D,
};
pub use dng_utils::{
    abs_real32, lerp_real64, max_int32, max_real32, max_uint32, min_int32, min_real32, min_uint32,
    pin_real32, real64_to_fixed64, max_distance_point_to_rect, min_uint32_3, max_uint32_3,
};
pub use dng_rect::intersect_rect;
pub use dng_pixel_buffer::{DngPixelBuffer, PixelData};
pub use dng_safe_arithmetic::{
    convert_double_to_float, convert_uint32_to_int32, round_up_uint32_to_multiple,
    safe_int32_sub, safe_uint32_add, safe_uint32_divide_up, safe_uint32_mult2,
    safe_uint32_sub, safe_uint64_add,
};




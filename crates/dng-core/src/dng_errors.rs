// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_errors.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_errors.h — error code values.
//
// The C++ SDK passes `dng_error_code` (int32) inside a `dng_exception`.  In
// Rust, errors are the `DngError` enum below (see ARCHITECTURE.md §Errors)
// and successful fallible results use `DngResult<T>`.
//
// The enum mirrors the 1.7.1 `dng_error_*` codes 1:1.  Variant names follow
// the ARCHITECTURE.md contract (`ReadFailure` for dng_error_read_file,
// `WriteFailure` for dng_error_write_file, `ArithmeticOverflow` for
// dng_error_overflow, ...).  The contract additionally requires the
// compatibility variants NotThorough/NotSupported/Syntax/AlreadyExists/
// MissingSubImages/Unsupported(String); these have no numeric counterpart in
// the 1.7.1 header and are listed in PORTING_NOTES.md.

/// Type for all error codes used in the DNG SDK (mirrors `dng_error_code`).
pub type dng_error_code = i32;

pub const dng_error_none: dng_error_code = 0;
pub const dng_error_unknown: dng_error_code = 100000;
pub const dng_error_not_yet_implemented: dng_error_code = 100001;
pub const dng_error_silent: dng_error_code = 100002;
pub const dng_error_user_canceled: dng_error_code = 100003;
pub const dng_error_host_insufficient: dng_error_code = 100004;
pub const dng_error_memory: dng_error_code = 100005;
pub const dng_error_bad_format: dng_error_code = 100006;
pub const dng_error_matrix_math: dng_error_code = 100007;
pub const dng_error_open_file: dng_error_code = 100008;
pub const dng_error_read_file: dng_error_code = 100009;
pub const dng_error_write_file: dng_error_code = 100010;
pub const dng_error_end_of_file: dng_error_code = 100011;
pub const dng_error_file_is_damaged: dng_error_code = 100012;
pub const dng_error_image_too_big_dng: dng_error_code = 100013;
pub const dng_error_image_too_big_tiff: dng_error_code = 100014;
pub const dng_error_unsupported_dng: dng_error_code = 100015;
pub const dng_error_overflow: dng_error_code = 100016;
pub const dng_error_jxl_encoder: dng_error_code = 100017;
pub const dng_error_jxl_decoder: dng_error_code = 100018;

/// All errors produced by the DNG SDK.  Replaces C++ `dng_exception`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DngError {
    /// Logic or program error or other unclassifiable error.
    Unknown,
    /// Functionality requested is not yet implemented.
    NotYetImplemented,
    /// An error which should not be signalled to user.
    Silent,
    /// Processing stopped by user (or host application) request.
    UserCancel,
    /// Necessary host functionality is not present.
    HostInsufficient,
    /// Out of memory.
    Memory,
    /// File format is not valid.
    BadFormat,
    /// Matrix has wrong shape, is badly conditioned, or similar problem.
    MatrixMath,
    /// Could not open file.
    OpenFile,
    /// Error reading file (dng_error_read_file).
    ReadFailure,
    /// Error writing file (dng_error_write_file).
    WriteFailure,
    /// Unexpected end of file.
    EndOfFile,
    /// File is damaged in some way.
    FileIsDamaged,
    /// Image is too big to save as DNG.
    ImageTooBigDng,
    /// Image is too big to save as TIFF.
    ImageTooBigTiff,
    /// DNG version is unsupported.
    UnsupportedDng,
    /// Arithmetic overflow (dng_error_overflow).
    ArithmeticOverflow,
    /// JPEG XL encoder error.
    JxlEncoder,
    /// JPEG XL decoder error.
    JxlDecoder,

    // ----- compatibility variants required by ARCHITECTURE.md -------------
    /// Processing not "thorough" (deprecated legacy code; no 1.7.1 numeric code).
    NotThorough,
    /// Functionality not supported (used where C++ has no code path at all).
    NotSupported,
    /// Syntax error (legacy compatibility variant).
    Syntax,
    /// Object already exists (legacy compatibility variant).
    AlreadyExists,
    /// Negative is missing sub-images (legacy compatibility variant).
    MissingSubImages,
    /// Catch-all carrying a human readable detail message.
    Unsupported(String),
}

impl DngError {
    /// Numeric code mirroring dng_errors.h (dng_error_none for the
    /// compatibility variants that have no C++ 1.7.1 counterpart).
    pub fn error_code(&self) -> dng_error_code {
        match self {
            DngError::Unknown => dng_error_unknown,
            DngError::NotYetImplemented => dng_error_not_yet_implemented,
            DngError::Silent => dng_error_silent,
            DngError::UserCancel => dng_error_user_canceled,
            DngError::HostInsufficient => dng_error_host_insufficient,
            DngError::Memory => dng_error_memory,
            DngError::BadFormat => dng_error_bad_format,
            DngError::MatrixMath => dng_error_matrix_math,
            DngError::OpenFile => dng_error_open_file,
            DngError::ReadFailure => dng_error_read_file,
            DngError::WriteFailure => dng_error_write_file,
            DngError::EndOfFile => dng_error_end_of_file,
            DngError::FileIsDamaged => dng_error_file_is_damaged,
            DngError::ImageTooBigDng => dng_error_image_too_big_dng,
            DngError::ImageTooBigTiff => dng_error_image_too_big_tiff,
            DngError::UnsupportedDng => dng_error_unsupported_dng,
            DngError::ArithmeticOverflow => dng_error_overflow,
            DngError::JxlEncoder => dng_error_jxl_encoder,
            DngError::JxlDecoder => dng_error_jxl_decoder,
            // Compatibility variants have no 1.7.1 numeric code.
            _ => dng_error_none,
        }
    }

    /// True if this error should not be signalled to the user
    /// (C++ `dng_error_silent` / `dng_error_user_canceled`).
    pub fn is_silent(&self) -> bool {
        matches!(self, DngError::Silent | DngError::UserCancel)
    }
}

impl std::fmt::Display for DngError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DngError::{:?} ({})", self, self.error_code())
    }
}

impl std::error::Error for DngError {}

/// Result alias used by all fallible DNG SDK APIs.
pub type DngResult<T> = Result<T, DngError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_codes_match_cpp() {
        assert_eq!(DngError::Unknown.error_code(), 100000);
        assert_eq!(DngError::UserCancel.error_code(), 100003);
        assert_eq!(DngError::Memory.error_code(), 100005);
        assert_eq!(DngError::EndOfFile.error_code(), 100011);
        assert_eq!(DngError::ArithmeticOverflow.error_code(), 100016);
    }
}

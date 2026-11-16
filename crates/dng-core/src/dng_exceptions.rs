// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_exceptions.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_exceptions.h — C++ exception support replaced by DngError.
//
// Every C++ `ThrowXxx (...)` helper has a Rust twin `xxx_error()` returning
// `DngError` (instead of throwing) so call sites read
// `return Err(read_failure());`.  `fail_dng_error` mirrors `Fail_dng_error`.

use crate::dng_errors::{dng_error_code, dng_error_none, DngError, DngResult};

/// Display a warning message (stderr).  Mirrors `ReportWarning`.
pub fn report_warning(message: &str, sub_message: Option<&str>) {
    eprintln!("Warning: {}{}", message, sub_message.unwrap_or(""));
}

/// Display an error message (stderr).  Mirrors `ReportError`.
pub fn report_error(message: &str, sub_message: Option<&str>) {
    eprintln!("Error: {}{}", message, sub_message.unwrap_or(""));
}

/// Build the error for an arbitrary error code.  Mirrors `Throw_dng_error`.
/// The `silent` flag is advisory only in Rust (C++ uses it to decide whether
/// to report); the message arguments are carried in `DngError::Unsupported`
/// when non-null so no information is lost.
pub fn throw_dng_error(
    err: dng_error_code,
    message: Option<&str>,
    sub_message: Option<&str>,
    _silent: bool,
) -> DngError {
    let detail = match (message, sub_message) {
        (Some(m), Some(s)) => Some(format!("{}: {}", m, s)),
        (Some(m), None) => Some(m.to_string()),
        (None, Some(s)) => Some(s.to_string()),
        (None, None) => None,
    };

    match detail {
        Some(d) => DngError::Unsupported(format!("{:?}: {}", code_to_error(err), d)),
        None => code_to_error(err),
    }
}

fn code_to_error(err: dng_error_code) -> DngError {
    match err {
        crate::dng_errors::dng_error_unknown => DngError::Unknown,
        crate::dng_errors::dng_error_not_yet_implemented => DngError::NotYetImplemented,
        crate::dng_errors::dng_error_silent => DngError::Silent,
        crate::dng_errors::dng_error_user_canceled => DngError::UserCancel,
        crate::dng_errors::dng_error_host_insufficient => DngError::HostInsufficient,
        crate::dng_errors::dng_error_memory => DngError::Memory,
        crate::dng_errors::dng_error_bad_format => DngError::BadFormat,
        crate::dng_errors::dng_error_matrix_math => DngError::MatrixMath,
        crate::dng_errors::dng_error_open_file => DngError::OpenFile,
        crate::dng_errors::dng_error_read_file => DngError::ReadFailure,
        crate::dng_errors::dng_error_write_file => DngError::WriteFailure,
        crate::dng_errors::dng_error_end_of_file => DngError::EndOfFile,
        crate::dng_errors::dng_error_file_is_damaged => DngError::FileIsDamaged,
        crate::dng_errors::dng_error_image_too_big_dng => DngError::ImageTooBigDng,
        crate::dng_errors::dng_error_image_too_big_tiff => DngError::ImageTooBigTiff,
        crate::dng_errors::dng_error_unsupported_dng => DngError::UnsupportedDng,
        crate::dng_errors::dng_error_overflow => DngError::ArithmeticOverflow,
        crate::dng_errors::dng_error_jxl_encoder => DngError::JxlEncoder,
        crate::dng_errors::dng_error_jxl_decoder => DngError::JxlDecoder,
        _ => DngError::Unknown,
    }
}

/// Mirrors `Fail_dng_error`: turn a numeric error code into a Result.
pub fn fail_dng_error(err: dng_error_code) -> DngResult<()> {
    if err != dng_error_none {
        Err(throw_dng_error(err, None, None, false))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Convenience error constructors mirroring the C++ Throw* helpers.
// ---------------------------------------------------------------------------

/// dng_error_unknown ("program error").
pub fn program_error() -> DngError {
    DngError::Unknown
}

pub fn program_error_msg(sub_message: &str) -> DngError {
    DngError::Unsupported(sub_message.to_string())
}

/// dng_error_overflow.
pub fn overflow_error() -> DngError {
    DngError::ArithmeticOverflow
}

/// dng_error_not_yet_implemented.
pub fn not_yet_implemented() -> DngError {
    DngError::NotYetImplemented
}

/// dng_error_silent.
pub fn silent_error() -> DngError {
    DngError::Silent
}

/// dng_error_user_canceled.  C++ `ThrowUserCanceled` / abort sniffer
/// `ThrowUserCancel`.
pub fn user_cancel() -> DngError {
    DngError::UserCancel
}

/// dng_error_host_insufficient.
pub fn host_insufficient() -> DngError {
    DngError::HostInsufficient
}

/// dng_error_memory.
pub fn memory_full() -> DngError {
    DngError::Memory
}

/// dng_error_bad_format.
pub fn bad_format() -> DngError {
    DngError::BadFormat
}


/// dng_error_matrix_math.
pub fn matrix_math_error() -> DngError {
    DngError::MatrixMath
}

/// dng_error_open_file.
pub fn open_file_error() -> DngError {
    DngError::OpenFile
}

/// dng_error_read_file.  (ARCHITECTURE.md names this helper `read_failure`.)
pub fn read_failure() -> DngError {
    DngError::ReadFailure
}

/// dng_error_write_file.  (ARCHITECTURE.md names this helper `write_failure`.)
pub fn write_failure() -> DngError {
    DngError::WriteFailure
}

/// dng_error_end_of_file.
pub fn eof_failure() -> DngError {
    DngError::EndOfFile
}

/// dng_error_file_is_damaged.
pub fn file_is_damaged() -> DngError {
    DngError::FileIsDamaged
}

/// dng_error_image_too_big_dng.
pub fn image_too_big_dng() -> DngError {
    DngError::ImageTooBigDng
}

/// dng_error_image_too_big_tiff.
pub fn image_too_big_tiff() -> DngError {
    DngError::ImageTooBigTiff
}

/// dng_error_unsupported_dng.
pub fn unsupported_dng() -> DngError {
    DngError::UnsupportedDng
}

/// Compatibility helper required by ARCHITECTURE.md (features that are kept
/// as API but cannot be implemented here).
pub fn not_supported() -> DngError {
    DngError::NotSupported
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_map_to_right_codes() {
        assert_eq!(read_failure().error_code(), 100009);
        assert_eq!(write_failure().error_code(), 100010);
        assert_eq!(user_cancel().error_code(), 100003);
        assert_eq!(overflow_error().error_code(), 100016);
        assert!(user_cancel().is_silent());
    }
}

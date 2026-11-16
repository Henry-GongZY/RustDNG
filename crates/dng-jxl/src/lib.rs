// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1.
//
// Copyright 2005-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! `dng-jxl` — JPEG XL decode + encode for RustDNG.
//!
//! Replaces the DNG SDK's libjxl dependency (see `dng_jxl.cpp` /
//! `JXL_ReadMe.txt` in the SDK source):
//!
//! - **Decode**: wraps the pure-Rust `jxl-oxide` crate. Handles both the
//!   bare codestream (`0xFF 0x0A`) and the BMFF container form
//!   (`ftyp JXL ` + `jxlc`/`jxlp` + metadata boxes), integer 8/16-bit and
//!   float32 samples, gray/RGB(+alpha), ICC profiles and Exif/XMP boxes.
//! - **Encode**: a ground-up, hand-written pure-Rust JPEG XL **modular-mode
//!   lossless** encoder (no VarDCT). Output streams are validated by
//!   round-tripping through `jxl-oxide` in the test suite. The BMFF
//!   container form (with `Exif` / `xml ` / `colr` boxes) is emitted when
//!   the caller supplies Exif/XMP/ICC data, mirroring
//!   `dng_jxl.cpp EncodeJXL`.
//!
//! No C/C++, no `unsafe`, no `-sys` crates.

#![allow(non_upper_case_globals)]

pub mod container;
pub mod decode;
pub mod encode;

pub use decode::{decode_jxl, jxl_image_info, JxlDecodedImage, JxlImageInfo};
pub use encode::{encode_jxl, JxlEncodeOptions};

use std::fmt;

// ---------------------------------------------------------------------------
// Public image types (ARCHITECTURE.md codec contract)
// ---------------------------------------------------------------------------

/// An uncompressed image ready for JXL encoding.
///
/// Samples are interleaved, little-endian, row-major:
/// `data[(y * width + x) * channels + c]`.
///
/// Supported shapes: bits=8/16 integer (1, 3, 4 channels) and bits=32
/// float32 (1, 3, 4 channels).
#[derive(Debug, Clone, PartialEq)]
pub struct JxlImage {
    pub width: u32,
    pub height: u32,
    /// 1 = gray, 3 = RGB, 4 = RGBA.
    pub channels: u32,
    /// 8, 16 (integer) or 32 (float32).
    pub bits: u32,
    /// Interleaved little-endian samples.
    pub data: Vec<u8>,
}

impl JxlImage {
    /// Validate geometry and sample depth.
    pub(crate) fn validate(&self) -> Result<(), JxlError> {
        if self.width == 0 || self.height == 0 {
            return Err(JxlError::BadGeometry("zero-sized image".into()));
        }
        if self.width > (1 << 30) || self.height > (1 << 30) {
            return Err(JxlError::Unsupported("dimensions too large for JXL".into()));
        }
        match (self.bits, self.channels) {
            (8, 1) | (8, 3) | (8, 4) | (16, 1) | (16, 3) | (16, 4) | (32, 1) | (32, 3)
            | (32, 4) => {}
            (bits, channels) => {
                return Err(JxlError::Unsupported(format!(
                    "unsupported bits/channels combination: bits={bits} channels={channels}"
                )));
            }
        }
        let expected = self.expected_data_len()?;
        if self.data.len() < expected {
            return Err(JxlError::BadGeometry(format!(
                "data len {} < expected {expected}",
                self.data.len()
            )));
        }
        Ok(())
    }

    pub(crate) fn expected_data_len(&self) -> Result<usize, JxlError> {
        (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|n| n.checked_mul(self.channels as usize))
            .and_then(|n| n.checked_mul((self.bits / 8) as usize))
            .ok_or_else(|| JxlError::BadGeometry("dimensions overflow".into()))
    }

}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type for the dng-jxl facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JxlError {
    /// Input is not a valid/complete JPEG XL stream.
    BadFormat(String),
    /// Input ended prematurely (truncated stream).
    EndOfFile(String),
    /// A feature that is out of scope for this facade was requested.
    Unsupported(String),
    /// Encoding failed.
    Encode(String),
    /// Geometry mismatch.
    BadGeometry(String),
    /// Container/box structure error.
    Container(String),
}

impl fmt::Display for JxlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JxlError::BadFormat(s) => write!(f, "bad JXL format: {s}"),
            JxlError::EndOfFile(s) => write!(f, "unexpected end of JXL data: {s}"),
            JxlError::Unsupported(s) => write!(f, "unsupported JXL feature: {s}"),
            JxlError::Encode(s) => write!(f, "JXL encode error: {s}"),
            JxlError::BadGeometry(s) => write!(f, "bad JXL geometry: {s}"),
            JxlError::Container(s) => write!(f, "JXL container error: {s}"),
        }
    }
}

impl std::error::Error for JxlError {}



#[cfg(test)]
mod tests;

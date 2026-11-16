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
//! Decoder: wraps `jxl-oxide`, exposes the RustDNG codec contract.
//!
//! Box handling: jxl-oxide exposes Exif/XMP via its aux-box reader
//! (including brotli-compressed `brob` boxes); the `colr` ICC box and
//! fallback uncompressed box extraction are handled by the local container
//! parser in [`crate::container`].

use crate::container::{self, split_input};
use crate::JxlError;

/// Decoded JPEG XL image in native bit depth.
#[derive(Debug, Clone, PartialEq)]
pub struct JxlDecodedImage {
    pub width: u32,
    pub height: u32,
    /// 1 = gray, 2 = gray+alpha, 3 = RGB, 4 = RGBA.
    pub channels: u32,
    /// 8, 16 (integer) or 32 (float32).
    pub bits: u32,
    /// True when samples are float32.
    pub is_float: bool,
    /// Interleaved little-endian samples in native depth.
    pub data: Vec<u8>,
    /// ICC profile (embedded `colr` box, or synthesized when the codestream
    /// carries an ICC stream).
    pub icc: Option<Vec<u8>>,
    /// Raw `Exif` box content (leading 4-byte offset field included).
    pub exif: Option<Vec<u8>>,
    /// Raw `xml ` (XMP) box content.
    pub xmp: Option<Vec<u8>>,
}

/// Image metadata without decoding pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JxlImageInfo {
    pub width: u32,
    pub height: u32,
    pub bits_per_sample: u32,
    pub is_float: bool,
    /// Color channels + alpha (1..=4).
    pub channels: u32,
    pub alpha: bool,
    pub uses_original_profile: bool,
    /// Embedded ICC profile when present (from `colr` box or codestream).
    pub icc: Option<Vec<u8>>,
    /// Raw `Exif` box content.
    pub exif: Option<Vec<u8>>,
    /// Raw `xml ` box content.
    pub xmp: Option<Vec<u8>>,
}

/// Get image metadata (dimensions, bit depth, channels, boxes) without
/// decoding pixel data.
pub fn jxl_image_info(bytes: &[u8]) -> Result<JxlImageInfo, JxlError> {
    let input = split_input(bytes)?;
    let image = jxl_oxide::JxlImage::read_with_defaults(&input.codestream[..])
        .map_err(|e| JxlError::BadFormat(format!("JXL decode: {e}")))?;

    let header = image.image_header();
    let metadata = &header.metadata;

    let bits_per_sample = metadata.bit_depth.bits_per_sample();
    let is_float = matches!(
        metadata.bit_depth,
        jxl_oxide::image::BitDepth::FloatSample { .. }
    );

    let pixel_format = image.pixel_format();
    let alpha = matches!(
        pixel_format,
        jxl_oxide::PixelFormat::Graya
            | jxl_oxide::PixelFormat::Rgba
            | jxl_oxide::PixelFormat::Cmyka
    );
    let channels = pixel_format.channels() as u32;

    // ICC: prefer the container `colr` box; fall back to what jxl-oxide
    // embedded (want_icc codestream stream).
    let icc = input
        .icc
        .clone()
        .or_else(|| image.original_icc().map(|s| s.to_vec()));

    Ok(JxlImageInfo {
        width: image.width(),
        height: image.height(),
        bits_per_sample,
        is_float,
        channels,
        alpha,
        uses_original_profile: !metadata.xyb_encoded,
        icc,
        exif: exif_box(&image, &input),
        xmp: xmp_box(&image, &input),
    })
}

/// Decode a JPEG XL stream (bare codestream or container) into interleaved
/// native-depth samples plus metadata boxes.
pub fn decode_jxl(bytes: &[u8]) -> Result<JxlDecodedImage, JxlError> {
    let input = split_input(bytes)?;
    let image = jxl_oxide::JxlImage::read_with_defaults(&input.codestream[..])
        .map_err(|e| JxlError::BadFormat(format!("JXL decode: {e}")))?;

    let width = image.width();
    let height = image.height();
    let bits_per_sample = image.image_header().metadata.bit_depth.bits_per_sample();
    let is_float = matches!(
        image.image_header().metadata.bit_depth,
        jxl_oxide::image::BitDepth::FloatSample { .. }
    );

    // Decode the first (and only expected) keyframe.
    let render = image
        .render_frame(0)
        .map_err(|e| JxlError::BadFormat(format!("JXL render: {e}")))?;
    let fb = render.image_all_channels();

    let channels = fb.channels() as u32;
    if fb.width() != width as usize || fb.height() != height as usize {
        return Err(JxlError::BadFormat(format!(
            "rendered frame {}x{} != header {}x{}",
            fb.width(),
            fb.height(),
            width,
            height
        )));
    }

    // Convert f32 framebuffer to native depth.
    let count = width as usize * height as usize * channels as usize;
    let mut data = Vec::with_capacity(count * (bits_per_sample as usize / 8).max(1));
    let (min_val, max_val): (f32, f32) = if is_float {
        (f32::NEG_INFINITY, f32::INFINITY)
    } else {
        (0.0, 1.0)
    };
    for &v in fb.buf() {
        let v = if is_float {
            v
        } else {
            // Integer samples: clamp to [0,1] then scale to the full range.
            v.clamp(min_val, max_val)
        };
        if is_float {
            data.extend_from_slice(&v.to_le_bytes());
        } else if bits_per_sample <= 8 {
            let q = (v * 255.0).round().clamp(0.0, 255.0) as u8;
            data.push(q);
        } else if bits_per_sample <= 16 {
            let q = (v * 65535.0).round().clamp(0.0, 65535.0) as u16;
            data.extend_from_slice(&q.to_le_bytes());
        } else {
            // >16-bit integer samples: widen into f32 storage is not
            // representable; store as f32 is wrong, so error out.
            return Err(JxlError::Unsupported(format!(
                "integer bit depth {bits_per_sample} > 16 not supported by this facade"
            )));
        }
    }

    let icc = input
        .icc
        .clone()
        .or_else(|| image.original_icc().map(|s| s.to_vec()));

    Ok(JxlDecodedImage {
        width,
        height,
        channels,
        bits: if is_float { 32 } else { bits_per_sample },
        is_float,
        data,
        icc,
        exif: exif_box(&image, &input),
        xmp: xmp_box(&image, &input),
    })
}

fn exif_box(image: &jxl_oxide::JxlImage, input: &container::ParsedInput) -> Option<Vec<u8>> {
    // Prefer jxl-oxide (handles brob); fall back to the raw container box.
    // The aux-box payload excludes the 4-byte TIFF offset prefix; return
    // the full box content (offset + payload) like dng_jxl.cpp sees it.
    if let Ok(box_data) = image.aux_boxes().first_exif() {
        if box_data.has_data() {
            let exif = box_data.unwrap();
            let mut out = [0u8, 0, 0, 0].to_vec();
            out.extend_from_slice(exif.payload());
            return Some(out);
        }
    }
    input.exif.clone()
}

fn xmp_box(image: &jxl_oxide::JxlImage, input: &container::ParsedInput) -> Option<Vec<u8>> {
    let box_data = image.aux_boxes().first_xml();
    if box_data.has_data() {
        return Some(box_data.unwrap().to_vec());
    }
    input.xmp.clone()
}

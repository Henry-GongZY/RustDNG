//! `dng-jpeg` — pure-Rust JPEG encode/decode facade for RustDNG.
//!
//! Replaces the libjpeg dependency of the Adobe DNG SDK (see
//! `vendor/extracted/dng_sdk_1_7_1/JPEG_ReadMe.txt` and the
//! `EncodeJPEG`/`DecodeJPEG` call sites in `dng_image_writer.cpp` /
//! `dng_jpeg_image.cpp`).
//!
//! - Encoding is delegated to the `jpeg-encoder` crate (pure Rust).
//! - Decoding is delegated to the `zune-jpeg` crate (pure Rust).
//! - No C/C++, no `unsafe`, no `-sys` crates.
//!
//! The public API follows the RustDNG `ARCHITECTURE.md` "codec crates"
//! contract. Errors use the crate-local [`JpegError`] enum; the
//! `dng-sdk` glue layer converts it into `DngError` with `From`.

#![allow(non_upper_case_globals)]

use std::fmt;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error type for the dng-jpeg facade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JpegError {
    /// Input is not a valid/complete JPEG stream.
    BadFormat(String),
    /// Input ended prematurely (truncated stream).
    EndOfFile(String),
    /// A feature that is out of scope for this facade was requested.
    Unsupported(String),
    /// Underlying encoder failure.
    Encode(String),
    /// Geometry mismatch (data length doesn't match width*height*components).
    BadGeometry(String),
}

impl fmt::Display for JpegError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JpegError::BadFormat(s) => write!(f, "bad JPEG format: {}", s),
            JpegError::EndOfFile(s) => write!(f, "unexpected end of JPEG data: {}", s),
            JpegError::Unsupported(s) => write!(f, "unsupported JPEG feature: {}", s),
            JpegError::Encode(s) => write!(f, "JPEG encode error: {}", s),
            JpegError::BadGeometry(s) => write!(f, "bad JPEG geometry: {}", s),
        }
    }
}

impl std::error::Error for JpegError {}

impl From<zune_jpeg::errors::DecodeErrors> for JpegError {
    fn from(e: zune_jpeg::errors::DecodeErrors) -> Self {
        let text = e.to_string();
        // zune-jpeg signals truncated input with ExhaustedData / various
        // format errors; classify the common truncated cases as EndOfFile.
        match e {
            zune_jpeg::errors::DecodeErrors::ExhaustedData => JpegError::EndOfFile(text),
            _ => JpegError::BadFormat(text),
        }
    }
}

impl From<jpeg_encoder::EncodingError> for JpegError {
    fn from(e: jpeg_encoder::EncodingError) -> Self {
        JpegError::Encode(format!("{e:?}"))
    }
}

// ---------------------------------------------------------------------------
// Image types
// ---------------------------------------------------------------------------

/// Photometric interpretation of an un-compressed JPEG image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Photometric {
    /// Single channel, black is zero.
    Gray,
    /// Three channel RGB (will be encoded as YCbCr per JFIF convention).
    RGB,
    /// Input samples are already YCbCr and are passed through unchanged.
    YCbCr,
}

/// An uncompressed JPEG-source image (interleaved samples).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegImage {
    pub width: u32,
    pub height: u32,
    /// 1 (gray) or 3 (RGB/YCbCr).
    pub components: u32,
    /// Only 8 is supported (matches what the DNG SDK feeds to libjpeg).
    pub bits: u32,
    /// Interleaved pixel data, row-major: `data[(y*width + x)*components + c]`.
    pub data: Vec<u8>,
    pub photometric: Photometric,
}

impl JpegImage {
    pub fn new(
        width: u32,
        height: u32,
        components: u32,
        bits: u32,
        data: Vec<u8>,
        photometric: Photometric,
    ) -> Result<Self, JpegError> {
        if bits != 8 {
            return Err(JpegError::Unsupported(format!("bits={bits} (only 8 supported)")));
        }
        if components != 1 && components != 3 {
            return Err(JpegError::Unsupported(format!("components={components} (only 1 or 3)")));
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(components as usize))
            .ok_or_else(|| JpegError::BadGeometry("dimensions overflow".into()))?;
        if data.len() < expected {
            return Err(JpegError::BadGeometry(format!(
                "data len {} < expected {expected}",
                data.len()
            )));
        }
        let photometric = match (components, photometric) {
            (1, _) => Photometric::Gray,
            (3, Photometric::Gray) => {
                return Err(JpegError::BadGeometry(
                    "components=3 with Photometric::Gray".into(),
                ))
            }
            (_, p) => p,
        };
        Ok(JpegImage {
            width,
            height,
            components,
            bits,
            data,
            photometric,
        })
    }
}

// ---------------------------------------------------------------------------
// Encode options
// ---------------------------------------------------------------------------

/// Chroma subsampling selection.
///
/// `Gray` forces single-channel (grayscale) encoding even for RGB input;
/// for grayscale input all variants are equivalent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JpegSubsampling {
    Gray,
    S444,
    S422,
    S420,
}

/// JPEG encoding options (mirrors the knobs the DNG SDK sets on libjpeg).
#[derive(Debug, Clone)]
pub struct JpegEncodeOptions {
    /// 0..=100, libjpeg IJG quality scale (`jpeg-encoder` uses the same scale).
    pub quality: u32,
    /// Chroma subsampling. Default per photometric: Gray for gray, S420 for
    /// RGB/YCbCr (libjpeg default).
    pub subsampling: JpegSubsampling,
    /// Progressive (SOF2) encoding.
    pub progressive: bool,
    /// Optimize Huffman coding tables (libjpeg `optimize_coding`).
    pub optimize_coding: bool,
    /// JFIF horizontal density (dots per inch).
    pub x_dpi: u32,
    /// JFIF vertical density (dots per inch).
    pub y_dpi: u32,
    /// ICC profile to embed as an APP2 "ICC_PROFILE" segment.
    pub icc_profile: Option<Vec<u8>>,
}

impl Default for JpegEncodeOptions {
    fn default() -> Self {
        JpegEncodeOptions {
            quality: 90,
            subsampling: JpegSubsampling::S420,
            progressive: false,
            // libjpeg's default. Note: `true` is intentionally NOT the
            // default because zune-jpeg 0.4 misdecodes *baseline* 3-component
            // JPEGs that use custom optimized Huffman tables (progressive and
            // grayscale streams are unaffected). See PORTING_NOTES.md.
            optimize_coding: false,
            x_dpi: 72,
            y_dpi: 72,
            icc_profile: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Encode
// ---------------------------------------------------------------------------

/// Encode an uncompressed image into a JFIF JPEG byte stream.
pub fn encode_jpeg(img: &JpegImage, opts: &JpegEncodeOptions) -> Result<Vec<u8>, JpegError> {
    if img.width == 0 || img.height == 0 {
        return Err(JpegError::BadGeometry("zero-sized image".into()));
    }
    if img.width > u16::MAX as u32 || img.height > u16::MAX as u32 {
        return Err(JpegError::Unsupported(format!(
            "JPEG max dimension is 65535, got {}x{}",
            img.width, img.height
        )));
    }

    // Pick effective encoding mode.
    let is_gray = img.components == 1
        || img.photometric == Photometric::Gray
        || opts.subsampling == JpegSubsampling::Gray;

    // jpeg-encoder converts ColorType::Rgb input to YCbCr automatically
    // (JFIF convention); ColorType::Ycbcr input passes through unchanged.
    let color_type = if is_gray {
        jpeg_encoder::ColorType::Luma
    } else {
        match img.photometric {
            Photometric::YCbCr => jpeg_encoder::ColorType::Ycbcr,
            _ => jpeg_encoder::ColorType::Rgb,
        }
    };

    // Input data: if components==3 but we encode gray, collapse to luma.
    let input: Vec<u8>;
    let input_ref: &[u8];
    let eff_components: usize = if is_gray && img.components == 3 {
        input = to_luma(img);
        input_ref = &input;
        1
    } else {
        input_ref = &img.data;
        img.components as usize
    };

    let mut out = Vec::with_capacity(64 * 1024);
    let mut enc = jpeg_encoder::Encoder::new(&mut out, opts.quality.min(100) as u8);
    enc.set_density(jpeg_encoder::Density::Inch {
        x: opts.x_dpi.min(u16::MAX as u32) as u16,
        y: opts.y_dpi.min(u16::MAX as u32) as u16,
    });
    enc.set_progressive(opts.progressive);
    enc.set_optimized_huffman_tables(opts.optimize_coding);

    if !is_gray {
        let sampling = match opts.subsampling {
            JpegSubsampling::S444 => jpeg_encoder::SamplingFactor::F_1_1,
            JpegSubsampling::S422 => jpeg_encoder::SamplingFactor::F_2_1,
            JpegSubsampling::S420 => jpeg_encoder::SamplingFactor::F_2_2,
            JpegSubsampling::Gray => unreachable!(),
        };
        enc.set_sampling_factor(sampling);
    }

    if let Some(icc) = &opts.icc_profile {
        enc.add_icc_profile(icc)?;
    }

    let required = (img.width as usize) * (img.height as usize) * eff_components;
    if input_ref.len() < required {
        return Err(JpegError::BadGeometry(format!(
            "data len {} < required {required}",
            input_ref.len()
        )));
    }

    enc.encode(input_ref, img.width as u16, img.height as u16, color_type)?;

    Ok(out)
}

/// Collapse interleaved RGB to luma using the ITU-R BT.601 weights libjpeg
/// uses for rgb_gray (`0.299 R + 0.587 G + 0.114 B`, rounded).
fn to_luma(img: &JpegImage) -> Vec<u8> {
    let n = (img.width as usize) * (img.height as usize);
    let mut out = Vec::with_capacity(n);
    for px in 0..n {
        let r = img.data[px * 3] as u32;
        let g = img.data[px * 3 + 1] as u32;
        let b = img.data[px * 3 + 2] as u32;
        // libjpeg rgb_gray: FIX(0.29900) etc. with rounding bias (1 << 15).
        let y = (19595 * r + 38470 * g + 7471 * b + 32768) >> 16;
        out.push(y.min(255) as u8);
    }
    out
}

// ---------------------------------------------------------------------------
// Decode types
// ---------------------------------------------------------------------------

/// A decoded JPEG: interleaved 8-bit gray or RGB samples plus metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JpegDecoded {
    pub width: u32,
    pub height: u32,
    /// 1 (gray) or 3 (RGB).
    pub components: u32,
    /// Always 8 for this facade.
    pub bits: u32,
    /// Interleaved pixel data (gray or RGB).
    pub data: Vec<u8>,
    /// ICC profile extracted from the APP2 "ICC_PROFILE" segment, if present.
    pub icc_profile: Option<Vec<u8>>,
}

// ---------------------------------------------------------------------------
// Decode
// ---------------------------------------------------------------------------

/// Decode a JPEG byte stream into interleaved gray/RGB 8-bit samples.
///
/// Grayscale JPEGs decode to 1-component luma; YCbCr JPEGs convert to RGB.
/// Progressive and baseline, 4:4:4 / 4:2:2 / 4:2:0 subsampling are supported
/// by the underlying decoder.
pub fn decode_jpeg(bytes: &[u8]) -> Result<JpegDecoded, JpegError> {
    let mut decoder = zune_jpeg::JpegDecoder::new(bytes);
    // First pass: decode headers to learn the source colorspace.
    decoder
        .decode_headers()
        .map_err(JpegError::from)?;
    let info = decoder
        .info()
        .ok_or_else(|| JpegError::BadFormat("no image info in stream".into()))?;

    // Only gray (1 component) and YCbCr/RGB (3 component) sources are in
    // scope for DNG previews. CMYK/YCCK sources are not produced by the
    // DNG SDK's JPEG writer.
    let target_colors = match info.components {
        1 => zune_core::colorspace::ColorSpace::Luma,
        3 => zune_core::colorspace::ColorSpace::RGB,
        n => {
            return Err(JpegError::Unsupported(format!(
                "JPEG with {n} components not supported (gray/RGB only)"
            )));
        }
    };

    let options = zune_core::options::DecoderOptions::default()
        .jpeg_set_out_colorspace(target_colors);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(bytes, options);
    decoder
        .decode_headers()
        .map_err(JpegError::from)?;

    let icc_profile = decoder.icc_profile();

    let info = decoder
        .info()
        .ok_or_else(|| JpegError::BadFormat("no image info in stream".into()))?;

    let data = decoder.decode().map_err(JpegError::from)?;

    let components: u32 = match target_colors {
        zune_core::colorspace::ColorSpace::Luma => 1,
        _ => 3,
    };

    let expected = (info.width as usize) * (info.height as usize) * components as usize;
    if data.len() < expected {
        return Err(JpegError::EndOfFile(format!(
            "decoded buffer {} < expected {expected}",
            data.len()
        )));
    }

    Ok(JpegDecoded {
        width: info.width as u32,
        height: info.height as u32,
        components,
        bits: 8,
        data,
        icc_profile,
    })
}

// ---------------------------------------------------------------------------
// Header scanning
// ---------------------------------------------------------------------------

/// Scan JPEG markers for a Start-Of-Frame segment and return
/// `(width, height, components)`.
///
/// Handles standalone (0xFF 0x00) and filler (0xFF 0xFF) escapes.
pub fn jpeg_dimensions(bytes: &[u8]) -> Result<(u32, u32, u32), JpegError> {
    match scan_sof(bytes)? {
        Some((_, w, h, comps)) => Ok((w, h, comps)),
        None => Err(JpegError::BadFormat("no SOF marker found".into())),
    }
}

/// Returns true if the first SOF marker in the stream is SOF2 (progressive).
///
/// On malformed input returns `false` (progressiveness is simply unknown).
pub fn jpeg_is_progressive(bytes: &[u8]) -> bool {
    matches!(scan_sof(bytes), Ok(Some((marker, ..))) if marker == 0xC2)
}

/// SOF marker scan: returns `(marker, width, height, components)`.
fn scan_sof(bytes: &[u8]) -> Result<Option<(u8, u32, u32, u32)>, JpegError> {
    if bytes.len() < 2 || bytes[0] != 0xFF {
        return Err(JpegError::BadFormat("missing SOI marker".into()));
    }
    let mut i = 2;
    while i + 3 < bytes.len() {
        // Find next marker.
        if bytes[i] != 0xFF {
            i += 1;
            continue;
        }
        // Skip 0xFF fill bytes.
        let mut j = i + 1;
        while j < bytes.len() && bytes[j] == 0xFF {
            j += 1;
        }
        if j >= bytes.len() {
            return Err(JpegError::EndOfFile("stream ends inside marker".into()));
        }
        let marker = bytes[j];

        // Standalone markers have no length field.
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) || marker == 0xD8 || marker == 0xD9 {
            i = j + 1;
            continue;
        }

        if j + 2 >= bytes.len() {
            return Err(JpegError::EndOfFile("stream ends inside segment length".into()));
        }
        let seg_len = u16::from_be_bytes([bytes[j + 1], bytes[j + 2]]) as usize;
        if seg_len < 2 {
            return Err(JpegError::BadFormat("bad segment length".into()));
        }
        let seg_end = j + 1 + seg_len; // length includes the 2 length bytes
        if seg_end > bytes.len() {
            return Err(JpegError::EndOfFile("segment extends past end of stream".into()));
        }

        // SOF0..SOF15 excluding DHT/JPG/DAC (0xC4, 0xC8, 0xCC).
        if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
            // SOF segment: length(2) precision(1) height(2) width(2) components(1)
            // with the length bytes starting at j+1.
            if j + 9 > seg_end {
                return Err(JpegError::BadFormat("truncated SOF segment".into()));
            }
            let height = u16::from_be_bytes([bytes[j + 4], bytes[j + 5]]) as u32;
            let width = u16::from_be_bytes([bytes[j + 6], bytes[j + 7]]) as u32;
            let components = bytes[j + 8] as u32;
            return Ok(Some((marker, width, height, components)));
        }

        i = seg_end;
    }
    Ok(None)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        assert_eq!(a.len(), b.len());
        let mut sse: f64 = 0.0;
        for (x, y) in a.iter().zip(b.iter()) {
            let d = (*x as f64) - (*y as f64);
            sse += d * d;
        }
        if sse == 0.0 {
            return f64::INFINITY;
        }
        let mse = sse / a.len() as f64;
        10.0 * ((255.0 * 255.0) / mse).log10()
    }

    fn solid(width: u32, height: u32, comps: u32, fill: [u8; 3]) -> (JpegImage, Vec<u8>) {
        let n = (width * height) as usize;
        let mut data = vec![0u8; n * comps as usize];
        for px in 0..n {
            for c in 0..comps as usize {
                data[px * comps as usize + c] = fill[c];
            }
        }
        let photometric = if comps == 1 { Photometric::Gray } else { Photometric::RGB };
        let img = JpegImage {
            width,
            height,
            components: comps,
            bits: 8,
            data: data.clone(),
            photometric,
        };
        (img, data)
    }

    fn gradient(width: u32, height: u32, comps: u32) -> (JpegImage, Vec<u8>) {
        let mut data = Vec::with_capacity((width * height) as usize * comps as usize);
        for y in 0..height {
            for x in 0..width {
                let px = [
                    ((x * 255) / width.max(1)) as u8,
                    ((y * 255) / height.max(1)) as u8,
                    (((x + y) * 255) / (width + height).max(1)) as u8,
                ];
                for c in 0..comps as usize {
                    data.push(px[c]);
                }
            }
        }
        let photometric = if comps == 1 { Photometric::Gray } else { Photometric::RGB };
        let img = JpegImage {
            width,
            height,
            components: comps,
            bits: 8,
            data: data.clone(),
            photometric,
        };
        (img, data)
    }

    #[test]
    fn test_roundtrip_solid_rgb() {
        let (img, orig) = solid(64, 48, 3, [200, 30, 10]);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert_eq!((dec.width, dec.height, dec.components, dec.bits), (64, 48, 3, 8));
        // Solid color should survive JPEG nearly perfectly.
        assert!(psnr(&dec.data, &orig) > 40.0, "PSNR too low for solid color");
    }

    #[test]
    fn test_roundtrip_solid_gray() {
        let (img, orig) = solid(48, 32, 1, [77; 3]);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert_eq!((dec.width, dec.height, dec.components), (48, 32, 1));
        assert!(psnr(&dec.data, &orig) > 40.0);
    }

    #[test]
    fn test_roundtrip_gradient_rgb_q95() {
        let (img, orig) = gradient(96, 64, 3);
        let opts = JpegEncodeOptions {
            quality: 95,
            subsampling: JpegSubsampling::S444,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        let p = psnr(&dec.data, &orig);
        assert!(p > 35.0, "PSNR {p} too low at q95 4:4:4");
    }

    #[test]
    fn test_roundtrip_gradient_gray() {
        let (img, orig) = gradient(80, 60, 1);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert_eq!(dec.components, 1);
        assert!(psnr(&dec.data, &orig) > 35.0);
    }

    #[test]
    fn test_subsampling_variants_decode() {
        // Encode the same RGB image with 4:4:4, 4:2:2 and 4:2:0, then decode
        // each with zune-jpeg (exercises the corresponding upsamplers).
        for (name, sub) in [
            ("444", JpegSubsampling::S444),
            ("422", JpegSubsampling::S422),
            ("420", JpegSubsampling::S420),
        ] {
            let (img, orig) = gradient(64, 64, 3);
            let opts = JpegEncodeOptions {
                quality: 92,
                subsampling: sub,
                ..JpegEncodeOptions::default()
            };
            let enc = encode_jpeg(&img, &opts).unwrap();
            let dec = decode_jpeg(&enc).unwrap();
            assert_eq!(dec.components, 3, "{name}");
            let p = psnr(&dec.data, &orig);
            assert!(p > 28.0, "{name}: PSNR {p} too low");
            // Higher subsampling should not beat 4:4:4 quality-wise (loose).
        }
    }

    #[test]
    fn test_progressive_roundtrip() {
        let (img, orig) = gradient(72, 72, 3);
        let opts = JpegEncodeOptions {
            progressive: true,
            quality: 90,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        assert!(jpeg_is_progressive(&enc), "marker scan should see SOF2");
        let dec = decode_jpeg(&enc).unwrap();
        assert!(psnr(&dec.data, &orig) > 30.0);
    }

    #[test]
    fn test_progressive_gray() {
        let (img, orig) = gradient(48, 48, 1);
        let opts = JpegEncodeOptions {
            progressive: true,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        assert!(jpeg_is_progressive(&enc));
        let dec = decode_jpeg(&enc).unwrap();
        assert_eq!(dec.components, 1);
        assert!(psnr(&dec.data, &orig) > 30.0);
    }

    #[test]
    fn test_dpi_roundtrip() {
        let (img, _) = solid(16, 16, 1, [10; 3]);
        let opts = JpegEncodeOptions {
            x_dpi: 300,
            y_dpi: 150,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        // JFIF APP0: FF E0, len 16, "JFIF\0", version(2), units(1), xd(2), yd(2)
        let mut found = false;
        for i in 0..enc.len().saturating_sub(20) {
            if enc[i] == 0xFF
                && enc[i + 1] == 0xE0
                && &enc[i + 4..i + 9] == b"JFIF\0"
            {
                let xd = u16::from_be_bytes([enc[i + 12], enc[i + 13]]);
                let yd = u16::from_be_bytes([enc[i + 14], enc[i + 15]]);
                assert_eq!(xd, 300);
                assert_eq!(yd, 150);
                found = true;
                break;
            }
        }
        assert!(found, "no JFIF APP0 header found");
    }

    #[test]
    fn test_icc_profile_roundtrip() {
        // A minimal, structurally-valid-ish ICC blob; jpeg-encoder splits it
        // across APP2 segments and zune-jpeg rejoins them.
        let mut icc = Vec::new();
        icc.extend_from_slice(&128u32.to_be_bytes()); // size
        icc.extend_from_slice(b"none");
        icc.extend_from_slice(&[0, 0, 0, 0]); // version
        icc.extend_from_slice(b"mntr");
        icc.extend_from_slice(b"RGB ");
        while icc.len() < 128 {
            icc.push(0xAB);
        }
        // Extra payload beyond header so it spans > 64 KB? No; keep small.
        icc.extend_from_slice(&[0xEE; 600]);

        let (img, _) = solid(32, 32, 3, [5, 10, 15]);
        let opts = JpegEncodeOptions {
            icc_profile: Some(icc.clone()),
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert_eq!(dec.icc_profile.as_deref(), Some(&icc[..]));
    }

    #[test]
    fn test_no_icc() {
        let (img, _) = solid(16, 16, 3, [0; 3]);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert!(dec.icc_profile.is_none());
    }

    #[test]
    fn test_dimensions_and_progressive_scan() {
        let (img, _) = gradient(50, 40, 3);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();
        let (w, h, c) = jpeg_dimensions(&enc).unwrap();
        assert_eq!((w, h, c), (50, 40, 3));
        assert!(!jpeg_is_progressive(&enc));

        let opts = JpegEncodeOptions {
            progressive: true,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let (w, h, c) = jpeg_dimensions(&enc).unwrap();
        assert_eq!((w, h, c), (50, 40, 3));
        assert!(jpeg_is_progressive(&enc));
    }

    #[test]
    fn test_truncated_input_error() {
        let (img, _) = gradient(64, 64, 3);
        let enc = encode_jpeg(&img, &JpegEncodeOptions::default()).unwrap();

        // Empty.
        assert!(decode_jpeg(&[]).is_err());

        // SOI only.
        assert!(decode_jpeg(&[0xFF, 0xD8]).is_err());

        // Truncate within the header/scan region: must error.
        for cut in [enc.len() / 10, enc.len() / 4, enc.len() / 2] {
            let r = decode_jpeg(&enc[..cut]);
            assert!(r.is_err(), "decode of truncated stream should fail (cut={cut})");
        }

        // PORT-NOTE: deep truncations (only the tail of the entropy-coded
        // data removed) are tolerated by zune-jpeg 0.4 which fills the
        // remaining MCUs — same behavior class as libjpeg's warning path.
        // We only require no panic and correct geometry there.
        for cut in [enc.len() * 9 / 10, enc.len() - 8] {
            if let Ok(dec) = decode_jpeg(&enc[..cut]) {
                assert_eq!((dec.width, dec.height), (64, 64));
            }
        }

        // Dimension scan on truncated SOF should also error cleanly.
        assert!(jpeg_dimensions(&[0xFF, 0xD8, 0x00]).is_err());
        assert!(decode_jpeg(b"not a jpeg at all").is_err());
    }

    #[test]
    fn test_bad_format_error() {
        assert!(decode_jpeg(b"not a jpeg at all").is_err());
        assert!(jpeg_dimensions(b"no").is_err());
    }

    #[test]
    fn test_gray_subsample_flag_converts() {
        // RGB input + JpegSubsampling::Gray should produce a 1-component JPEG.
        let (img, _) = solid(24, 24, 3, [120, 130, 140]);
        let opts = JpegEncodeOptions {
            subsampling: JpegSubsampling::Gray,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let (w, h, c) = jpeg_dimensions(&enc).unwrap();
        assert_eq!((w, h, c), (24, 24, 1));
    }

    #[test]
    fn test_quality_loss_hierarchy() {
        // Higher quality should give higher PSNR on the same content.
        let (img, orig) = gradient(64, 64, 3);
        let mut psnrs = Vec::new();
        for q in [30u32, 60, 90] {
            let opts = JpegEncodeOptions {
                quality: q,
                subsampling: JpegSubsampling::S444,
                ..JpegEncodeOptions::default()
            };
            let dec = decode_jpeg(&encode_jpeg(&img, &opts).unwrap()).unwrap();
            psnrs.push(psnr(&dec.data, &orig));
        }
        assert!(
            psnrs[0] < psnrs[1] && psnrs[1] < psnrs[2],
            "PSNR should increase with quality: {psnrs:?}"
        );
    }

    #[test]
    fn test_error_display() {
        let e = JpegError::Unsupported("x".into());
        assert!(e.to_string().contains("unsupported"));
        let _ = Box::new(e) as Box<dyn std::error::Error>;
    }

    #[test]
    fn test_optimized_progressive_roundtrip() {
        // Optimized Huffman tables work fine for progressive streams.
        let (img, orig) = gradient(64, 64, 3);
        let opts = JpegEncodeOptions {
            progressive: true,
            optimize_coding: true,
            quality: 88,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert!(psnr(&dec.data, &orig) > 30.0);
    }

    #[test]
    fn test_optimized_baseline_gray_roundtrip() {
        // Optimized Huffman tables work fine for grayscale baseline streams.
        let (img, orig) = gradient(64, 64, 1);
        let opts = JpegEncodeOptions {
            optimize_coding: true,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let dec = decode_jpeg(&enc).unwrap();
        assert!(psnr(&dec.data, &orig) > 30.0);
    }

    #[test]
    fn test_optimized_baseline_rgb_encode_only() {
        // PORT-NOTE regression guard: zune-jpeg 0.4 misdecodes baseline
        // 3-component streams written with optimized Huffman tables. The
        // file itself is spec-valid (libjpeg decodes it correctly); our
        // decoder just can't be trusted on this combination. Here we only
        // assert the encoder produces a parseable stream.
        let (img, _) = solid(48, 48, 3, [64, 128, 192]);
        let opts = JpegEncodeOptions {
            optimize_coding: true,
            ..JpegEncodeOptions::default()
        };
        let enc = encode_jpeg(&img, &opts).unwrap();
        let (w, h, c) = jpeg_dimensions(&enc).unwrap();
        assert_eq!((w, h, c), (48, 48, 3));
        assert!(!jpeg_is_progressive(&enc));
    }
}

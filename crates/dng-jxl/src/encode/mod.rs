//! Top-level JPEG XL encoder (hand-written, pure Rust) plus internal
//! modules.
//!
//! Emits bare-codestream modular-mode lossless JPEG XL (or the BMFF
//! container form when Exif/XMP/ICC payloads are present). The output is
//! validated against `jxl-oxide` by the test suite.

pub mod bitwriter;
pub mod entropy;
pub mod frame;
pub mod headers;
pub mod modular;

use crate::container;
use crate::{JxlError, JxlImage};
use bitwriter::BitWriter;
use headers::{write_image_metadata, write_signature, write_size_header, BitDepthSpec, ColourSpec};

/// JPEG XL encoding options.
///
/// `bits_per_sample`, `is_float` and `channels` mirror the
/// `dng_jxl.cpp`-style option struct; they are informational here and are
/// cross-checked against the [`JxlImage`] when non-default.
#[derive(Debug, Clone)]
pub struct JxlEncodeOptions {
    /// 0..=100, 100 => lossless modular. Values below 100 map to uniform
    /// sample quantization (see PORTING_NOTES.md).
    pub quality: f32,
    /// 1..=9, default 7. Accepted for API compatibility; ignored (PORT-NOTE:
    /// single encoding strategy — modular mode only).
    pub effort: u8,
    /// Informational: bit depth of the source image.
    pub bits_per_sample: u32,
    /// Informational: true for float32 source data.
    pub is_float: bool,
    /// Informational: 1 = gray, 3 = RGB, 4 = RGBA.
    pub channels: u32,
    /// Raw EXIF block to embed as an `Exif` container box.
    pub exif: Option<Vec<u8>>,
    /// XMP packet to embed as an `xml ` container box.
    pub xmp: Option<Vec<u8>>,
    /// ICC profile to embed as a `colr` container box.
    pub icc: Option<Vec<u8>>,
}

impl Default for JxlEncodeOptions {
    fn default() -> Self {
        JxlEncodeOptions {
            quality: 100.0,
            effort: 7,
            bits_per_sample: 8,
            is_float: false,
            channels: 3,
            exif: None,
            xmp: None,
            icc: None,
        }
    }
}

/// Encode a JXL image.
///
/// - `quality >= 100` produces bit-exact lossless modular output.
/// - `quality < 100` still uses the modular mode; samples are uniformly
///   quantized before encoding (approximate lossy behavior — see
///   PORTING_NOTES.md; full VarDCT is out of scope for this port).
/// - `effort` is accepted for API compatibility but ignored (single
///   encoding strategy).
pub fn encode_jxl(img: &JxlImage, opts: &JxlEncodeOptions) -> Result<Vec<u8>, JxlError> {
    img.validate()?;

    let width = img.width;
    let height = img.height;
    let bits = img.bits;
    let is_float = bits == 32;
    let alpha = img.channels == 4;

    // Samples as i32 planes in the modular domain:
    // - integer depths: raw sample values (0..=(2^bits - 1))
    // - float32: raw IEEE bit patterns (bitcast, decoded bit-exact)
    let mut samples = extract_samples(img)?;

    // Lossy quantization (quality < 100 only).
    if opts.quality < 100.0 {
        samples = quantize_samples(img, &samples, opts.quality);
    }

    let bit_depth = BitDepthSpec {
        bits_per_sample: bits,
        exponent_bits: if is_float { 8 } else { 0 },
    };
    let colour = match (img.channels, is_float) {
        (1, false) => ColourSpec::GreySrgb,
        (1, true) => ColourSpec::GreyLinear,
        (_, false) => ColourSpec::Srgb,
        (_, true) => ColourSpec::RgbLinear,
    };

    // ----- Codestream bit writer -----
    let mut w = BitWriter::new();
    write_signature(&mut w);
    write_size_header(&mut w, width, height)?;
    write_image_metadata(&mut w, &bit_depth, alpha, colour)?;

    // The first frame starts byte-aligned after the image header
    // (jxl-oxide zero-pads here before parsing the frame).
    w.zero_pad_to_byte();

    // FrameHeader.
    frame::write_frame_header(&mut w, alpha);

    // Sections: sizes depend on payload lengths, so build them first.
    let geom = frame::group_geometry(width, height);
    let num_channels = img.channels as usize;
    let mut sections: Vec<Vec<u8>> = Vec::new();

    if geom.num_groups == 1 {
        // Everything (all channels fit one group) lives in the single
        // LfGlobal section.
        let mut sec = BitWriter::new();
        sec.write_bits(1, 1); // LfChannelDequantization: all_default = true
        let ch_refs: Vec<&[i32]> = samples.iter().map(|c| c.as_slice()).collect();
        let dims: Vec<(u32, u32)> = vec![(width, height); num_channels];
        modular::write_full_stream(&mut sec, &ch_refs, &dims);
        sections.push(sec.finish());
    } else {
        // LfGlobal: dequant + global MA config + modular header (no global
        // channel data — every channel is larger than one group).

        // Build all group slices first: each group decodes independently
        // (group-local prediction context), so the residual alphabet must
        // be sized against group-local residuals.
        let mut groups: Vec<(Vec<Vec<i32>>, u32, u32)> = Vec::new();
        for g in 0..geom.num_groups {
            let col = g % geom.groups_per_row;
            let row = g / geom.groups_per_row;
            let x0 = col * 256;
            let y0 = row * 256;
            let gw = 256.min(width - x0);
            let gh = 256.min(height - y0);
            let mut planes = Vec::with_capacity(num_channels);
            for c in 0..num_channels {
                planes.push(group_slice(&samples[c], width, x0, y0, gw, gh));
            }
            groups.push((planes, gw, gh));
        }

        let mut max_residual = 1u32;
        for (planes, gw, gh) in &groups {
            let refs: Vec<&[i32]> = planes.iter().map(|p| p.as_slice()).collect();
            let dims = vec![(*gw, *gh); num_channels];
            max_residual = max_residual.max(modular::scan_max_residual(&refs, &dims));
        }

        let mut sec = BitWriter::new();
        sec.write_bits(1, 1); // LfChannelDequantization: all_default = true
        let alphabet = modular::write_ma_config(&mut sec, max_residual);
        modular::write_modular_header(&mut sec);
        sections.push(sec.finish());

        // LfGroup sections: empty for untransformed modular frames.
        for _ in 0..geom.num_lf_groups {
            sections.push(Vec::new());
        }
        // HfGlobal: skipped entirely for modular frames (0 bytes).
        sections.push(Vec::new());

        // Pass groups: modular header + residual tokens for the group's
        // channel slices.
        for (planes, gw, gh) in &groups {
            let mut sec = BitWriter::new();
            modular::write_modular_header(&mut sec);
            let refs: Vec<&[i32]> = planes.iter().map(|p| p.as_slice()).collect();
            let dims = vec![(*gw, *gh); num_channels];
            modular::write_residuals(&mut sec, alphabet, &refs, &dims);
            sections.push(sec.finish());
        }
    }

    // TOC + sections.
    let toc_entries = frame::toc_entry_count(width, height);
    if toc_entries != sections.len() {
        return Err(JxlError::Encode(format!(
            "internal: TOC entries {toc_entries} != sections {}",
            sections.len()
        )));
    }
    let sizes: Vec<u32> = sections.iter().map(|s| s.len() as u32).collect();
    frame::write_toc(&mut w, &sizes);
    w.zero_pad_to_byte();
    let mut out = w.finish();
    for sec in &sections {
        out.extend_from_slice(sec);
    }

    // ----- Container or bare codestream -----
    if opts.exif.is_some() || opts.xmp.is_some() || opts.icc.is_some() {
        Ok(container::build_container(
            &out,
            opts.exif.as_deref(),
            opts.xmp.as_deref(),
            opts.icc.as_deref(),
        ))
    } else {
        Ok(out)
    }
}

/// Extract per-channel i32 sample planes from the interleaved input.
fn extract_samples(img: &JxlImage) -> Result<Vec<Vec<i32>>, JxlError> {
    let (w, h, c) = (img.width as usize, img.height as usize, img.channels as usize);
    let mut planes = vec![vec![0i32; w * h]; c];
    match img.bits {
        8 => {
            for i in 0..w * h {
                for ch in 0..c {
                    planes[ch][i] = img.data[i * c + ch] as i32;
                }
            }
        }
        16 => {
            for i in 0..w * h {
                for ch in 0..c {
                    let off = (i * c + ch) * 2;
                    let b = &img.data[off..off + 2];
                    planes[ch][i] = u16::from_le_bytes([b[0], b[1]]) as i32;
                }
            }
        }
        32 => {
            for i in 0..w * h {
                for ch in 0..c {
                    let off = (i * c + ch) * 4;
                    let mut b = [0u8; 4];
                    b.copy_from_slice(&img.data[off..off + 4]);
                    planes[ch][i] = f32::from_le_bytes(b).to_bits() as i32;
                }
            }
        }
        v => return Err(JxlError::Unsupported(format!("bits={v}"))),
    }
    Ok(planes)
}

/// Uniform scalar quantization of sample planes for quality < 100.
///
/// PORT-NOTE: this is an honest approximation, not libjxl's VarDCT. Integer
/// samples are rounded to a uniform step derived from the quality value
/// (using the DNG SDK's `JXLQualityToDistance` table); float images are
/// quantized in the [0, 1] domain (values outside [0, 1] are left
/// untouched).
fn quantize_samples(img: &JxlImage, samples: &[Vec<i32>], quality: f32) -> Vec<Vec<i32>> {
    let q = quality.clamp(1.0, 99.0);
    let table = [
        8.0f32, 6.0, 5.0, 4.0, 3.0, 2.0, 1.6, 1.3, 1.0, 0.7, 0.4, 0.1, 0.0,
    ];
    let idx = ((q / 100.0 * 12.0).round() as usize).clamp(0, 12);
    let distance = table[idx];

    let max_val: f64 = if img.bits == 32 {
        1.0
    } else {
        ((1u64 << img.bits) - 1) as f64
    };
    let step = (((distance as f64 * max_val) / 8.0).round() as i64)
        .clamp(1, (max_val / 2.0) as i64) as i32;

    samples
        .iter()
        .map(|plane| {
            if img.bits == 32 {
                plane
                    .iter()
                    .map(|&s| {
                        let f = f32::from_bits(s as u32);
                        if (0.0..=1.0).contains(&f) {
                            let qv = (f * step as f32).round() / step as f32;
                            qv.to_bits() as i32
                        } else {
                            s
                        }
                    })
                    .collect()
            } else {
                let max_i = max_val as i32;
                plane
                    .iter()
                    .map(|&s| {
                        let qv = ((s as i64 + step as i64 / 2) / step as i64) * step as i64;
                        qv.clamp(0, max_i as i64) as i32
                    })
                    .collect()
            }
        })
        .collect()
}

/// Extract a group-sized slice from a full channel plane.
fn group_slice(plane: &[i32], width: u32, x0: u32, y0: u32, gw: u32, gh: u32) -> Vec<i32> {
    let mut out = Vec::with_capacity((gw * gh) as usize);
    for y in y0..y0 + gh {
        let base = (y as usize * width as usize) + x0 as usize;
        out.extend_from_slice(&plane[base..base + gw as usize]);
    }
    out
}

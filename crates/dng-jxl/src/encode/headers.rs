//! Codestream header writers: signature, SizeHeader, ImageMetadata
//! (including ColourEncoding), matching the `jxl-image` bundle parsers.

use super::bitwriter::{write_u32_spec, BitWriter};
use crate::JxlError;

/// Write the codestream signature (0xFF 0x0A) as the first 16 bits.
pub fn write_signature(w: &mut BitWriter) {
    // LSB-first 16-bit value 0x0AFF == bytes FF 0A.
    w.write_bits(0x0AFF, 16);
}

/// Height/width U32 spec used by SizeHeader: `U32(1 + u(9), 1 + u(13), 1 + u(18), 1 + u(30))`.
const SIZE_U32: [(u32, usize); 4] = [(1, 9), (1, 13), (1, 18), (1, 30)];

/// Write the SizeHeader.
pub fn write_size_header(w: &mut BitWriter, width: u32, height: u32) -> Result<(), JxlError> {
    // div8 = false.
    w.write_bits(0, 1);
    // height.
    write_u32_spec(w, SIZE_U32, height)?;
    // ratio = 0 (custom width follows).
    w.write_bits(0, 3);
    // width.
    write_u32_spec(w, SIZE_U32, width)?;
    Ok(())
}

/// Bit depth to write.
#[derive(Debug, Clone, Copy)]
pub struct BitDepthSpec {
    pub bits_per_sample: u32,
    /// 0 for integer samples; exponent bits for float (5 = f16, 8 = f32).
    pub exponent_bits: u32,
}

/// Write the BitDepth bundle.
pub fn write_bit_depth(w: &mut BitWriter, bd: &BitDepthSpec) -> Result<(), JxlError> {
    if bd.exponent_bits == 0 {
        // IntegerSample.
        w.write_bits(0, 1);
        // U32(8, 10, 12, 1 + U(6)).
        match bd.bits_per_sample {
            8 => w.write_bits(0, 2),
            10 => w.write_bits(1, 2),
            12 => w.write_bits(2, 2),
            v => {
                w.write_bits(3, 2);
                w.write_bits((v - 1) as u64, 6);
            }
        }
    } else {
        // FloatSample.
        w.write_bits(1, 1);
        // U32(32, 16, 24, 1 + U(6)).
        match bd.bits_per_sample {
            32 => w.write_bits(0, 2),
            16 => w.write_bits(1, 2),
            24 => w.write_bits(2, 2),
            v => {
                w.write_bits(3, 2);
                w.write_bits((v - 1) as u64, 6);
            }
        }
        // exp_bits = read_bits(4) + 1.
        w.write_bits((bd.exponent_bits - 1) as u64, 4);
    }
    Ok(())
}

/// Color encoding to declare in the codestream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColourSpec {
    /// sRGB (enum encoding, all_default bit).
    Srgb,
    /// Grayscale with the sRGB transfer function.
    GreySrgb,
    /// Grayscale, linear transfer function.
    GreyLinear,
    /// RGB, linear transfer function (for float linear data).
    RgbLinear,
}

/// Write the ColourEncoding bundle.
pub fn write_colour_encoding(w: &mut BitWriter, spec: ColourSpec) {
    match spec {
        ColourSpec::Srgb => {
            // all_default = true -> sRGB.
            w.write_bits(1, 1);
        }
        _ => {
            w.write_bits(0, 1); // all_default = false
            w.write_bits(0, 1); // want_icc = false
            let (grey, tf_linear) = match spec {
                ColourSpec::GreySrgb => (true, false),
                ColourSpec::GreyLinear => (true, true),
                ColourSpec::RgbLinear => (false, true),
                ColourSpec::Srgb => unreachable!(),
            };
            // colour_space = read_enum::<ColourSpace>: Rgb=0 -> selector 0,
            // Grey=1 -> selector 1.
            if grey {
                w.write_bits(1, 2); // selector 1 -> value 1 (Grey)
            } else {
                w.write_bits(0, 2); // selector 0 -> value 0 (Rgb)
            }
            // white_point = read_enum::<WhitePointDiscriminator>:
            // D65=1 -> selector 1 (read_u32(0,1,2+U(4),18+U(6))).
            w.write_bits(1, 2);
            // primaries: skipped for Grey; Srgb=1 -> selector 1 for RGB.
            if !grey {
                w.write_bits(1, 2);
            }
            // transfer_function: has_gamma bool; false -> read_enum.
            w.write_bits(0, 1);
            // TransferFunction::Srgb=13 -> selector 2, 4-bit payload 11.
            // TransferFunction::Linear=8 -> selector 2, 4-bit payload 6.
            if tf_linear {
                w.write_bits(2, 2);
                w.write_bits(6, 4);
            } else {
                w.write_bits(2, 2);
                w.write_bits(11, 4);
            }
            // rendering_intent = Relative=1 -> selector 1.
            w.write_bits(1, 2);
        }
    }
}

/// Write the full ImageMetadata (excluding the signature and SizeHeader).
///
/// One frame, no extra fields, no preview/animation, `modular_16bit_buffers`
/// disabled (i32 modular samples — avoids the narrow-buffer wraparound
/// subtleties), optional alpha extra channel, xyb_encoded = false.
pub fn write_image_metadata(
    w: &mut BitWriter,
    bit_depth: &BitDepthSpec,
    alpha: bool,
    colour: ColourSpec,
) -> Result<(), JxlError> {
    // all_default = false.
    w.write_bits(0, 1);
    // extra_fields = false (orientation 1, no preview, no animation).
    w.write_bits(0, 1);
    // bit_depth (cond !all_default).
    write_bit_depth(w, bit_depth)?;
    // modular_16bit_buffers = false (cond !all_default).
    w.write_bits(0, 1);
    // num_extra = U32(0, 1, 2 + u(4), 1 + u(12)).
    if alpha {
        w.write_bits(1, 2); // selector 1 -> 1 extra channel
        // ExtraChannelInfo: default_alpha_channel = true gives an Alpha
        // channel with 8-bit default bit depth; for other depths write the
        // explicit form.
        if bit_depth.bits_per_sample == 8 && bit_depth.exponent_bits == 0 {
            w.write_bits(1, 1); // default alpha channel
        } else {
            w.write_bits(0, 1); // not default
            // ty_id = read_enum (Alpha = 0 -> selector 0).
            w.write_bits(0, 2);
            // BitDepth.
            write_bit_depth(
                w,
                &BitDepthSpec {
                    bits_per_sample: bit_depth.bits_per_sample,
                    exponent_bits: bit_depth.exponent_bits,
                },
            )?;
            // dim_shift = U32(0, 3, 4, 1 + U(3)) -> selector 0 (0).
            w.write_bits(0, 2);
            // name: Name bundle, length 0.
            w.write_bits(0, 2); // U32(0, u(4), 16+u(5), 48+u(10)) selector 0
            // (Alpha has alpha_associated bool after name in the parse? No:
            // the bool is read inside the ty match after name — check: the
            // parse reads name then matches ty_id, reading alpha_associated
            // for Alpha. Order in code: ty_id, bit_depth, dim_shift, name,
            // then the match reads alpha_associated.)
            // alpha_associated = false (non-premultiplied).
            w.write_bits(0, 1);
        }
    } else {
        w.write_bits(0, 2); // selector 0 -> 0 extra channels
    }
    // xyb_encoded = false (cond !all_default): original profile data.
    w.write_bits(0, 1);
    // colour_encoding (cond !all_default).
    write_colour_encoding(w, colour);
    // tone_mapping: cond(extra_fields) — skipped.
    // extensions: U64 — 2-bit selector 0 (value 0).
    w.write_bits(0, 2);
    // default_m: unconditional bool.
    w.write_bits(1, 1);
    // opsin inverse matrix etc.: cond(!default_m && xyb_encoded) — skipped.
    // up2/up4/up8 weights: cond(cw_mask & n) — cw_mask cond(!default_m) —
    // skipped since default_m = true.
    Ok(())
}

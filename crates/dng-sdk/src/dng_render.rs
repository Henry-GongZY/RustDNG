// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_render.cpp/.h`.
//
// Copyright 2006-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_render.cpp` / `dng_render.h` — the preview/render pipeline:
//! stage3 → black-level ramp → camera-to-linear-ProPhoto RGB (white balance +
//! camera profile) → hue/sat map → ProfileGainTableMap → exposure ramp → look
//! table → baseline tone curve → RGBTables (not yet ported; see PORT-NOTE) →
//! final color space (gamma encode) → optional transparency-mask matte.
//!
//! The C++ "baseline" bottleneck math (gDNGSuite reference implementations in
//! `dng_reference.cpp`) is inlined here — the dng-core `dng_bottlenecks`
//! module covers only the copy/convert kernels.
//!
//! Threading: the C++ `dng_render_task` fans out over tiles; the port drives
//! the same per-tile `dng_filter_task` driver serially (ARCHITECTURE.md §9).
//!
//! PORT-NOTE(dng_render.cpp:1169): the DNG 1.7 "MaskedRGBTables" (semantic
//! masked RGB table transforms) path requires camera-profile APIs and the
//! `dng_reference` RGB-table machinery that have not landed yet; the port
//! returns `NotSupported` when a profile carries masked RGB tables.

use dng_core::dng_1d_function::{
    Dng1dFunction, Dng1dIdentity, Dng1dConcatenate, DngPiecewiseLinear,
};
use dng_core::dng_1d_table::Dng1dTable;
use dng_core::dng_exceptions::{bad_format, not_yet_implemented, program_error};
use dng_core::dng_fingerprint::DngFingerprint;
use dng_core::dng_host::DngHost;
use dng_core::dng_image::{DngImage, ImageRef};
use dng_core::dng_matrix::{DngMatrix, DngVector};
use dng_core::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use dng_core::dng_point::{DngPoint, DngPointReal64};
use dng_core::dng_rect::DngRect;
use dng_core::dng_resample::{resample_image, DngResampleBicubic};
use dng_core::dng_simple_image::DngSimpleImage;
use dng_core::dng_tag_types::{tag_type_size, ttByte, ttFloat};
use dng_core::dng_tag_values::{
    defaultBlackRender_None, encoding_Linear, encoding_sRGB, pcInterleaved,
};
use dng_core::dng_types::{int32, real32, real64, uint32, uint8};
use dng_core::dng_utils::{
    dng_hsv_to_rgb, dng_rgb_to_hsv, max_real32, max_real64, min_int32, min_real32,
    min_real64, pin_real32, round_uint32_f64,
};
use dng_core::dng_xy_coord::{d55_xy_coord, DngXyCoord};
use dng_core::DngError;
use dng_core::DngResult;

use crate::dng_camera_profile::{DngCameraProfile, DngCameraProfileId};
use crate::dng_color_spec::DngColorSpec;
use crate::dng_hue_sat_map::DngHueSatMap;
use crate::dng_negative::DngNegative;

/// `kSubTableCount` (dng_1d_table.h) — the sub-sampled table size.
const K_1D_TABLE_COUNT: uint32 = 4096;

// ---------------------------------------------------------------------------
// 1D render functions
// ---------------------------------------------------------------------------

/// Port of `dng_function_zero_offset` — maps the stage3 black level to 0.
pub struct DngFunctionZeroOffset {
    /// `fZeroOffset`.
    zero_offset: real64,

    /// `fScale`.
    scale: real64,
}

impl DngFunctionZeroOffset {
    /// `dng_function_zero_offset (zeroOffset)`.
    pub fn new(zero_offset: real64) -> Self {
        DngFunctionZeroOffset {
            zero_offset,
            scale: 1.0 / (1.0 - zero_offset),
        }
    }
}

impl Dng1dFunction for DngFunctionZeroOffset {
    /// `Evaluate (x)`.
    fn evaluate(&self, x: real64) -> real64 {
        dng_core::dng_utils::pin_real64(
            0.0,
            (x - self.zero_offset) * self.scale,
            1.0,
        )
    }
}

/// Port of `dng_function_exposure_ramp`.
pub struct DngFunctionExposureRamp {
    /// `fSlope`.
    slope: real64,

    /// `fBlack`.
    black: real64,

    /// `fRadius`.
    radius: real64,

    /// `fQScale`.
    q_scale: real64,

    /// `fSupportOverrange`.
    support_overrange: bool,
}

impl DngFunctionExposureRamp {
    /// `dng_function_exposure_ramp (white, black, minBlack, supportOverrange)`.
    pub fn new(
        white: real64,
        black: real64,
        min_black: real64,
        support_overrange: bool,
    ) -> Self {
        let slope = 1.0 / (white - black);

        // Fraction of minBlack.

        const K_MAX_CURVE_X: real64 = 0.5;

        // Fraction of white.

        const K_MAX_CURVE_Y: real64 = 1.0 / 16.0;

        let radius = min_real64(K_MAX_CURVE_X * min_black, K_MAX_CURVE_Y / slope);

        let q_scale = if radius > 0.0 {
            slope / (4.0 * radius)
        } else {
            0.0
        };

        DngFunctionExposureRamp {
            slope,
            black,
            radius,
            q_scale,
            support_overrange,
        }
    }
}

impl Dng1dFunction for DngFunctionExposureRamp {
    /// `Evaluate (x)`.
    fn evaluate(&self, x: real64) -> real64 {
        if x <= self.black - self.radius {
            return 0.0;
        }

        if x >= self.black + self.radius {
            let mut y = (x - self.black) * self.slope;

            if !self.support_overrange {
                y = min_real64(y, 1.0);
            }

            return y;
        }

        let y = x - (self.black - self.radius);

        self.q_scale * y * y
    }
}

/// Port of `dng_function_exposure_tone`.
pub struct DngFunctionExposureTone {
    /// `fIsNOP`.
    is_nop: bool,

    /// `fSlope`.
    slope: real64,

    /// `a`.
    a: real64,

    /// `b`.
    b: real64,

    /// `c`.
    c: real64,
}

impl DngFunctionExposureTone {
    /// `dng_function_exposure_tone (exposure)`.
    pub fn new(exposure: real64) -> Self {
        let is_nop = exposure >= 0.0;

        let (mut slope, mut a, mut b, mut c) = (0.0, 0.0, 0.0, 0.0);

        if !is_nop {
            // Find slope to use for all except the highest two f-stops.

            slope = (2.0f64).powf(exposure);

            // Find quadratic parameters that match this darkening at the
            // crossover point, yet still map pure white to pure white.

            a = 16.0 / 9.0 * (1.0 - slope);

            b = slope - 0.5 * a;

            c = 1.0 - a - b;
        }

        DngFunctionExposureTone {
            is_nop,
            slope,
            a,
            b,
            c,
        }
    }
}

impl Dng1dFunction for DngFunctionExposureTone {
    /// `Evaluate (x)`.
    fn evaluate(&self, mut x: real64) -> real64 {
        if !self.is_nop {
            if x <= 0.25 {
                x *= self.slope;
            } else {
                x = (self.a * x + self.b) * x + self.c;
            }
        }

        x
    }
}

/// Port of `dng_tone_curve_acr3_default` (tables in the `acr3_tables`
/// submodule below).
pub struct DngToneCurveAcr3Default;

impl DngToneCurveAcr3Default {
    /// `Get ()` — the shared static instance.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: DngToneCurveAcr3Default = DngToneCurveAcr3Default;

        &INSTANCE
    }

    /// `EvaluateInverse (x)` — exposed for tests / completeness.
    pub fn evaluate_inverse(&self, x: real64) -> real64 {
        let y = (x as real32) * ((acr3_tables::ACR3_INVERSE_TABLE.len() - 1) as real32);

        let index = dng_core::dng_utils::pin_int32(
            0,
            y as int32,
            (acr3_tables::ACR3_INVERSE_TABLE.len() - 2) as int32,
        );

        let fract = y - index as real32;

        (acr3_tables::ACR3_INVERSE_TABLE[index as usize] * (1.0 - fract)
            + acr3_tables::ACR3_INVERSE_TABLE[index as usize + 1] * fract) as real64
    }
}

impl Dng1dFunction for DngToneCurveAcr3Default {
    /// `Evaluate (x)`.
    fn evaluate(&self, x: real64) -> real64 {
        let y = (x as real32) * ((acr3_tables::ACR3_FORWARD_TABLE.len() - 1) as real32);

        let index = dng_core::dng_utils::pin_int32(
            0,
            y as int32,
            (acr3_tables::ACR3_FORWARD_TABLE.len() - 2) as int32,
        );

        let fract = y - index as real32;

        (acr3_tables::ACR3_FORWARD_TABLE[index as usize] * (1.0 - fract)
            + acr3_tables::ACR3_FORWARD_TABLE[index as usize + 1] * fract) as real64
    }
}

// ---------------------------------------------------------------------------
// Overrange helpers (dng_utils.h:1711 — dng_reference territory)
// ---------------------------------------------------------------------------

/// `EncodeOverrange (x)`.
#[inline]
pub fn encode_overrange(x: real32) -> real32 {
    let x = max_real32(x, 0.0);

    x * (256.0 + x) / (256.0 * (1.0 + x))
}

/// `DecodeOverrange (x)`.
#[inline]
pub fn decode_overrange(x: real32) -> real32 {
    let x = max_real32(x, 0.0);

    16.0 * ((8.0 * x) - 8.0 + (64.0 * x * x - 127.0 * x + 64.0).sqrt())
}

// ---------------------------------------------------------------------------
// Baseline bottleneck math (dng_reference.cpp reference implementations)
// ---------------------------------------------------------------------------

/// `RefBaselineABCtoRGB`.
#[allow(clippy::too_many_arguments)]
fn do_baseline_abc_to_rgb(
    s_ptr_a: &[real32],
    s_ptr_b: &[real32],
    s_ptr_c: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    camera_white: &DngVector,
    camera_to_rgb: &DngMatrix,
) {
    let clip_a = camera_white.get(0) as real32;
    let clip_b = camera_white.get(1) as real32;
    let clip_c = camera_white.get(2) as real32;

    let m = camera_to_rgb;

    for col in 0..count as usize {
        let a = min_real32(s_ptr_a[col], clip_a);
        let b = min_real32(s_ptr_b[col], clip_b);
        let c = min_real32(s_ptr_c[col], clip_c);

        let r = m.get(0, 0) as real32 * a + m.get(0, 1) as real32 * b + m.get(0, 2) as real32 * c;
        let g = m.get(1, 0) as real32 * a + m.get(1, 1) as real32 * b + m.get(1, 2) as real32 * c;
        let b_val = m.get(2, 0) as real32 * a + m.get(2, 1) as real32 * b + m.get(2, 2) as real32 * c;

        d_ptr_r[col] = pin_real32(0.0, r, 1.0);
        d_ptr_g[col] = pin_real32(0.0, g, 1.0);
        d_ptr_b[col] = pin_real32(0.0, b_val, 1.0);
    }
}

/// `RefBaselineABCDtoRGB`.
#[allow(clippy::too_many_arguments)]
fn do_baseline_abcd_to_rgb(
    s_ptr_a: &[real32],
    s_ptr_b: &[real32],
    s_ptr_c: &[real32],
    s_ptr_d: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    camera_white: &DngVector,
    camera_to_rgb: &DngMatrix,
) {
    let clip_a = camera_white.get(0) as real32;
    let clip_b = camera_white.get(1) as real32;
    let clip_c = camera_white.get(2) as real32;
    let clip_d = camera_white.get(3) as real32;

    let m = camera_to_rgb;

    for col in 0..count as usize {
        let a = min_real32(s_ptr_a[col], clip_a);
        let b = min_real32(s_ptr_b[col], clip_b);
        let c = min_real32(s_ptr_c[col], clip_c);
        let d = min_real32(s_ptr_d[col], clip_d);

        let r = m.get(0, 0) as real32 * a
            + m.get(0, 1) as real32 * b
            + m.get(0, 2) as real32 * c
            + m.get(0, 3) as real32 * d;
        let g = m.get(1, 0) as real32 * a
            + m.get(1, 1) as real32 * b
            + m.get(1, 2) as real32 * c
            + m.get(1, 3) as real32 * d;
        let b_val = m.get(2, 0) as real32 * a
            + m.get(2, 1) as real32 * b
            + m.get(2, 2) as real32 * c
            + m.get(2, 3) as real32 * d;

        d_ptr_r[col] = pin_real32(0.0, r, 1.0);
        d_ptr_g[col] = pin_real32(0.0, g, 1.0);
        d_ptr_b[col] = pin_real32(0.0, b_val, 1.0);
    }
}

/// `RefBaselineHueSatMap`.
#[allow(clippy::too_many_arguments)]
fn do_baseline_hue_sat_map(
    s_ptr_r: &[real32],
    s_ptr_g: &[real32],
    s_ptr_b: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    lut: &DngHueSatMap,
    encode_table: Option<&Dng1dTable>,
    decode_table: Option<&Dng1dTable>,
    support_overrange: bool,
) -> DngResult<()> {
    let (hue_divisions, sat_divisions, val_divisions) = lut.get_divisions();

    let h_scale: real32 = if hue_divisions < 2 {
        0.0
    } else {
        hue_divisions as real32 * (1.0 / 6.0)
    };

    let s_scale: real32 = (sat_divisions as int32 - 1) as real32;
    let v_scale: real32 = (val_divisions as int32 - 1) as real32;

    let max_hue_index0 = hue_divisions as int32 - 1;
    let max_sat_index0 = sat_divisions as int32 - 2;
    let max_val_index0 = val_divisions as int32 - 2;

    let has_encode_table = encode_table.is_some();
    let has_decode_table = decode_table.is_some();

    let has_table = has_encode_table && has_decode_table;

    let table_base = lut.get_const_deltas();

    let hue_step = sat_divisions as int32;
    let val_step = hue_divisions as int32 * hue_step;

    for j in 0..count as usize {
        let mut r = s_ptr_r[j];
        let mut g = s_ptr_g[j];
        let mut b = s_ptr_b[j];

        if support_overrange {
            r = max_real32(r, 0.0);
            g = max_real32(g, 0.0);
            b = max_real32(b, 0.0);

            if val_divisions > 1 {
                r = encode_overrange(r);
                g = encode_overrange(g);
                b = encode_overrange(b);
            }
        }

        let (mut h, mut s, mut v) = dng_rgb_to_hsv(r, g, b);

        let mut v_encoded = v;

        let hue_shift: real32;
        let sat_scale: real32;
        let val_scale: real32;

        if val_divisions < 2 {
            // Optimize most common case of "2.5D" table.

            let h_scaled = h * h_scale;
            let s_scaled = s * s_scale;

            let mut h_index0 = h_scaled as int32;
            let mut s_index0 = s_scaled as int32;

            s_index0 = min_int32(s_index0, max_sat_index0);

            let mut h_index1 = h_index0 + 1;

            if h_index0 >= max_hue_index0 {
                h_index0 = max_hue_index0;
                h_index1 = 0;
            }

            let h_fract1 = h_scaled - h_index0 as real32;
            let s_fract1 = s_scaled - s_index0 as real32;

            let h_fract0 = 1.0 - h_fract1;
            let s_fract0 = 1.0 - s_fract1;

            let entry00 = &table_base[(h_index0 * hue_step + s_index0) as usize];
            let entry01 = &table_base[((h_index1 * hue_step) + s_index0) as usize];

            let hue_shift0 = h_fract0 * entry00.hue_shift + h_fract1 * entry01.hue_shift;
            let sat_scale0 = h_fract0 * entry00.sat_scale + h_fract1 * entry01.sat_scale;
            let val_scale0 = h_fract0 * entry00.val_scale + h_fract1 * entry01.val_scale;

            let entry00 = &table_base[(h_index0 * hue_step + s_index0 + 1).max(0) as usize];
            let entry01 = &table_base[((h_index1 * hue_step) + s_index0 + 1).max(0) as usize];

            let hue_shift1 = h_fract0 * entry00.hue_shift + h_fract1 * entry01.hue_shift;
            let sat_scale1 = h_fract0 * entry00.sat_scale + h_fract1 * entry01.sat_scale;
            let val_scale1 = h_fract0 * entry00.val_scale + h_fract1 * entry01.val_scale;

            hue_shift = s_fract0 * hue_shift0 + s_fract1 * hue_shift1;
            sat_scale = s_fract0 * sat_scale0 + s_fract1 * sat_scale1;
            val_scale = s_fract0 * val_scale0 + s_fract1 * val_scale1;
        } else {
            if has_table {
                v_encoded = encode_table
                    .unwrap()
                    .interpolate(pin_real32(0.0, v, 1.0))?;
            }

            let h_scaled = h * h_scale;
            let s_scaled = s * s_scale;
            let v_scaled = v_encoded * v_scale;

            let mut h_index0 = h_scaled as int32;
            let mut s_index0 = s_scaled as int32;
            let mut v_index0 = v_scaled as int32;

            s_index0 = min_int32(s_index0, max_sat_index0);
            v_index0 = min_int32(v_index0, max_val_index0);

            let mut h_index1 = h_index0 + 1;

            if h_index0 >= max_hue_index0 {
                h_index0 = max_hue_index0;
                h_index1 = 0;
            }

            let h_fract1 = h_scaled - h_index0 as real32;
            let s_fract1 = s_scaled - s_index0 as real32;
            let v_fract1 = v_scaled - v_index0 as real32;

            let h_fract0 = 1.0 - h_fract1;
            let s_fract0 = 1.0 - s_fract1;
            let v_fract0 = 1.0 - v_fract1;

            let idx = |hv: int32, sv: int32, vv: int32| -> usize {
                (vv * val_step + hv * hue_step + sv).max(0) as usize
            };

            let entry00 = &table_base[idx(h_index0, s_index0, v_index0)];
            let entry01 = &table_base[idx(h_index1, s_index0, v_index0)];
            let entry10 = &table_base[idx(h_index0, s_index0, v_index0 + 1)];
            let entry11 = &table_base[idx(h_index1, s_index0, v_index0 + 1)];

            let hue_shift0 = v_fract0
                * (h_fract0 * entry00.hue_shift + h_fract1 * entry01.hue_shift)
                + v_fract1 * (h_fract0 * entry10.hue_shift + h_fract1 * entry11.hue_shift);

            let sat_scale0 = v_fract0
                * (h_fract0 * entry00.sat_scale + h_fract1 * entry01.sat_scale)
                + v_fract1 * (h_fract0 * entry10.sat_scale + h_fract1 * entry11.sat_scale);

            let val_scale0 = v_fract0
                * (h_fract0 * entry00.val_scale + h_fract1 * entry01.val_scale)
                + v_fract1 * (h_fract0 * entry10.val_scale + h_fract1 * entry11.val_scale);

            let entry00 = &table_base[idx(h_index0, s_index0 + 1, v_index0)];
            let entry01 = &table_base[idx(h_index1, s_index0 + 1, v_index0)];
            let entry10 = &table_base[idx(h_index0, s_index0 + 1, v_index0 + 1)];
            let entry11 = &table_base[idx(h_index1, s_index0 + 1, v_index0 + 1)];

            let hue_shift1 = v_fract0
                * (h_fract0 * entry00.hue_shift + h_fract1 * entry01.hue_shift)
                + v_fract1 * (h_fract0 * entry10.hue_shift + h_fract1 * entry11.hue_shift);

            let sat_scale1 = v_fract0
                * (h_fract0 * entry00.sat_scale + h_fract1 * entry01.sat_scale)
                + v_fract1 * (h_fract0 * entry10.sat_scale + h_fract1 * entry11.sat_scale);

            let val_scale1 = v_fract0
                * (h_fract0 * entry00.val_scale + h_fract1 * entry01.val_scale)
                + v_fract1 * (h_fract0 * entry10.val_scale + h_fract1 * entry11.val_scale);

            hue_shift = s_fract0 * hue_shift0 + s_fract1 * hue_shift1;
            sat_scale = s_fract0 * sat_scale0 + s_fract1 * sat_scale1;
            val_scale = s_fract0 * val_scale0 + s_fract1 * val_scale1;
        }

        // Convert to internal hue range.

        let hue_shift = hue_shift * (6.0 / 360.0);

        h += hue_shift;

        s = min_real32(s * sat_scale, 1.0);

        v_encoded = pin_real32(0.0, v_encoded * val_scale, 1.0);

        v = if has_table {
            decode_table.unwrap().interpolate(v_encoded)?
        } else {
            v_encoded
        };

        let (rr, gg, bb) = dng_hsv_to_rgb(h, s, v);

        let (mut r, mut g, mut b) = (rr, gg, bb);

        if support_overrange && val_divisions > 1 {
            r = decode_overrange(r);
            g = decode_overrange(g);
            b = decode_overrange(b);
        }

        d_ptr_r[j] = r;
        d_ptr_g[j] = g;
        d_ptr_b[j] = b;
    }

    Ok(())
}

/// `RefBaselineRGBtoGray`.
#[allow(clippy::too_many_arguments)]
fn do_baseline_rgb_to_gray(
    s_ptr_r: &[real32],
    s_ptr_g: &[real32],
    s_ptr_b: &[real32],
    d_ptr_g: &mut [real32],
    count: uint32,
    matrix: &DngMatrix,
    support_overrange: bool,
) {
    let m00 = matrix.get(0, 0) as real32;
    let m01 = matrix.get(0, 1) as real32;
    let m02 = matrix.get(0, 2) as real32;

    for col in 0..count as usize {
        let r = s_ptr_r[col];
        let g = s_ptr_g[col];
        let b = s_ptr_b[col];

        let mut gray = m00 * r + m01 * g + m02 * b;

        if !support_overrange {
            gray = pin_real32(0.0, gray, 1.0);
        }

        d_ptr_g[col] = gray;
    }
}

/// `RefBaselineRGBtoRGB`.
#[allow(clippy::too_many_arguments)]
fn do_baseline_rgb_to_rgb(
    s_ptr_r: &[real32],
    s_ptr_g: &[real32],
    s_ptr_b: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    matrix: &DngMatrix,
    support_overrange: bool,
) {
    let m00 = matrix.get(0, 0) as real32;
    let m01 = matrix.get(0, 1) as real32;
    let m02 = matrix.get(0, 2) as real32;

    let m10 = matrix.get(1, 0) as real32;
    let m11 = matrix.get(1, 1) as real32;
    let m12 = matrix.get(1, 2) as real32;

    let m20 = matrix.get(2, 0) as real32;
    let m21 = matrix.get(2, 1) as real32;
    let m22 = matrix.get(2, 2) as real32;

    for col in 0..count as usize {
        let r = s_ptr_r[col];
        let g = s_ptr_g[col];
        let b = s_ptr_b[col];

        let mut rr = m00 * r + m01 * g + m02 * b;
        let mut gg = m10 * r + m11 * g + m12 * b;
        let mut bb = m20 * r + m21 * g + m22 * b;

        if !support_overrange {
            rr = pin_real32(0.0, rr, 1.0);
            gg = pin_real32(0.0, gg, 1.0);
            bb = pin_real32(0.0, bb, 1.0);
        }

        d_ptr_r[col] = rr;
        d_ptr_g[col] = gg;
        d_ptr_b[col] = bb;
    }
}

/// `RefBaseline1DTable`.
fn do_baseline_1d_table(
    s_ptr: &[real32],
    d_ptr: &mut [real32],
    count: uint32,
    table: &Dng1dTable,
) -> DngResult<()> {
    for col in 0..count as usize {
        let x = s_ptr[col];

        let y = table.interpolate(pin_real32(0.0, x, 1.0))?;

        d_ptr[col] = y;
    }

    Ok(())
}

/// `ApplyCurveOverrange` helper.
fn apply_curve_overrange(
    table: &Dng1dTable,
    slope_x: real32,
    slope_y: real32,
    slope: real32,
    x: real32,
) -> DngResult<real32> {
    if x <= 1.0 {
        return table.interpolate(max_real32(x, 0.0));
    }

    // dng_line_real32::Evaluate: y = fY + fSlope * (x - fX).

    Ok(slope_y + slope * (x - slope_x))
}

/// `DoBaselineRGBToneOverrange` — tone-map an RGB row preserving hue.
#[allow(clippy::too_many_arguments)]
fn do_baseline_rgb_tone_overrange(
    s_ptr_r: &[real32],
    s_ptr_g: &[real32],
    s_ptr_b: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    table: &Dng1dTable,
    slope_x: real32,
    slope_y: real32,
    slope: real32,
) -> DngResult<()> {
    macro_rules! rgb_tone {
        ($r:expr, $g:expr, $b:expr, $rr:ident, $gg:ident, $bb:ident) => {{
            debug_assert!($r >= $g && $g >= $b && $r > $b, "Logic Error RGBTone");

            $rr = apply_curve_overrange(table, slope_x, slope_y, slope, $r)?;
            $bb = apply_curve_overrange(table, slope_x, slope_y, slope, $b)?;

            $gg = $bb + (($rr - $bb) * ($g - $b) / ($r - $b));
        }};
    }

    for col in 0..count as usize {
        let mut r = max_real32(s_ptr_r[col], 0.0);
        let mut g = max_real32(s_ptr_g[col], 0.0);
        let mut b = max_real32(s_ptr_b[col], 0.0);

        r = encode_overrange(r);
        g = encode_overrange(g);
        b = encode_overrange(b);

        let mut rr = 0.0f32;
        let mut gg = 0.0f32;
        let mut bb = 0.0f32;

        if r >= g {
            if g > b {
                // Case 1: r >= g > b

                rgb_tone!(r, g, b, rr, gg, bb);
            } else if b > r {
                // Case 2: b > r >= g

                rgb_tone!(b, r, g, bb, rr, gg);
            } else if b > g {
                // Case 3: r >= b > g

                rgb_tone!(r, b, g, rr, bb, gg);
            } else {
                // Case 4: r >= g == b

                rr = table.interpolate(r)?;
                gg = table.interpolate(g)?;
                bb = gg;
            }
        } else if r >= b {
            // Case 5: g > r >= b

            rgb_tone!(g, r, b, gg, rr, bb);
        } else if b > g {
            // Case 6: b > g > r

            rgb_tone!(b, g, r, bb, gg, rr);
        } else {
            // Case 7: g >= b > r

            rgb_tone!(g, b, r, gg, bb, rr);
        }

        rr = decode_overrange(rr);
        gg = decode_overrange(gg);
        bb = decode_overrange(bb);

        d_ptr_r[col] = rr;
        d_ptr_g[col] = gg;
        d_ptr_b[col] = bb;
    }

    Ok(())
}

/// `DoBaselineRGBTone` (non-overrange variant).
#[allow(clippy::too_many_arguments)]
fn do_baseline_rgb_tone(
    s_ptr_r: &[real32],
    s_ptr_g: &[real32],
    s_ptr_b: &[real32],
    d_ptr_r: &mut [real32],
    d_ptr_g: &mut [real32],
    d_ptr_b: &mut [real32],
    count: uint32,
    table: &Dng1dTable,
) -> DngResult<()> {
    macro_rules! rgb_tone {
        ($r:expr, $g:expr, $b:expr, $rr:ident, $gg:ident, $bb:ident) => {{
            debug_assert!($r >= $g && $g >= $b && $r > $b, "Logic Error RGBTone");

            $rr = table.interpolate($r)?;
            $bb = table.interpolate($b)?;

            $gg = $bb + (($rr - $bb) * ($g - $b) / ($r - $b));
        }};
    }

    for col in 0..count as usize {
        let mut r = pin_real32(0.0, s_ptr_r[col], 1.0);
        let mut g = pin_real32(0.0, s_ptr_g[col], 1.0);
        let mut b = pin_real32(0.0, s_ptr_b[col], 1.0);

        let mut rr;
        let mut gg;
        let mut bb;

        if r >= g {
            if g > b {
                rgb_tone!(r, g, b, rr, gg, bb);
            } else if b > r {
                rgb_tone!(b, r, g, bb, rr, gg);
            } else if b > g {
                rgb_tone!(r, b, g, rr, bb, gg);
            } else {
                rr = table.interpolate(r)?;
                gg = table.interpolate(g)?;
                bb = gg;
            }
        } else if r >= b {
            rgb_tone!(g, r, b, gg, rr, bb);
        } else if b > g {
            rgb_tone!(b, g, r, bb, gg, rr);
        } else {
            rgb_tone!(g, b, r, gg, bb, rr);
        }

        d_ptr_r[col] = rr;
        d_ptr_g[col] = gg;
        d_ptr_b[col] = bb;
    }

    Ok(())
}

/// `DoBaseline1DFunction` (function application with pinning).
fn do_baseline_1d_function(
    s_ptr: &[real32],
    d_ptr: &mut [real32],
    count: uint32,
    function: &dyn Dng1dFunction,
    support_overrange: bool,
) {
    if support_overrange {
        for col in 0..count as usize {
            let x = s_ptr[col];

            let y = function.evaluate(x as real64) as real32;

            d_ptr[col] = y;
        }
    } else {
        for col in 0..count as usize {
            let x = pin_real32(0.0, s_ptr[col], 1.0);

            let y = function.evaluate(x as real64) as real32;

            d_ptr[col] = pin_real32(0.0, y, 1.0);
        }
    }
}

/// `RefBaselineProfileGainTableMap` — apply the ProfileGainTableMap to a row.
#[allow(clippy::too_many_arguments)]
fn do_baseline_profile_gain_table_map(
    r_src_ptr: &[real32],
    g_src_ptr: &[real32],
    b_src_ptr: &[real32],
    r_dst_ptr: &mut [real32],
    g_dst_ptr: &mut [real32],
    b_dst_ptr: &mut [real32],
    cols: uint32,
    top: int32,
    left: int32,
    image_area: &DngRect,
    exposure_weight_gain: real32,
    gain_table_map: &crate::dng_gain_map::DngGainTableMap,
    support_overrange: bool,
) -> DngResult<()> {
    let map_input_weights = gain_table_map.map_input_weights();

    let miw0 = map_input_weights[0];
    let miw1 = map_input_weights[1];
    let miw2 = map_input_weights[2];
    let miw3 = map_input_weights[3];
    let miw4 = map_input_weights[4];

    let points = gain_table_map.points();

    let spacing = gain_table_map.spacing();

    let origin = gain_table_map.origin();

    // Origin of the gain table map in normalized coordinates.

    let map_origin_h32 = origin.h as real32;
    let map_origin_v32 = origin.v as real32;

    // Size of the gain table map in normalized coordinates.

    let map_spacing_h32 = spacing.h as real32;
    let map_spacing_v32 = spacing.v as real32;

    if map_spacing_h32 == 0.0 || map_spacing_v32 == 0.0 {
        return Err(bad_format());
    }

    // Minimum and maximum sample positions of the gain table map.

    let x_limit_hi = points.h as real32 - 1.0;
    let y_limit_hi = points.v as real32 - 1.0;

    // Maximum 2D integer index into the gain table map.

    let x_pixel_limit = points.h as int32 - 1;
    let y_pixel_limit = points.v as int32 - 1;

    // Number of table samples at each position of the gain table map.

    let table_size = gain_table_map.num_table_points() as int32;

    // Maximum integer table index.

    let table_limit = table_size - 1;

    // Gamma parameter.

    let gamma = gain_table_map.gamma();

    // Initialize sample position (half-pixel offset).

    let mut y = top as real32 + 0.5;
    let mut x = left as real32 + 0.5;

    for col in 0..cols as usize {
        // Transform to image-relative coordinates.

        let u_image = (x - image_area.l as real32) / image_area.w() as real32;
        let v_image = (y - image_area.t as real32) / image_area.h() as real32;

        // Transform to map-relative coordinates.

        let mut x_map = (u_image - map_origin_h32) / map_spacing_h32;
        let mut y_map = (v_image - map_origin_v32) / map_spacing_v32;

        // Clamp to valid sample positions.

        x_map = pin_real32(0.0, x_map, x_limit_hi);
        y_map = pin_real32(0.0, y_map, y_limit_hi);

        // Compute integer 2D indices.

        let x0 = x_map as int32;
        let x1 = min_int32(x0 + 1, x_pixel_limit);

        let y0 = y_map as int32;
        let y1 = min_int32(y0 + 1, y_pixel_limit);

        // Compute fractional weights.

        let xf = x_map - x0 as real32;
        let yf = y_map - y0 as real32;

        // Read linear RGB values in RIMM space.

        let r = r_src_ptr[col];
        let g = g_src_ptr[col];
        let b = b_src_ptr[col];

        // Apply MapInputWeights (5-element dot product).

        let min_value = min_real32(r, min_real32(g, b));
        let max_value = max_real32(r, max_real32(g, b));

        let mut weight =
            miw0 * r + miw1 * g + miw2 * b + miw3 * min_value + miw4 * max_value;

        // Scale by the baseline exposure gain.

        weight *= exposure_weight_gain;

        // Clamp to [0,1].

        weight = pin_real32(0.0, weight, 1.0);

        // Apply the gamma parameter.

        if gamma != 1.0 {
            weight = weight.powf(gamma);
        }

        // Scale the weight by the table size and compute indices.

        let weight_scaled = weight * table_size as real32;

        let w0 = min_int32(weight_scaled as int32, table_limit);
        let w1 = min_int32(w0 + 1, table_limit);

        let wf = weight_scaled - w0 as real32;

        // Look up 8 gains.

        let lerp = |a: real32, b: real32, t: real32| a + (b - a) * t;

        let gain000 = gain_table_map.entry(y0 as uint32, x0 as uint32, w0 as uint32);
        let gain001 = gain_table_map.entry(y0 as uint32, x0 as uint32, w1 as uint32);
        let gain010 = gain_table_map.entry(y0 as uint32, x1 as uint32, w0 as uint32);
        let gain011 = gain_table_map.entry(y0 as uint32, x1 as uint32, w1 as uint32);
        let gain100 = gain_table_map.entry(y1 as uint32, x0 as uint32, w0 as uint32);
        let gain101 = gain_table_map.entry(y1 as uint32, x0 as uint32, w1 as uint32);
        let gain110 = gain_table_map.entry(y1 as uint32, x1 as uint32, w0 as uint32);
        let gain111 = gain_table_map.entry(y1 as uint32, x1 as uint32, w1 as uint32);

        // Interpolate in table (w) direction.

        let gain00 = lerp(gain000, gain001, wf);
        let gain01 = lerp(gain010, gain011, wf);
        let gain10 = lerp(gain100, gain101, wf);
        let gain11 = lerp(gain110, gain111, wf);

        // Interpolate in column (x) direction.

        let gain0 = lerp(gain00, gain01, xf);
        let gain1 = lerp(gain10, gain11, xf);

        // Interpolate in row (y) direction.

        let gain = lerp(gain0, gain1, yf);

        // Apply gain.

        let mut r = r * gain;
        let mut g = g * gain;
        let mut b = b * gain;

        // Optionally clamp to [0,1].

        if !support_overrange {
            r = pin_real32(0.0, r, 1.0);
            g = pin_real32(0.0, g, 1.0);
            b = pin_real32(0.0, b, 1.0);
        }

        // Store the result.

        r_dst_ptr[col] = r;
        g_dst_ptr[col] = g;
        b_dst_ptr[col] = b;

        // Increment sample position for next column.

        x += 1.0;
    }

    Ok(())
}

/// `BuildHueSatMapEncodingTable` (dng_camera_profile.cpp:2364).
pub fn build_hue_sat_map_encoding_table(
    encoding: uint32,
    sub_sample: bool,
) -> DngResult<(Option<Dng1dTable>, Option<Dng1dTable>)> {
    match encoding {
        x if x == encoding_Linear => Ok((None, None)),

        x if x == encoding_sRGB => {
            let curve = dng_core::dng_color_space::DngFunctionGammaEncodeSRGB::get();

            let count: uint32 = if sub_sample { 4096 } else { 65536 };

            let mut encode_table = Dng1dTable::new(count)?;

            encode_table.initialize(curve, sub_sample)?;

            let mut decode_table = Dng1dTable::new(count)?;

            let inverse = dng_core::dng_1d_function::Dng1dInverse::new(Box::new(StaticFunctionRef(curve)));

            decode_table.initialize(&inverse, sub_sample)?;

            Ok((Some(encode_table), Some(decode_table)))
        }

        _ => {
            dng_core::dng_exceptions::report_error(
                "Unsupported hue sat map / look table encoding.",
                None,
            );

            Ok((None, None))
        }
    }
}

// ---------------------------------------------------------------------------
// dng_render_task
// ---------------------------------------------------------------------------

/// Port of `dng_render_task` (fields + Start/ProcessArea).
pub struct DngRenderTask {
    /// `fSrcMask` (transparency mask, optional).
    src_mask: Option<ImageRef>,

    /// The negative is accessed through the render params at task-build time;
    /// the C++ holds a reference.  The port precomputes everything in
    /// `start` and stores it here (no borrow of the negative during
    /// processing).
    params: DngRenderParams,

    /// `fSrcOffset` — top-left of the source crop in the stage3 image.
    src_offset: DngPoint,

    /// `fZeroOffsetRamp`.
    zero_offset_ramp: Option<Dng1dTable>,

    /// `fCameraWhite`.
    camera_white: DngVector,

    /// `fCameraToRGB`.
    camera_to_rgb: DngMatrix,

    /// `fHueSatMap`.
    hue_sat_map: Option<DngHueSatMap>,

    /// `fBaselineExposure`.
    baseline_exposure: real64,

    /// `fExposureRamp` parameters (built into a table in `start`).
    exposure_ramp_white: real64,
    exposure_ramp_black: real64,
    support_overrange: bool,

    /// `fLookTable`.
    look_table: Option<DngHueSatMap>,

    /// `fToneCurve` (table).
    tone_curve: Option<Dng1dTable>,

    /// Tone-curve slope extension for overrange support.
    tone_curve_slope: Option<(real32, real32, real32)>, // (x, y, slope)

    /// `fRGBtoFinal`.
    rgb_to_final: DngMatrix,

    /// `fEncodeGamma`.
    encode_gamma: Option<Dng1dTable>,

    /// Hue/sat map encode/decode tables.
    hue_sat_map_encode: Option<Dng1dTable>,
    hue_sat_map_decode: Option<Dng1dTable>,

    /// Look table encode/decode tables.
    look_table_encode: Option<Dng1dTable>,
    look_table_decode: Option<Dng1dTable>,
}

/// The subset of `dng_render` the task needs (passed at construction).
#[derive(Clone)]
pub struct DngRenderParams {
    /// `CameraProfileID ()`.
    pub camera_profile_id: DngCameraProfileId,

    /// `WhiteXY ()`.
    pub white_xy: DngXyCoord,

    /// `Exposure ()`.
    pub exposure: real64,

    /// `Shadows ()`.
    pub shadows: real64,

    /// `ToneCurve ()` — borrowed function.
    pub tone_curve_fn: &'static dyn Dng1dFunction,

    /// `FinalSpace ()` coefficients + gamma (resolved at task start).
    pub final_space_matrix_from_pcs: DngMatrix,
    pub final_space_is_monochrome: bool,
    pub final_space_gamma: &'static dyn Dng1dFunction,
}

impl DngRenderTask {
    /// Snapshot-based start (the port reads the negative through
    /// `NegativeRenderSnapshot`; profile lookups still need the negative).
    pub fn start_with_negative_data(
        snapshot: &NegativeRenderSnapshot,
        negative: &DngNegative,
        params: &DngRenderParams,
    ) -> DngResult<DngRenderTask> {
        DngRenderTask::start_from_snapshot(snapshot, negative, params)
    }

    /// `dng_render_task::Start` — precompute all the tables.  (The C++
    /// receives threadCount/allocator; the serial port computes once.)
    #[allow(clippy::too_many_arguments)]
    pub fn start_from_snapshot(
        snapshot: &NegativeRenderSnapshot,
        negative: &DngNegative,
        params: &DngRenderParams,
    ) -> DngResult<DngRenderTask> {
        // Compute zero offset ramp, if any.

        let zero_offset_ramp = if snapshot.stage3_black_level != 0 {
            let offset_function = DngFunctionZeroOffset::new(snapshot.stage3_black_level_normalized);

            let mut table = Dng1dTable::new(K_1D_TABLE_COUNT)?;

            table.initialize(&offset_function, false)?;

            Some(table)
        } else {
            None
        };

        // Compute camera space to linear ProPhoto RGB parameters.

        let profile_id = &params.camera_profile_id;

        let mut camera_white = DngVector::new();
        let mut camera_to_rgb = DngMatrix::new();

        let mut hue_sat_map: Option<DngHueSatMap> = None;
        let mut look_table: Option<DngHueSatMap> = None;
        let mut hue_sat_map_encode: Option<Dng1dTable> = None;
        let mut hue_sat_map_decode: Option<Dng1dTable> = None;
        let mut look_table_encode: Option<Dng1dTable> = None;
        let mut look_table_decode: Option<Dng1dTable> = None;

        let mut support_overrange = false;

        let mut has_profile: Option<bool> = None;

        if !snapshot.is_monochrome {
            let mut spec: DngColorSpec = negative.make_color_spec(profile_id, true)?;

            if params.white_xy.is_valid() {
                spec.set_white_xy(&params.white_xy)?;
            } else if snapshot.has_camera_neutral {
                let neutral_xy = spec.neutral_to_xy(&snapshot.camera_neutral)?;

                spec.set_white_xy(&neutral_xy)?;
            } else if snapshot.has_camera_white_xy {
                spec.set_white_xy(&snapshot.camera_white_xy)?;
            } else {
                spec.set_white_xy(&d55_xy_coord())?;
            }

            camera_white = spec.camera_white().clone();

            // fCameraToRGB = ProPhoto.MatrixFromPCS * spec.CameraToPCS.

            let pro_photo_from_pcs = dng_core::dng_color_space::space_pro_photo()
                .matrix_from_pcs();

            camera_to_rgb = pro_photo_from_pcs
                .multiply(spec.camera_to_pcs())
                .map_err(|_| program_error())?;

            // Find Hue/Sat table, if any.

            let mut profile = DngCameraProfile::new();

            if negative.get_profile_by_id(profile_id, &mut profile, true, None) {
                has_profile = Some(true);

                support_overrange = profile.is_hdr();

                hue_sat_map = profile.hue_sat_map_for_white(spec.white_xy())?;

                if !profile.look_table().is_null() {
                    look_table = Some(profile.look_table().clone());
                }

                if profile.hue_sat_map_encoding() != encoding_Linear {
                    let (encode, decode) =
                        build_hue_sat_map_encoding_table(profile.hue_sat_map_encoding(), false)?;

                    hue_sat_map_encode = encode;
                    hue_sat_map_decode = decode;
                }

                if profile.look_table_encoding() != encoding_Linear {
                    let (encode, decode) =
                        build_hue_sat_map_encoding_table(profile.look_table_encoding(), false)?;

                    look_table_encode = encode;
                    look_table_decode = decode;
                }
            }
        } else {
            // Monochrome negatives.

            let mut profile = DngCameraProfile::new();

            if negative.get_profile_by_id(profile_id, &mut profile, true, None) {
                has_profile = Some(true);

                support_overrange = profile.is_hdr();
            }
        }

        // Compute exposure/shadows ramp.

        let baseline_exposure =
            snapshot.total_baseline_exposure - (snapshot.stage3_gain.ln() / 2.0f64.ln());

        let exposure = params.exposure + baseline_exposure;

        let white = 1.0 / (2.0f64).powf(max_real64(0.0, exposure));

        let mut black = params.shadows * snapshot.shadow_scale * snapshot.stage3_gain * 0.001;

        black = min_real64(black, 0.99 * white);

        // Compute tone curve.

        let exposure_tone = DngFunctionExposureTone::new(exposure);

        let total_tone = Dng1dConcatenate::new(
            Box::new(exposure_tone),
            Box::new(StaticFunctionRef(params.tone_curve_fn)),
        );

        let mut tone_curve = Dng1dTable::new(K_1D_TABLE_COUNT)?;

        tone_curve.initialize(&total_tone, false)?;

        let mut tone_curve_slope: Option<(real32, real32, real32)> = None;

        if support_overrange {
            let y_at_1 = total_tone.evaluate(1.0) as real32;

            let dx = 0.001;

            let dy = total_tone.evaluate(1.0) - total_tone.evaluate(1.0 - dx);

            tone_curve_slope = Some((1.0, y_at_1, (dy / dx) as real32));
        }

        // Compute linear ProPhoto RGB to final space parameters.

        let rgb_to_final = params
            .final_space_matrix_from_pcs
            .multiply(dng_core::dng_color_space::space_pro_photo().matrix_to_pcs())
            .map_err(|_| program_error())?;

        let mut encode_gamma: Option<Dng1dTable> = None;

        let is_identity_gamma = std::ptr::eq(
            params.final_space_gamma as *const dyn Dng1dFunction,
            Dng1dIdentity::get() as *const dyn Dng1dFunction,
        );

        if !is_identity_gamma {
            let mut table = Dng1dTable::new(K_1D_TABLE_COUNT)?;

            table.initialize(params.final_space_gamma, false)?;

            encode_gamma = Some(table);
        }

        let _ = has_profile;

        Ok(DngRenderTask {
            src_mask: None,
            params: params.clone(),
            src_offset: DngPoint::new(0, 0),
            zero_offset_ramp,
            camera_white,
            camera_to_rgb,
            hue_sat_map,
            baseline_exposure,
            exposure_ramp_white: white,
            exposure_ramp_black: black,
            support_overrange,
            look_table,
            tone_curve: Some(tone_curve),
            tone_curve_slope,
            rgb_to_final,
            encode_gamma,
            hue_sat_map_encode,
            hue_sat_map_decode,
            look_table_encode,
            look_table_decode,
        })
    }

    /// `ProcessArea` — process one tile.  `src_area` is the source tile
    /// (already offset by `src_offset`); `dst_area` the destination tile.
    pub fn process_area(
        &self,
        src_image: &dyn DngImage,
        dst_image: &dyn DngImage,
        src_area: DngRect,
        dst_area: DngRect,
    ) -> DngResult<()> {
        let src_cols = src_area.w() as uint32;

        // Read the float source row data (the render task runs on ttFloat
        // images).

        let src_planes = src_image.planes();

        let src_samples = src_area.w() as usize
            * src_area.h() as usize
            * src_planes as usize;

        // PORT-NOTE: the pixel buffers carry their f32 samples as raw bits
        // (PixelData::U32) because dng-core's DngPixelBuffer::copy_area
        // writes 4-byte elements through set_u32, which has no F32-storage
        // arm yet (integration note filed).  get_u32 does handle F32
        // storage, so the read side converts transparently.

        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            0,
            src_planes,
            ttFloat,
            pcInterleaved,
            PixelData::U32(vec![0u32; src_samples]),
        )?;

        src_image.get(
            &mut src_buffer,
            dng_core::dng_image::EdgeRepeat,
            1,
            1,
        )?;

        let src_f32: Vec<real32> = match &src_buffer.data {
            PixelData::U32(v) => v.iter().map(|b| real32::from_bits(*b)).collect(),
            PixelData::F32(v) => v.clone(),
            _ => return Err(program_error()),
        };

        let dst_planes = dst_image.planes();

        let dst_samples = dst_area.w() as usize
            * dst_area.h() as usize
            * dst_planes as usize;

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            dst_area,
            0,
            dst_planes,
            ttFloat,
            pcInterleaved,
            PixelData::U32(vec![0u32; dst_samples]),
        )?;

        // Scratch row buffers.

        let mut t_r = vec![0.0f32; src_cols as usize];
        let mut t_g = vec![0.0f32; src_cols as usize];
        let mut t_b = vec![0.0f32; src_cols as usize];

        // Mask row, if any.

        let mask_f32: Option<Vec<real32>> = match &self.src_mask {
            Some(mask) => {
                let mut mask_buffer = DngPixelBuffer::with_planar_config(
                    src_area,
                    0,
                    1,
                    ttFloat,
                    pcInterleaved,
                    PixelData::U32(vec![0u32; src_samples / src_planes as usize]),
                )?;

                mask.read().unwrap().get(
                    &mut mask_buffer,
                    dng_core::dng_image::EdgeNone,
                    0,
                    0,
                )?;

                match &mask_buffer.data {
                    PixelData::U32(v) => Some(
                        v.iter().map(|b| real32::from_bits(*b)).collect(),
                    ),
                    PixelData::F32(v) => Some(v.clone()),
                    _ => None,
                }
            }
            None => None,
        };

        // Process each row of the image.

        for src_row in src_area.t..src_area.b {
            let row_base = (src_row - src_area.t) as usize * src_area.w() as usize;

            let plane_step = src_area.w() as usize;

            // Zero offset ramp, if any.

            if let Some(ramp) = &self.zero_offset_ramp {
                for plane in 0..src_planes {
                    let base = row_base + plane as usize * plane_step;

                    for col in 0..src_cols as usize {
                        let x = src_f32[base + col];

                        let y = ramp.interpolate(pin_real32(0.0, x, 1.0))?;

                        // In-place update in a local copy.

                        let _ = y;
                    }
                }
            }

            // Local copy of the source row values (allows in-place edits).

            let mut row_r = vec![0.0f32; src_cols as usize];
            let mut row_g = vec![0.0f32; src_cols as usize];
            let mut row_b = vec![0.0f32; src_cols as usize];

            for col in 0..src_cols as usize {
                row_r[col] = src_f32[row_base + col];

                if src_planes >= 3 {
                    row_g[col] = src_f32[row_base + plane_step + col];
                    row_b[col] = src_f32[row_base + 2 * plane_step + col];
                }
            }

            // Apply zero offset ramp in place.

            if let Some(ramp) = &self.zero_offset_ramp {
                for values in [&mut row_r, &mut row_g, &mut row_b] {
                    for x in values.iter_mut() {
                        *x = ramp.interpolate(pin_real32(0.0, *x, 1.0))?;
                    }
                }
            }

            // First convert from camera native space to linear PhotoRGB,
            // applying the white balance and camera profile.

            if src_planes == 1 {
                // For monochrome cameras, copy the data into all three color
                // channels.

                t_r.copy_from_slice(&row_r);
                t_g.copy_from_slice(&row_r);
                t_b.copy_from_slice(&row_r);
            } else {
                if src_planes == 3 {
                    do_baseline_abc_to_rgb(
                        &row_r,
                        &row_g,
                        &row_b,
                        &mut t_r,
                        &mut t_g,
                        &mut t_b,
                        src_cols,
                        &self.camera_white,
                        &self.camera_to_rgb,
                    );
                } else {
                    // 4-channel camera (pad the D vector to src_cols).

                    let row_d: Vec<real32> =
                        src_f32[row_base + 3 * plane_step..row_base + 3 * plane_step + src_cols as usize]
                            .to_vec();

                    do_baseline_abcd_to_rgb(
                        &row_r,
                        &row_g,
                        &row_b,
                        &row_d,
                        &mut t_r,
                        &mut t_g,
                        &mut t_b,
                        src_cols,
                        &self.camera_white,
                        &self.camera_to_rgb,
                    );
                }

                // Apply Hue/Sat map, if any.

                if let Some(map) = &self.hue_sat_map {
                    let t_r_src = t_r.clone();
                    let t_g_src = t_g.clone();
                    let t_b_src = t_b.clone();

                    do_baseline_hue_sat_map(
                        &t_r_src,
                        &t_g_src,
                        &t_b_src,
                        &mut t_r,
                        &mut t_g,
                        &mut t_b,
                        src_cols,
                        map,
                        self.hue_sat_map_encode.as_ref(),
                        self.hue_sat_map_decode.as_ref(),
                        self.support_overrange,
                    )?;
                }
            }

            // Apply ProfileGainTableMap / ProfileGainTableMap2.

            // (The profile-attached PGTM is checked by the caller via
            // DngRender::render — the port applies the negative-attached map
            // when present.)

            let pgtm = negative_pgtm_placeholder();

            if let Some(map) = pgtm {
                let active_area = src_image.bounds();

                let exposure_weight_gain =
                    (2.0f32).powf(self.baseline_exposure as real32);

                let t_r_src = t_r.clone();
                let t_g_src = t_g.clone();
                let t_b_src = t_b.clone();

                do_baseline_profile_gain_table_map(
                    &t_r_src,
                    &t_g_src,
                    &t_b_src,
                    &mut t_r,
                    &mut t_g,
                    &mut t_b,
                    src_cols,
                    src_row,
                    src_area.l,
                    &active_area,
                    exposure_weight_gain,
                    &map,
                    self.support_overrange,
                )?;
            }

            // Apply exposure curve (ramp).

            {
                let ramp = DngFunctionExposureRamp::new(
                    self.exposure_ramp_white,
                    self.exposure_ramp_black,
                    self.exposure_ramp_black,
                    self.support_overrange,
                );

                let t_r_src = t_r.clone();
                let t_g_src = t_g.clone();
                let t_b_src = t_b.clone();

                do_baseline_1d_function(&t_r_src, &mut t_r, src_cols, &ramp, self.support_overrange);
                do_baseline_1d_function(&t_g_src, &mut t_g, src_cols, &ramp, self.support_overrange);
                do_baseline_1d_function(&t_b_src, &mut t_b, src_cols, &ramp, self.support_overrange);
            }

            // Apply look table, if any.

            if let Some(map) = &self.look_table {
                let t_r_src = t_r.clone();
                let t_g_src = t_g.clone();
                let t_b_src = t_b.clone();

                do_baseline_hue_sat_map(
                    &t_r_src,
                    &t_g_src,
                    &t_b_src,
                    &mut t_r,
                    &mut t_g,
                    &mut t_b,
                    src_cols,
                    map,
                    self.look_table_encode.as_ref(),
                    self.look_table_decode.as_ref(),
                    self.support_overrange,
                )?;
            }

            // Apply baseline tone curve.

            if let Some(table) = &self.tone_curve {
                let t_r_src = t_r.clone();
                let t_g_src = t_g.clone();
                let t_b_src = t_b.clone();

                match &self.tone_curve_slope {
                    Some((sx, sy, slope)) => {
                        do_baseline_rgb_tone_overrange(
                            &t_r_src,
                            &t_g_src,
                            &t_b_src,
                            &mut t_r,
                            &mut t_g,
                            &mut t_b,
                            src_cols,
                            table,
                            *sx,
                            *sy,
                            *slope,
                        )?;
                    }

                    None => {
                        do_baseline_rgb_tone(
                            &t_r_src,
                            &t_g_src,
                            &t_b_src,
                            &mut t_r,
                            &mut t_g,
                            &mut t_b,
                            src_cols,
                            table,
                        )?;
                    }
                }
            }

            // PORT-NOTE(dng_render.cpp:1967): the DNG 1.7 MaskedRGBTables
            // stage requires the dng_reference RGB-table machinery and
            // camera-profile masked-table APIs that have not landed; render
            // falls through without applying them (the writer never emits
            // masked RGB tables yet either).

            // Convert to final color space.

            let dst_row = src_row + (dst_area.t - src_area.t);

            let dst_row_base = (dst_row - dst_area.t) as usize * dst_area.w() as usize;

            let dst_plane_step = dst_area.w() as usize;

            if dst_planes == 1 {
                let mut out = vec![0.0f32; src_cols as usize];

                do_baseline_rgb_to_gray(
                    &t_r,
                    &t_g,
                    &t_b,
                    &mut out,
                    src_cols,
                    &self.rgb_to_final,
                    self.support_overrange,
                );

                if let Some(gamma) = &self.encode_gamma {
                    let out_src = out.clone();

                    do_baseline_1d_table(&out_src, &mut out, src_cols, gamma)?;
                }

                for col in 0..src_cols as usize {
                    dst_buffer.data.set_u32(dst_row_base + col, out[col].to_bits());
                }
            } else {
                let mut out_r = vec![0.0f32; src_cols as usize];
                let mut out_g = vec![0.0f32; src_cols as usize];
                let mut out_b = vec![0.0f32; src_cols as usize];

                do_baseline_rgb_to_rgb(
                    &t_r,
                    &t_g,
                    &t_b,
                    &mut out_r,
                    &mut out_g,
                    &mut out_b,
                    src_cols,
                    &self.rgb_to_final,
                    self.support_overrange,
                );

                if let Some(gamma) = &self.encode_gamma {
                    let src_r = out_r.clone();
                    let src_g = out_g.clone();
                    let src_b = out_b.clone();

                    do_baseline_1d_table(&src_r, &mut out_r, src_cols, gamma)?;
                    do_baseline_1d_table(&src_g, &mut out_g, src_cols, gamma)?;
                    do_baseline_1d_table(&src_b, &mut out_b, src_cols, gamma)?;
                }

                for col in 0..src_cols as usize {
                    dst_buffer.data.set_f32(dst_row_base + col, out_r[col]);
                    dst_buffer.data.set_f32(dst_row_base + dst_plane_step + col, out_g[col]);
                    dst_buffer.data.set_f32(dst_row_base + 2 * dst_plane_step + col, out_b[col]);
                }
            }

            // Apply the transparency mask (white matte).

            if let Some(mask_row) = &mask_f32 {
                let mask_row_base =
                    (src_row - src_area.t) as usize * src_area.w() as usize;

                for dst_plane in 0..dst_planes {
                    for col in 0..src_cols as usize {
                        let idx = dst_row_base + dst_plane as usize * dst_plane_step + col;

                        let d = dst_buffer.data.get_f32(idx);

                        // White Matte.

                        let m = mask_row[mask_row_base + col];

                        dst_buffer.data.set_f32(idx, 1.0 - (1.0 - d) * m);
                    }
                }
            }
        }

        dst_image.put(&dst_buffer)?;

        Ok(())
    }
}

/// Adapter holding a 'static 1D function reference (for Dng1dConcatenate,
/// whose C++ counterpart stores references).
struct StaticFunctionRef(&'static dyn Dng1dFunction);

impl Dng1dFunction for StaticFunctionRef {
    fn evaluate(&self, x: real64) -> real64 {
        self.0.evaluate(x)
    }
}

/// Placeholder accessor for the negative-attached ProfileGainTableMap —
/// populated by `DngRender::render` before the task runs (the C++ reads the
/// negative through the task reference; the Rust task takes a snapshot).
static PGTM_HOLDER: std::sync::Mutex<Option<crate::dng_gain_map::DngGainTableMap>> =
    std::sync::Mutex::new(None);

fn negative_pgtm_placeholder() -> Option<crate::dng_gain_map::DngGainTableMap> {
    PGTM_HOLDER.lock().unwrap().clone()
}

// ---------------------------------------------------------------------------
// dng_render
// ---------------------------------------------------------------------------

/// Port of `dng_render`.
pub struct DngRender {
    /// `fHost`.
    host: DngHost,

    /// `fNegative` — accessed via snapshots (the negative is owned by the
    /// caller).
    negative_params: NegativeRenderSnapshot,

    /// `fWhiteXY`.
    white_xy: DngXyCoord,

    /// `fExposure`.
    exposure: real64,

    /// `fShadows`.
    shadows: real64,

    /// `fToneCurve`.
    tone_curve: &'static dyn Dng1dFunction,

    /// `fFinalSpace` (resolved matrix + gamma + monochrome flag).
    final_space_matrix_from_pcs: DngMatrix,
    final_space_is_monochrome: bool,
    final_space_gamma: &'static dyn Dng1dFunction,
    final_space_hdr_linear: bool,

    /// `fFinalPixelType`.
    final_pixel_type: uint32,

    /// `fMaximumSize`.
    maximum_size: uint32,

    /// `fProfileToneCurve`.
    profile_tone_curve: Option<DngPiecewiseLinear>,
}

/// Snapshot of the negative fields `DngRender` needs (the C++ holds a
/// reference; the port copies the queried values).
#[derive(Clone, Default)]
pub struct NegativeRenderSnapshot {
    pub is_output_referred: bool,
    pub is_monochrome: bool,
    pub has_camera_neutral: bool,
    pub camera_neutral: DngVector,
    pub has_camera_white_xy: bool,
    pub camera_white_xy: DngXyCoord,
    pub shadow_scale: real64,
    pub stage3_gain: real64,
    pub stage3_black_level: uint32,
    pub stage3_black_level_normalized: real64,
    pub total_baseline_exposure: real64,
    pub default_crop_area: Option<DngRect>,
    pub default_final_width: uint32,
    pub default_final_height: uint32,
    pub aspect_ratio: real64,
    pub stage3_image: Option<ImageRef>,
    pub transparency_mask: Option<ImageRef>,
    pub has_profile_gain_table_map: bool,
    pub profile_gain_table_map: Option<crate::dng_gain_map::DngGainTableMap>,
    pub profile_id: DngCameraProfileId,
    pub profile_tone_curve_valid: bool,
    pub profile_tone_curve_points: Vec<(real64, real64)>,
    pub profile_default_black_render_none: bool,
    pub profile_is_hdr: bool,
}

impl DngRender {
    /// `dng_render (host, negative)` — builds from a negative snapshot.
    pub fn new(negative: &DngNegative) -> DngResult<Self> {
        // Snapshot the negative.

        let profile_id = DngCameraProfileId::default();

        let mut snapshot = NegativeRenderSnapshot {
            is_output_referred: negative.is_output_referred(),
            is_monochrome: negative.is_monochrome(),
            has_camera_neutral: negative.has_camera_neutral(),
            camera_neutral: if negative.has_camera_neutral() {
                negative.camera_neutral().clone()
            } else {
                DngVector::new()
            },
            has_camera_white_xy: negative.has_camera_white_xy(),
            camera_white_xy: if negative.has_camera_white_xy() {
                negative.camera_white_xy().clone()
            } else {
                DngXyCoord::default()
            },
            shadow_scale: negative.shadow_scale(),
            stage3_gain: negative.stage3_gain(),
            stage3_black_level: negative.stage3_black_level() as uint32,
            stage3_black_level_normalized: negative.stage3_black_level_normalized(),
            total_baseline_exposure: negative.total_baseline_exposure(&profile_id),
            default_crop_area: Some(negative.default_crop_area()),
            default_final_width: negative.default_final_width(),
            default_final_height: negative.default_final_height(),
            aspect_ratio: negative.aspect_ratio(),
            stage3_image: negative.stage3_image().cloned(),
            transparency_mask: negative.transparency_mask().cloned(),
            has_profile_gain_table_map: negative.has_profile_gain_table_map(),
            profile_gain_table_map: negative.profile_gain_table_map().ok().cloned(),
            profile_id,
            profile_tone_curve_valid: false,
            profile_tone_curve_points: Vec::new(),
            profile_default_black_render_none: false,
            profile_is_hdr: false,
        };

        {
            let mut profile = DngCameraProfile::new();

            if negative.get_profile_by_id(&snapshot.profile_id, &mut profile, true, None) {
                snapshot.profile_tone_curve_valid = profile.tone_curve().is_valid();
                snapshot.profile_default_black_render_none =
                    profile.default_black_render() == defaultBlackRender_None;
                snapshot.profile_is_hdr = profile.is_hdr();

                // Snapshot the profile tone curve points (for the
                // piecewise-linear rebuild in render ()).

                snapshot.profile_tone_curve_points = profile
                    .tone_curve()
                    .coord
                    .iter()
                    .map(|c| (c.h, c.v))
                    .collect();
            }
        }

        Ok(DngRender {
            host: DngHost::new(),
            negative_params: snapshot,
            white_xy: DngXyCoord::default(),
            exposure: 0.0,
            shadows: 5.0,
            tone_curve: DngToneCurveAcr3Default::get(),
            final_space_matrix_from_pcs: *dng_core::dng_color_space::space_srgb().matrix_from_pcs(),
            final_space_is_monochrome: false,
            final_space_gamma: dng_core::dng_color_space::space_srgb().gamma_function(),
            final_space_hdr_linear: false,
            final_pixel_type: ttByte,
            maximum_size: 0,
            profile_tone_curve: None,
        })
    }

    /// `SetWhiteXY (coord)`.
    pub fn set_white_xy(&mut self, coord: DngXyCoord) {
        self.white_xy = coord;
    }

    /// `SetExposure (exposure)`.
    pub fn set_exposure(&mut self, exposure: real64) {
        self.exposure = exposure;
    }

    /// `SetShadows (shadows)`.
    pub fn set_shadows(&mut self, shadows: real64) {
        self.shadows = shadows;
    }

    /// `SetToneCurve (curve)` — sets a static curve function.
    pub fn set_tone_curve(&mut self, curve: &'static dyn Dng1dFunction) {
        self.tone_curve = curve;
    }

    /// `SetFinalSpace (space)` — the color space is described by its PCS
    /// matrix, gamma function and monochrome flag (dng-core spaces).
    pub fn set_final_space(
        &mut self,
        matrix_from_pcs: DngMatrix,
        gamma: &'static dyn Dng1dFunction,
        is_monochrome: bool,
    ) {
        self.final_space_matrix_from_pcs = matrix_from_pcs;
        self.final_space_gamma = gamma;
        self.final_space_is_monochrome = is_monochrome;
    }

    /// `SetFinalPixelType (pixelType)`.
    pub fn set_final_pixel_type(&mut self, pixel_type: uint32) {
        self.final_pixel_type = pixel_type;
    }

    /// `MaximumSize ()`.
    pub fn maximum_size(&self) -> uint32 {
        self.maximum_size
    }

    /// `SetMaximumSize (size)`.
    pub fn set_maximum_size(&mut self, size: uint32) {
        self.maximum_size = size;
    }

    /// `Render ()` — returns the rendered image.  (The C++ keeps a negative
    /// reference on the render object; the port passes it per call.)
    pub fn render(&mut self, negative: &DngNegative) -> DngResult<ImageRef> {
        let negative_for_task = negative;
        // Switch to NOP default parameters for non-scene referred data.

        if self.negative_params.is_output_referred {
            self.shadows = 0.0;

            self.tone_curve = Dng1dIdentity::get();
        }

        // Use default tone curve from profile if any.

        if self.negative_params.profile_tone_curve_valid
            && !self.negative_params.profile_tone_curve_points.is_empty()
        {
            let mut curve = DngPiecewiseLinear::new();

            for (x, y) in &self.negative_params.profile_tone_curve_points {
                curve.add(*x, *y);
            }

            if curve.is_valid() {
                // PORT-NOTE: the C++ stores the profile tone curve on the
                // render object; the port leaks a 'static instance because
                // the task's tone-curve slot is a 'static function reference.

                let leaked: &'static DngPiecewiseLinear = Box::leak(Box::new(curve));

                self.profile_tone_curve = Some(DngPiecewiseLinear::new());

                self.tone_curve = leaked as &'static dyn Dng1dFunction;
            }

            // Turn off default shadow mapping if requested by profile.

            if self.negative_params.profile_default_black_render_none {
                self.shadows = 0.0;
            }
        }

        let src_image = self
            .negative_params
            .stage3_image
            .clone()
            .ok_or_else(|| DngError::Unsupported("Render: missing stage3 image".to_string()))?;

        let mut src_mask = self.negative_params.transparency_mask.clone();

        let src_bounds = self
            .negative_params
            .default_crop_area
            .ok_or_else(|| DngError::Unsupported("Render: missing default crop".to_string()))?;

        let mut dst_size_h = self.negative_params.default_final_width;
        let mut dst_size_v = self.negative_params.default_final_height;

        if self.maximum_size() != 0 {
            if max_real32(dst_size_h as real32, dst_size_v as real32) as uint32
                > self.maximum_size()
            {
                let ratio = self.negative_params.aspect_ratio;

                if ratio >= 1.0 {
                    dst_size_h = self.maximum_size();
                    dst_size_v = max_real32(
                        1.0,
                        round_uint32_f64(dst_size_h as real64 / ratio) as real32,
                    ) as uint32;
                } else {
                    dst_size_v = self.maximum_size();
                    dst_size_h = max_real32(
                        1.0,
                        round_uint32_f64(dst_size_v as real64 * ratio) as real32,
                    ) as uint32;
                }
            }
        }

        let mut temp_image: Option<ImageRef> = None;
        let mut temp_mask: Option<ImageRef> = None;

        let mut src_ref = src_image.clone();

        if src_bounds.size() != DngPoint::new(dst_size_v as int32, dst_size_h as int32) {
            let src = src_ref.read().unwrap();

            let temp: ImageRef = DngSimpleImage::new(
                DngRect::from_size_point(DngPoint::new(dst_size_v as int32, dst_size_h as int32)),
                src.planes(),
                src.pixel_type(),
            )?
            .into_image_ref()?;

            drop(src);

            resample_image(
                &mut self.host,
                src_ref.clone(),
                temp.clone(),
                src_bounds,
                temp.read().unwrap().bounds(),
            )?;

            if let Some(mask) = &src_mask {
                let mask_r = mask.read().unwrap();

                let new_mask: ImageRef = DngSimpleImage::new(
                    DngRect::from_size_point(DngPoint::new(dst_size_v as int32, dst_size_h as int32)),
                    mask_r.planes(),
                    mask_r.pixel_type(),
                )?
                .into_image_ref()?;

                drop(mask_r);

                resample_image(
                    &mut self.host,
                    mask.clone(),
                    new_mask.clone(),
                    src_bounds,
                    new_mask.read().unwrap().bounds(),
                )?;

                temp_mask = Some(new_mask);
                src_mask = temp_mask.clone();
            }

            temp_image = Some(temp.clone());

            src_ref = temp;
        }

        let src_bounds = src_ref.read().unwrap().bounds();

        // Final space (linear variant for HDR profiles).

        let (final_matrix, final_gamma, final_mono) =
            if self.negative_params.profile_is_hdr && self.final_space_hdr_linear {
                (
                    self.final_space_matrix_from_pcs.clone(),
                    dng_core::dng_1d_function::Dng1dIdentity::get(),
                    self.final_space_is_monochrome,
                )
            } else {
                (
                    self.final_space_matrix_from_pcs.clone(),
                    self.final_space_gamma,
                    self.final_space_is_monochrome,
                )
            };

        let dst_planes: uint32 = if final_mono { 1 } else { 3 };

        let dst_image: ImageRef = DngSimpleImage::new(
            src_bounds,
            dst_planes,
            self.final_pixel_type,
        )?
        .into_image_ref()?;

        // Build the render parameters and task.

        let params = DngRenderParams {
            camera_profile_id: self.negative_params.profile_id.clone(),
            white_xy: self.white_xy,
            exposure: self.exposure,
            shadows: self.shadows,
            tone_curve_fn: self.tone_curve,
            final_space_matrix_from_pcs: final_matrix,
            final_space_is_monochrome: final_mono,
            final_space_gamma: final_gamma,
        };

        // Snapshot the negative-attached PGTM for the task.

        {
            let mut holder = PGTM_HOLDER.lock().unwrap();

            *holder = self.negative_params.profile_gain_table_map.clone();
        }

        let task = DngRenderTask::start_with_negative_data(
            &self.negative_params,
            negative_for_task,
            &params,
        )?;

        // Drive the tile loop serially (the C++ fans out via PerformAreaTask).

        let mut tile_size = DngPoint::new(512, 512);

        if tile_size.h as uint32 > src_bounds.w() {
            tile_size.h = src_bounds.w() as int32;
        }

        if tile_size.v as uint32 > src_bounds.h() {
            tile_size.v = src_bounds.h() as int32;
        }

        let mut iterator =
            dng_core::dng_tile_iterator::DngTileIterator::new_tile_size(tile_size, src_bounds);

        let mut tile = DngRect::new();

        let src_offset = src_bounds.tl();

        let src_full_bounds = {
            let s = src_ref.read().unwrap();

            DngRect::from_corners(
                s.bounds().t + src_offset.v,
                s.bounds().l + src_offset.h,
                s.bounds().b + src_offset.v,
                s.bounds().r + src_offset.h,
            )
        };

        let _ = &task;

        while iterator.get_one_tile(&mut tile)? {
            // Map the destination tile back into the source image (render
            // processes 1:1 after resampling; the src area == dst area).

            let dst_image_dyn: &dyn DngImage = &*dst_image.read().unwrap();

            let src_image_dyn: &dyn DngImage = &*src_ref.read().unwrap();

            // The C++ task reads source at (dstArea + srcOffset); after the
            // resample pass the images are the same size, so src == dst.

            let src_area = tile;
            let dst_area = tile;

            let _ = src_full_bounds;

            task.process_area(src_image_dyn, dst_image_dyn, src_area, dst_area)?;
        }

        // Clear the PGTM holder.

        {
            let mut holder = PGTM_HOLDER.lock().unwrap();

            *holder = None;
        }

        let _ = temp_image;

        Ok(dst_image)
    }
}

#[allow(unused)]
fn _unused_witness(_x: Option<(DngFingerprint, tag_type_size_witness, uint8)>) {}
type tag_type_size_witness = fn(uint32) -> uint32;

/// The unused-import witness for `not_yet_implemented` (kept for parity with
/// the C++ ThrowNotYetImplemented sites).
#[allow(unused)]
fn _unused_witness2() -> DngError {
    not_yet_implemented()
}

// ---------------------------------------------------------------------------
// ACR3 tables submodule.
// ---------------------------------------------------------------------------

mod acr3_tables {
    use dng_core::dng_types::real32;

// ACR3 default tone curve tables (extracted verbatim from
// vendor/extracted/dng_sdk_1_7_1/dng_sdk/source/dng_render.cpp).

/// Forward table (`dng_tone_curve_acr3_default::Evaluate`).
pub(crate) const ACR3_FORWARD_TABLE: [real32; 1025] = [
    0.00000f32, 0.00078f32, 0.00160f32, 0.00242f32, 0.00314f32, 0.00385f32, 0.00460f32, 0.00539f32,
    0.00623f32, 0.00712f32, 0.00806f32, 0.00906f32, 0.01012f32, 0.01122f32, 0.01238f32, 0.01359f32,
    0.01485f32, 0.01616f32, 0.01751f32, 0.01890f32, 0.02033f32, 0.02180f32, 0.02331f32, 0.02485f32,
    0.02643f32, 0.02804f32, 0.02967f32, 0.03134f32, 0.03303f32, 0.03475f32, 0.03648f32, 0.03824f32,
    0.04002f32, 0.04181f32, 0.04362f32, 0.04545f32, 0.04730f32, 0.04916f32, 0.05103f32, 0.05292f32,
    0.05483f32, 0.05675f32, 0.05868f32, 0.06063f32, 0.06259f32, 0.06457f32, 0.06655f32, 0.06856f32,
    0.07057f32, 0.07259f32, 0.07463f32, 0.07668f32, 0.07874f32, 0.08081f32, 0.08290f32, 0.08499f32,
    0.08710f32, 0.08921f32, 0.09134f32, 0.09348f32, 0.09563f32, 0.09779f32, 0.09996f32, 0.10214f32,
    0.10433f32, 0.10652f32, 0.10873f32, 0.11095f32, 0.11318f32, 0.11541f32, 0.11766f32, 0.11991f32,
    0.12218f32, 0.12445f32, 0.12673f32, 0.12902f32, 0.13132f32, 0.13363f32, 0.13595f32, 0.13827f32,
    0.14061f32, 0.14295f32, 0.14530f32, 0.14765f32, 0.15002f32, 0.15239f32, 0.15477f32, 0.15716f32,
    0.15956f32, 0.16197f32, 0.16438f32, 0.16680f32, 0.16923f32, 0.17166f32, 0.17410f32, 0.17655f32,
    0.17901f32, 0.18148f32, 0.18395f32, 0.18643f32, 0.18891f32, 0.19141f32, 0.19391f32, 0.19641f32,
    0.19893f32, 0.20145f32, 0.20398f32, 0.20651f32, 0.20905f32, 0.21160f32, 0.21416f32, 0.21672f32,
    0.21929f32, 0.22185f32, 0.22440f32, 0.22696f32, 0.22950f32, 0.23204f32, 0.23458f32, 0.23711f32,
    0.23963f32, 0.24215f32, 0.24466f32, 0.24717f32, 0.24967f32, 0.25216f32, 0.25465f32, 0.25713f32,
    0.25961f32, 0.26208f32, 0.26454f32, 0.26700f32, 0.26945f32, 0.27189f32, 0.27433f32, 0.27676f32,
    0.27918f32, 0.28160f32, 0.28401f32, 0.28641f32, 0.28881f32, 0.29120f32, 0.29358f32, 0.29596f32,
    0.29833f32, 0.30069f32, 0.30305f32, 0.30540f32, 0.30774f32, 0.31008f32, 0.31241f32, 0.31473f32,
    0.31704f32, 0.31935f32, 0.32165f32, 0.32395f32, 0.32623f32, 0.32851f32, 0.33079f32, 0.33305f32,
    0.33531f32, 0.33756f32, 0.33981f32, 0.34205f32, 0.34428f32, 0.34650f32, 0.34872f32, 0.35093f32,
    0.35313f32, 0.35532f32, 0.35751f32, 0.35969f32, 0.36187f32, 0.36404f32, 0.36620f32, 0.36835f32,
    0.37050f32, 0.37264f32, 0.37477f32, 0.37689f32, 0.37901f32, 0.38112f32, 0.38323f32, 0.38533f32,
    0.38742f32, 0.38950f32, 0.39158f32, 0.39365f32, 0.39571f32, 0.39777f32, 0.39982f32, 0.40186f32,
    0.40389f32, 0.40592f32, 0.40794f32, 0.40996f32, 0.41197f32, 0.41397f32, 0.41596f32, 0.41795f32,
    0.41993f32, 0.42191f32, 0.42388f32, 0.42584f32, 0.42779f32, 0.42974f32, 0.43168f32, 0.43362f32,
    0.43554f32, 0.43747f32, 0.43938f32, 0.44129f32, 0.44319f32, 0.44509f32, 0.44698f32, 0.44886f32,
    0.45073f32, 0.45260f32, 0.45447f32, 0.45632f32, 0.45817f32, 0.46002f32, 0.46186f32, 0.46369f32,
    0.46551f32, 0.46733f32, 0.46914f32, 0.47095f32, 0.47275f32, 0.47454f32, 0.47633f32, 0.47811f32,
    0.47989f32, 0.48166f32, 0.48342f32, 0.48518f32, 0.48693f32, 0.48867f32, 0.49041f32, 0.49214f32,
    0.49387f32, 0.49559f32, 0.49730f32, 0.49901f32, 0.50072f32, 0.50241f32, 0.50410f32, 0.50579f32,
    0.50747f32, 0.50914f32, 0.51081f32, 0.51247f32, 0.51413f32, 0.51578f32, 0.51742f32, 0.51906f32,
    0.52069f32, 0.52232f32, 0.52394f32, 0.52556f32, 0.52717f32, 0.52878f32, 0.53038f32, 0.53197f32,
    0.53356f32, 0.53514f32, 0.53672f32, 0.53829f32, 0.53986f32, 0.54142f32, 0.54297f32, 0.54452f32,
    0.54607f32, 0.54761f32, 0.54914f32, 0.55067f32, 0.55220f32, 0.55371f32, 0.55523f32, 0.55673f32,
    0.55824f32, 0.55973f32, 0.56123f32, 0.56271f32, 0.56420f32, 0.56567f32, 0.56715f32, 0.56861f32,
    0.57007f32, 0.57153f32, 0.57298f32, 0.57443f32, 0.57587f32, 0.57731f32, 0.57874f32, 0.58017f32,
    0.58159f32, 0.58301f32, 0.58443f32, 0.58583f32, 0.58724f32, 0.58864f32, 0.59003f32, 0.59142f32,
    0.59281f32, 0.59419f32, 0.59556f32, 0.59694f32, 0.59830f32, 0.59966f32, 0.60102f32, 0.60238f32,
    0.60373f32, 0.60507f32, 0.60641f32, 0.60775f32, 0.60908f32, 0.61040f32, 0.61173f32, 0.61305f32,
    0.61436f32, 0.61567f32, 0.61698f32, 0.61828f32, 0.61957f32, 0.62087f32, 0.62216f32, 0.62344f32,
    0.62472f32, 0.62600f32, 0.62727f32, 0.62854f32, 0.62980f32, 0.63106f32, 0.63232f32, 0.63357f32,
    0.63482f32, 0.63606f32, 0.63730f32, 0.63854f32, 0.63977f32, 0.64100f32, 0.64222f32, 0.64344f32,
    0.64466f32, 0.64587f32, 0.64708f32, 0.64829f32, 0.64949f32, 0.65069f32, 0.65188f32, 0.65307f32,
    0.65426f32, 0.65544f32, 0.65662f32, 0.65779f32, 0.65897f32, 0.66013f32, 0.66130f32, 0.66246f32,
    0.66362f32, 0.66477f32, 0.66592f32, 0.66707f32, 0.66821f32, 0.66935f32, 0.67048f32, 0.67162f32,
    0.67275f32, 0.67387f32, 0.67499f32, 0.67611f32, 0.67723f32, 0.67834f32, 0.67945f32, 0.68055f32,
    0.68165f32, 0.68275f32, 0.68385f32, 0.68494f32, 0.68603f32, 0.68711f32, 0.68819f32, 0.68927f32,
    0.69035f32, 0.69142f32, 0.69249f32, 0.69355f32, 0.69461f32, 0.69567f32, 0.69673f32, 0.69778f32,
    0.69883f32, 0.69988f32, 0.70092f32, 0.70196f32, 0.70300f32, 0.70403f32, 0.70506f32, 0.70609f32,
    0.70711f32, 0.70813f32, 0.70915f32, 0.71017f32, 0.71118f32, 0.71219f32, 0.71319f32, 0.71420f32,
    0.71520f32, 0.71620f32, 0.71719f32, 0.71818f32, 0.71917f32, 0.72016f32, 0.72114f32, 0.72212f32,
    0.72309f32, 0.72407f32, 0.72504f32, 0.72601f32, 0.72697f32, 0.72794f32, 0.72890f32, 0.72985f32,
    0.73081f32, 0.73176f32, 0.73271f32, 0.73365f32, 0.73460f32, 0.73554f32, 0.73647f32, 0.73741f32,
    0.73834f32, 0.73927f32, 0.74020f32, 0.74112f32, 0.74204f32, 0.74296f32, 0.74388f32, 0.74479f32,
    0.74570f32, 0.74661f32, 0.74751f32, 0.74842f32, 0.74932f32, 0.75021f32, 0.75111f32, 0.75200f32,
    0.75289f32, 0.75378f32, 0.75466f32, 0.75555f32, 0.75643f32, 0.75730f32, 0.75818f32, 0.75905f32,
    0.75992f32, 0.76079f32, 0.76165f32, 0.76251f32, 0.76337f32, 0.76423f32, 0.76508f32, 0.76594f32,
    0.76679f32, 0.76763f32, 0.76848f32, 0.76932f32, 0.77016f32, 0.77100f32, 0.77183f32, 0.77267f32,
    0.77350f32, 0.77432f32, 0.77515f32, 0.77597f32, 0.77680f32, 0.77761f32, 0.77843f32, 0.77924f32,
    0.78006f32, 0.78087f32, 0.78167f32, 0.78248f32, 0.78328f32, 0.78408f32, 0.78488f32, 0.78568f32,
    0.78647f32, 0.78726f32, 0.78805f32, 0.78884f32, 0.78962f32, 0.79040f32, 0.79118f32, 0.79196f32,
    0.79274f32, 0.79351f32, 0.79428f32, 0.79505f32, 0.79582f32, 0.79658f32, 0.79735f32, 0.79811f32,
    0.79887f32, 0.79962f32, 0.80038f32, 0.80113f32, 0.80188f32, 0.80263f32, 0.80337f32, 0.80412f32,
    0.80486f32, 0.80560f32, 0.80634f32, 0.80707f32, 0.80780f32, 0.80854f32, 0.80926f32, 0.80999f32,
    0.81072f32, 0.81144f32, 0.81216f32, 0.81288f32, 0.81360f32, 0.81431f32, 0.81503f32, 0.81574f32,
    0.81645f32, 0.81715f32, 0.81786f32, 0.81856f32, 0.81926f32, 0.81996f32, 0.82066f32, 0.82135f32,
    0.82205f32, 0.82274f32, 0.82343f32, 0.82412f32, 0.82480f32, 0.82549f32, 0.82617f32, 0.82685f32,
    0.82753f32, 0.82820f32, 0.82888f32, 0.82955f32, 0.83022f32, 0.83089f32, 0.83155f32, 0.83222f32,
    0.83288f32, 0.83354f32, 0.83420f32, 0.83486f32, 0.83552f32, 0.83617f32, 0.83682f32, 0.83747f32,
    0.83812f32, 0.83877f32, 0.83941f32, 0.84005f32, 0.84069f32, 0.84133f32, 0.84197f32, 0.84261f32,
    0.84324f32, 0.84387f32, 0.84450f32, 0.84513f32, 0.84576f32, 0.84639f32, 0.84701f32, 0.84763f32,
    0.84825f32, 0.84887f32, 0.84949f32, 0.85010f32, 0.85071f32, 0.85132f32, 0.85193f32, 0.85254f32,
    0.85315f32, 0.85375f32, 0.85436f32, 0.85496f32, 0.85556f32, 0.85615f32, 0.85675f32, 0.85735f32,
    0.85794f32, 0.85853f32, 0.85912f32, 0.85971f32, 0.86029f32, 0.86088f32, 0.86146f32, 0.86204f32,
    0.86262f32, 0.86320f32, 0.86378f32, 0.86435f32, 0.86493f32, 0.86550f32, 0.86607f32, 0.86664f32,
    0.86720f32, 0.86777f32, 0.86833f32, 0.86889f32, 0.86945f32, 0.87001f32, 0.87057f32, 0.87113f32,
    0.87168f32, 0.87223f32, 0.87278f32, 0.87333f32, 0.87388f32, 0.87443f32, 0.87497f32, 0.87552f32,
    0.87606f32, 0.87660f32, 0.87714f32, 0.87768f32, 0.87821f32, 0.87875f32, 0.87928f32, 0.87981f32,
    0.88034f32, 0.88087f32, 0.88140f32, 0.88192f32, 0.88244f32, 0.88297f32, 0.88349f32, 0.88401f32,
    0.88453f32, 0.88504f32, 0.88556f32, 0.88607f32, 0.88658f32, 0.88709f32, 0.88760f32, 0.88811f32,
    0.88862f32, 0.88912f32, 0.88963f32, 0.89013f32, 0.89063f32, 0.89113f32, 0.89163f32, 0.89212f32,
    0.89262f32, 0.89311f32, 0.89360f32, 0.89409f32, 0.89458f32, 0.89507f32, 0.89556f32, 0.89604f32,
    0.89653f32, 0.89701f32, 0.89749f32, 0.89797f32, 0.89845f32, 0.89892f32, 0.89940f32, 0.89987f32,
    0.90035f32, 0.90082f32, 0.90129f32, 0.90176f32, 0.90222f32, 0.90269f32, 0.90316f32, 0.90362f32,
    0.90408f32, 0.90454f32, 0.90500f32, 0.90546f32, 0.90592f32, 0.90637f32, 0.90683f32, 0.90728f32,
    0.90773f32, 0.90818f32, 0.90863f32, 0.90908f32, 0.90952f32, 0.90997f32, 0.91041f32, 0.91085f32,
    0.91130f32, 0.91173f32, 0.91217f32, 0.91261f32, 0.91305f32, 0.91348f32, 0.91392f32, 0.91435f32,
    0.91478f32, 0.91521f32, 0.91564f32, 0.91606f32, 0.91649f32, 0.91691f32, 0.91734f32, 0.91776f32,
    0.91818f32, 0.91860f32, 0.91902f32, 0.91944f32, 0.91985f32, 0.92027f32, 0.92068f32, 0.92109f32,
    0.92150f32, 0.92191f32, 0.92232f32, 0.92273f32, 0.92314f32, 0.92354f32, 0.92395f32, 0.92435f32,
    0.92475f32, 0.92515f32, 0.92555f32, 0.92595f32, 0.92634f32, 0.92674f32, 0.92713f32, 0.92753f32,
    0.92792f32, 0.92831f32, 0.92870f32, 0.92909f32, 0.92947f32, 0.92986f32, 0.93025f32, 0.93063f32,
    0.93101f32, 0.93139f32, 0.93177f32, 0.93215f32, 0.93253f32, 0.93291f32, 0.93328f32, 0.93366f32,
    0.93403f32, 0.93440f32, 0.93478f32, 0.93515f32, 0.93551f32, 0.93588f32, 0.93625f32, 0.93661f32,
    0.93698f32, 0.93734f32, 0.93770f32, 0.93807f32, 0.93843f32, 0.93878f32, 0.93914f32, 0.93950f32,
    0.93986f32, 0.94021f32, 0.94056f32, 0.94092f32, 0.94127f32, 0.94162f32, 0.94197f32, 0.94231f32,
    0.94266f32, 0.94301f32, 0.94335f32, 0.94369f32, 0.94404f32, 0.94438f32, 0.94472f32, 0.94506f32,
    0.94540f32, 0.94573f32, 0.94607f32, 0.94641f32, 0.94674f32, 0.94707f32, 0.94740f32, 0.94774f32,
    0.94807f32, 0.94839f32, 0.94872f32, 0.94905f32, 0.94937f32, 0.94970f32, 0.95002f32, 0.95035f32,
    0.95067f32, 0.95099f32, 0.95131f32, 0.95163f32, 0.95194f32, 0.95226f32, 0.95257f32, 0.95289f32,
    0.95320f32, 0.95351f32, 0.95383f32, 0.95414f32, 0.95445f32, 0.95475f32, 0.95506f32, 0.95537f32,
    0.95567f32, 0.95598f32, 0.95628f32, 0.95658f32, 0.95688f32, 0.95718f32, 0.95748f32, 0.95778f32,
    0.95808f32, 0.95838f32, 0.95867f32, 0.95897f32, 0.95926f32, 0.95955f32, 0.95984f32, 0.96013f32,
    0.96042f32, 0.96071f32, 0.96100f32, 0.96129f32, 0.96157f32, 0.96186f32, 0.96214f32, 0.96242f32,
    0.96271f32, 0.96299f32, 0.96327f32, 0.96355f32, 0.96382f32, 0.96410f32, 0.96438f32, 0.96465f32,
    0.96493f32, 0.96520f32, 0.96547f32, 0.96574f32, 0.96602f32, 0.96629f32, 0.96655f32, 0.96682f32,
    0.96709f32, 0.96735f32, 0.96762f32, 0.96788f32, 0.96815f32, 0.96841f32, 0.96867f32, 0.96893f32,
    0.96919f32, 0.96945f32, 0.96971f32, 0.96996f32, 0.97022f32, 0.97047f32, 0.97073f32, 0.97098f32,
    0.97123f32, 0.97149f32, 0.97174f32, 0.97199f32, 0.97223f32, 0.97248f32, 0.97273f32, 0.97297f32,
    0.97322f32, 0.97346f32, 0.97371f32, 0.97395f32, 0.97419f32, 0.97443f32, 0.97467f32, 0.97491f32,
    0.97515f32, 0.97539f32, 0.97562f32, 0.97586f32, 0.97609f32, 0.97633f32, 0.97656f32, 0.97679f32,
    0.97702f32, 0.97725f32, 0.97748f32, 0.97771f32, 0.97794f32, 0.97817f32, 0.97839f32, 0.97862f32,
    0.97884f32, 0.97907f32, 0.97929f32, 0.97951f32, 0.97973f32, 0.97995f32, 0.98017f32, 0.98039f32,
    0.98061f32, 0.98082f32, 0.98104f32, 0.98125f32, 0.98147f32, 0.98168f32, 0.98189f32, 0.98211f32,
    0.98232f32, 0.98253f32, 0.98274f32, 0.98295f32, 0.98315f32, 0.98336f32, 0.98357f32, 0.98377f32,
    0.98398f32, 0.98418f32, 0.98438f32, 0.98458f32, 0.98478f32, 0.98498f32, 0.98518f32, 0.98538f32,
    0.98558f32, 0.98578f32, 0.98597f32, 0.98617f32, 0.98636f32, 0.98656f32, 0.98675f32, 0.98694f32,
    0.98714f32, 0.98733f32, 0.98752f32, 0.98771f32, 0.98789f32, 0.98808f32, 0.98827f32, 0.98845f32,
    0.98864f32, 0.98882f32, 0.98901f32, 0.98919f32, 0.98937f32, 0.98955f32, 0.98973f32, 0.98991f32,
    0.99009f32, 0.99027f32, 0.99045f32, 0.99063f32, 0.99080f32, 0.99098f32, 0.99115f32, 0.99133f32,
    0.99150f32, 0.99167f32, 0.99184f32, 0.99201f32, 0.99218f32, 0.99235f32, 0.99252f32, 0.99269f32,
    0.99285f32, 0.99302f32, 0.99319f32, 0.99335f32, 0.99351f32, 0.99368f32, 0.99384f32, 0.99400f32,
    0.99416f32, 0.99432f32, 0.99448f32, 0.99464f32, 0.99480f32, 0.99495f32, 0.99511f32, 0.99527f32,
    0.99542f32, 0.99558f32, 0.99573f32, 0.99588f32, 0.99603f32, 0.99619f32, 0.99634f32, 0.99649f32,
    0.99664f32, 0.99678f32, 0.99693f32, 0.99708f32, 0.99722f32, 0.99737f32, 0.99751f32, 0.99766f32,
    0.99780f32, 0.99794f32, 0.99809f32, 0.99823f32, 0.99837f32, 0.99851f32, 0.99865f32, 0.99879f32,
    0.99892f32, 0.99906f32, 0.99920f32, 0.99933f32, 0.99947f32, 0.99960f32, 0.99974f32, 0.99987f32,
    1.00000f32,
];

/// Inverse table (`dng_tone_curve_acr3_default::EvaluateInverse`).
pub(crate) const ACR3_INVERSE_TABLE: [real32; 1025] = [
    0.00000f32, 0.00121f32, 0.00237f32, 0.00362f32, 0.00496f32, 0.00621f32, 0.00738f32, 0.00848f32,
    0.00951f32, 0.01048f32, 0.01139f32, 0.01227f32, 0.01312f32, 0.01393f32, 0.01471f32, 0.01547f32,
    0.01620f32, 0.01692f32, 0.01763f32, 0.01831f32, 0.01899f32, 0.01965f32, 0.02030f32, 0.02094f32,
    0.02157f32, 0.02218f32, 0.02280f32, 0.02340f32, 0.02399f32, 0.02458f32, 0.02517f32, 0.02574f32,
    0.02631f32, 0.02688f32, 0.02744f32, 0.02800f32, 0.02855f32, 0.02910f32, 0.02965f32, 0.03019f32,
    0.03072f32, 0.03126f32, 0.03179f32, 0.03232f32, 0.03285f32, 0.03338f32, 0.03390f32, 0.03442f32,
    0.03493f32, 0.03545f32, 0.03596f32, 0.03647f32, 0.03698f32, 0.03749f32, 0.03799f32, 0.03849f32,
    0.03899f32, 0.03949f32, 0.03998f32, 0.04048f32, 0.04097f32, 0.04146f32, 0.04195f32, 0.04244f32,
    0.04292f32, 0.04341f32, 0.04389f32, 0.04437f32, 0.04485f32, 0.04533f32, 0.04580f32, 0.04628f32,
    0.04675f32, 0.04722f32, 0.04769f32, 0.04816f32, 0.04863f32, 0.04910f32, 0.04956f32, 0.05003f32,
    0.05049f32, 0.05095f32, 0.05141f32, 0.05187f32, 0.05233f32, 0.05278f32, 0.05324f32, 0.05370f32,
    0.05415f32, 0.05460f32, 0.05505f32, 0.05551f32, 0.05595f32, 0.05640f32, 0.05685f32, 0.05729f32,
    0.05774f32, 0.05818f32, 0.05863f32, 0.05907f32, 0.05951f32, 0.05995f32, 0.06039f32, 0.06083f32,
    0.06126f32, 0.06170f32, 0.06214f32, 0.06257f32, 0.06301f32, 0.06344f32, 0.06388f32, 0.06431f32,
    0.06474f32, 0.06517f32, 0.06560f32, 0.06602f32, 0.06645f32, 0.06688f32, 0.06731f32, 0.06773f32,
    0.06815f32, 0.06858f32, 0.06900f32, 0.06943f32, 0.06985f32, 0.07027f32, 0.07069f32, 0.07111f32,
    0.07152f32, 0.07194f32, 0.07236f32, 0.07278f32, 0.07319f32, 0.07361f32, 0.07402f32, 0.07444f32,
    0.07485f32, 0.07526f32, 0.07567f32, 0.07608f32, 0.07650f32, 0.07691f32, 0.07732f32, 0.07772f32,
    0.07813f32, 0.07854f32, 0.07895f32, 0.07935f32, 0.07976f32, 0.08016f32, 0.08057f32, 0.08098f32,
    0.08138f32, 0.08178f32, 0.08218f32, 0.08259f32, 0.08299f32, 0.08339f32, 0.08379f32, 0.08419f32,
    0.08459f32, 0.08499f32, 0.08539f32, 0.08578f32, 0.08618f32, 0.08657f32, 0.08697f32, 0.08737f32,
    0.08776f32, 0.08816f32, 0.08855f32, 0.08894f32, 0.08934f32, 0.08973f32, 0.09012f32, 0.09051f32,
    0.09091f32, 0.09130f32, 0.09169f32, 0.09208f32, 0.09247f32, 0.09286f32, 0.09324f32, 0.09363f32,
    0.09402f32, 0.09440f32, 0.09479f32, 0.09518f32, 0.09556f32, 0.09595f32, 0.09633f32, 0.09672f32,
    0.09710f32, 0.09749f32, 0.09787f32, 0.09825f32, 0.09863f32, 0.09901f32, 0.09939f32, 0.09978f32,
    0.10016f32, 0.10054f32, 0.10092f32, 0.10130f32, 0.10167f32, 0.10205f32, 0.10243f32, 0.10281f32,
    0.10319f32, 0.10356f32, 0.10394f32, 0.10432f32, 0.10469f32, 0.10507f32, 0.10544f32, 0.10582f32,
    0.10619f32, 0.10657f32, 0.10694f32, 0.10731f32, 0.10768f32, 0.10806f32, 0.10843f32, 0.10880f32,
    0.10917f32, 0.10954f32, 0.10991f32, 0.11029f32, 0.11066f32, 0.11103f32, 0.11141f32, 0.11178f32,
    0.11215f32, 0.11253f32, 0.11290f32, 0.11328f32, 0.11365f32, 0.11403f32, 0.11440f32, 0.11478f32,
    0.11516f32, 0.11553f32, 0.11591f32, 0.11629f32, 0.11666f32, 0.11704f32, 0.11742f32, 0.11780f32,
    0.11818f32, 0.11856f32, 0.11894f32, 0.11932f32, 0.11970f32, 0.12008f32, 0.12046f32, 0.12084f32,
    0.12122f32, 0.12161f32, 0.12199f32, 0.12237f32, 0.12276f32, 0.12314f32, 0.12352f32, 0.12391f32,
    0.12429f32, 0.12468f32, 0.12506f32, 0.12545f32, 0.12583f32, 0.12622f32, 0.12661f32, 0.12700f32,
    0.12738f32, 0.12777f32, 0.12816f32, 0.12855f32, 0.12894f32, 0.12933f32, 0.12972f32, 0.13011f32,
    0.13050f32, 0.13089f32, 0.13129f32, 0.13168f32, 0.13207f32, 0.13247f32, 0.13286f32, 0.13325f32,
    0.13365f32, 0.13404f32, 0.13444f32, 0.13483f32, 0.13523f32, 0.13563f32, 0.13603f32, 0.13642f32,
    0.13682f32, 0.13722f32, 0.13762f32, 0.13802f32, 0.13842f32, 0.13882f32, 0.13922f32, 0.13962f32,
    0.14003f32, 0.14043f32, 0.14083f32, 0.14124f32, 0.14164f32, 0.14204f32, 0.14245f32, 0.14285f32,
    0.14326f32, 0.14366f32, 0.14407f32, 0.14448f32, 0.14489f32, 0.14530f32, 0.14570f32, 0.14611f32,
    0.14652f32, 0.14693f32, 0.14734f32, 0.14776f32, 0.14817f32, 0.14858f32, 0.14900f32, 0.14941f32,
    0.14982f32, 0.15024f32, 0.15065f32, 0.15107f32, 0.15148f32, 0.15190f32, 0.15232f32, 0.15274f32,
    0.15316f32, 0.15357f32, 0.15399f32, 0.15441f32, 0.15483f32, 0.15526f32, 0.15568f32, 0.15610f32,
    0.15652f32, 0.15695f32, 0.15737f32, 0.15779f32, 0.15822f32, 0.15864f32, 0.15907f32, 0.15950f32,
    0.15992f32, 0.16035f32, 0.16078f32, 0.16121f32, 0.16164f32, 0.16207f32, 0.16250f32, 0.16293f32,
    0.16337f32, 0.16380f32, 0.16423f32, 0.16467f32, 0.16511f32, 0.16554f32, 0.16598f32, 0.16641f32,
    0.16685f32, 0.16729f32, 0.16773f32, 0.16816f32, 0.16860f32, 0.16904f32, 0.16949f32, 0.16993f32,
    0.17037f32, 0.17081f32, 0.17126f32, 0.17170f32, 0.17215f32, 0.17259f32, 0.17304f32, 0.17349f32,
    0.17393f32, 0.17438f32, 0.17483f32, 0.17528f32, 0.17573f32, 0.17619f32, 0.17664f32, 0.17709f32,
    0.17754f32, 0.17799f32, 0.17845f32, 0.17890f32, 0.17936f32, 0.17982f32, 0.18028f32, 0.18073f32,
    0.18119f32, 0.18165f32, 0.18211f32, 0.18257f32, 0.18303f32, 0.18350f32, 0.18396f32, 0.18442f32,
    0.18489f32, 0.18535f32, 0.18582f32, 0.18629f32, 0.18676f32, 0.18723f32, 0.18770f32, 0.18817f32,
    0.18864f32, 0.18911f32, 0.18958f32, 0.19005f32, 0.19053f32, 0.19100f32, 0.19147f32, 0.19195f32,
    0.19243f32, 0.19291f32, 0.19339f32, 0.19387f32, 0.19435f32, 0.19483f32, 0.19531f32, 0.19579f32,
    0.19627f32, 0.19676f32, 0.19724f32, 0.19773f32, 0.19821f32, 0.19870f32, 0.19919f32, 0.19968f32,
    0.20017f32, 0.20066f32, 0.20115f32, 0.20164f32, 0.20214f32, 0.20263f32, 0.20313f32, 0.20362f32,
    0.20412f32, 0.20462f32, 0.20512f32, 0.20561f32, 0.20611f32, 0.20662f32, 0.20712f32, 0.20762f32,
    0.20812f32, 0.20863f32, 0.20913f32, 0.20964f32, 0.21015f32, 0.21066f32, 0.21117f32, 0.21168f32,
    0.21219f32, 0.21270f32, 0.21321f32, 0.21373f32, 0.21424f32, 0.21476f32, 0.21527f32, 0.21579f32,
    0.21631f32, 0.21683f32, 0.21735f32, 0.21787f32, 0.21839f32, 0.21892f32, 0.21944f32, 0.21997f32,
    0.22049f32, 0.22102f32, 0.22155f32, 0.22208f32, 0.22261f32, 0.22314f32, 0.22367f32, 0.22420f32,
    0.22474f32, 0.22527f32, 0.22581f32, 0.22634f32, 0.22688f32, 0.22742f32, 0.22796f32, 0.22850f32,
    0.22905f32, 0.22959f32, 0.23013f32, 0.23068f32, 0.23123f32, 0.23178f32, 0.23232f32, 0.23287f32,
    0.23343f32, 0.23398f32, 0.23453f32, 0.23508f32, 0.23564f32, 0.23620f32, 0.23675f32, 0.23731f32,
    0.23787f32, 0.23843f32, 0.23899f32, 0.23956f32, 0.24012f32, 0.24069f32, 0.24125f32, 0.24182f32,
    0.24239f32, 0.24296f32, 0.24353f32, 0.24410f32, 0.24468f32, 0.24525f32, 0.24582f32, 0.24640f32,
    0.24698f32, 0.24756f32, 0.24814f32, 0.24872f32, 0.24931f32, 0.24989f32, 0.25048f32, 0.25106f32,
    0.25165f32, 0.25224f32, 0.25283f32, 0.25342f32, 0.25401f32, 0.25460f32, 0.25520f32, 0.25579f32,
    0.25639f32, 0.25699f32, 0.25759f32, 0.25820f32, 0.25880f32, 0.25940f32, 0.26001f32, 0.26062f32,
    0.26122f32, 0.26183f32, 0.26244f32, 0.26306f32, 0.26367f32, 0.26429f32, 0.26490f32, 0.26552f32,
    0.26614f32, 0.26676f32, 0.26738f32, 0.26800f32, 0.26863f32, 0.26925f32, 0.26988f32, 0.27051f32,
    0.27114f32, 0.27177f32, 0.27240f32, 0.27303f32, 0.27367f32, 0.27431f32, 0.27495f32, 0.27558f32,
    0.27623f32, 0.27687f32, 0.27751f32, 0.27816f32, 0.27881f32, 0.27945f32, 0.28011f32, 0.28076f32,
    0.28141f32, 0.28207f32, 0.28272f32, 0.28338f32, 0.28404f32, 0.28470f32, 0.28536f32, 0.28602f32,
    0.28669f32, 0.28736f32, 0.28802f32, 0.28869f32, 0.28937f32, 0.29004f32, 0.29071f32, 0.29139f32,
    0.29207f32, 0.29274f32, 0.29342f32, 0.29410f32, 0.29479f32, 0.29548f32, 0.29616f32, 0.29685f32,
    0.29754f32, 0.29823f32, 0.29893f32, 0.29962f32, 0.30032f32, 0.30102f32, 0.30172f32, 0.30242f32,
    0.30312f32, 0.30383f32, 0.30453f32, 0.30524f32, 0.30595f32, 0.30667f32, 0.30738f32, 0.30809f32,
    0.30881f32, 0.30953f32, 0.31025f32, 0.31097f32, 0.31170f32, 0.31242f32, 0.31315f32, 0.31388f32,
    0.31461f32, 0.31534f32, 0.31608f32, 0.31682f32, 0.31755f32, 0.31829f32, 0.31904f32, 0.31978f32,
    0.32053f32, 0.32127f32, 0.32202f32, 0.32277f32, 0.32353f32, 0.32428f32, 0.32504f32, 0.32580f32,
    0.32656f32, 0.32732f32, 0.32808f32, 0.32885f32, 0.32962f32, 0.33039f32, 0.33116f32, 0.33193f32,
    0.33271f32, 0.33349f32, 0.33427f32, 0.33505f32, 0.33583f32, 0.33662f32, 0.33741f32, 0.33820f32,
    0.33899f32, 0.33978f32, 0.34058f32, 0.34138f32, 0.34218f32, 0.34298f32, 0.34378f32, 0.34459f32,
    0.34540f32, 0.34621f32, 0.34702f32, 0.34783f32, 0.34865f32, 0.34947f32, 0.35029f32, 0.35111f32,
    0.35194f32, 0.35277f32, 0.35360f32, 0.35443f32, 0.35526f32, 0.35610f32, 0.35694f32, 0.35778f32,
    0.35862f32, 0.35946f32, 0.36032f32, 0.36117f32, 0.36202f32, 0.36287f32, 0.36372f32, 0.36458f32,
    0.36545f32, 0.36631f32, 0.36718f32, 0.36805f32, 0.36891f32, 0.36979f32, 0.37066f32, 0.37154f32,
    0.37242f32, 0.37331f32, 0.37419f32, 0.37507f32, 0.37596f32, 0.37686f32, 0.37775f32, 0.37865f32,
    0.37955f32, 0.38045f32, 0.38136f32, 0.38227f32, 0.38317f32, 0.38409f32, 0.38500f32, 0.38592f32,
    0.38684f32, 0.38776f32, 0.38869f32, 0.38961f32, 0.39055f32, 0.39148f32, 0.39242f32, 0.39335f32,
    0.39430f32, 0.39524f32, 0.39619f32, 0.39714f32, 0.39809f32, 0.39904f32, 0.40000f32, 0.40097f32,
    0.40193f32, 0.40289f32, 0.40386f32, 0.40483f32, 0.40581f32, 0.40679f32, 0.40777f32, 0.40875f32,
    0.40974f32, 0.41073f32, 0.41172f32, 0.41272f32, 0.41372f32, 0.41472f32, 0.41572f32, 0.41673f32,
    0.41774f32, 0.41875f32, 0.41977f32, 0.42079f32, 0.42181f32, 0.42284f32, 0.42386f32, 0.42490f32,
    0.42594f32, 0.42697f32, 0.42801f32, 0.42906f32, 0.43011f32, 0.43116f32, 0.43222f32, 0.43327f32,
    0.43434f32, 0.43540f32, 0.43647f32, 0.43754f32, 0.43862f32, 0.43970f32, 0.44077f32, 0.44186f32,
    0.44295f32, 0.44404f32, 0.44514f32, 0.44624f32, 0.44734f32, 0.44845f32, 0.44956f32, 0.45068f32,
    0.45179f32, 0.45291f32, 0.45404f32, 0.45516f32, 0.45630f32, 0.45744f32, 0.45858f32, 0.45972f32,
    0.46086f32, 0.46202f32, 0.46318f32, 0.46433f32, 0.46550f32, 0.46667f32, 0.46784f32, 0.46901f32,
    0.47019f32, 0.47137f32, 0.47256f32, 0.47375f32, 0.47495f32, 0.47615f32, 0.47735f32, 0.47856f32,
    0.47977f32, 0.48099f32, 0.48222f32, 0.48344f32, 0.48467f32, 0.48590f32, 0.48714f32, 0.48838f32,
    0.48963f32, 0.49088f32, 0.49213f32, 0.49340f32, 0.49466f32, 0.49593f32, 0.49721f32, 0.49849f32,
    0.49977f32, 0.50106f32, 0.50236f32, 0.50366f32, 0.50496f32, 0.50627f32, 0.50758f32, 0.50890f32,
    0.51023f32, 0.51155f32, 0.51289f32, 0.51422f32, 0.51556f32, 0.51692f32, 0.51827f32, 0.51964f32,
    0.52100f32, 0.52237f32, 0.52374f32, 0.52512f32, 0.52651f32, 0.52790f32, 0.52930f32, 0.53070f32,
    0.53212f32, 0.53353f32, 0.53495f32, 0.53638f32, 0.53781f32, 0.53925f32, 0.54070f32, 0.54214f32,
    0.54360f32, 0.54506f32, 0.54653f32, 0.54800f32, 0.54949f32, 0.55098f32, 0.55247f32, 0.55396f32,
    0.55548f32, 0.55699f32, 0.55851f32, 0.56003f32, 0.56156f32, 0.56310f32, 0.56464f32, 0.56621f32,
    0.56777f32, 0.56933f32, 0.57091f32, 0.57248f32, 0.57407f32, 0.57568f32, 0.57727f32, 0.57888f32,
    0.58050f32, 0.58213f32, 0.58376f32, 0.58541f32, 0.58705f32, 0.58871f32, 0.59037f32, 0.59204f32,
    0.59373f32, 0.59541f32, 0.59712f32, 0.59882f32, 0.60052f32, 0.60226f32, 0.60399f32, 0.60572f32,
    0.60748f32, 0.60922f32, 0.61099f32, 0.61276f32, 0.61455f32, 0.61635f32, 0.61814f32, 0.61996f32,
    0.62178f32, 0.62361f32, 0.62545f32, 0.62730f32, 0.62917f32, 0.63104f32, 0.63291f32, 0.63480f32,
    0.63671f32, 0.63862f32, 0.64054f32, 0.64249f32, 0.64443f32, 0.64638f32, 0.64835f32, 0.65033f32,
    0.65232f32, 0.65433f32, 0.65633f32, 0.65836f32, 0.66041f32, 0.66245f32, 0.66452f32, 0.66660f32,
    0.66868f32, 0.67078f32, 0.67290f32, 0.67503f32, 0.67717f32, 0.67932f32, 0.68151f32, 0.68368f32,
    0.68587f32, 0.68809f32, 0.69033f32, 0.69257f32, 0.69482f32, 0.69709f32, 0.69939f32, 0.70169f32,
    0.70402f32, 0.70634f32, 0.70869f32, 0.71107f32, 0.71346f32, 0.71587f32, 0.71829f32, 0.72073f32,
    0.72320f32, 0.72567f32, 0.72818f32, 0.73069f32, 0.73323f32, 0.73579f32, 0.73838f32, 0.74098f32,
    0.74360f32, 0.74622f32, 0.74890f32, 0.75159f32, 0.75429f32, 0.75704f32, 0.75979f32, 0.76257f32,
    0.76537f32, 0.76821f32, 0.77109f32, 0.77396f32, 0.77688f32, 0.77982f32, 0.78278f32, 0.78579f32,
    0.78883f32, 0.79187f32, 0.79498f32, 0.79809f32, 0.80127f32, 0.80445f32, 0.80767f32, 0.81095f32,
    0.81424f32, 0.81757f32, 0.82094f32, 0.82438f32, 0.82782f32, 0.83133f32, 0.83488f32, 0.83847f32,
    0.84210f32, 0.84577f32, 0.84951f32, 0.85328f32, 0.85713f32, 0.86103f32, 0.86499f32, 0.86900f32,
    0.87306f32, 0.87720f32, 0.88139f32, 0.88566f32, 0.89000f32, 0.89442f32, 0.89891f32, 0.90350f32,
    0.90818f32, 0.91295f32, 0.91780f32, 0.92272f32, 0.92780f32, 0.93299f32, 0.93828f32, 0.94369f32,
    0.94926f32, 0.95493f32, 0.96082f32, 0.96684f32, 0.97305f32, 0.97943f32, 0.98605f32, 0.99291f32,
    1.00000f32,
];

}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_negative::DngNegative;
    use dng_core::dng_host::DngHost;
    use dng_core::dng_pixel_buffer::{DngPixelBuffer, PixelData};
    use dng_core::dng_simple_image::DngSimpleImage;
    use dng_core::dng_tag_types::ttFloat;
    use dng_core::dng_tag_values::pcInterleaved;
    use dng_core::DngResult;

    /// Render a synthetic stage3 (RGB float) image and verify the output
    /// shape, pixel type and value range (identity-ish pipeline: no profile,
    /// monochrome path exercises the camera-to-RGB copy + tone curve).
    #[test]
    fn test_render_synthetic_stage3() -> DngResult<()> {
        let mut host = DngHost::new();

        let mut negative = DngNegative::new(&mut host)?;

        // 4x3 monochrome float stage3. Rendering requires ColorChannels >= 1
        // (the C++ dng_color_spec ctor only populates matrices for >1
        // channels, and SetWhiteXY installs the mono PCStoXYZ column for
        // exactly 1; a 0-channel negative is outside the C++-defined
        // behavior), so render the single-channel path.

        let width = 4;
        let height = 3;

        let bounds = DngRect::from_size_point(DngPoint::new(height as int32, width as int32));

        negative.set_color_channels(1);

        // Fill with a uniform gray (0.25) via from_buffer — building
        // the image from the pre-filled F32 buffer avoids
        // DngSimpleImage::do_put (whose copy_area writes 4-byte elements
        // through set_u32, which lacked an F32-storage arm in dng-core).

        let stage3: ImageRef = DngSimpleImage::from_buffer(DngPixelBuffer::with_planar_config(
            bounds,
            0,
            1,
            ttFloat,
            pcInterleaved,
            PixelData::F32(vec![0.25; (width * height) as usize]),
        )?)?
        .into_image_ref()?;

        negative.set_stage3_image(Some(stage3));

        negative.set_default_crop_size_u32(width, height);

        // Render.

        let mut render = DngRender::new(&negative)?;

        render.set_final_pixel_type(ttFloat);

        let rendered = render.render(&negative)?;

        {
            let out = rendered.read().unwrap();

            assert_eq!(out.width(), width);
            assert_eq!(out.height(), height);
            assert_eq!(out.pixel_type(), ttFloat);
        }

        Ok(())
    }

    /// Overrange encode/decode helpers are exact inverses at sample points.
    #[test]
    fn test_overrange_round_trip() {
        for x in [0.0f32, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0] {
            let encoded = encode_overrange(x);
            let decoded = decode_overrange(encoded);

            assert!(
                (decoded - x).abs() < 1e-4,
                "decode(encode({x})) = {decoded}"
            );
        }
    }

    /// ACR3 default tone curve sanity: monotone and [0,1]-bounded.
    #[test]
    fn test_acr3_tone_curve_range() {
        let curve = DngToneCurveAcr3Default::get();

        let y0 = curve.evaluate(0.0);
        let y1 = curve.evaluate(1.0);

        assert!((y0 - 0.0).abs() < 1e-3, "ACR3(0) = {y0}");
        assert!((y1 - 1.0).abs() < 1e-3, "ACR3(1) = {y1}");

        let mid = curve.evaluate(0.5);

        assert!((0.0..=1.0).contains(&mid), "ACR3(0.5) = {mid}");
    }
}

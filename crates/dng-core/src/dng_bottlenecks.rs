// Port of dng_bottlenecks.h + the scalar implementations from
// dng_reference.cpp (the AVX2 dispatch table is replaced by direct scalar
// functions; results are bit-identical to the C++ Scalar path).
//
// PORT-NOTE(dng_bottlenecks.h:715): phase-1 ports the full pixel-buffer /
// resample / equal-area suite.  The render-side bottlenecks that depend on
// phase-2 types (DoBaselineHueSatMap -> dng_hue_sat_map,
// DoBaselineProfileGainTableMap -> gain tables, DoVignetteMask16/DoVignette16/
// DoVignette32/DoMapArea16/DoBaselineMapPoly32/DoBaseline*Color/DoRGBtoRGB-
// Table3D/1D/DoDecodeLosslessJPEG/DoEncodeLosslessJPEG) are implemented in
// phase 2 alongside their parameter types; see PORTING_NOTES.md.
//
// PORT-NOTE(dng_reference.cpp:22): dng_reference.cpp is exactly the
// platform-reference (scalar) implementation of the bottleneck suite used as
// SIMD fallback; the functions below are its faithful ports.

use crate::dng_types::{int16, int32, real32, uint16, uint32, uint8};
use crate::dng_utils::{pin_int32, pin_real32_01};

// ------------------------------------------------------------------ bytes

/// `RefZeroBytes`.
pub fn zero_bytes(d: &mut [uint8]) {
    d.fill(0);
}

/// `RefCopyBytes`.
pub fn copy_bytes(s: &[uint8], d: &mut [uint8]) {
    d[..s.len()].copy_from_slice(s);
}

/// `RefSwapBytes16`.
pub fn swap_bytes16(d: &mut [uint16]) {
    for v in d.iter_mut() {
        *v = v.swap_bytes();
    }
}

/// `RefSwapBytes32`.
pub fn swap_bytes32(d: &mut [uint32]) {
    for v in d.iter_mut() {
        *v = v.swap_bytes();
    }
}

// ------------------------------------------------------------------ set area

/// `RefSetArea8`.
#[allow(clippy::too_many_arguments)]
pub fn set_area8(
    d: &mut [uint8],
    base: usize,
    value: uint8,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    row_step: int32,
    col_step: int32,
    plane_step: int32,
) {
    let mut row_ptr = base as isize;
    for _ in 0..rows {
        let mut col_ptr = row_ptr;
        for _ in 0..cols {
            let mut plane_ptr = col_ptr;
            for _ in 0..planes {
                d[plane_ptr as usize] = value;
                plane_ptr += plane_step as isize;
            }
            col_ptr += col_step as isize;
        }
        row_ptr += row_step as isize;
    }
}

/// `RefSetArea` (16-bit element version).
#[allow(clippy::too_many_arguments)]
pub fn set_area16(
    d: &mut [uint16],
    base: usize,
    value: uint16,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    row_step: int32,
    col_step: int32,
    plane_step: int32,
) {
    let mut row_ptr = base as isize;
    for _ in 0..rows {
        let mut col_ptr = row_ptr;
        for _ in 0..cols {
            let mut plane_ptr = col_ptr;
            for _ in 0..planes {
                d[plane_ptr as usize] = value;
                plane_ptr += plane_step as isize;
            }
            col_ptr += col_step as isize;
        }
        row_ptr += row_step as isize;
    }
}

/// `RefSetArea` (32-bit element version).
#[allow(clippy::too_many_arguments)]
pub fn set_area32(
    d: &mut [uint32],
    base: usize,
    value: uint32,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    row_step: int32,
    col_step: int32,
    plane_step: int32,
) {
    let mut row_ptr = base as isize;
    for _ in 0..rows {
        let mut col_ptr = row_ptr;
        for _ in 0..cols {
            let mut plane_ptr = col_ptr;
            for _ in 0..planes {
                d[plane_ptr as usize] = value;
                plane_ptr += plane_step as isize;
            }
            col_ptr += col_step as isize;
        }
        row_ptr += row_step as isize;
    }
}

// ------------------------------------------------------------------ copy areas

macro_rules! copy_area_same {
    ($name:ident, $ty:ty) => {
        /// Same-type area copy (`RefCopyArea8/16/32` pattern).
        #[allow(clippy::too_many_arguments)]
        pub fn $name(
            s: &[$ty],
            s_base: usize,
            d: &mut [$ty],
            d_base: usize,
            rows: uint32,
            cols: uint32,
            planes: uint32,
            s_row_step: int32,
            s_col_step: int32,
            s_plane_step: int32,
            d_row_step: int32,
            d_col_step: int32,
            d_plane_step: int32,
        ) {
            let mut s_row = s_base as isize;
            let mut d_row = d_base as isize;
            for _ in 0..rows {
                let mut s_col = s_row;
                let mut d_col = d_row;
                for _ in 0..cols {
                    let mut s_plane = s_col;
                    let mut d_plane = d_col;
                    for _ in 0..planes {
                        d[d_plane as usize] = s[s_plane as usize];
                        s_plane += s_plane_step as isize;
                        d_plane += d_plane_step as isize;
                    }
                    s_col += s_col_step as isize;
                    d_col += d_col_step as isize;
                }
                s_row += s_row_step as isize;
                d_row += d_row_step as isize;
            }
        }
    };
}

copy_area_same!(copy_area8, uint8);
copy_area_same!(copy_area16, uint16);
copy_area_same!(copy_area32, uint32);

/// `RefCopyArea8_16` — widen.
#[allow(clippy::too_many_arguments)]
pub fn copy_area8_16(
    s: &[uint8],
    s_base: usize,
    d: &mut [uint16],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
) {
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = s[s_plane as usize] as uint16;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea8_S16` — widen + sign-bit bias (0x8000).
#[allow(clippy::too_many_arguments)]
pub fn copy_area8_s16(
    s: &[uint8],
    s_base: usize,
    d: &mut [int16],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
) {
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                let x = s[s_plane as usize] as uint16;
                d[d_plane as usize] = (x ^ 0x8000) as int16;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea8_32` — widen.
#[allow(clippy::too_many_arguments)]
pub fn copy_area8_32(
    s: &[uint8],
    s_base: usize,
    d: &mut [uint32],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
) {
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = s[s_plane as usize] as uint32;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea16_S16` — sign-bit toggle.
#[allow(clippy::too_many_arguments)]
pub fn copy_area16_s16(
    s: &[uint16],
    s_base: usize,
    d: &mut [int16],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
) {
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = (s[s_plane as usize] ^ 0x8000) as int16;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea16_32` — widen.
#[allow(clippy::too_many_arguments)]
pub fn copy_area16_32(
    s: &[uint16],
    s_base: usize,
    d: &mut [uint32],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
) {
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = s[s_plane as usize] as uint32;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea8_R32` — normalize by pixel range.
#[allow(clippy::too_many_arguments)]
pub fn copy_area8_r32(
    s: &[uint8],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = 1.0f32 / pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = scale * s[s_plane as usize] as real32;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyArea16_R32` — normalize by pixel range.
#[allow(clippy::too_many_arguments)]
pub fn copy_area16_r32(
    s: &[uint16],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = 1.0f32 / pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = scale * s[s_plane as usize] as real32;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyAreaS16_R32` — unbias (x + 32768) and normalize.
#[allow(clippy::too_many_arguments)]
pub fn copy_area_s16_r32(
    s: &[int16],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = 1.0f32 / pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                let x = s[s_plane as usize] as int32;
                d[d_plane as usize] = scale * (x + 32768) as real32;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyAreaR32_8` — denormalize with Pin_Overrange clamp.
#[allow(clippy::too_many_arguments)]
pub fn copy_area_r32_8(
    s: &[real32],
    s_base: usize,
    d: &mut [uint8],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = (pin_real32_01(s[s_plane as usize]) * scale + 0.5) as uint8;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyAreaR32_16`.
#[allow(clippy::too_many_arguments)]
pub fn copy_area_r32_16(
    s: &[real32],
    s_base: usize,
    d: &mut [uint16],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                d[d_plane as usize] = (pin_real32_01(s[s_plane as usize]) * scale + 0.5) as uint16;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

/// `RefCopyAreaR32_S16`.
#[allow(clippy::too_many_arguments)]
pub fn copy_area_r32_s16(
    s: &[real32],
    s_base: usize,
    d: &mut [int16],
    d_base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    s_row_step: int32,
    s_col_step: int32,
    s_plane_step: int32,
    d_row_step: int32,
    d_col_step: int32,
    d_plane_step: int32,
    pixel_range: uint32,
) {
    let scale = pixel_range as real32;
    let mut s_row = s_base as isize;
    let mut d_row = d_base as isize;
    for _ in 0..rows {
        let mut s_col = s_row;
        let mut d_col = d_row;
        for _ in 0..cols {
            let mut s_plane = s_col;
            let mut d_plane = d_col;
            for _ in 0..planes {
                let x = (pin_real32_01(s[s_plane as usize]) * scale + 0.5) as int32;
                d[d_plane as usize] = ((x as uint16) ^ 0x8000) as int16;
                s_plane += s_plane_step as isize;
                d_plane += d_plane_step as isize;
            }
            s_col += s_col_step as isize;
            d_col += d_col_step as isize;
        }
        s_row += s_row_step as isize;
        d_row += d_row_step as isize;
    }
}

// ------------------------------------------------------------------ repeat

macro_rules! repeat_area {
    ($name:ident, $ty:ty) => {
        /// `RefRepeatArea8/16/32` pattern.
        #[allow(clippy::too_many_arguments)]
        pub fn $name(
            s: &[$ty],
            s_base: usize,
            d: &mut [$ty],
            d_base: usize,
            rows: uint32,
            cols: uint32,
            planes: uint32,
            row_step: int32,
            col_step: int32,
            plane_step: int32,
            repeat_v: uint32,
            repeat_h: uint32,
            mut phase_v: uint32,
            phase_h: uint32,
        ) {
            let mut s_ptr0 = s_base as isize
                + phase_v as isize * row_step as isize
                + phase_h as isize * col_step as isize;

            let back_step_v = (repeat_v as isize - 1) * row_step as isize;
            let back_step_h = (repeat_h as isize - 1) * col_step as isize;

            let mut d_row = d_base as isize;
            for _ in 0..rows {
                let mut s_ptr1 = s_ptr0;
                let mut d_ptr1 = d_row;

                let mut col_phase = phase_h;

                for _ in 0..cols {
                    let mut s_ptr2 = s_ptr1;
                    let mut d_ptr2 = d_ptr1;

                    for _ in 0..planes {
                        d[d_ptr2 as usize] = s[s_ptr2 as usize];
                        s_ptr2 += plane_step as isize;
                        d_ptr2 += plane_step as isize;
                    }

                    col_phase += 1;
                    if col_phase == repeat_h {
                        col_phase = 0;
                        s_ptr1 -= back_step_h;
                    } else {
                        s_ptr1 += col_step as isize;
                    }

                    d_ptr1 += col_step as isize;
                }

                phase_v += 1;
                if phase_v == repeat_v {
                    phase_v = 0;
                    s_ptr0 -= back_step_v;
                } else {
                    s_ptr0 += row_step as isize;
                }

                d_row += row_step as isize;
            }
        }
    };
}

repeat_area!(repeat_area8, uint8);
repeat_area!(repeat_area16, uint16);
repeat_area!(repeat_area32, uint32);

/// `RefShiftRight16`.
#[allow(clippy::too_many_arguments)]
pub fn shift_right16(
    d: &mut [uint16],
    base: usize,
    rows: uint32,
    cols: uint32,
    planes: uint32,
    row_step: int32,
    col_step: int32,
    plane_step: int32,
    shift: uint32,
) {
    let mut row_ptr = base as isize;
    for _ in 0..rows {
        let mut col_ptr = row_ptr;
        for _ in 0..cols {
            let mut plane_ptr = col_ptr;
            for _ in 0..planes {
                d[plane_ptr as usize] >>= shift;
                plane_ptr += plane_step as isize;
            }
            col_ptr += col_step as isize;
        }
        row_ptr += row_step as isize;
    }
}

// ------------------------------------------------------------------ resample

/// `RefResampleDown16`.
pub fn resample_down16(
    s: &[uint16],
    s_base: usize,
    d: &mut [uint16],
    d_base: usize,
    s_count: uint32,
    s_row_step: int32,
    w: &[int16],
    w_count: uint32,
    pixel_range: uint32,
) {
    for j in 0..s_count as usize {
        let mut total: int32 = 8192;

        let mut p = (s_base + j) as isize;

        for k in 0..w_count as usize {
            total += w[k] as int32 * s[p as usize] as int32;
            p += s_row_step as isize;
        }

        d[d_base + j] = pin_int32(0, total >> 14, pixel_range as int32) as uint16;
    }
}

/// `RefResampleDown32`.
pub fn resample_down32(
    s: &[real32],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    s_count: uint32,
    s_row_step: int32,
    w: &[real32],
    w_count: uint32,
) {
    // Process first row.
    let mut weight = w[0];
    for col in 0..s_count as usize {
        d[d_base + col] = weight * s[s_base + col];
    }
    let mut p = (s_base as isize) + s_row_step as isize;

    // Process middle rows.
    for j in 1..(w_count as usize - 1) {
        weight = w[j];
        for col in 0..s_count as usize {
            d[d_base + col] += weight * s[p as usize + col];
        }
        p += s_row_step as isize;
    }

    // Process last row.
    weight = w[w_count as usize - 1];
    for col in 0..s_count as usize {
        d[d_base + col] = crate::dng_utils::pin_real32(0.0, d[d_base + col] + weight * s[p as usize + col], 1.0);
    }
}

/// `RefResampleAcross16`.
pub fn resample_across16(
    s: &[uint16],
    s_base: usize,
    d: &mut [uint16],
    d_base: usize,
    d_count: uint32,
    coord: &[int32],
    w: &[int16],
    w_count: uint32,
    w_step: uint32,
    pixel_range: uint32,
) {
    use crate::dng_resample::{kResampleSubsampleBits, kResampleSubsampleMask};

    for j in 0..d_count as usize {
        let s_coord = coord[j];

        let s_fract = (s_coord & kResampleSubsampleMask as int32) as usize;
        let s_pixel = (s_coord >> kResampleSubsampleBits) as usize;

        let w_row = s_fract * w_step as usize;
        let s_col = s_base + s_pixel;

        let mut total: int32 = w[w_row] as int32 * s[s_col] as int32;

        for k in 1..w_count as usize {
            total += w[w_row + k] as int32 * s[s_col + k] as int32;
        }

        d[d_base + j] = pin_int32(0, (total + 8192) >> 14, pixel_range as int32) as uint16;
    }
}

/// `RefResampleAcross32`.
pub fn resample_across32(
    s: &[real32],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    d_count: uint32,
    coord: &[int32],
    w: &[real32],
    w_count: uint32,
    w_step: uint32,
) {
    use crate::dng_resample::{kResampleSubsampleBits, kResampleSubsampleMask};

    for j in 0..d_count as usize {
        let s_coord = coord[j];

        let s_fract = (s_coord & kResampleSubsampleMask as int32) as usize;
        let s_pixel = (s_coord >> kResampleSubsampleBits) as usize;

        let w_row = s_fract * w_step as usize;
        let s_col = s_base + s_pixel;

        let mut total: real32 = w[w_row] * s[s_col];

        for k in 1..w_count as usize {
            total += w[w_row + k] * s[s_col + k];
        }

        d[d_base + j] = pin_real32_01(total);
    }
}

// ------------------------------------------------------------------ bilinear

/// `RefBilinearRow16`.
#[allow(clippy::too_many_arguments)]
pub fn bilinear_row16(
    s: &[uint16],
    s_base: usize,
    d: &mut [uint16],
    d_base: usize,
    cols: uint32,
    mut pat_phase: uint32,
    pat_count: uint32,
    kern_counts: &[uint32],
    kern_offsets: &[&[int32]],
    kern_weights: &[&[uint16]],
    s_shift: uint32,
) {
    for j in 0..cols as usize {
        let p = s_base + (j >> s_shift);

        let count = kern_counts[pat_phase as usize] as usize;

        let offsets = kern_offsets[pat_phase as usize];
        let weights = kern_weights[pat_phase as usize];

        pat_phase += 1;
        if pat_phase == pat_count {
            pat_phase = 0;
        }

        let mut total: uint32 = 128;

        for k in 0..count {
            let offset = offsets[k];
            let weight = weights[k] as uint32;

            let pixel = s[(p as isize + offset as isize) as usize] as uint32;

            total += pixel * weight;
        }

        d[d_base + j] = (total >> 8) as uint16;
    }
}

/// `RefBilinearRow32`.
#[allow(clippy::too_many_arguments)]
pub fn bilinear_row32(
    s: &[real32],
    s_base: usize,
    d: &mut [real32],
    d_base: usize,
    cols: uint32,
    mut pat_phase: uint32,
    pat_count: uint32,
    kern_counts: &[uint32],
    kern_offsets: &[&[int32]],
    kern_weights: &[&[real32]],
    s_shift: uint32,
) {
    for j in 0..cols as usize {
        let p = s_base + (j >> s_shift);

        let count = kern_counts[pat_phase as usize] as usize;

        let offsets = kern_offsets[pat_phase as usize];
        let weights = kern_weights[pat_phase as usize];

        pat_phase += 1;
        if pat_phase == pat_count {
            pat_phase = 0;
        }

        let mut total: real32 = 0.0;

        for k in 0..count {
            let offset = offsets[k];
            let weight = weights[k];

            let pixel = s[(p as isize + offset as isize) as usize];

            total += pixel * weight;
        }

        d[d_base + j] = total;
    }
}

// ------------------------------------------------------------------ equal

/// `RefEqualBytes`.
pub fn equal_bytes(s: &[uint8], s_base: usize, d: &[uint8], d_base: usize, count: uint32) -> bool {
    s[s_base..s_base + count as usize] == d[d_base..d_base + count as usize]
}

macro_rules! equal_area {
    ($name:ident, $ty:ty) => {
        /// `RefEqualArea8/16/32` pattern.
        #[allow(clippy::too_many_arguments)]
        pub fn $name(
            s: &[$ty],
            s_base: usize,
            d: &[$ty],
            d_base: usize,
            rows: uint32,
            cols: uint32,
            planes: uint32,
            s_row_step: int32,
            s_col_step: int32,
            s_plane_step: int32,
            d_row_step: int32,
            d_col_step: int32,
            d_plane_step: int32,
        ) -> bool {
            let mut s_row = s_base as isize;
            let mut d_row = d_base as isize;
            for _ in 0..rows {
                let mut s_col = s_row;
                let mut d_col = d_row;
                for _ in 0..cols {
                    let mut s_plane = s_col;
                    let mut d_plane = d_col;
                    for _ in 0..planes {
                        if s[s_plane as usize] != d[d_plane as usize] {
                            return false;
                        }
                        s_plane += s_plane_step as isize;
                        d_plane += d_plane_step as isize;
                    }
                    s_col += s_col_step as isize;
                    d_col += d_col_step as isize;
                }
                s_row += s_row_step as isize;
                d_row += d_row_step as isize;
            }
            true
        }
    };
}

equal_area!(equal_area8, uint8);
equal_area!(equal_area16, uint16);
equal_area!(equal_area32, uint32);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_area8_same_layout() {
        let s = vec![1u8, 2, 3, 4, 5, 6];
        let mut d = vec![0u8; 6];
        copy_area8(&s, 0, &mut d, 0, 2, 3, 1, 3, 1, 1, 3, 1, 1);
        assert_eq!(d, s);
    }

    #[test]
    fn copy_area8_s16_bias() {
        let s = vec![0u8, 255, 128];
        let mut d = vec![0i16; 3];
        copy_area8_s16(&s, 0, &mut d, 0, 1, 3, 1, 1, 1, 1, 1, 1, 1);
        assert_eq!(d[0], 0x8000u16 as i16); // 0 ^ 0x8000
        assert_eq!((d[1] as u16) ^ 0x8000, 0xFF);
    }

    #[test]
    fn resample_down16_sums() {
        // Two rows of identical value 100, weights [8192, 8192] -> 100.
        let s = vec![100u16; 16]; // two rows
        let w = vec![8192i16, 8192];
        let mut d = vec![0u16; 8];
        resample_down16(&s, 0, &mut d, 0, 8, 8, &w, 2, 65535);
        assert!(d.iter().all(|&v| (v as i32 - 100).abs() <= 1));
    }

    #[test]
    fn resample_across16_identity_coords() {
        let s = vec![50u16; 16];
        // coord for output j: pixel j, fract 0.
        let coord: Vec<int32> = (0..8).map(|j| j * 128).collect();
        let w = vec![16384i16; 8]; // one row of 16384
        let mut d = vec![0u16; 8];
        resample_across16(&s, 0, &mut d, 0, 8, &coord, &w, 1, 8, 65535);
        assert!(d.iter().all(|&v| v == 50));
    }

    #[test]
    fn resample32_identity() {
        let s: Vec<real32> = (0..16).map(|i| i as f32 / 20.0).collect();
        let coord: Vec<int32> = (0..8).map(|j| j * 128).collect();
        let w = vec![1.0f32; 8];
        let mut d = vec![0f32; 8];
        resample_across32(&s, 0, &mut d, 0, 8, &coord, &w, 1, 8);
        for j in 0..8 {
            assert!((d[j] - s[j]).abs() < 1e-6);
        }
    }

    #[test]
    fn equal_area_detection() {
        let a = vec![1u8, 2, 3, 4];
        let b = vec![1u8, 2, 3, 5];
        assert!(equal_area8(&a, 0, &a, 0, 2, 2, 1, 2, 1, 1, 2, 1, 1));
        assert!(!equal_area8(&a, 0, &b, 0, 2, 2, 1, 2, 1, 1, 2, 1, 1));
    }

    #[test]
    fn shift_right() {
        let mut d = vec![0xFFFFu16; 4];
        shift_right16(&mut d, 0, 2, 2, 1, 2, 1, 1, 4);
        assert!(d.iter().all(|&v| v == 0x0FFF));
    }

    #[test]
    fn repeat_area8_works() {
        // Source is one 1x2 row [7 8]; dest is 2x2 with repeat_v=1,
        // repeat_h=2 (phase 0,0): each dest row copies the source row.
        let s = vec![7u8, 8];
        let mut d = vec![0u8; 4];
        // The C++ RefRepeatArea8 uses ONE step set for both src and dst:
        // dest is 2x2 (row_step 2), the 1-row source wraps every row.
        repeat_area8(&s, 0, &mut d, 0, 2, 2, 1, 2, 1, 1, 1, 2, 0, 0);
        assert_eq!(d, vec![7, 8, 7, 8]);
    }
}

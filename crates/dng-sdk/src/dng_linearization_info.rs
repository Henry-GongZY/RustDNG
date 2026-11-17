// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_linearization_info.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_linearization_info.h/.cpp (SDK 1.7.1) — linearization table
// and black level tag support.
//
// Porting notes:
//   * `AutoPtr<dng_memory_block>` data blocks become typed `Vec`s
//     (fLinearizationTable → Option<Vec<u16>>, fBlackDeltaH/V →
//     Option<Vec<real64>>).
//   * `dng_linearize_plane` / `dng_linearize_image` (dng_area_task subclass)
//     run serially via dng-core's `DngAreaTask` (ARCHITECTURE.md §9).
//   * Tile access uses `DngConstTileBuffer` / `DngDirtyTileBuffer` copies
//     (dng-core DESIGN.md) instead of C++ borrowed raw pointers.

use dng_core::{
    max_int32, max_real64, min_int32, min_real64, pin_real32, pin_uint16, round_int32_f64,
    round_uint32_f64, DngError, DngHost, DngPoint, DngRect, DngResult, DngSRational,
    DngStream, DngURational, ImageRef, int32, real32, real64, uint32, uint16,
};
use dng_core::dng_area_task::DngAreaTask;
use dng_core::dng_image::{DngConstTileBuffer, DngDirtyTileBuffer};
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_sdk_limits::{kMaxBlackPattern, kMaxColorPlanes, kMaxMaskedAreas,
                               kMaxStage3BlackLevelNormalized};
use dng_core::dng_tag_types::{ttByte, ttFloat, ttLong, ttShort};
use dng_core::dng_tag_values::{pcInterleaved, sfFloatingPoint};
use dng_core::dng_utils::{max_uint32, min_uint32};

use crate::dng_info::DngInfo;
use crate::dng_negative::DngNegative;

// ---------------------------------------------------------------------------
// dng_linearize_plane.
// ---------------------------------------------------------------------------

/// Port of `dng_linearize_plane` — per-plane linearization tables.
struct DngLinearizePlane {
    plane: uint32,

    active_area: DngRect,

    src_pixel_type: uint32,
    dst_pixel_type: uint32,

    real32_math: bool,

    scale: real32,

    /// `fScale_buffer` — scale LUT (uint16 / real32 / int32 depending on
    /// pixel types).
    scale_buffer_u16: Vec<uint16>,
    scale_buffer_i32: Vec<int32>,
    scale_buffer_f32: Vec<real32>,

    black_2d_rows: uint32,
    black_2d_cols: uint32,

    black_2d_i32: Vec<int32>,
    black_2d_f32: Vec<real32>,

    black_1d_rows: uint32,

    black_1d_i32: Vec<int32>,
    black_1d_f32: Vec<real32>,
}

impl DngLinearizePlane {
    /// `dng_linearize_plane (host, info, dstBlackLevel, forceClipBlackLevel,
    /// srcImage, dstImage, plane)`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        info: &DngLinearizationInfo,
        dst_black_level: uint16,
        force_clip_black_level: bool,
        src_image: &ImageRef,
        dst_image: &ImageRef,
        plane: uint32,
    ) -> DngResult<Self> {
        let src_pixel_type = src_image.read().unwrap().pixel_type();
        let dst_pixel_type = dst_image.read().unwrap().pixel_type();

        // Make sure the source pixel type is supported.

        if src_pixel_type != ttByte
            && src_pixel_type != ttShort
            && src_pixel_type != ttLong
            && src_pixel_type != ttFloat
        {
            return Err(DngError::Unsupported(
                "Unsupported source pixel type".to_string(),
            ));
        }

        if dst_pixel_type != ttShort && dst_pixel_type != ttFloat {
            return Err(DngError::Unsupported(
                "Unsupported destination pixel type".to_string(),
            ));
        }

        if src_pixel_type == ttFloat && dst_pixel_type != ttFloat {
            return Err(DngError::Unsupported(
                "Cannot convert floating point stage1 to non-floating stage2".to_string(),
            ));
        }

        // Are we using floating point math?

        let real32_math = src_pixel_type == ttLong || dst_pixel_type == ttFloat;

        // Find the scale for this plane.

        let max_black = info.max_black_level(plane);

        let min_range = info.white_level[plane as usize] - max_black;

        if min_range <= 0.0 {
            return Err(DngError::BadFormat);
        }

        let scale = 1.0 / min_range;

        let f_scale = scale as real32;

        // Calculate two-dimensional black pattern, if any.

        let mut black_2d_rows: uint32 = 0;
        let mut black_2d_cols: uint32 = 0;

        if info.black_delta_h.is_some() {
            black_2d_rows = info.black_level_repeat_rows;
            black_2d_cols = info.active_area.w();
        } else if info.black_level_repeat_cols > 1 {
            black_2d_rows = info.black_level_repeat_rows;
            black_2d_cols = info.black_level_repeat_cols;
        }

        let mut black_2d_i32: Vec<int32> = Vec::new();
        let mut black_2d_f32: Vec<real32> = Vec::new();

        if black_2d_rows != 0 {
            let count = (black_2d_rows * black_2d_cols) as usize;

            if real32_math {
                black_2d_f32 = Vec::<real32>::with_capacity(count);
                for j in 0..black_2d_rows {
                    for k in 0..black_2d_cols {
                        let mut x =
                            info.black_level[j as usize][(k % info.black_level_repeat_cols) as usize]
                                [plane as usize];

                        if let Some(delta_h) = &info.black_delta_h {
                            x += delta_h[k as usize];
                        }

                        x *= scale;

                        black_2d_f32.push(x as real32);
                    }
                }
            } else {
                black_2d_i32 = Vec::<int32>::with_capacity(count);
                for j in 0..black_2d_rows {
                    for k in 0..black_2d_cols {
                        let mut x =
                            info.black_level[j as usize][(k % info.black_level_repeat_cols) as usize]
                                [plane as usize];

                        if let Some(delta_h) = &info.black_delta_h {
                            x += delta_h[k as usize];
                        }

                        x *= scale;

                        let x = x * ((0x0FFFF - dst_black_level) * 256) as real64;

                        black_2d_i32.push(round_int32_f64(x));
                    }
                }
            }
        }

        // Calculate one-dimensional (per row) black pattern, if any.

        let mut black_1d_rows: uint32 = 0;

        if info.black_delta_v.is_some() {
            black_1d_rows = info.active_area.h();
        } else if black_2d_rows == 0
            && (info.black_level_repeat_rows > 1
                || (src_pixel_type != ttShort && src_pixel_type != ttByte))
        {
            black_1d_rows = info.black_level_repeat_rows;
        }

        let mut black_1d_i32: Vec<int32> = Vec::new();
        let mut black_1d_f32: Vec<real32> = Vec::new();

        if black_1d_rows != 0 {
            let mut all_zero = true;

            if real32_math {
                let mut v = Vec::<real32>::with_capacity(black_1d_rows as usize);
                for j in 0..black_1d_rows {
                    let mut x = 0.0;

                    if black_2d_rows == 0 {
                        x = info.black_level
                            [(j % info.black_level_repeat_rows) as usize][0][plane as usize];
                    }

                    if let Some(delta_v) = &info.black_delta_v {
                        x += delta_v[j as usize];
                    }

                    all_zero = all_zero && (x == 0.0);

                    x *= scale;

                    v.push(x as real32);
                }
                if all_zero {
                    black_1d_rows = 0;
                } else {
                    black_1d_f32 = v;
                }
            } else {
                let mut v = Vec::<int32>::with_capacity(black_1d_rows as usize);
                for j in 0..black_1d_rows {
                    let mut x = 0.0;

                    if black_2d_rows == 0 {
                        x = info.black_level
                            [(j % info.black_level_repeat_rows) as usize][0][plane as usize];
                    }

                    if let Some(delta_v) = &info.black_delta_v {
                        x += delta_v[j as usize];
                    }

                    all_zero = all_zero && (x == 0.0);

                    x *= scale;

                    let x = x * ((0x0FFFF - dst_black_level) * 256) as real64;

                    v.push(round_int32_f64(x));
                }
                if all_zero {
                    black_1d_rows = 0;
                } else {
                    black_1d_i32 = v;
                }
            }
        }

        // Calculate scale table, if any.

        let mut scale_buffer_u16: Vec<uint16> = Vec::new();
        let mut scale_buffer_i32: Vec<int32> = Vec::new();
        let mut scale_buffer_f32: Vec<real32> = Vec::new();

        if src_pixel_type != ttLong && src_pixel_type != ttFloat {
            // Find linearization table, if any.

            let lut: Option<&Vec<uint16>> = info.linearization_table.as_ref();
            let lut_entries = lut.as_ref().map_or(0, |t| t.len() as uint32);

            // If the black level does not vary from pixel to pixel, then
            // the entire process can be a single LUT.

            if black_1d_rows == 0 && black_2d_rows == 0 {
                let scale_lut_entries: uint32 = if src_pixel_type == ttByte {
                    0x100
                } else {
                    0x10000
                };

                if dst_pixel_type == ttShort {
                    let mut v = vec![0u16; scale_lut_entries as usize];
                    for j in 0..scale_lut_entries {
                        let mut x = j;

                        // Apply linearization table, if any.

                        if let Some(table) = lut {
                            x = min_uint32(x, lut_entries - 1);
                            x = table[x as usize] as uint32;
                        }

                        // Subtract constant black level.

                        let mut y = x as real64 - info.black_level[0][0][plane as usize];

                        // Apply scale.

                        y *= scale;

                        // Burn in the clipping if requested.

                        if force_clip_black_level {
                            y = y.clamp(0.0, 1.0);
                        }

                        // Store output value in table.

                        v[j as usize] = pin_uint16(round_int32_f64(
                            y * ((0x0FFFF - dst_black_level) as real64)
                                + dst_black_level as real64,
                        ));
                    }
                    scale_buffer_u16 = v;
                } else {
                    let mut v = vec![0.0f32; scale_lut_entries as usize];
                    for j in 0..scale_lut_entries {
                        let mut x = j;

                        if let Some(table) = lut {
                            x = min_uint32(x, lut_entries - 1);
                            x = table[x as usize] as uint32;
                        }

                        let mut y = x as real64 - info.black_level[0][0][plane as usize];

                        y *= scale;

                        if force_clip_black_level {
                            y = y.clamp(0.0, 1.0);
                        }

                        v[j as usize] = y as real32;
                    }
                    scale_buffer_f32 = v;
                }
            }
            // Else we only do the scaling operation in the scale table.
            else {
                if real32_math {
                    let mut v = vec![0.0f32; 0x10000];
                    for j in 0..0x10000u32 {
                        let mut x = j;

                        if let Some(table) = lut {
                            x = min_uint32(x, lut_entries - 1);
                            x = table[x as usize] as uint32;
                        }

                        let y = x as real64 * scale;

                        v[j as usize] = y as real32;
                    }
                    scale_buffer_f32 = v;
                } else {
                    let mut v = vec![0i32; 0x10000];
                    for j in 0..0x10000u32 {
                        let mut x = j;

                        if let Some(table) = lut {
                            x = min_uint32(x, lut_entries - 1);
                            x = table[x as usize] as uint32;
                        }

                        let y = x as real64 * scale;

                        v[j as usize] = round_int32_f64(
                            (y * ((0x0FFFF - dst_black_level) as real64)
                                + dst_black_level as real64)
                                * 256.0,
                        );
                    }
                    scale_buffer_i32 = v;
                }
            }
        }

        Ok(DngLinearizePlane {
            plane,
            active_area: info.active_area,
            src_pixel_type,
            dst_pixel_type,
            real32_math,
            scale: f_scale,
            scale_buffer_u16,
            scale_buffer_i32,
            scale_buffer_f32,
            black_2d_rows,
            black_2d_cols,
            black_2d_i32,
            black_2d_f32,
            black_1d_rows,
            black_1d_i32,
            black_1d_f32,
        })
    }

    fn black_1d_f32(&self, row: uint32) -> real32 {
        self.black_1d_f32[(row % self.black_1d_rows) as usize]
    }

    fn black_1d_i32(&self, row: uint32) -> int32 {
        self.black_1d_i32[(row % self.black_1d_rows) as usize]
    }

    fn black_2d_f32_row(&self, row: uint32) -> &[real32] {
        let start = (self.black_2d_cols * (row % self.black_2d_rows)) as usize;
        &self.black_2d_f32[start..start + self.black_2d_cols as usize]
    }

    fn black_2d_i32_row(&self, row: uint32) -> &[int32] {
        let start = (self.black_2d_cols * (row % self.black_2d_rows)) as usize;
        &self.black_2d_i32[start..start + self.black_2d_cols as usize]
    }

    /// `Process (srcTile)`.
    fn process(&self, src_image: &ImageRef, dst_image: &ImageRef, src_tile: DngRect) -> DngResult<()> {
        // Process tile.

        let dst_tile = src_tile.sub_offset(DngPoint::new(self.active_area.t, self.active_area.l));

        let src_buffer = DngConstTileBuffer::new(&*src_image.read().unwrap(), src_tile)?;
        let mut dst_buffer =
            DngDirtyTileBuffer::new(&*dst_image.read().unwrap(), dst_tile)?;

        let count = src_tile.w() as usize;

        let dst_col = dst_tile.l;

        let rows = src_tile.h();

        for row in 0..rows {
            let dst_row = dst_tile.t + row as int32;

            let src_row = src_tile.t + row as int32;

            // Floating point source case.

            if self.src_pixel_type == ttFloat {
                let scale = self.scale;

                let s_step = src_buffer.buffer.col_step;
                let d_step = dst_buffer.buffer.col_step;

                let src_idx0 =
                    src_buffer
                        .buffer
                        .pixel_element_index_public(src_row, src_tile.l, self.plane)?;
                let dst_idx0 =
                    dst_buffer
                        .buffer
                        .pixel_element_index_public(dst_row, dst_col, self.plane)?;

                let src = src_buffer.buffer.as_f32_slice().unwrap();
                let dst = dst_buffer.buffer.as_f32_slice_mut().unwrap();

                // Optimize scale only case, which is the most common.

                if self.black_1d_rows == 0 && self.black_2d_cols == 0 {
                    let mut si = src_idx0;
                    let mut di = dst_idx0;
                    for _ in 0..count {
                        dst[di] = src[si] * scale;
                        si += s_step as usize;
                        di += d_step as usize;
                    }
                } else {
                    let b1 = if self.black_1d_rows != 0 {
                        self.black_1d_f32(dst_row as uint32)
                    } else {
                        0.0
                    };

                    let b2_row = if self.black_2d_cols != 0 {
                        Some(self.black_2d_f32_row(dst_row as uint32))
                    } else {
                        None
                    };

                    let mut b2_phase = if self.black_2d_cols != 0 {
                        (dst_col as uint32 % self.black_2d_cols) as usize
                    } else {
                        0
                    };

                    let mut si = src_idx0;
                    let mut di = dst_idx0;
                    for _ in 0..count {
                        let mut x = src[si] * scale - b1;

                        if let Some(b2) = b2_row {
                            x -= b2[b2_phase];
                            b2_phase += 1;
                            if b2_phase == b2.len() {
                                b2_phase = 0;
                            }
                        }

                        dst[di] = x;
                        si += s_step as usize;
                        di += d_step as usize;
                    }
                }
            }
            // Simple LUT case.
            else if self.black_1d_rows == 0
                && self.black_2d_rows == 0
                && self.src_pixel_type != ttLong
            {
                let s_step = src_buffer.buffer.col_step;
                let d_step = dst_buffer.buffer.col_step;

                let src_idx0 =
                    src_buffer
                        .buffer
                        .pixel_element_index_public(src_row, src_tile.l, self.plane)?;
                let dst_idx0 =
                    dst_buffer
                        .buffer
                        .pixel_element_index_public(dst_row, dst_col, self.plane)?;

                if self.dst_pixel_type == ttShort {
                    let lut = &self.scale_buffer_u16;

                    let dst = dst_buffer.buffer.as_u16_slice_mut().unwrap();

                    if self.src_pixel_type == ttByte {
                        let src = src_buffer.buffer.as_u8_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            dst[di] = lut[src[si] as usize];
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    } else {
                        let src = src_buffer.buffer.as_u16_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            dst[di] = lut[src[si] as usize];
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    }
                } else {
                    let lut = &self.scale_buffer_f32;
                    let dst = dst_buffer.buffer.as_f32_slice_mut().unwrap();

                    if self.src_pixel_type == ttByte {
                        let src = src_buffer.buffer.as_u8_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            dst[di] = lut[src[si] as usize];
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    } else {
                        let src = src_buffer.buffer.as_u16_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            dst[di] = lut[src[si] as usize];
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    }
                }
            }
            // Integer math case.
            else if !self.real32_math {
                let lut = &self.scale_buffer_i32;

                let mut b1 = if self.black_1d_rows != 0 {
                    self.black_1d_i32(dst_row as uint32)
                } else {
                    0
                };

                let b2_row = if self.black_2d_cols != 0 {
                    Some(self.black_2d_i32_row(dst_row as uint32))
                } else {
                    None
                };

                let mut b2_phase = if self.black_2d_cols != 0 {
                    (dst_col as uint32 % self.black_2d_cols) as usize
                } else {
                    0
                };

                let s_step = src_buffer.buffer.col_step;
                let d_step = dst_buffer.buffer.col_step;

                let src_idx0 =
                    src_buffer
                        .buffer
                        .pixel_element_index_public(src_row, src_tile.l, self.plane)?;
                let dst_idx0 =
                    dst_buffer
                        .buffer
                        .pixel_element_index_public(dst_row, dst_col, self.plane)?;

                let dst = dst_buffer.buffer.as_u16_slice_mut().unwrap();

                b1 -= 128; // Rounding for 8 bit shift

                if self.src_pixel_type == ttByte {
                    let src = src_buffer.buffer.as_u8_slice().unwrap();
                    let mut si = src_idx0;
                    let mut di = dst_idx0;
                    for _ in 0..count {
                        let mut x = lut[src[si] as usize] - b1;

                        if let Some(b2) = b2_row {
                            x -= b2[b2_phase];
                            b2_phase += 1;
                            if b2_phase == b2.len() {
                                b2_phase = 0;
                            }
                        }

                        x >>= 8;

                        dst[di] = pin_uint16(x);
                        si += s_step as usize;
                        di += d_step as usize;
                    }
                } else {
                    let src = src_buffer.buffer.as_u16_slice().unwrap();
                    let mut si = src_idx0;
                    let mut di = dst_idx0;
                    for _ in 0..count {
                        let mut x = lut[src[si] as usize] - b1;

                        if let Some(b2) = b2_row {
                            x -= b2[b2_phase];
                            b2_phase += 1;
                            if b2_phase == b2.len() {
                                b2_phase = 0;
                            }
                        }

                        x >>= 8;

                        dst[di] = pin_uint16(x);
                        si += s_step as usize;
                        di += d_step as usize;
                    }
                }
            }
            // Floating point math cases.
            else {
                let b1 = if self.black_1d_rows != 0 {
                    self.black_1d_f32(dst_row as uint32)
                } else {
                    0.0
                };

                let b2_row = if self.black_2d_cols != 0 {
                    Some(self.black_2d_f32_row(dst_row as uint32))
                } else {
                    None
                };

                let mut b2_phase = if self.black_2d_cols != 0 {
                    (dst_col as uint32 % self.black_2d_cols) as usize
                } else {
                    0
                };

                let s_step = src_buffer.buffer.col_step;
                let d_step = dst_buffer.buffer.col_step;

                let src_idx0 =
                    src_buffer
                        .buffer
                        .pixel_element_index_public(src_row, src_tile.l, self.plane)?;
                let dst_idx0 =
                    dst_buffer
                        .buffer
                        .pixel_element_index_public(dst_row, dst_col, self.plane)?;

                // Case 1: uint8/uint16 -> real32

                if self.src_pixel_type != ttLong {
                    let lut = &self.scale_buffer_f32;
                    let dst = dst_buffer.buffer.as_f32_slice_mut().unwrap();

                    if self.src_pixel_type == ttByte {
                        let src = src_buffer.buffer.as_u8_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            let mut x = lut[src[si] as usize] - b1;

                            if let Some(b2) = b2_row {
                                x -= b2[b2_phase];
                                b2_phase += 1;
                                if b2_phase == b2.len() {
                                    b2_phase = 0;
                                }
                            }

                            x = pin_real32(0.0, x, 1.0);
                            dst[di] = x;
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    } else {
                        let src = src_buffer.buffer.as_u16_slice().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            let mut x = lut[src[si] as usize] - b1;

                            if let Some(b2) = b2_row {
                                x -= b2[b2_phase];
                                b2_phase += 1;
                                if b2_phase == b2.len() {
                                    b2_phase = 0;
                                }
                            }

                            x = pin_real32(0.0, x, 1.0);
                            dst[di] = x;
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    }
                }
                // Otherwise source is uint32
                else {
                    let scale = self.scale;
                    let src = src_buffer.buffer.as_u32_slice().unwrap();

                    // Case 2: uint32 -> real32

                    if self.dst_pixel_type == ttFloat {
                        let dst = dst_buffer.buffer.as_f32_slice_mut().unwrap();
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            let mut x = (src[si] as real32) * scale - b1;

                            if let Some(b2) = b2_row {
                                x -= b2[b2_phase];
                                b2_phase += 1;
                                if b2_phase == b2.len() {
                                    b2_phase = 0;
                                }
                            }

                            x = pin_real32(0.0, x, 1.0);
                            dst[di] = x;
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    }
                    // Case 3: uint32 -> uint16
                    else {
                        let dst = dst_buffer.buffer.as_u16_slice_mut().unwrap();
                        let dst_scale: real32 = 0x0FFFF as real32;
                        let mut si = src_idx0;
                        let mut di = dst_idx0;
                        for _ in 0..count {
                            let mut x = (src[si] as real32) * scale - b1;

                            if let Some(b2) = b2_row {
                                x -= b2[b2_phase];
                                b2_phase += 1;
                                if b2_phase == b2.len() {
                                    b2_phase = 0;
                                }
                            }

                            x = pin_real32(0.0, x, 1.0);
                            dst[di] = (x * dst_scale + 0.5) as uint16;
                            si += s_step as usize;
                            di += d_step as usize;
                        }
                    }
                }
            }
        }

        dst_buffer.flush(&*dst_image.read().unwrap())?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_linearize_image (area task).
// ---------------------------------------------------------------------------

/// Port of `dng_linearize_image` — serial area task.
struct DngLinearizeImage {
    src_image: ImageRef,
    dst_image: ImageRef,
    active_area: DngRect,
    plane_tasks: Vec<Option<DngLinearizePlane>>,
    max_tile_size: DngPoint,
}

impl DngLinearizeImage {
    fn new(
        info: &DngLinearizationInfo,
        dst_black_level: uint16,
        force_clip_black_level: bool,
        src_image: ImageRef,
        dst_image: ImageRef,
    ) -> DngResult<Self> {
        let planes = src_image.read().unwrap().planes();

        let mut plane_tasks = Vec::with_capacity(planes as usize);

        for plane in 0..planes {
            plane_tasks.push(Some(DngLinearizePlane::new(
                info,
                dst_black_level,
                force_clip_black_level,
                &src_image,
                &dst_image,
                plane,
            )?));
        }

        Ok(DngLinearizeImage {
            src_image,
            dst_image,
            active_area: info.active_area,
            plane_tasks,
            max_tile_size: DngPoint::new(1024, 1024),
        })
    }
}

impl DngAreaTask for DngLinearizeImage {
    fn name(&self) -> &str {
        "dng_linearization_image"
    }

    fn max_tile_size(&self) -> DngPoint {
        self.max_tile_size
    }

    fn repeating_tile1(&self) -> DngRect {
        self.src_image.read().unwrap().repeating_tile()
    }

    fn repeating_tile2(&self) -> DngRect {
        self.dst_image
            .read()
            .unwrap()
            .repeating_tile()
            .add_offset(DngPoint::new(self.active_area.t, self.active_area.l))
    }

    fn process(
        &self,
        _thread_index: uint32,
        src_tile: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        // Process each plane.

        for plane_task in self.plane_tasks.iter() {
            if let Some(task) = plane_task {
                task.process(&self.src_image, &self.dst_image, src_tile)?;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_linearization_info.
// ---------------------------------------------------------------------------

/// Port of `dng_linearization_info`.
#[derive(Debug, Clone)]
pub struct DngLinearizationInfo {
    /// `fActiveArea` — active (non-masked) pixels of the sensor.
    /// Coordinate order: top, left, bottom, right.
    pub active_area: DngRect,

    /// `fMaskedAreaCount`.
    pub masked_area_count: uint32,

    /// `fMaskedArea` — list of non-overlapping rectangles of fully masked
    /// pixels.
    pub masked_area: Vec<DngRect>,

    /// `fLinearizationTable` — lookup table mapping stored values into
    /// linear values.
    pub linearization_table: Option<Vec<uint16>>,

    /// `fBlackLevelRepeatRows` — actual number of rows in the black level
    /// pattern.
    pub black_level_repeat_rows: uint32,

    /// `fBlackLevelRepeatCols` — actual number of columns in the black level
    /// pattern.
    pub black_level_repeat_cols: uint32,

    /// `fBlackLevel` — repeating pattern of black levels
    /// [row][col][plane].
    pub black_level: Vec<Vec<Vec<real64>>>,

    /// `fBlackDeltaH` — per-column black level deltas.
    pub black_delta_h: Option<Vec<real64>>,

    /// `fBlackDeltaV` — per-row black level deltas.
    pub black_delta_v: Option<Vec<real64>>,

    /// `fWhiteLevel` — single white level (maximum sensor value) for each
    /// sample plane.
    pub white_level: Vec<real64>,

    /// `fBlackDenom` (protected in C++).
    pub(crate) black_denom: uint32,

    /// Convenience view of the active masked areas (kept in sync with
    /// `masked_area`/`masked_area_count`); consumed by the writer (B).
    pub masked_areas: Vec<DngRect>,
}

impl Default for DngLinearizationInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl DngLinearizationInfo {
    /// `dng_linearization_info ()`.
    pub fn new() -> Self {
        DngLinearizationInfo {
            active_area: DngRect::new(),
            masked_area_count: 0,
            masked_area: vec![DngRect::new(); kMaxMaskedAreas as usize],
            linearization_table: None,
            black_level_repeat_rows: 1,
            black_level_repeat_cols: 1,
            black_level: vec![
                vec![vec![0.0; kMaxColorPlanes as usize]; kMaxBlackPattern as usize];
                kMaxBlackPattern as usize
            ],
            black_delta_h: None,
            black_delta_v: None,
            white_level: vec![65535.0; kMaxColorPlanes as usize],
            black_denom: 256,
            masked_areas: Vec::new(),
        }
    }

    /// `RoundBlacks ()`.
    pub fn round_blacks(&mut self) {
        let mut max_abs = 0.0;

        for j in 0..self.black_level_repeat_rows as usize {
            for k in 0..self.black_level_repeat_cols as usize {
                for n in 0..kMaxColorPlanes as usize {
                    max_abs = max_real64(max_abs, self.black_level[j][k][n].abs());
                }
            }
        }

        let count = self.row_black_count();

        if let Some(delta_v) = &self.black_delta_v {
            for j in 0..count as usize {
                max_abs = max_real64(max_abs, delta_v[j].abs());
            }
        }

        let count = self.column_black_count();

        if let Some(delta_h) = &self.black_delta_h {
            for j in 0..count as usize {
                max_abs = max_real64(max_abs, delta_h[j].abs());
            }
        }

        self.black_denom = 256;

        while self.black_denom > 1 && (max_abs * self.black_denom as real64) >= 30000.0 * 65536.0 {
            self.black_denom >>= 1;
        }

        for j in 0..self.black_level_repeat_rows as usize {
            for k in 0..self.black_level_repeat_cols as usize {
                for n in 0..kMaxColorPlanes as usize {
                    self.black_level[j][k][n] = self.black_level(j as uint32, k as uint32, n as uint32).as_real64();
                }
            }
        }

        let count = self.row_black_count();

        if let Some(mut delta_v) = self.black_delta_v.take() {
            for j in 0..count as usize {
                let mut r = DngSRational::default();
                r.set_real64(delta_v[j], self.black_denom as int32);

                delta_v[j] = r.as_real64();
            }

            self.black_delta_v = Some(delta_v);
        }

        let count = self.column_black_count();

        if let Some(mut delta_h) = self.black_delta_h.take() {
            for j in 0..count as usize {
                let mut r = DngSRational::default();
                r.set_real64(delta_h[j], self.black_denom as int32);

                delta_h[j] = r.as_real64();
            }

            self.black_delta_h = Some(delta_h);
        }
    }

    /// `Parse (host, stream, info)`.
    pub fn parse(
        &mut self,
        _host: &mut DngHost,
        stream: &mut DngStream,
        info: &DngInfo,
    ) -> DngResult<()> {
        // Find main image IFD.

        let main_index = info.main_index as usize;

        let raw_ifd = info.ifd[main_index].clone();

        // Copy active area.

        self.active_area = raw_ifd.active_area;

        // Copy masked areas.

        self.masked_area_count = min_uint32(raw_ifd.masked_area_count, kMaxMaskedAreas);

        for j in 0..self.masked_area_count as usize {
            self.masked_area[j] = raw_ifd.masked_area[j];
        }

        self.masked_areas = self.masked_area[..self.masked_area_count as usize].to_vec();

        // Read linearization LUT.

        if raw_ifd.linearization_table_count != 0 {
            let mut table = Vec::with_capacity(raw_ifd.linearization_table_count as usize);

            stream.set_read_position(raw_ifd.linearization_table_offset)?;

            for _ in 0..raw_ifd.linearization_table_count {
                table.push(stream.get_uint16()?);
            }

            self.linearization_table = Some(table);
        }

        // Copy black level pattern.

        self.black_level_repeat_rows = raw_ifd.black_level_repeat_rows;
        self.black_level_repeat_cols = raw_ifd.black_level_repeat_cols;

        if !(self.black_level_repeat_rows >= 1
            && self.black_level_repeat_rows <= kMaxBlackPattern)
        {
            return Err(DngError::Unsupported(
                "Invalid fBlackLevelRepeatRows".to_string(),
            ));
        }

        if !(self.black_level_repeat_cols >= 1
            && self.black_level_repeat_cols <= kMaxBlackPattern)
        {
            return Err(DngError::Unsupported(
                "Invalid fBlackLevelRepeatCols".to_string(),
            ));
        }

        for j in 0..kMaxBlackPattern as usize {
            for k in 0..kMaxBlackPattern as usize {
                for n in 0..kMaxColorPlanes as usize {
                    self.black_level[j][k][n] = raw_ifd.black_level[j][k][n];
                }
            }
        }

        // Read BlackDeltaH.

        if raw_ifd.black_level_delta_h_count != 0 {
            let mut blacks = Vec::with_capacity(raw_ifd.black_level_delta_h_count as usize);

            stream.set_read_position(raw_ifd.black_level_delta_h_offset)?;

            for _ in 0..raw_ifd.black_level_delta_h_count {
                blacks.push(stream.tag_value_real64(raw_ifd.black_level_delta_h_type)?);
            }

            self.black_delta_h = Some(blacks);
        }

        // Read BlackDeltaV.

        if raw_ifd.black_level_delta_v_count != 0 {
            let mut blacks = Vec::with_capacity(raw_ifd.black_level_delta_v_count as usize);

            stream.set_read_position(raw_ifd.black_level_delta_v_offset)?;

            for _ in 0..raw_ifd.black_level_delta_v_count {
                blacks.push(stream.tag_value_real64(raw_ifd.black_level_delta_v_type)?);
            }

            self.black_delta_v = Some(blacks);
        }

        // Copy white level.

        for n in 0..kMaxColorPlanes as usize {
            self.white_level[n] = raw_ifd.white_level[n];
        }

        // Round off black levels.

        self.round_blacks();

        Ok(())
    }

    /// `PostParse (host, negative)`.
    pub fn post_parse(&mut self, _host: &mut DngHost, negative: &DngNegative) {
        if self.active_area.is_empty() {
            if let Some(stage1) = negative.stage1_image() {
                self.active_area = stage1.read().unwrap().bounds();
            }
        }
    }

    /// `MaxBlackLevel (plane)` — maximum black level for a given sample
    /// plane taking into account base black level, repeated black level
    /// pattern, and row/column delta maps.
    pub fn max_black_level(&self, plane: uint32) -> real64 {
        // Find maximum value of fBlackDeltaH for each phase of black pattern.

        let mut max_delta_h = vec![0.0; kMaxBlackPattern as usize];

        if let Some(table) = &self.black_delta_h {
            let entries = table.len() as uint32;

            for j in 0..entries {
                let entry = &mut max_delta_h[(j % self.black_level_repeat_cols) as usize];
                if j < self.black_level_repeat_cols {
                    *entry = table[j as usize];
                } else {
                    *entry = max_real64(*entry, table[j as usize]);
                }
            }
        }

        // Find maximum value of fBlackDeltaV for each phase of black pattern.

        let mut max_delta_v = vec![0.0; kMaxBlackPattern as usize];

        if let Some(table) = &self.black_delta_v {
            let entries = table.len() as uint32;

            for j in 0..entries {
                let entry = &mut max_delta_v[(j % self.black_level_repeat_rows) as usize];
                if j < self.black_level_repeat_rows {
                    *entry = table[j as usize];
                } else {
                    *entry = max_real64(*entry, table[j as usize]);
                }
            }
        }

        // Now scan the pattern and find the maximum value after row and
        // column deltas.

        let mut max_black = 0.0;

        for j in 0..self.black_level_repeat_rows as usize {
            for k in 0..self.black_level_repeat_cols as usize {
                let mut black = self.black_level[j][k][plane as usize];

                black += max_delta_h[k];
                black += max_delta_v[j];

                if j == 0 && k == 0 {
                    max_black = black;
                } else {
                    max_black = max_real64(max_black, black);
                }
            }
        }

        max_black
    }

    /// `Linearize (host, negative, srcImage, dstImage)`.
    pub fn linearize(
        &self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        src_image: &ImageRef,
        dst_image: &ImageRef,
    ) -> DngResult<()> {
        let allow_preserve_black_levels = negative.supports_preserved_black_levels(host);

        if allow_preserve_black_levels
            && negative.is_scene_referred()
            && dst_image.read().unwrap().pixel_type() == ttShort
        {
            let mut zero_fract = 0.0;

            for plane in 0..src_image.read().unwrap().planes() {
                let max_black_level = self.max_black_level(plane);
                let white_level = self.white_level[plane as usize];

                if max_black_level > 0.0 && max_black_level < white_level {
                    zero_fract = max_real64(zero_fract, max_black_level / white_level);
                }
            }

            zero_fract = min_real64(zero_fract, kMaxStage3BlackLevelNormalized);

            let mut dst_black_level = round_uint32_f64(65535.0 * zero_fract) as uint16;

            if negative.get_mosaic_info().is_some() {
                // If we have a mosaic image that supports non-zero black
                // levels, enforce a minimum black level to give the demosaic
                // algorithms some "footroom".

                dst_black_level = max_uint32(dst_black_level as uint32, 0x0404) as uint16;
            }

            negative.set_stage3_black_level(dst_black_level);
        }

        let force_clip_black_level = !allow_preserve_black_levels;

        let mut processor = DngLinearizeImage::new(
            self,
            negative.stage3_black_level(),
            force_clip_black_level,
            src_image.clone(),
            dst_image.clone(),
        )?;

        host.perform_area_task(&mut processor, self.active_area)?;

        Ok(())
    }

    /// `BlackLevel (row, col, plane)` — compute black level for one
    /// coordinate and sample plane in the image.
    pub fn black_level(&self, row: uint32, col: uint32, plane: uint32) -> DngURational {
        let mut r = DngURational::default();

        r.set_real64(
            self.black_level[row as usize][col as usize][plane as usize],
            self.black_denom,
        );

        r
    }

    /// `RowBlackCount ()` — number of per-row black level deltas.
    pub fn row_black_count(&self) -> uint32 {
        match &self.black_delta_v {
            Some(v) => v.len() as uint32,
            None => 0,
        }
    }

    /// `RowBlack (row)` — lookup black level delta for a given row.
    pub fn row_black(&self, row: uint32) -> DngSRational {
        if let Some(v) = &self.black_delta_v {
            if (row as usize) < v.len() {
                let mut r = DngSRational::default();
                r.set_real64(v[row as usize], self.black_denom as int32);
                return r;
            }
        }

        DngSRational::new(0, 1)
    }

    /// `ColumnBlackCount ()` — number of per-column black level deltas.
    pub fn column_black_count(&self) -> uint32 {
        match &self.black_delta_h {
            Some(v) => v.len() as uint32,
            None => 0,
        }
    }

    /// `ColumnBlack (col)` — lookup black level delta for a given column.
    pub fn column_black(&self, col: uint32) -> DngSRational {
        if let Some(v) = &self.black_delta_h {
            if (col as usize) < v.len() {
                let mut r = DngSRational::default();
                r.set_real64(v[col as usize], self.black_denom as int32);
                return r;
            }
        }

        DngSRational::new(0, 1)
    }
}


#[allow(unused)]
fn unused_helpers() {
    let _ = (max_int32, min_int32, sfFloatingPoint, pcInterleaved, ttByte);
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_simple_image::DngSimpleImage;
    use std::sync::{Arc, RwLock};

    fn make_image(w: uint32, h: uint32, planes: uint32, pixel_type: uint32) -> ImageRef {
        let img = DngSimpleImage::new(
            DngRect::from_corners(0, 0, h as int32, w as int32),
            planes,
            pixel_type,
        )
        .unwrap();
        Arc::new(RwLock::new(img)) as ImageRef
    }

    #[test]
    fn defaults_match_cpp() {
        let info = DngLinearizationInfo::new();

        assert!(info.active_area.is_empty());
        assert_eq!(info.masked_area_count, 0);
        assert!(info.linearization_table.is_none());
        assert_eq!(info.black_level_repeat_rows, 1);
        assert_eq!(info.black_level_repeat_cols, 1);
        assert_eq!(info.black_level[0][0][0], 0.0);
        assert_eq!(info.white_level[0], 65535.0);
        assert_eq!(info.black_denom, 256);
    }

    #[test]
    fn max_black_level_with_pattern() {
        let mut info = DngLinearizationInfo::new();

        // 2x2 pattern with different blacks, no deltas.
        info.black_level_repeat_rows = 2;
        info.black_level_repeat_cols = 2;

        info.black_level[0][0][0] = 10.0;
        info.black_level[0][1][0] = 20.0;
        info.black_level[1][0][0] = 30.0;
        info.black_level[1][1][0] = 40.0;

        assert!((info.max_black_level(0) - 40.0).abs() < 1e-9);
        assert!((info.max_black_level(1) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn max_black_level_with_deltas() {
        let mut info = DngLinearizationInfo::new();

        info.black_level_repeat_rows = 2;
        info.black_level_repeat_cols = 2;

        info.black_level[0][0][0] = 0.0;
        info.black_level[1][1][0] = 10.0;

        info.black_delta_h = Some(vec![1.0, 2.0, 5.0]);
        info.black_delta_v = Some(vec![3.0, 4.0]);

        // maxDeltaH per col phase: col0=max(1,5)=5, col1=2.
        // maxDeltaV per row phase: row0=3, row1=4.
        // blacks: (0,0)=0+5+3=8; (0,1)=0+2+3=5; (1,0)=0+5+4=9; (1,1)=10+2+4=16.
        assert!((info.max_black_level(0) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn round_blacks_quantizes() {
        let mut info = DngLinearizationInfo::new();

        info.black_level[0][0][0] = 0.123456789;

        info.round_blacks();

        // Value quantized through the fBlackDenom rational.
        let r = info.black_level(0, 0, 0);
        assert_eq!(info.black_level[0][0][0], r.as_real64());
    }

    #[test]
    fn linearize_short_without_table() {
        // Constant black level of 0: a pure identity scale for ttShort.
        let mut info = DngLinearizationInfo::new();
        info.active_area = DngRect::from_corners(0, 0, 4, 4);

        let mut host = DngHost::new();
        let mut negative = DngNegative::new(&mut host).unwrap();

        let src = make_image(4, 4, 1, ttShort);
        let dst = make_image(4, 4, 1, ttShort);

        // Fill source with a ramp.
        {
            let mut img = src.write().unwrap();
            let area = img.bounds();
            let mut buf = DngPixelBuffer::with_planar_config(
                area,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
            )
            .unwrap();
            {
                let data = buf.as_u16_slice_mut().unwrap();
                for (j, v) in data.iter_mut().enumerate() {
                    *v = (j % 1000) as u16;
                }
            }
            img.do_put(&buf).unwrap();
        }

        info.linearize(&mut host, &mut negative, &src, &dst).unwrap();

        // Check a few destination pixels: value 0..16 maps to itself
        // (black level 0, white level 65535 => scale 1/65535 then
        // multiplied by 65535 in the LUT burn-in).
        let mut img = dst.write().unwrap();
        let area = img.bounds();
        let mut buf = DngPixelBuffer::with_planar_config(
            area,
            0,
            1,
            ttShort,
            pcInterleaved,
            DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
        )
        .unwrap();
        img.do_get(&mut buf).unwrap();

        let data = buf.as_u16_slice().unwrap();
        // 4x4 image holds values 0..=15; identity mapping (black 0, white
        // 65535) preserves them.
        assert_eq!(data.len(), 16);
        assert_eq!(data[0], 0);
        assert_eq!(data[1], 1);
        assert_eq!(data[15], 15);
    }

    #[test]
    fn linearize_with_black_and_scale() {
        // Black level 1000, white 65535: value 1000 must map to 0.
        let mut info = DngLinearizationInfo::new();
        info.active_area = DngRect::from_corners(0, 0, 4, 4);
        info.black_level[0][0][0] = 1000.0;

        let mut host = DngHost::new();
        let mut negative = DngNegative::new(&mut host).unwrap();

        let src = make_image(4, 4, 1, ttShort);
        let dst = make_image(4, 4, 1, ttShort);

        {
            let mut img = src.write().unwrap();
            let area = img.bounds();
            let mut buf = DngPixelBuffer::with_planar_config(
                area,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
            )
            .unwrap();
            {
                let data = buf.as_u16_slice_mut().unwrap();
                data[0] = 1000;
                data[1] = 65535;
            }
            img.do_put(&buf).unwrap();
        }

        info.linearize(&mut host, &mut negative, &src, &dst).unwrap();

        let mut img = dst.write().unwrap();
        let area = img.bounds();
        let mut buf = DngPixelBuffer::with_planar_config(
            area,
            0,
            1,
            ttShort,
            pcInterleaved,
            DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
        )
        .unwrap();
        img.do_get(&mut buf).unwrap();

        let data = buf.as_u16_slice().unwrap();
        assert_eq!(data[0], 0);
        assert_eq!(data[1], 0x0FFFF);
    }

    #[test]
    fn row_and_column_black() {
        let mut info = DngLinearizationInfo::new();

        assert_eq!(info.row_black_count(), 0);
        assert_eq!(info.column_black_count(), 0);
        assert_eq!(info.row_black(0), DngSRational::new(0, 1));

        info.black_delta_v = Some(vec![10.0, -20.0]);
        info.black_delta_h = Some(vec![2.5]);

        info.round_blacks();

        assert_eq!(info.row_black_count(), 2);
        assert_eq!(info.column_black_count(), 1);
        assert_eq!(info.row_black(0).as_real64(), 10.0);
        assert_eq!(info.row_black(1).as_real64(), -20.0);
        assert_eq!(info.column_black(0).as_real64(), 2.5);
    }
}

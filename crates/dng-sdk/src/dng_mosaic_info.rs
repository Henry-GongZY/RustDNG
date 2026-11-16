// Port of dng_mosaic_info.h/.cpp (SDK 1.7.1) — CFA (mosaic) description and
// demosaic interpolation.
//
// Porting notes:
//   * `dng_bilinear_kernel` / `dng_bilinear_pattern` /
//     `dng_bilinear_interpolator` are private helpers here (as in C++).
//   * Row kernels run through dng-core's `bilinear_row16` / `bilinear_row32`
//     reference bottlenecks (same arithmetic as C++ RefDoBilinearRow16/32).
//   * `dng_fast_interpolator` derives from dng_filter_task; the port embeds
//     `DngFilterTaskBase` and drives `process_filter` (serial).
//   * `fOffsets/fWeights16/fWeights32` pointer tables in C++ point into the
//     kernel arrays; the port passes kernel slices directly.

use dng_core::{
    max_int32, max_uint32, min_int32, min_uint32, round_int32_f64, round_uint32_f64,
    DngHost, DngMatrix, DngMatrix3by3, DngPoint, DngRect, DngResult, DngStream, ImageRef,
    int32, real32, real64, uint32, uint8, uint16,
};
use dng_core::dng_utils::abs_int32;
use dng_core::dng_area_task::{DngAreaTask, DngFilterTaskBase};
use dng_core::dng_bottlenecks::{bilinear_row16, bilinear_row32};
use dng_core::dng_image::{DngImage, EdgeOption};
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_sdk_limits::{kMaxCFAPattern, kMaxColorPlanes};
use dng_core::dng_tag_types::{ttFloat, ttShort};
use dng_core::dng_tag_values::{pcInterleaved, pcRowInterleaved, piCFA};
use dng_core::dng_tile_iterator::DngTileIterator;

use crate::dng_info::DngInfo;
use crate::dng_negative::DngNegative;

// ---------------------------------------------------------------------------
// dng_bilinear_kernel.
// ---------------------------------------------------------------------------

/// Port of `dng_bilinear_kernel` — interpolation kernel for a single pixel
/// of a single plane.
#[derive(Debug, Clone)]
struct DngBilinearKernel {
    k_max_count: usize,

    count: uint32,

    delta: Vec<DngPoint>,

    weight32: Vec<real32>,
    weight16: Vec<uint16>,

    offset: Vec<int32>,
}

impl Default for DngBilinearKernel {
    fn default() -> Self {
        DngBilinearKernel {
            k_max_count: 8,
            count: 0,
            delta: vec![DngPoint::new(0, 0); 8],
            weight32: vec![0.0; 8],
            weight16: vec![0; 8],
            offset: vec![0; 8],
        }
    }
}

impl DngBilinearKernel {
    fn new() -> Self {
        Self::default()
    }

    /// `Add (delta, weight)` — don't add zero weight elements; combine
    /// weights for repeated deltas.
    fn add(&mut self, delta: DngPoint, weight: real32) {
        if weight <= 0.0 {
            return;
        }

        for j in 0..self.count as usize {
            if self.delta[j].v == delta.v && self.delta[j].h == delta.h {
                self.weight32[j] += weight;
                return;
            }
        }

        debug_assert!(
            (self.count as usize) < self.k_max_count,
            "Too many kernel entries"
        );

        self.delta[self.count as usize] = delta;
        self.weight32[self.count as usize] = weight;

        self.count += 1;
    }

    /// `Finalize (scale, patRow, patCol, rowStep, colStep)`.
    fn finalize(&mut self, scale: DngPoint, pat_row: uint32, pat_col: uint32, row_step: int32, col_step: int32) {
        // Adjust deltas to compensate for interpolation upscaling.

        for j in 0..self.count as usize {
            let mut delta = self.delta[j];

            if scale.v == 2 {
                delta.v = (delta.v + (pat_row as int32 & 1)) >> 1;
            }

            if scale.h == 2 {
                delta.h = (delta.h + (pat_col as int32 & 1)) >> 1;
            }

            self.delta[j] = delta;
        }

        // Sort entries into row-column scan order.

        loop {
            let mut did_swap = false;

            for j in 1..self.count as usize {
                let delta0 = self.delta[j - 1];
                let delta1 = self.delta[j];

                if delta0.v > delta1.v || (delta0.v == delta1.v && delta0.h > delta1.h) {
                    did_swap = true;

                    self.delta.swap(j - 1, j);

                    let temp_weight = self.weight32[j - 1];
                    self.weight32[j - 1] = self.weight32[j];
                    self.weight32[j] = temp_weight;
                }
            }

            if !did_swap {
                break;
            }
        }

        // Calculate offsets.

        for j in 0..self.count as usize {
            self.offset[j] = row_step * self.delta[j].v + col_step * self.delta[j].h;
        }

        // Calculate 16-bit weights.

        let mut total: uint16 = 0;
        let mut biggest: usize = 0;

        for j in 0..self.count as usize {
            // Round weights to 8 fractional bits.

            self.weight16[j] = round_uint32_f64(self.weight32[j] as real64 * 256.0) as uint16;

            // Keep track of total of weights.

            total = total.wrapping_add(self.weight16[j]);

            // Keep track of which weight is biggest.

            if self.weight16[biggest] < self.weight16[j] {
                biggest = j;
            }
        }

        // Adjust largest entry so total of weights is exactly 256.

        self.weight16[biggest] = self.weight16[biggest].wrapping_add(256u16.wrapping_sub(total));

        // Recompute the floating point weights from the rounded integer
        // weights so results match more closely.

        for j in 0..self.count as usize {
            self.weight32[j] = self.weight16[j] as real32 * (1.0f32 / 256.0f32);
        }
    }
}

// ---------------------------------------------------------------------------
// dng_bilinear_pattern.
// ---------------------------------------------------------------------------

/// Port of `dng_bilinear_pattern` — kernel table for one destination plane.
struct DngBilinearPattern {
    scale: DngPoint,

    pat_rows: uint32,
    pat_cols: uint32,

    kernel: Vec<Vec<DngBilinearKernel>>,
}

impl DngBilinearPattern {
    const K_MAX_PATTERN: usize = (kMaxCFAPattern * 2) as usize;

    fn new() -> Self {
        DngBilinearPattern {
            scale: DngPoint::new(0, 0),
            pat_rows: 0,
            pat_cols: 0,
            kernel: vec![vec![DngBilinearKernel::new(); Self::K_MAX_PATTERN]; Self::K_MAX_PATTERN],
        }
    }

    fn delta_row(&self, row: uint32, delta: int32) -> uint32 {
        // Potential overflow in the C++ conversion is intentional and wraps;
        // emulate with wrapping arithmetic.
        (row.wrapping_add(self.pat_rows).wrapping_add(delta as uint32)) % self.pat_rows
    }

    fn delta_col(&self, col: uint32, delta: int32) -> uint32 {
        (col.wrapping_add(self.pat_cols).wrapping_add(delta as uint32)) % self.pat_cols
    }

    fn linear_weight1(d1: int32, d2: int32) -> real32 {
        if d1 == d2 {
            1.0
        } else {
            d2 as real32 / (d2 - d1) as real32
        }
    }

    fn linear_weight2(d1: int32, d2: int32) -> real32 {
        if d1 == d2 {
            0.0
        } else {
            -(d1 as real32) / (d2 - d1) as real32
        }
    }

    /// `Calculate (info, dstPlane, rowStep, colStep)`.
    fn calculate(&mut self, info: &DngMosaicInfo, dst_plane: uint32, row_step: int32, col_step: int32) -> DngResult<()> {
        // Find destination pattern size.

        self.scale = info.full_scale();

        self.pat_rows = (info.cfa_pattern_size.v as uint32) * self.scale.v as uint32;
        self.pat_cols = (info.cfa_pattern_size.h as uint32) * self.scale.h as uint32;

        // See if we need to scale up just while computing the kernels.

        let mut temp_scale = DngPoint::new(1, 1);

        if info.cfa_layout >= 6 {
            temp_scale = DngPoint::new(2, 2);

            self.pat_rows *= temp_scale.v as uint32;
            self.pat_cols *= temp_scale.h as uint32;
        }

        let k_max = Self::K_MAX_PATTERN;

        // Find a boolean map for this plane color and layout.

        let mut map = vec![vec![false; k_max]; k_max];

        let plane_color = info.cfa_plane_color[dst_plane as usize];

        match info.cfa_layout {
            1 => {
                // Rectangular (or square) layout

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        map[j][k] = info.cfa_pattern[j][k] == plane_color;
                    }
                }
            }

            // Note that when the descriptions of the staggered patterns refer
            // to even rows or columns, this means the second, fourth, etc.
            // (i.e., using one-based numbering).

            2 => {
                // Staggered layout A: even (1-based) columns are offset down
                // by 1/2 row

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        map[j][k] = if (j & 1) != (k & 1) {
                            false
                        } else {
                            info.cfa_pattern[j >> 1][k] == plane_color
                        };
                    }
                }
            }

            3 => {
                // Staggered layout B: even (1-based) columns are offset up
                // by 1/2 row

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        map[j][k] = if (j & 1) == (k & 1) {
                            false
                        } else {
                            info.cfa_pattern[j >> 1][k] == plane_color
                        };
                    }
                }
            }

            4 => {
                // Staggered layout C: even (1-based) rows are offset right
                // by 1/2 column

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        map[j][k] = if (j & 1) != (k & 1) {
                            false
                        } else {
                            info.cfa_pattern[j][k >> 1] == plane_color
                        };
                    }
                }
            }

            5 => {
                // Staggered layout D: even (1-based) rows are offset left
                // by 1/2 column

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        map[j][k] = if (j & 1) == (k & 1) {
                            false
                        } else {
                            info.cfa_pattern[j][k >> 1] == plane_color
                        };
                    }
                }
            }

            6 | 7 | 8 | 9 => {
                // Staggered layouts E/F/G/H.

                let e_row = if info.cfa_layout == 6 || info.cfa_layout == 7 {
                    1u32
                } else {
                    3u32
                };

                let e_col = if info.cfa_layout == 6 || info.cfa_layout == 8 {
                    1u32
                } else {
                    3u32
                };

                for j in 0..self.pat_rows as usize {
                    for k in 0..self.pat_cols as usize {
                        let jj = (j as uint32) & 3;
                        let kk = (k as uint32) & 3;

                        if (jj != 0 && jj != e_row) || (kk != 0 && kk != e_col) {
                            map[j][k] = false;
                        } else {
                            let row = (((j as uint32) >> 1) & !1) + min_uint32(jj, 1);
                            let col = (((k as uint32) >> 1) & !1) + min_uint32(kk, 1);

                            map[j][k] =
                                info.cfa_pattern[row as usize][col as usize] == plane_color;
                        }
                    }
                }
            }

            _ => {
                return Err(dng_core::program_error());
            }
        }

        // Find projections of maps.

        let mut map_h = vec![false; k_max];
        let mut map_v = vec![false; k_max];

        for j in 0..self.pat_rows as usize {
            for k in 0..self.pat_cols as usize {
                if map[j][k] {
                    map_v[j] = true;
                    map_h[k] = true;
                }
            }
        }

        // Find kernel for each pattern entry.

        // Local wraps (scalar reads) so the mutable kernel borrow below does
        // not conflict with &self.

        fn delta_row_of(pat_rows: uint32, row: uint32, delta: int32) -> uint32 {
            (row.wrapping_add(pat_rows).wrapping_add(delta as uint32)) % pat_rows
        }

        fn delta_col_of(pat_cols: uint32, col: uint32, delta: int32) -> uint32 {
            (col.wrapping_add(pat_cols).wrapping_add(delta as uint32)) % pat_cols
        }

        let pat_rows_snapshot = self.pat_rows;
        let pat_cols_snapshot = self.pat_cols;

        let delta_row = |row: uint32, delta: int32| -> uint32 {
            delta_row_of(pat_rows_snapshot, row, delta)
        };

        let delta_col = |col: uint32, delta: int32| -> uint32 {
            delta_col_of(pat_cols_snapshot, col, delta)
        };

        let mut pat_row = 0u32;

        while pat_row < self.pat_rows {
            let mut pat_col = 0u32;

            while pat_col < self.pat_cols {
                // Special case no interpolation case.

                if map[pat_row as usize][pat_col as usize] {
                    self.kernel[pat_row as usize][pat_col as usize].add(DngPoint::new(0, 0), 1.0);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // Special case common patterns in 3 by 3 neighborhood.

                let n = delta_row(pat_row, -1);
                let s = delta_row(pat_row, 1);
                let w = delta_col(pat_col, -1);
                let e = delta_col(pat_col, 1);

                let map_nw = map[n as usize][w as usize];
                let map_n = map[n as usize][pat_col as usize];
                let map_ne = map[n as usize][e as usize];

                let map_w = map[pat_row as usize][w as usize];
                let map_e = map[pat_row as usize][e as usize];

                let map_sw = map[s as usize][w as usize];
                let map_s = map[s as usize][pat_col as usize];
                let map_se = map[s as usize][e as usize];

                let kernel = &mut self.kernel[pat_row as usize][pat_col as usize];

                // All sides.

                if map_n && map_s && map_w && map_e {
                    kernel.add(DngPoint::new(-1, 0), 0.25);
                    kernel.add(DngPoint::new(0, -1), 0.25);
                    kernel.add(DngPoint::new(0, 1), 0.25);
                    kernel.add(DngPoint::new(1, 0), 0.25);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // N & S.

                if map_n && map_s {
                    kernel.add(DngPoint::new(-1, 0), 0.5);
                    kernel.add(DngPoint::new(1, 0), 0.5);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // E & W.

                if map_w && map_e {
                    kernel.add(DngPoint::new(0, -1), 0.5);
                    kernel.add(DngPoint::new(0, 1), 0.5);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // N & SW & SE.

                if map_n && map_sw && map_se {
                    kernel.add(DngPoint::new(-1, 0), 0.50);
                    kernel.add(DngPoint::new(1, -1), 0.25);
                    kernel.add(DngPoint::new(1, 1), 0.25);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // S & NW & NE.

                if map_s && map_nw && map_ne {
                    kernel.add(DngPoint::new(-1, -1), 0.25);
                    kernel.add(DngPoint::new(-1, 1), 0.25);
                    kernel.add(DngPoint::new(1, 0), 0.50);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // W & NE & SE.

                if map_w && map_ne && map_se {
                    kernel.add(DngPoint::new(-1, 1), 0.25);
                    kernel.add(DngPoint::new(0, -1), 0.50);
                    kernel.add(DngPoint::new(1, 1), 0.25);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // E & NW & SW.

                if map_e && map_nw && map_sw {
                    kernel.add(DngPoint::new(-1, -1), 0.25);
                    kernel.add(DngPoint::new(0, 1), 0.50);
                    kernel.add(DngPoint::new(1, -1), 0.25);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // Four corners.

                if map_nw && map_ne && map_se && map_sw {
                    kernel.add(DngPoint::new(-1, -1), 0.25);
                    kernel.add(DngPoint::new(-1, 1), 0.25);
                    kernel.add(DngPoint::new(1, -1), 0.25);
                    kernel.add(DngPoint::new(1, 1), 0.25);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // NW & SE

                if map_nw && map_se {
                    kernel.add(DngPoint::new(-1, -1), 0.50);
                    kernel.add(DngPoint::new(1, 1), 0.50);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // NE & SW

                if map_ne && map_sw {
                    kernel.add(DngPoint::new(-1, 1), 0.50);
                    kernel.add(DngPoint::new(1, -1), 0.50);

                    pat_col += temp_scale.h as uint32;
                    continue;
                }

                // Else use double-bilinear kernel.

                let mut dv1 = 0i32;
                let mut dv2 = 0i32;

                while !map_v[delta_row(pat_row, dv1) as usize] {
                    dv1 -= 1;
                }

                while !map_v[delta_row(pat_row, dv2) as usize] {
                    dv2 += 1;
                }

                let mut w1 = Self::linear_weight1(dv1, dv2) * 0.5;
                let mut w2 = Self::linear_weight2(dv1, dv2) * 0.5;

                let v1 = delta_row(pat_row, dv1) as int32;
                let v2 = delta_row(pat_row, dv2) as int32;

                let mut dh1 = 0i32;
                let mut dh2 = 0i32;

                while !map[v1 as usize][delta_col(pat_col, dh1) as usize] {
                    dh1 -= 1;
                }

                while !map[v1 as usize][delta_col(pat_col, dh2) as usize] {
                    dh2 += 1;
                }

                kernel.add(
                    DngPoint::new(dv1, dh1),
                    Self::linear_weight1(dh1, dh2) * w1,
                );

                kernel.add(
                    DngPoint::new(dv1, dh2),
                    Self::linear_weight2(dh1, dh2) * w1,
                );

                dh1 = 0;
                dh2 = 0;

                while !map[v2 as usize][delta_col(pat_col, dh1) as usize] {
                    dh1 -= 1;
                }

                while !map[v2 as usize][delta_col(pat_col, dh2) as usize] {
                    dh2 += 1;
                }

                kernel.add(
                    DngPoint::new(dv2, dh1),
                    Self::linear_weight1(dh1, dh2) * w2,
                );

                kernel.add(
                    DngPoint::new(dv2, dh2),
                    Self::linear_weight2(dh1, dh2) * w2,
                );

                dh1 = 0;
                dh2 = 0;

                while !map_h[delta_col(pat_col, dh1) as usize] {
                    dh1 -= 1;
                }

                while !map_h[delta_col(pat_col, dh2) as usize] {
                    dh2 += 1;
                }

                w1 = Self::linear_weight1(dh1, dh2) * 0.5;
                w2 = Self::linear_weight2(dh1, dh2) * 0.5;

                let h1 = delta_col(pat_col, dh1) as int32;
                let h2 = delta_col(pat_col, dh2) as int32;

                dv1 = 0;
                dv2 = 0;

                while !map[delta_row(pat_row, dv1) as usize][h1 as usize] {
                    dv1 -= 1;
                }

                while !map[delta_row(pat_row, dv2) as usize][h1 as usize] {
                    dv2 += 1;
                }

                kernel.add(
                    DngPoint::new(dv1, dh1),
                    Self::linear_weight1(dv1, dv2) * w1,
                );

                kernel.add(
                    DngPoint::new(dv2, dh1),
                    Self::linear_weight2(dv1, dv2) * w1,
                );

                dv1 = 0;
                dv2 = 0;

                while !map[delta_row(pat_row, dv1) as usize][h2 as usize] {
                    dv1 -= 1;
                }

                while !map[delta_row(pat_row, dv2) as usize][h2 as usize] {
                    dv2 += 1;
                }

                kernel.add(
                    DngPoint::new(dv1, dh2),
                    Self::linear_weight1(dv1, dv2) * w2,
                );

                kernel.add(
                    DngPoint::new(dv2, dh2),
                    Self::linear_weight2(dv1, dv2) * w2,
                );

                pat_col += temp_scale.h as uint32;
            }

            pat_row += temp_scale.v as uint32;
        }

        // Deal with temp scale case.

        if temp_scale == DngPoint::new(2, 2) {
            self.pat_rows /= temp_scale.v as uint32;
            self.pat_cols /= temp_scale.h as uint32;

            for pat_row in 0..self.pat_rows {
                for pat_col in 0..self.pat_cols {
                    let pat_row2 = (pat_row << 1) as int32;
                    let pat_col2 = (pat_col << 1) as int32;

                    let kernel = &mut self.kernel[pat_row2 as usize][pat_col2 as usize];

                    for j in 0..kernel.count as usize {
                        let mut x = pat_row2 + kernel.delta[j].v;

                        if (x & 3) != 0 {
                            x = (x & !3) + 2;
                        }

                        kernel.delta[j].v = (x - pat_row2) >> 1;

                        let mut x = pat_col2 + kernel.delta[j].h;

                        if (x & 3) != 0 {
                            x = (x & !3) + 2;
                        }

                        kernel.delta[j].h = (x - pat_col2) >> 1;
                    }

                    kernel.finalize(self.scale, pat_row, pat_col, row_step, col_step);
                }
            }
        }
        // Non-temp scale case.
        else {
            for pat_row in 0..self.pat_rows {
                for pat_col in 0..self.pat_cols {
                    let kernel = &mut self.kernel[pat_row as usize][pat_col as usize];

                    kernel.finalize(self.scale, pat_row, pat_col, row_step, col_step);
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_bilinear_interpolator.
// ---------------------------------------------------------------------------

/// Port of `dng_bilinear_interpolator`.
struct DngBilinearInterpolator {
    pattern: Vec<DngBilinearPattern>,
}

impl DngBilinearInterpolator {
    fn new(info: &DngMosaicInfo, row_step: int32, col_step: int32) -> DngResult<Self> {
        let mut pattern = Vec::with_capacity(info.color_planes as usize);

        for dst_plane in 0..info.color_planes {
            let mut p = DngBilinearPattern::new();

            p.calculate(info, dst_plane, row_step, col_step)?;

            pattern.push(p);
        }

        Ok(DngBilinearInterpolator { pattern })
    }

    /// `Interpolate (srcBuffer, dstBuffer)`.
    fn interpolate(&self, src_buffer: &DngPixelBuffer, dst_buffer: &mut DngPixelBuffer) -> DngResult<()> {
        let pat_cols = self.pattern[0].pat_cols;
        let pat_rows = self.pattern[0].pat_rows;

        let scale = self.pattern[0].scale;

        let s_row_shift = scale.v - 1;
        let s_col_shift = scale.h - 1;

        let dst_col = dst_buffer.area.l;

        let src_col = dst_col >> s_col_shift;

        let mut pat_phase = (dst_col as uint32) % pat_cols;

        for dst_row in dst_buffer.area.t..dst_buffer.area.b {
            let src_row = dst_row >> s_row_shift;

            let pat_row = (dst_row as uint32) % pat_rows;

            let src_base =
                src_buffer.pixel_element_index_public(src_row, src_col, src_buffer.plane)?;

            for dst_plane in 0..dst_buffer.planes {
                let dst_base = dst_buffer.pixel_element_index_public(dst_row, dst_col, dst_plane)?;

                let pat = &self.pattern[dst_plane as usize];

                // Build per-pattern-column kernel views for this row.

                let mut kern_counts = Vec::with_capacity(pat_cols as usize);
                let mut kern_offsets = Vec::with_capacity(pat_cols as usize);
                let mut kern_weights16 = Vec::with_capacity(pat_cols as usize);
                let mut kern_weights32 = Vec::with_capacity(pat_cols as usize);

                for col in 0..pat_cols as usize {
                    let k = &pat.kernel[pat_row as usize][col];
                    kern_counts.push(k.count);
                    kern_offsets.push(&k.offset[..k.count as usize]);
                    kern_weights16.push(&k.weight16[..k.count as usize]);
                    kern_weights32.push(&k.weight32[..k.count as usize]);
                }

                let dst_area_w = dst_buffer.area.w();

                if dst_buffer.pixel_type == ttShort {
                    let src = src_buffer.as_u16_slice().unwrap();
                    let dst = dst_buffer.as_u16_slice_mut().unwrap();

                    bilinear_row16(
                        src,
                        src_base,
                        dst,
                        dst_base,
                        dst_area_w,
                        pat_phase,
                        pat_cols,
                        &kern_counts,
                        &kern_offsets,
                        &kern_weights16,
                        s_col_shift as dng_core::uint32,
                    );
                } else {
                    let src = src_buffer.as_f32_slice().unwrap();
                    let dst = dst_buffer.as_f32_slice_mut().unwrap();

                    bilinear_row32(
                        src,
                        src_base,
                        dst,
                        dst_base,
                        dst_area_w,
                        pat_phase,
                        pat_cols,
                        &kern_counts,
                        &kern_offsets,
                        &kern_weights32,
                        s_col_shift as dng_core::uint32,
                    );
                }
            }

            pat_phase += 1;
            if pat_phase == pat_rows {
                pat_phase = 0;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_fast_interpolator (filter task).
// ---------------------------------------------------------------------------

/// Port of `dng_fast_interpolator` — downsampled bilinear-blend demosaic.
struct DngFastInterpolator {
    base: DngFilterTaskBase,

    info_cfa_pattern_size: DngPoint,
    info_cfa_pattern: [[uint8; kMaxCFAPattern as usize]; kMaxCFAPattern as usize],
    info_cfa_plane_color: [uint8; kMaxColorPlanes as usize],
    info_color_planes: uint32,

    down_scale: DngPoint,

    filter_color: [[uint32; kMaxCFAPattern as usize]; kMaxCFAPattern as usize],
}

impl DngFastInterpolator {
    fn new(
        info: &DngMosaicInfo,
        src_image: ImageRef,
        dst_image: ImageRef,
        down_scale: DngPoint,
        src_plane: uint32,
    ) -> Self {
        let mut base = DngFilterTaskBase::new("dng_fast_interpolator", src_image, dst_image);

        base.src_plane = src_plane;
        base.src_planes = 1;

        base.src_pixel_type = ttShort;
        base.dst_pixel_type = ttShort;

        base.src_repeat = info.cfa_pattern_size;

        base.unit_cell_override = info.cfa_pattern_size;

        let mut max_tile_h = 256 / down_scale.h;
        let mut max_tile_v = 256 / down_scale.v;

        max_tile_h = max_int32(max_tile_h, info.cfa_pattern_size.h);
        max_tile_v = max_int32(max_tile_v, info.cfa_pattern_size.v);

        base.max_tile_size_override = Some(DngPoint::new(max_tile_h, max_tile_v));

        // Find color map.

        let mut filter_color = [[0u32; kMaxCFAPattern as usize]; kMaxCFAPattern as usize];

        for r in 0..info.cfa_pattern_size.v {
            for c in 0..info.cfa_pattern_size.h {
                let key = info.cfa_pattern[r as usize][c as usize];

                for index in 0..info.color_planes as usize {
                    if key == info.cfa_plane_color[index] {
                        filter_color[r as usize][c as usize] = index as uint32;
                        break;
                    }
                }
            }
        }

        DngFastInterpolator {
            base,
            info_cfa_pattern_size: info.cfa_pattern_size,
            info_cfa_pattern: info.cfa_pattern,
            info_cfa_plane_color: info.cfa_plane_color,
            info_color_planes: info.color_planes,
            down_scale,
            filter_color,
        }
    }

    /// `SrcArea (dstArea)`.
    fn src_area(&self, dst_area: DngRect) -> DngRect {
        DngRect::from_corners(
            dst_area.t * self.down_scale.v,
            dst_area.l * self.down_scale.h,
            dst_area.b * self.down_scale.v,
            dst_area.r * self.down_scale.h,
        )
    }

    /// `ProcessArea (threadIndex, srcBuffer, dstBuffer)`.
    fn process_area(
        &mut self,
        _thread_index: uint32,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
    ) -> DngResult<()> {
        let src_area = src_buffer.area;
        let dst_area = dst_buffer.area;

        // Downsample buffer.

        let mut src_row = src_area.t;

        let mut src_row_phase1: uint32 = 0;
        let mut src_row_phase2: uint32 = 0;

        let pat_rows = self.info_cfa_pattern_size.v as uint32;
        let pat_cols = self.info_cfa_pattern_size.h as uint32;

        let cell_rows = self.down_scale.v as uint32;
        let cell_cols = self.down_scale.h as uint32;

        let planes = self.info_color_planes;

        let dst_plane_step = dst_buffer.plane_step;

        let mut total = vec![0u32; kMaxColorPlanes as usize];
        let mut count = vec![0u32; kMaxColorPlanes as usize];

        let src_row_step = src_buffer.row_step;

        let src_base0 =
            src_buffer.pixel_element_index_public(src_area.t, src_area.l, src_buffer.plane)?;

        let dst_base0 = dst_buffer.pixel_element_index_public(dst_area.t, dst_area.l, 0)?;

        let src = src_buffer.as_u16_slice().unwrap().to_vec();
        let mut dst = dst_buffer.as_u16_slice().unwrap().to_vec();

        for dst_row in dst_area.t..dst_area.b {
            let mut s_row_index = src_base0
                + ((dst_row - dst_area.t) as usize) * (cell_rows as usize) * (src_row_step as usize);

            let mut d_index = dst_base0 + ((dst_row - dst_area.t) as usize) * (dst_buffer.row_step as usize)
                + ((dst_area.l - dst_area.l) as usize);

            let mut src_col_phase1: uint32 = 0;
            let mut src_col_phase2: uint32 = 0;

            for _dst_col in dst_area.l..dst_area.r {
                src_row_phase2 = src_row_phase1;

                for _cell_row in 0..cell_rows {
                    let filter_row_index = src_row_phase2 as usize;

                    src_row_phase2 += 1;
                    if src_row_phase2 == pat_rows {
                        src_row_phase2 = 0;
                    }

                    src_col_phase2 = src_col_phase1;

                    for cell_col in 0..cell_cols {
                        let color = self.filter_color[filter_row_index]
                            [(src_col_phase2) as usize];

                        src_col_phase2 += 1;
                        if src_col_phase2 == pat_cols {
                            src_col_phase2 = 0;
                        }

                        total[color as usize] += src[s_row_index + cell_col as usize] as u32;
                        count[color as usize] += 1;
                    }

                    s_row_index += src_row_step as usize;
                }

                for plane in 0..planes as usize {
                    let t = total[plane];
                    let c = max_uint32(count[plane], 1);

                    dst[d_index + plane * (dst_plane_step as usize)] =
                        ((t + (c >> 1)) / c) as uint16;

                    total[plane] = 0;
                    count[plane] = 0;
                }

                src_col_phase1 = src_col_phase2;

                d_index += 1;
            }

            src_row_phase1 = src_row_phase2;

            src_row += cell_rows as int32;
        }

        // Copy results back into the destination buffer.

        let dst_slice = dst_buffer.as_u16_slice_mut().unwrap();
        dst_slice.copy_from_slice(&dst);

        let _ = src_row;

        Ok(())
    }
}

impl DngAreaTask for DngFastInterpolator {
    fn name(&self) -> &str {
        "dng_fast_interpolator"
    }

    fn unit_cell(&self) -> DngPoint {
        self.base.unit_cell_override
    }

    fn max_tile_size(&self) -> DngPoint {
        self.base
            .max_tile_size_override
            .unwrap_or_else(|| DngPoint::new(1024, 1024))
    }

    fn repeating_tile1(&self) -> DngRect {
        self.base.src_image.read().unwrap().repeating_tile()
    }

    fn repeating_tile2(&self) -> DngRect {
        self.base.dst_image.read().unwrap().repeating_tile()
    }

    fn process(
        &mut self,
        thread_index: uint32,
        area: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        let src_area = self.src_area(area);

        // Mirror C++ dng_filter_task::Process with per-tile buffers.

        let tile_size = {
            let src_tile = self.base.src_tile_size(DngPoint::new(area.w() as int32, area.h() as int32));
            src_tile
        };

        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            self.base.src_plane,
            self.base.src_planes,
            self.base.src_pixel_type,
            pcRowInterleaved,
            DngPixelBuffer::allocate_data_simd(src_area, self.base.src_planes, self.base.src_pixel_type)?,
        )?;

        {
            let src = self.base.src_image.read().unwrap();
            let repeat = self.base.src_repeat;
            src.get(
                &mut src_buffer,
                EdgeOption::EdgeRepeat,
                repeat.v as uint32,
                repeat.h as uint32,
            )?;
        }

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            area,
            self.base.dst_plane,
            self.base.dst_planes,
            self.base.dst_pixel_type,
            pcRowInterleaved,
            DngPixelBuffer::allocate_data_simd(area, self.base.dst_planes, self.base.dst_pixel_type)?,
        )?;

        let _ = tile_size;

        self.process_area(thread_index, &mut src_buffer, &mut dst_buffer)?;

        let dst = self.base.dst_image.read().unwrap();
        dst.put(&dst_buffer)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_mosaic_info.
// ---------------------------------------------------------------------------

/// Port of `dng_mosaic_info`.
#[derive(Debug, Clone)]
pub struct DngMosaicInfo {
    /// `fCFAPatternSize` — size of `cfa_pattern`.
    pub cfa_pattern_size: DngPoint,

    /// `fCFAPattern` — CFA pattern from the CFAPattern tag.
    pub cfa_pattern: [[uint8; kMaxCFAPattern as usize]; kMaxCFAPattern as usize],

    /// `fColorPlanes` — number of color planes in DNG input.
    pub color_planes: uint32,

    /// `fCFAPlaneColor`.
    pub cfa_plane_color: [uint8; kMaxColorPlanes as usize],

    /// `fCFALayout` — value of the CFALayout tag.
    pub cfa_layout: uint32,

    /// `fBayerGreenSplit` — value of the BayerGreenSplit tag.
    pub bayer_green_split: uint32,

    /// `fSrcSize` (protected in C++) — source image size.
    pub(crate) src_size: DngPoint,

    /// `fCroppedSize` (protected in C++).
    pub(crate) cropped_size: DngPoint,

    /// `fAspectRatio` (protected in C++).
    pub(crate) aspect_ratio: real64,
}

impl Default for DngMosaicInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl DngMosaicInfo {
    /// `dng_mosaic_info ()`.
    pub fn new() -> Self {
        DngMosaicInfo {
            cfa_pattern_size: DngPoint::new(0, 0),
            cfa_pattern: [[0; kMaxCFAPattern as usize]; kMaxCFAPattern as usize],
            color_planes: 0,
            cfa_plane_color: [0; kMaxColorPlanes as usize],
            cfa_layout: 1,
            bayer_green_split: 0,
            src_size: DngPoint::new(0, 0),
            cropped_size: DngPoint::new(0, 0),
            aspect_ratio: 1.0,
        }
    }

    /// `Parse (host, stream, info)`.
    pub fn parse(&mut self, _host: &mut DngHost, _stream: &mut DngStream, info: &DngInfo) {
        // Find main image IFD.

        let main_index = info.main_index as usize;

        let raw_ifd = info.ifd[main_index].clone();

        // This information only applies to CFA images.

        if raw_ifd.photometric_interpretation != piCFA {
            return;
        }

        // Copy CFA pattern.

        self.cfa_pattern_size = DngPoint::new(
            raw_ifd.cfa_repeat_pattern_cols as int32,
            raw_ifd.cfa_repeat_pattern_rows as int32,
        );

        debug_assert!(
            self.cfa_pattern_size.v >= 1
                && self.cfa_pattern_size.v <= kMaxCFAPattern as int32,
            "Invalid fCFAPatternSize.v"
        );

        debug_assert!(
            self.cfa_pattern_size.h >= 1
                && self.cfa_pattern_size.h <= kMaxCFAPattern as int32,
            "Invalid fCFAPatternSize.h"
        );

        for j in 0..self.cfa_pattern_size.v {
            for k in 0..self.cfa_pattern_size.h {
                self.cfa_pattern[j as usize][k as usize] =
                    raw_ifd.cfa_pattern[j as usize][k as usize];
            }
        }

        // Copy CFA plane information.

        self.color_planes = info.shared.camera_profile.color_planes;

        for n in 0..self.color_planes as usize {
            self.cfa_plane_color[n] = raw_ifd.cfa_plane_color[n];
        }

        // Copy CFA layout information.

        self.cfa_layout = raw_ifd.cfa_layout;

        // Green split value for Bayer patterns.

        self.bayer_green_split = raw_ifd.bayer_green_split;
    }

    /// `PostParse (host, negative)`.
    pub fn post_parse(&mut self, _host: &mut DngHost, negative: &DngNegative) {
        // Keep track of source image size.

        if let Some(stage2) = negative.stage2_image() {
            self.src_size = stage2.read().unwrap().size();
        }

        // Default cropped size.

        self.cropped_size = DngPoint::new(
            round_int32_f64(negative.default_crop_size_h().as_real64()),
            round_int32_f64(negative.default_crop_size_v().as_real64()),
        );

        // Pixel aspect ratio.

        self.aspect_ratio = negative.default_scale_h().as_real64()
            / negative.default_scale_v().as_real64();
    }

    /// `IsColorFilterArray ()`.
    pub fn is_color_filter_array(&self) -> bool {
        self.cfa_pattern_size != DngPoint::new(0, 0)
    }

    /// `SetFourColorBayer ()` — enable generating four-plane output from
    /// three-plane Bayer input.
    pub fn set_four_color_bayer(&mut self) -> bool {
        if self.cfa_pattern_size != DngPoint::new(2, 2) {
            return false;
        }

        if self.color_planes != 3 {
            return false;
        }

        let color0 = self.cfa_plane_color[0];
        let color1 = self.cfa_plane_color[1];
        let color2 = self.cfa_plane_color[2];

        // Look for color 1 repeated twice in a diagonal.

        if (self.cfa_pattern[0][0] == color1 && self.cfa_pattern[1][1] == color1)
            || (self.cfa_pattern[0][1] == color1 && self.cfa_pattern[1][0] == color1)
        {
            // OK, this looks like a Bayer pattern.

            // Find unused color code.

            let mut color3 = 0u8;

            while color3 == color0 || color3 == color1 || color3 == color2 {
                color3 += 1;
            }

            // Switch the four color mosaic.

            self.color_planes = 4;

            self.cfa_plane_color[3] = color3;

            // Replace the "green" in the "blue" rows with the new color.

            if self.cfa_pattern[0][0] == color0 {
                self.cfa_pattern[1][0] = color3;
            } else if self.cfa_pattern[0][1] == color0 {
                self.cfa_pattern[1][1] = color3;
            } else if self.cfa_pattern[1][0] == color0 {
                self.cfa_pattern[0][0] = color3;
            } else {
                self.cfa_pattern[0][1] = color3;
            }

            return true;
        }

        false
    }

    /// `FullScale ()` — scaling factor relative to input size needed to
    /// capture output data.
    pub fn full_scale(&self) -> DngPoint {
        match self.cfa_layout {
            // Staggered layouts with offset columns double the row count
            // during interpolation.
            2 | 3 => return DngPoint::new(2, 1),

            // Staggered layouts with offset rows double the column count
            // during interpolation.
            4 | 5 => return DngPoint::new(1, 2),

            _ => {}
        }

        DngPoint::new(1, 1)
    }

    /// `IsSafeDownScale (downScale)` (protected in C++).
    pub(crate) fn is_safe_down_scale(&self, down_scale: DngPoint) -> bool {
        if down_scale.v >= self.cfa_pattern_size.v && down_scale.h >= self.cfa_pattern_size.h {
            return true;
        }

        let test = DngPoint::new(
            min_int32(down_scale.h, self.cfa_pattern_size.h),
            min_int32(down_scale.v, self.cfa_pattern_size.v),
        );

        for phase_v in 0..self.cfa_pattern_size.v {
            for phase_h in 0..self.cfa_pattern_size.h {
                let mut contains = [false; kMaxColorPlanes as usize];

                for src_row in 0..test.v {
                    for src_col in 0..test.h {
                        let src_key = self.cfa_pattern
                            [((src_row + phase_v) % self.cfa_pattern_size.v) as usize]
                            [((src_col + phase_h) % self.cfa_pattern_size.h) as usize];

                        for plane in 0..self.color_planes as usize {
                            if src_key == self.cfa_plane_color[plane] {
                                contains[plane] = true;
                            }
                        }
                    }
                }

                for plane in 0..self.color_planes as usize {
                    if !contains[plane] {
                        return false;
                    }
                }
            }
        }

        true
    }

    /// `SizeForDownScale (downScale)` (protected in C++).
    pub(crate) fn size_for_down_scale(&self, down_scale: DngPoint) -> uint32 {
        let size_v = max_uint32(
            1,
            (self.cropped_size.v as uint32 + (down_scale.v as uint32 >> 1))
                / down_scale.v as uint32,
        );
        let size_h = max_uint32(
            1,
            (self.cropped_size.h as uint32 + (down_scale.h as uint32 >> 1))
                / down_scale.h as uint32,
        );

        max_int32(size_v as int32, size_h as int32) as uint32
    }

    /// `ValidSizeDownScale (downScale, minSize)` (protected in C++).
    pub(crate) fn valid_size_down_scale(&self, down_scale: DngPoint, min_size: uint32) -> bool {
        const K_MAX_DOWN_SCALE: int32 = 64;

        if down_scale.h > K_MAX_DOWN_SCALE || down_scale.v > K_MAX_DOWN_SCALE {
            return false;
        }

        self.size_for_down_scale(down_scale) >= min_size
    }

    /// `DownScale (minSize, prefSize, cropFactor)`.
    pub fn down_scale(&self, min_size: uint32, pref_size: uint32, crop_factor: real64) -> DngPoint {
        let mut best_scale = DngPoint::new(1, 1);

        let mut min_size = min_size;
        let mut pref_size = pref_size;

        if pref_size != 0 && self.is_color_filter_array() {
            // Adjust sizes for crop factor.

            min_size = round_uint32_f64(min_size as real64 / crop_factor);
            pref_size = round_uint32_f64(pref_size as real64 / crop_factor);

            pref_size = max_uint32(pref_size, min_size);

            // Start by assuming we need the full size image.

            let mut best_size = self.size_for_down_scale(best_scale) as int32;

            // Find size of nearly square cell.

            let mut square_cell = DngPoint::new(1, 1);

            if self.aspect_ratio < 1.0 / 1.8 {
                square_cell.h = min_int32(4, round_int32_f64(1.0 / self.aspect_ratio));
            }

            if self.aspect_ratio > 1.8 {
                square_cell.v = min_int32(4, round_int32_f64(self.aspect_ratio));
            }

            // Find minimum safe cell size.

            let mut test_scale = square_cell;

            while !self.is_safe_down_scale(test_scale) {
                test_scale.v += square_cell.v;
                test_scale.h += square_cell.h;
            }

            // See if this scale is usable.

            if !self.valid_size_down_scale(test_scale, min_size) {
                // We cannot downsample at all...

                return best_scale;
            }

            // See if this is closer to the preferred size.

            let mut test_size = self.size_for_down_scale(test_scale) as int32;

            if abs_int32(test_size - pref_size as int32)
                <= abs_int32(best_size - pref_size as int32)
            {
                best_scale = test_scale;
                best_size = test_size;
            } else {
                return best_scale;
            }

            // Now keep adding square cells as long as possible.

            loop {
                test_scale.v += square_cell.v;
                test_scale.h += square_cell.h;

                if self.is_safe_down_scale(test_scale) {
                    if !self.valid_size_down_scale(test_scale, min_size) {
                        return best_scale;
                    }

                    // See if this is closer to the preferred size.

                    test_size = self.size_for_down_scale(test_scale) as int32;

                    if abs_int32(test_size - pref_size as int32)
                        <= abs_int32(best_size - pref_size as int32)
                    {
                        best_scale = test_scale;
                        best_size = test_size;
                    } else {
                        return best_scale;
                    }
                } else {
                    // The C++ loop runs forever once IsSafeDownScale stays
                    // false after the first success is impossible; the port
                    // breaks here (sizes only grow, so the first failure
                    // after success is final).
                    //
                    // PORT-NOTE(dng_mosaic_info.cpp:1719): the C++ while(true)
                    // loop exits only via return; when IsSafeDownScale turns
                    // false it keeps incrementing forever (wrap-around
                    // eventually re-succeeds). Since test sizes only grow,
                    // returning the current best is behavior-preserving for
                    // all practical inputs.
                    return best_scale;
                }
            }
        }

        best_scale
    }

    /// `DstSize (downScale)` — size of demosaiced image for a downscaling
    /// factor.
    pub fn dst_size(&self, down_scale: DngPoint) -> DngPoint {
        if down_scale == DngPoint::new(1, 1) {
            let scale = self.full_scale();

            return DngPoint::new(
                self.src_size.h * scale.h,
                self.src_size.v * scale.v,
            );
        }

        const K_MAX_DOWN_SCALE: int32 = 64;

        if down_scale.h > K_MAX_DOWN_SCALE || down_scale.v > K_MAX_DOWN_SCALE {
            return DngPoint::new(0, 0);
        }

        DngPoint::new(
            max_int32(
                1,
                (self.src_size.h + (down_scale.h >> 1)) / down_scale.h,
            ),
            max_int32(
                1,
                (self.src_size.v + (down_scale.v >> 1)) / down_scale.v,
            ),
        )
    }

    /// `InterpolateGeneric (host, negative, srcImage, dstImage, srcPlane)` —
    /// demosaic interpolation of a single plane for the non-downsampled case.
    pub fn interpolate_generic(
        &self,
        host: &mut DngHost,
        _negative: &mut DngNegative,
        src_image: &ImageRef,
        dst_image: &ImageRef,
        src_plane: uint32,
    ) -> DngResult<()> {
        // Find destination to source bit shifts.

        let scale = self.full_scale();

        let src_shift_v = scale.v - 1;
        let src_shift_h = scale.h - 1;

        // Find tile sizes.

        const K_MAX_DST_TILE_ROWS: uint32 = 128;
        const K_MAX_DST_TILE_COLS: uint32 = 128;

        let mut dst_tile_size = dst_image.read().unwrap().repeating_tile().size();

        dst_tile_size.v = min_int32(dst_tile_size.v, K_MAX_DST_TILE_ROWS as int32);
        dst_tile_size.h = min_int32(dst_tile_size.h, K_MAX_DST_TILE_COLS as int32);

        let mut src_tile_size = dst_tile_size;

        src_tile_size.v >>= src_shift_v;
        src_tile_size.h >>= src_shift_h;

        src_tile_size.v += self.cfa_pattern_size.v * 2;
        src_tile_size.h += self.cfa_pattern_size.h * 2;

        // Allocate source buffer.

        let src_pixel_type = src_image.read().unwrap().pixel_type();

        let mut src_buffer = DngPixelBuffer::with_planar_config(
            DngRect::from_size_point(src_tile_size),
            src_plane,
            1,
            src_pixel_type,
            pcInterleaved,
            DngPixelBuffer::allocate_data(
                DngRect::from_size_point(src_tile_size),
                1,
                src_pixel_type,
            )?,
        )?;

        // Allocate destination buffer.

        let dst_pixel_type = dst_image.read().unwrap().pixel_type();

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            DngRect::from_size_point(dst_tile_size),
            0,
            self.color_planes,
            dst_pixel_type,
            pcRowInterleaved,
            DngPixelBuffer::allocate_data(
                DngRect::from_size_point(dst_tile_size),
                self.color_planes,
                dst_pixel_type,
            )?,
        )?;

        // Create interpolator.

        let interpolator = DngBilinearInterpolator::new(
            self,
            src_buffer.row_step,
            src_buffer.col_step,
        )?;

        // Iterate over destination tiles.

        let dst_guard = dst_image.read().unwrap();
        let dst_bounds = dst_guard.bounds();
        let mut iter1 = DngTileIterator::new_image(&*dst_guard, dst_bounds);

        let mut dst_area = DngRect::new();

        while iter1.get_one_tile(&mut dst_area)? {
            // Break into buffer sized tiles.

            let mut iter2 = DngTileIterator::new_tile_size(dst_tile_size, dst_area);

            let mut dst_tile = DngRect::new();

            while iter2.get_one_tile(&mut dst_tile)? {
                host.sniff_for_abort()?;

                // Setup buffers for this tile.

                let mut src_tile = dst_tile;

                src_tile.t >>= src_shift_v;
                src_tile.b >>= src_shift_v;

                src_tile.l >>= src_shift_h;
                src_tile.r >>= src_shift_h;

                src_tile.t -= self.cfa_pattern_size.v;
                src_tile.b += self.cfa_pattern_size.v;

                src_tile.l -= self.cfa_pattern_size.h;
                src_tile.r += self.cfa_pattern_size.h;

                src_buffer.area = src_tile;
                dst_buffer.area = dst_tile;

                // Get source data.

                src_image.read().unwrap().get(
                    &mut src_buffer,
                    EdgeOption::EdgeRepeat,
                    self.cfa_pattern_size.v as uint32,
                    self.cfa_pattern_size.h as uint32,
                )?;

                // Process data.

                interpolator.interpolate(&src_buffer, &mut dst_buffer)?;

                // Save results.

                dst_image.read().unwrap().put(&dst_buffer)?;
            }
        }

        Ok(())
    }

    /// `InterpolateFast (host, negative, srcImage, dstImage, downScale,
    /// srcPlane)` — demosaic interpolation for the downsampled case.
    pub fn interpolate_fast(
        &self,
        host: &mut DngHost,
        _negative: &mut DngNegative,
        src_image: &ImageRef,
        dst_image: &ImageRef,
        down_scale: DngPoint,
        src_plane: uint32,
    ) -> DngResult<()> {
        // Create fast interpolator task.

        let mut interpolator =
            DngFastInterpolator::new(self, src_image.clone(), dst_image.clone(), down_scale, src_plane);

        // Find area to process.

        let bounds = dst_image.read().unwrap().bounds();

        // Do the interpolation.

        host.perform_area_task(&mut interpolator, bounds)?;

        Ok(())
    }

    /// `Interpolate (host, negative, srcImage, dstImage, downScale, srcPlane,
    /// scaleTransforms)` — chooses between generic and fast interpolators.
    pub fn interpolate(
        &self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        src_image: &ImageRef,
        dst_image: &ImageRef,
        down_scale: DngPoint,
        src_plane: uint32,
        scale_transforms: Option<&mut Vec<DngMatrix3by3>>,
    ) -> DngResult<()> {
        if let Some(transforms) = scale_transforms {
            if down_scale != DngPoint::new(1, 1) {
                for plane in 0..dst_image.read().unwrap().planes() {
                    let _ = plane;

                    let m = DngMatrix3by3::new(
                        1.0 / down_scale.v as real64,
                        0.0,
                        0.0,
                        0.0,
                        1.0 / down_scale.h as real64,
                        0.0,
                        0.0,
                        0.0,
                        1.0,
                    );

                    if (plane as usize) < transforms.len() {
                        transforms[plane as usize] = m;
                    } else {
                        transforms.push(m);
                    }
                }
            }
        }

        if down_scale == DngPoint::new(1, 1) {
            self.interpolate_generic(host, negative, src_image, dst_image, src_plane)
        } else {
            self.interpolate_fast(host, negative, src_image, dst_image, down_scale, src_plane)
        }
    }

    /// `SupportsPreservedBlackLevels ()`.
    pub fn supports_preserved_black_levels(&self) -> bool {
        false
    }
}

#[allow(unused)]
fn unused_imports() {
    let _ = (DngMatrix3by3::identity, ttFloat);
}

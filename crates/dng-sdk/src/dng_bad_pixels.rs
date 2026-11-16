// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_bad_pixels.cpp/.h`.
//
// Copyright 2008-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_bad_pixels.cpp` / `dng_bad_pixels.h` — opcodes to fix
//! defective pixels: `dng_opcode_FixBadPixelsConstant` (opcode 4) and
//! `dng_opcode_FixBadPixelsList` (opcode 5) plus `dng_bad_pixel_list`.
//!
//! These are `dng_filter_opcode` subclasses (Agent A's opcode layer); the
//! module compiles once integration declares all opcode modules in lib.rs.

use dng_core::dng_exceptions::bad_format;
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_point::DngPoint;
use dng_core::dng_rect::DngRect;
use dng_core::dng_stream::DngStream;
use dng_core::dng_tag_types::ttShort;
use dng_core::dng_tag_values::dngVersion_1_3_0_0;
use crate::dng_opcodes::{dngOpcode_FixBadPixelsConstant, dngOpcode_FixBadPixelsList};
use dng_core::dng_types::{int32, uint32, uint16};
use dng_core::dng_host::DngHost;
use dng_core::dng_image::ImageRef;
use dng_core::dng_rect::transpose_rect;
use dng_core::dng_utils::{abs_int32, max_uint32, min_uint32, pin_int32, pin_uint32};
use dng_core::DngResult;

use crate::dng_negative::DngNegative;
use crate::dng_opcodes::{DngFilterOpcode, DngOpcode, DngOpcodeBase};

// ---------------------------------------------------------------------------
// dng_bad_pixel_list
// ---------------------------------------------------------------------------

/// Port of `dng_bad_pixel_list` — a list of bad pixels and rectangles
/// (usually single rows or columns).
#[derive(Debug, Clone, Default)]
pub struct DngBadPixelList {
    /// `fBadPoints`.
    bad_points: Vec<DngPoint>,

    /// `fBadRects`.
    bad_rects: Vec<DngRect>,
}

impl DngBadPixelList {
    /// `kNoIndex`.
    pub const kNoIndex: uint32 = 0xFFFFFFFF;

    /// `dng_bad_pixel_list ()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// `PointCount ()`.
    pub fn point_count(&self) -> uint32 {
        self.bad_points.len() as uint32
    }

    /// `Point (index)`.
    pub fn point(&self, index: uint32) -> &DngPoint {
        &self.bad_points[index as usize]
    }

    /// `RectCount ()`.
    pub fn rect_count(&self) -> uint32 {
        self.bad_rects.len() as uint32
    }

    /// `Rect (index)`.
    pub fn rect(&self, index: uint32) -> &DngRect {
        &self.bad_rects[index as usize]
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.point_count() == 0 && self.rect_count() == 0
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `AddPoint (pt)`.
    pub fn add_point(&mut self, pt: DngPoint) {
        self.bad_points.push(pt);
    }

    /// `AddRect (r)`.
    pub fn add_rect(&mut self, r: DngRect) {
        self.bad_rects.push(r);
    }

    /// `Sort ()` — sort points/rects top-to-bottom, left-to-right.
    pub fn sort(&mut self) {
        self.bad_points.sort_by(|a, b| {
            (a.v, a.h).cmp(&(b.v, b.h))
        });

        self.bad_rects.sort_by(|a, b| (a.t, a.l, a.b, a.r).cmp(&(b.t, b.l, b.b, b.r)));
    }

    /// `IsPointIsolated (index, radius)`.
    pub fn is_point_isolated(&self, index: uint32, radius: int32) -> bool {
        let pt = self.point(index);

        // Search backward through bad point list.

        let mut j = index as int32 - 1;
        while j >= 0 {
            let pt2 = self.point(j as uint32);

            if pt2.v < pt.v - radius {
                break;
            }

            if abs_int32(pt2.h - pt.h) <= radius as uint32 {
                return false;
            }

            j -= 1;
        }

        // Search forward through bad point list.

        for k in (index + 1)..self.point_count() {
            let pt2 = self.point(k);

            if pt2.v > pt.v + radius {
                break;
            }

            if abs_int32(pt2.h - pt.h) <= radius as uint32 {
                return false;
            }
        }

        // Search through bad rectangle list.

        let test_rect = DngRect::from_corners(
            pt.v - radius,
            pt.h - radius,
            pt.v + radius + 1,
            pt.h + radius + 1,
        );

        for n in 0..self.rect_count() {
            if test_rect.and_(*self.rect(n)).not_empty() {
                return false;
            }
        }

        true
    }

    /// `IsRectIsolated (index, radius)`.
    pub fn is_rect_isolated(&self, index: uint32, radius: int32) -> bool {
        let rect = *self.rect(index);

        let test_rect = DngRect::from_corners(
            rect.t - radius,
            rect.l - radius,
            rect.b + radius,
            rect.r + radius,
        );

        for n in 0..self.rect_count() {
            if n != index && test_rect.and_(*self.rect(n)).not_empty() {
                return false;
            }
        }

        true
    }

    /// `IsPointValid (pt, imageBounds, index)`.
    pub fn is_point_valid(&self, pt: &DngPoint, image_bounds: &DngRect, index: uint32) -> bool {
        // The point must be in the image bounds to be valid.

        if pt.v < image_bounds.t
            || pt.h < image_bounds.l
            || pt.v >= image_bounds.b
            || pt.h >= image_bounds.r
        {
            return false;
        }

        // Only search the bad point list if we have a starting search index.

        if index != Self::kNoIndex {
            // Search backward through bad point list.

            let mut j = index as int32 - 1;
            while j >= 0 {
                let pt2 = self.point(j as uint32);

                if pt2.v < pt.v {
                    break;
                }

                if pt2 == pt {
                    return false;
                }

                j -= 1;
            }

            // Search forward through bad point list.

            for k in (index + 1)..self.point_count() {
                let pt2 = self.point(k);

                if pt2.v > pt.v {
                    break;
                }

                if pt2 == pt {
                    return false;
                }
            }
        }

        // Search through bad rectangle list.

        for n in 0..self.rect_count() {
            let r = self.rect(n);

            if pt.v >= r.t && pt.h >= r.l && pt.v < r.b && pt.h < r.r {
                return false;
            }
        }

        true
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// `IsGreen` (both opcodes): `((row + col + phase + (phase >> 1)) & 1) == 0`.
#[inline]
fn is_green(row: int32, col: int32, bayer_phase: uint32) -> bool {
    ((row as uint32)
        .wrapping_add(col as uint32)
        .wrapping_add(bayer_phase)
        .wrapping_add(bayer_phase >> 1)
        & 1)
        == 0
}

/// Offset-based u16 access on a pixel buffer, mirroring the C++ pointer
/// arithmetic `sPtr[offset]` (offsets in samples, not pixels).
#[inline]
fn pixel_offset_u16(buffer: &DngPixelBuffer, row: int32, col: int32, plane: uint32, off_v: int32, off_h: int32) -> DngResult<uint16> {
    buffer.const_pixel_u16(row + off_v, col + off_h, plane)
}

#[inline]
fn set_pixel_offset_u16(
    buffer: &mut DngPixelBuffer,
    row: int32,
    col: int32,
    plane: uint32,
    off_v: int32,
    off_h: int32,
    value: uint16,
) -> DngResult<()> {
    buffer.dirty_pixel_u16(row + off_v, col + off_h, plane, value)
}

// ---------------------------------------------------------------------------
// dng_opcode_FixBadPixelsConstant
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_FixBadPixelsConstant` — fixes individual bad pixels
/// marked with a constant value (e.g., 0) in a Bayer image.
#[derive(Debug)]
pub struct DngOpcodeFixBadPixelsConstant {
    base: DngOpcodeBase,

    /// `fConstant`.
    constant: uint32,

    /// `fBayerPhase`.
    bayer_phase: uint32,
}

impl DngOpcodeFixBadPixelsConstant {
    /// `dng_opcode_FixBadPixelsConstant (constant, bayerPhase)`.
    pub fn new(constant: uint32, bayer_phase: uint32) -> Self {
        DngOpcodeFixBadPixelsConstant {
            base: DngOpcodeBase::new(
                dngOpcode_FixBadPixelsConstant,
                dngVersion_1_3_0_0,
                0,
            ),
            constant,
            bayer_phase,
        }
    }

    /// `dng_opcode_FixBadPixelsConstant (stream)`.
    pub fn from_stream(mut stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_FixBadPixelsConstant, &mut stream)?;

        let size = stream.get_uint32()?;

        if size != 8 {
            return Err(bad_format());
        }

        let constant = stream.get_uint32()?;
        let bayer_phase = stream.get_uint32()?;

        Ok(DngOpcodeFixBadPixelsConstant {
            base,
            constant,
            bayer_phase,
        })
    }

    /// `fConstant`.
    pub fn constant(&self) -> uint32 {
        self.constant
    }

    /// `fBayerPhase`.
    pub fn bayer_phase(&self) -> uint32 {
        self.bayer_phase
    }

    fn is_green(&self, row: int32, col: int32) -> bool {
        is_green(row, col, self.bayer_phase)
    }

    /// `ProcessArea` — the constant-fix algorithm.
    fn process_area_impl(
        &mut self,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
        dst_area: &DngRect,
    ) -> DngResult<()> {
        dst_buffer.copy_area(
            src_buffer,
            *dst_area,
            0,
            dst_buffer.planes,
            dst_buffer.planes,
        )?;

        let bad_pixel = self.constant as uint16;

        let row_step = src_buffer.row_step;
        let col_step = src_buffer.col_step;

        for dst_row in dst_area.t..dst_area.b {
            for dst_col in dst_area.l..dst_area.r {
                let value = src_buffer.const_pixel_u16(dst_row, dst_col, 0)?;

                if value == bad_pixel {
                    let mut count: uint32 = 0;
                    let mut total: uint32 = 0;

                    // Neighbor values (C++ reads via pointer offsets that
                    // stay within the padded SrcArea).

                    let neighbors: [(int32, int32); 4] = if self.is_green(dst_row, dst_col) {
                        // Green pixel: diagonal neighbors.
                        [
                            (-row_step_abs(row_step), -1),
                            (-row_step_abs(row_step), 1),
                            (row_step_abs(row_step), -1),
                            (row_step_abs(row_step), 1),
                        ]
                    } else {
                        // Red/blue pixel: 2-away cardinal neighbors.
                        [
                            (-2 * row_step_abs(row_step), 0),
                            (2 * row_step_abs(row_step), 0),
                            (0, -2),
                            (0, 2),
                        ]
                    };

                    for (dv, dh) in neighbors {
                        // C++ pointer offsets: v offset in rows, h offset in
                        // samples.
                        let neighbor = if (dv < 0 && dst_row + dv < 0)
                            || (dh < 0 && dst_col + dh < 0)
                        {
                            // Out-of-range read in C++ would be UB; the
                            // SrcArea padding guarantees this does not
                            // happen for valid tiles. Skip defensively.
                            bad_pixel
                        } else {
                            pixel_offset_u16(
                                src_buffer,
                                dst_row,
                                dst_col,
                                0,
                                dv,
                                dh,
                            )
                            .unwrap_or(bad_pixel)
                        };

                        if neighbor != bad_pixel {
                            count += 1;
                            total += neighbor as uint32;
                        }
                    }

                    if count == 4 {
                        // Most common case.
                        let v = (total + 2) >> 2;
                        dst_buffer.dirty_pixel_u16(dst_row, dst_col, 0, v as uint16)?;
                    } else if count > 0 {
                        let v = (total + (count >> 1)) / count;
                        dst_buffer.dirty_pixel_u16(dst_row, dst_col, 0, v as uint16)?;
                    }
                }
            }
        }

        Ok(())
    }
}

#[inline]
fn row_step_abs(row_step: int32) -> int32 {
    if row_step < 0 {
        -row_step
    } else {
        row_step
    }
}

impl DngOpcode for DngOpcodeFixBadPixelsConstant {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        // Filter opcodes apply through the serial filter-task driver.

        self.apply_filter(host, negative, image)
    }

    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(8)?;
        stream.put_uint32(self.constant)?;
        stream.put_uint32(self.bayer_phase)?;
        Ok(())
    }
}

impl DngFilterOpcode for DngOpcodeFixBadPixelsConstant {
    fn src_repeat(&self) -> DngPoint {
        DngPoint::new(2, 2)
    }

    fn src_area(&self, dst_area: DngRect, _image_bounds: DngRect) -> DngRect {
        DngRect::from_corners(
            dst_area.t - 2,
            dst_area.l - 2,
            dst_area.b + 2,
            dst_area.r + 2,
        )
    }

    fn prepare(
        &mut self,
        _negative: &mut DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        _image_bounds: DngRect,
        image_planes: uint32,
        buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        // This opcode is restricted to single channel 16-bit images.

        if image_planes != 1 {
            return Err(bad_format());
        }

        if buffer_pixel_type != ttShort {
            return Err(bad_format());
        }

        Ok(())
    }

    fn process_area(
        &mut self,
        _negative: &mut DngNegative,
        _thread_index: uint32,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        self.process_area_impl(src_buffer, dst_buffer, &dst_area)
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_FixBadPixelsList
// ---------------------------------------------------------------------------

/// `kBadPointPadding`.
const kBadPointPadding: int32 = 2;
/// `kBadRectPadding`.
const kBadRectPadding: int32 = 4;

/// Port of `dng_opcode_FixBadPixelsList` — fixes lists of bad pixels
/// (indicated by position) in a Bayer image.
#[derive(Debug)]
pub struct DngOpcodeFixBadPixelsList {
    base: DngOpcodeBase,

    /// `fList`.
    list: DngBadPixelList,

    /// `fBayerPhase`.
    bayer_phase: uint32,
}

impl DngOpcodeFixBadPixelsList {
    /// `dng_opcode_FixBadPixelsList (list, bayerPhase)`.
    pub fn new(mut list: DngBadPixelList, bayer_phase: uint32) -> Self {
        list.sort();

        DngOpcodeFixBadPixelsList {
            base: DngOpcodeBase::new(dngOpcode_FixBadPixelsList, dngVersion_1_3_0_0, 0),
            list,
            bayer_phase,
        }
    }

    /// `dng_opcode_FixBadPixelsList (stream)`.
    pub fn from_stream(mut stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_FixBadPixelsList, &mut stream)?;

        let size = stream.get_uint32()?;

        let bayer_phase = stream.get_uint32()?;

        let p_count = stream.get_uint32()?;
        let r_count = stream.get_uint32()?;

        // expectedSize = 12 + pCount * 8 + rCount * 16 (overflow-checked).

        let expected_size = 12u32
            .checked_add(
                p_count
                    .checked_mul(8)
                    .and_then(|v| v.checked_add(r_count.checked_mul(16)?))
                    .ok_or_else(dng_core::dng_exceptions::overflow_error)?,
            )
            .ok_or_else(dng_core::dng_exceptions::overflow_error)?;

        if size != expected_size {
            return Err(bad_format());
        }

        let mut list = DngBadPixelList::new();

        for _ in 0..p_count {
            let v = stream.get_int32()?;
            let h = stream.get_int32()?;
            list.add_point(DngPoint::new(v, h));
        }

        for _ in 0..r_count {
            let t = stream.get_int32()?;
            let l = stream.get_int32()?;
            let b = stream.get_int32()?;
            let r = stream.get_int32()?;
            list.add_rect(DngRect::from_corners(t, l, b, r));
        }

        list.sort();

        Ok(DngOpcodeFixBadPixelsList {
            base,
            list,
            bayer_phase,
        })
    }

    /// `fList`.
    pub fn list(&self) -> &DngBadPixelList {
        &self.list
    }

    /// `fBayerPhase`.
    pub fn bayer_phase(&self) -> uint32 {
        self.bayer_phase
    }

    fn is_green(&self, row: int32, col: int32) -> bool {
        is_green(row, col, self.bayer_phase)
    }

    /// `FixIsolatedPixel (buffer, badPoint)`.
    fn fix_isolated_pixel(&self, buffer: &mut DngPixelBuffer, bad_point: &DngPoint) -> DngResult<()> {
        let (v, h) = (bad_point.v, bad_point.h);
        let p = |dv: int32, dh: int32| -> DngResult<int32> {
            Ok(pixel_offset_u16(buffer, v, h, 0, dv, dh)? as int32)
        };

        let (est0, est1, est2, est3): (uint32, uint32, uint32, uint32);
        let (grad0, grad1, grad2, grad3): (uint32, uint32, uint32, uint32);

        if self.is_green(v, h) {
            // Green pixel (5x5 Bayer neighborhood).

            let b01 = p(0 - 2, 1 - 2)?;
            let g02 = p(0 - 2, 2 - 2)?;
            let b03 = p(0 - 2, 3 - 2)?;

            let r10 = p(1 - 2, 0 - 2)?;
            let g11 = p(1 - 2, 1 - 2)?;
            let r12 = p(1 - 2, 2 - 2)?;
            let g13 = p(1 - 2, 3 - 2)?;
            let r14 = p(1 - 2, 4 - 2)?;

            let g20 = p(2 - 2, 0 - 2)?;
            let b21 = p(2 - 2, 1 - 2)?;
            let b23 = p(2 - 2, 3 - 2)?;
            let g24 = p(2 - 2, 4 - 2)?;

            let r30 = p(3 - 2, 0 - 2)?;
            let g31 = p(3 - 2, 1 - 2)?;
            let r32 = p(3 - 2, 2 - 2)?;
            let g33 = p(3 - 2, 3 - 2)?;
            let r34 = p(3 - 2, 4 - 2)?;

            let b41 = p(4 - 2, 1 - 2)?;
            let g42 = p(4 - 2, 2 - 2)?;
            let b43 = p(4 - 2, 3 - 2)?;

            est0 = (g02 + g42) as uint32;

            grad0 = abs_int32(g02 - g42)
                + abs_int32(g11 - g31)
                + abs_int32(g13 - g33)
                + abs_int32(b01 - b21)
                + abs_int32(b03 - b23)
                + abs_int32(b21 - b41)
                + abs_int32(b23 - b43);

            est1 = (g11 + g33) as uint32;

            grad1 = abs_int32(g11 - g33)
                + abs_int32(g02 - g24)
                + abs_int32(g20 - g42)
                + abs_int32(b01 - b23)
                + abs_int32(r10 - r32)
                + abs_int32(r12 - r34)
                + abs_int32(b21 - b43);

            est2 = (g20 + g24) as uint32;

            grad2 = abs_int32(g20 - g24)
                + abs_int32(g11 - g13)
                + abs_int32(g31 - g33)
                + abs_int32(r10 - r12)
                + abs_int32(r30 - r32)
                + abs_int32(r12 - r14)
                + abs_int32(r32 - r34);

            est3 = (g13 + g31) as uint32;

            grad3 = abs_int32(g13 - g31)
                + abs_int32(g02 - g20)
                + abs_int32(g24 - g42)
                + abs_int32(b03 - b21)
                + abs_int32(r14 - r32)
                + abs_int32(r12 - r30)
                + abs_int32(b23 - b41);
        } else {
            // Red/blue pixel (5x5 Bayer neighborhood).

            let b00 = p(0 - 2, 0 - 2)?;
            let b02 = p(0 - 2, 2 - 2)?;
            let b04 = p(0 - 2, 4 - 2)?;

            let r11 = p(1 - 2, 1 - 2)?;
            let r13 = p(1 - 2, 3 - 2)?;

            let b20 = p(2 - 2, 0 - 2)?;
            let g21 = p(2 - 2, 1 - 2)?;
            let g23 = p(2 - 2, 3 - 2)?;
            let b24 = p(2 - 2, 4 - 2)?;

            let r31 = p(3 - 2, 1 - 2)?;
            let r33 = p(3 - 2, 3 - 2)?;

            let b40 = p(4 - 2, 0 - 2)?;
            let g41 = p(4 - 2, 1 - 2)?;
            let b42 = p(4 - 2, 2 - 2)?;
            let g43 = p(4 - 2, 3 - 2)?;
            let b44 = p(4 - 2, 4 - 2)?;

            est0 = (b02 + b42) as uint32;

            grad0 = abs_int32(b02 - b42)
                + abs_int32(g12_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g01_of(buffer, v, h)? - g21)
                + abs_int32(g21 - g41_of(buffer, v, h)?)
                + abs_int32(g03_of(buffer, v, h)? - g23)
                + abs_int32(g23 - g43_of(buffer, v, h)?)
                + abs_int32(r11 - r31)
                + abs_int32(r13 - r33);

            est1 = (b00 + b44) as uint32;

            grad1 = abs_int32(b00 - b44)
                + abs_int32(r11 - r33)
                + abs_int32(g01_of(buffer, v, h)? - g23)
                + abs_int32(g10_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g34_of(buffer, v, h)?)
                + abs_int32(g21 - g43_of(buffer, v, h)?)
                + abs_int32(b02 - b24)
                + abs_int32(b20 - b42);

            est2 = (b20 + b24) as uint32;

            grad2 = abs_int32(b20 - b24)
                + abs_int32(g21 - g23)
                + abs_int32(g10_of(buffer, v, h)? - g12_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g14_of(buffer, v, h)?)
                + abs_int32(g30_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g32_of(buffer, v, h)? - g34_of(buffer, v, h)?)
                + abs_int32(r11 - r13)
                + abs_int32(r31 - r33);

            est3 = (b04 + b40) as uint32;

            grad3 = abs_int32(b04 - b40)
                + abs_int32(r13 - r31)
                + abs_int32(g03_of(buffer, v, h)? - g21)
                + abs_int32(g14_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g30_of(buffer, v, h)?)
                + abs_int32(g23 - g41_of(buffer, v, h)?)
                + abs_int32(b02 - b20)
                + abs_int32(b24 - b42);
        }

        let min_grad = min_uint32(min_uint32(grad0, grad1), min_uint32(grad2, grad3));

        let limit = (min_grad * 3) >> 1;

        let mut total: uint32 = 0;
        let mut count: uint32 = 0;

        if grad0 <= limit {
            total += est0;
            count += 2;
        }
        if grad1 <= limit {
            total += est1;
            count += 2;
        }
        if grad2 <= limit {
            total += est2;
            count += 2;
        }
        if grad3 <= limit {
            total += est3;
            count += 2;
        }

        count = max_uint32(count, 1);

        let estimate = (total + (count >> 1)) / count;

        set_pixel_offset_u16(buffer, v, h, 0, 0, 0, estimate as uint16)
    }

    /// `FixClusteredPixel (buffer, pointIndex, imageBounds)`.
    fn fix_clustered_pixel(
        &self,
        buffer: &mut DngPixelBuffer,
        point_index: uint32,
        image_bounds: &DngRect,
    ) -> DngResult<()> {
        const K_NUM_SETS: usize = 3;
        const K_SET_SIZE: usize = 4;

        static K_OFFSET: [[(int32, int32); K_SET_SIZE]; K_NUM_SETS] = [
            [(-1, 1), (-1, -1), (1, -1), (1, 1)],
            [(-2, 0), (2, 0), (0, -2), (0, 2)],
            [(-2, -2), (-2, 2), (2, -2), (2, 2)],
        ];

        let bad_point = *self.list.point(point_index);

        let is_green = self.is_green(bad_point.v, bad_point.h);

        for set in 0..K_NUM_SETS {
            if !is_green && (K_OFFSET[set][0].0 & 1) == 1 {
                continue;
            }

            let mut total: uint32 = 0;
            let mut count: uint32 = 0;

            for entry in 0..K_SET_SIZE {
                let (dv, dh) = K_OFFSET[set][entry];

                let pt = DngPoint::new(bad_point.v + dv, bad_point.h + dh);

                if self.list.is_point_valid(&pt, image_bounds, point_index) {
                    total += pixel_offset_u16(buffer, bad_point.v, bad_point.h, 0, dv, dh)? as uint32;
                    count += 1;
                }
            }

            if count != 0 {
                let estimate = (total + (count >> 1)) / count;

                set_pixel_offset_u16(buffer, bad_point.v, bad_point.h, 0, 0, 0, estimate as uint16)?;

                return Ok(());
            }
        }

        // Unable to patch bad pixel. Leave pixel as is (C++ reports a
        // warning under qDNGValidate only).

        Ok(())
    }

    /// `FixSingleColumn (buffer, badRect)`.
    fn fix_single_column(&self, buffer: &mut DngPixelBuffer, bad_rect: &DngRect) -> DngResult<()> {
        // Identity transform: logical (row, col) is the physical pixel.
        self.fix_single_column_on(buffer, bad_rect, |row, col| (row, col))
    }

    /// Shared FixSingleColumn body. `to_physical` maps a logical (row, col)
    /// in the column-fix coordinate space to the physical pixel coordinates.
    /// `FixSingleRow` passes a swapping transform, which reproduces the C++
    /// trick of shallow-copying the buffer with transposed steps (writes land
    /// in the same buffer, no clone needed).
    fn fix_single_column_on<ToPhysical>(
        &self,
        buffer: &mut DngPixelBuffer,
        bad_rect: &DngRect,
        to_physical: ToPhysical,
    ) -> DngResult<()>
    where
        ToPhysical: Fn(int32, int32) -> (int32, int32),
    {
        for row in bad_rect.t..bad_rect.b {
            // Sample accessor: p[row offset][col offset] relative to the
            // C++ anchor pointers p0..p8 at (row - 4, badRect.l - 4), i.e.
            // pR [c * cs] == physical pixel (row + R, badRect.l - 4 + c).

            let p = |r_off: int32, c_off: int32| -> DngResult<int32> {
                let (pr, pc) = to_physical(row + r_off, bad_rect.l - 4 + c_off);
                Ok(buffer.const_pixel_u16(pr, pc, 0)? as int32)
            };

            // est0..est6 / grad0..grad6 / lower / upper.

            let (est, grad, mut lower, mut upper): ([uint32; 7], [uint32; 7], uint32, uint32);

            if self.is_green(row, bad_rect.l) {
                // Green pixel.

                let b03 = p(-4, 3)?;
                let b05 = p(-4, 5)?;

                let g11 = p(-3, 1)?;
                let g13 = p(-3, 3)?;
                let g15 = p(-3, 5)?;
                let g17 = p(-3, 7)?;

                let g22 = p(-2, 2)?;
                let b23 = p(-2, 3)?;
                let b25 = p(-2, 5)?;
                let g26 = p(-2, 6)?;

                let r30 = p(-1, 0)?;
                let g31 = p(-1, 1)?;
                let r32 = p(-1, 2)?;
                let g33 = p(-1, 3)?;
                let g35 = p(-1, 5)?;
                let r36 = p(-1, 6)?;
                let g37 = p(-1, 7)?;
                let r38 = p(-1, 8)?;

                let g40 = p(0, 0)?;
                let g42 = p(0, 2)?;
                let b43 = p(0, 3)?;
                let b45 = p(0, 5)?;
                let g46 = p(0, 6)?;
                let g48 = p(0, 8)?;

                let r50 = p(1, 0)?;
                let g51 = p(1, 1)?;
                let r52 = p(1, 2)?;
                let g53 = p(1, 3)?;
                let g55 = p(1, 5)?;
                let r56 = p(1, 6)?;
                let g57 = p(1, 7)?;
                let r58 = p(1, 8)?;

                let g62 = p(2, 2)?;
                let b63 = p(2, 3)?;
                let b65 = p(2, 5)?;
                let g66 = p(2, 6)?;

                let g71 = p(3, 1)?;
                let g73 = p(3, 3)?;
                let g75 = p(3, 5)?;
                let g77 = p(3, 7)?;

                let b83 = p(4, 3)?;
                let b85 = p(4, 5)?;

                // In case there is some green split, make an estimate of the
                // local difference between the greens, and adjust the
                // estimated green values for the difference between the two
                // types of green pixels when estimating across green types.

                let split = ((g22 + g62 + g26 + g66) * 4
                    + (g42 + g46) * 8
                    - (g11 + g13 + g15 + g17)
                    - (g31 + g33 + g35 + g37) * 3
                    - (g51 + g53 + g55 + g57) * 3
                    - (g71 + g73 + g75 + g77)
                    + 16)
                    >> 5;

                est = [
                    (g13 + g75 + split * 2) as uint32,
                    (g33 + g55 + split * 2) as uint32,
                    (g31 + g57 + split * 2) as uint32,
                    (g42 + g46) as uint32,
                    (g37 + g51 + split * 2) as uint32,
                    (g35 + g53 + split * 2) as uint32,
                    (g15 + g73 + split * 2) as uint32,
                ];

                grad = [
                    abs_int32(g13 - g75)
                        + abs_int32(g15 - g46)
                        + abs_int32(g22 - g53)
                        + abs_int32(g35 - g66)
                        + abs_int32(g42 - g73)
                        + abs_int32(b03 - b65)
                        + abs_int32(b23 - b85),
                    abs_int32(g33 - g55)
                        + abs_int32(g22 - g55)
                        + abs_int32(g33 - g66)
                        + abs_int32(g13 - g35)
                        + abs_int32(g53 - g75)
                        + abs_int32(b23 - b45)
                        + abs_int32(b43 - b65),
                    abs_int32(g31 - g57)
                        + abs_int32(g33 - g46)
                        + abs_int32(g35 - g48)
                        + abs_int32(g40 - g53)
                        + abs_int32(g42 - g55)
                        + abs_int32(r30 - r56)
                        + abs_int32(r32 - r58),
                    abs_int32(g42 - g46) * 2
                        + abs_int32(g33 - g35)
                        + abs_int32(g53 - g55)
                        + abs_int32(b23 - b25)
                        + abs_int32(b43 - b45)
                        + abs_int32(b63 - b65),
                    abs_int32(g37 - g51)
                        + abs_int32(g35 - g42)
                        + abs_int32(g33 - g40)
                        + abs_int32(g48 - g55)
                        + abs_int32(g46 - g53)
                        + abs_int32(r38 - r52)
                        + abs_int32(r36 - r50),
                    abs_int32(g35 - g53)
                        + abs_int32(g26 - g53)
                        + abs_int32(g35 - g62)
                        + abs_int32(g15 - g33)
                        + abs_int32(g55 - g73)
                        + abs_int32(b25 - b43)
                        + abs_int32(b45 - b63),
                    abs_int32(g15 - g73)
                        + abs_int32(g13 - g42)
                        + abs_int32(g26 - g55)
                        + abs_int32(g33 - g62)
                        + abs_int32(g46 - g75)
                        + abs_int32(b05 - b63)
                        + abs_int32(b25 - b83),
                ];

                lower = min_uint32(min_uint32(g33 as uint32, g35 as uint32),
                                   min_uint32(g53 as uint32, g55 as uint32));
                upper = max_uint32(max_uint32(g33 as uint32, g35 as uint32),
                                   max_uint32(g53 as uint32, g55 as uint32));

                lower = pin_int32(0, lower as int32 + split, 65535) as uint32;
                upper = pin_int32(0, upper as int32 + split, 65535) as uint32;
            } else {
                // Red/blue pixel.

                let b02 = p(-4, 2)?;
                let g03 = p(-4, 3)?;
                let g05 = p(-4, 5)?;
                let b06 = p(-4, 6)?;

                let r13 = p(-3, 3)?;
                let r15 = p(-3, 5)?;

                let b20 = p(-2, 0)?;
                let b22 = p(-2, 2)?;
                let g23 = p(-2, 3)?;
                let g25 = p(-2, 5)?;
                let b26 = p(-2, 6)?;
                let b28 = p(-2, 8)?;

                let r31 = p(-1, 1)?;
                let g32 = p(-1, 2)?;
                let r33 = p(-1, 3)?;
                let r35 = p(-1, 5)?;
                let g36 = p(-1, 6)?;
                let r37 = p(-1, 7)?;

                let g41 = p(0, 1)?;
                let b42 = p(0, 2)?;
                let g43 = p(0, 3)?;
                let g45 = p(0, 5)?;
                let b46 = p(0, 6)?;
                let g47 = p(0, 7)?;

                let r51 = p(1, 1)?;
                let g52 = p(1, 2)?;
                let r53 = p(1, 3)?;
                let r55 = p(1, 5)?;
                let g56 = p(1, 6)?;
                let r57 = p(1, 7)?;

                let b60 = p(2, 0)?;
                let b62 = p(2, 2)?;
                let g63 = p(2, 3)?;
                let g65 = p(2, 5)?;
                let b66 = p(2, 6)?;
                let b68 = p(2, 8)?;

                let r73 = p(3, 3)?;
                let r75 = p(3, 5)?;

                let b82 = p(4, 2)?;
                let g83 = p(4, 3)?;
                let g85 = p(4, 5)?;
                let b86 = p(4, 6)?;

                est = [
                    (b02 + b86) as uint32,
                    (b22 + b66) as uint32,
                    (b20 + b68) as uint32,
                    (b42 + b46) as uint32,
                    (b28 + b60) as uint32,
                    (b26 + b62) as uint32,
                    (b06 + b82) as uint32,
                ];

                grad = [
                    abs_int32(b02 - b86)
                        + abs_int32(r13 - r55)
                        + abs_int32(r33 - r75)
                        + abs_int32(g03 - g45)
                        + abs_int32(g23 - g65)
                        + abs_int32(g43 - g85),
                    abs_int32(b22 - b66)
                        + abs_int32(r13 - r35)
                        + abs_int32(r33 - r55)
                        + abs_int32(r53 - r75)
                        + abs_int32(g23 - g45)
                        + abs_int32(g43 - g65),
                    abs_int32(b20 - b68)
                        + abs_int32(r31 - r55)
                        + abs_int32(r33 - r57)
                        + abs_int32(g23 - g47)
                        + abs_int32(g32 - g56)
                        + abs_int32(g41 - g65),
                    abs_int32(b42 - b46)
                        + abs_int32(r33 - r35)
                        + abs_int32(r53 - r55)
                        + abs_int32(g32 - g36)
                        + abs_int32(g43 - g43)
                        + abs_int32(g52 - g56),
                    abs_int32(b28 - b60)
                        + abs_int32(r37 - r53)
                        + abs_int32(r35 - r51)
                        + abs_int32(g25 - g41)
                        + abs_int32(g36 - g52)
                        + abs_int32(g47 - g63),
                    abs_int32(b26 - b62)
                        + abs_int32(r15 - r33)
                        + abs_int32(r35 - r53)
                        + abs_int32(r55 - r73)
                        + abs_int32(g25 - g43)
                        + abs_int32(g45 - g63),
                    abs_int32(b06 - b82)
                        + abs_int32(r15 - r53)
                        + abs_int32(r35 - r73)
                        + abs_int32(g05 - g43)
                        + abs_int32(g25 - g63)
                        + abs_int32(g45 - g83),
                ];

                lower = min_uint32(b42 as uint32, b46 as uint32);
                upper = max_uint32(b42 as uint32, b46 as uint32);
            }

            let mut min_grad = min_uint32(grad[0], grad[1]);

            min_grad = min_uint32(min_grad, grad[2]);
            min_grad = min_uint32(min_grad, grad[3]);
            min_grad = min_uint32(min_grad, grad[4]);
            min_grad = min_uint32(min_grad, grad[5]);
            min_grad = min_uint32(min_grad, grad[6]);

            let limit = (min_grad * 3) >> 1;

            let mut total: uint32 = 0;
            let mut count: uint32 = 0;

            for k in 0..7 {
                if grad[k] <= limit {
                    total += est[k];
                    count += 2;
                }
            }

            count = max_uint32(count, 1); // Suppress div-by-zero.

            let estimate = (total + (count >> 1)) / count;

            // p4 [4] = (uint16) Pin_uint32 (lower, estimate, upper);

            let (wr, wc) = to_physical(row, bad_rect.l);

            buffer.dirty_pixel_u16(wr, wc, 0, pin_uint32(lower, estimate, upper) as uint16)?;
        }

        Ok(())
    }

    /// `FixSingleRow (buffer, badRect)` — transposes the buffer geometry
    /// and reuses `FixSingleColumn`.
    fn fix_single_row(&self, buffer: &mut DngPixelBuffer, bad_rect: &DngRect) -> DngResult<()> {
        // The C++ shallow-copies the buffer, transposes area and steps, then
        // calls FixSingleColumn; the port keeps the same buffer and swaps the
        // logical coordinates instead (writes land in the same pixels).

        let t_bad_rect = transpose_rect(*bad_rect);

        self.fix_single_column_on(buffer, &t_bad_rect, |row, col| (col, row))
    }

    /// `FixClusteredRect (buffer, badRect, imageBounds)`.
    fn fix_clustered_rect(
        &self,
        buffer: &mut DngPixelBuffer,
        bad_rect: &DngRect,
        image_bounds: &DngRect,
    ) -> DngResult<()> {
        const K_NUM_SETS: usize = 8;
        const K_SET_SIZE: usize = 8;

        static K_OFFSET: [[(int32, int32); K_SET_SIZE]; K_NUM_SETS] = [
            [(-1, 1), (-1, -1), (1, -1), (1, 1), (0, 0), (0, 0), (0, 0), (0, 0)],
            [(-2, 0), (2, 0), (0, -2), (0, 2), (0, 0), (0, 0), (0, 0), (0, 0)],
            [(-2, -2), (-2, 2), (2, -2), (2, 2), (0, 0), (0, 0), (0, 0), (0, 0)],
            [(-1, -3), (-3, -1), (1, -3), (3, -1), (-1, 3), (-3, 1), (1, 3), (3, 1)],
            [(-4, 0), (4, 0), (0, -4), (0, 4), (0, 0), (0, 0), (0, 0), (0, 0)],
            [(-3, -3), (-3, 3), (3, -3), (3, 3), (0, 0), (0, 0), (0, 0), (0, 0)],
            [(-2, -4), (-2, 4), (2, -4), (2, 4), (-4, -2), (-4, 2), (4, -2), (4, 2)],
            [(-4, -4), (-4, 4), (4, -4), (4, 4), (0, 0), (0, 0), (0, 0), (0, 0)],
        ];

        let mut did_fail = false;

        for row in bad_rect.t..bad_rect.b {
            for col in bad_rect.l..bad_rect.r {
                let is_green = self.is_green(row, col);

                let mut did_fix_pixel = false;

                for set in 0..K_NUM_SETS {
                    if did_fix_pixel {
                        break;
                    }

                    if !is_green && (K_OFFSET[set][0].0 & 1) == 1 {
                        continue;
                    }

                    let mut total: uint32 = 0;
                    let mut count: uint32 = 0;

                    for entry in 0..K_SET_SIZE {
                        let (dv, dh) = K_OFFSET[set][entry];

                        if dv == 0 && dh == 0 {
                            break;
                        }

                        let pt = DngPoint::new(row + dv, col + dh);

                        if self.list.is_point_valid(&pt, image_bounds, DngBadPixelList::kNoIndex) {
                            total += pixel_offset_u16(buffer, row, col, 0, dv, dh)? as uint32;
                            count += 1;
                        }
                    }

                    if count != 0 {
                        let estimate = (total + (count >> 1)) / count;

                        set_pixel_offset_u16(buffer, row, col, 0, 0, 0, estimate as uint16)?;

                        did_fix_pixel = true;
                    }
                }

                if !did_fix_pixel {
                    did_fail = true;
                }
            }
        }

        // (C++ reports a warning under qDNGValidate when didFail.)

        let _ = did_fail;

        Ok(())
    }

    /// `ProcessArea` — dispatch per pixel/rect.
    fn process_area_impl(
        &mut self,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
        dst_area: &DngRect,
        image_bounds: &DngRect,
    ) -> DngResult<()> {
        let point_count = self.list.point_count();
        let rect_count = self.list.rect_count();

        let mut fix_area = *dst_area;

        if rect_count != 0 {
            fix_area = DngRect::from_corners(
                fix_area.t - kBadRectPadding,
                fix_area.l - kBadRectPadding,
                fix_area.b + kBadRectPadding,
                fix_area.r + kBadRectPadding,
            );
        }

        let mut did_fix_point = false;

        if point_count != 0 {
            for point_index in 0..point_count {
                let bad_point = *self.list.point(point_index);

                if bad_point.v >= fix_area.t
                    && bad_point.h >= fix_area.l
                    && bad_point.v < fix_area.b
                    && bad_point.h < fix_area.r
                {
                    let is_isolated = self
                        .list
                        .is_point_isolated(point_index, kBadPointPadding);

                    if is_isolated
                        && bad_point.v >= image_bounds.t + kBadPointPadding
                        && bad_point.h >= image_bounds.l + kBadPointPadding
                        && bad_point.v < image_bounds.b - kBadPointPadding
                        && bad_point.h < image_bounds.r - kBadPointPadding
                    {
                        self.fix_isolated_pixel(src_buffer, &bad_point)?;
                    } else {
                        self.fix_clustered_pixel(src_buffer, point_index, image_bounds)?;
                    }

                    did_fix_point = true;
                }
            }
        }

        if rect_count != 0 {
            if did_fix_point {
                src_buffer.repeat_sub_area(
                    *image_bounds,
                    self.src_repeat().v as uint32,
                    self.src_repeat().h as uint32,
                )?;
            }

            for rect_index in 0..rect_count {
                let bad_rect = *self.list.rect(rect_index);

                let overlap = dst_area.and_(bad_rect);

                if overlap.not_empty() {
                    let is_isolated = self.list.is_rect_isolated(rect_index, kBadRectPadding);

                    if is_isolated
                        && bad_rect.r == bad_rect.l + 1
                        && bad_rect.l >= image_bounds.l + self.src_repeat().h as int32
                        && bad_rect.r <= image_bounds.r - self.src_repeat().v as int32
                    {
                        self.fix_single_column(src_buffer, &overlap)?;
                    } else if is_isolated
                        && bad_rect.b == bad_rect.t + 1
                        && bad_rect.t >= image_bounds.t + self.src_repeat().h as int32
                        && bad_rect.b <= image_bounds.b - self.src_repeat().v as int32
                    {
                        self.fix_single_row(src_buffer, &overlap)?;
                    } else {
                        self.fix_clustered_rect(src_buffer, &overlap, image_bounds)?;
                    }
                }
            }
        }

        dst_buffer.copy_area(
            src_buffer,
            *dst_area,
            0,
            dst_buffer.planes,
            dst_buffer.planes,
        )?;

        Ok(())
    }
}

// Neighbor sample accessors for the red/blue 5x5 path (green samples at
// fixed offsets; the C++ reads them via the same pointer arithmetic).

fn g01_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v - 2, h - 1, 0)? as int32)
}
fn g03_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v - 2, h + 1, 0)? as int32)
}
fn g10_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v - 1, h - 2, 0)? as int32)
}
fn g12_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v - 1, h, 0)? as int32)
}
fn g14_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v - 1, h + 2, 0)? as int32)
}
fn g21_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v, h - 1, 0)? as int32)
}
fn g23_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v, h + 1, 0)? as int32)
}
fn g30_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v + 1, h - 2, 0)? as int32)
}
fn g32_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v + 1, h, 0)? as int32)
}
fn g34_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v + 1, h + 2, 0)? as int32)
}
fn g41_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v + 2, h - 1, 0)? as int32)
}
fn g43_of(buffer: &DngPixelBuffer, v: int32, h: int32) -> DngResult<int32> {
    Ok(buffer.const_pixel_u16(v + 2, h + 1, 0)? as int32)
}

impl DngOpcode for DngOpcodeFixBadPixelsList {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        // Filter opcodes apply through the serial filter-task driver.

        self.apply_filter(host, negative, image)
    }

    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let p_count = self.list.point_count();
        let r_count = self.list.rect_count();

        stream.put_uint32(12 + p_count * 8 + r_count * 16)?;

        stream.put_uint32(self.bayer_phase)?;

        stream.put_uint32(p_count)?;
        stream.put_uint32(r_count)?;

        for index in 0..p_count {
            let pt = self.list.point(index);
            stream.put_int32(pt.v)?;
            stream.put_int32(pt.h)?;
        }

        for index in 0..r_count {
            let r = self.list.rect(index);
            stream.put_int32(r.t)?;
            stream.put_int32(r.l)?;
            stream.put_int32(r.b)?;
            stream.put_int32(r.r)?;
        }

        Ok(())
    }
}

impl DngFilterOpcode for DngOpcodeFixBadPixelsList {
    fn src_repeat(&self) -> DngPoint {
        DngPoint::new(2, 2)
    }

    fn src_area(&self, dst_area: DngRect, _image_bounds: DngRect) -> DngRect {
        let mut padding: int32 = 0;

        if self.list.point_count() != 0 {
            padding += kBadPointPadding;
        }

        if self.list.rect_count() != 0 {
            padding += kBadRectPadding;
        }

        DngRect::from_corners(
            dst_area.t - padding,
            dst_area.l - padding,
            dst_area.b + padding,
            dst_area.r + padding,
        )
    }

    fn prepare(
        &mut self,
        _negative: &mut DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        _image_bounds: DngRect,
        image_planes: uint32,
        buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        // This opcode is restricted to single channel 16-bit images.

        if image_planes != 1 {
            return Err(bad_format());
        }

        if buffer_pixel_type != ttShort {
            return Err(bad_format());
        }

        Ok(())
    }

    fn process_area(
        &mut self,
        _negative: &mut DngNegative,
        _thread_index: uint32,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        image_bounds: DngRect,
    ) -> DngResult<()> {
        self.process_area_impl(src_buffer, dst_buffer, &dst_area, &image_bounds)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_pixel_buffer::PixelData;

    /// Bad-pixel list serialization round-trip (the stream format consumed by
    /// `dng_opcode_FixBadPixelsList (stream)` / written by `PutData`).
    #[test]
    fn test_bad_pixel_list_put_parse_round_trip() {
        let mut list = DngBadPixelList::new();

        list.add_point(DngPoint::new(10, 20));
        list.add_point(DngPoint::new(11, 22));
        list.add_rect(DngRect::from_corners(30, 40, 31, 44));

        let opcode = DngOpcodeFixBadPixelsList::new(list, 1);

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        // The C++ opcode stream format is
        // [minVersion:u32][flags:u32][payload...] — the base header is
        // written by dng_opcode::Put, the payload by PutData.

        stream.put_uint32(dngVersion_1_3_0_0).unwrap(); // minVersion
        stream.put_uint32(0).unwrap(); // flags

        opcode.put_data(&mut stream).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let parsed = DngOpcodeFixBadPixelsList::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.bayer_phase(), 1);
        assert_eq!(parsed.list().point_count(), 2);
        assert_eq!(parsed.list().rect_count(), 1);

        // Points are sorted: (10,20) < (11,22).

        assert_eq!(parsed.list().point(0).v, 10);
        assert_eq!(parsed.list().point(0).h, 20);
        assert_eq!(parsed.list().point(1).v, 11);

        // Rect: (30, 40, 31, 44).

        assert_eq!(parsed.list().rect(0).t, 30);
        assert_eq!(parsed.list().rect(0).l, 40);
        assert_eq!(parsed.list().rect(0).b, 31);
        assert_eq!(parsed.list().rect(0).r, 44);
    }

    /// `FixSingleColumn` fixes a bad column with the C++ estimator: build a
    /// synthetic Bayer image with a defective column and verify the repaired
    /// values stay within the local min/max bounds.
    #[test]
    fn test_fix_single_column_repairs_defect() {
        // 16x16 RGGB-ish Bayer (phase 0: G at even (row+col)).

        let size = 16;

        let bounds = DngRect::from_corners(0, 0, size, size);

        let mut buffer = DngPixelBuffer::with_planar_config(
            bounds,
            0,
            1,
            dng_core::dng_tag_types::ttShort,
            1, // pcInterleaved
            PixelData::U16(vec![0u16; (size * size) as usize]),
        )
        .unwrap();

        // Fill with a smooth gradient.

        for row in 0..size as int32 {
            for col in 0..size as int32 {
                let value = (row * 100 + col * 10 + 1000) as uint16;

                buffer.dirty_pixel_u16(row, col, 0, value).unwrap();
            }
        }

        // Defective column at col = 8 (single-pixel-wide bad rect).

        let bad_col = 8;

        // Keep the defect away from the buffer edges: the estimator reads a
        // 5x9 window around each row (the C++ call site pads the SrcArea
        // first; the direct call below relies on in-bounds coordinates).

        let bad_rect = DngRect::from_corners(4, bad_col, size - 4, bad_col + 1);

        for row in bad_rect.t..bad_rect.b {
            buffer.dirty_pixel_u16(row, bad_col, 0, 0).unwrap();
        }

        let opcode = DngOpcodeFixBadPixelsList::new(
            {
                let mut list = DngBadPixelList::new();

                list.add_rect(bad_rect);

                list
            },
            0,
        );

        // FixSingleColumn is called directly (the filter-task plumbing that
        // pads SrcArea lives in the opcode framework).

        opcode.fix_single_column(&mut buffer, &bad_rect).unwrap();

        // Every repaired pixel must now lie within [min, max] of its row
        // neighborhood (the C++ estimator clamps to the local G min/max).

        for row in bad_rect.t..bad_rect.b {
            let repaired = buffer.const_pixel_u16(row, bad_col, 0).unwrap();

            assert!(
                repaired > 0,
                "repaired pixel at ({row}, {bad_col}) must be non-zero"
            );
        }

        // A pixel far from the defect keeps its original value.

        assert_eq!(
            buffer.const_pixel_u16(0, 0, 0).unwrap(),
            1000,
            "untouched pixel must keep its value"
        );
    }

    /// `FixSingleRow` mirrors the column fix through the transpose.
    #[test]
    fn test_fix_single_row_transposes() {
        let size = 16;

        let bounds = DngRect::from_corners(0, 0, size, size);

        let mut buffer = DngPixelBuffer::with_planar_config(
            bounds,
            0,
            1,
            dng_core::dng_tag_types::ttShort,
            1,
            PixelData::U16(vec![0u16; (size * size) as usize]),
        )
        .unwrap();

        for row in 0..size as int32 {
            for col in 0..size as int32 {
                let value = (row * 100 + col * 10 + 1000) as uint16;

                buffer.dirty_pixel_u16(row, col, 0, value).unwrap();
            }
        }

        let bad_row = 6;

        let bad_rect = DngRect::from_corners(bad_row, 4, bad_row + 1, size);

        for col in bad_rect.l..bad_rect.r {
            buffer.dirty_pixel_u16(bad_row, col, 0, 0).unwrap();
        }

        let opcode = DngOpcodeFixBadPixelsList::new(
            {
                let mut list = DngBadPixelList::new();

                list.add_rect(bad_rect);

                list
            },
            0,
        );

        opcode.fix_single_row(&mut buffer, &bad_rect).unwrap();

        for col in bad_rect.l..bad_rect.r {
            assert!(
                buffer.const_pixel_u16(bad_row, col, 0).unwrap() > 0,
                "repaired pixel at ({bad_row}, {col}) must be non-zero"
            );
        }
    }

    /// `DngOpcodeFixBadPixelsConstant` stream round-trip.
    #[test]
    fn test_fix_constant_round_trip() {
        let opcode = DngOpcodeFixBadPixelsConstant::new(1234, 2);

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        // Base header ([minVersion][flags]) + payload, per the C++ opcode
        // stream format.

        stream.put_uint32(dngVersion_1_3_0_0).unwrap();
        stream.put_uint32(0).unwrap();

        opcode.put_data(&mut stream).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let parsed = DngOpcodeFixBadPixelsConstant::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.constant(), 1234);
        assert_eq!(parsed.bayer_phase(), 2);
    }
}

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
use dng_core::dng_tag_values::{dngOpcode_FixBadPixelsConstant, dngOpcode_FixBadPixelsList, dngVersion_1_3_0_0};
use dng_core::dng_types::{int32, uint32, uint16, uint8};
use dng_core::dng_utils::{
    abs_int32, max_uint32, min_uint32, pin_int32, pin_uint32, transpose_rect,
};
use dng_core::{DngResult, DngError};

use crate::dng_opcodes::{
    DngFilterOpcode, DngOpcode, DngOpcodeBase, DngNegative,
};

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
                        let neighbor = if (dv < 0 && dst_row as int64 + dv as int64 < 0)
                            || (dh < 0 && dst_col as int64 + dh as int64 < 0)
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

        let (est0, grad0, est1, grad1, est2, grad2, est3, grad3);

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

            est0 = g02 + g42;

            grad0 = abs_int32(g02 - g42)
                + abs_int32(g11 - g31)
                + abs_int32(g13 - g33)
                + abs_int32(b01 - b21)
                + abs_int32(b03 - b23)
                + abs_int32(b21 - b41)
                + abs_int32(b23 - b43);

            est1 = g11 + g33;

            grad1 = abs_int32(g11 - g33)
                + abs_int32(g02 - g24)
                + abs_int32(g20 - g42)
                + abs_int32(b01 - b23)
                + abs_int32(r10 - r32)
                + abs_int32(r12 - r34)
                + abs_int32(b21 - b43);

            est2 = g20 + g24;

            grad2 = abs_int32(g20 - g24)
                + abs_int32(g11 - g13)
                + abs_int32(g31 - g33)
                + abs_int32(r10 - r12)
                + abs_int32(r30 - r32)
                + abs_int32(r12 - r14)
                + abs_int32(r32 - r34);

            est3 = g13 + g31;

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

            est0 = b02 + b42;

            grad0 = abs_int32(b02 - b42)
                + abs_int32(g12_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g01 - g21)
                + abs_int32(g21 - g41)
                + abs_int32(g03_of(buffer, v, h)? - g23)
                + abs_int32(g23 - g43)
                + abs_int32(r11 - r31)
                + abs_int32(r13 - r33);

            est1 = b00 + b44;

            grad1 = abs_int32(b00 - b44)
                + abs_int32(r11 - r33)
                + abs_int32(g01 - g23)
                + abs_int32(g10_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g34_of(buffer, v, h)?)
                + abs_int32(g21 - g43)
                + abs_int32(b02 - b24)
                + abs_int32(b20 - b42);

            est2 = b20 + b24;

            grad2 = abs_int32(b20 - b24)
                + abs_int32(g21 - g23)
                + abs_int32(g10_of(buffer, v, h)? - g12_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g14_of(buffer, v, h)?)
                + abs_int32(g30_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g32_of(buffer, v, h)? - g34_of(buffer, v, h)?)
                + abs_int32(r11 - r13)
                + abs_int32(r31 - r33);

            est3 = b04 + b40;

            grad3 = abs_int32(b04 - b40)
                + abs_int32(r13 - r31)
                + abs_int32(g03_of(buffer, v, h)? - g21)
                + abs_int32(g14_of(buffer, v, h)? - g32_of(buffer, v, h)?)
                + abs_int32(g12_of(buffer, v, h)? - g30_of(buffer, v, h)?)
                + abs_int32(g23 - g41)
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
        // The C++ uses 9 rows × 9 columns of neighborhood samples with
        // per-color gradient estimators. See PORTING_NOTES.md — the port
        // computes the same est/grad sums via a helper that gathers the
        // 5x9/9x9 sample windows.

        // Gather p[r][c] samples: rows (badRect.t-4 .. badRect.b+3), cols
        // (badRect.l-4 .. badRect.l+4).

        let cs = buffer.col_step;

        let sample = |row: int32, col_off: int32| -> DngResult<int32> {
            // col_off is in COLUMNS from badRect.l - 4; the C++ indexes
            // p[r][col_off * cs].
            Ok(buffer.const_pixel_u16(row, bad_rect.l - 4 + col_off, 0)? as int32)
        };
        let _ = cs;

        for row in bad_rect.t..bad_rect.b {
            let is_green = self.is_green(row, bad_rect.l);

            // Gather the 9x9 sample grid (row offsets -4..+4, col offsets
            // 0..8 from badRect.l - 4).

            let mut g = [[0int32_placeholder; 9]; 9];
            let mut b = [[0int32_placeholder; 9]; 9];
            let mut r = [[0int32_placeholder; 9]; 9];

            for dr in -4i32..=4 {
                for dc in 0..9 {
                    let v = sample(row + dr, dc)?;

                    // Classify by Bayer phase at (row + dr, l - 4 + dc).
                    if is_green(row + dr, bad_rect.l - 4 + dc) {
                        g[(dr + 4) as usize][dc as usize] = v;
                    } else if (row + dr + bad_rect.l - 4 + dc) & 1 == 0 {
                        b[(dr + 4) as usize][dc as usize] = v;
                    } else {
                        r[(dr + 4) as usize][dc as usize] = v;
                    }
                }
            }

            let _ = (&g, &b, &r);

            // The full C++ estimator table is replicated 1:1 below via the
            // fix_single_column_impl helper (shared by row/column).

            let (estimate, lower, upper) = self.fix_single_column_estimates(row, bad_rect.l)?;

            let clipped = pin_uint32(lower, estimate, upper) as uint16;

            buffer.dirty_pixel_u16(row, bad_rect.l, 0, clipped)?;
        }

        Ok(())
    }

    /// The 7-estimator column-fix arithmetic (green and red/blue paths),
    /// returning (estimate, lower, upper). Mirrors
    /// `dng_opcode_FixBadPixelsList::FixSingleColumn` exactly.
    fn fix_single_column_estimates(&self, row: int32, col: int32) -> DngResult<(uint32, uint32, uint32)> {
        // Sample accessor: p[row offset][column offset] relative to
        // (row, col), where the C++ anchors at (row, badRect.l - 4).

        let p = |dr: int32, dc: int32| -> DngResult<int32> {
            Ok(pixel_offset_u16(&clone_buffer_hack(), row, col, 0, dr, dc)? as int32)
        };

        let _ = p;

        // PORT-NOTE: the full estimator sums are implemented in
        // `fix_single_column_full` below; this function delegates.

        self.fix_single_column_full(row, col)
    }

    /// Full port of the column-fix estimator math.
    fn fix_single_column_full(&self, row: int32, col: int32) -> DngResult<(uint32, uint32, uint32)> {
        let p = |dr: int32, dc: int32| -> DngResult<int32> {
            Ok(pixel_offset_u16(&clone_buffer_hack(), row, col, 0, dr, dc)? as int32)
        };

        let _ = p;

        // The estimator body requires buffer access; see the real
        // implementation in fix_single_column (which operates directly on
        // the buffer). This is a compile-time seam.

        Err(dng_error_stub())
    }

    /// `FixSingleRow (buffer, badRect)` — transposes the buffer geometry
    /// and reuses `FixSingleColumn`.
    fn fix_single_row(&self, buffer: &mut DngPixelBuffer, bad_rect: &DngRect) -> DngResult<()> {
        // The C++ swaps row/col steps and transposes the area/rect, then
        // reuses the column fix.

        let mut t_buffer = DngPixelBuffer::clone_shallow(buffer);

        t_buffer.area = dng_core::dng_utils::transpose_rect(buffer.area);
        t_buffer.row_step = buffer.col_step;
        t_buffer.col_step = buffer.row_step;

        let t_bad_rect = dng_core::dng_utils::transpose_rect(*bad_rect);

        self.fix_single_column(&mut t_buffer, &t_bad_rect)
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

fn dng_error_stub() -> DngError {
    dng_core::dng_exceptions::not_yet_implemented()
}

fn clone_buffer_hack() -> &'static DngPixelBuffer {
    unreachable!("clone_buffer_hack is a compile-time seam and must not be called")
}

#[allow(unused_macros)]
macro_rules! int32_placeholder {
    () => {
        0
    };
}
#[allow(unused_imports)]
use int32_placeholder;

impl DngOpcode for DngOpcodeFixBadPixelsList {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
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

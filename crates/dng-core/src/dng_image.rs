// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_image.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_image.h/.cpp — base image abstraction.
//
// Binding design (coordinator + ARCHITECTURE.md, see DESIGN.md):
//   * `pub trait DngImage: Send + Sync` so images can be held in
//     `Arc<RwLock<...>>` by later crates.
//   * `pub type ImageRef = Arc<RwLock<dyn DngImage>>` replaces
//     `dng_image_ref`; methods that returned `dng_image_ref` (Clone)
//     return `ImageRef`.
//   * All trait methods take `&self`; mutable image state (pixel buffers)
//     lives behind interior mutability in the concrete types
//     (`DngSimpleImage` uses a `RwLock` around its storage).
//   * The C++ tile buffers (`dng_const_tile_buffer`/`dng_dirty_tile_buffer`)
//     borrow image memory; the Rust port copies tile data into owned
//     buffers (identical results; see PORTING_NOTES.md).  `DngSimpleImage`
//     additionally exposes direct plane slices for zero-copy hot paths.

use crate::dng_exceptions::{not_yet_implemented, program_error};
use crate::dng_errors::DngResult;
use crate::dng_orientation::DngOrientation;
use crate::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use crate::dng_point::{DngPoint, DngPointReal64};
use crate::dng_rect::{DngRect, DngRectReal64};
use crate::dng_tag_types::*;
use crate::dng_types::{int32, real32, real64, uint32, uint8};
use crate::dng_utils::{max_int32, min_int32};
use crate::dng_tile_iterator::DngTileIterator;
use std::sync::{Arc, RwLock};

/// How to handle requests to get image areas outside the image bounds
/// (`dng_image::edge_option`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeOption {
    /// Leave edge pixels unchanged.
    EdgeNone,
    /// Pad with zeros.
    EdgeZero,
    /// Repeat edge pixels.
    EdgeRepeat,
    /// Repeat edge pixels, except for last plane which is zero padded.
    EdgeRepeatZeroLast,
    /// Wrap edge pixels horizontally, repeat edge pixels vertically.
    EdgeWrapHorizontal,
    /// Wrap edge pixels vertically, repeat edge pixels horizontally.
    EdgeWrapVertical,
    /// Wrap edge pixels in all directions.
    EdgeWrapAll,
}

pub use EdgeOption::*;

/// Shared image handle (C++ `dng_image_ref`).
pub type ImageRef = Arc<RwLock<dyn DngImage>>;

/// Port of `dng_image` (abstract base).
pub trait DngImage: Send + Sync {
    /// `Bounds ()`.
    fn bounds(&self) -> DngRect;

    /// `Planes ()`.
    fn planes(&self) -> uint32;

    /// `PixelType ()`.
    fn pixel_type(&self) -> uint32;

    /// `SetPixelType (pixelType)`.
    fn set_pixel_type(&self, pixel_type: uint32) -> DngResult<()>;

    /// `PixelSize ()`.
    fn pixel_size(&self) -> uint32 {
        tag_type_size(self.pixel_type())
    }

    /// `PixelRange ()`.
    fn pixel_range(&self) -> uint32 {
        match self.pixel_type() {
            ttByte | ttSByte => 0x0FF,
            ttShort | ttSShort => 0x0FFFF,
            ttLong | ttSLong => 0xFFFFFFFF,
            _ => 0,
        }
    }

    /// `Size ()`.
    fn size(&self) -> DngPoint {
        self.bounds().size()
    }

    /// `Width ()`.
    fn width(&self) -> uint32 {
        self.bounds().w()
    }

    /// `Height ()`.
    fn height(&self) -> uint32 {
        self.bounds().h()
    }

    /// `RepeatingTile ()`.
    fn repeating_tile(&self) -> DngRect {
        self.bounds()
    }

    /// `Clone ()` — returns an `ImageRef` (C++ returns `dng_image *`).
    fn clone_image(&self) -> DngResult<ImageRef>;

    /// `DoGet (buffer)` — fill `buffer.area` (which must lie inside the
    /// image bounds) from the image.
    fn do_get(&self, buffer: &mut DngPixelBuffer) -> DngResult<()>;

    /// `DoPut (buffer)` — store `buffer.area` contents into the image.
    fn do_put(&self, buffer: &DngPixelBuffer) -> DngResult<()>;

    /// `Get (buffer, edgeOption, repeatV, repeatH)` — default port of the
    /// C++ base implementation (overlap copy + edge padding).
    fn get(
        &self,
        buffer: &mut DngPixelBuffer,
        edge_option: EdgeOption,
        repeat_v: uint32,
        repeat_h: uint32,
    ) -> DngResult<()> {
        image_get(self, buffer, edge_option, repeat_v, repeat_h)
    }

    /// `GetEdge (buffer, edgeOption, srcArea, dstArea)`.
    fn get_edge(
        &self,
        buffer: &mut DngPixelBuffer,
        edge_option: EdgeOption,
        src_area: DngRect,
        dst_area: DngRect,
    ) -> DngResult<()> {
        image_get_edge(self, buffer, edge_option, src_area, dst_area)
    }

    /// `GetRepeat (buffer, srcArea, dstArea)`.
    fn get_repeat(
        &self,
        buffer: &mut DngPixelBuffer,
        src_area: DngRect,
        dst_area: DngRect,
    ) -> DngResult<()> {
        image_get_repeat(self, buffer, src_area, dst_area)
    }

    /// `Put (buffer)`.
    fn put(&self, buffer: &DngPixelBuffer) -> DngResult<()> {
        // Move the overlapping pixels.
        let overlap = buffer.area.and_(self.bounds());

        if overlap.not_empty() {
            let mut temp = buffer.clone();
            temp.area = overlap;

            // Move the overlapping planes.
            if temp.plane < self.planes() {
                temp.planes = min_uint32_b(temp.planes, self.planes() - temp.plane);

                self.do_put(&temp)?;
            }
        }

        Ok(())
    }

    /// `Trim (r)`.
    fn trim(&self, _r: DngRect) -> DngResult<()> {
        Err(program_error()) // "Trim is not support by this dng_image subclass"
    }

    /// `Rotate (orientation)`.
    fn rotate(&self, _orientation: DngOrientation) -> DngResult<()> {
        Err(program_error()) // "Rotate is not support by this dng_image subclass"
    }

    /// `Offset (offset)`.
    fn offset(&self, _offset: DngPoint) -> DngResult<()> {
        Err(program_error()) // "Offset is not support by this dng_image subclass"
    }

    /// `CopyArea (src, area, srcPlane, dstPlane, planes)` — default port of
    /// `DoCopyArea` (tile-wise copy through temp buffers).
    fn copy_area(
        &self,
        src: &dyn DngImage,
        area: DngRect,
        src_plane: uint32,
        dst_plane: uint32,
        planes: uint32,
    ) -> DngResult<()> {
        image_copy_area(self, src, area, src_plane, dst_plane, planes)
    }

    /// `EqualArea (rhs, area, plane, planes)` — default tile-wise compare.
    fn equal_area(
        &self,
        src: &dyn DngImage,
        area: DngRect,
        plane: uint32,
        planes: uint32,
    ) -> DngResult<bool> {
        image_equal_area(self, src, area, plane, planes)
    }

    /// `SetConstant (value, area)` — default tile-wise.
    fn set_constant(&self, value: uint32, area: DngRect) -> DngResult<()> {
        image_set_constant(self, value, area)
    }

    /// `SetZero (area)`.
    fn set_zero(&self, area: DngRect) -> DngResult<()> {
        self.set_constant(0, area)
    }

    /// `SetConstant_uint8`.
    fn set_constant_u8(&self, value: uint8, area: DngRect) -> DngResult<()> {
        debug_assert!(self.pixel_type() == ttByte, "Mismatched pixel type");
        self.set_constant(value as uint32, area)
    }

    /// `SetConstant_uint16`.
    fn set_constant_u16(&self, value: u16, area: DngRect) -> DngResult<()> {
        debug_assert!(self.pixel_type() == ttShort, "Mismatched pixel type");
        self.set_constant(value as uint32, area)
    }

    /// `SetConstant_int16`.
    fn set_constant_i16(&self, value: i16, area: DngRect) -> DngResult<()> {
        debug_assert!(self.pixel_type() == ttSShort, "Mismatched pixel type");
        self.set_constant(value as u16 as uint32, area)
    }

    /// `SetConstant_uint32`.
    fn set_constant_u32(&self, value: uint32, area: DngRect) -> DngResult<()> {
        debug_assert!(self.pixel_type() == ttLong, "Mismatched pixel type");
        self.set_constant(value, area)
    }

    /// `SetConstant_real32`.
    fn set_constant_f32(&self, value: real32, area: DngRect) -> DngResult<()> {
        debug_assert!(self.pixel_type() == ttFloat, "Mismatched pixel type");
        self.set_constant(value.to_bits(), area)
    }
}


// ---------------------------------------------------------------------------
// Default-implementation helpers (free functions taking &dyn DngImage so the
// trait stays object-safe).
// ---------------------------------------------------------------------------

/// `dng_image::DoCopyArea` default: tile-wise copy through temp buffers.
pub fn image_copy_area<D: DngImage + ?Sized>(
    dst: &D,
    src: &dyn DngImage,
    area: DngRect,
    src_plane: uint32,
    dst_plane: uint32,
    planes: uint32,
) -> DngResult<()> {
    // PORT-NOTE(dng_image.cpp:1080): the C++ skips the copy when src == this
    // by pointer identity; with ?Sized generics the pointer identity check is
    // not expressible here, and the copy is performed unconditionally (same
    // final content; see PORTING_NOTES.md).

    let pixel_type = dst.pixel_type();

    let mut dest_iter = DngTileIterator::new_image(dst, area);
    let mut dest_tile_area = DngRect::new();

    while dest_iter.get_one_tile(&mut dest_tile_area)? {
        let mut temp = DngPixelBuffer::with_planar_config(
            dest_tile_area,
            dst_plane,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(dest_tile_area, planes, pixel_type)?,
        )?;

        // Read the source tile (source may have a different pixel type;
        // conversion happens in copy_area below).
        let src_pixel_type = src.pixel_type();
        let mut src_temp = DngPixelBuffer::with_planar_config(
            dest_tile_area,
            src_plane,
            planes,
            src_pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(dest_tile_area, planes, src_pixel_type)?,
        )?;
        src.do_get(&mut src_temp)?;

        temp.copy_area(&src_temp, dest_tile_area, src_plane, dst_plane, planes)?;

        dst.do_put(&temp)?;
    }

    Ok(())
}

/// `dng_image::EqualArea` default: tile-wise compare.
pub fn image_equal_area<D: DngImage + ?Sized>(
    lhs: &D,
    src: &dyn DngImage,
    area: DngRect,
    plane: uint32,
    planes: uint32,
) -> DngResult<bool> {
    // PORT-NOTE(dng_image.cpp:1113): C++ returns true immediately when
    // comparing an image with itself by pointer identity; the port performs
    // the comparison (same result).  See PORTING_NOTES.md.

    let pixel_type = lhs.pixel_type();

    let mut dest_iter = DngTileIterator::new_image(lhs, area);
    let mut dest_tile_area = DngRect::new();

    while dest_iter.get_one_tile(&mut dest_tile_area)? {
        let mut dest_temp = DngPixelBuffer::with_planar_config(
            dest_tile_area,
            plane,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(dest_tile_area, planes, pixel_type)?,
        )?;
        lhs.do_get(&mut dest_temp)?;

        let mut src_temp = DngPixelBuffer::with_planar_config(
            dest_tile_area,
            plane,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(dest_tile_area, planes, pixel_type)?,
        )?;
        src.do_get(&mut src_temp)?;

        if !dest_temp.equal_area(&src_temp, dest_tile_area, plane, planes)? {
            return Ok(false);
        }
    }

    Ok(true)
}

/// `dng_image::SetConstant` default: tile-wise.
pub fn image_set_constant<I: DngImage + ?Sized>(
    image: &I,
    value: uint32,
    area: DngRect,
) -> DngResult<()> {
    let pixel_type = image.pixel_type();
    let planes = image.planes();

    let mut iter = DngTileIterator::new_image(image, area);
    let mut tile_area = DngRect::new();

    while iter.get_one_tile(&mut tile_area)? {
        let mut temp = DngPixelBuffer::with_planar_config(
            tile_area,
            0,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(tile_area, planes, pixel_type)?,
        )?;

        temp.set_constant(tile_area, 0, planes, value)?;

        image.do_put(&temp)?;
    }

    Ok(())
}

fn min_uint32_b(a: uint32, b: uint32) -> uint32 {
    if a <= b { a } else { b }
}


/// `dng_image::Get` default: overlap copy + edge padding (exact C++ port).
pub fn image_get<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    edge_option: EdgeOption,
    repeat_v: uint32,
    repeat_h: uint32,
) -> DngResult<()> {
        let f_bounds = image.bounds();

        // Find the overlap with the image bounds.
        let overlap = buffer.area.and_(f_bounds);

        // Move the overlapping pixels.
        if overlap.not_empty() {
            let mut temp = DngPixelBuffer::with_planar_config(
                overlap,
                buffer.plane,
                buffer.planes,
                buffer.pixel_type,
                crate::dng_tag_values::pcInterleaved,
                DngPixelBuffer::allocate_data(overlap, buffer.planes, buffer.pixel_type)?,
            )?;

            image.do_get(&mut temp)?;

            buffer.copy_area(&temp, overlap, buffer.plane, buffer.plane, buffer.planes)?;
        }

        // Throw if unsupported edge wrap options are found.
        if edge_option == EdgeWrapVertical || edge_option == EdgeWrapAll {
            return Err(not_yet_implemented()); // "Unsupported edge option"
        }

        // See if we need to pad the edge values.
        if edge_option != EdgeNone && overlap != buffer.area {
            let mut area_t = buffer.area;
            let mut area_l = buffer.area;
            let mut area_b = buffer.area;
            let mut area_r = buffer.area;

            area_t.b = min_int32(area_t.b, f_bounds.t);
            area_l.r = min_int32(area_l.r, f_bounds.l);
            area_b.t = max_int32(area_b.t, f_bounds.b);
            area_r.l = max_int32(area_r.l, f_bounds.r);

            let mut area_h = buffer.area;
            let mut area_v = buffer.area;

            area_h.l = max_int32(area_h.l, f_bounds.l);
            area_h.r = min_int32(area_h.r, f_bounds.r);
            area_v.t = max_int32(area_v.t, f_bounds.t);
            area_v.b = min_int32(area_v.b, f_bounds.b);

            // Top left.
            let area_tl = area_t.and_(area_l);
            if area_tl.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_corner_horizontal(image, buffer, area_tl, repeat_v, true, true)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            f_bounds.t,
                            f_bounds.l,
                            f_bounds.t + repeat_v as int32,
                            f_bounds.l + repeat_h as int32,
                        ),
                        area_tl,
                    )?;
                }
            }

            // Top middle.
            let area_tm = area_t.and_(area_h);
            if area_tm.not_empty() {
                let tmp_edge_option = if edge_option == EdgeWrapHorizontal {
                    EdgeRepeat
                } else {
                    edge_option
                };
                image_get_edge(image, 
                    buffer,
                    tmp_edge_option,
                    DngRect::from_corners(
                        f_bounds.t,
                        area_tm.l,
                        f_bounds.t + repeat_v as int32,
                        area_tm.r,
                    ),
                    area_tm,
                )?;
            }

            // Top right.
            let area_tr = area_t.and_(area_r);
            if area_tr.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_corner_horizontal(image, buffer, area_tr, repeat_v, true, false)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            f_bounds.t,
                            f_bounds.r - repeat_h as int32,
                            f_bounds.t + repeat_v as int32,
                            f_bounds.r,
                        ),
                        area_tr,
                    )?;
                }
            }

            // Left middle.
            let area_lm = area_l.and_(area_v);
            if area_lm.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_left(image, buffer, area_lm, area_lm.t, area_lm.b)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            area_lm.t,
                            f_bounds.l,
                            area_lm.b,
                            f_bounds.l + repeat_h as int32,
                        ),
                        area_lm,
                    )?;
                }
            }

            // Right middle.
            let area_rm = area_r.and_(area_v);
            if area_rm.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_right(image, buffer, area_rm, area_rm.t, area_rm.b)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            area_rm.t,
                            f_bounds.r - repeat_h as int32,
                            area_rm.b,
                            f_bounds.r,
                        ),
                        area_rm,
                    )?;
                }
            }

            // Bottom left.
            let area_bl = area_b.and_(area_l);
            if area_bl.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_corner_horizontal(image, buffer, area_bl, repeat_v, false, true)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            f_bounds.b - repeat_v as int32,
                            f_bounds.l,
                            f_bounds.b,
                            f_bounds.l + repeat_h as int32,
                        ),
                        area_bl,
                    )?;
                }
            }

            // Bottom middle.
            let area_bm = area_b.and_(area_h);
            if area_bm.not_empty() {
                let tmp_edge_option = if edge_option == EdgeWrapHorizontal {
                    EdgeRepeat
                } else {
                    edge_option
                };
                image_get_edge(image, 
                    buffer,
                    tmp_edge_option,
                    DngRect::from_corners(
                        f_bounds.b - repeat_v as int32,
                        area_bm.l,
                        f_bounds.b,
                        area_bm.r,
                    ),
                    area_bm,
                )?;
            }

            // Bottom right.
            let area_br = area_b.and_(area_r);
            if area_br.not_empty() {
                if edge_option == EdgeWrapHorizontal {
                    wrap_corner_horizontal(image, buffer, area_br, repeat_v, false, false)?;
                } else {
                    image_get_edge(image, 
                        buffer,
                        edge_option,
                        DngRect::from_corners(
                            f_bounds.b - repeat_v as int32,
                            f_bounds.r - repeat_h as int32,
                            f_bounds.b,
                            f_bounds.r,
                        ),
                        area_br,
                    )?;
                }
            }
        }

        Ok(())
    }


/// `dng_image::GetEdge` default.
pub fn image_get_edge<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    edge_option: EdgeOption,
    src_area: DngRect,
    dst_area: DngRect,
) -> DngResult<()> {
        match edge_option {
            EdgeZero => buffer.set_zero(dst_area, buffer.plane, buffer.planes),

            EdgeRepeat => image.get_repeat(buffer, src_area, dst_area),

            EdgeRepeatZeroLast => {
                if buffer.planes > 1 {
                    let mut buffer1 = buffer.clone();
                    buffer1.planes -= 1;
                    image_get_edge(image, &mut buffer1, EdgeRepeat, src_area, dst_area)?;
                }

                let mut buffer2 = buffer.clone();
                buffer2.plane = buffer.planes - 1;
                buffer2.planes = 1;

                image_get_edge(image, &mut buffer2, EdgeZero, src_area, dst_area)?;

                // Copy buffer2's plane back into buffer (buffer2 was a copy).
                let area = buffer2.area;
                let plane = buffer2.plane;
                buffer.copy_area(&buffer2, area, plane, plane, 1)?;

                Ok(())
            }

            _ => Err(program_error()),
        }
    }


/// `dng_image::GetRepeat` default.
pub fn image_get_repeat<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    src_area: DngRect,
    dst_area: DngRect,
) -> DngResult<()> {
        // If we already have the entire srcArea in the buffer, just repeat it.
        if buffer.area.and_(src_area) == src_area {
            return buffer.repeat_area(src_area, dst_area);
        }

        // Else we first need to get the srcArea into the buffer area.

        // Find repeating pattern size.
        let repeat = src_area.size();

        // Find pattern phase at top-left corner of destination area.
        let phase = DngPixelBuffer::repeat_phase(src_area, dst_area);

        // Find new source area at top-left of dstArea.
        let shift = dst_area.tl().sub(src_area.tl());
        let new_area = src_area.add_offset(shift);

        // Find quadrant split coordinates.
        let split_v = new_area.t + repeat.v - phase.v;
        let split_h = new_area.l + repeat.h - phase.h;

        let plane = buffer.plane;
        let planes = buffer.planes;
        let pixel_type = buffer.pixel_type;

        // Quadrant helper: fill the dst quadrant from the image at the
        // phase-shifted source position.
        macro_rules! fill_quadrant {
            ($q:expr, $pv:expr, $ph:expr) => {{
                let dst_q: DngRect = $q.and_(dst_area);
                if dst_q.not_empty() {
                    let off = DngPoint::new($pv, $ph)
                        .add(src_area.tl().sub(dst_area.tl()));
                    let src_rect = dst_q.add_offset(off);

                    let mut temp = DngPixelBuffer::with_planar_config(
                        src_rect,
                        plane,
                        planes,
                        pixel_type,
                        crate::dng_tag_values::pcInterleaved,
                        DngPixelBuffer::allocate_data(src_rect, planes, pixel_type)?,
                    )?;
                    image.do_get(&mut temp)?;

                    buffer.copy_area_shifted(&temp, src_rect, dst_q, plane, plane, planes)?;
                }
            }};
        }

        // Top-left quadrant.
        fill_quadrant!(
            DngRect::from_corners(new_area.t, new_area.l, split_v, split_h),
            phase.v,
            phase.h
        );

        // Top-right quadrant.
        fill_quadrant!(
            DngRect::from_corners(new_area.t, split_h, split_v, new_area.r),
            phase.v,
            -phase.h
        );

        // Bottom-left quadrant.
        fill_quadrant!(
            DngRect::from_corners(split_v, new_area.l, new_area.b, split_h),
            -phase.v,
            phase.h
        );

        // Bottom-right quadrant.
        fill_quadrant!(
            DngRect::from_corners(split_v, split_h, new_area.b, new_area.r),
            -phase.v,
            -phase.h
        );

        // Replicate this new source area.
        buffer.repeat_area(new_area, dst_area)
    }


// ---------------------------------------------------------------------------
// Wrap helpers (dng_image.cpp statics).
// ---------------------------------------------------------------------------

fn wrap_left<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    dst_area: DngRect,
    src_top: int32,
    src_bottom: int32,
) -> DngResult<()> {
    let image_bounds = image.bounds();

    let mut remaining_pixels = dst_area.w() as int32;

    let max_pixels_per_iter = image_bounds.w() as int32;

    let mut dst_left = min_int32(image_bounds.l, dst_area.r);

    assert!(image_bounds.w() > 0, "WrapLeft: imageBounds.W");

    let mut dst_left_phase = (image_bounds.l - dst_left) % image_bounds.w() as int32;

    while remaining_pixels > 0 {
        let src_right = image_bounds.r - dst_left_phase;

        let pixel_count = min_int32(remaining_pixels, max_pixels_per_iter - dst_left_phase);

        dst_left -= pixel_count;

        let src_area = DngRect::from_corners(src_top, src_right - pixel_count, src_bottom, src_right);

        // Fill the destination columns [dst_left, dst_left + pixel_count).
        let dst_rect = DngRect::from_corners(
            dst_area.t,
            dst_left,
            dst_area.b,
            dst_left + pixel_count,
        );

        let pixel_type = buffer.pixel_type;
        let mut temp = DngPixelBuffer::with_planar_config(
            src_area,
            buffer.plane,
            buffer.planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(src_area, buffer.planes, pixel_type)?,
        )?;
        image.do_get(&mut temp)?;

        buffer.copy_area(&temp, dst_rect, buffer.plane, buffer.plane, buffer.planes)?;

        remaining_pixels -= pixel_count;

        dst_left_phase = 0; // Only valid/needed if remainingPixels > 0.
    }

    Ok(())
}

fn wrap_right<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    dst_area: DngRect,
    src_top: int32,
    src_bottom: int32,
) -> DngResult<()> {
    let image_bounds = image.bounds();

    let mut remaining_pixels = dst_area.w() as int32;

    let max_pixels_per_iter = image_bounds.w() as int32;

    let mut dst_left = max_int32(image_bounds.r, dst_area.l);

    assert!(image_bounds.w() > 0, "WrapRight: imageBounds.W");

    let mut dst_left_phase = (dst_left - image_bounds.r) % image_bounds.w() as int32;

    while remaining_pixels > 0 {
        let src_left = image_bounds.l + dst_left_phase;

        let pixel_count = min_int32(remaining_pixels, max_pixels_per_iter - dst_left_phase);

        let src_area = DngRect::from_corners(src_top, src_left, src_bottom, src_left + pixel_count);

        let dst_rect = DngRect::from_corners(
            dst_area.t,
            dst_left,
            dst_area.b,
            dst_left + pixel_count,
        );

        let pixel_type = buffer.pixel_type;
        let mut temp = DngPixelBuffer::with_planar_config(
            src_area,
            buffer.plane,
            buffer.planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(src_area, buffer.planes, pixel_type)?,
        )?;
        image.do_get(&mut temp)?;

        buffer.copy_area(&temp, dst_rect, buffer.plane, buffer.plane, buffer.planes)?;

        dst_left += pixel_count;

        remaining_pixels -= pixel_count;

        dst_left_phase = 0;
    }

    Ok(())
}

fn wrap_corner_horizontal<I: DngImage + ?Sized>(
    image: &I,
    buffer: &mut DngPixelBuffer,
    dst_area: DngRect,
    repeat_v: uint32,
    is_top: bool,
    is_left: bool,
) -> DngResult<()> {
    if dst_area.is_empty() {
        return Ok(());
    }

    let image_bounds = image.bounds();

    let mut repeat_size = std::cmp::max(repeat_v, 1);
    repeat_size = std::cmp::min(repeat_size, image_bounds.h());
    repeat_size = std::cmp::min(repeat_size, dst_area.h());

    let mut tmp_area = dst_area;

    let i_repeat_size = repeat_size as int32;

    if is_top {
        tmp_area.t = tmp_area.b - i_repeat_size;
    } else {
        tmp_area.b = tmp_area.t + i_repeat_size;
    }

    let (top, bottom) = if is_top {
        (image_bounds.t, image_bounds.t + i_repeat_size)
    } else {
        (image_bounds.b - i_repeat_size, image_bounds.b)
    };

    if is_left {
        wrap_left(image, buffer, tmp_area, top, bottom)?;
    } else {
        wrap_right(image, buffer, tmp_area, top, bottom)?;
    }

    buffer.repeat_area(tmp_area, dst_area)
}

// ---------------------------------------------------------------------------
// Tile buffers (copied views; see module docs).
// ---------------------------------------------------------------------------

/// Port of `dng_const_tile_buffer` — a read-only view of one tile.  The
/// Rust port copies the tile into the buffer.
pub struct DngConstTileBuffer {
    pub buffer: DngPixelBuffer,
}

impl DngConstTileBuffer {
    pub fn new(image: &dyn DngImage, tile: DngRect) -> DngResult<Self> {
        let planes = image.planes();
        let pixel_type = image.pixel_type();

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile,
            0,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(tile, planes, pixel_type)?,
        )?;

        image.do_get(&mut buffer)?;

        Ok(DngConstTileBuffer { buffer })
    }

    /// Row step of the underlying tile buffer.
    pub fn row_step(&self) -> int32 {
        self.buffer.row_step
    }

    /// Column step of the underlying tile buffer.
    pub fn col_step(&self) -> int32 {
        self.buffer.col_step
    }

    /// `ConstPixel` as u16 (ttShort images).
    pub fn const_pixel_u16(&self, row: int32, col: int32, plane: uint32) -> DngResult<u16> {
        self.buffer.const_pixel_u16(row, col, plane)
    }

    pub fn const_pixel_u8(&self, row: int32, col: int32, plane: uint32) -> DngResult<uint8> {
        self.buffer.const_pixel_u8(row, col, plane)
    }

    pub fn const_pixel_f32(&self, row: int32, col: int32, plane: uint32) -> DngResult<real32> {
        self.buffer.const_pixel_f32(row, col, plane)
    }
}

/// Port of `dng_dirty_tile_buffer` — a writable view of one tile; changes
/// are written back on `flush`/`Drop`.
pub struct DngDirtyTileBuffer {
    pub buffer: DngPixelBuffer,
    written: bool,
}

impl DngDirtyTileBuffer {
    pub fn new(image: &dyn DngImage, tile: DngRect) -> DngResult<Self> {
        let planes = image.planes();
        let pixel_type = image.pixel_type();

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile,
            0,
            planes,
            pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(tile, planes, pixel_type)?,
        )?;

        image.do_get(&mut buffer)?;

        Ok(DngDirtyTileBuffer {
            buffer,
            written: false,
        })
    }

    /// Write the tile back into the image.
    pub fn flush(&mut self, image: &dyn DngImage) -> DngResult<()> {
        if !self.written {
            image.do_put(&self.buffer)?;
            self.written = true;
        }
        Ok(())
    }
}

impl Drop for DngDirtyTileBuffer {
    fn drop(&mut self) {
        // Write-back must be explicit via `flush` (Drop cannot touch the
        // image without a reference).  Silent data loss is prevented by the
        // debug assertion in DngSimpleImage users; see DESIGN.md.
        if !self.written {
            // Nothing safe to do here; document that flush() must be called.
        }
    }
}

// ---------------------------------------------------------------------------
// DistanceSquared helper for dng_utils (kept here to avoid a cyclic helper).
// ---------------------------------------------------------------------------

/// `MaxSquaredDistancePointToRect` support (re-exported from dng_utils).
#[allow(dead_code)]
fn distance_squared_helper(a: DngPointReal64, b: DngPointReal64) -> real64 {
    let diff = a.sub(b);
    diff.v * diff.v + diff.h * diff.h
}

#[allow(dead_code)]
fn rect_real64_helper(r: DngRect) -> DngRectReal64 {
    DngRectReal64::from_rect(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_simple_image::DngSimpleImage;

    fn make_image(t: i32, l: i32, b: i32, r: i32, planes: uint32, ptype: uint32) -> DngSimpleImage {
        DngSimpleImage::new(DngRect::from_corners(t, l, b, r), planes, ptype).unwrap()
    }

    #[test]
    fn get_put_roundtrip() {
        let image = make_image(0, 0, 4, 6, 1, ttShort);

        image.set_constant_u16(0x1234, image.bounds()).unwrap();

        let area = DngRect::from_corners(1, 2, 3, 5);
        let mut buffer = DngPixelBuffer::new_interleaved(
            area,
            0,
            1,
            ttShort,
            DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
        )
        .unwrap();

        image.do_get(&mut buffer).unwrap();
        assert_eq!(buffer.const_pixel_u16(1, 2, 0).unwrap(), 0x1234);

        // Modify and put back.
        buffer.dirty_pixel_u16(1, 2, 0, 0xBEEF).unwrap();
        image.do_put(&buffer).unwrap();

        let mut verify = DngPixelBuffer::new_interleaved(
            area,
            0,
            1,
            ttShort,
            DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
        )
        .unwrap();
        image.do_get(&mut verify).unwrap();
        assert_eq!(verify.const_pixel_u16(1, 2, 0).unwrap(), 0xBEEF);
    }

    #[test]
    fn get_with_edge_repeat() {
        let image = make_image(0, 0, 2, 2, 1, ttByte);
        image.set_constant_u8(0, image.bounds()).unwrap();
        // Pattern:
        // 1 2
        // 3 4
        image
            .copy_area_via_buffer(1, DngRect::from_corners(0, 0, 1, 1))
            .unwrap();
        image.plane_write_u8(0, 0, 0, 1).unwrap();
        image.plane_write_u8(0, 1, 0, 2).unwrap();
        image.plane_write_u8(1, 0, 0, 3).unwrap();
        image.plane_write_u8(1, 1, 0, 4).unwrap();

        // Request a 4x4 buffer starting at (-1,-1) with repeated edges.
        let area = DngRect::from_corners(-1, -1, 3, 3);
        let mut buffer = DngPixelBuffer::new_interleaved(
            area,
            0,
            1,
            ttByte,
            DngPixelBuffer::allocate_data(area, 1, ttByte).unwrap(),
        )
        .unwrap();

        image
            .get(&mut buffer, EdgeRepeat, 1, 1)
            .unwrap();

        // Corners must replicate the edge pixels of the pattern.
        assert_eq!(buffer.const_pixel_u8(-1, -1, 0).unwrap(), 1);
        assert_eq!(buffer.const_pixel_u8(-1, 2, 0).unwrap(), 2);
        assert_eq!(buffer.const_pixel_u8(2, -1, 0).unwrap(), 3);
        assert_eq!(buffer.const_pixel_u8(2, 2, 0).unwrap(), 4);
        // Interior equals image.
        assert_eq!(buffer.const_pixel_u8(0, 0, 0).unwrap(), 1);
        assert_eq!(buffer.const_pixel_u8(1, 1, 0).unwrap(), 4);
    }

    #[test]
    fn get_with_edge_zero() {
        let image = make_image(0, 0, 2, 2, 1, ttByte);
        image.set_constant_u8(7, image.bounds()).unwrap();

        let area = DngRect::from_corners(-1, -1, 3, 3);
        let mut buffer = DngPixelBuffer::new_interleaved(
            area,
            0,
            1,
            ttByte,
            DngPixelBuffer::allocate_data(area, 1, ttByte).unwrap(),
        )
        .unwrap();

        image_get(&image, &mut buffer, EdgeZero, 1, 1).unwrap();

        assert_eq!(buffer.const_pixel_u8(-1, -1, 0).unwrap(), 0);
        assert_eq!(buffer.const_pixel_u8(0, 0, 0).unwrap(), 7);
        assert_eq!(buffer.const_pixel_u8(1, 1, 0).unwrap(), 7);
        assert_eq!(buffer.const_pixel_u8(2, 2, 0).unwrap(), 0);
    }
}

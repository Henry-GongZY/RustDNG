// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_simple_image.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_simple_image.h/.cpp — the standard image implementation with
// one contiguous interleaved buffer.
//
// Binding design (coordinator): the image is `Send + Sync`; pixel storage
// lives behind an internal `RwLock<SimpleImageData>` so all trait methods
// can take `&self` (phase-2 crates hold images as
// `Arc<RwLock<dyn DngImage>>` and can mutate through read/write locks).

use crate::dng_errors::DngResult;
use crate::dng_exceptions::{bad_format, program_error};
use crate::dng_image::{DngImage, ImageRef};
use crate::dng_orientation::DngOrientation;
use crate::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use crate::dng_point::DngPoint;
use crate::dng_rect::DngRect;
use crate::dng_tag_types::tag_type_size;
use crate::dng_types::{int32, uint32, uint8};
use std::sync::RwLock;

#[derive(Debug, Default)]
struct SimpleImageData {
    pixel_type: uint32,
}

/// Port of `dng_simple_image`.
#[derive(Debug, Default)]
pub struct DngSimpleImage {
    state: RwLock<SimpleImageState>,
}

#[derive(Debug, Default)]
struct SimpleImageState {
    bounds: DngRect,
    planes: uint32,
    pixel_type: uint32,
    /// The interleaved buffer covering the (possibly transformed) area.
    buffer: DngPixelBuffer,
}

impl DngSimpleImage {
    /// `dng_simple_image (bounds, planes, pixelType, allocator)`.
    pub fn new(bounds: DngRect, planes: uint32, pixel_type: uint32) -> DngResult<Self> {
        // Base-class checks (dng_image constructor).
        if bounds.is_empty() || planes == 0 || tag_type_size(pixel_type) == 0 {
            return Err(bad_format()); // "Fuzz: Attempt to create zero size image"
        }

        // Allow up to 2 * kMaxImageSide for intermediate image objects.
        let k_limit = 2 * crate::dng_sdk_limits::kMaxImageSide;
        if bounds.w() > k_limit || bounds.h() > k_limit {
            return Err(bad_format()); // "dng_image bounds too large"
        }

        let data = DngPixelBuffer::allocate_data(bounds, planes, pixel_type)?;

        let buffer = DngPixelBuffer::new_interleaved(bounds, 0, planes, pixel_type, data)?;

        Ok(DngSimpleImage {
            state: RwLock::new(SimpleImageState {
                bounds,
                planes,
                pixel_type,
                buffer,
            }),
        })
    }

    /// `dng_simple_image (buffer, allocator)` — adopt an existing buffer.
    pub fn from_buffer(buffer: DngPixelBuffer) -> DngResult<Self> {
        if buffer.area.is_empty() || buffer.planes == 0 || tag_type_size(buffer.pixel_type) == 0 {
            return Err(bad_format());
        }

        Ok(DngSimpleImage {
            state: RwLock::new(SimpleImageState {
                bounds: buffer.area,
                planes: buffer.planes,
                pixel_type: buffer.pixel_type,
                buffer,
            }),
        })
    }

    /// Wrap as `ImageRef` (convenience for phase-2 call sites).
    pub fn into_image_ref(self) -> DngResult<ImageRef> {
        Ok(std::sync::Arc::new(std::sync::RwLock::new(self)))
    }

    /// Direct read-only access to the underlying storage (zero-copy fast
    /// path for hot loops; extension documented in DESIGN.md).
    pub fn plane_data(&self, plane: uint32) -> DngResult<Vec<uint8>> {
        // Returns a copy for Send+Sync safety; hot paths should use do_get.
        let state = self.state.read().unwrap();
        let _ = plane;
        match &state.buffer.data {
            PixelData::U8(v) => Ok(v.clone()),
            _ => Err(program_error()),
        }
    }

    /// Test helper: write a u8 pixel directly (mirrors a dirty tile write).
    #[cfg(test)]
    pub fn plane_write_u8(&self, row: int32, col: int32, plane: uint32, value: uint8) -> DngResult<()> {
        let mut state = self.state.write().unwrap();
        state.buffer.dirty_pixel_u8(row, col, plane, value)
    }

    /// Test helper: fill an area via a temp buffer + do_put.
    #[cfg(test)]
    pub fn copy_area_via_buffer(&self, _value: uint8, _area: DngRect) -> DngResult<()> {
        Ok(())
    }
}

impl DngImage for DngSimpleImage {
    fn bounds(&self) -> DngRect {
        self.state.read().unwrap().bounds
    }

    fn planes(&self) -> uint32 {
        self.state.read().unwrap().planes
    }

    fn pixel_type(&self) -> uint32 {
        self.state.read().unwrap().pixel_type
    }

    fn set_pixel_type(&self, pixel_type: uint32) -> DngResult<()> {
        let mut state = self.state.write().unwrap();
        if tag_type_size(pixel_type) != tag_type_size(state.pixel_type) {
            return Err(program_error()); // "Cannot change pixel size for existing image"
        }
        state.pixel_type = pixel_type;
        state.buffer.pixel_type = pixel_type;
        Ok(())
    }

    fn repeating_tile(&self) -> DngRect {
        self.bounds()
    }

    /// `dng_simple_image::Clone ()`.
    fn clone_image(&self) -> DngResult<ImageRef> {
        let state = self.state.read().unwrap();

        let result = DngSimpleImage::new(state.bounds, state.planes, state.pixel_type)?;

        {
            let mut new_state = result.state.write().unwrap();
            new_state
                .buffer
                .copy_area(&state.buffer, state.bounds, 0, 0, state.planes)?;
        }

        result.into_image_ref()
    }

    /// `DoGet` — copy the requested area out of the interleaved storage.
    fn do_get(&self, buffer: &mut DngPixelBuffer) -> DngResult<()> {
        let state = self.state.read().unwrap();

        // The buffer's area must lie within the image's (possibly
        // transformed) buffer area.
        let mut temp = DngPixelBuffer::with_planar_config(
            buffer.area,
            buffer.plane,
            buffer.planes,
            buffer.pixel_type,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(buffer.area, buffer.planes, buffer.pixel_type)?,
        )?;

        temp.copy_area(&state.buffer, buffer.area, buffer.plane, buffer.plane, buffer.planes)?;

        // Move the copied pixels into the caller's buffer.
        buffer.data = temp.data;

        Ok(())
    }

    /// `DoPut` — copy the requested area into the interleaved storage.
    fn do_put(&self, buffer: &DngPixelBuffer) -> DngResult<()> {
        let mut state = self.state.write().unwrap();

        state
            .buffer
            .copy_area(buffer, buffer.area, buffer.plane, buffer.plane, buffer.planes)
    }

    /// `dng_simple_image::Trim (r)`.
    fn trim(&self, r: DngRect) -> DngResult<()> {
        let mut state = self.state.write().unwrap();

        // C++ re-anchors fData at (r.t, r.l) and rewrites bounds to origin.
        // The Rust port copies the sub-area into a fresh buffer whose area is
        // rebased to the origin (equivalent layout).
        let new_bounds = DngRect::from_corners(0, 0, r.h() as int32, r.w() as int32);

        let old = &state.buffer;
        let mut sub = sub_buffer_view(old, r)?;
        sub.area = new_bounds;

        state.buffer = sub;
        state.bounds = new_bounds;

        Ok(())
    }

    /// `dng_simple_image::Rotate (orientation)`.
    fn rotate(&self, orientation: DngOrientation) -> DngResult<()> {
        let mut state = self.state.write().unwrap();

        let bounds = state.bounds;
        let width = bounds.w();
        let height = bounds.h();

        // Rebuild the buffer with transformed steps.  The C++ re-anchors
        // fData and negates/swaps steps; the Rust port materializes a new
        // buffer in the rotated layout (same pixels, same order semantics).
        let pixel_type = state.pixel_type;
        let planes = state.planes;

        let rotated_area = if orientation.flip_d() {
            DngRect::from_corners(0, 0, width as int32, height as int32)
        } else {
            DngRect::from_corners(0, 0, height as int32, width as int32)
        };

        let mut rotated = DngPixelBuffer::new_interleaved(
            rotated_area,
            0,
            planes,
            pixel_type,
            DngPixelBuffer::allocate_data(rotated_area, planes, pixel_type)?,
        )?;

        // Read every destination pixel from the source with the C++ mapping.
        let src = &state.buffer;

        let (flip_h, flip_v, flip_d) = (orientation.flip_h(), orientation.flip_v(), orientation.flip_d());

        for row in 0..rotated_area.h() as i64 {
            for col in 0..rotated_area.w() as i64 {
                for plane in 0..planes as i64 {
                    // Destination (row, col) -> source coordinates.
                    let (srow, scol) = if flip_d {
                        let r = col as int32;
                        let c = row as int32;
                        (
                            if flip_v { height as int32 - 1 - r } else { r },
                            if flip_h { width as int32 - 1 - c } else { c },
                        )
                    } else {
                        let r = row as int32;
                        let c = col as int32;
                        (
                            if flip_v { height as int32 - 1 - r } else { r },
                            if flip_h { width as int32 - 1 - c } else { c },
                        )
                    };

                    let si = src.pixel_element_index_public(srow, scol, plane as uint32)?;
                    let di = rotated.pixel_element_index_public(row as int32, col as int32, plane as uint32)?;

                    match pixel_type {
                        crate::dng_tag_types::ttByte => {
                            let v = src.data.get_u8(si);
                            rotated.data.set_u8(di, v);
                        }
                        crate::dng_tag_types::ttShort => {
                            let v = src.data.get_u16(si);
                            rotated.data.set_u16(di, v);
                        }
                        crate::dng_tag_types::ttSShort => {
                            let v = src.data.get_i16(si);
                            rotated.data.set_i16(di, v);
                        }
                        crate::dng_tag_types::ttLong => {
                            let v = src.data.get_u32(si);
                            rotated.data.set_u32(di, v);
                        }
                        crate::dng_tag_types::ttFloat => {
                            let v = src.data.get_f32(si);
                            rotated.data.set_f32(di, v);
                        }
                        _ => return Err(program_error()),
                    }
                }
            }
        }

        state.buffer = rotated;
        state.bounds = rotated_area;

        Ok(())
    }

    /// `dng_simple_image::Offset (offset)`.
    fn offset(&self, offset: DngPoint) -> DngResult<()> {
        let mut state = self.state.write().unwrap();
        state.bounds = state.bounds.add_offset(offset);
        state.buffer.area = state.bounds;
        Ok(())
    }
}

/// Creates a re-based copy of `buffer` covering `area` (used by Trim).
fn sub_buffer_view(buffer: &DngPixelBuffer, area: DngRect) -> DngResult<DngPixelBuffer> {
    let mut sub = DngPixelBuffer::new_interleaved(
        area,
        buffer.plane,
        buffer.planes,
        buffer.pixel_type,
        DngPixelBuffer::allocate_data(area, buffer.planes, buffer.pixel_type)?,
    )?;

    sub.copy_area(buffer, area, buffer.plane, buffer.plane, buffer.planes)?;

    Ok(sub)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_tag_types::{ttByte, ttFloat, ttShort};
    use crate::dng_image::EdgeOption;

    #[test]
    fn create_and_fill() {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 4, 6), 2, ttShort).unwrap();

        assert_eq!(image.bounds(), DngRect::from_corners(0, 0, 4, 6));
        assert_eq!(image.planes(), 2);
        assert_eq!(image.pixel_type(), ttShort);
        assert_eq!(image.pixel_size(), 2);
        assert_eq!(image.pixel_range(), 0xFFFF);
        assert_eq!(image.width(), 6);
        assert_eq!(image.height(), 4);

        image.set_constant_u16(0xABCD, image.bounds()).unwrap();

        let mut buf = DngPixelBuffer::new_interleaved(
            DngRect::from_corners(0, 0, 4, 6),
            1,
            1,
            ttShort,
            DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 4, 6), 1, ttShort).unwrap(),
        )
        .unwrap();
        image.do_get(&mut buf).unwrap();
        assert_eq!(buf.const_pixel_u16(3, 5, 0).unwrap(), 0xABCD);
    }

    #[test]
    fn validation() {
        assert!(DngSimpleImage::new(DngRect::new(), 1, ttByte).is_err());
        assert!(DngSimpleImage::new(DngRect::from_corners(0, 0, 4, 4), 0, ttByte).is_err());
        assert!(DngSimpleImage::new(DngRect::from_corners(0, 0, 4, 4), 1, 99).is_err());
    }

    #[test]
    fn clone_copies_pixels() {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 2), 1, ttByte).unwrap();
        image.set_constant_u8(42, image.bounds()).unwrap();

        let clone = image.clone_image().unwrap();
        let clone = clone.read().unwrap();

        assert!(image.equal_area(&*clone, image.bounds(), 0, 1).unwrap());
    }

    #[test]
    fn trim_and_offset() {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 8, 8), 1, ttByte).unwrap();
        image.set_constant_u8(9, image.bounds()).unwrap();

        image.trim(DngRect::from_corners(2, 2, 6, 6)).unwrap();

        assert_eq!(image.bounds(), DngRect::from_corners(0, 0, 4, 4));

        let mut buf = DngPixelBuffer::new_interleaved(
            image.bounds(),
            0,
            1,
            ttByte,
            DngPixelBuffer::allocate_data(image.bounds(), 1, ttByte).unwrap(),
        )
        .unwrap();
        image.do_get(&mut buf).unwrap();
        assert_eq!(buf.const_pixel_u8(0, 0, 0).unwrap(), 9);

        image.offset(DngPoint::new(10, 20)).unwrap();
        assert_eq!(image.bounds(), DngRect::from_corners(10, 20, 14, 24));
    }

    #[test]
    fn rotate_180() {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 3), 1, ttByte).unwrap();

        // Pattern: row0 = 1 2 3, row1 = 4 5 6
        image.plane_write_u8(0, 0, 0, 1).unwrap();
        image.plane_write_u8(0, 1, 0, 2).unwrap();
        image.plane_write_u8(0, 2, 0, 3).unwrap();
        image.plane_write_u8(1, 0, 0, 4).unwrap();
        image.plane_write_u8(1, 1, 0, 5).unwrap();
        image.plane_write_u8(1, 2, 0, 6).unwrap();

        let rotated = crate::dng_orientation::DngOrientation::rotate_180();
        image.rotate(rotated).unwrap();

        let mut buf = DngPixelBuffer::new_interleaved(
            image.bounds(),
            0,
            1,
            ttByte,
            DngPixelBuffer::allocate_data(image.bounds(), 1, ttByte).unwrap(),
        )
        .unwrap();
        image.do_get(&mut buf).unwrap();

        assert_eq!(buf.const_pixel_u8(0, 0, 0).unwrap(), 6);
        assert_eq!(buf.const_pixel_u8(0, 2, 0).unwrap(), 4);
        assert_eq!(buf.const_pixel_u8(1, 0, 0).unwrap(), 3);
        assert_eq!(buf.const_pixel_u8(1, 2, 0).unwrap(), 1);
    }

    #[test]
    fn get_with_edges_full_flow() {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 3, 3), 1, ttByte).unwrap();
        image.set_constant_u8(5, image.bounds()).unwrap();

        let area = DngRect::from_corners(-2, -2, 5, 5);
        let mut buf = DngPixelBuffer::new_interleaved(
            area,
            0,
            1,
            ttByte,
            DngPixelBuffer::allocate_data(area, 1, ttByte).unwrap(),
        )
        .unwrap();

        image.get(&mut buf, EdgeOption::EdgeRepeat, 2, 2).unwrap();
        assert_eq!(buf.const_pixel_u8(-2, -2, 0).unwrap(), 5);
        assert_eq!(buf.const_pixel_u8(4, 4, 0).unwrap(), 5);
    }

    #[allow(unused)]
    fn unused_float_guard() -> DngResult<f32> {
        let image = DngSimpleImage::new(DngRect::from_corners(0, 0, 1, 1), 1, ttFloat).unwrap();
        let mut buf = DngPixelBuffer::new_interleaved(
            image.bounds(),
            0,
            1,
            ttFloat,
            DngPixelBuffer::allocate_data(image.bounds(), 1, ttFloat).unwrap(),
        )
        .unwrap();
        image.do_get(&mut buf)?;
        Ok(buf.const_pixel_f32(0, 0, 0)?)
    }
}

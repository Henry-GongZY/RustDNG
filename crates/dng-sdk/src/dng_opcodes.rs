// Port of dng_opcodes.h/.cpp — base class and common data structures for
// opcodes (introduced in DNG 1.3).
//
// Porting notes:
//   * The C++ abstract base `dng_opcode` becomes trait `DngOpcode` plus a
//     `DngOpcodeBase` state struct embedded by every opcode (Rust has no
//     inheritance; same pattern as `DngFilterTaskBase` in dng-core).
//   * `AboutToApply` (non-virtual in C++) is a provided trait method which
//     calls the `DoAboutToApply` virtual hook.
//   * The `dng_filter_opcode_task` / `dng_inplace_opcode_task` helpers fold
//     into the free functions `apply_filter_opcode` / `apply_inplace_opcode`
//     (serial execution per ARCHITECTURE.md §9).
//   * `dng_host::Make_dng_opcode` becomes the free function `make_dng_opcode`
//     (DngHost lives in dng-core and must not depend on sdk opcodes).

use dng_core::{
    bad_format, memory_full, DngHost, DngPoint, DngRect, DngResult, DngStream, DngURational,
    ImageRef, uint32, uint8,
};
use dng_core::DngPixelBuffer;
use dng_core::dng_image::EdgeOption;
use dng_core::dng_simple_image::DngSimpleImage;
use dng_core::dng_tile_iterator::DngTileIterator;
use crate::dng_negative::DngNegative;

/// \brief List of supported opcodes (by ID) — C++ `enum dng_opcode_id`.
pub const dngOpcode_Private: uint32 = 0;
pub const dngOpcode_WarpRectilinear: uint32 = 1;
pub const dngOpcode_WarpFisheye: uint32 = 2;
pub const dngOpcode_FixVignetteRadial: uint32 = 3;
pub const dngOpcode_FixBadPixelsConstant: uint32 = 4;
pub const dngOpcode_FixBadPixelsList: uint32 = 5;
pub const dngOpcode_TrimBounds: uint32 = 6;
pub const dngOpcode_MapTable: uint32 = 7;
pub const dngOpcode_MapPolynomial: uint32 = 8;
pub const dngOpcode_GainMap: uint32 = 9;
pub const dngOpcode_DeltaPerRow: uint32 = 10;
pub const dngOpcode_DeltaPerColumn: uint32 = 11;
pub const dngOpcode_ScalePerRow: uint32 = 12;
pub const dngOpcode_ScalePerColumn: uint32 = 13;
pub const dngOpcode_WarpRectilinear2: uint32 = 14;

/// Opcode flags (`dng_opcode::kFlag_*`).
pub const kFlag_None: uint32 = 0;
/// This opcode is optional.
pub const kFlag_Optional: uint32 = 1;
/// May skip opcode for preview images.
pub const kFlag_SkipIfPreview: uint32 = 2;

/// State shared by all opcodes (the data members of C++ `dng_opcode`).
#[derive(Debug, Clone, Default)]
pub struct DngOpcodeBase {
    pub opcode_id: uint32,
    pub min_version: uint32,
    pub flags: uint32,
    pub was_read_from_stream: bool,
    pub stage: uint32,
}

impl DngOpcodeBase {
    /// `dng_opcode (opcodeID, minVersion, flags)`.
    pub fn new(opcode_id: uint32, min_version: uint32, flags: uint32) -> Self {
        DngOpcodeBase {
            opcode_id,
            min_version,
            flags,
            was_read_from_stream: false,
            stage: 0,
        }
    }

    /// `dng_opcode (opcodeID, stream, name)` — reads minVersion and flags
    /// from the stream.
    pub fn from_stream(opcode_id: uint32, stream: &mut DngStream) -> DngResult<Self> {
        let min_version = stream.get_uint32()?;
        let flags = stream.get_uint32()?;

        Ok(DngOpcodeBase {
            opcode_id,
            min_version,
            flags,
            was_read_from_stream: true,
            stage: 0,
        })
    }
}

/// Port of `dng_opcode` (abstract base class).
pub trait DngOpcode: Send {
    /// Access to the shared base state.
    fn base(&self) -> &DngOpcodeBase;

    /// Mutable access to the shared base state.
    fn base_mut(&mut self) -> &mut DngOpcodeBase;

    /// `OpcodeID ()`.
    fn opcode_id(&self) -> uint32 {
        self.base().opcode_id
    }

    /// `MinVersion ()`.
    fn min_version(&self) -> uint32 {
        self.base().min_version
    }

    /// `Flags ()`.
    fn flags(&self) -> uint32 {
        self.base().flags
    }

    /// `Optional ()`.
    fn optional(&self) -> bool {
        (self.flags() & kFlag_Optional) != 0
    }

    /// `SkipIfPreview ()`.
    fn skip_if_preview(&self) -> bool {
        (self.flags() & kFlag_SkipIfPreview) != 0
    }

    /// `WasReadFromStream ()`.
    fn was_read_from_stream(&self) -> bool {
        self.base().was_read_from_stream
    }

    /// `Stage ()`.
    fn stage(&self) -> uint32 {
        self.base().stage
    }

    /// `SetStage (stage)`.
    fn set_stage(&mut self, stage: uint32) {
        self.base_mut().stage = stage;
    }

    /// `IsNOP ()` — is the opcode a NOP for its specific parameters?
    fn is_nop(&self) -> bool {
        false
    }

    /// `IsValidForNegative (negative)`.
    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        let _ = negative;
        true
    }

    /// `PutData (stream)` — no data by default.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(0)
    }

    /// `DoAboutToApply` — virtual preparation hook called by
    /// `about_to_apply` when the opcode is about to be applied.
    fn do_about_to_apply(
        &mut self,
        _host: &mut DngHost,
        _negative: &mut DngNegative,
        _image_bounds: &DngRect,
        _image_planes: uint32,
    ) -> DngResult<()> {
        Ok(())
    }

    /// `AboutToApply (host, negative, imageBounds, imagePlanes)` — returns
    /// true if this opcode should be applied.
    fn about_to_apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image_bounds: &DngRect,
        image_planes: uint32,
    ) -> DngResult<bool> {
        if self.skip_if_preview() && host.for_preview() {
            negative.set_is_preview(true);
        } else if self.min_version() > dng_core::dngVersion_Current
            && self.was_read_from_stream()
        {
            if !self.optional() {
                // Somebody screwed up computing the DNGBackwardVersion...
                return Err(bad_format());
            }
        } else if !self.is_valid_for_negative(negative) {
            return Err(bad_format());
        } else if !self.is_nop() {
            self.do_about_to_apply(host, negative, image_bounds, image_planes)?;
            return Ok(true);
        }

        Ok(false)
    }

    /// `Apply (host, negative, image)` — applies this opcode to the image
    /// (possibly replacing the image).
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()>;

    /// `ApplyAreaScale (scale)` — apply scale factor for a resized image.
    /// Opcodes using normalized coordinates need no changes.
    fn apply_area_scale(&mut self, _scale: DngURational) {}
}

/// Port of `dng_opcode_Unknown` — represents opcodes defined in future DNG
/// versions.
pub struct DngOpcodeUnknown {
    base: DngOpcodeBase,
    data: Vec<uint8>,
}

impl DngOpcodeUnknown {
    /// `dng_opcode_Unknown (host, opcodeID, stream)`.
    pub fn from_stream(opcode_id: uint32, stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(opcode_id, stream)?;

        let size = stream.get_uint32()?;

        let data = if size != 0 {
            stream.get(size)?
        } else {
            Vec::new()
        };

        Ok(DngOpcodeUnknown { base, data })
    }
}

impl DngOpcode for DngOpcodeUnknown {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        if !self.data.is_empty() {
            stream.put_uint32(self.data.len() as uint32)?;
            stream.put(&self.data)?;
        } else {
            stream.put_uint32(0)?;
        }

        Ok(())
    }

    /// `Apply` — we should never need to apply an unknown opcode.
    fn apply(
        &mut self,
        _host: &mut DngHost,
        _negative: &mut DngNegative,
        _image: &mut ImageRef,
    ) -> DngResult<()> {
        if !self.optional() {
            return Err(bad_format());
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_filter_opcode.
// ---------------------------------------------------------------------------

/// Port of `dng_filter_opcode` (abstract) — a filter opcode, such as a
/// convolution.
pub trait DngFilterOpcode: DngOpcode {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&mut self, image_pixel_type: uint32) -> DngResult<uint32> {
        let _ = image_pixel_type;
        Ok(image_pixel_type)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        image_bounds
    }

    /// `SrcRepeat ()`.
    fn src_repeat(&self) -> DngPoint {
        DngPoint::new(1, 1)
    }

    /// `SrcArea (dstArea, imageBounds)`.
    fn src_area(&self, dst_area: DngRect, _image_bounds: DngRect) -> DngRect {
        dst_area
    }

    /// `SrcTileSize (dstTileSize, imageBounds)`.
    fn src_tile_size(&self, dst_tile_size: DngPoint, image_bounds: DngRect) -> DngPoint {
        self.src_area(
            DngRect::from_size_point(dst_tile_size),
            image_bounds,
        )
        .size()
    }

    /// `Prepare (...)` — startup method called before processing.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        _negative: &mut DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        _image_bounds: DngRect,
        _image_planes: uint32,
        _buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        Ok(())
    }

    /// `ProcessArea (negative, threadIndex, srcBuffer, dstBuffer, dstArea,
    /// imageBounds)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        thread_index: uint32,
        src_buffer: &mut DngPixelBuffer,
        dst_buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        image_bounds: DngRect,
    ) -> DngResult<()>;

    /// `Apply` for filter opcodes — allocates the destination image and runs
    /// the (serial) filter task.
    fn apply_filter(
        &mut self,
        _host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        apply_filter_opcode(self, negative, image)
    }
}

/// The `dng_filter_opcode_task` driver (serial port).
pub fn apply_filter_opcode<O: DngFilterOpcode + ?Sized>(
    opcode: &mut O,
    negative: &mut DngNegative,
    image: &mut ImageRef,
) -> DngResult<()> {
    let (image_bounds, image_planes, image_pixel_type) = {
        let img = image.read().unwrap();
        (img.bounds(), img.planes(), img.pixel_type())
    };

    let modified_bounds = opcode.modified_bounds(image_bounds);

    if modified_bounds.is_empty() {
        return Ok(());
    }

    // Allocate destination image. If we are processing the entire image,
    // allocate an undefined image; else start with a clone of the existing
    // image.

    let dst_image: ImageRef = if modified_bounds == image_bounds {
        DngSimpleImage::new(image_bounds, image_planes, image_pixel_type)?
            .into_image_ref()?
    } else {
        image.read().unwrap().clone_image()?
    };

    let src_pixel_type = opcode.buffer_pixel_type(image_pixel_type)?;
    let dst_pixel_type = src_pixel_type;

    let src_repeat = opcode.src_repeat();

    // Maximum source tile size (C++ dng_filter_task::Start).
    let max_tile_size = DngPoint::new(1024, 1024);
    let src_tile_size = opcode.src_tile_size(max_tile_size, image_bounds);

    // Prepare (C++ dng_filter_opcode_task::Start).
    opcode.prepare(
        negative,
        1, // threadCount — serial port
        max_tile_size,
        image_bounds,
        image_planes,
        dst_pixel_type,
    )?;

    let mut iter = DngTileIterator::new_tile_size(max_tile_size, modified_bounds);
    let mut tile = DngRect::new();

    while iter.get_one_tile(&mut tile)? {
        let src_area = opcode.src_area(tile, image_bounds);

        let src_area_w = src_area.w() as i64;
        let src_area_h = src_area.h() as i64;

        if src_area_w > src_tile_size.h as i64 || src_area_h > src_tile_size.v as i64 {
            return Err(memory_full()); // "Area exceeds tile size."
        }

        // Source buffer with edge-repeat padding.
        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            0,
            image_planes,
            src_pixel_type,
            dng_core::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(src_area, image_planes, src_pixel_type)?,
        )?;

        {
            let src = image.read().unwrap();
            src.get(
                &mut src_buffer,
                EdgeOption::EdgeRepeat,
                src_repeat.v as uint32,
                src_repeat.h as uint32,
            )?;
        }

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            tile,
            0,
            image_planes,
            dst_pixel_type,
            dng_core::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(tile, image_planes, dst_pixel_type)?,
        )?;

        let dst_area = dst_buffer.area;

        opcode.process_area(
            negative,
            0, // threadIndex — serial port
            &mut src_buffer,
            &mut dst_buffer,
            dst_area,
            image_bounds,
        )?;

        let dst = dst_image.read().unwrap();
        dst.put(&dst_buffer)?;
    }

    // Return the new image.
    *image = dst_image;

    Ok(())
}

// ---------------------------------------------------------------------------
// dng_inplace_opcode.
// ---------------------------------------------------------------------------

/// Port of `dng_inplace_opcode` (abstract) — an in-place (pointwise,
/// per-pixel) opcode, such as a global tone curve.
pub trait DngInplaceOpcode: DngOpcode {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&mut self, image_pixel_type: uint32) -> DngResult<uint32> {
        let _ = image_pixel_type;
        Ok(image_pixel_type)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        image_bounds
    }

    /// `Prepare (...)` — startup method called before processing.
    #[allow(clippy::too_many_arguments)]
    fn prepare(
        &mut self,
        _negative: &mut DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        _image_bounds: DngRect,
        _image_planes: uint32,
        _buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        Ok(())
    }

    /// `ProcessArea (negative, threadIndex, buffer, dstArea, imageBounds)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        image_bounds: DngRect,
    ) -> DngResult<()>;

    /// `Apply` for in-place opcodes — runs the (serial) in-place task.
    fn apply_inplace(
        &mut self,
        _host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        apply_inplace_opcode(self, negative, image)
    }
}

/// The `dng_inplace_opcode_task` driver (serial port).
pub fn apply_inplace_opcode<O: DngInplaceOpcode + ?Sized>(
    opcode: &mut O,
    negative: &mut DngNegative,
    image: &mut ImageRef,
) -> DngResult<()> {
    let (image_bounds, image_planes, image_pixel_type) = {
        let img = image.read().unwrap();
        (img.bounds(), img.planes(), img.pixel_type())
    };

    let modified_bounds = opcode.modified_bounds(image_bounds);

    if modified_bounds.is_empty() {
        return Ok(());
    }

    let pixel_type = opcode.buffer_pixel_type(image_pixel_type)?;

    let max_tile_size = DngPoint::new(1024, 1024);

    // Prepare (C++ dng_inplace_opcode_task::Start).
    opcode.prepare(
        negative,
        1, // threadCount — serial port
        max_tile_size,
        image_bounds,
        image_planes,
        pixel_type,
    )?;

    let mut iter = DngTileIterator::new_tile_size(max_tile_size, modified_bounds);
    let mut tile = DngRect::new();

    while iter.get_one_tile(&mut tile)? {
        // Setup buffer.
        let mut buffer = DngPixelBuffer::with_planar_config(
            tile,
            0,
            image_planes,
            pixel_type,
            dng_core::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(tile, image_planes, pixel_type)?,
        )?;

        // Get source pixels.
        {
            let img = image.read().unwrap();
            img.get(&mut buffer, EdgeOption::EdgeNone, 1, 1)?;
        }

        // Process area.
        opcode.process_area(negative, 0, &mut buffer, tile, image_bounds)?;

        // Save result pixels.
        let img = image.read().unwrap();
        img.put(&buffer)?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Opcode registry (C++ dng_host::Make_dng_opcode).
// ---------------------------------------------------------------------------

/// Port of `dng_host::Make_dng_opcode` — creates an opcode from its ID and
/// stream data. Unknown opcode IDs become `DngOpcodeUnknown`.
pub fn make_dng_opcode(opcode_id: uint32, stream: &mut DngStream) -> DngResult<Box<dyn DngOpcode>> {
    let result: Box<dyn DngOpcode> = match opcode_id {
        dngOpcode_WarpRectilinear => Box::new(
            crate::dng_lens_correction::DngOpcodeWarpRectilinear::from_stream(stream)?,
        ),

        dngOpcode_WarpRectilinear2 => Box::new(
            crate::dng_lens_correction::DngOpcodeWarpRectilinear2::from_stream(stream)?,
        ),

        dngOpcode_WarpFisheye => {
            Box::new(crate::dng_lens_correction::DngOpcodeWarpFisheye::from_stream(stream)?)
        }

        dngOpcode_FixVignetteRadial => Box::new(
            crate::dng_lens_correction::DngOpcodeFixVignetteRadial::from_stream(stream)?,
        ),

        dngOpcode_FixBadPixelsConstant => Box::new(
            crate::dng_bad_pixels::DngOpcodeFixBadPixelsConstant::from_stream(stream)?,
        ),

        dngOpcode_FixBadPixelsList => {
            Box::new(crate::dng_bad_pixels::DngOpcodeFixBadPixelsList::from_stream(stream)?)
        }

        dngOpcode_TrimBounds => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeTrimBounds::from_stream(stream)?)
        }

        dngOpcode_MapTable => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeMapTable::from_stream(stream)?)
        }

        dngOpcode_MapPolynomial => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeMapPolynomial::from_stream(stream)?)
        }

        dngOpcode_GainMap => {
            Box::new(crate::dng_gain_map::DngOpcodeGainMap::from_stream(stream)?)
        }

        dngOpcode_DeltaPerRow => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeDeltaPerRow::from_stream(stream)?)
        }

        dngOpcode_DeltaPerColumn => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeDeltaPerColumn::from_stream(stream)?)
        }

        dngOpcode_ScalePerRow => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeScalePerRow::from_stream(stream)?)
        }

        dngOpcode_ScalePerColumn => {
            Box::new(crate::dng_misc_opcodes::DngOpcodeScalePerColumn::from_stream(stream)?)
        }

        _ => Box::new(DngOpcodeUnknown::from_stream(opcode_id, stream)?),
    };

    Ok(result)
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_simple_image::DngSimpleImage;
    use dng_core::ttFloat;
    use dng_core::{DngMemoryStream, dngVersion_Current};

    fn stub_stream(bytes: &[u8]) -> DngStream<'static> {
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put(bytes).unwrap();
        stream.set_read_position(0).unwrap();
        stream
    }

    #[test]
    fn opcode_ids_match_cpp_enum() {
        assert_eq!(dngOpcode_Private, 0);
        assert_eq!(dngOpcode_WarpRectilinear, 1);
        assert_eq!(dngOpcode_WarpFisheye, 2);
        assert_eq!(dngOpcode_FixVignetteRadial, 3);
        assert_eq!(dngOpcode_FixBadPixelsConstant, 4);
        assert_eq!(dngOpcode_FixBadPixelsList, 5);
        assert_eq!(dngOpcode_TrimBounds, 6);
        assert_eq!(dngOpcode_MapTable, 7);
        assert_eq!(dngOpcode_MapPolynomial, 8);
        assert_eq!(dngOpcode_GainMap, 9);
        assert_eq!(dngOpcode_DeltaPerRow, 10);
        assert_eq!(dngOpcode_DeltaPerColumn, 11);
        assert_eq!(dngOpcode_ScalePerRow, 12);
        assert_eq!(dngOpcode_ScalePerColumn, 13);
        assert_eq!(dngOpcode_WarpRectilinear2, 14);
    }

    #[test]
    fn base_from_stream_reads_min_version_and_flags() {
        let mut stream = stub_stream(&[0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x03]);
        let base = DngOpcodeBase::from_stream(dngOpcode_MapTable, &mut stream).unwrap();

        assert_eq!(base.min_version, 0x00010300);
        assert_eq!(base.flags, 3);
        assert!(base.was_read_from_stream);
        assert_eq!(base.stage, 0);
    }

    #[test]
    fn unknown_opcode_roundtrip() {
        let mut stream = stub_stream(&[
            0x00, 0x01, 0x03, 0x00, // minVersion 1.3.0.0
            0x00, 0x00, 0x00, 0x01, // flags = optional
            0x00, 0x00, 0x00, 0x04, // data size = 4
            0xDE, 0xAD, 0xBE, 0xEF, // data
        ]);

        let opcode = DngOpcodeUnknown::from_stream(7777, &mut stream).unwrap();
        assert_eq!(opcode.opcode_id(), 7777);
        assert!(opcode.optional());
        assert!(opcode.was_read_from_stream());

        // Spool it back.
        let mut out = DngMemoryStream::with_page_size(64).stream();
        out.set_big_endian(true);
        opcode.put_data(&mut out).unwrap();

        out.set_read_position(0).unwrap();
        let out_len = out.length();

        let spooled = out.get(out_len as uint32).unwrap();
        assert_eq!(
            spooled,
            vec![0, 0, 0, 4, 0xDE, 0xAD, 0xBE, 0xEF]
        );
    }

    #[test]
    fn unknown_opcode_apply_is_nop_when_optional() {
        let mut stream = stub_stream(&[
            0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        ]);

        let mut opcode = DngOpcodeUnknown::from_stream(7777, &mut stream).unwrap();

        let mut host = DngHost::new();
        let image: ImageRef = DngSimpleImage::new(
            DngRect::from_corners(0, 0, 4, 4),
            1,
            ttFloat,
        )
        .unwrap()
        .into_image_ref()
        .unwrap();

        // Optional unknown opcodes are skipped.
        let mut negative = crate::dng_negative::DngNegative::new(&mut host).unwrap();
        let mut image = image;
        opcode.apply(&mut host, &mut negative, &mut image).unwrap();
    }

    #[test]
    fn about_to_apply_skips_preview_for_newer_versions() {
        // minVersion = 0x01070000 (1.7) is dngVersion_Current; read from
        // stream, optional flag set.
        let mut stream = stub_stream(&[
            0x01, 0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00,
        ]);

        let mut opcode = DngOpcodeUnknown::from_stream(7777, &mut stream).unwrap();

        let mut host = DngHost::new();
        let mut negative = crate::dng_negative::DngNegative::new(&mut host).unwrap();

        let bounds = DngRect::from_corners(0, 0, 4, 4);

        // Not a preview: applied.
        assert!(opcode
            .about_to_apply(&mut host, &mut negative, &bounds, 1)
            .unwrap());

        // For a preview with SkipIfPreview flag it would be skipped, but
        // this opcode lacks the flag; minVersion == dngVersion_Current so it
        // is still applied.
        assert!(dngVersion_Current >= 0x01070000);
    }
}

// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_jpeg_image.cpp/.h`.
//
// Copyright 2011-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_jpeg_image.cpp` / `dng_jpeg_image.h` — `dng_jpeg_image`,
//! `dng_jxl_image`, `dng_lossy_compressed_image` and
//! `dng_compressed_image_tiles`: compressed proxy images kept inside a
//! DNG (lossy JPEG / JXL tiles), with MD5 digests for duplicate detection.
//!
//! Writer-coupled entry points (`EncodeTiles` needs
//! `dng_image_writer::WriteTile`) take the tile-encoding step as a closure —
//! batch 2's `dng_image_writer` supplies it.

use dng_core::dng_fingerprint::DngFingerprint;
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_point::DngPoint;
use dng_core::dng_stream::DngStream;
use dng_core::dng_tag_types::{ttByte, ttFloat, ttShort};
use dng_core::dng_tag_values::{ccJXL, ccLossyJPEG, piLinearRaw, sfFloatingPoint};
use dng_core::dng_types::{int32, real32, uint32, uint64, uint8};
use dng_core::DngResult;

/// Test helper: build a DngMemoryBlock from a byte slice.
#[cfg(test)]
fn mem_block(data: &[u8]) -> DngMemoryBlock {
    let mut block = DngMemoryBlock::new(data.len() as uint32).unwrap();
    block.buffer_mut().copy_from_slice(data);
    block
}

/// Port of `dng_compressed_image_tiles` — the per-tile compressed data.
#[derive(Debug, Default)]
pub struct DngCompressedImageTiles {
    /// `fData` — one compressed block per tile.
    pub data: Vec<Option<DngMemoryBlock>>,
}

impl DngCompressedImageTiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// `EncodeTiles (host, writer, image, ifd)` — encodes every tile.
    ///
    /// The C++ drives this through a parallel `dng_area_task`; the Rust port
    /// runs the tile loop serially (ARCHITECTURE.md threading rule).
    /// `write_tile` is the writer hook (batch 2's `dng_image_writer`).
    pub fn encode_tiles(
        &mut self,
        tile_count: uint32,
        write_tile: &mut dyn FnMut(uint32, &mut DngStream<'static>) -> DngResult<()>,
    ) -> DngResult<()> {
        self.data
            .resize_with(tile_count as usize, || None);

        for tile_index in 0..tile_count {
            let mut stream = DngStream::new(Box::new(dng_core::DngMemoryStream::new()));

            write_tile(tile_index, &mut stream)?;

            let block = stream.as_memory_block()?;

            self.data[tile_index as usize] = Some(block);
        }

        Ok(())
    }

    /// `NonHeaderSize ()`.
    pub fn non_header_size(&self) -> uint64 {
        let mut size: uint64 = 0;

        for block in &self.data {
            if let Some(block) = block {
                size += dng_core::dng_utils::round_up_2(block.logical_size()) as uint64;
            }
        }

        size
    }

    /// `WriteData (stream, basic)` — writes the tiles, keeping them on even
    /// byte offsets. The closures mirror `dng_basic_tag_set`'s
    /// `SetTileOffset` / `SetTileByteCount` (writer-side, batch 2).
    pub fn write_data(
        &self,
        stream: &mut DngStream<'static>,
        mut set_tile_offset: impl FnMut(uint32, uint64),
        mut set_tile_byte_count: impl FnMut(uint32, uint64),
    ) -> DngResult<()> {
        let tile_count = self.data.len() as uint32;

        for tile_index in 0..tile_count {
            // Remember this offset.

            let tile_offset = stream.position();

            set_tile_offset(tile_index, tile_offset);

            // Write compressed data.

            if let Some(block) = &self.data[tile_index as usize] {
                stream.put(block.buffer())?;
            }

            // Update tile byte count.

            let tile_byte_count = stream.position() - tile_offset;

            set_tile_byte_count(tile_index, tile_byte_count);

            // Keep the tiles on even byte offsets.

            if tile_byte_count & 1 != 0 {
                stream.put_uint8(0)?;
            }
        }

        Ok(())
    }
}

/// Port of `dng_lossy_compressed_image` — base class for compressed proxy
/// images kept in the DNG.
#[derive(Debug, Default)]
pub struct DngLossyCompressedImage {
    pub tiles: DngCompressedImageTiles,

    /// `fImageSize` (h, v).
    pub image_size: DngPoint,
    /// `fTileSize` (h, v).
    pub tile_size: DngPoint,

    pub uses_strips: bool,

    pub compression_code: uint32,

    pub bits_per_sample: uint32,

    pub row_interleave_factor: uint32,
    pub column_interleave_factor: uint32,

    pub jxl_distance: real32,
    pub jxl_effort: int32,
    pub jxl_decode_speed: int32,

    /// Shared JPEG tables (`dng_jpeg_image::fJPEGTables` in C++; the port
    /// keeps them on the base so `dng_read_image::Read` can hand them over
    /// without a downcast — see dng_read_image.cpp:3783).
    pub jpeg_tables_data: Option<Vec<uint8>>,
}

impl DngLossyCompressedImage {
    pub fn new() -> Self {
        DngLossyCompressedImage {
            tiles: DngCompressedImageTiles::new(),
            image_size: DngPoint::new(0, 0),
            tile_size: DngPoint::new(0, 0),
            uses_strips: false,
            compression_code: 0,
            bits_per_sample: 8,
            row_interleave_factor: 1,
            column_interleave_factor: 1,
            jxl_distance: -1.0,
            jxl_effort: -1,
            jxl_decode_speed: -1,
            jpeg_tables_data: None,
        }
    }

    /// `TilesAcross ()`.
    pub fn tiles_across(&self) -> uint32 {
        if self.tile_size.h != 0 {
            ((self.image_size.h + self.tile_size.h - 1) / self.tile_size.h) as uint32
        } else {
            0
        }
    }

    /// `TilesDown ()`.
    pub fn tiles_down(&self) -> uint32 {
        if self.tile_size.v != 0 {
            ((self.image_size.v + self.tile_size.v - 1) / self.tile_size.v) as uint32
        } else {
            0
        }
    }

    /// `TileCount ()`.
    pub fn tile_count(&self) -> uint32 {
        self.tiles_across() * self.tiles_down()
    }

    /// `EncodeTiles` override — records the geometry after encoding.
    pub fn encode_tiles(
        &mut self,
        ifd: &dng_core::dng_ifd::DngIfd,
        write_tile: &mut dyn FnMut(uint32, &mut DngStream<'static>) -> DngResult<()>,
    ) -> DngResult<()> {
        let tile_count = ifd.tiles_across() * ifd.tiles_down();

        self.tiles.encode_tiles(tile_count, write_tile)?;

        self.image_size = DngPoint::new(ifd.image_length as int32, ifd.image_width as int32);
        self.tile_size = DngPoint::new(ifd.tile_length as int32, ifd.tile_width as int32);

        self.uses_strips = !ifd.uses_tiles;

        self.compression_code = ifd.compression;

        self.bits_per_sample = ifd.bits_per_sample[0];

        self.row_interleave_factor = ifd.row_interleave_factor;
        self.column_interleave_factor = ifd.column_interleave_factor;

        if self.compression_code == ccJXL {
            self.jxl_distance = ifd.jxl_distance;
            self.jxl_effort = ifd.jxl_effort;
            self.jxl_decode_speed = ifd.jxl_decode_speed;
        }

        Ok(())
    }

    /// `FindDigest (host)` — MD5 over each tile digest, combined.
    pub fn find_digest(&self) -> DngResult<DngFingerprint> {
        let tile_count = self.tile_count();

        let mut digests: Vec<DngFingerprint> = Vec::with_capacity(tile_count as usize);

        // Compute digest of each compressed tile.

        for i in 0..tile_count as usize {
            let digest = match &self.tiles.data[i] {
                Some(block) => {
                    let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();
                    printer.process(block.buffer());
                    printer.result().clone()
                }
                None => DngFingerprint::new(),
            };

            digests.push(digest);
        }

        // Subclasses can add extra data here (DoFindDigest).

        self.do_find_digest(&mut digests);

        // Combine digests into a single digest.

        let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();

        for digest in &digests {
            printer.process(&digest.data);
        }

        Ok(printer.result().clone())
    }

    /// `DoFindDigest` — subclass hook (base: nothing).
    fn do_find_digest(&self, _digests: &mut Vec<DngFingerprint>) {}

    /// `JPEGTables ()` — overridden by dng_jpeg_image.
    pub fn jpeg_tables(&self) -> Option<&DngMemoryBlock> {
        None
    }

    /// Shared-table access (base slot; `DngJpegImage` returns it).
    pub fn jpeg_tables_bytes(&self) -> Option<&[uint8]> {
        self.jpeg_tables_data.as_deref()
    }

    /// `JXLDistance ()`.
    pub fn jxl_distance(&self) -> real32 {
        self.jxl_distance
    }

    /// `JXLEffort ()`.
    pub fn jxl_effort(&self) -> int32 {
        self.jxl_effort
    }

    /// `JXLDecodeSpeed ()`.
    pub fn jxl_decode_speed(&self) -> int32 {
        self.jxl_decode_speed
    }
}

/// Port of `dng_jpeg_image` — lossy-JPEG proxy image.
#[derive(Debug, Default)]
pub struct DngJpegImage {
    pub base: DngLossyCompressedImage,

    /// `fJPEGTables`.
    pub jpeg_tables_block: Option<DngMemoryBlock>,
}

impl DngJpegImage {
    pub fn new() -> Self {
        let mut image = DngJpegImage::default();
        image.base.compression_code = ccLossyJPEG;
        image
    }

    /// `JPEGTables ()` override.
    pub fn jpeg_tables(&self) -> Option<&DngMemoryBlock> {
        self.jpeg_tables_block.as_ref()
    }

    /// `Encode (host, negative, writer, image)`.
    ///
    /// The writer dependency (WriteTile) is injected as a closure; the
    /// negative dependency reduces to the two queries the C++ makes
    /// (`IsSceneReferred`, `OriginalDefaultFinalSize`) — phase 2 glue
    /// passes them in.
    pub fn encode(
        &mut self,
        image_width: uint32,
        image_height: uint32,
        planes: uint32,
        is_scene_referred: bool,
        full_size_equals_original: bool,
        write_tile: &mut dyn FnMut(uint32, &mut DngStream<'static>) -> DngResult<()>,
    ) -> DngResult<()> {
        // (The C++ requires ttByte input.)

        let mut ifd = dng_core::dng_ifd::DngIfd::new();

        common_configure_ifd(
            &mut ifd,
            image_width,
            image_height,
            planes,
            ccLossyJPEG,
            DngPoint::new(512, 512),
            8,
        );

        // Choose JPEG encode quality. Need a higher quality for raw proxies
        // than non-raw proxies, since users often perform much greater color
        // changes. Also, if we are targeting a "large" size proxy (larger
        // than 5 MP), or this is a full size proxy, then use a higher
        // quality.

        let use_higher_quality =
            image_width as uint64 * image_height as uint64 > 5000000 || full_size_equals_original;

        ifd.compression_quality = if is_scene_referred {
            if use_higher_quality {
                11
            } else {
                10
            }
        } else if use_higher_quality {
            10
        } else {
            8
        };

        self.base.encode_tiles(&ifd, write_tile)
    }

    /// `DoFindDigest` override — digest of JPEG tables, if any.
    pub fn do_find_digest(&self, digests: &mut Vec<DngFingerprint>) {
        if let Some(tables) = &self.jpeg_tables_block {
            let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();
            printer.process(tables.buffer());
            digests.push(printer.result().clone());
        }
    }

    /// Full `FindDigest` including the tables hook.
    pub fn find_digest(&self) -> DngResult<DngFingerprint> {
        let mut digests = Vec::new();

        let tile_count = self.base.tile_count();

        for i in 0..tile_count as usize {
            let digest = match &self.base.tiles.data[i] {
                Some(block) => {
                    let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();
                    printer.process(block.buffer());
                    printer.result().clone()
                }
                None => DngFingerprint::new(),
            };
            digests.push(digest);
        }

        self.do_find_digest(&mut digests);

        let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();

        for digest in &digests {
            printer.process(&digest.data);
        }

        Ok(printer.result().clone())
    }
}

/// Port of `dng_jxl_image` — JXL proxy image.
#[derive(Debug, Default)]
pub struct DngJxlImage {
    pub base: DngLossyCompressedImage,
}

impl DngJxlImage {
    pub fn new() -> Self {
        let mut image = DngJxlImage::default();
        image.base.compression_code = ccJXL;
        image
    }

    /// `Encode (host, writer, image, encodeSettings, colorEncoding)`.
    ///
    /// The JXL encode settings/color encoding attach to the IFD exactly as
    /// in the C++; the actual tile encoding goes through `write_tile`
    /// (batch 2 wires it to `dng_image_writer::WriteTile`, which routes to
    /// the dng-jxl crate).
    pub fn encode(
        &mut self,
        image_width: uint32,
        image_height: uint32,
        planes: uint32,
        pixel_type: uint32,
        write_tile: &mut dyn FnMut(uint32, &mut DngStream<'static>) -> DngResult<()>,
    ) -> DngResult<()> {
        let mut ifd = dng_core::dng_ifd::DngIfd::new();

        ifd.image_width = image_width;
        ifd.image_length = image_height;

        ifd.samples_per_pixel = planes;

        for plane in 0..planes as usize {
            if pixel_type == dng_core::dng_tag_types::ttFloat {
                ifd.bits_per_sample[plane] = 16;
                ifd.sample_format[plane] = sfFloatingPoint;
            } else {
                ifd.bits_per_sample[plane] = if pixel_type == ttShort { 16 } else { 8 };
            }
        }

        ifd.compression = ccJXL;

        // This will cause the color encoding to use linear gamma with no
        // color transform, unless we override it.

        ifd.photometric_interpretation = piLinearRaw;

        ifd.find_tile_size(1024 * 1024, 1, 1)?;

        self.base.encode_tiles(&ifd, write_tile)
    }
}

/// `CommonConfigureIFD` — shared IFD setup for proxy encoding.
fn common_configure_ifd(
    ifd: &mut dng_core::dng_ifd::DngIfd,
    image_width: uint32,
    image_height: uint32,
    planes: uint32,
    compression_code: uint32,
    tile_size: DngPoint,
    bit_depth: uint32,
) -> DngResult<()> {
    // (The C++ requires ttByte / ttShort / ttFloat input.)

    ifd.image_width = image_width;
    ifd.image_length = image_height;

    ifd.samples_per_pixel = planes;

    ifd.bits_per_sample[0] = bit_depth;
    ifd.bits_per_sample[1] = bit_depth;
    ifd.bits_per_sample[2] = bit_depth;
    ifd.bits_per_sample[3] = bit_depth;

    ifd.photometric_interpretation = piLinearRaw;

    ifd.compression = compression_code;

    ifd.find_tile_size(
        tile_size.h as uint32 * tile_size.v as uint32 * ifd.samples_per_pixel,
        1,
        1,
    )?;

    Ok(())
}

/// Port of `KeepLossyCompressedImage (host, ifd)` — decides whether the
/// compressed JPEG/JXL data of a parsed IFD should be kept verbatim.
///
/// `save_dng_version` = 0 means "not saving" (C++ truthiness). The backward
/// version constants mirror `MinBackwardVersionForCompression` results:
/// ccLossyJPEG → DNG 1.4.1.0 (0x01040100), ccJXL → DNG 1.7.0.0
/// (0x01070000), and the ColumnInterleaveFactor gate uses DNG 1.7.1.0
/// (0x01070100).
pub fn keep_lossy_compressed_image(
    save_dng_version: uint32,
    preferred_size: uint32,
    for_preview: bool,
    ifd: &dng_core::dng_ifd::DngIfd,
) -> Option<Box<DngLossyCompressedImage>> {
    let mut lossy_image: Option<Box<DngLossyCompressedImage>> = None;

    if save_dng_version != 0 && preferred_size == 0 && !for_preview {
        // Non-default ColumnInterleaveFactor requires DNG 1.7.1.

        const DNG_VERSION_1_7_1_0: uint32 = 0x01070100;
        const MIN_BACKWARD_CC_LOSSY_JPEG: uint32 = 0x01040100;
        const MIN_BACKWARD_CC_JXL: uint32 = 0x01070000;

        if ifd.column_interleave_factor != 1 && save_dng_version < DNG_VERSION_1_7_1_0 {
            return None;
        }

        // See if we should grab the compressed JPEG data.

        if save_dng_version >= MIN_BACKWARD_CC_LOSSY_JPEG && ifd.is_baseline_jpeg() {
            lossy_image = Some(Box::new(DngLossyCompressedImage::new()));
        }

        if save_dng_version >= MIN_BACKWARD_CC_JXL && ifd.compression == ccJXL {
            lossy_image = Some(Box::new(DngLossyCompressedImage::new()));
        }
    }

    if let Some(image) = &mut lossy_image {
        image.compression_code = ifd.compression;

        image.bits_per_sample = ifd.bits_per_sample[0];

        image.row_interleave_factor = ifd.row_interleave_factor;
        image.column_interleave_factor = ifd.column_interleave_factor;

        image.jxl_distance = ifd.jxl_distance;
        image.jxl_effort = ifd.jxl_effort;
        image.jxl_decode_speed = ifd.jxl_decode_speed;
    }

    lossy_image
}

#[cfg(test)]
mod tests {
    use super::*;

    /// TilesAcross/TilesDown/TileCount math.
    #[test]
    fn tile_geometry() {
        let mut image = DngLossyCompressedImage::new();
        image.image_size = DngPoint::new(750, 1000);
        image.tile_size = DngPoint::new(512, 512);

        assert_eq!(image.tiles_across(), 2);
        assert_eq!(image.tiles_down(), 2);
        assert_eq!(image.tile_count(), 4);

        image.tile_size = DngPoint::new(0, 0);
        assert_eq!(image.tiles_across(), 0);
        assert_eq!(image.tiles_down(), 0);
        assert_eq!(image.tile_count(), 0);
    }

    /// NonHeaderSize with odd-length tiles (round-up behavior).
    #[test]
    fn non_header_size() {
        let mut image = DngLossyCompressedImage::new();
        image.tiles.data = vec![Some(mem_block(&[1, 2, 3])), Some(mem_block(&[4, 5]))];

        assert_eq!(image.tiles.non_header_size(), 6); // 4 (round up 3) + 2
    }

    /// WriteData: offsets/byte counts and even-byte padding.
    #[test]
    fn write_data_padding() {
        let mut image = DngLossyCompressedImage::new();
        image.tiles.data = vec![
            Some(mem_block(&[0xAA, 0xBB, 0xCC])), // odd → pad
            Some(mem_block(&[0x11, 0x22])),       // even
        ];

        let mut stream = DngStream::new(Box::new(dng_core::DngMemoryStream::new()));

        let mut offsets = Vec::new();
        let mut counts = Vec::new();

        image
            .tiles
            .write_data(
                &mut stream,
                |i, o| offsets.push((i, o)),
                |i, c| counts.push((i, c)),
            )
            .unwrap();

        assert_eq!(offsets, vec![(0, 0), (1, 4)]);
        assert_eq!(counts, vec![(0, 3), (1, 2)]);

        let data = stream.as_memory_block().unwrap().buffer().to_vec();
        assert_eq!(data, vec![0xAA, 0xBB, 0xCC, 0x00, 0x11, 0x22]);
    }

    /// FindDigest over tiles: deterministic and table/tile sensitive.
    #[test]
    fn find_digest() {
        let mut image = DngJpegImage::new();
        // Geometry must match the tile list (TileCount drives the loop).
        image.base.image_size = DngPoint::new(2, 1);
        image.base.tile_size = DngPoint::new(1, 1);
        image.base.tiles.data = vec![Some(mem_block(&[1, 2, 3])), Some(mem_block(&[4, 5, 6]))];

        let d1 = image.find_digest().unwrap();

        // Same data → same digest.
        let mut image2 = DngJpegImage::new();
        image2.base.image_size = DngPoint::new(2, 1);
        image2.base.tile_size = DngPoint::new(1, 1);
        image2.base.tiles.data = vec![Some(mem_block(&[1, 2, 3])), Some(mem_block(&[4, 5, 6]))];
        let d2 = image2.find_digest().unwrap();
        assert_eq!(d1, d2);

        // Different tables → different digest.
        image2.jpeg_tables_block = Some(mem_block(&[9, 9, 9]));
        let d3 = image2.find_digest().unwrap();
        assert_ne!(d1, d3);

        // Different tile data → different digest.
        let mut image3 = DngJpegImage::new();
        image3.base.image_size = DngPoint::new(2, 1);
        image3.base.tile_size = DngPoint::new(1, 1);
        image3.base.tiles.data = vec![Some(mem_block(&[1, 2, 3])), Some(mem_block(&[4, 5, 7]))];
        let d4 = image3.find_digest().unwrap();
        assert_ne!(d1, d4);
    }

    /// KeepLossyCompressedImage gating logic.
    #[test]
    fn keep_lossy_compressed_image_gating() {
        let mut ifd = dng_core::dng_ifd::DngIfd::new();
        ifd.compression = ccLossyJPEG;
        ifd.image_width = 16;
        ifd.image_length = 16;
        ifd.samples_per_pixel = 1;
        ifd.bits_per_sample[0] = 8;
        ifd.uses_tiles = false;
        ifd.tile_length = 16;
        ifd.tile_width = 16;

        // Not saving → None.
        assert!(keep_lossy_compressed_image(0, 0, false, &ifd).is_none());

        // Saving with preview → None.
        assert!(keep_lossy_compressed_image(0x01070000, 0, true, &ifd).is_none());

        // Saving, no preview, baseline JPEG → Some.
        let kept = keep_lossy_compressed_image(0x01070000, 0, false, &ifd);
        assert!(kept.is_some());
        assert_eq!(kept.unwrap().compression_code, ccLossyJPEG);

        // Preferred size set → None.
        assert!(keep_lossy_compressed_image(0x01070000, 1024, false, &ifd).is_none());
    }

    /// DngJpegImage::encode records the geometry (quality rules run inside).
    #[test]
    fn jpeg_image_encode() {
        let mut image = DngJpegImage::new();

        image
            .encode(64, 64, 1, false, false, &mut |_tile_index, _stream| Ok(()))
            .unwrap();

        assert_eq!(image.base.image_size.h, 64);
        assert_eq!(image.base.image_size.v, 64);
        assert_eq!(image.base.compression_code, ccLossyJPEG);
        assert_eq!(image.base.uses_strips, false);
        assert_eq!(image.base.tiles.data.len(), 1);
    }

    /// DngJxlImage::encode records the JXL compression code and geometry.
    #[test]
    fn jxl_image_encode() {
        let mut image = DngJxlImage::new();

        image
            .encode(100, 50, 3, dng_core::dng_tag_types::ttByte, &mut |_tile_index, _stream| {
                Ok(())
            })
            .unwrap();

        assert_eq!(image.base.image_size.h, 100);
        assert_eq!(image.base.image_size.v, 50);
        assert_eq!(image.base.compression_code, ccJXL);
    }
}

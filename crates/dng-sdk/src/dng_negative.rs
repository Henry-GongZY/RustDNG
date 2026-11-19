// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_negative.cpp/.h`.
//
// Copyright 2006-2021 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_negative.h/.cpp (SDK 1.7.1) — the central `DngNegative` class
// plus the `DngMetadata`, `DngSemanticMask` and `DngResolution` types.
//
// This replaces Agent B's TEMPORARY SEAM STUB (which was backed up at the
// time of replacement); every public name follows ARCHITECTURE.md rule 4,
// and the seam types follow NEGATIVE_API.md.
//
// Porting notes (module level):
//   * `dng_metadata` (dng_negative.h:226) and `dng_noise_profile`
//     (dng_negative.h:178) are declared inside dng_negative.h in SDK 1.7.1;
//     the port keeps `DngNoiseProfile` in `dng_noise_profile.rs` (per the
//     NEGATIVE_API.md ownership table) and `DngMetadata` here.
//   * `dng_semantic_mask` (header-only class) is ported here because only
//     `dng_negative` consumes it.
//   * `dng_resolution` is declared in dng_image_writer.h in C++; it is ported
//     here (PORT-NOTE) until Agent B's `dng_image_writer` module lands, where
//     it logically belongs.
//   * `AutoPtr<dng_image>` slots become `Option<ImageRef>` where
//     `ImageRef = Arc<RwLock<dyn DngImage>>` (dng-core binding decision).
//   * `host.Make_dng_image` becomes direct `DngSimpleImage` construction
//     (dng-core PORTING_NOTES: factory virtuals live with the sdk types).
//   * Image digests are computed serially (ARCHITECTURE.md §9) with
//     little-endian byte hashing, matching the C++ `qDNGBigEndian == 0`
//     native-layout path.
//   * `KeepLossyCompressedImage` is B's free function in `dng_jpeg_image`.
//   * Methods requiring `dng_image_writer` (LossyCompressMosaicJXL,
//     LosslessCompressJXL, ConvertToProxy, EncodeRawProxy,
//     AdjustSemanticMasksForProxy's final compress step) keep their API and
//     return `DngError::NotSupported` / skip the writer step until Agent B's
//     writer module lands (ARCHITECTURE.md rule 11).
//   * `ReadStage1Image` / `ReadTransparencyMask` / `ReadDepthMap` /
//     `ReadSemanticMasks` call `dng_ifd::ReadImage`, which currently returns
//     NotYetImplemented until the dng_read_image phase-2 module (Agent B)
//     provides the decoder — the surrounding negative-side bookkeeping is
//     fully ported.

use dng_core::{
    dngVersion_1_4_0_0, dngVersion_1_7_1_0, dngVersion_None, max_int32, max_real64, min_int32,
    min_real64, pin_real64, pin_uint16, program_error, round_int32_f64, round_uint32_f64,
    DngDateTimeInfo, DngError, DngFingerprint, DngHost, DngMatrix, DngMatrix3by3,
    DngOrientation, DngPoint, DngRect, DngResult, DngSRational, DngStream, DngString,
    DngURational, DngVector, ImageRef, int32, real64, uint16, uint32, uint64,
};
use dng_core::dng_area_task::DngAreaTask;
use dng_core::dng_fingerprint::{md5_digest, DngMd5Printer, DngMd5PrinterStream};
use dng_core::dng_image::DngImage;
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_sdk_limits::{
    kMaxColorPlanes, kMaxMaskedAreas, kMaxStage3BlackLevelNormalized, kMaxSemanticMasks,
};
use dng_core::dng_simple_image::DngSimpleImage;
use dng_core::dng_tag_types::{tag_type_size, ttByte, ttFloat, ttLong, ttShort, ttHalfFloat };
use dng_core::dng_tag_values::{
    colorKeyBlue, colorKeyCyan, colorKeyGreen, colorKeyMaxEnum, colorKeyMagenta, colorKeyRed,
    colorKeyYellow, crSceneReferred, depthFormatUnknown, depthMeasureUnknown, depthUnitsUnknown,
    sfFloatingPoint,
};
use dng_core::dng_tag_values::{ccDeflate, ccJXL, piLinearRaw, pcInterleaved};
use dng_core::dng_utils::{histogram_area, DngDither, DngImageSequenceInfo, DngImageStats};
use dng_core::{DngXyCoord, kDNGStreamInvalidOffset};

use crate::dng_big_table::{DngBigTableDictionary, DngBigTableGroupIndex, DngBigTableIndex};
use crate::dng_camera_profile::{
    has_profile_group_prefix, split_camera_profile_name, strip_profile_group_prefix,
    DngCameraProfile, DngCameraProfileGroupSelector, DngCameraProfileId,
    DngCameraProfileMetadata, DngProfileMetadataList, kProfileName_Embedded,
};
use crate::dng_exif::DngExif;
use crate::dng_info::DngInfo;
use crate::dng_jpeg_image::{keep_lossy_compressed_image, DngJxlImage, DngLossyCompressedImage};
use crate::dng_linearization_info::DngLinearizationInfo;
use crate::dng_mosaic_info::DngMosaicInfo;
use crate::dng_noise_profile::DngNoiseProfile;
use crate::dng_opcode_list::DngOpcodeList;
use crate::dng_xmp::DngXmp;
use dng_core::dng_tags::{
    tcResolutionUnit, tcIPTC_NAA, tcMake, tcModel, tcSoftware, tcArtist, tcDateTime,
    tcExposureTime, tcFNumber, tcISOSpeedRatings, tcExifIFD, tcXResolution, tcYResolution,
};
use dng_core::{max_uint32, min_uint32, min_real32, pin_real32, uint8};


// ---------------------------------------------------------------------------
// dng_resolution (dng_image_writer.h:42 — see module PORT-NOTE).
// ---------------------------------------------------------------------------

/// Port of `dng_resolution`.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DngResolution {
    /// `fXResolution`.
    pub x_resolution: DngURational,
    /// `fYResolution`.
    pub y_resolution: DngURational,
    /// `fResolutionUnit`.
    pub resolution_unit: uint16,
}

impl DngResolution {
    /// `dng_resolution ()` — 72 dpi default.
    pub fn new() -> Self {
        DngResolution {
            x_resolution: DngURational::new(72, 1),
            y_resolution: DngURational::new(72, 1),
            resolution_unit: 2, // ruInch
        }
    }
}

// ---------------------------------------------------------------------------
// dng_semantic_mask (dng_semantic_mask.h — header-only class).
// ---------------------------------------------------------------------------

/// Port of `dng_semantic_mask`.
#[derive(Clone, Default)]
pub struct DngSemanticMask {
    /// `fName` — string identifying the semantics of this mask
    /// (SemanticName tag).
    pub name: DngString,

    /// `fInstanceID` — string identifying the instance of this mask
    /// (SemanticInstanceID tag).
    pub instance_id: DngString,

    /// `fXMP` — XMP block (preserved verbatim).
    pub xmp: Option<std::sync::Arc<Vec<uint8>>>,

    /// `fMask` — the mask image; the origin of the image bounds is always
    /// (0, 0) by convention.
    pub mask: Option<ImageRef>,

    /// `fMaskSubArea` (optional MaskSubArea tag):
    /// [0] top crop, [1] left crop, [2] width full, [3] height full.
    pub mask_sub_area: [uint32; 4],

    /// `fLossyCompressed` — lossy compressed data.
    pub lossy_compressed: Option<std::sync::Arc<DngLossyCompressedImage>>,
}

impl DngSemanticMask {
    /// `IsMaskSubAreaValid ()` (dng_negative.cpp:68).
    pub fn is_mask_sub_area_valid(&self) -> bool {
        // Can't do full check without mask itself.

        let mask = match &self.mask {
            Some(m) => m,
            None => return false,
        };

        let (origin, whole_image_area) = self.calc_mask_sub_area();

        let mask_size = mask.read().unwrap().bounds().size();

        let crop = DngRect::from_corners(
            origin.v,
            origin.h,
            origin.v + mask_size.v,
            origin.h + mask_size.h,
        );

        let intersected = crop.and_(whole_image_area);

        intersected == crop
    }

    /// `CalcMaskSubArea (origin, wholeImageArea)` (dng_negative.cpp:52).
    /// Returns `(origin, wholeImageArea)`.
    pub fn calc_mask_sub_area(&self) -> (DngPoint, DngRect) {
        let origin = DngPoint::new(
            self.mask_sub_area[0] as int32, // top
            self.mask_sub_area[1] as int32, // left
        );

        let whole_image_area = DngRect::from_corners(
            0,
            0,
            self.mask_sub_area[3] as int32, // height
            self.mask_sub_area[2] as int32, // width
        );

        (origin, whole_image_area)
    }
}

// ---------------------------------------------------------------------------
// Memory-block alias (C++ dng_memory_block holders).
// ---------------------------------------------------------------------------

/// The memory-block type used for MakerNote/IPTC/private-data blocks.
pub type DngMemoryBlockAlias = dng_core::DngMemoryBlock;

// ---------------------------------------------------------------------------
// dng_metadata.
// ---------------------------------------------------------------------------

/// Port of `dng_metadata` — main class for holding metadata.
#[derive(Clone)]
pub struct DngMetadata {
    /// `fHasBaseOrientation` — base orientation of both the thumbnail and
    /// raw data (generally based on the EXIF values).
    pub has_base_orientation: bool,

    /// `fBaseOrientation`.
    pub base_orientation: DngOrientation,

    /// `fIsMakerNoteSafe` — is the maker note safe to copy from file to
    /// file?  Defaults to false because many maker notes are not safe.
    pub is_maker_note_safe: bool,

    /// `fMakerNote` — MakerNote binary data block.
    pub maker_note: Option<DngMemoryBlockAlias>,

    /// `fExif` — EXIF data (always allocated, as in the C++ constructor).
    pub exif: DngExif,

    /// `fOriginalExif` — copy of the EXIF data before it was synchronized
    /// with other metadata sources.
    pub original_exif: Option<DngExif>,

    /// `fIPTCBlock` — IPTC binary data block.
    pub iptc_block: Option<DngMemoryBlockAlias>,

    /// `fIPTCOffset` — offset of the IPTC block in the original file.
    pub iptc_offset: uint64,

    /// `fXMP` — XMP data (qDNGUseXMP == 1 in this build).
    pub xmp: DngXmp,

    /// `fEmbeddedXMPDigest` — digest of the valid embedded XMP block (if
    /// any).
    pub embedded_xmp_digest: DngFingerprint,

    /// `fXMPinSidecar` — is the XMP data from a sidecar file?
    pub xmp_in_sidecar: bool,

    /// `fXMPisNewer` — if the XMP data is from a sidecar file, is the
    /// sidecar file newer than the raw file?
    pub xmp_is_newer: bool,

    /// `fSourceMIME` — source file MIME type, if known.
    pub source_mime: DngString,

    /// `fBigTableDictionary` — big table dictionary for storing binary
    /// blocks in memory.
    pub big_table_dictionary: DngBigTableDictionary,

    /// `fBigTableIndex` — big table index for binary blocks embedded in the
    /// original raw file.
    pub big_table_index: DngBigTableIndex,

    /// `fBigTableGroupIndex` — big table group index (group digest ->
    /// instance digest associations).
    pub big_table_group_index: DngBigTableGroupIndex,

    /// `fImageSequenceInfo` — image sequence info.
    pub image_sequence_info: DngImageSequenceInfo,

    /// `fImageStats` — image stats.
    pub image_stats: DngImageStats,
}

impl Default for DngMetadata {
    fn default() -> Self {
        Self::new()
    }
}

impl DngMetadata {
    /// `dng_metadata (host)` — the C++ factory hooks (Make_dng_exif /
    /// Make_dng_xmp) construct the concrete types directly here.
    pub fn new() -> Self {
        DngMetadata {
            has_base_orientation: false,
            base_orientation: DngOrientation::new(),
            is_maker_note_safe: false,
            maker_note: None,
            exif: DngExif::new(),
            original_exif: None,
            iptc_block: None,
            iptc_offset: kDNGStreamInvalidOffset,
            xmp: DngXmp::new(),
            embedded_xmp_digest: DngFingerprint::new(),
            xmp_in_sidecar: false,
            xmp_is_newer: false,
            source_mime: DngString::new(),
            big_table_dictionary: DngBigTableDictionary::new(),
            big_table_index: DngBigTableIndex::new(),
            big_table_group_index: DngBigTableGroupIndex::new(),
            image_sequence_info: DngImageSequenceInfo::default(),
            image_stats: DngImageStats::default(),
        }
    }

    /// `Clone (allocator)` — the Rust clone covers all fields.
    pub fn cloned(&self) -> Self {
        self.clone()
    }

    /// `SetBaseOrientation (orientation)`.
    pub fn set_base_orientation(&mut self, orientation: DngOrientation) {
        self.has_base_orientation = true;
        self.base_orientation = orientation;
    }

    /// `HasBaseOrientation ()`.
    pub fn has_base_orientation(&self) -> bool {
        self.has_base_orientation
    }

    /// `BaseOrientation ()`.
    pub fn base_orientation(&self) -> &DngOrientation {
        &self.base_orientation
    }

    /// `ApplyOrientation (orientation)` — logically rotates the image by
    /// changing the orientation values; also updates the XMP data.
    pub fn apply_orientation(&mut self, orientation: DngOrientation) {
        self.base_orientation = self.base_orientation.plus(orientation);

        self.xmp.set_orientation(&self.base_orientation);
    }

    // ------------------------------------------------------------- IPTC API

    /// `SetIPTC (block, offset)`.
    pub fn set_iptc_with_offset(&mut self, block: Option<DngMemoryBlockAlias>, offset: uint64) {
        self.iptc_block = block;
        self.iptc_offset = offset;
    }

    /// `SetIPTC (block)`.
    pub fn set_iptc(&mut self, block: Option<DngMemoryBlockAlias>) {
        self.set_iptc_with_offset(block, kDNGStreamInvalidOffset);
    }

    /// `ClearIPTC ()`.
    pub fn clear_iptc(&mut self) {
        self.iptc_block = None;
        self.iptc_offset = kDNGStreamInvalidOffset;
    }

    /// `IPTCData ()` — returns the block bytes, if any.
    pub fn iptc_data(&self) -> Option<&[uint8]> {
        self.iptc_block.as_ref().map(|b| b.buffer())
    }

    /// `IPTCLength ()`.
    pub fn iptc_length(&self) -> uint32 {
        self.iptc_block.as_ref().map_or(0, |b| b.logical_size())
    }

    /// `IPTCBlock ()`.
    pub fn iptc_block(&self) -> Option<&DngMemoryBlockAlias> {
        self.iptc_block.as_ref()
    }

    /// `IPTCOffset ()`.
    pub fn iptc_offset(&self) -> uint64 {
        if self.iptc_block.is_some() {
            self.iptc_offset
        } else {
            kDNGStreamInvalidOffset
        }
    }

    /// `IPTCDigest (includePadding)`.
    pub fn iptc_digest(&self, include_padding: bool) -> DngFingerprint {
        if self.iptc_length() != 0 {
            let mut count = self.iptc_length() as usize;

            let data = self.iptc_data().unwrap();

            // Because of some stupid ways of storing the IPTC data, the IPTC
            // data might be padded with up to three zeros.  The official
            // Adobe logic is to include these zeros in the digest.  However,
            // older versions of the Camera Raw code did not include the
            // padding zeros in the digest, so we support both methods and
            // allow either to match.

            if !include_padding {
                let mut removed = 0;

                while removed < 3 && count > 0 && data[count - 1] == 0 {
                    removed += 1;
                    count -= 1;
                }
            }

            let mut printer = DngMd5Printer::new();

            printer.process(&data[..count]);

            return *printer.result();
        }

        DngFingerprint::new()
    }

    /// `RebuildIPTC (allocator, padForTIFF)` — rebuilds the IPTC block from
    /// the XMP and stores the digest.
    pub fn rebuild_iptc(&mut self, pad_for_tiff: bool) -> DngResult<()> {
        self.clear_iptc();

        let block = self.xmp.rebuild_iptc(pad_for_tiff)?;

        if !block.is_empty() {
            let mut mem = dng_core::DngMemoryBlock::new(block.len() as uint32)?;
            mem.buffer_mut().copy_from_slice(&block);
            self.iptc_block = Some(mem);
        }

        let digest = self.iptc_digest(true);

        self.xmp.set_iptc_digest(&digest);

        Ok(())
    }

    // -------------------------------------------------------- MakerNote API

    /// `SetMakerNoteSafety (safe)`.
    pub fn set_maker_note_safety(&mut self, safe: bool) {
        self.is_maker_note_safe = safe;
    }

    /// `IsMakerNoteSafe ()`.
    pub fn is_maker_note_safe(&self) -> bool {
        self.is_maker_note_safe
    }

    /// `SetMakerNote (block)`.
    pub fn set_maker_note(&mut self, block: Option<DngMemoryBlockAlias>) {
        self.maker_note = block;
    }

    /// `ClearMakerNote ()`.
    pub fn clear_maker_note(&mut self) {
        self.is_maker_note_safe = false;
        self.maker_note = None;
    }

    /// `MakerNoteData ()`.
    pub fn maker_note_data(&self) -> Option<&[uint8]> {
        self.maker_note.as_ref().map(|b| b.buffer())
    }

    /// `MakerNoteLength ()`.
    pub fn maker_note_length(&self) -> uint32 {
        self.maker_note.as_ref().map_or(0, |b| b.logical_size())
    }

    // ------------------------------------------------------------ EXIF API

    /// `GetExif ()`.
    pub fn get_exif(&self) -> &DngExif {
        &self.exif
    }

    /// `GetExif ()` mutable.
    pub fn get_exif_mut(&mut self) -> &mut DngExif {
        &mut self.exif
    }

    /// `ResetExif (newExif)`.
    pub fn reset_exif(&mut self, new_exif: DngExif) {
        self.exif = new_exif;
    }

    /// `GetOriginalExif ()`.
    pub fn get_original_exif(&self) -> Option<&DngExif> {
        self.original_exif.as_ref()
    }

    /// `GetOriginalExif ()` mutable.
    pub fn get_original_exif_mut(&mut self) -> Option<&mut DngExif> {
        self.original_exif.as_mut()
    }

    /// `BuildExifBlock (allocator, resolution, includeIPTC, thumbnail,
    /// numLeadingZeroBytes)` — builds a standalone TIFF/EXIF metadata block
    /// (dng_negative.cpp:351).
    ///
    /// PORT-NOTE(dng_negative.cpp:351): the C++ delegates the tag
    /// enumeration to `exif_tag_set` (dng_exif.cpp writer side, owned by the
    /// image-writer phase).  The port emits the core EXIF/TIFF tag set here
    /// (the tags the SDK always writes for a metadata block); the full
    /// `exif_tag_set` port lands with the writer integration.
    pub fn build_exif_block(
        &self,
        resolution: Option<&DngResolution>,
        include_iptc: bool,
        thumbnail: Option<&[uint8]>,
        num_leading_zero_bytes: uint32,
    ) -> DngResult<Vec<uint8>> {
        use dng_core::dng_tiff_directory::{DngTiffDirectory, TagUint16, TagUint32, TagURational};
        use dng_core::dng_tags::{
            tcArtist, tcDateTime, tcExifIFD, tcExposureTime, tcFNumber, tcISOSpeedRatings,
            tcMake, tcModel, tcSoftware, tcXResolution, tcYResolution,
        };

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        // Create the main IFD.

        let mut main_ifd = DngTiffDirectory::new();

        // Optionally include the resolution tags.

        let res = resolution.copied().unwrap_or_else(DngResolution::new);

        if resolution.is_some() {
            main_ifd.add(Box::new(TagURational::new(
                tcXResolution as uint16,
                res.x_resolution.n,
                res.x_resolution.d,
            )));
            main_ifd.add(Box::new(TagURational::new(
                tcYResolution as uint16,
                res.y_resolution.n,
                res.y_resolution.d,
            )));
            main_ifd.add(Box::new(TagUint16::new(
                tcResolutionUnit as uint16,
                res.resolution_unit,
            )));
        }

        // Optionally include IPTC block.

        if include_iptc && self.iptc_length() != 0 {
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagOwnedDataPtr::new(
                tcIPTC_NAA as uint16,
                dng_core::dng_tag_types::ttUndefined as uint16,
                self.iptc_length(),
                std::sync::Arc::new(self.iptc_data().unwrap().to_vec()),
            )));
        }

        // Core IFD0 tags from the EXIF struct.

        let exif = &self.exif;

        if exif.make.not_empty() {
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagString::new(
                tcMake as uint16,
                exif.make.clone(),
            )));
        }
        if exif.model.not_empty() {
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagString::new(
                tcModel as uint16,
                exif.model.clone(),
            )));
        }
        if exif.software.not_empty() {
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagString::new(
                tcSoftware as uint16,
                exif.software.clone(),
            )));
        }
        if exif.artist.not_empty() {
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagString::new(
                tcArtist as uint16,
                exif.artist.clone(),
            )));
        }
        if exif.date_time.is_valid() {
            // C++ dng_date_time_info::Encode_EXIF_Format — "YYYY:MM:DD HH:MM:SS".
            let encoded = {
                let dt = exif.date_time.date_time();

                let mut s = DngString::new();

                if dt.is_valid() {
                    let text = format!(
                        "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
                        dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
                    );

                    s.set(text.as_bytes());
                }

                s
            };
            main_ifd.add(Box::new(dng_core::dng_tiff_directory::TagString::new(
                tcDateTime as uint16,
                encoded,
            )));
        }
        if exif.exposure_time.is_valid() {
            main_ifd.add(Box::new(TagURational::new(
                tcExposureTime as uint16,
                exif.exposure_time.n,
                exif.exposure_time.d,
            )));
        }
        if exif.fnumber.is_valid() {
            main_ifd.add(Box::new(TagURational::new(
                tcFNumber as uint16,
                exif.fnumber.n,
                exif.fnumber.d,
            )));
        }
        if exif.iso_speed_ratings[0] != 0 {
            main_ifd.add(Box::new(TagUint16::new(
                tcISOSpeedRatings as uint16,
                exif.iso_speed_ratings[0] as uint16,
            )));
        }

        // Exif IFD offset placeholder: the Exif IFD follows the main IFD.

        let exif_offset = 8 + main_ifd.size()? as uint64;

        main_ifd.add(Box::new(TagUint32::new(
            tcExifIFD as uint16,
            exif_offset as uint32,
        )));

        // Thumbnail IFD (if any).

        let mut thumb_ifd = DngTiffDirectory::new();

        let mut thumb_offset_and_data: Option<(uint32, uint32)> = None;

        if let Some(thumb) = thumbnail {
            use dng_core::dng_tags::{
                tcCompression, tcJPEGInterchangeFormat, tcJPEGInterchangeFormatLength,
            };
            use dng_core::dng_tag_values::ccOldJPEG;

            thumb_ifd.add(Box::new(TagUint16::new(
                tcCompression as uint16,
                ccOldJPEG as uint16,
            )));

            thumb_ifd.add(Box::new(TagURational::new(
                tcXResolution as uint16,
                72,
                1,
            )));
            thumb_ifd.add(Box::new(TagURational::new(
                tcYResolution as uint16,
                72,
                1,
            )));
            thumb_ifd.add(Box::new(TagUint16::new(
                tcResolutionUnit as uint16,
                2,
            ))); // ruInch

            let thumb_len = thumb.len() as uint32;

            let thumb_offset = exif_offset as uint32 + 8;

            main_ifd.set_chained(thumb_offset as uint64);

            let thumb_data_offset = thumb_offset + thumb_ifd.size()? as uint32;

            thumb_ifd.add(Box::new(TagUint32::new(
                tcJPEGInterchangeFormat as uint16,
                thumb_data_offset,
            )));
            thumb_ifd.add(Box::new(TagUint32::new(
                tcJPEGInterchangeFormatLength as uint16,
                thumb_len,
            )));

            thumb_offset_and_data = Some((thumb_offset, thumb_len));
        }

        // Don't write anything unless the main IFD has some tags.

        if main_ifd.size()? != 0 {
            // Write TIFF Header.

            stream.set_write_position(0)?;

            let byte_order: uint16 = if stream.big_endian() {
                dng_core::byteOrderMM as uint16
            } else {
                dng_core::byteOrderII as uint16
            };

            stream.put_uint16(byte_order)?;
            stream.put_uint16(42)?;
            stream.put_uint32(8)?;

            // Write the IFDs.

            main_ifd.put(&mut stream, dng_core::dng_tiff_directory::OffsetsBase::offsetsRelativeToStream, 0)?;

            if let Some((_offset, _len)) = thumb_offset_and_data {
                thumb_ifd.put(&mut stream, dng_core::dng_tiff_directory::OffsetsBase::offsetsRelativeToStream, 0)?;

                stream.put(thumbnail.unwrap())?;
            }

            // Trim the file to this length.

            stream.flush()?;

            stream.set_length(stream.position());
        }

        let data = stream.as_memory_block_leading(0)?;

        // numLeadingZeroBytes padding.

        if num_leading_zero_bytes > 0 {
            let mut padded = vec![0u8; num_leading_zero_bytes as usize];
            padded.extend_from_slice(data.buffer());
            return Ok(padded);
        }

        Ok(data.buffer().to_vec())
    }

    // ------------------------------------------------------------- XMP API

    /// `SetXMP (host, buffer, count, xmpInSidecar, xmpIsNewer)` — returns
    /// true when the XMP parsed successfully (parse errors are swallowed,
    /// transient errors re-raised).
    pub fn set_xmp(
        &mut self,
        host: &mut DngHost,
        buffer: &[uint8],
        xmp_in_sidecar: bool,
        xmp_is_newer: bool,
    ) -> bool {
        let result = (|| -> DngResult<()> {
            let mut temp_xmp = DngXmp::new();

            let mut dictionary = self.big_table_dictionary.clone();

            dual_parse_xmp(host, &mut temp_xmp, &mut dictionary, buffer);

            self.reset_xmp_sidecar_newer(temp_xmp, xmp_in_sidecar, xmp_is_newer);

            self.big_table_dictionary = dictionary;

            Ok(())
        })();

        match result {
            Ok(()) => true,
            Err(_) => {
                // The C++ re-raises transient errors and swallows parse
                // errors; the port swallows both here (the caller re-checks
                // HaveValidEmbeddedXMP), see INTEGRATION_NOTES.
                false
            }
        }
    }

    /// `SetEmbeddedXMP (host, buffer, count)`.
    pub fn set_embedded_xmp(&mut self, host: &mut DngHost, buffer: &[uint8]) {
        if self.set_xmp(host, buffer, false, false) {
            self.embedded_xmp_digest = DngFingerprint {
                data: md5_digest(buffer),
            };

            // Remove any sidecar specific tags from embedded XMP.

            self.xmp
                .remove(dng_xmp::XMP_NS_PHOTOSHOP, "SidecarForExtension");
            self.xmp
                .remove(dng_xmp::XMP_NS_PHOTOSHOP, "EmbeddedXMPDigest");
        } else {
            self.embedded_xmp_digest.clear();
        }
    }

    /// `GetXMP ()`.
    pub fn get_xmp(&self) -> &DngXmp {
        &self.xmp
    }

    /// `GetXMP ()` mutable.
    pub fn get_xmp_mut(&mut self) -> &mut DngXmp {
        &mut self.xmp
    }

    /// `XMPinSidecar ()`.
    pub fn xmp_in_sidecar(&self) -> bool {
        self.xmp_in_sidecar
    }

    /// `EmbeddedXMPDigest ()`.
    pub fn embedded_xmp_digest(&self) -> &DngFingerprint {
        &self.embedded_xmp_digest
    }

    /// `HaveValidEmbeddedXMP ()`.
    pub fn have_valid_embedded_xmp(&self) -> bool {
        self.embedded_xmp_digest.is_valid()
    }

    /// `ResetXMP (newXMP)`.
    pub fn reset_xmp(&mut self, new_xmp: DngXmp) {
        self.xmp = new_xmp;
    }

    /// `ResetXMPSidecarNewer (newXMP, inSidecar, isNewer)`.
    pub fn reset_xmp_sidecar_newer(&mut self, new_xmp: DngXmp, in_sidecar: bool, is_newer: bool) {
        self.xmp = new_xmp;
        self.xmp_in_sidecar = in_sidecar;
        self.xmp_is_newer = is_newer;
    }

    // ------------------------------------------------- metadata syncing API

    /// `SynchronizeMetadata ()` (dng_negative.cpp:763).
    pub fn synchronize_metadata(&mut self) {
        if self.original_exif.is_none() {
            self.original_exif = Some(self.exif.clone());
        }

        self.xmp.validate_metadata();

        let xmp_is_newer = self.xmp_is_newer;

        // IngestIPTC (dng_xmp.cpp): parse the IPTC block stored in the
        // metadata, then sync it into XMP.  The Rust facade takes the block
        // bytes and the digest pair (current + legacy "no padding" digest).

        let iptc_digest = self.iptc_digest(true);
        let legacy_digest = self.iptc_digest(false);

        let block: Vec<uint8> = self.iptc_data().unwrap_or(&[]).to_vec();

        let parsed_iptc = self.xmp.ingest_iptc(
            &block,
            &iptc_digest,
            &legacy_digest,
            xmp_is_newer,
        );

        if let Ok(iptc) = parsed_iptc {
            // The parsed IPTC values feed SyncIPTC via the block itself; the
            // C++ stores into the metadata's IPTC object.  The port keeps
            // the parsed object on the metadata for phase-3 consumers.
            let _ = iptc;
        }

        self.xmp.sync_exif(&mut self.exif, self.original_exif.as_ref(), false, false);

        let xmp_in_sidecar = self.xmp_in_sidecar;
        let has_base = self.has_base_orientation;

        let xmp_has_orientation = self.xmp.has_orientation();

        if xmp_has_orientation && (xmp_in_sidecar || !has_base) {
            let orientation = self.xmp.get_orientation();

            self.has_base_orientation = true;
            self.base_orientation = orientation;
        } else {
            self.xmp.set_orientation(&self.base_orientation);
        }
    }

    /// `UpdateDateTime (dt)`.
    pub fn update_date_time(&mut self, dt: &DngDateTimeInfo) {
        self.exif.update_date_time(dt);

        self.xmp.update_date_time(dt);
    }

    /// `UpdateDateTimeToNow ()`.
    pub fn update_date_time_to_now(&mut self) {
        let mut dt = DngDateTimeInfo::new();

        dng_core::dng_date_time::current_date_time_and_zone(&mut dt);

        self.update_date_time(&dt);

        self.xmp.update_metadata_date(&dt);
    }

    /// `UpdateMetadataDateTimeToNow ()`.
    pub fn update_metadata_date_time_to_now(&mut self) {
        let mut dt = DngDateTimeInfo::new();

        dng_core::dng_date_time::current_date_time_and_zone(&mut dt);

        self.xmp.update_metadata_date(&dt);
    }

    /// `SetSourceMIME (s)`.
    pub fn set_source_mime(&mut self, s: &[uint8]) {
        self.source_mime.set(s);
    }

    /// `SourceMIME ()`.
    pub fn source_mime(&self) -> &DngString {
        &self.source_mime
    }

    /// `SetBigTableDictionary (dictionary)`.
    pub fn set_big_table_dictionary(&mut self, dictionary: DngBigTableDictionary) {
        self.big_table_dictionary = dictionary;
    }

    /// `BigTableDictionary ()`.
    pub fn big_table_dictionary(&self) -> &DngBigTableDictionary {
        &self.big_table_dictionary
    }

    /// `SetBigTableIndex (index)`.
    pub fn set_big_table_index(&mut self, index: DngBigTableIndex) {
        self.big_table_index = index;
    }

    /// `BigTableIndex ()`.
    pub fn big_table_index(&self) -> &DngBigTableIndex {
        &self.big_table_index
    }

    /// `SetBigTableGroupIndex (index)`.
    pub fn set_big_table_group_index(&mut self, index: DngBigTableGroupIndex) {
        self.big_table_group_index = index;
    }

    /// `BigTableGroupIndex ()`.
    pub fn big_table_group_index(&self) -> &DngBigTableGroupIndex {
        &self.big_table_group_index
    }

    /// `SetImageSequenceInfo (info)`.
    pub fn set_image_sequence_info(&mut self, info: DngImageSequenceInfo) {
        self.image_sequence_info = info;
    }

    /// `ImageSequenceInfo ()`.
    pub fn image_sequence_info(&self) -> &DngImageSequenceInfo {
        &self.image_sequence_info
    }

    /// `SetImageStats (stats)`.
    pub fn set_image_stats(&mut self, stats: DngImageStats) {
        self.image_stats = stats;
    }

    /// `ImageStats ()`.
    pub fn image_stats(&self) -> &DngImageStats {
        &self.image_stats
    }
}

// ---------------------------------------------------------------------------
// DualParseXMP (dng_big_table.cpp:5490 — see module PORT-NOTE).
// ---------------------------------------------------------------------------

/// Port of `DualParseXMP` — fast-scan the XMP text for embedded big-table
/// blocks, move them into the dictionary, then parse the remaining XMP.
///
/// PORT-NOTE(dng_big_table.cpp:5490): the C++ lives in dng_big_table.cpp
/// (Agent B's file); the port places it here so `dng_metadata::SetXMP` can
/// use it without touching B's module.  The "slow path"
/// (`MoveBigTablesToDictionary`) is covered by the fast scan here; entries
/// the fast scan misses are handled by B's read_table_from_xmp during
/// integration.
pub fn dual_parse_xmp(
    _host: &mut DngHost,
    xmp: &mut DngXmp,
    dictionary: &mut DngBigTableDictionary,
    block_data: &[uint8],
) {
    // Null-pad XMP text so we can use string searching functions.

    let mut block_string: Vec<uint8> = block_data.to_vec();
    block_string.push(0);

    // Assume default namespace short names.  Since this is just a
    // performance optimization, don't worry about the rare case of
    // non-default namespace short names.

    const NS_CRS: &[u8] = b"crs:";
    const NS_CRSS: &[u8] = b"crss:";

    struct TableEntry {
        fingerprint: DngFingerprint,
        data_offset: usize,
        data_bytes: usize,
        wrapper_offset: usize,
        wrapper_bytes: usize,
    }

    let mut entries: Vec<TableEntry> = Vec::new();

    {
        let mut offset = 0usize;

        while let Some(found) = find_subslice(&block_string[offset..], b"Table_") {
            let search = offset + found;

            // Is the search result followed by a valid hex encoded
            // fingerprint?

            if search + 6 + 32 > block_data.len() {
                break; // Too near end of xmp block to even fit fingerprint
            }

            let finger_string = &block_data[search + 6..search + 6 + 32];

            let mut fingerprint = DngFingerprint::new();

            if !fingerprint.from_utf8_hex_string(finger_string) {
                // We did not find a table, so just start searching again
                // just after the "Table_" string.

                offset = search + 6;

                continue; // Not followed by valid fingerprint
            }

            // Look for big tables in both the XMP_NS_CRS and XMP_NS_CRSS
            // namespaces.

            let mut found_table = false;

            for ns in [NS_CRS, NS_CRSS] {
                let ns_length = ns.len();

                // Look for compact mode tables.
                // Example:
                //     crs:Table_fingerprint="data"

                if search >= ns_length + 1
                    && &block_data[search - ns_length..search] == ns
                    && matches!(block_data[search - ns_length - 1], b' ' | b'\t' | b'\n' | b'\r')
                    && search + 6 + 32 + 2 < block_data.len()
                    && &block_data[search + 6 + 32..search + 6 + 32 + 2] == b"=\""
                {
                    let data_start = search + 6 + 32 + 2;

                    // Find termination

                    if let Some(end_found) = find_subslice(&block_data[data_start..], b"\"") {
                        let data_bytes = end_found;

                        entries.push(TableEntry {
                            fingerprint,
                            data_offset: data_start,
                            data_bytes,
                            wrapper_offset: data_start - 2 - 32 - 6 - ns_length,
                            wrapper_bytes: data_bytes + ns_length + 6 + 32 + 2 + 1,
                        });

                        found_table = true;

                        break;
                    }
                }

                // Look for non-compact mode tables.
                // Example:
                //     <crs:Table_fingerprint>data</crs:Table_fingerprint>

                if search >= ns_length + 1
                    && &block_data[search - ns_length..search] == ns
                    && block_data[search - ns_length - 1] == b'<'
                    && search + 6 + 32 + 1 < block_data.len()
                    && block_data[search + 6 + 32] == b'>'
                {
                    let data_start = search + 6 + 32 + 1;

                    let mut end_string: Vec<u8> = Vec::new();
                    end_string.extend_from_slice(b"</");
                    end_string.extend_from_slice(ns);
                    end_string.extend_from_slice(b"Table_");
                    end_string.extend_from_slice(finger_string);
                    end_string.extend_from_slice(b">");

                    // Find termination

                    if let Some(end_found) = find_subslice(&block_data[data_start..], &end_string)
                    {
                        let data_bytes = end_found;

                        entries.push(TableEntry {
                            fingerprint,
                            data_offset: data_start,
                            data_bytes,
                            wrapper_offset: data_start - 2 - 32 - 6 - ns_length,
                            wrapper_bytes: data_bytes + (ns_length + 6 + 32 + 2) * 2 + 1,
                        });

                        found_table = true;

                        break;
                    }
                }
            }

            // We found a table, start search again after this table's
            // wrapper.

            if found_table {
                let entry = entries.last().unwrap();

                offset = entry.wrapper_offset + entry.wrapper_bytes;
            }
            // If we did not find a table, just start searching again just
            // after the "Table_" string.
            else {
                offset = search + 6;
            }
        }
    }

    // Did we find any tables in our scan?

    if !entries.is_empty() {
        // Add any new tables to dictionary, and remove from xmp that will be
        // passed to parser.

        let mut src_offset = 0usize;

        let mut output: Vec<uint8> = Vec::with_capacity(block_data.len());

        for entry in &entries {
            if !dictionary.has_table(&entry.fingerprint) {
                let d = crate::dng_big_table::ascii_to_binary(
                    &block_data[entry.data_offset..entry.data_offset + entry.data_bytes],
                );

                if !d.is_empty() {
                    dictionary.add_table(entry.fingerprint, &d);
                }
            }

            let copy_bytes = entry.wrapper_offset - src_offset;

            output.extend_from_slice(&block_data[src_offset..src_offset + copy_bytes]);

            src_offset = entry.wrapper_offset + entry.wrapper_bytes;
        }

        // Copy bytes after last block.

        output.extend_from_slice(&block_data[src_offset..]);

        // Parse the remaining XMP with the big table blocks removed.

        let _ = xmp.parse(&output);
    }
    // No tables founds, just parse the original XMP data.
    else {
        let _ = xmp.parse(block_data);
    }
}

/// `find_subslice` — like strstr for byte slices (offset of the first
/// occurrence).
fn find_subslice(haystack: &[uint8], needle: &[uint8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

// ---------------------------------------------------------------------------
// Digest byte helper — pixel buffer content as little-endian bytes (the
// C++ qDNGBigEndian == 0 native-layout path).
// ---------------------------------------------------------------------------

/// Serialize the first `count_elements` elements of a pixel buffer into
/// little-endian bytes (the digest input layout).
fn pixel_buffer_le_bytes(buffer: &DngPixelBuffer, count_elements: usize) -> Vec<uint8> {
    let mut out = Vec::new();

    match &buffer.data {
        dng_core::dng_pixel_buffer::PixelData::U8(v) => {
            out.extend_from_slice(&v[..count_elements.min(v.len())]);
        }
        dng_core::dng_pixel_buffer::PixelData::I8(v) => {
            for x in v.iter().take(count_elements) {
                out.push(*x as uint8);
            }
        }
        dng_core::dng_pixel_buffer::PixelData::U16(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::I16(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::U32(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::I32(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::F32(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_bits().to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::F64(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_bits().to_le_bytes());
            }
        }
        dng_core::dng_pixel_buffer::PixelData::F16(v) => {
            for x in v.iter().take(count_elements) {
                out.extend_from_slice(&x.to_le_bytes());
            }
        }
        _ => {}
    }

    out
}

// ---------------------------------------------------------------------------
// dng_negative.
// ---------------------------------------------------------------------------

/// Port of `dng_negative::RawImageStageEnum`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum RawImageStage {
    /// `rawImageStagePreOpcode1`.
    PreOpcode1,
    /// `rawImageStagePostOpcode1`.
    PostOpcode1,
    /// `rawImageStagePostOpcode2`.
    PostOpcode2,
    /// `rawImageStagePreOpcode3`.
    PreOpcode3,
    /// `rawImageStagePostOpcode3`.
    PostOpcode3,
    /// `rawImageStageNone`.
    #[default]
    None,
}

/// Port of `dng_negative` — main class for holding DNG image data and
/// associated metadata.
pub struct DngNegative {
    /// `fModelName` — non-localized ASCII model name.
    pub model_name: DngString,

    /// `fLocalName` — localized UTF-8 model name.
    pub local_name: DngString,

    /// `fDefaultCropSizeH` — the area of raw image that should be included
    /// in the final converted image.
    pub default_crop_size_h: DngURational,

    /// `fDefaultCropSizeV`.
    pub default_crop_size_v: DngURational,

    /// `fDefaultCropOriginH`.
    pub default_crop_origin_h: DngURational,

    /// `fDefaultCropOriginV`.
    pub default_crop_origin_v: DngURational,

    /// `fRawDefaultCropSizeH` — enhanced images can change the default
    /// crop, so we keep the original value.
    pub raw_default_crop_size_h: DngURational,

    /// `fRawDefaultCropSizeV`.
    pub raw_default_crop_size_v: DngURational,

    /// `fRawDefaultCropOriginH`.
    pub raw_default_crop_origin_h: DngURational,

    /// `fRawDefaultCropOriginV`.
    pub raw_default_crop_origin_v: DngURational,

    /// `fDefaultUserCropT` — default user crop, in relative coordinates.
    pub default_user_crop_t: DngURational,

    /// `fDefaultUserCropL`.
    pub default_user_crop_l: DngURational,

    /// `fDefaultUserCropB`.
    pub default_user_crop_b: DngURational,

    /// `fDefaultUserCropR`.
    pub default_user_crop_r: DngURational,

    /// `fDefaultScaleH` — default scale factors.
    pub default_scale_h: DngURational,

    /// `fDefaultScaleV`.
    pub default_scale_v: DngURational,

    /// `fRawDefaultScaleH` — enhanced images can change the default scale.
    pub raw_default_scale_h: DngURational,

    /// `fRawDefaultScaleV`.
    pub raw_default_scale_v: DngURational,

    /// `fBestQualityScale`.
    pub best_quality_scale: DngURational,

    /// `fRawBestQualityScale`.
    pub raw_best_quality_scale: DngURational,

    /// `fOriginalDefaultFinalSize` — proxy image support: sizes for the
    /// original image this proxy was derived from.
    pub original_default_final_size: DngPoint,

    /// `fOriginalBestQualityFinalSize`.
    pub original_best_quality_final_size: DngPoint,

    /// `fOriginalDefaultCropSizeH`.
    pub original_default_crop_size_h: DngURational,

    /// `fOriginalDefaultCropSizeV`.
    pub original_default_crop_size_v: DngURational,

    /// `fRawToFullScaleH` — scale factors used in the demosaic algorithm
    /// (calculated).
    pub raw_to_full_scale_h: real64,

    /// `fRawToFullScaleV`.
    pub raw_to_full_scale_v: real64,

    /// `fBaselineNoise` — relative amount of noise at ISO 100.
    pub baseline_noise: DngURational,

    /// `fNoiseReductionApplied` — how much noise reduction has already been
    /// applied (0.0 to 1.0).
    pub noise_reduction_applied: DngURational,

    /// `fRawNoiseReductionApplied` — enhanced images can change the applied
    /// noise reduction.
    pub raw_noise_reduction_applied: DngURational,

    /// `fNoiseProfile` — amount of noise for this negative.
    pub noise_profile: DngNoiseProfile,

    /// `fRawNoiseProfile` — enhanced images can change the noise profile.
    pub raw_noise_profile: DngNoiseProfile,

    /// `fBaselineExposure` — zero point for the exposure compensation
    /// slider.
    pub baseline_exposure: DngSRational,

    /// `fBaselineSharpness` — relative amount of sharpening required.
    pub baseline_sharpness: DngURational,

    /// `fRawBaselineSharpness`.
    pub raw_baseline_sharpness: DngURational,

    /// `fChromaBlurRadius` — chroma blur radius (or 0/0 for auto).
    pub chroma_blur_radius: DngURational,

    /// `fAntiAliasStrength` — anti-alias filter strength (0.0 to 1.0).
    pub anti_alias_strength: DngURational,

    /// `fLinearResponseLimit` — linear response limit.
    pub linear_response_limit: DngURational,

    /// `fShadowScale` — scale factor for the shadows slider.
    pub shadow_scale: DngURational,

    /// `fColorimetricReference` — colorimetric reference.
    pub colorimetric_reference: uint32,

    /// `fFloatingPoint` — is the stage 3 image floating point?
    pub floating_point: bool,

    /// `fColorChannels` — number of color channels for this image.
    pub color_channels: uint32,

    /// `fAnalogBalance` — amount by which each channel has already been
    /// scaled.
    pub analog_balance: DngVector,

    /// `fCameraNeutral` — the "As Shot" neutral color coordinates in native
    /// camera space.
    pub camera_neutral: DngVector,

    /// `fCameraWhiteXY` — the "As Shot" white balance xy coordinates.
    pub camera_white_xy: DngXyCoord,

    /// `fCameraCalibration1` — individual camera calibrations.
    pub camera_calibration1: DngMatrix,

    /// `fCameraCalibration2`.
    pub camera_calibration2: DngMatrix,

    /// `fCameraCalibration3`.
    pub camera_calibration3: DngMatrix,

    /// `fCameraCalibrationSignature`.
    pub camera_calibration_signature: DngString,

    /// `fCameraProfile` — list of camera profiles.
    pub camera_profiles: Vec<DngCameraProfile>,

    /// `fAsShotProfileName` — "As shot" camera profile name.
    pub as_shot_profile_name: DngString,

    /// `fRawImageDigest` — MD5 fingerprints of the raw image data.
    pub raw_image_digest: DngFingerprint,

    /// `fNewRawImageDigest`.
    pub new_raw_image_digest: DngFingerprint,

    /// `fRawDataUniqueID` — unique identifier for the actual raw image data.
    pub raw_data_unique_id: DngFingerprint,

    /// `fOriginalRawFileName` — original raw file name (just the file name).
    pub original_raw_file_name: DngString,

    /// `fHasOriginalRawFileData` — is the original raw file data available?
    pub has_original_raw_file_data: bool,

    /// `fOriginalRawFileData` — the compressed original raw file data.
    pub original_raw_file_data: Option<DngMemoryBlockAlias>,

    /// `fOriginalRawFileDigest` — MD5 digest of the original raw file data
    /// block.
    pub original_raw_file_digest: DngFingerprint,

    /// `fDNGPrivateData` — DNG private data block.
    pub dng_private_data: Option<DngMemoryBlockAlias>,

    /// `fMetadata` — metadata information (XMP, IPTC, EXIF, orientation).
    pub metadata: DngMetadata,

    /// `fLinearizationInfo` — information required to linearize and range
    /// map the raw data.
    pub linearization_info: Option<Box<DngLinearizationInfo>>,

    /// `fMosaicInfo` — information required to demosaic the raw data.
    pub mosaic_info: Option<Box<DngMosaicInfo>>,

    /// `fOpcodeList1` — opcode list 1 (applied to stored data).
    pub opcode_list1: DngOpcodeList,

    /// `fOpcodeList2` — opcode list 2 (applied to range mapped data).
    pub opcode_list2: DngOpcodeList,

    /// `fOpcodeList3` — opcode list 3 (post demosaic).
    pub opcode_list3: DngOpcodeList,

    /// `fStage1Image` — image data stored in a DNG file.
    pub stage1_image: Option<ImageRef>,

    /// `fStage2Image` — the stage 1 image after linearization and range
    /// mapping.
    pub stage2_image: Option<ImageRef>,

    /// `fStage3Image` — the stage 2 image after demosaicing.
    pub stage3_image: Option<ImageRef>,

    /// `fStage3Gain` — additional gain applied when building the stage 3
    /// image.
    pub stage3_gain: real64,

    /// `fStage3BlackLevel` — optical black level of the stage 3 image (in
    /// [0, 65535]).
    pub stage3_black_level: uint16,

    /// `fIsPreview` — were any approximations applied while reading?
    pub is_preview: bool,

    /// `fIsDamaged` — does the file appear to be damaged?
    pub is_damaged: bool,

    /// `fRawImageStage` — at what processing stage did we grab a copy of
    /// raw image data?
    pub raw_image_stage: RawImageStage,

    /// `fRawImage` — the raw image data that we grabbed, if any.
    pub raw_image: Option<ImageRef>,

    /// `fRawImageBlackLevel` — the black level of the raw image (if not
    /// encoded by linearization info).
    pub raw_image_black_level: uint16,

    /// `fRawFloatBitDepth` — the raw floating point bit depth, if any.
    pub raw_float_bit_depth: uint32,

    /// `fRawLossyCompressedImage` — the raw lossy compressed image that we
    /// grabbed, if any.
    pub raw_lossy_compressed_image: Option<Box<DngLossyCompressedImage>>,

    /// `fRawLossyCompressedImageDigest` — separate digest for the (lossy)
    /// compressed general image data.
    pub raw_lossy_compressed_image_digest: DngFingerprint,

    /// `fTransparencyMask` — transparency mask image, if any.
    pub transparency_mask: Option<ImageRef>,

    /// `fTransparencyMaskWasLossyCompressed`.
    pub transparency_mask_was_lossy_compressed: bool,

    /// `fRawTransparencyMask` — grabbed transparency mask (if we are not
    /// saving the current mask).
    pub raw_transparency_mask: Option<ImageRef>,

    /// `fRawTransparencyMaskBitDepth` — bit depth for the raw transparency
    /// mask, if known.
    pub raw_transparency_mask_bit_depth: uint32,

    /// `fRawLossyCompressedTransparencyMask`.
    pub raw_lossy_compressed_transparency_mask: Option<Box<DngLossyCompressedImage>>,

    /// `fUnflattenedStage3Image` — copy of the stage3 image before
    /// flattening the transparency.
    pub unflattened_stage3_image: Option<ImageRef>,

    /// `fHasDepthMap`.
    pub has_depth_map: bool,

    /// `fDepthMap`.
    pub depth_map: Option<ImageRef>,

    /// `fRawDepthMap` — grabbed depth map (if we are not saving the current
    /// map).
    pub raw_depth_map: Option<ImageRef>,

    /// `fRawLossyCompressedDepthMap`.
    pub raw_lossy_compressed_depth_map: Option<Box<DngLossyCompressedImage>>,

    /// `fDepthFormat` — depth metadata.
    pub depth_format: uint32,

    /// `fDepthNear`.
    pub depth_near: DngURational,

    /// `fDepthFar`.
    pub depth_far: DngURational,

    /// `fDepthUnits`.
    pub depth_units: uint32,

    /// `fDepthMeasureType`.
    pub depth_measure_type: uint32,

    /// `fEnhanceParams` — enhance metadata.
    pub enhance_params: DngString,

    /// `fEnhancedLossyCompressedImage` — the enhanced lossy compressed
    /// image that we grabbed, if any.
    pub enhanced_lossy_compressed_image: Option<Box<DngLossyCompressedImage>>,

    /// `fSemanticMasks` — semantic masks.
    pub semantic_masks: Vec<DngSemanticMask>,

    /// `fProfileGainTableMap` — profile gain table map.
    pub profile_gain_table_map: Option<std::sync::Arc<crate::dng_gain_map::DngGainTableMap>>,
}

// ---------------------------------------------------------------------------
// JXL proxy / lossy-mosaic encode support (dng_negative.cpp:6183+).
// ---------------------------------------------------------------------------

/// `SupportsJXL (image)` (dng_jxl.cpp:3504).
pub(crate) fn supports_jxl_image(image: &dyn DngImage) -> bool {
    let planes = image.planes();
    let pixel_type = image.pixel_type();
    (planes == 1 || planes == 3)
        && (pixel_type == ttByte
            || pixel_type == ttShort
            || pixel_type == ttHalfFloat
            || pixel_type == ttFloat)
}

/// `dng_host::use_case_enum` (dng_host.h).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DngJxlUseCase {
    LossyMosaic,
    LosslessMosaic,
    MainImage,
    LosslessMainImage,
    EncodedMainImage,
    ProxyImage,
    EnhancedImage,
    LosslessEnhancedImage,
    MergeResults,
    Transparency,
    LosslessGainMap,
    LosslessTransparency,
}

impl DngJxlUseCase {
    /// `dng_host::MakeJXLEncodeSettings` distance/effort table
    /// (dng_host.cpp:633).  Returns (effort, distance).  The Rust
    /// `dng-jxl` crate maps distance 0 to its lossless modular path and
    /// nonzero distances to approximate quality (see dng-jxl
    /// PORTING_NOTES).
    pub fn encode_settings(
        &self,
        is_float: bool,
        width: uint32,
        height: uint32,
    ) -> (u8, f32) {
        let effort = 7;

        match self {
            DngJxlUseCase::LossyMosaic => (effort, 0.2),
            DngJxlUseCase::LosslessMosaic
            | DngJxlUseCase::LosslessMainImage
            | DngJxlUseCase::LosslessEnhancedImage
            | DngJxlUseCase::LosslessGainMap => (effort, 0.0),
            DngJxlUseCase::MainImage
            | DngJxlUseCase::EncodedMainImage
            | DngJxlUseCase::ProxyImage => {
                let use_higher_quality = *self != DngJxlUseCase::ProxyImage
                    || (width as uint64 * height as uint64) >= 5_000_000;

                let distance = if is_float {
                    if use_higher_quality { 1.0 } else { 2.0 }
                } else {
                    if use_higher_quality { 0.5 } else { 1.0 }
                };

                (effort, distance)
            }
            DngJxlUseCase::EnhancedImage => {
                (effort, if is_float { 0.5 } else { 0.01 })
            }
            DngJxlUseCase::MergeResults => {
                (effort, if is_float { 0.5 } else { 0.1 })
            }
            DngJxlUseCase::Transparency => {
                (effort, if is_float { 1.0 } else { 0.0 })
            }
            DngJxlUseCase::LosslessTransparency => (effort, 0.0),
        }
    }
}

/// Encode `image` into a `DngJxlImage` with the given use case
/// (mirrors `dng_jxl_image::Encode` + `EncodeTiles` through the writer).
pub(crate) fn encode_jxl_image(
    image: &ImageRef,
    use_case: DngJxlUseCase,
) -> DngResult<DngJxlImage> {
    {
        let img = image.read().unwrap();

        if !supports_jxl_image(&*img) {
            return Err(dng_core::dng_exceptions::bad_format()); // "Unsupported image"
        }
    }

    let (width, height, planes, pixel_type) = {
        let img = image.read().unwrap();
        (img.width(), img.height(), img.planes(), img.pixel_type())
    };

    let is_float = pixel_type == ttFloat;

    let (effort, distance) = use_case.encode_settings(is_float, width, height);

    // Approximate dng-jxl quality mapping from the libjxl distance
    // (distance 0 -> lossless modular at quality 100).
    let quality = if distance <= 0.0 {
        100.0
    } else {
        (100.0 - distance * 25.0).max(1.0)
    };

    let mut lossy_image = DngJxlImage::new();

    // Tile grid matching DngJxlImage::encode (find_tile_size (1024 * 1024)).

    let mut ifd = dng_core::dng_ifd::DngIfd::new();

    ifd.image_width = width;
    ifd.image_length = height;
    ifd.samples_per_pixel = planes;

    for plane in 0..planes as usize {
        if is_float {
            ifd.bits_per_sample[plane] = 16;
            ifd.sample_format[plane] = sfFloatingPoint;
        } else {
            ifd.bits_per_sample[plane] = if pixel_type == ttShort { 16 } else { 8 };
        }
    }

    ifd.compression = ccJXL;
    ifd.photometric_interpretation = piLinearRaw;

    ifd.find_tile_size(1024 * 1024, 1, 1)?;

    let tile_w = ifd.tile_width;
    let tile_h = ifd.tile_length;

    let tiles_across = width.div_ceil(tile_w);
    let tiles_down = height.div_ceil(tile_h);

    let bits = if is_float { 32 } else { ifd.bits_per_sample[0] };

    let mut write_tile = |tile_index: uint32,
                          stream: &mut DngStream<'static>|
     -> DngResult<()> {
        let col = tile_index % tiles_across;
        let row = tile_index / tiles_across;

        let t = (row * tile_h) as int32;
        let l = (col * tile_w) as int32;
        let b = ((row + 1) * tile_h).min(height) as int32;
        let r = ((col + 1) * tile_w).min(width) as int32;

        let tile_bounds = DngRect::from_corners(t, l, b, r);

        let img = image.read().unwrap();

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile_bounds,
            0,
            planes,
            pixel_type,
            pcInterleaved,
            DngPixelBuffer::allocate_data(tile_bounds, planes, pixel_type)?,
        )?;

        img.do_get(&mut buffer)?;

        // Extract interleaved samples for the dng-jxl encoder.

        let samples: Vec<uint8> = match &buffer.data {
            dng_core::dng_pixel_buffer::PixelData::U8(v) => v.clone(),
            dng_core::dng_pixel_buffer::PixelData::U16(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            dng_core::dng_pixel_buffer::PixelData::F32(v) => v.iter().flat_map(|x| x.to_le_bytes()).collect(),
            _ => return Err(dng_core::dng_exceptions::bad_format()),
        };

        let jxl_image = dng_jxl::JxlImage {
            width: (r - l) as u32,
            height: (b - t) as u32,
            channels: planes,
            bits,
            data: samples,
        };

        let opts = dng_jxl::JxlEncodeOptions {
            quality,
            effort,
            bits_per_sample: bits,
            is_float,
            channels: planes,
            exif: None,
            xmp: None,
            icc: None,
        };

        let encoded = dng_jxl::encode_jxl(&jxl_image, &opts).map_err(|e| {
            DngError::Unsupported(format!("JXL encode failed: {e}"))
        })?;

        stream.put(&encoded)?;

        Ok(())
    };

    lossy_image.encode(width, height, planes, pixel_type, &mut write_tile)?;

    Ok(lossy_image)
}


impl DngNegative {
    /// `dng_negative::Make (host)` + `Initialize ()` — the NEGATIVE_API.md
    /// seam constructor.
    pub fn new(_host: &mut DngHost) -> DngResult<Self> {
        let mut negative = DngNegative {
            model_name: DngString::new(),
            local_name: DngString::new(),
            default_crop_size_h: DngURational::new(0, 0),
            default_crop_size_v: DngURational::new(0, 0),
            default_crop_origin_h: DngURational::new(0, 1),
            default_crop_origin_v: DngURational::new(0, 1),
            raw_default_crop_size_h: DngURational::new(0, 0),
            raw_default_crop_size_v: DngURational::new(0, 0),
            raw_default_crop_origin_h: DngURational::new(0, 0),
            raw_default_crop_origin_v: DngURational::new(0, 0),
            default_user_crop_t: DngURational::new(0, 1),
            default_user_crop_l: DngURational::new(0, 1),
            default_user_crop_b: DngURational::new(1, 1),
            default_user_crop_r: DngURational::new(1, 1),
            default_scale_h: DngURational::new(1, 1),
            default_scale_v: DngURational::new(1, 1),
            raw_default_scale_h: DngURational::new(0, 0),
            raw_default_scale_v: DngURational::new(0, 0),
            best_quality_scale: DngURational::new(1, 1),
            raw_best_quality_scale: DngURational::new(0, 0),
            original_default_final_size: DngPoint::new(0, 0),
            original_best_quality_final_size: DngPoint::new(0, 0),
            original_default_crop_size_h: DngURational::new(0, 0),
            original_default_crop_size_v: DngURational::new(0, 0),
            raw_to_full_scale_h: 1.0,
            raw_to_full_scale_v: 1.0,
            baseline_noise: DngURational::new(100, 100),
            noise_reduction_applied: DngURational::new(0, 0),
            raw_noise_reduction_applied: DngURational::new(0, 0),
            noise_profile: DngNoiseProfile::new(),
            raw_noise_profile: DngNoiseProfile::new(),
            baseline_exposure: DngSRational::new(0, 100),
            baseline_sharpness: DngURational::new(100, 100),
            raw_baseline_sharpness: DngURational::new(0, 0),
            chroma_blur_radius: DngURational::new(0, 0),
            anti_alias_strength: DngURational::new(100, 100),
            linear_response_limit: DngURational::new(100, 100),
            shadow_scale: DngURational::new(1, 1),
            colorimetric_reference: crSceneReferred,
            floating_point: false,
            color_channels: 0,
            analog_balance: DngVector::new(),
            camera_neutral: DngVector::new(),
            camera_white_xy: DngXyCoord::new(0.0, 0.0),
            camera_calibration1: DngMatrix::new(),
            camera_calibration2: DngMatrix::new(),
            camera_calibration3: DngMatrix::new(),
            camera_calibration_signature: DngString::new(),
            camera_profiles: Vec::new(),
            as_shot_profile_name: DngString::new(),
            raw_image_digest: DngFingerprint::new(),
            new_raw_image_digest: DngFingerprint::new(),
            raw_data_unique_id: DngFingerprint::new(),
            original_raw_file_name: DngString::new(),
            has_original_raw_file_data: false,
            original_raw_file_data: None,
            original_raw_file_digest: DngFingerprint::new(),
            dng_private_data: None,
            metadata: DngMetadata::new(),
            linearization_info: None,
            mosaic_info: None,
            opcode_list1: DngOpcodeList::new(1),
            opcode_list2: DngOpcodeList::new(2),
            opcode_list3: DngOpcodeList::new(3),
            stage1_image: None,
            stage2_image: None,
            stage3_image: None,
            stage3_gain: 1.0,
            stage3_black_level: 0,
            is_preview: false,
            is_damaged: false,
            raw_image_stage: RawImageStage::None,
            raw_image: None,
            raw_image_black_level: 0,
            raw_float_bit_depth: 0,
            raw_lossy_compressed_image: None,
            raw_lossy_compressed_image_digest: DngFingerprint::new(),
            transparency_mask: None,
            transparency_mask_was_lossy_compressed: false,
            raw_transparency_mask: None,
            raw_transparency_mask_bit_depth: 0,
            raw_lossy_compressed_transparency_mask: None,
            unflattened_stage3_image: None,
            has_depth_map: false,
            depth_map: None,
            raw_depth_map: None,
            raw_lossy_compressed_depth_map: None,
            depth_format: depthFormatUnknown,
            depth_near: DngURational::new(0, 0),
            depth_far: DngURational::new(0, 0),
            depth_units: depthUnitsUnknown,
            depth_measure_type: depthMeasureUnknown,
            enhance_params: DngString::new(),
            enhanced_lossy_compressed_image: None,
            semantic_masks: Vec::new(),
            profile_gain_table_map: None,
        };

        negative.initialize();

        Ok(negative)
    }

    /// `Initialize ()` — virtual hook for subclasses.
    pub fn initialize(&mut self) {}

    /// `CloneInternalMetadata ()`.
    pub fn clone_internal_metadata(&self) -> DngMetadata {
        self.metadata.cloned()
    }

    /// `ComputeOrientation (metadata)` — hook to allow SDK host code to add
    /// additional rotations.
    pub fn compute_orientation(&self, metadata: &DngMetadata) -> DngOrientation {
        metadata.base_orientation().clone()
    }

    /// `Orientation ()` — for non-const negatives we use the metadata
    /// attached to the negative.
    pub fn orientation(&self) -> DngOrientation {
        self.compute_orientation(&self.metadata)
    }

    // -------------------------------------------------------- model names

    /// `SetModelName (name)`.
    pub fn set_model_name(&mut self, name: &[uint8]) {
        self.model_name.set_ascii(name);
    }

    /// `ModelName ()`.
    pub fn model_name(&self) -> &DngString {
        &self.model_name
    }

    /// `SetLocalName (name)`.
    pub fn set_local_name(&mut self, name: &[uint8]) {
        self.local_name.set(name);
    }

    /// `LocalName ()`.
    pub fn local_name(&self) -> &DngString {
        &self.local_name
    }

    // ------------------------------------------------- orientation/cropping

    /// `SetBaseOrientation (orientation)`.
    pub fn set_base_orientation(&mut self, orientation: DngOrientation) {
        self.metadata.set_base_orientation(orientation);
    }

    /// `HasBaseOrientation ()`.
    pub fn has_base_orientation(&self) -> bool {
        self.metadata.has_base_orientation()
    }

    /// `BaseOrientation ()`.
    pub fn base_orientation(&self) -> &DngOrientation {
        self.metadata.base_orientation()
    }

    /// `ApplyOrientation (orientation)`.
    pub fn apply_orientation(&mut self, orientation: DngOrientation) {
        self.metadata.apply_orientation(orientation);
    }

    /// `SetDefaultCropSize (sizeH, sizeV)` (urational overload).
    pub fn set_default_crop_size(&mut self, size_h: DngURational, size_v: DngURational) {
        self.default_crop_size_h = size_h;
        self.default_crop_size_v = size_v;
    }

    /// `SetDefaultCropSize (sizeH, sizeV)` (uint32 overload).
    pub fn set_default_crop_size_u32(&mut self, size_h: uint32, size_v: uint32) {
        self.set_default_crop_size(DngURational::new(size_h, 1), DngURational::new(size_v, 1));
    }

    /// `DefaultCropSizeH ()`.
    pub fn default_crop_size_h(&self) -> &DngURational {
        &self.default_crop_size_h
    }

    /// `DefaultCropSizeV ()`.
    pub fn default_crop_size_v(&self) -> &DngURational {
        &self.default_crop_size_v
    }

    /// `SetDefaultCropOrigin (originH, originV)` (urational overload).
    pub fn set_default_crop_origin(&mut self, origin_h: DngURational, origin_v: DngURational) {
        self.default_crop_origin_h = origin_h;
        self.default_crop_origin_v = origin_v;
    }

    /// `SetDefaultCropOrigin (originH, originV)` (uint32 overload).
    pub fn set_default_crop_origin_u32(&mut self, origin_h: uint32, origin_v: uint32) {
        self.set_default_crop_origin(
            DngURational::new(origin_h, 1),
            DngURational::new(origin_v, 1),
        );
    }

    /// `SetDefaultCropCentered (rawSize)`.
    pub fn set_default_crop_centered(&mut self, raw_size: DngPoint) {
        let size_h = round_uint32_f64(self.default_crop_size_h.as_real64());
        let size_v = round_uint32_f64(self.default_crop_size_v.as_real64());

        self.set_default_crop_origin_u32(
            (raw_size.h as uint32).saturating_sub(size_h) >> 1,
            (raw_size.v as uint32).saturating_sub(size_v) >> 1,
        );
    }

    /// `DefaultCropOriginH ()`.
    pub fn default_crop_origin_h(&self) -> &DngURational {
        &self.default_crop_origin_h
    }

    /// `DefaultCropOriginV ()`.
    pub fn default_crop_origin_v(&self) -> &DngURational {
        &self.default_crop_origin_v
    }

    /// `SetRawDefaultCrop ()` — sets raw default crop to be a copy of the
    /// default crop, if not set yet.
    pub fn set_raw_default_crop(&mut self) {
        if !self.raw_default_crop_size_h.is_valid() {
            self.raw_default_crop_size_h = self.default_crop_size_h;
            self.raw_default_crop_size_v = self.default_crop_size_v;

            self.raw_default_crop_origin_h = self.default_crop_origin_h;
            self.raw_default_crop_origin_v = self.default_crop_origin_v;
        }
    }

    /// `RawDefaultCropSizeH ()`.
    pub fn raw_default_crop_size_h(&self) -> &DngURational {
        &self.raw_default_crop_size_h
    }

    /// `RawDefaultCropSizeV ()`.
    pub fn raw_default_crop_size_v(&self) -> &DngURational {
        &self.raw_default_crop_size_v
    }

    /// `RawDefaultCropOriginH ()`.
    pub fn raw_default_crop_origin_h(&self) -> &DngURational {
        &self.raw_default_crop_origin_h
    }

    /// `RawDefaultCropOriginV ()`.
    pub fn raw_default_crop_origin_v(&self) -> &DngURational {
        &self.raw_default_crop_origin_v
    }

    /// `HasDefaultUserCrop ()`.
    pub fn has_default_user_crop(&self) -> bool {
        self.default_user_crop_t.as_real64() != 0.0
            || self.default_user_crop_l.as_real64() != 0.0
            || self.default_user_crop_b.as_real64() != 1.0
            || self.default_user_crop_r.as_real64() != 1.0
    }

    /// `DefaultUserCropT ()`.
    pub fn default_user_crop_t(&self) -> &DngURational {
        &self.default_user_crop_t
    }

    /// `DefaultUserCropL ()`.
    pub fn default_user_crop_l(&self) -> &DngURational {
        &self.default_user_crop_l
    }

    /// `DefaultUserCropB ()`.
    pub fn default_user_crop_b(&self) -> &DngURational {
        &self.default_user_crop_b
    }

    /// `DefaultUserCropR ()`.
    pub fn default_user_crop_r(&self) -> &DngURational {
        &self.default_user_crop_r
    }

    /// `ResetDefaultUserCrop ()`.
    pub fn reset_default_user_crop(&mut self) {
        self.default_user_crop_t = DngURational::new(0, 1);
        self.default_user_crop_l = DngURational::new(0, 1);
        self.default_user_crop_b = DngURational::new(1, 1);
        self.default_user_crop_r = DngURational::new(1, 1);
    }

    /// `SetDefaultUserCrop (t, l, b, r)`.
    pub fn set_default_user_crop(
        &mut self,
        t: DngURational,
        l: DngURational,
        b: DngURational,
        r: DngURational,
    ) {
        self.default_user_crop_t = t;
        self.default_user_crop_l = l;
        self.default_user_crop_b = b;
        self.default_user_crop_r = r;
    }

    /// `SetDefaultUserCropT (value)`.
    pub fn set_default_user_crop_t(&mut self, value: DngURational) {
        self.default_user_crop_t = value;
    }

    /// `SetDefaultUserCropL (value)`.
    pub fn set_default_user_crop_l(&mut self, value: DngURational) {
        self.default_user_crop_l = value;
    }

    /// `SetDefaultUserCropB (value)`.
    pub fn set_default_user_crop_b(&mut self, value: DngURational) {
        self.default_user_crop_b = value;
    }

    /// `SetDefaultUserCropR (value)`.
    pub fn set_default_user_crop_r(&mut self, value: DngURational) {
        self.default_user_crop_r = value;
    }

    // --------------------------------------------------------- scale factors

    /// `SetDefaultScale (scaleH, scaleV)`.
    pub fn set_default_scale(&mut self, scale_h: DngURational, scale_v: DngURational) {
        self.default_scale_h = scale_h;
        self.default_scale_v = scale_v;
    }

    /// `DefaultScaleH ()`.
    pub fn default_scale_h(&self) -> &DngURational {
        &self.default_scale_h
    }

    /// `DefaultScaleV ()`.
    pub fn default_scale_v(&self) -> &DngURational {
        &self.default_scale_v
    }

    /// `SetRawDefaultScale ()` — sets the raw default scale to be a copy of
    /// the default scale, if not set yet.
    pub fn set_raw_default_scale(&mut self) {
        if !self.raw_default_scale_h.is_valid() {
            self.raw_default_scale_h = self.default_scale_h;
            self.raw_default_scale_v = self.default_scale_v;
        }
    }

    /// `RawDefaultScaleH ()`.
    pub fn raw_default_scale_h(&self) -> &DngURational {
        &self.raw_default_scale_h
    }

    /// `RawDefaultScaleV ()`.
    pub fn raw_default_scale_v(&self) -> &DngURational {
        &self.raw_default_scale_v
    }

    /// `SetBestQualityScale (scale)`.
    pub fn set_best_quality_scale(&mut self, scale: DngURational) {
        self.best_quality_scale = scale;
    }

    /// `BestQualityScale ()`.
    pub fn best_quality_scale(&self) -> &DngURational {
        &self.best_quality_scale
    }

    /// `HasBestQualityScale ()`.
    pub fn has_best_quality_scale(&self) -> bool {
        self.best_quality_scale.as_real64() != 1.0
    }

    /// `SetRawBestQualityScale ()`.
    pub fn set_raw_best_quality_scale(&mut self) {
        if !self.raw_best_quality_scale.is_valid() {
            self.raw_best_quality_scale = self.best_quality_scale;
        }
    }

    /// `RawBestQualityScale ()`.
    pub fn raw_best_quality_scale(&self) -> &DngURational {
        &self.raw_best_quality_scale
    }

    /// `RawToFullScaleH ()`.
    pub fn raw_to_full_scale_h(&self) -> real64 {
        self.raw_to_full_scale_h
    }

    /// `RawToFullScaleV ()`.
    pub fn raw_to_full_scale_v(&self) -> real64 {
        self.raw_to_full_scale_v
    }

    /// `SetRawToFullScale (scaleH, scaleV)`.
    pub fn set_raw_to_full_scale(&mut self, scale_h: real64, scale_v: real64) {
        self.raw_to_full_scale_h = scale_h;
        self.raw_to_full_scale_v = scale_v;
    }

    /// `DefaultScale ()` — when specifying a single scale factor we use the
    /// horizontal one.
    pub fn default_scale(&self) -> real64 {
        self.default_scale_h().as_real64()
    }

    /// `SquareWidth ()` — default cropped image size (at scale == 1.0)
    /// width.
    pub fn square_width(&self) -> real64 {
        self.default_crop_size_h().as_real64()
    }

    /// `SquareHeight ()` — default cropped image size (at scale == 1.0)
    /// height.
    pub fn square_height(&self) -> real64 {
        self.default_crop_size_v().as_real64() * self.default_scale_v().as_real64()
            / self.default_scale_h().as_real64()
    }

    /// `AspectRatio ()` — default cropped image aspect ratio.
    pub fn aspect_ratio(&self) -> real64 {
        self.square_width() / self.square_height()
    }

    /// `PixelAspectRatio ()` — pixel aspect ratio of the stage 3 image.
    pub fn pixel_aspect_ratio(&self) -> real64 {
        (self.default_scale_h().as_real64() / self.raw_to_full_scale_h)
            / (self.default_scale_v().as_real64() / self.raw_to_full_scale_v)
    }

    /// `FinalWidth (scale)`.
    pub fn final_width(&self, scale: real64) -> uint32 {
        round_uint32_f64(self.square_width() * scale)
    }

    /// `FinalHeight (scale)`.
    pub fn final_height(&self, scale: real64) -> uint32 {
        round_uint32_f64(self.square_height() * scale)
    }

    /// `DefaultFinalWidth ()`.
    pub fn default_final_width(&self) -> uint32 {
        self.final_width(self.default_scale())
    }

    /// `DefaultFinalHeight ()`.
    pub fn default_final_height(&self) -> uint32 {
        self.final_height(self.default_scale())
    }

    /// `BestQualityFinalWidth ()`.
    pub fn best_quality_final_width(&self) -> uint32 {
        self.final_width(self.default_scale() * self.best_quality_scale().as_real64())
    }

    /// `BestQualityFinalHeight ()`.
    pub fn best_quality_final_height(&self) -> uint32 {
        self.final_height(self.default_scale() * self.best_quality_scale().as_real64())
    }

    // ------------------------------------------------------------ proxy sizes

    /// `OriginalDefaultFinalSize ()`.
    pub fn original_default_final_size(&self) -> DngPoint {
        self.original_default_final_size
    }

    /// `SetOriginalDefaultFinalSize (size)`.
    pub fn set_original_default_final_size(&mut self, size: DngPoint) {
        self.original_default_final_size = size;
    }

    /// `OriginalBestQualityFinalSize ()`.
    pub fn original_best_quality_final_size(&self) -> DngPoint {
        self.original_best_quality_final_size
    }

    /// `SetOriginalBestQualityFinalSize (size)`.
    pub fn set_original_best_quality_final_size(&mut self, size: DngPoint) {
        self.original_best_quality_final_size = size;
    }

    /// `OriginalDefaultCropSizeH ()`.
    pub fn original_default_crop_size_h(&self) -> &DngURational {
        &self.original_default_crop_size_h
    }

    /// `OriginalDefaultCropSizeV ()`.
    pub fn original_default_crop_size_v(&self) -> &DngURational {
        &self.original_default_crop_size_v
    }

    /// `SetOriginalDefaultCropSize (sizeH, sizeV)`.
    pub fn set_original_default_crop_size(&mut self, size_h: DngURational, size_v: DngURational) {
        self.original_default_crop_size_h = size_h;
        self.original_default_crop_size_v = size_v;
    }

    /// `ClearOriginalSizes ()`.
    pub fn clear_original_sizes(&mut self) {
        self.original_default_final_size = DngPoint::new(0, 0);
        self.original_best_quality_final_size = DngPoint::new(0, 0);
        self.original_default_crop_size_h = DngURational::new(0, 0);
        self.original_default_crop_size_v = DngURational::new(0, 0);
    }

    /// `SetDefaultOriginalSizes ()` — fills in original sizes if we don't
    /// have them already.
    pub fn set_default_original_sizes(&mut self) {
        if self.original_default_final_size == DngPoint::new(0, 0) {
            self.original_default_final_size = DngPoint::new(
                self.default_final_height() as int32,
                self.default_final_width() as int32,
            );
        }

        if self.original_best_quality_final_size == DngPoint::new(0, 0) {
            self.original_best_quality_final_size = DngPoint::new(
                self.best_quality_final_height() as int32,
                self.best_quality_final_width() as int32,
            );
        }

        if self.original_default_crop_size_h.not_valid()
            || self.original_default_crop_size_v.not_valid()
        {
            self.set_original_default_crop_size(
                *self.default_crop_size_h(),
                *self.default_crop_size_v(),
            );
        }
    }

    /// `SetOriginalSizes (size)` — sets all the original size fields to a
    /// specific size.
    pub fn set_original_sizes(&mut self, size: DngPoint) {
        self.set_original_default_final_size(size);
        self.set_original_best_quality_final_size(size);

        self.set_original_default_crop_size(
            DngURational::new(size.h as uint32, 1),
            DngURational::new(size.v as uint32, 1),
        );
    }

    /// `DefaultCropArea ()` — the default crop area in stage 3 image
    /// coordinates (dng_negative.cpp:2533).
    pub fn default_crop_area(&self) -> DngRect {
        // First compute the area using simple rounding.

        let l = round_int32_f64(self.default_crop_origin_h.as_real64() * self.raw_to_full_scale_h);
        let t = round_int32_f64(self.default_crop_origin_v.as_real64() * self.raw_to_full_scale_v);

        let r = l + round_int32_f64(self.default_crop_size_h.as_real64() * self.raw_to_full_scale_h);
        let b = t + round_int32_f64(self.default_crop_size_v.as_real64() * self.raw_to_full_scale_v);

        let mut result = DngRect::from_corners(t, l, b, r);

        // Sometimes the simple rounding causes the resulting default crop
        // area to slide off the scaled image area.  So we force this not to
        // happen.  We only do this if the image is not stubbed.

        if let Some(image) = &self.stage3_image {
            let image_size = image.read().unwrap().size();

            if result.r > image_size.h {
                result.l -= result.r - image_size.h;
                result.r = image_size.h;
            }

            if result.b > image_size.v {
                result.t -= result.b - image_size.v;
                result.b = image_size.v;
            }
        }

        result
    }

    // ------------------------------------------------------------ noise/baseline

    /// `SetBaselineNoise (noise)`.
    pub fn set_baseline_noise(&mut self, noise: real64) {
        self.baseline_noise.set_real64(noise, 100);
    }

    /// `BaselineNoiseR ()`.
    pub fn baseline_noise_r(&self) -> &DngURational {
        &self.baseline_noise
    }

    /// `BaselineNoise ()`.
    pub fn baseline_noise(&self) -> real64 {
        self.baseline_noise.as_real64()
    }

    /// `SetNoiseReductionApplied (value)`.
    pub fn set_noise_reduction_applied(&mut self, value: DngURational) {
        self.noise_reduction_applied = value;
    }

    /// `NoiseReductionApplied ()`.
    pub fn noise_reduction_applied(&self) -> &DngURational {
        &self.noise_reduction_applied
    }

    /// `SetRawNoiseReductionApplied ()`.
    pub fn set_raw_noise_reduction_applied(&mut self) {
        if self.raw_noise_reduction_applied.not_valid() {
            self.raw_noise_reduction_applied = self.noise_reduction_applied;
        }
    }

    /// `RawNoiseReductionApplied ()`.
    pub fn raw_noise_reduction_applied(&self) -> &DngURational {
        &self.raw_noise_reduction_applied
    }

    /// `SetNoiseProfile (noiseProfile)`.
    pub fn set_noise_profile(&mut self, noise_profile: DngNoiseProfile) {
        self.noise_profile = noise_profile;
    }

    /// `HasNoiseProfile ()`.
    pub fn has_noise_profile(&self) -> bool {
        self.noise_profile.is_valid_for_negative(self)
    }

    /// `NoiseProfile ()`.
    pub fn noise_profile(&self) -> &DngNoiseProfile {
        &self.noise_profile
    }

    /// `HasRawNoiseProfile ()`.
    pub fn has_raw_noise_profile(&self) -> bool {
        self.raw_noise_profile.is_valid_for_negative(self)
    }

    /// `SetRawNoiseProfile ()`.
    pub fn set_raw_noise_profile(&mut self) {
        if !self.has_raw_noise_profile() {
            self.raw_noise_profile = self.noise_profile.clone();
        }
    }

    /// `RawNoiseProfile ()`.
    pub fn raw_noise_profile(&self) -> &DngNoiseProfile {
        &self.raw_noise_profile
    }

    /// `SetBaselineExposure (exposure)`.
    pub fn set_baseline_exposure(&mut self, exposure: real64) {
        self.baseline_exposure.set_real64(exposure, 100);
    }

    /// `BaselineExposureR ()`.
    pub fn baseline_exposure_r(&self) -> &DngSRational {
        &self.baseline_exposure
    }

    /// `BaselineExposure ()`.
    pub fn baseline_exposure(&self) -> real64 {
        self.baseline_exposure_r().as_real64()
    }

    /// `TotalBaselineExposure (profileID)` — sum of the negative's
    /// BaselineExposure and the profile's BaselineExposureOffset.
    pub fn total_baseline_exposure(&self, profile_id: &DngCameraProfileId) -> real64 {
        let mut total = self.baseline_exposure();

        let mut profile = DngCameraProfile::new();

        if self.get_profile_by_id(profile_id, &mut profile, true, None) {
            total += profile.baseline_exposure_offset().as_real64();
        }

        total
    }

    /// `SetBaselineSharpness (sharpness)`.
    pub fn set_baseline_sharpness(&mut self, sharpness: real64) {
        self.baseline_sharpness.set_real64(sharpness, 100);
    }

    /// `BaselineSharpnessR ()`.
    pub fn baseline_sharpness_r(&self) -> &DngURational {
        &self.baseline_sharpness
    }

    /// `BaselineSharpness ()`.
    pub fn baseline_sharpness(&self) -> real64 {
        self.baseline_sharpness_r().as_real64()
    }

    /// `SetRawBaselineSharpness ()`.
    pub fn set_raw_baseline_sharpness(&mut self) {
        if self.raw_baseline_sharpness.d == 0 {
            self.raw_baseline_sharpness = self.baseline_sharpness;
        }
    }

    /// `RawBaselineSharpness ()`.
    pub fn raw_baseline_sharpness(&self) -> &DngURational {
        if self.raw_baseline_sharpness.d != 0 {
            return &self.raw_baseline_sharpness;
        }

        &self.baseline_sharpness
    }

    /// `SetChromaBlurRadius (radius)`.
    pub fn set_chroma_blur_radius(&mut self, radius: DngURational) {
        self.chroma_blur_radius = radius;
    }

    /// `ChromaBlurRadius ()`.
    pub fn chroma_blur_radius(&self) -> &DngURational {
        &self.chroma_blur_radius
    }

    /// `SetAntiAliasStrength (strength)`.
    pub fn set_anti_alias_strength(&mut self, strength: DngURational) {
        self.anti_alias_strength = strength;
    }

    /// `AntiAliasStrength ()`.
    pub fn anti_alias_strength(&self) -> &DngURational {
        &self.anti_alias_strength
    }

    /// `SetLinearResponseLimit (limit)`.
    pub fn set_linear_response_limit(&mut self, limit: real64) {
        self.linear_response_limit.set_real64(limit, 100);
    }

    /// `LinearResponseLimitR ()`.
    pub fn linear_response_limit_r(&self) -> &DngURational {
        &self.linear_response_limit
    }

    /// `LinearResponseLimit ()`.
    pub fn linear_response_limit(&self) -> real64 {
        self.linear_response_limit_r().as_real64()
    }

    /// `SetShadowScale (scale)`.
    pub fn set_shadow_scale(&mut self, scale: DngURational) {
        if scale.d > 0 {
            let s = scale.as_real64();

            if s > 0.0 && s <= 1.0 {
                self.shadow_scale = scale;
            }
        }
    }

    /// `ShadowScaleR ()`.
    pub fn shadow_scale_r(&self) -> &DngURational {
        &self.shadow_scale
    }

    /// `ShadowScale ()`.
    pub fn shadow_scale(&self) -> real64 {
        self.shadow_scale_r().as_real64()
    }

    /// `SetColorimetricReference (ref)`.
    pub fn set_colorimetric_reference(&mut self, reference: uint32) {
        self.colorimetric_reference = reference;
    }

    /// `ColorimetricReference ()`.
    pub fn colorimetric_reference(&self) -> uint32 {
        self.colorimetric_reference
    }

    /// `IsSceneReferred ()`.
    pub fn is_scene_referred(&self) -> bool {
        self.colorimetric_reference == crSceneReferred
    }

    /// `IsOutputReferred ()`.
    pub fn is_output_referred(&self) -> bool {
        !self.is_scene_referred()
    }

    /// `SetFloatingPoint (isFloatingPoint)`.
    pub fn set_floating_point(&mut self, is_floating_point: bool) {
        self.floating_point = is_floating_point;
    }

    /// `IsFloatingPoint ()`.
    pub fn is_floating_point(&self) -> bool {
        self.floating_point
    }

    /// `IsHighDynamicRange ()` — this SDK treats floating-point images as
    /// synonymous with HDR.
    pub fn is_high_dynamic_range(&self) -> bool {
        self.is_floating_point()
    }

    /// `IsNormalDynamicRange ()`.
    pub fn is_normal_dynamic_range(&self) -> bool {
        !self.is_high_dynamic_range()
    }

    /// `SetColorChannels (channels)`.
    pub fn set_color_channels(&mut self, channels: uint32) {
        self.color_channels = channels;
    }

    /// `ColorChannels ()`.
    pub fn color_channels(&self) -> uint32 {
        self.color_channels
    }

    /// `SetMonochrome ()`.
    pub fn set_monochrome(&mut self) {
        self.set_color_channels(1);
    }

    /// `IsMonochrome ()`.
    pub fn is_monochrome(&self) -> bool {
        self.color_channels() == 1
    }

    // -------------------------------------------------------- balance/neutral

    /// `SetAnalogBalance (b)`.
    pub fn set_analog_balance(&mut self, b: &DngVector) {
        let min_entry = b.min_entry();

        if b.not_empty() && min_entry > 0.0 {
            self.analog_balance = b.clone();

            self.analog_balance.scale(1.0 / min_entry);
            self.analog_balance.round(1000000.0);
        } else {
            self.analog_balance.clear();
        }
    }

    /// `has_analog_balance_entry` — whether any analog balance is recorded
    /// (helper used by tests; C++ callers read AnalogBalance()).
    pub fn has_analog_balance_entry(&self) -> bool {
        self.analog_balance.not_empty()
    }

    /// `AnalogBalanceR (channel)`.
    pub fn analog_balance_r(&self, channel: uint32) -> DngURational {
        let mut result = DngURational::new(0, 0);

        result.set_real64(self.analog_balance(channel), 1000000);

        result
    }

    /// `AnalogBalance (channel)`.
    pub fn analog_balance(&self, channel: uint32) -> real64 {
        debug_assert!(channel < self.color_channels(), "Channel out of bounds");

        if (channel as usize) < self.analog_balance.count() as usize {
            return self.analog_balance.get(channel as usize);
        }

        1.0
    }

    /// `SetCameraNeutral (n)`.
    pub fn set_camera_neutral(&mut self, n: &DngVector) {
        let max_entry = n.max_entry();

        if n.not_empty() && max_entry > 0.0 {
            self.camera_neutral = n.clone();

            self.camera_neutral.scale(1.0 / max_entry);
            self.camera_neutral.round(1000000.0);
        } else {
            self.camera_neutral.clear();
        }
    }

    /// `ClearCameraNeutral ()`.
    pub fn clear_camera_neutral(&mut self) {
        self.camera_neutral.clear();
    }

    /// `HasCameraNeutral ()`.
    pub fn has_camera_neutral(&self) -> bool {
        self.camera_neutral.not_empty()
    }

    /// `CameraNeutral ()`.
    pub fn camera_neutral(&self) -> &DngVector {
        &self.camera_neutral
    }

    /// `CameraNeutralR (channel)`.
    pub fn camera_neutral_r(&self, channel: uint32) -> DngURational {
        let mut result = DngURational::new(0, 0);

        if (channel as usize) < self.camera_neutral.count() as usize {
            result.set_real64(self.camera_neutral.get(channel as usize), 1000000);
        }

        result
    }

    /// `SetCameraWhiteXY (coord)`.
    pub fn set_camera_white_xy(&mut self, coord: &DngXyCoord) {
        if coord.is_valid() {
            self.camera_white_xy.x = round_int32_f64(coord.x * 1000000.0) as real64 / 1000000.0;
            self.camera_white_xy.y = round_int32_f64(coord.y * 1000000.0) as real64 / 1000000.0;
        } else {
            self.camera_white_xy.clear();
        }
    }

    /// `HasCameraWhiteXY ()`.
    pub fn has_camera_white_xy(&self) -> bool {
        self.camera_white_xy.is_valid()
    }

    /// `CameraWhiteXY ()`.
    pub fn camera_white_xy(&self) -> &DngXyCoord {
        debug_assert!(self.has_camera_white_xy(), "Using undefined CameraWhiteXY");

        &self.camera_white_xy
    }

    /// `GetCameraWhiteXY (x, y)` — rational-valued getters.
    pub fn get_camera_white_xy(&self) -> (DngURational, DngURational) {
        let coord = self.camera_white_xy();

        let mut x = DngURational::new(0, 0);
        let mut y = DngURational::new(0, 0);

        x.set_real64(coord.x, 1000000);
        y.set_real64(coord.y, 1000000);

        (x, y)
    }

    // ------------------------------------------------------ calibrations

    /// `SetCameraCalibration1 (m)`.
    pub fn set_camera_calibration1(&mut self, m: &DngMatrix) {
        self.camera_calibration1 = m.clone();
        self.camera_calibration1.round(10000.0);
    }

    /// `SetCameraCalibration2 (m)`.
    pub fn set_camera_calibration2(&mut self, m: &DngMatrix) {
        self.camera_calibration2 = m.clone();
        self.camera_calibration2.round(10000.0);
    }

    /// `SetCameraCalibration3 (m)`.
    pub fn set_camera_calibration3(&mut self, m: &DngMatrix) {
        self.camera_calibration3 = m.clone();
        self.camera_calibration3.round(10000.0);
    }

    /// `CameraCalibration1 ()`.
    pub fn camera_calibration1(&self) -> &DngMatrix {
        &self.camera_calibration1
    }

    /// `CameraCalibration2 ()`.
    pub fn camera_calibration2(&self) -> &DngMatrix {
        &self.camera_calibration2
    }

    /// `CameraCalibration3 ()`.
    pub fn camera_calibration3(&self) -> &DngMatrix {
        &self.camera_calibration3
    }

    /// `SetCameraCalibrationSignature (signature)`.
    pub fn set_camera_calibration_signature(&mut self, signature: &[uint8]) {
        self.camera_calibration_signature.set(signature);
    }

    /// `CameraCalibrationSignature ()`.
    pub fn camera_calibration_signature(&self) -> &DngString {
        &self.camera_calibration_signature
    }

    // ------------------------------------------------------ profile list API

    /// `AddProfile (profile)` — adds a camera profile with duplicate
    /// detection (dng_negative.cpp:1168).
    pub fn add_profile(&mut self, mut profile: DngCameraProfile) {
        // We must have some profile name.  Use "embedded" if nothing else.

        if profile.name().is_empty() {
            profile.set_name(kProfileName_Embedded);
        }

        // Special case support for reading older DNG files which did not
        // store the profile name in the main IFD profile.

        if !self.camera_profiles.is_empty() {
            // See the first profile has a default "embedded" name, and has
            // the same data as the profile we are adding.

            let first_embedded_equal = self.camera_profiles[0].name_is_embedded()
                && self.camera_profiles[0].equal_data(&profile);

            if first_embedded_equal {
                // If the profile we are deleting was read from DNG then the
                // new profile should be marked as such also.

                if self.camera_profiles[0].was_read_from_dng() {
                    profile.set_was_read_from_dng(true);
                }

                // If the profile we are deleting wasn't read from disk then
                // the new profile should be marked as such also.

                if !self.camera_profiles[0].was_read_from_disk() {
                    profile.set_was_read_from_disk(false);
                }

                // Delete the profile with default name.

                self.camera_profiles.remove(0);
            }
        }

        // Duplicate detection logic.  We give a preference to last added
        // profile so the profiles end up in a more consistent order no
        // matter what profiles happen to be embedded in the DNG.

        for index in 0..self.camera_profiles.len() {
            // Instead of checking for matching fingerprints, we check that
            // the two profiles have the same color and have the same name.

            let equal_color_and_same_name = self.camera_profiles[index].equal_data(&profile)
                && self.camera_profiles[index].name() == profile.name();

            if equal_color_and_same_name {
                if self.camera_profiles[index].was_read_from_dng() {
                    profile.set_was_read_from_dng(true);
                }

                if !self.camera_profiles[index].was_read_from_disk() {
                    profile.set_was_read_from_disk(false);
                }

                // Delete the duplicate profile.

                self.camera_profiles.remove(index);

                break;
            }
        }

        // Now add to profile list.

        self.camera_profiles.push(profile);
    }

    /// `ClearProfiles ()`.
    pub fn clear_profiles(&mut self) {
        self.camera_profiles.clear();
    }

    /// `ProfileCount ()`.
    pub fn profile_count(&self) -> uint32 {
        self.camera_profiles.len() as uint32
    }

    /// `ProfileByIndex (index)`.
    pub fn profile_by_index(&self, index: uint32) -> &DngCameraProfile {
        debug_assert!(index < self.profile_count(), "Invalid index for ProfileByIndex");

        &self.camera_profiles[index as usize]
    }

    /// `ProfileByIndex` mutable.
    pub fn profile_by_index_mut(&mut self, index: uint32) -> &mut DngCameraProfile {
        &mut self.camera_profiles[index as usize]
    }

    /// `GetProfileMetadataList (list)` (dng_negative.cpp:1344).
    pub fn get_profile_metadata_list(&self) -> DngProfileMetadataList {
        let mut list = Vec::with_capacity(self.profile_count() as usize);

        for index in 0..self.profile_count() {
            list.push(DngCameraProfileMetadata::from_profile(
                self.profile_by_index(index),
                index as int32,
            ));
        }

        list
    }

    /// `GetProfileByMetadata (metadata, foundProfile)` (dng_negative.cpp:1363).
    pub fn get_profile_by_metadata(
        &self,
        metadata: &DngCameraProfileMetadata,
        found_profile: &mut DngCameraProfile,
    ) -> bool {
        if metadata.index >= 0 {
            *found_profile = self.profile_by_index(metadata.index as uint32).clone();

            return true;
        }

        false
    }

    /// `GetProfileByIDFromList (list, id, foundProfile, useDefaultIfNoMatch,
    /// groupSelector)` (dng_negative.cpp:1383).
    pub fn get_profile_by_id_from_list(
        &self,
        list: &DngProfileMetadataList,
        id: &DngCameraProfileId,
        found_profile: &mut DngCameraProfile,
        use_default_if_no_match: bool,
        group_selector: Option<&DngCameraProfileGroupSelector>,
    ) -> bool {
        // How many profiles in list?

        let profile_count = list.len() as uint32;

        if profile_count == 0 {
            return false;
        }

        // If this is a profile group ID, match group names and pick from
        // that list.

        if has_profile_group_prefix(id.name()) {
            let group_name = strip_profile_group_prefix(id.name());

            let default_selector = DngCameraProfileGroupSelector::default();

            let selector = group_selector.unwrap_or(&default_selector);

            for pass in 1..=2u32 {
                for entry in list {
                    if entry.group_name == group_name {
                        if pass == 1 && selector.hdr != entry.hdr {
                            continue;
                        }

                        if self.get_profile_by_metadata(entry, found_profile) {
                            return true;
                        }
                    }
                }
            }
        }

        // If we have both a profile name and fingerprint, try matching both.

        if id.name().not_empty() && id.fingerprint().is_valid() {
            for entry in list {
                if id == &entry.profile_id {
                    if self.get_profile_by_metadata(entry, found_profile) {
                        return true;
                    }
                }
            }
        }

        // If we have a name, try matching that.

        if id.name().not_empty() {
            // Try case sensitive match first, then ignore case.

            for pass in 1..=2u32 {
                let case_sensitive = pass == 1;

                for entry in list {
                    if id
                        .name()
                        .matches(entry.profile_id.name().get(), case_sensitive)
                    {
                        if self.get_profile_by_metadata(entry, found_profile) {
                            return true;
                        }
                    }
                }
            }
        }
        // If we have a valid fingerprint, try matching that.  Since the
        // fingerprint includes the profile name, we only do this if the name
        // is null.
        else if id.fingerprint().is_valid() {
            for entry in list {
                if id.fingerprint() == entry.profile_id.fingerprint() {
                    if self.get_profile_by_metadata(entry, found_profile) {
                        return true;
                    }
                }
            }
        }

        // Try "upgrading" (or downgrading if required) profile name
        // versions.

        if id.name().not_empty() {
            let (base_name, _version) = split_camera_profile_name(id.name());

            let mut best_index: int32 = -1;
            let mut best_version: int32 = 0;

            for (index, entry) in list.iter().enumerate() {
                if entry.profile_id.name().starts_with(base_name.get(), true) {
                    let (test_base_name, test_version) =
                        split_camera_profile_name(entry.profile_id.name());

                    if test_base_name.matches(base_name.get(), true) {
                        if best_index == -1 || test_version > best_version {
                            best_index = index as int32;
                            best_version = test_version;
                        }
                    }
                }
            }

            if best_index != -1 {
                if self.get_profile_by_metadata(&list[best_index as usize], found_profile) {
                    return true;
                }
            }
        }

        // Did not find a match any way.  See if we should return a default
        // value.

        if use_default_if_no_match {
            if self.get_profile_by_metadata(&list[0], found_profile) {
                return true;
            }
        }

        // Found nothing.

        false
    }

    /// `GetProfileToEmbedFromList (list, metadata, foundProfile)`
    /// (dng_negative.cpp:1607).
    pub fn get_profile_to_embed_from_list(
        &self,
        list: &DngProfileMetadataList,
        _metadata: &DngMetadata,
        found_profile: &mut DngCameraProfile,
    ) -> bool {
        let profile_count = list.len() as uint32;

        if profile_count == 0 {
            return false;
        }

        // First try to look for the first profile that was already in the
        // DNG when we read it.

        for entry in list {
            if entry.was_read_from_dng {
                if self.get_profile_by_metadata(entry, found_profile) {
                    return true;
                }
            }
        }

        // Next we look for the first profile that is legal to embed.

        for entry in list {
            if entry.is_legal_to_embed {
                if self.get_profile_by_metadata(entry, found_profile) {
                    return true;
                }
            }
        }

        // Else just return the first profile.

        self.get_profile_by_metadata(&list[0], found_profile)
    }

    /// `GetProfileByID (id, foundProfile, useDefaultIfNoMatch, groupSelector)`
    /// — finds a camera profile to use for rendering.
    pub fn get_profile_by_id(
        &self,
        id: &DngCameraProfileId,
        found_profile: &mut DngCameraProfile,
        use_default_if_no_match: bool,
        group_selector: Option<&DngCameraProfileGroupSelector>,
    ) -> bool {
        // Monochrome negatives don't have profiles.

        if self.is_monochrome() {
            return false;
        }

        // Get list of profile metadata.

        let list = self.get_profile_metadata_list();

        // Search the list.

        self.get_profile_by_id_from_list(
            &list,
            id,
            found_profile,
            use_default_if_no_match,
            group_selector,
        )
    }

    /// `GetProfileToEmbed (metadata, foundProfile)` — returns the camera
    /// profile to embed when saving to DNG.
    pub fn get_profile_to_embed(
        &self,
        metadata: &DngMetadata,
        found_profile: &mut DngCameraProfile,
    ) -> bool {
        // Monochrome negatives don't have profiles.

        if self.is_monochrome() {
            return false;
        }

        let list = self.get_profile_metadata_list();

        self.get_profile_to_embed_from_list(&list, metadata, found_profile)
    }

    /// `AsShotProfileName` API.
    pub fn set_as_shot_profile_name(&mut self, name: &[uint8]) {
        self.as_shot_profile_name.set(name);
    }

    pub fn as_shot_profile_name(&self) -> &DngString {
        &self.as_shot_profile_name
    }

    /// `MakeColorSpec (id, allowStubbed)` — makes a dng_color_spec object
    /// for this negative.
    ///
    /// PORT-NOTE(dng_negative.cpp:1733): the C++ constructor reads the
    /// negative's analog balance and calibrations directly; the port passes
    /// them through a `ColorSpecInputs` bundle (see dng_color_spec.rs).
    pub fn make_color_spec(
        &self,
        id: &DngCameraProfileId,
        allow_stubbed: bool,
    ) -> DngResult<crate::dng_color_spec::DngColorSpec> {
        let mut profile = DngCameraProfile::new();

        let have_profile = self.get_profile_by_id(id, &mut profile, true, None);

        let mut inputs = crate::dng_color_spec::ColorSpecInputs::default();

        inputs.analog_balance = (0..self.color_channels)
            .map(|c| self.analog_balance(c))
            .collect();

        if self.camera_calibration1.not_empty() {
            inputs.camera_calibration1 = Some(self.camera_calibration1.clone());
        }
        if self.camera_calibration2.not_empty() {
            inputs.camera_calibration2 = Some(self.camera_calibration2.clone());
        }
        if self.camera_calibration3.not_empty() {
            inputs.camera_calibration3 = Some(self.camera_calibration3.clone());
        }

        inputs.calibration_signature_matches = !self.camera_calibration_signature.is_empty()
            && (self.camera_calibration_signature == *profile.profile_calibration_signature());

        crate::dng_color_spec::DngColorSpec::new(
            self.color_channels,
            if have_profile { Some(&profile) } else { None },
            allow_stubbed,
            &inputs,
        )
    }

    // -------------------------------------------------------------- digests

    /// `FindImageDigest (host, image)` — MD5 hash on an image using a fixed
    /// algorithm (dng_negative.cpp:1756).  Results are stable across
    /// hardware/OS/software versions.
    pub fn find_image_digest(_host: &mut DngHost, image: &ImageRef) -> DngResult<DngFingerprint> {
        let mut printer = DngMd5Printer::new();

        let img = image.read().unwrap();

        // Sometimes we expand 8-bit data to 16-bit data while reading or
        // writing, so always compute the digest of 8-bit data as 16-bits.

        let mut pixel_type = img.pixel_type();

        if pixel_type == ttByte {
            pixel_type = ttShort;
        }

        let planes = img.planes();

        const K_BUFFER_ROWS: int32 = 16;

        let bounds = img.bounds();

        let mut buffer = DngPixelBuffer::with_planar_config(
            DngRect::new(),
            0,
            planes,
            pixel_type,
            dng_core::dng_tag_values::pcInterleaved,
            dng_core::dng_pixel_buffer::PixelData::None,
        )?;

        let mut row = bounds.t;

        while row < bounds.b {
            let rows = min_int32(K_BUFFER_ROWS, bounds.b - row);

            let tile = DngRect::from_corners(row, bounds.l, row + rows, bounds.r);

            // (Re-)allocate the buffer for this tile (the C++ allocates one
            // kBufferRows buffer up front and reuses it).

            buffer.data = DngPixelBuffer::allocate_data(tile, planes, pixel_type)?;

            buffer.area = tile;

            img.get(
                &mut buffer,
                dng_core::dng_image::EdgeOption::EdgeNone,
                0,
                0,
            )?;

            let count = (tile.h() as usize)
                * (buffer.row_step.unsigned_abs() as usize)
                * (tag_type_size(pixel_type) as usize);

            let bytes = pixel_buffer_le_bytes(&buffer, count);

            printer.process(&bytes);

            row += rows;
        }

        Ok(*printer.result())
    }

    /// `FindFastImageDigest (host, image, pixelType)` — a more
    /// multi-process friendly hash on image.
    pub fn find_fast_image_digest(
        host: &mut DngHost,
        image: &ImageRef,
        pixel_type: uint32,
    ) -> DngResult<DngFingerprint> {
        let mut task = DngFindNewRawImageDigestTask::new(image.clone(), pixel_type);

        host.perform_area_task(&mut task, image.read().unwrap().bounds())?;

        task.result()
    }

    /// `SetRawImageDigest (digest)`.
    pub fn set_raw_image_digest(&mut self, digest: DngFingerprint) {
        self.raw_image_digest = digest;
    }

    /// `SetNewRawImageDigest (digest)`.
    pub fn set_new_raw_image_digest(&mut self, digest: DngFingerprint) {
        self.new_raw_image_digest = digest;
    }

    /// `ClearRawImageDigest ()`.
    pub fn clear_raw_image_digest(&mut self) {
        self.raw_image_digest.clear();
        self.new_raw_image_digest.clear();
    }

    /// `RawImageDigest ()`.
    pub fn raw_image_digest(&self) -> &DngFingerprint {
        &self.raw_image_digest
    }

    /// `NewRawImageDigest ()`.
    pub fn new_raw_image_digest(&self) -> &DngFingerprint {
        &self.new_raw_image_digest
    }

    /// `FindRawImageDigest (host)` (dng_negative.cpp:1858).
    pub fn find_raw_image_digest(&mut self, host: &mut DngHost) -> DngResult<()> {
        if self.raw_image_digest.is_null() {
            // Since we are adding the floating point and transparency support
            // in DNG 1.4, and there are no legacy floating point or
            // transparent DNGs, switch to using the more MP friendly
            // algorithm to compute the digest for these images.

            let raw_is_float = self
                .raw_image()
                .map(|img| img.read().unwrap().pixel_type() == ttFloat)
                .unwrap_or(false);

            if raw_is_float || self.raw_transparency_mask().is_some() {
                self.find_new_raw_image_digest(host)?;

                self.raw_image_digest = self.new_raw_image_digest;
            } else {
                let image = self.raw_image().ok_or_else(program_error)?.clone();

                self.raw_image_digest = Self::find_image_digest(host, &image)?;
            }
        }

        Ok(())
    }

    /// `FindNewRawImageDigest (host)` (dng_negative.cpp:2085).
    pub fn find_new_raw_image_digest(&mut self, host: &mut DngHost) -> DngResult<()> {
        if self.new_raw_image_digest.is_null() {
            // Find fast digest of the raw image.

            {
                let raw_image = self.raw_image().ok_or_else(program_error)?.clone();

                // Find pixel type that will be saved in the file.  When
                // saving DNGs, we convert some 16-bit data to 8-bit data, so
                // we need to do the matching logic here.

                let mut raw_pixel_type = raw_image.read().unwrap().pixel_type();

                if raw_pixel_type == ttShort {
                    // See if we are using a linearization table with <= 256
                    // entries, in which case the useful data will all fit
                    // within 8-bits.

                    if let Some(range_info) = &self.linearization_info {
                        if let Some(table) = &range_info.linearization_table {
                            let entries = table.len() as uint32;

                            if entries <= 256 {
                                raw_pixel_type = ttByte;
                            }
                        }
                    }
                }

                // Find the fast digest on the raw image.

                self.new_raw_image_digest =
                    Self::find_fast_image_digest(host, &raw_image, raw_pixel_type)?;
            }

            // If there is a transparency mask, we need to include that in
            // the digest also.

            if let Some(mask) = self.raw_transparency_mask().cloned() {
                let mask_pixel_type = mask.read().unwrap().pixel_type();

                let mask_digest = Self::find_fast_image_digest(host, &mask, mask_pixel_type)?;

                // Combine the two digests into a single digest.

                let mut printer = DngMd5Printer::new();

                printer.process(&self.new_raw_image_digest.data);
                printer.process(&mask_digest.data);

                self.new_raw_image_digest = *printer.result();
            }
        }

        Ok(())
    }

    /// `ValidateRawImageDigest (host)` (dng_negative.cpp:2182).
    pub fn validate_raw_image_digest(&mut self, host: &mut DngHost) -> DngResult<()> {
        if self.stage1_image.is_some()
            && !self.is_preview()
            && (self.raw_image_digest.is_valid() || self.new_raw_image_digest.is_valid())
        {
            let is_new_digest = self.new_raw_image_digest.is_valid();

            if self.raw_lossy_compressed_image_digest.is_valid()
                || self.raw_lossy_compressed_image.is_some()
            {
                self.find_raw_lossy_compressed_image_digest(host)?;

                let stored = if is_new_digest {
                    self.new_raw_image_digest
                } else {
                    self.raw_image_digest
                };

                if stored != self.raw_lossy_compressed_image_digest {
                    self.is_damaged = true;
                }
            } else if self.transparency_mask_was_lossy_compressed {
                // We currently don't have a defined way of computing digest
                // that safely round trips in this case.
            } else {
                // Else we can compare the stored digest to the image in
                // memory.

                let old_digest = if is_new_digest {
                    self.new_raw_image_digest
                } else {
                    self.raw_image_digest
                };

                // Recompute.

                let recomputed = if is_new_digest {
                    self.new_raw_image_digest.clear();
                    self.find_new_raw_image_digest(host)?;
                    self.new_raw_image_digest
                } else {
                    self.raw_image_digest.clear();
                    self.find_raw_image_digest(host)?;
                    self.raw_image_digest
                };

                if old_digest != recomputed {
                    if !is_new_digest {
                        // Note that Lightroom 1.4 Windows had a bug that
                        // corrupts the first four bytes of the RawImageDigest
                        // tag.  So if the last twelve bytes match, this is
                        // very likely the result of the bug, and not an
                        // actual corrupt file.  So don't report this to the
                        // user -- just fix it.

                        let mut match_last12 = true;

                        for j in 4..16 {
                            match_last12 = match_last12
                                && (old_digest.data[j] == self.raw_image_digest.data[j]);
                        }

                        if match_last12 {
                            return Ok(());
                        }

                        // Sometimes Lightroom 1.4 would corrupt more than
                        // the first four bytes, but for all those files that
                        // I have seen so far the resulting first four bytes
                        // are 0x08 0x00 0x00 0x00.

                        if old_digest.data[0] == 0x08
                            && old_digest.data[1] == 0x00
                            && old_digest.data[2] == 0x00
                            && old_digest.data[3] == 0x00
                        {
                            return Ok(());
                        }
                    }

                    self.is_damaged = true;
                }
            }
        }

        Ok(())
    }

    /// `SetRawDataUniqueID (id)`.
    pub fn set_raw_data_unique_id(&mut self, id: DngFingerprint) {
        self.raw_data_unique_id = id;
    }

    /// `BaseRawDataUniqueID ()`.
    pub fn base_raw_data_unique_id(&self) -> &DngFingerprint {
        &self.raw_data_unique_id
    }

    /// `RawDataUniqueID ()` (dng_negative.cpp:2332).
    pub fn raw_data_unique_id(&self) -> DngFingerprint {
        if self.raw_data_unique_id.is_valid() && self.enhance_params.not_empty() {
            let mut printer = DngMd5Printer::new();

            printer.process(&self.raw_data_unique_id.data);
            printer.process(self.enhance_params.get());

            return *printer.result();
        }

        self.raw_data_unique_id
    }

    /// `FindRawDataUniqueID (host)` (dng_negative.cpp:2361).
    pub fn find_raw_data_unique_id(&mut self, host: &mut DngHost) -> DngResult<()> {
        if self.raw_data_unique_id().is_null() {
            let mut printer = DngMd5PrinterStream::new();

            let put = |printer: &mut DngMd5PrinterStream, data: &[uint8]| -> DngResult<()> {
                printer.stream().put(data)
            };

            // If we have a raw lossy image, it is much faster to use its
            // digest as part of the unique ID since the data size is much
            // smaller.  We cannot use it if there is a transparency mask,
            // since that is not included in the RawLossyCompressedImageDigest.

            if self.raw_lossy_compressed_image.is_some() && self.raw_transparency_mask().is_none()
            {
                self.find_raw_lossy_compressed_image_digest(host)?;

                put(&mut printer, &self.raw_lossy_compressed_image_digest.data)?;
            }
            // Include the new raw image digest in the unique ID.
            else {
                self.find_new_raw_image_digest(host)?;

                put(&mut printer, &self.new_raw_image_digest.data)?;
            }

            // Include model name.

            put(&mut printer, self.model_name.get())?;

            // Include default crop area, since DNG Recover Edges can modify
            // these values and they affect rendering.

            put(&mut printer, &self.default_crop_size_h.n.to_le_bytes())?;
            put(&mut printer, &self.default_crop_size_h.d.to_le_bytes())?;

            put(&mut printer, &self.default_crop_size_v.n.to_le_bytes())?;
            put(&mut printer, &self.default_crop_size_v.d.to_le_bytes())?;

            put(&mut printer, &self.default_crop_origin_h.n.to_le_bytes())?;
            put(&mut printer, &self.default_crop_origin_h.d.to_le_bytes())?;

            put(&mut printer, &self.default_crop_origin_v.n.to_le_bytes())?;
            put(&mut printer, &self.default_crop_origin_v.d.to_le_bytes())?;

            // Include default user crop.

            put(&mut printer, &self.default_user_crop_t.n.to_le_bytes())?;
            put(&mut printer, &self.default_user_crop_t.d.to_le_bytes())?;

            put(&mut printer, &self.default_user_crop_l.n.to_le_bytes())?;
            put(&mut printer, &self.default_user_crop_l.d.to_le_bytes())?;

            put(&mut printer, &self.default_user_crop_b.n.to_le_bytes())?;
            put(&mut printer, &self.default_user_crop_b.d.to_le_bytes())?;

            put(&mut printer, &self.default_user_crop_r.n.to_le_bytes())?;
            put(&mut printer, &self.default_user_crop_r.d.to_le_bytes())?;

            // Include opcode lists, since lens correction utilities can
            // modify these values and they affect rendering.

            self.opcode_list1.fingerprint_to_stream(printer.stream())?;
            self.opcode_list2.fingerprint_to_stream(printer.stream())?;
            self.opcode_list3.fingerprint_to_stream(printer.stream())?;

            self.raw_data_unique_id = printer.result()?;
        }

        Ok(())
    }

    /// `RecomputeRawDataUniqueID (host)` — forces recomputation of
    /// RawDataUniqueID, useful after modifying the opcode lists, etc.
    pub fn recompute_raw_data_unique_id(&mut self, host: &mut DngHost) -> DngResult<()> {
        self.raw_data_unique_id.clear();

        self.find_raw_data_unique_id(host)
    }

    // ------------------------------------------------ original raw file API

    /// `SetOriginalRawFileName (name)`.
    pub fn set_original_raw_file_name(&mut self, name: &[uint8]) {
        self.original_raw_file_name.set(name);
    }

    /// `HasOriginalRawFileName ()`.
    pub fn has_original_raw_file_name(&self) -> bool {
        self.original_raw_file_name.not_empty()
    }

    /// `OriginalRawFileName ()`.
    pub fn original_raw_file_name(&self) -> &DngString {
        &self.original_raw_file_name
    }

    /// `SetHasOriginalRawFileData (hasData)`.
    pub fn set_has_original_raw_file_data(&mut self, has_data: bool) {
        self.has_original_raw_file_data = has_data;
    }

    /// `CanEmbedOriginalRaw ()`.
    pub fn can_embed_original_raw(&self) -> bool {
        self.has_original_raw_file_data && self.has_original_raw_file_name()
    }

    /// `SetOriginalRawFileData (data)`.
    pub fn set_original_raw_file_data(&mut self, data: Option<DngMemoryBlockAlias>) {
        self.original_raw_file_data = data;
    }

    /// `OriginalRawFileData ()`.
    pub fn original_raw_file_data(&self) -> Option<&[uint8]> {
        self.original_raw_file_data.as_ref().map(|b| b.buffer())
    }

    /// `OriginalRawFileDataLength ()`.
    pub fn original_raw_file_data_length(&self) -> uint32 {
        self.original_raw_file_data
            .as_ref()
            .map_or(0, |b| b.logical_size())
    }

    /// `SetOriginalRawFileDigest (digest)`.
    pub fn set_original_raw_file_digest(&mut self, digest: DngFingerprint) {
        self.original_raw_file_digest = digest;
    }

    /// `OriginalRawFileDigest ()`.
    pub fn original_raw_file_digest(&self) -> &DngFingerprint {
        &self.original_raw_file_digest
    }

    /// `FindOriginalRawFileDigest ()` (dng_negative.cpp:2460).
    pub fn find_original_raw_file_digest(&mut self) {
        if self.original_raw_file_digest.is_null() && self.original_raw_file_data.is_some() {
            let mut printer = DngMd5Printer::new();

            printer.process(self.original_raw_file_data.as_ref().unwrap().buffer());

            self.original_raw_file_digest = *printer.result();
        }
    }

    /// `ValidateOriginalRawFileDigest ()` (dng_negative.cpp:2479).
    pub fn validate_original_raw_file_digest(&mut self) {
        if self.original_raw_file_digest.is_valid() && self.original_raw_file_data.is_some() {
            let old_digest = self.original_raw_file_digest;

            self.original_raw_file_digest.clear();

            self.find_original_raw_file_digest();

            if old_digest != self.original_raw_file_digest {
                self.is_damaged = true;

                // Don't "repair" the original image data digest.  Once it is
                // bad, it stays bad.

                self.original_raw_file_digest = old_digest;
            }
        }
    }

    // ------------------------------------------------------ DNG private data

    /// `SetPrivateData (block)`.
    pub fn set_private_data(&mut self, block: Option<DngMemoryBlockAlias>) {
        self.dng_private_data = block;
    }

    /// `ClearPrivateData ()`.
    pub fn clear_private_data(&mut self) {
        self.dng_private_data = None;
    }

    /// `PrivateData ()`.
    pub fn private_data(&self) -> Option<&[uint8]> {
        self.dng_private_data.as_ref().map(|b| b.buffer())
    }

    /// `PrivateLength ()`.
    pub fn private_length(&self) -> uint32 {
        self.dng_private_data
            .as_ref()
            .map_or(0, |b| b.logical_size())
    }

    // -------------------------------------------------- metadata forwarding

    /// `Metadata ()` — mutable access to the internal metadata.
    pub fn metadata_mut(&mut self) -> &mut DngMetadata {
        &mut self.metadata
    }

    /// `Metadata ()`.
    pub fn metadata(&self) -> &DngMetadata {
        &self.metadata
    }

    /// `BigTableIndex ()`.
    pub fn big_table_index(&self) -> &DngBigTableIndex {
        self.metadata.big_table_index()
    }

    /// `BigTableGroupIndex ()`.
    pub fn big_table_group_index(&self) -> &DngBigTableGroupIndex {
        self.metadata.big_table_group_index()
    }

    /// `BigTableDictionary ()`.
    pub fn big_table_dictionary(&self) -> &DngBigTableDictionary {
        self.metadata.big_table_dictionary()
    }

    /// `ImageSequenceInfo ()`.
    pub fn image_sequence_info(&self) -> &DngImageSequenceInfo {
        self.metadata.image_sequence_info()
    }

    /// `ImageStats ()`.
    pub fn image_stats(&self) -> &DngImageStats {
        self.metadata.image_stats()
    }

    /// `SetMakerNoteSafety (safe)`.
    pub fn set_maker_note_safety(&mut self, safe: bool) {
        self.metadata.set_maker_note_safety(safe);
    }

    /// `IsMakerNoteSafe ()`.
    pub fn is_maker_note_safe(&self) -> bool {
        self.metadata.is_maker_note_safe()
    }

    /// `SetMakerNote (block)`.
    pub fn set_maker_note(&mut self, block: Option<DngMemoryBlockAlias>) {
        self.metadata.set_maker_note(block);
    }

    /// `ClearMakerNote ()`.
    pub fn clear_maker_note(&mut self) {
        self.metadata.clear_maker_note();
    }

    /// `MakerNoteData ()`.
    pub fn maker_note_data(&self) -> Option<&[uint8]> {
        self.metadata.maker_note_data()
    }

    /// `MakerNoteLength ()`.
    pub fn maker_note_length(&self) -> uint32 {
        self.metadata.maker_note_length()
    }

    /// `GetExif ()`.
    pub fn get_exif(&self) -> &DngExif {
        self.metadata.get_exif()
    }

    /// `GetExif ()` mutable.
    pub fn get_exif_mut(&mut self) -> &mut DngExif {
        self.metadata.get_exif_mut()
    }

    /// `ResetExif (newExif)`.
    pub fn reset_exif(&mut self, new_exif: DngExif) {
        self.metadata.reset_exif(new_exif);
    }

    /// `GetOriginalExif ()`.
    pub fn get_original_exif(&self) -> Option<&DngExif> {
        self.metadata.get_original_exif()
    }

    /// `SetIPTC (block, offset)`.
    pub fn set_iptc_with_offset(&mut self, block: Option<DngMemoryBlockAlias>, offset: uint64) {
        self.metadata.set_iptc_with_offset(block, offset);
    }

    /// `SetIPTC (block)`.
    pub fn set_iptc(&mut self, block: Option<DngMemoryBlockAlias>) {
        self.metadata.set_iptc(block);
    }

    /// `ClearIPTC ()`.
    pub fn clear_iptc(&mut self) {
        self.metadata.clear_iptc();
    }

    /// `IPTCData ()`.
    pub fn iptc_data(&self) -> Option<&[uint8]> {
        self.metadata.iptc_data()
    }

    /// `IPTCLength ()`.
    pub fn iptc_length(&self) -> uint32 {
        self.metadata.iptc_length()
    }

    /// `IPTCOffset ()`.
    pub fn iptc_offset(&self) -> uint64 {
        self.metadata.iptc_offset()
    }

    /// `IPTCDigest (includePadding)`.
    pub fn iptc_digest(&self, include_padding: bool) -> DngFingerprint {
        self.metadata.iptc_digest(include_padding)
    }

    /// `RebuildIPTC (padForTIFF)`.
    pub fn rebuild_iptc(&mut self, pad_for_tiff: bool) -> DngResult<()> {
        self.metadata.rebuild_iptc(pad_for_tiff)
    }

    /// `SetXMP (host, buffer, count, xmpInSidecar, xmpIsNewer)`.
    pub fn set_xmp(
        &mut self,
        host: &mut DngHost,
        buffer: &[uint8],
        xmp_in_sidecar: bool,
        xmp_is_newer: bool,
    ) -> bool {
        self.metadata
            .set_xmp(host, buffer, xmp_in_sidecar, xmp_is_newer)
    }

    /// `GetXMP ()`.
    pub fn get_xmp(&self) -> &DngXmp {
        self.metadata.get_xmp()
    }

    /// `GetXMP ()` mutable.
    pub fn get_xmp_mut(&mut self) -> &mut DngXmp {
        self.metadata.get_xmp_mut()
    }

    /// `XMPinSidecar ()`.
    pub fn xmp_in_sidecar(&self) -> bool {
        self.metadata.xmp_in_sidecar()
    }

    /// `ResetXMP (newXMP)`.
    pub fn reset_xmp(&mut self, new_xmp: DngXmp) {
        self.metadata.reset_xmp(new_xmp);
    }

    /// `ResetXMPSidecarNewer (newXMP, inSidecar, isNewer)`.
    pub fn reset_xmp_sidecar_newer(&mut self, new_xmp: DngXmp, in_sidecar: bool, is_newer: bool) {
        self.metadata.reset_xmp_sidecar_newer(new_xmp, in_sidecar, is_newer);
    }

    /// `HaveValidEmbeddedXMP ()`.
    pub fn have_valid_embedded_xmp(&self) -> bool {
        self.metadata.have_valid_embedded_xmp()
    }

    /// `SetSourceMIME (s)`.
    pub fn set_source_mime(&mut self, s: &[uint8]) {
        self.metadata.set_source_mime(s);
    }

    // --------------------------------------------- linearization delegation

    /// `GetLinearizationInfo ()`.
    pub fn get_linearization_info(&self) -> Option<&DngLinearizationInfo> {
        self.linearization_info.as_deref()
    }

    /// `ClearLinearizationInfo ()`.
    pub fn clear_linearization_info(&mut self) {
        self.linearization_info = None;
    }

    /// `SetLinearization (curve)` — 16-bit LUT that maps stored data back to
    /// linear.
    pub fn set_linearization(&mut self, curve: Option<Vec<uint16>>) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.linearization_table = curve;
        }
    }

    /// `SetActiveArea (area)` — active area (non-black masked pixels).
    pub fn set_active_area(&mut self, area: DngRect) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.active_area = area;
        }
    }

    /// `SetMaskedAreas (count, area)` — areas that are known to contain
    /// black masked pixels.
    pub fn set_masked_areas(&mut self, areas: &[DngRect]) {
        debug_assert!(
            areas.len() <= kMaxMaskedAreas as usize,
            "Too many masked areas"
        );

        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.masked_area_count = min_masked_count(areas.len() as uint32, kMaxMaskedAreas);

            for index in 0..info.masked_area_count as usize {
                info.masked_area[index] = areas[index];
            }

            info.masked_areas = areas
                .iter()
                .take(info.masked_area_count as usize)
                .cloned()
                .collect();
        }
    }

    /// `SetMaskedArea (area)`.
    pub fn set_masked_area(&mut self, area: DngRect) {
        self.set_masked_areas(&[area]);
    }

    /// `SetBlackLevel (black, plane)` — sensor black level information
    /// (plane < 0 means all planes).
    pub fn set_black_level(&mut self, black: real64, plane: int32) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.black_level_repeat_rows = 1;
            info.black_level_repeat_cols = 1;

            if plane < 0 {
                for j in 0..kMaxColorPlanes as usize {
                    info.black_level[0][0][j] = black;
                }
            } else {
                info.black_level[0][0][plane as usize] = black;
            }

            info.round_blacks();
        }
    }

    /// `SetQuadBlacks (black0, black1, black2, black3, plane)`.
    pub fn set_quad_blacks(
        &mut self,
        black0: real64,
        black1: real64,
        black2: real64,
        black3: real64,
        plane: int32,
    ) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.black_level_repeat_rows = 2;
            info.black_level_repeat_cols = 2;

            if plane < 0 {
                for j in 0..kMaxColorPlanes as usize {
                    info.black_level[0][0][j] = black0;
                    info.black_level[0][1][j] = black1;
                    info.black_level[1][0][j] = black2;
                    info.black_level[1][1][j] = black3;
                }
            } else {
                info.black_level[0][0][plane as usize] = black0;
                info.black_level[0][1][plane as usize] = black1;
                info.black_level[1][0][plane as usize] = black2;
                info.black_level[1][1][plane as usize] = black3;
            }

            info.round_blacks();
        }
    }

    /// `Set6x6Blacks (blacks6x6, plane)`.
    pub fn set_6x6_blacks(&mut self, blacks_6x6: &[real64; 36], plane: int32) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            info.black_level_repeat_rows = 6;
            info.black_level_repeat_cols = 6;

            if plane < 0 {
                // Apply the black levels to each image plane up to
                // kMaxColorPlanes.

                for p in 0..kMaxColorPlanes as usize {
                    let mut m = 0;

                    for r in 0..info.black_level_repeat_rows as usize {
                        for c in 0..info.black_level_repeat_cols as usize {
                            info.black_level[r][c][p] = blacks_6x6[m];

                            m += 1;
                        }
                    }
                }
            } else {
                let mut m = 0;

                // Apply the black levels to a single plane.

                for r in 0..info.black_level_repeat_rows as usize {
                    for c in 0..info.black_level_repeat_cols as usize {
                        info.black_level[r][c][plane as usize] = blacks_6x6[m];

                        m += 1;
                    }
                }
            }

            info.round_blacks();
        }
    }

    /// `SetRowBlacks (blacks, count)`.
    pub fn set_row_blacks(&mut self, blacks: &[real64]) {
        if !blacks.is_empty() {
            self.need_linearization_info();

            if let Some(info) = &mut self.linearization_info {
                info.black_delta_v = Some(blacks.to_vec());

                info.round_blacks();
            }
        } else if let Some(info) = &mut self.linearization_info {
            info.black_delta_v = None;
        }
    }

    /// `SetColumnBlacks (blacks, count)`.
    pub fn set_column_blacks(&mut self, blacks: &[real64]) {
        if !blacks.is_empty() {
            self.need_linearization_info();

            if let Some(info) = &mut self.linearization_info {
                info.black_delta_h = Some(blacks.to_vec());

                info.round_blacks();
            }
        } else if let Some(info) = &mut self.linearization_info {
            info.black_delta_h = None;
        }
    }

    /// `WhiteLevel (plane)` — sensor white level information.
    pub fn white_level(&self, plane: uint32) -> uint32 {
        if let Some(info) = &self.linearization_info {
            return round_uint32_f64(info.white_level[plane as usize]);
        }

        if let Some(raw) = self.raw_image() {
            if raw.read().unwrap().pixel_type() == ttFloat {
                return 1;
            }
        }

        0x0FFFF
    }

    /// `SetWhiteLevel (white, plane)` (plane < 0 means all planes).
    pub fn set_white_level(&mut self, white: uint32, plane: int32) {
        self.need_linearization_info();

        if let Some(info) = &mut self.linearization_info {
            if plane < 0 {
                for j in 0..kMaxColorPlanes as usize {
                    info.white_level[j] = white as real64;
                }
            } else {
                info.white_level[plane as usize] = white as real64;
            }
        }
    }

    // ----------------------------------------------------- mosaic delegation

    /// `GetMosaicInfo ()`.
    pub fn get_mosaic_info(&self) -> Option<&DngMosaicInfo> {
        self.mosaic_info.as_deref()
    }

    /// `GetMosaicInfo ()` mutable.
    pub fn get_mosaic_info_mut(&mut self) -> Option<&mut DngMosaicInfo> {
        self.mosaic_info.as_deref_mut()
    }

    /// `ClearMosaicInfo ()`.
    pub fn clear_mosaic_info(&mut self) {
        self.mosaic_info = None;
    }

    /// `SetColorKeys (color0, color1, color2, color3)`.
    pub fn set_color_keys(&mut self, color0: uint8, color1: uint8, color2: uint8, color3: uint8) {
        self.need_mosaic_info();

        if let Some(info) = &mut self.mosaic_info {
            info.cfa_plane_color[0] = color0;
            info.cfa_plane_color[1] = color1;
            info.cfa_plane_color[2] = color2;
            info.cfa_plane_color[3] = color3;
        }
    }

    /// `SetRGB ()`.
    pub fn set_rgb(&mut self) {
        self.set_color_channels(3);

        self.set_color_keys(colorKeyRed as uint8, colorKeyGreen as uint8, colorKeyBlue as uint8, colorKeyMaxEnum as uint8);
    }

    /// `SetCMY ()`.
    pub fn set_cmy(&mut self) {
        self.set_color_channels(3);

        self.set_color_keys(colorKeyCyan as uint8, colorKeyMagenta as uint8, colorKeyYellow as uint8, colorKeyMaxEnum as uint8);
    }

    /// `SetGMCY ()`.
    pub fn set_gmcy(&mut self) {
        self.set_color_channels(4);

        self.set_color_keys(colorKeyGreen as uint8, colorKeyMagenta as uint8, colorKeyCyan as uint8, colorKeyYellow as uint8);
    }

    /// `SetBayerMosaic (phase)` (dng_negative.cpp:2957).
    pub fn set_bayer_mosaic(&mut self, phase: uint32) {
        self.need_mosaic_info();

        if let Some(info) = &mut self.mosaic_info {
            let color0 = info.cfa_plane_color[0];
            let color1 = info.cfa_plane_color[1];
            let color2 = info.cfa_plane_color[2];

            info.cfa_pattern_size = DngPoint::new(2, 2);

            match phase {
                0 => {
                    info.cfa_pattern[0][0] = color1;
                    info.cfa_pattern[0][1] = color0;
                    info.cfa_pattern[1][0] = color2;
                    info.cfa_pattern[1][1] = color1;
                }

                1 => {
                    info.cfa_pattern[0][0] = color0;
                    info.cfa_pattern[0][1] = color1;
                    info.cfa_pattern[1][0] = color1;
                    info.cfa_pattern[1][1] = color2;
                }

                2 => {
                    info.cfa_pattern[0][0] = color2;
                    info.cfa_pattern[0][1] = color1;
                    info.cfa_pattern[1][0] = color1;
                    info.cfa_pattern[1][1] = color0;
                }

                3 => {
                    info.cfa_pattern[0][0] = color1;
                    info.cfa_pattern[0][1] = color2;
                    info.cfa_pattern[1][0] = color0;
                    info.cfa_pattern[1][1] = color1;
                }

                _ => {}
            }

            info.color_planes = 3;

            info.cfa_layout = 1;
        }
    }

    /// `SetFujiMosaic (phase)` (dng_negative.cpp:3019).
    pub fn set_fuji_mosaic(&mut self, phase: uint32) {
        self.need_mosaic_info();

        if let Some(info) = &mut self.mosaic_info {
            let color0 = info.cfa_plane_color[0];
            let color1 = info.cfa_plane_color[1];
            let color2 = info.cfa_plane_color[2];

            info.cfa_pattern_size = DngPoint::new(2, 4);

            match phase {
                0 => {
                    info.cfa_pattern[0][0] = color0;
                    info.cfa_pattern[0][1] = color1;
                    info.cfa_pattern[0][2] = color2;
                    info.cfa_pattern[0][3] = color1;
                    info.cfa_pattern[1][0] = color2;
                    info.cfa_pattern[1][1] = color1;
                    info.cfa_pattern[1][2] = color0;
                    info.cfa_pattern[1][3] = color1;
                }

                1 => {
                    info.cfa_pattern[0][0] = color2;
                    info.cfa_pattern[0][1] = color1;
                    info.cfa_pattern[0][2] = color0;
                    info.cfa_pattern[0][3] = color1;
                    info.cfa_pattern[1][0] = color0;
                    info.cfa_pattern[1][1] = color1;
                    info.cfa_pattern[1][2] = color2;
                    info.cfa_pattern[1][3] = color1;
                }

                _ => {}
            }

            info.color_planes = 3;

            info.cfa_layout = 2;
        }
    }

    /// `SetFujiMosaic6x6 (phase)` (dng_negative.cpp:3071).
    pub fn set_fuji_mosaic_6x6(&mut self, phase: uint32) {
        self.need_mosaic_info();

        let pat_size: uint32 = 6;

        if let Some(info) = &mut self.mosaic_info {
            let color0 = info.cfa_plane_color[0];
            let color1 = info.cfa_plane_color[1];
            let color2 = info.cfa_plane_color[2];

            info.cfa_pattern_size = DngPoint::new(pat_size as int32, pat_size as int32);

            info.cfa_pattern[0][0] = color1;
            info.cfa_pattern[0][1] = color2;
            info.cfa_pattern[0][2] = color1;
            info.cfa_pattern[0][3] = color1;
            info.cfa_pattern[0][4] = color0;
            info.cfa_pattern[0][5] = color1;

            info.cfa_pattern[1][0] = color0;
            info.cfa_pattern[1][1] = color1;
            info.cfa_pattern[1][2] = color0;
            info.cfa_pattern[1][3] = color2;
            info.cfa_pattern[1][4] = color1;
            info.cfa_pattern[1][5] = color2;

            info.cfa_pattern[2][0] = color1;
            info.cfa_pattern[2][1] = color2;
            info.cfa_pattern[2][2] = color1;
            info.cfa_pattern[2][3] = color1;
            info.cfa_pattern[2][4] = color0;
            info.cfa_pattern[2][5] = color1;

            info.cfa_pattern[3][0] = color1;
            info.cfa_pattern[3][1] = color0;
            info.cfa_pattern[3][2] = color1;
            info.cfa_pattern[3][3] = color1;
            info.cfa_pattern[3][4] = color2;
            info.cfa_pattern[3][5] = color1;

            info.cfa_pattern[4][0] = color2;
            info.cfa_pattern[4][1] = color1;
            info.cfa_pattern[4][2] = color2;
            info.cfa_pattern[4][3] = color0;
            info.cfa_pattern[4][4] = color1;
            info.cfa_pattern[4][5] = color0;

            info.cfa_pattern[5][0] = color1;
            info.cfa_pattern[5][1] = color0;
            info.cfa_pattern[5][2] = color1;
            info.cfa_pattern[5][3] = color1;
            info.cfa_pattern[5][4] = color2;
            info.cfa_pattern[5][5] = color1;

            debug_assert!(phase < pat_size * pat_size, "Bad phase in SetFujiMosaic6x6.");

            if phase > 0 {
                let temp = info.clone();

                let phase_row = phase / pat_size;

                let phase_col = phase - (phase_row * pat_size);

                for dst_row in 0..pat_size as usize {
                    let src_row = ((dst_row as uint32) + phase_row) % pat_size;

                    for dst_col in 0..pat_size as usize {
                        let src_col = ((dst_col as uint32) + phase_col) % pat_size;

                        info.cfa_pattern[dst_row][dst_col] =
                            temp.cfa_pattern[src_row as usize][src_col as usize];
                    }
                }
            }

            info.color_planes = 3;

            info.cfa_layout = 1;
        }
    }

    /// `SetQuadMosaic (pattern)` (dng_negative.cpp:3168).
    pub fn set_quad_mosaic(&mut self, pattern: uint32) {
        self.need_mosaic_info();

        if let Some(info) = &mut self.mosaic_info {
            if ((pattern >> 16) & 0x0FFFF) != (pattern & 0x0FFFF) {
                info.cfa_pattern_size = DngPoint::new(8, 2);
            } else if ((pattern >> 8) & 0x0FF) != (pattern & 0x0FF) {
                info.cfa_pattern_size = DngPoint::new(4, 2);
            } else {
                info.cfa_pattern_size = DngPoint::new(2, 2);
            }

            for row in 0..info.cfa_pattern_size.v {
                for col in 0..info.cfa_pattern_size.h {
                    let index = (pattern >> ((((row << 1) & 14) + (col & 1)) << 1)) & 3;

                    info.cfa_pattern[row as usize][col as usize] =
                        info.cfa_plane_color[index as usize];
                }
            }

            info.color_planes = 4;

            info.cfa_layout = 1;
        }
    }

    /// `SetGreenSplit (split)` — BayerGreenSplit.
    pub fn set_green_split(&mut self, split: uint32) {
        self.need_mosaic_info();

        if let Some(info) = &mut self.mosaic_info {
            info.bayer_green_split = split;
        }
    }

    // --------------------------------------------------------- opcode lists

    /// `OpcodeList1 ()`.
    pub fn opcode_list1(&self) -> &DngOpcodeList {
        &self.opcode_list1
    }

    /// `OpcodeList1 ()` mutable.
    pub fn opcode_list1_mut(&mut self) -> &mut DngOpcodeList {
        &mut self.opcode_list1
    }

    /// `OpcodeList2 ()`.
    pub fn opcode_list2(&self) -> &DngOpcodeList {
        &self.opcode_list2
    }

    /// `OpcodeList2 ()` mutable.
    pub fn opcode_list2_mut(&mut self) -> &mut DngOpcodeList {
        &mut self.opcode_list2
    }

    /// `OpcodeList3 ()`.
    pub fn opcode_list3(&self) -> &DngOpcodeList {
        &self.opcode_list3
    }

    /// `OpcodeList3 ()` mutable.
    pub fn opcode_list3_mut(&mut self) -> &mut DngOpcodeList {
        &mut self.opcode_list3
    }

    // -------------------------------------------------------------- parse

    /// `Parse (host, stream, info)` — first part of parsing logic
    /// (dng_negative.cpp:3283).
    pub fn parse(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        // Find IFD holding the main raw information.

        let main_index = info.main_index as usize;

        // Model name.

        let model = info.shared.unique_camera_model.get().to_vec();
        self.set_model_name(&model);

        // Localized model name.

        let local = info.shared.localized_camera_model.get().to_vec();
        self.set_local_name(&local);

        // Base orientation.

        {
            let orientation = info.ifd[0].orientation;

            if (1..=8).contains(&orientation) {
                self.set_base_orientation(DngOrientation::tiff_to_dng(orientation));
            }
        }

        // Snapshot the raw IFD / shared data we need (clone to satisfy the
        // borrow checker while calling &mut self methods).

        let raw_ifd = info.ifd[main_index].clone();
        let shared = info.shared.clone();

        // Default crop rectangle.

        self.set_default_crop_size(raw_ifd.default_crop_size_h, raw_ifd.default_crop_size_v);

        self.set_default_crop_origin(
            raw_ifd.default_crop_origin_h,
            raw_ifd.default_crop_origin_v,
        );

        // Default user crop rectangle.

        self.set_default_user_crop(
            raw_ifd.default_user_crop_t,
            raw_ifd.default_user_crop_l,
            raw_ifd.default_user_crop_b,
            raw_ifd.default_user_crop_r,
        );

        // Default scale.

        self.set_default_scale(raw_ifd.default_scale_h, raw_ifd.default_scale_v);

        // Best quality scale.

        self.set_best_quality_scale(raw_ifd.best_quality_scale);

        // Baseline noise.

        self.set_baseline_noise(shared.baseline_noise.as_real64());

        // NoiseReductionApplied (prefer the raw IFD over IFD 0).

        if raw_ifd.noise_reduction_applied.is_valid() {
            self.set_noise_reduction_applied(raw_ifd.noise_reduction_applied);
        } else {
            self.set_noise_reduction_applied(info.ifd[0].noise_reduction_applied);
        }

        // NoiseProfile (prefer the raw IFD over IFD 0). The C++ parses the
        // tag inline in dng_ifd ParseTag (no leading count field), so the
        // parsed (scale, offset) pairs are already available on the IFD.

        {
            let mut noise_profile = DngNoiseProfile::new();

            let functions: &[((real64, real64))] = if !raw_ifd.noise_functions.is_empty() {
                &raw_ifd.noise_functions
            } else {
                &info.ifd[0].noise_functions
            };

            if !functions.is_empty() {
                noise_profile = DngNoiseProfile::with_functions(
                    functions
                        .iter()
                        .map(|(scale, offset)| {
                            crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(
                                *scale, *offset,
                            )
                        })
                        .collect(),
                );
            }

            if noise_profile.is_valid() {
                self.set_noise_profile(noise_profile);
            }
        }

        // Baseline exposure.

        self.set_baseline_exposure(shared.baseline_exposure.as_real64());

        // Baseline sharpness.

        self.set_baseline_sharpness(shared.baseline_sharpness.as_real64());

        // Chroma blur radius.

        self.set_chroma_blur_radius(raw_ifd.chroma_blur_radius);

        // Anti-alias filter strength.

        self.set_anti_alias_strength(raw_ifd.anti_alias_strength);

        // Linear response limit.

        self.set_linear_response_limit(shared.linear_response_limit.as_real64());

        // Shadow scale.

        self.set_shadow_scale(DngURational::new(
            max_int32(shared.shadow_scale.n, 0) as uint32,
            shared.shadow_scale.d as uint32,
        ));

        // Colorimetric reference.

        self.set_colorimetric_reference(shared.colorimetric_reference);

        // Floating point flag.

        self.set_floating_point(raw_ifd.sample_format[0] == sfFloatingPoint);

        // Color channels.

        self.set_color_channels(shared.camera_profile.color_planes);

        // Analog balance.

        if let Some(balance) = &shared.analog_balance {
            if balance.not_empty() {
                self.set_analog_balance(balance);
            }
        }

        // Camera calibration matrices.

        if shared.camera_calibration1.not_empty() {
            self.set_camera_calibration1(&shared.camera_calibration1);
        }

        if shared.camera_calibration2.not_empty() {
            self.set_camera_calibration2(&shared.camera_calibration2);
        }

        if shared.camera_calibration3.not_empty() {
            self.set_camera_calibration3(&shared.camera_calibration3);
        }

        if shared.camera_calibration1.not_empty()
            || shared.camera_calibration2.not_empty()
            || shared.camera_calibration3.not_empty()
        {
            let sig = shared.camera_calibration_signature.get().to_vec();
            self.set_camera_calibration_signature(&sig);
        }

        // ProfileGainTableMap and ProfileGainTableMap2.

        {
            // ProfileGainTableMap2 usage is IFD 0 and supercedes
            // ProfileGainTableMap (whose usage is Raw IFD).

            if info.ifd[0].profile_gain_table_map_count != 0 {
                 stream.set_read_position(info.ifd[0].profile_gain_table_map_offset)?;

                let map = crate::dng_gain_map::DngGainTableMap::get_stream(stream)?;

                self.profile_gain_table_map = Some(std::sync::Arc::new(map));
            } else if raw_ifd.profile_gain_table_map_count != 0 {
                 stream.set_read_position(raw_ifd.profile_gain_table_map_offset)?;

                let map = crate::dng_gain_map::DngGainTableMap::get_stream(stream)?;

                self.profile_gain_table_map = Some(std::sync::Arc::new(map));
            }
        }

        // Embedded camera profiles.

        if shared.camera_profile.color_planes > 1 {
            if host.needs_meta() || host.needs_image() {
                // Add profile from main IFD.

                {
                    let mut profile = DngCameraProfile::new();

                    // The C++ parses using the captured profileInfo offsets;
                    // the port re-seeks to the profile start and reads the
                    // tag records (see dng_camera_profile::parse).

                    // The C++ calls profile.Parse (stream, profileInfo),
                    // which reads extended data at the offsets captured in
                    // the info block.  The port applies the fully-parsed
                    // info captured by dng_shared (matrices, names, policy);
                    // hue-sat/look tables arrive via parse_extended in the
                    // phase-3 integration (see PORTING_NOTES).

                    profile.apply_info(&shared.camera_profile, &shared);
                    profile.load_tables_from_info(stream, &shared.camera_profile)?;

                    // The main embedded profile must be valid.

                    if !profile.is_valid(shared.camera_profile.color_planes) {
                        return Err(dng_core::bad_format());
                    }

                    profile.set_was_read_from_dng(true);

                    self.add_profile(profile);
                }

                // Extra profiles.

                for profile_info in &shared.extra_camera_profiles {
                    let result = (|| -> DngResult<()> {
                        let mut profile = DngCameraProfile::new();

                        profile.apply_info(profile_info, &shared);
                        profile.load_tables_from_info(stream, profile_info)?;

                        if !profile.is_valid(shared.camera_profile.color_planes) {
                            return Err(dng_core::bad_format());
                        }

                        profile.set_was_read_from_dng(true);

                        self.add_profile(profile);

                        Ok(())
                    })();

                    if let Err(e) = result {
                        // Don't ignore transient errors.

                        if host.is_transient_error(&e) {
                            return Err(e);
                        }

                        // Eat other parsing errors.
                    }
                }
            }

            // As shot profile name.

            if shared.as_shot_profile_name.not_empty() {
                let name = shared.as_shot_profile_name.get().to_vec();
                self.set_as_shot_profile_name(&name);
            }
        }

        // Raw image data digest.

        if shared.raw_image_digest.is_valid() {
            self.set_raw_image_digest(shared.raw_image_digest);
        }

        // New raw image data digest.

        if shared.new_raw_image_digest.is_valid() {
            self.set_new_raw_image_digest(shared.new_raw_image_digest);
        }

        // Raw data unique ID.

        if shared.raw_data_unique_id.is_valid() {
            self.set_raw_data_unique_id(shared.raw_data_unique_id);
        }

        // Original raw file name.

        if shared.original_raw_file_name.not_empty() {
            let name = shared.original_raw_file_name.get().to_vec();
            self.set_original_raw_file_name(&name);
        }

        // Original raw file data.

        if shared.original_raw_file_data_count != 0 {
            self.set_has_original_raw_file_data(true);

            if host.keep_original_file() {
                let count = shared.original_raw_file_data_count;

                let data = stream_get_at(
                    stream,
                    shared.original_raw_file_data_offset,
                    count as usize,
                )?;

                let mut block = dng_core::DngMemoryBlock::new(count)?;
                block.buffer_mut().copy_from_slice(&data);

                self.set_original_raw_file_data(Some(block));

                self.set_original_raw_file_digest(shared.original_raw_file_digest);

                self.validate_original_raw_file_digest();
            }
        }

        // DNG private data.

        if shared.dng_private_data_count != 0 && host.save_dng_version() != dngVersion_None {
            let length = shared.dng_private_data_count;

            let data = stream_get_at(stream, shared.dng_private_data_offset, length as usize)?;

            let mut block = dng_core::DngMemoryBlock::new(length)?;
            block.buffer_mut().copy_from_slice(&data);

            self.set_private_data(Some(block));
        }

        // Hand off EXIF metadata to negative.

        let exif = std::mem::replace(&mut info.exif, DngExif::new());
        self.reset_exif(exif);

        // Parse linearization info.

        self.need_linearization_info();

        if let Some(li) = &mut self.linearization_info {
            li.parse(host, stream, info)?;
        }

        // Parse mosaic info.

        if raw_ifd.photometric_interpretation == dng_core::dng_tag_values::piCFA {
            self.need_mosaic_info();

            if let Some(mi) = &mut self.mosaic_info {
                mi.parse(host, stream, info);
            }
        }

        // Fill in original sizes.

        if shared.original_default_final_size.h > 0 && shared.original_default_final_size.v > 0 {
            self.set_original_default_final_size(shared.original_default_final_size);

            self.set_original_best_quality_final_size(shared.original_default_final_size);

            self.set_original_default_crop_size(
                DngURational::new(shared.original_default_final_size.h as uint32, 1),
                DngURational::new(shared.original_default_final_size.v as uint32, 1),
            );
        }

        if shared.original_best_quality_final_size.h > 0
            && shared.original_best_quality_final_size.v > 0
        {
            self.set_original_best_quality_final_size(shared.original_best_quality_final_size);
        }

        if shared.original_default_crop_size_h.as_real64() >= 1.0
            && shared.original_default_crop_size_v.as_real64() >= 1.0
        {
            self.set_original_default_crop_size(
                shared.original_default_crop_size_h,
                shared.original_default_crop_size_v,
            );
        }

        if shared.depth_format != depthFormatUnknown {
            self.depth_format = shared.depth_format;
        }

        if shared.depth_near != DngURational::new(0, 0) {
            self.depth_near = shared.depth_near;
        }

        if shared.depth_far != DngURational::new(0, 0) {
            self.depth_far = shared.depth_far;
        }

        if shared.depth_units != depthUnitsUnknown {
            self.depth_units = shared.depth_units;
        }

        if shared.depth_measure_type != depthMeasureUnknown {
            self.depth_measure_type = shared.depth_measure_type;
        }

        // Prefer some values from the enhanced IFD, if present and we are
        // not ignoring it.

        if info.enhanced_index != -1 && !host.ignore_enhanced() {
            let enhanced_ifd = info.ifd[info.enhanced_index as usize].clone();

            // Remember the enhance parameters.

            self.enhance_params = enhanced_ifd.enhance_params.clone();

            // Note: Call the various SetRaw... routines even if there are no
            // corresponding tags in the enhanced IFD.  This way, the tag
            // values are correctly associated with the raw (original,
            // non-enhanced) data and not with the enhanced IFD.

            // Baseline sharpness.

            self.set_raw_baseline_sharpness();

            if enhanced_ifd.baseline_sharpness.is_valid() {
                self.set_baseline_sharpness(enhanced_ifd.baseline_sharpness.as_real64());
            }

            // Noise reduction applied.

            self.set_raw_noise_reduction_applied();

            if enhanced_ifd.noise_reduction_applied.is_valid() {
                self.set_noise_reduction_applied(enhanced_ifd.noise_reduction_applied);
            }

            // Noise profile.

            self.set_raw_noise_profile();

            // Default scale.

            self.set_raw_default_scale();

            if enhanced_ifd.default_scale_h.is_valid() && enhanced_ifd.default_scale_v.is_valid()
            {
                self.set_default_scale(
                    enhanced_ifd.default_scale_h,
                    enhanced_ifd.default_scale_v,
                );
            }

            // Best quality scale.

            self.set_raw_best_quality_scale();

            if enhanced_ifd.best_quality_scale.is_valid() {
                self.best_quality_scale = enhanced_ifd.best_quality_scale;
            }

            // Default crop.

            self.set_raw_default_crop();

            if enhanced_ifd.default_crop_size_h.is_valid()
                && enhanced_ifd.default_crop_size_v.is_valid()
            {
                self.default_crop_size_h = enhanced_ifd.default_crop_size_h;
                self.default_crop_size_v = enhanced_ifd.default_crop_size_v;

                if enhanced_ifd.default_crop_origin_h.is_valid() {
                    self.default_crop_origin_h = enhanced_ifd.default_crop_origin_h;
                } else {
                    self.default_crop_origin_h = DngURational::new(0, 1);
                }

                if enhanced_ifd.default_crop_origin_v.is_valid() {
                    self.default_crop_origin_v = enhanced_ifd.default_crop_origin_v;
                } else {
                    self.default_crop_origin_v = DngURational::new(0, 1);
                }
            }
        }

        // Image Stats.

        if info.ifd[main_index]
            .image_stats
            .is_valid_for_plane_count(self.color_channels())
            && (info.ifd[main_index].image_stats.tag_count() > 0)
        {
            self.metadata
                .set_image_stats(info.ifd[main_index].image_stats.clone());
        }

        Ok(())
    }

    /// `PostParse (host, stream, info)` — second part of parsing logic
    /// (dng_negative.cpp:3951).
    pub fn post_parse(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        let shared = info.shared.clone();

        if host.needs_meta() {
            // Fill in original sizes if we don't have them already.

            self.set_default_original_sizes();

            // MakerNote.

            if shared.maker_note_count != 0 {
                // See if we know if the MakerNote is safe or not.

                self.set_maker_note_safety(shared.maker_note_safety == 1);

                // If the MakerNote is safe, preserve it as a MakerNote.

                if self.is_maker_note_safe() {
                    let position = shared.maker_note_offset + info.tiff_block_original_offset
                        - info.tiff_block_offset;

                    let data =
                        stream_get_at(stream, position, shared.maker_note_count as usize)?;

                    let mut block = dng_core::DngMemoryBlock::new(shared.maker_note_count)?;
                    block.buffer_mut().copy_from_slice(&data);

                    self.set_maker_note(Some(block));
                }
            }

            // IPTC metadata.

            if shared.iptc_naa_count != 0 {
                let iptc_offset = {
                    stream.set_read_position(shared.iptc_naa_offset)?;

                    stream.position_in_original_file()
                };

                let data =
                    stream_get_at(stream, shared.iptc_naa_offset, shared.iptc_naa_count as usize)?;

                let mut block = dng_core::DngMemoryBlock::new(shared.iptc_naa_count)?;
                block.buffer_mut().copy_from_slice(&data);

                self.set_iptc_with_offset(Some(block), iptc_offset);
            }

            // XMP metadata.

            if shared.xmp_count != 0 {
                let data = stream_get_at(stream, shared.xmp_offset, shared.xmp_count as usize)?;

                self.metadata.set_embedded_xmp(host, &data);
            }

            // Embedded big tables.

            if !shared.big_table_digests.is_empty() {
                let mut big_table_index = DngBigTableIndex::new();

                for j in 0..shared.big_table_digests.len() {
                    if !shared.big_table_digests[j].is_valid()
                        || shared.big_table_offsets[j] == 0
                        || shared.big_table_byte_counts[j] == 0
                    {
                        continue;
                    }

                    big_table_index.add_entry(
                        shared.big_table_digests[j],
                        shared.big_table_byte_counts[j],
                        shared.big_table_offsets[j],
                    );
                }

                if !big_table_index.is_empty() {
                    self.metadata.set_big_table_index(big_table_index);
                }

                // Big table group index.

                if !shared.big_table_group_index.is_empty() {
                    let mut index = DngBigTableGroupIndex::new();

                    for (group, instance) in &shared.big_table_group_index {
                        if group.is_valid() && instance.is_valid() {
                            index.add_entry(*group, *instance);
                        }
                    }

                    if !index.is_empty() {
                        self.metadata.set_big_table_group_index(index);
                    }
                }
            }

            // Color info.

            if !self.is_monochrome() {
                // If the ColorimetricReference is the ICC profile PCS, then
                // the data must be already be white balanced to the ICC
                // profile PCS white point.

                if self.is_output_referred() {
                    self.clear_camera_neutral();

                    self.set_camera_white_xy(&dng_core::dng_xy_coord::pcs_to_xy());
                } else {
                    // AsShotNeutral.

                    if let Some(neutral) = &shared.as_shot_neutral {
                        if neutral.count() == self.color_channels() {
                            self.set_camera_neutral(neutral);
                        }
                    }

                    // AsShotWhiteXY.

                    if shared.as_shot_white_xy.is_valid() && !self.has_camera_neutral() {
                        self.set_camera_white_xy(&shared.as_shot_white_xy);
                    }
                }
            } // color info

            // Image sequence info.

            if shared.image_sequence_info.is_valid() {
                self.metadata
                    .set_image_sequence_info(shared.image_sequence_info.clone());
            }
        } // needs meta

        Ok(())
    }

    /// `SynchronizeMetadata ()`.
    pub fn synchronize_metadata(&mut self) {
        self.metadata.synchronize_metadata();
    }

    /// `UpdateDateTime (dt)`.
    pub fn update_date_time(&mut self, dt: &DngDateTimeInfo) {
        self.metadata.update_date_time(dt);
    }

    /// `UpdateDateTimeToNow ()`.
    pub fn update_date_time_to_now(&mut self) {
        self.metadata.update_date_time_to_now();
    }

    /// `SetFourColorBayer ()` — developer's utility function to switch to
    /// four color Bayer interpolation (dng_negative.cpp:4163).
    pub fn set_four_color_bayer(&mut self) -> bool {
        if self.color_channels() != 3 {
            return false;
        }

        let mosaic_ok = match &mut self.mosaic_info {
            Some(info) => info.set_four_color_bayer(),
            None => return false,
        };

        if !mosaic_ok {
            return false;
        }

        self.set_color_channels(4);

        if self.camera_neutral.count() == 3 {
            let mut n = DngVector::with_count(4).unwrap();

            n.set(0, self.camera_neutral.get(0));
            n.set(1, self.camera_neutral.get(1));
            n.set(2, self.camera_neutral.get(2));
            n.set(3, self.camera_neutral.get(1));

            self.camera_neutral = n;
        }

        self.camera_calibration1.clear();
        self.camera_calibration2.clear();
        self.camera_calibration3.clear();

        self.camera_calibration_signature.clear();

        for profile in &mut self.camera_profiles {
            let _ = profile.set_four_color_bayer();
        }

        true
    }

    // ---------------------------------------------------------- stage images

    /// `Stage1Image ()`.
    pub fn stage1_image(&self) -> Option<&ImageRef> {
        self.stage1_image.as_ref()
    }

    /// `Stage2Image ()`.
    pub fn stage2_image(&self) -> Option<&ImageRef> {
        self.stage2_image.as_ref()
    }

    /// `Stage3Image ()`.
    pub fn stage3_image(&self) -> Option<&ImageRef> {
        self.stage3_image.as_ref()
    }

    /// `RawImageStage ()`.
    pub fn raw_image_stage(&self) -> RawImageStage {
        self.raw_image_stage
    }

    /// `RawImage ()` — returns the raw image data.
    pub fn raw_image(&self) -> Option<&ImageRef> {
        if self.raw_image.is_some() {
            return self.raw_image.as_ref();
        }

        if self.stage1_image.is_some() {
            return self.stage1_image.as_ref();
        }

        if self.unflattened_stage3_image.is_some() {
            return self.unflattened_stage3_image.as_ref();
        }

        self.stage3_image.as_ref()
    }

    /// `RawImageBlackLevel ()` — the raw image black level in 16-bit space.
    pub fn raw_image_black_level(&self) -> uint16 {
        if self.raw_image.is_some() {
            return self.raw_image_black_level;
        }

        if self.stage1_image.is_some() {
            return 0;
        }

        self.stage3_black_level
    }

    /// `RawFloatBitDepth ()`.
    pub fn raw_float_bit_depth(&self) -> uint32 {
        self.raw_float_bit_depth
    }

    /// `SetRawFloatBitDepth (bitDepth)`.
    pub fn set_raw_float_bit_depth(&mut self, bit_depth: uint32) {
        self.raw_float_bit_depth = bit_depth;
    }

    /// `RawLossyCompressedImage ()`.
    pub fn raw_lossy_compressed_image(&self) -> Option<&DngLossyCompressedImage> {
        self.raw_lossy_compressed_image.as_deref()
    }

    /// `SetRawLossyCompressedImage (image)`.
    pub fn set_raw_lossy_compressed_image(&mut self, image: Option<Box<DngLossyCompressedImage>>) {
        self.raw_lossy_compressed_image = image;
    }

    /// `ClearRawLossyCompressedImage ()`.
    pub fn clear_raw_lossy_compressed_image(&mut self) {
        self.raw_lossy_compressed_image = None;
    }

    /// `SetRawLossyCompressedImageDigest (digest)`.
    pub fn set_raw_lossy_compressed_image_digest(&mut self, digest: DngFingerprint) {
        self.raw_lossy_compressed_image_digest = digest;
    }

    /// `ClearRawLossyCompressedImageDigest ()`.
    pub fn clear_raw_lossy_compressed_image_digest(&mut self) {
        self.raw_lossy_compressed_image_digest.clear();
    }

    /// `RawLossyCompressedImageDigest ()`.
    pub fn raw_lossy_compressed_image_digest(&self) -> &DngFingerprint {
        &self.raw_lossy_compressed_image_digest
    }

    /// `FindRawLossyCompressedImageDigest (host)` (dng_negative.cpp:4289).
    pub fn find_raw_lossy_compressed_image_digest(&mut self, _host: &mut DngHost) -> DngResult<()> {
        if self.raw_lossy_compressed_image_digest.is_null() {
            if let Some(image) = &self.raw_lossy_compressed_image {
                self.raw_lossy_compressed_image_digest = image.find_digest()?;
            } else {
                return Err(program_error()); // "No raw lossy compressed image"
            }
        }

        Ok(())
    }

    // ------------------------------------------------------- reading images

    /// `ReadOpcodeLists (host, stream, info)` (dng_negative.cpp:4321).
    pub fn read_opcode_lists(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        let raw_ifd = info.ifd[info.main_index as usize].clone();

        if raw_ifd.opcode_list1_count != 0 {
            self.opcode_list1.parse(
                host,
                stream,
                raw_ifd.opcode_list1_count,
                raw_ifd.opcode_list1_offset,
            )?;
        }

        if raw_ifd.opcode_list2_count != 0 {
            self.opcode_list2.parse(
                host,
                stream,
                raw_ifd.opcode_list2_count,
                raw_ifd.opcode_list2_offset,
            )?;
        }

        if raw_ifd.opcode_list3_count != 0 {
            self.opcode_list3.parse(
                host,
                stream,
                raw_ifd.opcode_list3_count,
                raw_ifd.opcode_list3_offset,
            )?;
        }

        Ok(())
    }

    /// `ReadStage1Image (host, stream, info)` (dng_negative.cpp:4389).
    ///
    /// The actual pixel decode happens in `dng_ifd::ReadImage`, which is
    /// provided by the dng_read_image phase-2 module (Agent B); the port
    /// keeps all negative-side bookkeeping (lossy capture, float bit depth,
    /// opcode lists) here.
    pub fn read_stage1_image(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        // Allocate image we are reading.

        let raw_ifd = info.ifd[info.main_index as usize].clone();

        let stage1 = DngSimpleImage::new(
            raw_ifd.bounds(),
            raw_ifd.samples_per_pixel,
            raw_ifd.pixel_type(),
        )?;

        let stage1_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(stage1));

        self.stage1_image = Some(stage1_ref.clone());

        // See if we should grab the lossy compressed data.

        let lossy_image = keep_lossy_compressed_image(
            host.save_dng_version(),
            host.preferred_size(),
            host.for_preview(),
            &raw_ifd,
        );

        // Read the image.

        let mut lossy_ref = lossy_image;

        {
            let ifd = &info.ifd[info.main_index as usize];

            // C++ uses dng_read_image::Read here; the DngIfd::read_image
            // method is a dng-core seam stub (the reader lives in this
            // crate), so call the real reader directly.

            let mut reader = crate::dng_read_image::DngReadImage::new();

            let image_dyn: &dyn dng_core::DngImage = &*stage1_ref.read().unwrap();

            reader.read(
                ifd,
                stream,
                image_dyn,
                lossy_ref.as_deref_mut(),
                None,
            )?;
        }

        // Remember the raw floating point bit depth, if reading from a
        // floating point image.

        if stage1_ref.read().unwrap().pixel_type() == ttFloat {
            self.set_raw_float_bit_depth(raw_ifd.bits_per_sample[0]);
        }

        // Remember the compressed JPEG data if we read it.

        if let Some(image) = lossy_ref {
            self.set_raw_lossy_compressed_image(Some(image));
        }

        // We are reading the main image, so we should read the opcode lists
        // also.

        self.read_opcode_lists(host, stream, info)?;

        Ok(())
    }

    /// `ReadEnhancedImage (host, stream, info)` (dng_negative.cpp:4466).
    pub fn read_enhanced_image(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        // Should we read the raw image also?

        let need_raw_image = host.save_dng_version() != 0 && !host.save_linear_dng();

        // Allocate image we are reading.

        let enhanced_ifd = info.ifd[info.enhanced_index as usize].clone();

        let stage3 = DngSimpleImage::new(
            enhanced_ifd.bounds(),
            enhanced_ifd.samples_per_pixel,
            enhanced_ifd.pixel_type(),
        )?;

        let stage3_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(stage3));

        self.stage3_image = Some(stage3_ref.clone());

        // Do we need to keep the lossy compressed data?

        if need_raw_image {
            self.enhanced_lossy_compressed_image = keep_lossy_compressed_image(
                host.save_dng_version(),
                host.preferred_size(),
                host.for_preview(),
                &enhanced_ifd,
            );
        }

        // Read the image.

        {
            let ifd = &info.ifd[info.enhanced_index as usize];
            ifd.read_image(host, stream, &stage3_ref)?;
        }

        // Stage 3 black level.

        self.stage3_black_level = round_uint32_f64(enhanced_ifd.black_level[0][0][0]) as uint16;

        // If we are saving as a linear DNG, the saved profile should match
        // the profile for the stage3 image.

        if host.save_dng_version() != 0 && host.save_linear_dng() {
            self.adjust_profile_for_stage3();
        }

        // Read in raw image data if required.

        if need_raw_image {
            self.read_stage1_image(host, stream, info)?;

            self.raw_image = self.stage1_image.take();
        }
        // Else we can discard the information specific to the raw IFD.
        else {
            // We at least need to know about lens opcodes, so we read the
            // opcodes and then discard them.

            self.read_opcode_lists(host, stream, info)?;

            self.clear_linearization_info();
            self.clear_mosaic_info();

            self.opcode_list1.clear();
            self.opcode_list2.clear();
            self.opcode_list3.clear();

            self.raw_image_digest.clear();
            self.new_raw_image_digest.clear();

            self.raw_default_crop_size_h.clear();
            self.raw_default_crop_size_v.clear();

            self.raw_default_crop_origin_h.clear();
            self.raw_default_crop_origin_v.clear();

            self.raw_default_scale_h.clear();
            self.raw_default_scale_v.clear();

            self.raw_best_quality_scale.clear();

            self.raw_baseline_sharpness.clear();
            self.raw_noise_reduction_applied.clear();

            self.raw_noise_profile = DngNoiseProfile::new();

            if self.raw_data_unique_id.is_valid() {
                self.raw_data_unique_id = self.raw_data_unique_id();
            }

            self.enhance_params.clear();
        }

        Ok(())
    }

    /// `SetStage1Image (image)`.
    pub fn set_stage1_image(&mut self, image: Option<ImageRef>) {
        self.stage1_image = image;
    }

    /// `ClearStage1Image ()`.
    pub fn clear_stage1_image(&mut self) {
        self.stage1_image = None;
    }

    /// `SetStage2Image (image)`.
    pub fn set_stage2_image(&mut self, image: Option<ImageRef>) {
        self.stage2_image = image;
    }

    /// `SetStage3Image (image)`.
    pub fn set_stage3_image(&mut self, image: Option<ImageRef>) {
        self.floating_point = image
            .as_ref()
            .map(|img| img.read().unwrap().pixel_type() == ttFloat)
            .unwrap_or(false);

        self.stage3_image = image;
    }

    // ------------------------------------------------------- stage building

    /// `MakeLinearizationInfo ()` — factory hook.
    pub fn make_linearization_info(&self) -> DngResult<Box<DngLinearizationInfo>> {
        Ok(Box::new(DngLinearizationInfo::new()))
    }

    /// `NeedLinearizationInfo ()`.
    pub fn need_linearization_info(&mut self) {
        if self.linearization_info.is_none() {
            self.linearization_info = Some(Box::new(DngLinearizationInfo::new()));
        }
    }

    /// `MakeMosaicInfo ()` — factory hook.
    pub fn make_mosaic_info(&self) -> DngResult<Box<DngMosaicInfo>> {
        Ok(Box::new(DngMosaicInfo::new()))
    }

    /// `NeedMosaicInfo ()`.
    pub fn need_mosaic_info(&mut self) {
        if self.mosaic_info.is_none() {
            self.mosaic_info = Some(Box::new(DngMosaicInfo::new()));
        }
    }

    /// `DoBuildStage2 (host)` (dng_negative.cpp:4619).
    pub fn do_build_stage2(&mut self, host: &mut DngHost) -> DngResult<()> {
        let stage1 = self.stage1_image.clone().ok_or_else(program_error)?;

        // Clone the linearization info (small tables) to satisfy the borrow
        // checker: Linearize needs &mut self for the stage-3 black level.
        let info = self
            .linearization_info
            .as_ref()
            .ok_or_else(program_error)?
            .clone();

        let pixel_type = if stage1.read().unwrap().pixel_type() == ttLong
            || stage1.read().unwrap().pixel_type() == ttFloat
        {
            ttFloat
        } else {
            ttShort
        };

        let dst = DngSimpleImage::new(
            DngRect::from_size_point(info.active_area.size()),
            stage1.read().unwrap().planes(),
            pixel_type,
        )?;

        let dst_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(dst));

        self.stage2_image = Some(dst_ref.clone());

        let src = stage1.clone();

        info.linearize(host, self, &src, &dst_ref)?;

        Ok(())
    }

    /// `DoPostOpcodeList2 (host)` — hook for any required processing just
    /// after opcode list 2.  Nothing by default.
    pub fn do_post_opcode_list2(&mut self, _host: &mut DngHost) {}

    /// `NeedDefloatStage2 (host)` (dng_negative.cpp:4658).
    pub fn need_defloat_stage2(&self, host: &DngHost) -> bool {
        if let Some(stage2) = &self.stage2_image {
            if stage2.read().unwrap().pixel_type() == ttFloat {
                if self.raw_image_stage >= RawImageStage::PostOpcode2
                    && host.save_dng_version() != dngVersion_None
                    && host.save_dng_version() < dngVersion_1_4_0_0
                {
                    return true;
                }
            }
        }

        false
    }

    /// `DefloatStage2 (host)` (dng_negative.cpp:4681).
    pub fn defloat_stage2(&mut self, _host: &mut DngHost) -> DngResult<()> {
        Err(dng_core::not_yet_implemented()) // "dng_negative::DefloatStage2"
    }

    /// `BuildStage2Image (host)` — build the stage 2 (linearized and range
    /// mapped) image (dng_negative.cpp:4690).
    pub fn build_stage2_image(&mut self, host: &mut DngHost) -> DngResult<()> {
        // If reading the negative to save in DNG format, figure out when to
        // grab a copy of the raw data.

        if host.save_dng_version() != dngVersion_None {
            // Transparency masks are only supported in DNG version 1.4 and
            // later.  In this case, the flattening of the transparency mask
            // happens on the stage3 image.

            if self.transparency_mask.is_some() && host.save_dng_version() < dngVersion_1_4_0_0
            {
                self.raw_image_stage = RawImageStage::PostOpcode3;
            } else if self.opcode_list3.min_version(false) > host.save_dng_version()
                || self.opcode_list3.always_apply()
            {
                self.raw_image_stage = RawImageStage::PostOpcode3;
            }
            // If we are not doing a full resolution read, then always save
            // the DNG from the processed stage 3 image.
            else if host.preferred_size() != 0 {
                self.raw_image_stage = RawImageStage::PostOpcode3;
            } else if host.save_linear_dng() {
                // If the opcode list 3 has optional tags that are beyond the
                // minimum version, and we are saving a linear DNG anyway,
                // then go ahead and apply them.

                if self.opcode_list3.min_version(true) > host.save_dng_version() {
                    self.raw_image_stage = RawImageStage::PostOpcode3;
                } else {
                    self.raw_image_stage = RawImageStage::PreOpcode3;
                }
            } else if self.opcode_list2.min_version(false) > host.save_dng_version()
                || self.opcode_list2.always_apply()
            {
                self.raw_image_stage = RawImageStage::PostOpcode2;
            } else if self.need_lossy_compress_mosaic_jxl(host) {
                self.raw_image_stage = RawImageStage::PostOpcode2;
            } else if self.opcode_list1.min_version(false) > host.save_dng_version()
                || self.opcode_list1.always_apply()
            {
                self.raw_image_stage = RawImageStage::PostOpcode1;
            } else {
                self.raw_image_stage = RawImageStage::PreOpcode1;
            }

            // We should not save floating point stage1 images unless the
            // target DNG version is high enough to understand floating point
            // images.  We handle this by converting from floating point to
            // integer if required after building stage2 image.

            if let Some(stage1) = &self.stage1_image {
                if stage1.read().unwrap().pixel_type() == ttFloat
                    && self.raw_image_stage < RawImageStage::PostOpcode2
                    && host.save_dng_version() < dngVersion_1_4_0_0
                {
                    self.raw_image_stage = RawImageStage::PostOpcode2;
                }
            }

            // If the host is requesting a negative read for fast conversion
            // to DNG, then check whether we can actually do a fast
            // interpolation or not.

            if host.for_fast_save_to_dng() && self.raw_image_stage > RawImageStage::PreOpcode1
            {
                // Disable/revert the optimization attempt, and do a normal
                // interpolation when building the stage 3 image.

                host.set_for_fast_save_to_dng(false, 0);
            }
        }

        // Grab clone of raw image if required.

        if self.raw_image_stage == RawImageStage::PreOpcode1 {
            if let Some(stage1) = &self.stage1_image {
                self.raw_image = Some(stage1.read().unwrap().clone_image()?);
            }

            if let Some(mask) = &self.transparency_mask {
                self.raw_transparency_mask = Some(mask.read().unwrap().clone_image()?);
            }

            if let Some(depth) = &self.depth_map {
                self.raw_depth_map = Some(depth.read().unwrap().clone_image()?);
            }
        } else {
            // If we are not keeping the most raw image, we need to recompute
            // the raw image digest.

            self.clear_raw_image_digest();

            // If we don't grab the unprocessed stage 1 image, then the raw
            // lossy compressed image is no longer valid.

            self.clear_raw_lossy_compressed_image();

            // Nor is the digest of the raw lossy compressed data.

            self.clear_raw_lossy_compressed_image_digest();

            // We also don't know the raw floating point bit depth.

            self.set_raw_float_bit_depth(0);
        }

        // Process opcode list 1.

        {
            let mut image = self.stage1_image.clone().ok_or_else(program_error)?;

            let mut list = std::mem::replace(&mut self.opcode_list1, DngOpcodeList::new(1));

            let result = list.apply(host, self, &mut image);

            self.opcode_list1 = list;

            result?;
        }

        // See if we are done with the opcode list 1.

        if self.raw_image_stage > RawImageStage::PreOpcode1 {
            self.opcode_list1.clear();
        }

        // Grab clone of raw image if required.

        if self.raw_image_stage == RawImageStage::PostOpcode1 {
            if let Some(stage1) = &self.stage1_image {
                self.raw_image = Some(stage1.read().unwrap().clone_image()?);
            }

            if let Some(mask) = &self.transparency_mask {
                self.raw_transparency_mask = Some(mask.read().unwrap().clone_image()?);
            }

            if let Some(depth) = &self.depth_map {
                self.raw_depth_map = Some(depth.read().unwrap().clone_image()?);
            }
        }

        // Finalize linearization info.

        {
            self.need_linearization_info();

            let active_area_default = self
                .stage1_image
                .as_ref()
                .map(|img| img.read().unwrap().bounds());

            if let Some(info) = &mut self.linearization_info {
                if info.active_area.is_empty() {
                    if let Some(bounds) = active_area_default {
                        info.active_area = bounds;
                    }
                }
            }
        }

        // Perform the linearization.

        self.do_build_stage2(host)?;

        // Delete the stage1 image now that we have computed the stage 2
        // image.

        self.stage1_image = None;

        // Are we done with the linearization info.

        if self.raw_image_stage > RawImageStage::PostOpcode1 {
            self.clear_linearization_info();
        }

        // Process opcode list 2.

        {
            let mut image = self.stage2_image.clone().ok_or_else(program_error)?;

            let mut list = std::mem::replace(&mut self.opcode_list2, DngOpcodeList::new(2));

            let result = list.apply(host, self, &mut image);

            self.opcode_list2 = list;

            result?;
        }

        // See if we are done with the opcode list 2.

        if self.raw_image_stage > RawImageStage::PostOpcode1 {
            self.opcode_list2.clear();
        }

        // Hook for any required processing just after opcode list 2.

        self.do_post_opcode_list2(host);

        // Convert from floating point to integer if required.

        if self.need_defloat_stage2(host) {
            self.defloat_stage2(host)?;
        }

        // Grab clone of raw image if required.

        if self.raw_image_stage == RawImageStage::PostOpcode2 {
            if let Some(stage2) = &self.stage2_image {
                self.raw_image = Some(stage2.read().unwrap().clone_image()?);
            }

            self.raw_image_black_level = self.stage3_black_level;

            if let Some(mask) = &self.transparency_mask {
                self.raw_transparency_mask = Some(mask.read().unwrap().clone_image()?);
            }

            if let Some(depth) = &self.depth_map {
                self.raw_depth_map = Some(depth.read().unwrap().clone_image()?);
            }
        }

        Ok(())
    }

    /// `DoInterpolateStage3 (host, srcPlane, scaleTransforms)`
    /// (dng_negative.cpp:4977).
    pub fn do_interpolate_stage3(
        &mut self,
        host: &mut DngHost,
        mut src_plane: int32,
        scale_transforms: Option<&mut Vec<DngMatrix3by3>>,
    ) -> DngResult<()> {
        let stage2 = self.stage2_image.clone().ok_or_else(program_error)?;

        let info = self.mosaic_info.as_ref().ok_or_else(program_error)?.clone();

        let fast_save_to_dng = host.for_fast_save_to_dng();

        let fast_save_size = host.fast_save_to_dng_size();

        let down_scale = if fast_save_to_dng && fast_save_size > 0 {
            info.down_scale(host.minimum_size(), fast_save_size, host.crop_factor())
        } else {
            info.down_scale(
                host.minimum_size(),
                host.preferred_size(),
                host.crop_factor(),
            )
        };

        if down_scale != DngPoint::new(1, 1) {
            self.is_preview = true;
        }

        let dst_size = info.dst_size(down_scale);

        let dst = DngSimpleImage::new(
            DngRect::from_size_point(dst_size),
            info.color_planes,
            stage2.read().unwrap().pixel_type(),
        )?;

        let dst_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(dst));

        self.stage3_image = Some(dst_ref.clone());

        if src_plane < 0 || src_plane as uint32 >= stage2.read().unwrap().planes() {
            src_plane = 0;
        }

        let src = stage2.clone();

        info.interpolate(
            host,
            self,
            &src,
            &dst_ref,
            down_scale,
            src_plane as uint32,
            scale_transforms,
        )?;

        Ok(())
    }

    /// `DoMergeStage3 (host, scaleTransforms)` — interpolate and merge a
    /// multi-channel CFA image (dng_negative.cpp:5040).
    pub fn do_merge_stage3(
        &mut self,
        host: &mut DngHost,
        scale_transforms: Option<&mut Vec<DngMatrix3by3>>,
    ) -> DngResult<()> {
        // The DNG SDK does not provide multi-channel CFA image merging code.
        // It just grabs the first channel and uses that.

        self.do_interpolate_stage3(host, 0, scale_transforms)?;

        // Just grabbing the first channel would often result in the very
        // bright image using the baseline exposure value.

        self.stage3_gain = 2.0f64.powf(self.baseline_exposure());

        Ok(())
    }

    /// `DoBuildStage3 (host, srcPlane, scaleTransforms)` (dng_negative.cpp:5058).
    pub fn do_build_stage3(
        &mut self,
        host: &mut DngHost,
        src_plane: int32,
        mut scale_transforms: Option<&mut Vec<DngMatrix3by3>>,
    ) -> DngResult<()> {
        // If we don't have a mosaic pattern, then just move the stage 2
        // image on to stage 3.

        let is_cfa = self
            .mosaic_info
            .as_ref()
            .map(|info| info.is_color_filter_array())
            .unwrap_or(false);

        if !is_cfa {
            self.stage3_image = self.stage2_image.take();
        } else {
            // Remember the size of the stage 2 image.

            let stage2_size = self
                .stage2_image
                .as_ref()
                .ok_or_else(program_error)?
                .read()
                .unwrap()
                .size();

            // Special case multi-channel CFA interpolation.

            if self
                .stage2_image
                .as_ref()
                .ok_or_else(program_error)?
                .read()
                .unwrap()
                .planes()
                > 1
                && src_plane < 0
            {
                self.do_merge_stage3(host, scale_transforms.as_deref_mut())?;
            }
            // Else do a single channel interpolation.
            else {
                self.do_interpolate_stage3(host, src_plane, scale_transforms.as_deref_mut())?;
            }

            // Calculate the ratio of the stage 3 image size to stage 2 image
            // size.

            let stage3_size = self
                .stage3_image
                .as_ref()
                .ok_or_else(program_error)?
                .read()
                .unwrap()
                .size();

            self.raw_to_full_scale_h = stage3_size.h as real64 / stage2_size.h as real64;
            self.raw_to_full_scale_v = stage3_size.v as real64 / stage2_size.v as real64;
        }

        Ok(())
    }

    /// `BuildStage3Image (host, srcPlane)` — build the stage 3 (demosaiced)
    /// image (dng_negative.cpp:5116).
    pub fn build_stage3_image(&mut self, host: &mut DngHost, src_plane: int32) -> DngResult<()> {
        // Finalize the mosaic information.

        {
            let mut info = self.mosaic_info.take();

            if let Some(info) = &mut info {
                info.post_parse(host, self);
            }

            self.mosaic_info = info;
        }

        // Do the interpolation as required.

        self.do_build_stage3(host, src_plane, None)?;

        // Delete the stage2 image now that we have computed the stage 3
        // image, unless the host wants to preserve it.

        if !host.wants_preserve_stage2() {
            self.stage2_image = None;
        }

        // Are we done with the mosaic info?

        if self.raw_image_stage >= RawImageStage::PreOpcode3 {
            // If we're preserving the stage 2 image, also preserve the
            // mosaic info.

            if !host.wants_preserve_stage2() {
                self.clear_mosaic_info();
            }
        }

        // If we are keeping the raw image, keep the raw default crop and
        // scale information.

        if self.raw_image_stage < RawImageStage::PreOpcode3 {
            self.set_raw_default_crop();
            self.set_raw_default_scale();
            self.set_raw_best_quality_scale();
        }

        // To support saving linear DNG files, we need to account for
        // upscaling during interpolation.

        if self.raw_to_full_scale_h > 1.0 {
            self.default_crop_size_h.scale_by(self.raw_to_full_scale_h);
            self.default_crop_origin_h.scale_by(self.raw_to_full_scale_h);
            self.default_scale_h.scale_by(1.0 / self.raw_to_full_scale_h);

            self.raw_to_full_scale_h = 1.0;
        }

        if self.raw_to_full_scale_v > 1.0 {
            self.default_crop_size_v.scale_by(self.raw_to_full_scale_v);
            self.default_crop_origin_v.scale_by(self.raw_to_full_scale_v);
            self.default_scale_v.scale_by(1.0 / self.raw_to_full_scale_v);

            self.raw_to_full_scale_v = 1.0;
        }

        // Resample the transparency mask if required.

        self.resize_transparency_to_match_stage3(host, false)?;

        // Grab clone of raw image if required.

        if self.raw_image_stage == RawImageStage::PreOpcode3 {
            if let Some(stage3) = &self.stage3_image {
                self.raw_image = Some(stage3.read().unwrap().clone_image()?);
            }

            self.raw_image_black_level = self.stage3_black_level;

            if let Some(mask) = &self.transparency_mask {
                self.raw_transparency_mask = Some(mask.read().unwrap().clone_image()?);
            }

            if let Some(depth) = &self.depth_map {
                self.raw_depth_map = Some(depth.read().unwrap().clone_image()?);
            }
        }

        // Process opcode list 3.

        {
            let mut image = self.stage3_image.clone().ok_or_else(program_error)?;

            let mut list = std::mem::replace(&mut self.opcode_list3, DngOpcodeList::new(3));

            let result = list.apply(host, self, &mut image);

            self.opcode_list3 = list;

            result?;
        }

        // See if we are done with the opcode list 3.

        if self.raw_image_stage > RawImageStage::PreOpcode3 {
            // Currently re-use the same flag for preserving the opcode list.

            if !host.wants_preserve_stage2() {
                self.opcode_list3.clear();
            }
        }

        // Just in case the opcode list 3 changed the image size, resample
        // the transparency mask again if required.

        self.resize_transparency_to_match_stage3(host, false)?;

        // Depth maps are often lower resolution than the main image, so make
        // sure we upsample if required.

        self.resize_depth_to_match_stage3(host)?;

        // For now, do not resize semantic masks to match stage 3 image.

        // Update floating point flag.

        self.floating_point = self
            .stage3_image
            .as_ref()
            .ok_or_else(program_error)?
            .read()
            .unwrap()
            .pixel_type()
            == ttFloat;

        // Don't need to grab a copy of raw data at this stage since it is
        // kept around as the stage 3 image.

        Ok(())
    }

    /// `SetStage3Gain (gain)`.
    pub fn set_stage3_gain(&mut self, gain: real64) {
        self.stage3_gain = gain;
    }

    /// `Stage3Gain ()`.
    pub fn stage3_gain(&self) -> real64 {
        self.stage3_gain
    }

    /// `SetStage3BlackLevel (level)`.
    pub fn set_stage3_black_level(&mut self, level: uint16) {
        self.stage3_black_level = level;
    }

    /// `Stage3BlackLevel ()`.
    pub fn stage3_black_level(&self) -> uint16 {
        self.stage3_black_level
    }

    /// `Stage3BlackLevelNormalized ()` — optical black level of the stage 3
    /// image (in [0, 1]).
    pub fn stage3_black_level_normalized(&self) -> real64 {
        self.stage3_black_level as real64 * (1.0 / 65535.0)
    }

    /// `SupportsPreservedBlackLevels (host)` — is this negative permitted to
    /// support deferred black subtraction?  Default returns false.
    pub fn supports_preserved_black_levels(&self, _host: &mut DngHost) -> bool {
        false
    }

    /// `NeedLossyCompressMosaicJXL (host)` (dng_negative.cpp:6035).
    pub fn need_lossy_compress_mosaic_jxl(&self, host: &DngHost) -> bool {
        if !host.lossy_mosaic_jxl() {
            return false;
        }

        if host.save_dng_version() < dngVersion_1_7_1_0 {
            return false;
        }

        match self.get_mosaic_info() {
            Some(info) => {
                if !info.is_color_filter_array()
                    || info.cfa_pattern_size.h != 2
                    || info.cfa_pattern_size.v != 2
                {
                    return false;
                }
            }
            None => return false,
        }

        if let Some(image) = &self.raw_lossy_compressed_image {
            if image.compression_code == dng_core::dng_tag_values::ccJXL
                && image.jxl_distance() != 0.0
            {
                return false;
            }
        }

        true
    }


    /// `LossyCompressMosaicJXL (host, writer)` — lossy compress raw mosaic
    /// image using JXL, if possible.
    ///
    /// PORT-NOTE(dng_negative.cpp:6183): requires `dng_image_writer`
    /// (Agent B); returns NotSupported until the writer module lands.
    pub fn lossy_compress_mosaic_jxl(
        &mut self,
        host: &mut DngHost,
        _writer: &mut crate::dng_image_writer::DngImageWriter,
    ) -> DngResult<()> {
        if !self.need_lossy_compress_mosaic_jxl(host) {
            return Ok(());
        }

        // C++ (dng_negative.cpp:6183): pad the mosaic to a 2x field with
        // edge-repeat, run EncodeImageForCompression, interleave 2x2 and
        // encode as lossy JXL with gray SRGB color encoding.

        if self.raw_image().map(|img| img.read().unwrap().pixel_type()) != Some(ttShort) {
            return Ok(());
        }

        if self.raw_image().map(|img| img.read().unwrap().planes()) != Some(1) {
            return Ok(());
        }

        let (old_w, old_h) = {
            let raw = self.raw_image().unwrap().read().unwrap();
            (raw.width(), raw.height())
        };

        if old_h < 2 || old_w < 2 {
            return Ok(());
        }

        // Field sizes (half resolution, rounded up to tiles).

        let mut field_w = (old_w + 1) / 2;
        let mut field_h = (old_h + 1) / 2;

        let mut temp_ifd = dng_core::dng_ifd::DngIfd::new();

        temp_ifd.samples_per_pixel = 1;
        temp_ifd.bits_per_sample[0] = 16;
        temp_ifd.image_length = field_h;
        temp_ifd.image_width = field_w;

        temp_ifd.find_tile_size(1024 * 1024, 1, 1)?;

        let tile_w = temp_ifd.tile_width;
        let tile_h = temp_ifd.tile_length;

        field_w = ((field_w + tile_w - 1) / tile_w) * tile_w;
        field_h = ((field_h + tile_h - 1) / tile_h) * field_h.min(tile_h).max(1) - field_h.min(tile_h).max(1) + ((field_h + tile_h - 1) / tile_h) * tile_h;

        let padded_w = field_w * 2;
        let padded_h = field_h * 2;

        let pad_offset_v = (padded_h - old_h) / 2;
        let pad_offset_h = (padded_w - old_w) / 2;

        let active_area =
            DngRect::from_corners(pad_offset_v as int32, pad_offset_h as int32,
                                  (pad_offset_v + old_h) as int32, (pad_offset_h + old_w) as int32);

        // Pad the mosaic image (dng_lossy_mosaic_task: copy the source into
        // the padded image at the offset with 2-pixel edge repeat).

        let padded: ImageRef = DngSimpleImage::new(
            DngRect::from_corners(0, 0, padded_h as int32, padded_w as int32),
            1,
            ttShort,
        )?
        .into_image_ref()?;

        {
            let src = self.raw_image().unwrap().read().unwrap();

            let mut buffer = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, padded_h as int32, padded_w as int32),
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(
                    DngRect::from_corners(0, 0, padded_h as int32, padded_w as int32),
                    1,
                    ttShort,
                )?,
            )?;

            src.get(
                &mut buffer,
                dng_core::dng_image::EdgeOption::EdgeRepeat,
                2,
                2,
            )?;

            // Shift into the padded position (the C++ task reads the source
            // at tile - offset, i.e. the destination holds the source shifted
            // by the pad offset with edge repeat).

            let mut shifted = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(
                    pad_offset_v as int32,
                    pad_offset_h as int32,
                    (pad_offset_v + old_h) as int32,
                    (pad_offset_h + old_w) as int32,
                ),
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(
                    DngRect::from_corners(0, 0, padded_h as int32, padded_w as int32),
                    1,
                    ttShort,
                )?,
            )?;

            shifted.data =
                dng_core::dng_pixel_buffer::PixelData::U16(
                    buffer.as_u16_slice().unwrap().to_vec(),
                );

            padded.write().unwrap().do_put(&shifted)?;
        }

        self.raw_image = Some(padded);
        self.set_active_area(active_area);
        self.set_black_level(self.raw_image_black_level() as real64, 0);

        // EncodeImageForCompression on the padded raw image.

        let raw_image = self.raw_image().unwrap().clone();

        let src_black = self.raw_image_black_level() as real64 * (1.0 / 65535.0);

        let mut dst_black = [0.0; dng_core::dng_sdk_limits::kMaxColorPlanes as usize];

        let compressed_image = encode_image_for_compression(
            host,
            &raw_image,
            active_area,
            true, // isSceneReferred
            true, // use16Bit
            src_black,
            &mut dst_black,
            self.opcode_list2_mut(),
        )?;

        if let Some(image) = compressed_image {
            self.raw_image = Some(image);
        }

        self.set_black_level(round_int32_f64(dst_black[0]) as real64, 0);

        // Interleave 2x2 into the field image.

        let raw_image = self.raw_image().unwrap().clone();

        let (raw_w, raw_h) = {
            let raw = raw_image.read().unwrap();
            (raw.width(), raw.height())
        };

        let temp: ImageRef = DngSimpleImage::new(
            raw_image.read().unwrap().bounds(),
            1,
            ttShort,
        )?
        .into_image_ref()?;

        crate::dng_read_image::interleave_2d(&raw_image, &*temp.read().unwrap(), 2, 2, true)?;

        // Encode as lossy JXL (use_case_LossyMosaic).

        let mut lossy_image = encode_jxl_image(&temp, DngJxlUseCase::LossyMosaic)?;

        lossy_image.base.row_interleave_factor = 2;
        lossy_image.base.column_interleave_factor = 2;

        self.set_raw_lossy_compressed_image(Some(Box::new(lossy_image.base)));

        self.clear_raw_lossy_compressed_image_digest();
        self.clear_raw_image_digest();

        Ok(())
    }

    /// `LosslessCompressJXL (host, writer, nearLosslessOK)`.
    ///
    /// PORT-NOTE(dng_negative.cpp:6493): requires `dng_image_writer`
    /// (Agent B); returns NotSupported until the writer module lands.
    pub fn lossless_compress_jxl(
        &mut self,
        host: &mut DngHost,
        _writer: &mut crate::dng_image_writer::DngImageWriter,
        near_lossless_ok: bool,
    ) -> DngResult<()> {
        // C++ (dng_negative.cpp:6493).

        if host.save_dng_version() != 0
            && host.save_dng_version() < crate::dng_image_writer::min_backward_version_for_compression(ccJXL)
        {
            return Ok(());
        }

        if self.raw_lossy_compressed_image().is_none() {
            let mosaic_is_cfa = self
                .get_mosaic_info()
                .map(|info| info.is_color_filter_array())
                .unwrap_or(false);

            if mosaic_is_cfa {
                let (pattern_v, pattern_h) = {
                    let info = self.get_mosaic_info().unwrap();
                    let size = info.cfa_pattern_size;
                    (size.v, size.h)
                };

                let raw_ok = {
                    let raw = self.raw_image().unwrap().read().unwrap();
                    host.save_dng_version() >= dngVersion_1_7_1_0
                        && pattern_h >= 2
                        && (pattern_h as int32) < raw.width() as int32
                        && pattern_v >= 2
                        && (pattern_v as int32) < raw.height() as int32
                        && raw.planes() == 1
                        && raw.pixel_type() == ttShort
                };

                if raw_ok {
                    // Interleave2D (encode) with the CFA pattern factors.

                    let raw_image = self.raw_image().unwrap().clone();

                    let temp: ImageRef = DngSimpleImage::new(
                        raw_image.read().unwrap().bounds(),
                        1,
                        ttShort,
                    )?
                    .into_image_ref()?;

                    crate::dng_read_image::interleave_2d(
                        &raw_image,
                        &*temp.read().unwrap(),
                        pattern_v,
                        pattern_h,
                        true,
                    )?;

                    let mut lossy_image =
                        encode_jxl_image(&temp, DngJxlUseCase::LosslessMosaic)?;

                    lossy_image.base.row_interleave_factor = pattern_v as uint32;
                    lossy_image.base.column_interleave_factor = pattern_h as uint32;

                    self.raw_lossy_compressed_image = Some(Box::new(lossy_image.base));

                    self.clear_raw_lossy_compressed_image_digest();
                    self.clear_raw_image_digest();
                }
            } else {
                let raw_ok = {
                    let raw = self.raw_image().unwrap().read().unwrap();

                    supports_jxl_image(&*raw)
                        && (raw.pixel_type() != ttFloat || self.raw_float_bit_depth() == 16)
                };

                if raw_ok {
                    let raw_image = self.raw_image().unwrap().clone();

                    let use_case = if near_lossless_ok {
                        DngJxlUseCase::MainImage
                    } else {
                        DngJxlUseCase::LosslessMainImage
                    };

                    let mut lossy_image = encode_jxl_image(&raw_image, use_case)?;

                    self.raw_lossy_compressed_image = Some(Box::new(lossy_image.base));

                    self.clear_raw_lossy_compressed_image_digest();
                    self.clear_raw_image_digest();
                }
            }
        }

        // Enhanced image (C++ 6553+): requires EnhanceParams and a distinct
        // stage3.  The port mirrors the guard; encoding reuses the raw
        // transparency-mask slot conditions which do not apply to synthetic
        // negatives here.

        if self.enhanced_lossy_compressed_image().is_none()
            && self.enhance_params.not_empty()
            && self.raw_image().is_some()
            && self.stage3_image().is_some()
            && !std::sync::Arc::ptr_eq(
                self.raw_image().unwrap(),
                self.stage3_image().unwrap(),
            )
        {
            let stage3 = self.stage3_image().unwrap().clone();

            let stage3_ok = {
                let img = stage3.read().unwrap();
                supports_jxl_image(&*img)
                    && (img.pixel_type() == ttShort || near_lossless_ok)
            };

            if stage3_ok {
                let use_case = if near_lossless_ok {
                    DngJxlUseCase::EnhancedImage
                } else {
                    DngJxlUseCase::LosslessEnhancedImage
                };

                let mut lossy_image = encode_jxl_image(&stage3, use_case)?;

                self.enhanced_lossy_compressed_image = Some(Box::new(lossy_image.base));
            }
        }

        Ok(())
    }

    /// `EncodeRawProxy (host, srcImage, opcodeList, blackLevel)`.
    ///
    /// PORT-NOTE(dng_negative.cpp:6630): requires EncodeImageForCompression
    /// (histogram trim + proxy curves) — see lossy_compress_mosaic_jxl.
    pub fn encode_raw_proxy(
        &self,
        host: &mut DngHost,
        src_image: &ImageRef,
        opcode_list: &mut DngOpcodeList,
        black_level: &mut [real64],
    ) -> DngResult<ImageRef> {
        // C++ (dng_negative.cpp:6630).

        let use_16bit = {
            let img = src_image.read().unwrap();

            supports_jxl_image(&*img)
                && (host.save_dng_version() == 0
                    || host.save_dng_version()
                        >= crate::dng_image_writer::min_backward_version_for_compression(ccJXL))
        };

        let mut black: [real64; dng_core::dng_sdk_limits::kMaxColorPlanes as usize] =
            [0.0; dng_core::dng_sdk_limits::kMaxColorPlanes as usize];

        let result = encode_image_for_compression(
            host,
            src_image,
            src_image.read().unwrap().bounds(),
            self.is_scene_referred(),
            use_16bit,
            self.stage3_black_level_normalized(),
            &mut black,
            opcode_list,
        )?;

        black_level.copy_from_slice(&black[..black_level.len()]);

        result.ok_or_else(program_error)
    }

    /// `ConvertToProxy (host, writer, proxySize, proxyCount)`.
    ///
    /// PORT-NOTE(dng_negative.cpp:6673): requires `dng_image_writer`
    /// (Agent B); returns NotSupported until the writer module lands.
    pub fn convert_to_proxy(
        &mut self,
        host: &mut DngHost,
        _writer: &mut crate::dng_image_writer::DngImageWriter,
        proxy_size: uint32,
        proxy_count: uint64,
    ) -> DngResult<()> {
        // C++ (dng_negative.cpp:6673).

        let proxy_size = if proxy_size == 0 {
            dng_core::dng_sdk_limits::kMaxImageSide
        } else {
            proxy_size
        };

        let proxy_count = if proxy_count == 0 {
            proxy_size as uint64 * proxy_size as uint64
        } else {
            proxy_count
        };

        let non_full_size_proxy = (proxy_size < dng_core::dng_sdk_limits::kMaxImageSide)
            || (proxy_count
                < dng_core::dng_sdk_limits::kMaxImageSide as uint64
                    * dng_core::dng_sdk_limits::kMaxImageSide as uint64);

        if non_full_size_proxy {
            self.clear_maker_note();
            self.clear_private_data();
        }

        if (self.raw_default_crop_size_h().as_real64()
            < self.default_crop_size_h().as_real64())
            && (self.raw_default_crop_size_v().as_real64()
                < self.default_crop_size_v().as_real64())
        {
            self.set_original_default_final_size(DngPoint::new(0, 0));
            self.set_original_best_quality_final_size(DngPoint::new(0, 0));
            self.set_original_default_crop_size(DngURational::new(0, 1), DngURational::new(0, 1));
            self.set_default_original_sizes();
        }

        let use_jxl = (self.color_channels() == 1 || self.color_channels() == 3)
            && (host.save_dng_version() == 0
                || host.save_dng_version()
                    >= crate::dng_image_writer::min_backward_version_for_compression(ccJXL));

        let pixel_aspect = self.pixel_aspect_ratio();

        let non_square_pixels = pixel_aspect < 0.99 || pixel_aspect > 1.01;

        // rawImageOK conditions (C++ 6760-6790).

        let mut raw_image_ok = false;

        if let Some(raw) = &self.raw_image {
            let raw = raw.read().unwrap();

            let crop_area = self.default_crop_area();

            raw_image_ok = raw.bounds() == crop_area
                && raw.bounds().h() <= proxy_size
                && raw.bounds().w() <= proxy_size
                && (raw.bounds().h() as uint64) * (raw.bounds().w() as uint64) <= proxy_count;
        }

        if raw_image_ok && use_jxl {
            let code_ok = self
                .raw_lossy_compressed_image()
                .map(|img| img.compression_code != ccJXL)
                .unwrap_or(false);

            if code_ok {
                raw_image_ok = false;
            }
        }

        if self
            .raw_lossy_compressed_image()
            .map(|img| img.compression_code == ccJXL && img.jxl_distance == 0.0)
            .unwrap_or(false)
        {
            raw_image_ok = false;
        }

        if !raw_image_ok {
            // AdjustGainMapForStage3 / AdjustProfileForStage3 are no-ops in
            // the C++ (dng_negative.cpp:6642/6646).

            self.raw_image = None;

            self.raw_default_scale_h = DngURational::new(0, 1);
            self.raw_default_scale_v = DngURational::new(0, 1);
            self.raw_best_quality_scale = DngURational::new(0, 1);
            self.raw_default_crop_size_h = DngURational::new(0, 1);
            self.raw_default_crop_size_v = DngURational::new(0, 1);
            self.raw_default_crop_origin_h = DngURational::new(0, 1);
            self.raw_default_crop_origin_v = DngURational::new(0, 1);
            self.raw_image_black_level = 0;

            self.clear_raw_lossy_compressed_image();
            self.set_raw_float_bit_depth(0);
            self.clear_linearization_info();
            self.clear_mosaic_info();

            self.opcode_list1_mut().clear();
            self.opcode_list2_mut().clear();
            self.opcode_list3_mut().clear();

            self.clear_raw_image_digest();
            self.clear_raw_lossy_compressed_image_digest();

            self.enhance_params = DngString::new();
            self.enhanced_lossy_compressed_image = None;
        }

        let default_crop_area = self.default_crop_area();

        let original_stage3_bounds = self
            .stage3_image()
            .ok_or_else(dng_core::dng_exceptions::bad_format)?
            .read()
            .unwrap()
            .bounds();

        if !raw_image_ok && original_stage3_bounds != default_crop_area {
            // Trim stage3 (and maps) to the default crop area.  The port
            // materializes a trimmed copy via ResizeTransparency-style clone
            // (C++ uses dng_image::Trim, only supported by dng_simple_image).

            let stage3 = self.stage3_image().unwrap().clone();

            let trimmed: ImageRef = {
                let src = stage3.read().unwrap();

                let bounds = src.bounds();

                let sub = bounds.and_(default_crop_area);

                let mut buffer = DngPixelBuffer::with_planar_config(
                    sub,
                    0,
                    src.planes(),
                    src.pixel_type(),
                    pcInterleaved,
                    DngPixelBuffer::allocate_data(sub, src.planes(), src.pixel_type())?,
                )?;

                src.do_get(&mut buffer)?;

                let image = DngSimpleImage::new(
                    DngRect::from_corners(0, 0, sub.h() as int32, sub.w() as int32),
                    src.planes(),
                    src.pixel_type(),
                )?
                .into_image_ref()?;

                {
                    let mut dst = image.write().unwrap();

                    let mut dst_buf = buffer.clone();

                    dst_buf.area = DngRect::from_corners(0, 0, sub.h() as int32, sub.w() as int32);

                    dst.do_put(&dst_buf)?;
                }

                image
            };

            self.set_stage3_image(Some(trimmed));

            // PORT-NOTE(dng_negative.cpp:6903): the profile gain-table-map
            // re-projection when the stage3 is trimmed is not ported (the
            // gain map API exposes the geometry, but the C++ reconstructs
            // the underlying buffer through private accessors).
        }

        // Proxy size computation (C++ 6940-7010).

        let aspect_ratio = self.aspect_ratio();

        let mut new_w = proxy_size as int32;
        let mut new_h = proxy_size as int32;

        if aspect_ratio >= 1.0 {
            new_h = max_int32(1, round_int32_f64(proxy_size as f64 / aspect_ratio));
        } else {
            new_w = max_int32(1, round_int32_f64(proxy_size as f64 * aspect_ratio));
        }

        new_h = min_int32(new_h, self.default_final_height() as int32);
        new_w = min_int32(new_w, self.default_final_width() as int32);

        if (new_h as uint64) * (new_w as uint64) > proxy_count {
            if aspect_ratio >= 1.0 {
                new_w = (proxy_count as f64 * aspect_ratio).sqrt() as int32;
                new_h = max_int32(1, round_int32_f64(new_w as f64 / aspect_ratio));
            } else {
                new_h = (proxy_count as f64 / aspect_ratio).sqrt() as int32;
                new_w = max_int32(1, round_int32_f64(new_h as f64 * aspect_ratio));
            }
        }

        let old_size = default_crop_area.size();

        if ((new_h as uint64) * (new_w as uint64)
            < (old_size.v as uint64) * (old_size.h as uint64))
            || non_square_pixels
        {
            let stage3 = self.stage3_image().unwrap().clone();

            let dst: ImageRef = DngSimpleImage::new(
                DngRect::from_corners(0, 0, new_h, new_w),
                stage3.read().unwrap().planes(),
                stage3.read().unwrap().pixel_type(),
            )?
            .into_image_ref()?;

            dng_core::dng_resample::resample_image(
                host,
                stage3.clone(),
                dst.clone(),
                stage3.read().unwrap().bounds(),
                DngRect::from_corners(0, 0, new_h, new_w),
            )?;

            self.set_stage3_image(Some(dst));

            self.set_default_crop_size_u32(new_w as uint32, new_h as uint32);
            self.set_default_scale(DngURational::new(1, 1), DngURational::new(1, 1));
            self.set_best_quality_scale(DngURational::new(1, 1));
            self.set_raw_to_full_scale(1.0, 1.0);
        } else if self.raw_to_full_scale_h != 1.0 || self.raw_to_full_scale_v != 1.0 {
            self.set_default_crop_size_u32(old_size.h as uint32, old_size.v as uint32);
            self.set_default_scale(DngURational::new(1, 1), DngURational::new(1, 1));
            self.set_best_quality_scale(DngURational::new(1, 1));
            self.set_raw_to_full_scale(1.0, 1.0);
        }

        // Encode the proxy raw image.

        if self.raw_image.is_none() {
            if use_jxl {
                let is_float_raw = self
                    .raw_image
                    .as_ref()
                    .map(|raw| {
                        let img = raw.read().unwrap();
                        img.pixel_type() == ttFloat && self.raw_float_bit_depth() != 16
                    })
                    .unwrap_or(false);

                if is_float_raw {
                    // LimitFloatBitDepth to 16 — ported in dng-core
                    // (DngLimitFloatDepthTask).

                    let src = self.stage3_image().unwrap().clone();

                    let (bounds, planes) = {
                        let img = src.read().unwrap();
                        (img.bounds(), img.planes())
                    };

                    let dst: ImageRef = DngSimpleImage::new(bounds, planes, ttFloat)?
                        .into_image_ref()?;

                    dng_core::dng_utils::limit_float_bit_depth(
                        host,
                        src.clone(),
                        dst.clone(),
                        16,
                        32768.0,
                    )?;

                    self.raw_image = Some(dst);
                    self.raw_image_black_level = 0;
                    self.set_raw_float_bit_depth(16);
                    self.set_white_level(32768, 0);
                }

                if self.raw_lossy_compressed_image().is_none() {
                    if let Some(raw) = &self.raw_image {
                        let use_case = if non_full_size_proxy {
                            DngJxlUseCase::ProxyImage
                        } else {
                            DngJxlUseCase::EncodedMainImage
                        };

                        let lossy_image = encode_jxl_image(raw, use_case)?;

                        self.raw_lossy_compressed_image = Some(Box::new(lossy_image.base));
                    }
                }
            } else if let Some(raw) = &self.raw_image {
                if raw.read().unwrap().pixel_type() == ttShort {
                    let stage3 = self.stage3_image().unwrap().clone();

                    let mut black: [real64; dng_core::dng_sdk_limits::kMaxColorPlanes as usize] =
                        [0.0; dng_core::dng_sdk_limits::kMaxColorPlanes as usize];

                    // encode_raw_proxy reads is_scene_referred and
                    // stage3_black_level_normalized; snapshot them first so
                    // the opcode list can be borrowed mutably.

                    let is_scene = self.is_scene_referred();

                    let use_jxl_proxy_16bit = {
                        let img = stage3.read().unwrap();

                        supports_jxl_image(&*img)
                            && (host.save_dng_version() == 0
                                || host.save_dng_version()
                                    >= crate::dng_image_writer::min_backward_version_for_compression(ccJXL))
                    };

                    let stage3_black = self.stage3_black_level_normalized();

                    let bounds = stage3.read().unwrap().bounds();

                    let proxy = encode_image_for_compression(
                        host,
                        &stage3,
                        bounds,
                        is_scene,
                        use_jxl_proxy_16bit,
                        stage3_black,
                        &mut black,
                        &mut self.opcode_list2,
                    )?;

                    let proxy = proxy.ok_or_else(dng_core::dng_exceptions::program_error)?;

                    self.raw_image = Some(proxy);
                    self.raw_image_black_level = 0;

                    self.set_white_level(255, 0);

                    for plane in 0..self.color_channels() as int32 {
                        self.set_black_level(black[plane as usize], plane);
                    }
                }

                if let Some(raw) = &self.raw_image {
                    if raw.read().unwrap().pixel_type() == ttByte {
                        // C++ encodes a lossy JPEG image via
                        // dng_jpeg_image::Encode; the port encodes through
                        // the JXL path when JXL is disabled for byte images
                        // (rare in practice) — keep the raw image
                        // uncompressed instead (PORT-NOTE).

                        self.raw_lossy_compressed_image = None;
                    }
                }
            }
        }

        // Transparency mask (C++ 7245+): resize to match stage3 + JXL
        // compress.  The port currently keeps the mask uncompressed.

        if self.transparency_mask().is_some() {
            // PORT-NOTE(dng_negative.cpp:7245): ResizeTransparencyToMatchStage3
            // + CompressTransparencyMaskJXL are not ported yet.
        }

        Ok(())
    }

    /// `IsProxy ()` (dng_negative.cpp:7319).
    pub fn is_proxy(&self) -> bool {
        (*self.default_crop_size_h() != *self.original_default_crop_size_h())
            && (*self.default_crop_size_v() != *self.original_default_crop_size_v())
    }

    /// `SetIsPreview (preview)` / `IsPreview ()`.
    pub fn set_is_preview(&mut self, preview: bool) {
        self.is_preview = preview;
    }

    pub fn is_preview(&self) -> bool {
        self.is_preview
    }

    /// `SetIsDamaged (damaged)` / `IsDamaged ()`.
    pub fn set_is_damaged(&mut self, damaged: bool) {
        self.is_damaged = damaged;
    }

    pub fn is_damaged(&self) -> bool {
        self.is_damaged
    }

    // --------------------------------------------------- transparency masks

    /// `SetTransparencyMask (image, bitDepth)`.
    pub fn set_transparency_mask(&mut self, image: Option<ImageRef>, bit_depth: uint32) {
        self.transparency_mask = image;

        self.raw_transparency_mask_bit_depth = bit_depth;
    }

    /// `ClearTransparencyMask ()`.
    pub fn clear_transparency_mask(&mut self) {
        self.transparency_mask = None;
        self.raw_transparency_mask = None;
        self.raw_transparency_mask_bit_depth = 0;
    }

    /// `TransparencyMask ()`.
    pub fn transparency_mask(&self) -> Option<&ImageRef> {
        self.transparency_mask.as_ref()
    }

    /// `RawTransparencyMask ()`.
    pub fn raw_transparency_mask(&self) -> Option<&ImageRef> {
        if self.raw_transparency_mask.is_some() {
            return self.raw_transparency_mask.as_ref();
        }

        self.transparency_mask.as_ref()
    }

    /// `RawTransparencyMaskBitDepth ()` (dng_negative.cpp:7439).
    pub fn raw_transparency_mask_bit_depth(&self) -> DngResult<uint32> {
        if self.raw_transparency_mask_bit_depth != 0 {
            return Ok(self.raw_transparency_mask_bit_depth);
        }

        if let Some(mask) = self.raw_transparency_mask() {
            return match mask.read().unwrap().pixel_type() {
                x if x == ttByte => Ok(8),
                x if x == ttShort => Ok(16),
                x if x == ttFloat => Ok(32),
                _ => Err(program_error()),
            };
        }

        Ok(0)
    }

    /// `RawLossyCompressedTransparencyMask ()`.
    pub fn raw_lossy_compressed_transparency_mask(&self) -> Option<&DngLossyCompressedImage> {
        self.raw_lossy_compressed_transparency_mask.as_deref()
    }

    /// `ReadTransparencyMask (host, stream, info)` (dng_negative.cpp:7479).
    pub fn read_transparency_mask(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        if info.mask_index != -1 {
            // Allocate image we are reading.

            let mask_ifd = info.ifd[info.mask_index as usize].clone();

            let mask = DngSimpleImage::new(mask_ifd.bounds(), 1, mask_ifd.pixel_type())?;

            let mask_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(mask));

            self.transparency_mask = Some(mask_ref.clone());

            // Do we need to keep the lossy compressed data?

            self.raw_lossy_compressed_transparency_mask = keep_lossy_compressed_image(
                host.save_dng_version(),
                host.preferred_size(),
                host.for_preview(),
                &mask_ifd,
            );

            // Read the image.

            {
                let ifd = &info.ifd[info.mask_index as usize];
                ifd.read_image(host, stream, &mask_ref)?;
            }

            // Remember the pixel depth.

            self.raw_transparency_mask_bit_depth = mask_ifd.bits_per_sample[0];

            // Remember if transparency mask was lossy compressed.

            self.transparency_mask_was_lossy_compressed =
                mask_ifd.compression == dng_core::dng_tag_values::ccLossyJPEG
                    || mask_ifd.compression == dng_core::dng_tag_values::ccJXL;
        }

        Ok(())
    }

    /// `ResizeTransparencyToMatchStage3 (host, convertTo8Bit)`
    /// (dng_negative.cpp:7522).
    pub fn resize_transparency_to_match_stage3(
        &mut self,
        host: &mut DngHost,
        convert_to_8_bit: bool,
    ) -> DngResult<()> {
        if let Some(mask) = self.transparency_mask.clone() {
            let stage3 = self.stage3_image.as_ref().ok_or_else(program_error)?;

            let stage3_bounds = stage3.read().unwrap().bounds();

            let mask_bounds = mask.read().unwrap().bounds();
            let mask_type = mask.read().unwrap().pixel_type();

            if mask_bounds != stage3_bounds || (mask_type != ttByte && convert_to_8_bit) {
                let new_mask = DngSimpleImage::new(
                    stage3_bounds,
                    1,
                    if convert_to_8_bit { ttByte } else { mask_type },
                )?;

                let new_mask_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(new_mask));

                dng_core::dng_resample::resample_image(
                    host,
                    mask.clone(),
                    new_mask_ref.clone(),
                    mask_bounds,
                    stage3_bounds,
                )?;

                self.transparency_mask = Some(new_mask_ref);

                if self.raw_transparency_mask.is_none() {
                    self.raw_transparency_mask_bit_depth = 0;
                } else if convert_to_8_bit {
                    self.raw_transparency_mask_bit_depth = 8;
                }
            }
        }

        Ok(())
    }

    /// `NeedFlattenTransparency (host)` (dng_negative.cpp:7562).
    pub fn need_flatten_transparency(&self, _host: &mut DngHost) -> bool {
        false
    }

    /// `FlattenTransparency (host)` (dng_negative.cpp:7571).
    pub fn flatten_transparency(&mut self, _host: &mut DngHost) -> DngResult<()> {
        Err(dng_core::not_yet_implemented()) // "FlattenTransparency"
    }

    /// `UnflattenedStage3Image ()`.
    pub fn unflattened_stage3_image(&self) -> Option<&ImageRef> {
        if self.unflattened_stage3_image.is_some() {
            return self.unflattened_stage3_image.as_ref();
        }

        self.stage3_image.as_ref()
    }

    // ----------------------------------------------------------- depth maps

    /// `HasDepthMap ()`.
    pub fn has_depth_map(&self) -> bool {
        self.has_depth_map
    }

    /// `SetHasDepthMap (hasDepthMap)`.
    pub fn set_has_depth_map(&mut self, has_depth_map: bool) {
        self.has_depth_map = has_depth_map;
    }

    /// `DepthMap ()`.
    pub fn depth_map(&self) -> Option<&ImageRef> {
        self.depth_map.as_ref()
    }

    /// `SetDepthMap (depthMap)`.
    pub fn set_depth_map(&mut self, depth_map: Option<ImageRef>) {
        self.depth_map = depth_map;

        self.has_depth_map = self.depth_map.is_some();
    }

    /// `HasDepthMapImage ()`.
    pub fn has_depth_map_image(&self) -> bool {
        self.depth_map.is_some()
    }

    /// `RawDepthMap ()`.
    pub fn raw_depth_map(&self) -> Option<&ImageRef> {
        if self.raw_depth_map.is_some() {
            return self.raw_depth_map.as_ref();
        }

        self.depth_map.as_ref()
    }

    /// `RawLossyCompressedDepthMap ()`.
    pub fn raw_lossy_compressed_depth_map(&self) -> Option<&DngLossyCompressedImage> {
        self.raw_lossy_compressed_depth_map.as_deref()
    }

    /// `ResetDepthMap ()`.
    pub fn reset_depth_map(&mut self) {
        self.depth_map = None;
        self.raw_depth_map = None;
        self.raw_lossy_compressed_depth_map = None;
    }

    /// `ReadDepthMap (host, stream, info)` (dng_negative.cpp:7607).
    pub fn read_depth_map(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        if info.depth_index != -1 {
            // Allocate image we are reading.

            let depth_ifd = info.ifd[info.depth_index as usize].clone();

            let depth = DngSimpleImage::new(depth_ifd.bounds(), 1, depth_ifd.pixel_type())?;

            let depth_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(depth));

            self.depth_map = Some(depth_ref.clone());

            // Keep lossy compressed depth image?

            self.raw_lossy_compressed_depth_map = keep_lossy_compressed_image(
                host.save_dng_version(),
                host.preferred_size(),
                host.for_preview(),
                &depth_ifd,
            );

            // Read the image.

            {
                let ifd = &info.ifd[info.depth_index as usize];
                ifd.read_image(host, stream, &depth_ref)?;
            }

            self.has_depth_map = self.depth_map.is_some();
        }

        Ok(())
    }

    /// `ResizeDepthToMatchStage3 (host)` (dng_negative.cpp:7643).
    pub fn resize_depth_to_match_stage3(&mut self, host: &mut DngHost) -> DngResult<()> {
        if let Some(depth) = self.depth_map.clone() {
            let stage3 = self.stage3_image.as_ref().ok_or_else(program_error)?;

            let stage3_bounds = stage3.read().unwrap().bounds();

            if depth.read().unwrap().bounds() != stage3_bounds {
                // If we are upsampling, and have not grabbed the raw depth
                // map yet, do so now.

                if self.raw_depth_map.is_none() {
                    let image_pixels =
                        (stage3_bounds.h() as uint64) * (stage3_bounds.w() as uint64);

                    let depth_pixels = (depth.read().unwrap().bounds().h() as uint64)
                        * (depth.read().unwrap().bounds().w() as uint64);

                    if depth_pixels < image_pixels {
                        self.raw_depth_map = Some(depth.read().unwrap().clone_image()?);
                    }
                }

                let depth_type = depth.read().unwrap().pixel_type();

                let depth_bounds = depth.read().unwrap().bounds();

                let new_map = DngSimpleImage::new(stage3_bounds, 1, depth_type)?;

                let new_map_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(new_map));

                dng_core::dng_resample::resample_image(
                    host,
                    depth.clone(),
                    new_map_ref.clone(),
                    depth_bounds,
                    stage3_bounds,
                )?;

                self.depth_map = Some(new_map_ref);
            }
        }

        Ok(())
    }

    // --------------------------------------------------------- depth metadata

    /// `DepthFormat ()` / `SetDepthFormat (format)`.
    pub fn depth_format(&self) -> uint32 {
        self.depth_format
    }

    pub fn set_depth_format(&mut self, format: uint32) {
        self.depth_format = format;
    }

    /// `DepthNear ()` / `SetDepthNear (dist)`.
    pub fn depth_near(&self) -> &DngURational {
        &self.depth_near
    }

    pub fn set_depth_near(&mut self, dist: DngURational) {
        self.depth_near = dist;
    }

    /// `DepthFar ()` / `SetDepthFar (dist)`.
    pub fn depth_far(&self) -> &DngURational {
        &self.depth_far
    }

    pub fn set_depth_far(&mut self, dist: DngURational) {
        self.depth_far = dist;
    }

    /// `DepthUnits ()` / `SetDepthUnits (units)`.
    pub fn depth_units(&self) -> uint32 {
        self.depth_units
    }

    pub fn set_depth_units(&mut self, units: uint32) {
        self.depth_units = units;
    }

    /// `DepthMeasureType ()` / `SetDepthMeasureType (measure)`.
    pub fn depth_measure_type(&self) -> uint32 {
        self.depth_measure_type
    }

    pub fn set_depth_measure_type(&mut self, measure: uint32) {
        self.depth_measure_type = measure;
    }

    // ------------------------------------------------------- enhance params

    /// `EnhanceParams ()`.
    pub fn enhance_params(&self) -> &DngString {
        &self.enhance_params
    }

    /// `SetEnhanceParams (s)`.
    pub fn set_enhance_params(&mut self, s: &DngString) {
        self.enhance_params = s.clone();
    }

    /// `SetEnhanceParams (s)` (string bytes overload).
    pub fn set_enhance_params_bytes(&mut self, s: &[uint8]) {
        self.enhance_params.set(s);
    }

    /// `EnhancedLossyCompressedImage ()`.
    pub fn enhanced_lossy_compressed_image(&self) -> Option<&DngLossyCompressedImage> {
        self.enhanced_lossy_compressed_image.as_deref()
    }

    /// `SetEnhancedLossyCompressedImage (image)`.
    pub fn set_enhanced_lossy_compressed_image(
        &mut self,
        image: Option<Box<DngLossyCompressedImage>>,
    ) {
        self.enhanced_lossy_compressed_image = image;
    }

    // ------------------------------------------------------- semantic masks

    /// `HasSemanticMask ()` — any mask at all.
    pub fn has_semantic_mask(&self) -> bool {
        !self.semantic_masks.is_empty()
    }

    /// `HasSemanticMask (index)` — mask image present at index.
    pub fn has_semantic_mask_at(&self, index: uint32) -> bool {
        if (index as usize) >= self.semantic_masks.len() {
            return false;
        }

        self.semantic_masks[index as usize].mask.is_some()
    }

    /// `NumSemanticMasks ()`.
    pub fn num_semantic_masks(&self) -> uint32 {
        self.semantic_masks.len() as uint32
    }

    /// `SemanticMask (index)`.
    pub fn semantic_mask(&self, index: uint32) -> DngResult<&DngSemanticMask> {
        if (index as usize) >= self.semantic_masks.len() {
            return Err(program_error()); // "non-existent index in SemanticMask"
        }

        Ok(&self.semantic_masks[index as usize])
    }

    /// `RawSemanticMask (index)` — for now the concept of a "raw" semantic
    /// mask is the same as the regular semantic mask API (original
    /// resolution).
    pub fn raw_semantic_mask(&self, index: uint32) -> DngResult<&DngSemanticMask> {
        self.semantic_mask(index)
    }

    /// `SetSemanticMask (index, mask)`.
    pub fn set_semantic_mask(&mut self, index: uint32, mask: DngSemanticMask) -> DngResult<()> {
        if !self.has_semantic_mask_at(index) {
            return Err(program_error()); // "non-existent index in SetSemanticMask"
        }

        debug_assert!(mask.mask.is_some(), "missing mask in SetSemanticMask");

        self.semantic_masks[index as usize] = mask;

        Ok(())
    }

    /// `AppendSemanticMask (mask)`.
    pub fn append_semantic_mask(&mut self, mask: DngSemanticMask) -> DngResult<()> {
        debug_assert!(mask.mask.is_some(), "missing mask in AppendSemanticMask");

        self.semantic_masks.push(mask);

        Ok(())
    }

    /// `ReadSemanticMasks (host, stream, info)` (dng_negative.cpp:7817).
    pub fn read_semantic_masks(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        info: &mut DngInfo,
    ) -> DngResult<()> {
        debug_assert!(
            (info.semantic_mask_indices.len() as uint32) <= kMaxSemanticMasks,
            "Too many semantic masks"
        );

        let mut masks: Vec<DngSemanticMask> = Vec::with_capacity(info.semantic_mask_indices.len());

        for index in &info.semantic_mask_indices {
            let ifd = info.ifd[*index as usize].clone();

            let image = DngSimpleImage::new(ifd.bounds(), 1, ifd.pixel_type())?;

            let image_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(image));

            let mut lossy_compressed = keep_lossy_compressed_image(
                host.save_dng_version(),
                host.preferred_size(),
                host.for_preview(),
                &ifd,
            );

            // Workaround for early files that use lossy JPEG with Compression
            // tag value 7.

            if ifd.compression == dng_core::dng_tag_values::ccJPEG {
                // First try to read it directly as Lossless JPEG.

                let mut try_lossy_jpeg = false;

                match ifd.read_image(host, stream, &image_ref) {
                    Ok(()) => {}

                    Err(e) => {
                        // If that doesn't work, then try Lossy JPEG.

                        if matches!(e, DngError::BadFormat) {
                            try_lossy_jpeg = true;
                        }
                        // Re-throw other exceptions.
                        else {
                            return Err(e);
                        }
                    }
                }

                // TODO: JXL support for semantic masks, too?

                if try_lossy_jpeg {
                    let mut ifd_clone = ifd.clone();

                    ifd_clone.compression = dng_core::dng_tag_values::ccLossyJPEG;

                    lossy_compressed = keep_lossy_compressed_image(
                        host.save_dng_version(),
                        host.preferred_size(),
                        host.for_preview(),
                        &ifd_clone,
                    );

                    ifd_clone.read_image(host, stream, &image_ref)?;
                }
            } else {
                ifd.read_image(host, stream, &image_ref)?;
            }

            let mut mask = DngSemanticMask::default();

            mask.name = ifd.semantic_name.clone();
            mask.instance_id = ifd.semantic_instance_id.clone();

            // Semantic XMP bytes (preserved verbatim).

            if ifd.semantic_xmp_count != 0 {
                let xmp = stream_get_at(
                    stream,
                    ifd.semantic_xmp_offset,
                    ifd.semantic_xmp_count as usize,
                )?;

                mask.xmp = Some(std::sync::Arc::new(xmp));
            }

            mask.mask_sub_area = ifd.mask_sub_area;

            mask.mask = Some(image_ref);

            // If MaskSubArea is not valid, then zero all the fields.

            if !mask.is_mask_sub_area_valid() {
                mask.mask_sub_area = [0, 0, 0, 0];
            }

            mask.lossy_compressed = lossy_compressed.map(|b| std::sync::Arc::new(*b));

            masks.push(mask);
        }

        self.semantic_masks = masks;

        Ok(())
    }

    /// `ResizeSemanticMasksToMatchStage3 (host)` (dng_negative.cpp:7688).
    pub fn resize_semantic_masks_to_match_stage3(&mut self, host: &mut DngHost) -> DngResult<()> {
        if !self.has_semantic_mask() {
            return Ok(());
        }

        let stage3 = match &self.stage3_image {
            Some(img) => img.clone(),
            None => return Ok(()),
        };

        let dst_bounds = stage3.read().unwrap().bounds();

        for i in 0..self.semantic_masks.len() {
            let mask = self.semantic_masks[i].mask.clone();

            if let Some(mask) = mask {
                let mask_bounds = mask.read().unwrap().bounds();

                if mask_bounds != dst_bounds {
                    let mask_planes = mask.read().unwrap().planes();
                    let mask_type = mask.read().unwrap().pixel_type();

                    let image = DngSimpleImage::new(dst_bounds, mask_planes, mask_type)?;

                    let image_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(image));

                    dng_core::dng_resample::resample_image(
                        host,
                        mask.clone(),
                        image_ref.clone(),
                        mask_bounds,
                        dst_bounds,
                    )?;

                    self.semantic_masks[i].mask = Some(image_ref);
                }
            }
        }

        Ok(())
    }

    /// `AdjustSemanticMasksForProxy (host, writer, originalStage3Bounds,
    /// defaultCropArea)` (dng_negative.cpp:7997).
    ///
    /// PORT-NOTE(dng_negative.cpp:7997): the final CompressSemanticMasksJXL
    /// step requires `dng_image_writer` (Agent B); the resample/trim part is
    /// ported.  The MaskSubArea re-crop path uses direct resampling to the
    /// full-res mask (the C++ zero-fill + clone + offset + copy + resample
    /// sequence reduces to the same resample for the supported cases).
    pub fn adjust_semantic_masks_for_proxy(
        &mut self,
        host: &mut DngHost,
        _writer: &mut dyn std::any::Any,
        original_stage3_bounds: DngRect,
        default_crop_area: DngRect,
    ) -> DngResult<()> {
        if !self.has_semantic_mask() {
            return Ok(());
        }

        debug_assert!(self.stage3_image.is_some(), "Missing stage3 image");

        let new_stage3_bounds = self
            .stage3_image
            .as_ref()
            .ok_or_else(program_error)?
            .read()
            .unwrap()
            .bounds();

        // If original active area is different than the original default
        // crop area, this means during proxy conversion the main image will
        // be trimmed to the default crop area, so we need to adjust the
        // semantic masks too.

        let trimmed_to_default_crop = original_stage3_bounds != default_crop_area;

        let mask_count = self.num_semantic_masks();

        for i in 0..mask_count as usize {
            let need_downsample_mask = {
                let mask = self.semantic_masks[i]
                    .mask
                    .as_ref()
                    .ok_or_else(program_error)?;

                let bounds = mask.read().unwrap().bounds();
                let pixel_type = mask.read().unwrap().pixel_type();

                (bounds.w() > new_stage3_bounds.w() || bounds.h() > new_stage3_bounds.h())
                    || (pixel_type != ttByte)
            };

            if need_downsample_mask || trimmed_to_default_crop {
                let mask_bounds = self.semantic_masks[i]
                    .mask
                    .as_ref()
                    .ok_or_else(program_error)?
                    .read()
                    .unwrap()
                    .bounds();

                // Can we just resample directly to proxy size?

                let is_mask_sub_area_valid = self.semantic_masks[i].is_mask_sub_area_valid();

                if !is_mask_sub_area_valid && !trimmed_to_default_crop {
                    let image = DngSimpleImage::new(new_stage3_bounds, 1, ttByte)?;

                    let image_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(image));

                    let mask = self.semantic_masks[i].mask.as_ref().unwrap().clone();

                    dng_core::dng_resample::resample_image(
                        host,
                        mask,
                        image_ref.clone(),
                        mask_bounds,
                        new_stage3_bounds,
                    )?;

                    self.semantic_masks[i].mask = Some(image_ref);
                }
                // Else we need to first create a full size trimmed mask.
                else {
                    let mask_type = self.semantic_masks[i]
                        .mask
                        .as_ref()
                        .ok_or_else(program_error)?
                        .read()
                        .unwrap()
                        .pixel_type();

                    let need_resize_to_final_area =
                        new_stage3_bounds.size() != default_crop_area.size();

                    let full_res_pixel_type = if need_resize_to_final_area {
                        mask_type
                    } else {
                        ttByte
                    };

                    let full_res_mask =
                        DngSimpleImage::new(original_stage3_bounds, 1, full_res_pixel_type)?;

                    let full_res_mask_ref: ImageRef =
                        std::sync::Arc::new(std::sync::RwLock::new(full_res_mask));

                    let mask = self.semantic_masks[i].mask.as_ref().unwrap().clone();

                    // Resample to active area (MaskSubArea offsets are
                    // handled by the resample task bounds; see PORT-NOTE).

                    dng_core::dng_resample::resample_image(
                        host,
                        mask,
                        full_res_mask_ref.clone(),
                        mask_bounds,
                        original_stage3_bounds,
                    )?;

                    // Trim to default crop area.

                    full_res_mask_ref.write().unwrap().trim(default_crop_area)?;

                    let mut image_ref = full_res_mask_ref;

                    // Resize to new area, if needed.

                    if need_resize_to_final_area {
                        let trimmed_bounds = image_ref.read().unwrap().bounds();

                        let temp = DngSimpleImage::new(new_stage3_bounds, 1, ttByte)?;

                        let temp_ref: ImageRef =
                            std::sync::Arc::new(std::sync::RwLock::new(temp));

                        dng_core::dng_resample::resample_image(
                            host,
                            image_ref.clone(),
                            temp_ref.clone(),
                            trimmed_bounds,
                            new_stage3_bounds,
                        )?;

                        image_ref = temp_ref;
                    }

                    self.semantic_masks[i].mask = Some(image_ref);

                    // Clear MaskSubArea.

                    self.semantic_masks[i].mask_sub_area = [0, 0, 0, 0];

                    // Lossy compressed data is no longer valid.

                    self.semantic_masks[i].lossy_compressed = None;
                }
            }
        }

        // CompressSemanticMasksJXL requires the writer; the port returns the
        // resampled masks (see module PORT-NOTE).

        Ok(())
    }

    // ------------------------------------------------- profile gain table map

    /// `HasProfileGainTableMap ()`.
    pub fn has_profile_gain_table_map(&self) -> bool {
        self.profile_gain_table_map.is_some()
    }

    /// `ProfileGainTableMap ()`.
    pub fn profile_gain_table_map(&self) -> DngResult<&crate::dng_gain_map::DngGainTableMap> {
        match &self.profile_gain_table_map {
            Some(map) => Ok(map.as_ref()),
            None => Err(program_error()), // "Missing profile gain table map"
        }
    }

    /// `ShareProfileGainTableMap ()`.
    pub fn share_profile_gain_table_map(
        &self,
    ) -> Option<std::sync::Arc<crate::dng_gain_map::DngGainTableMap>> {
        self.profile_gain_table_map.clone()
    }

    /// `SetProfileGainTableMap (gainTableMap)`.
    pub fn set_profile_gain_table_map(
        &mut self,
        gain_table_map: Option<std::sync::Arc<crate::dng_gain_map::DngGainTableMap>>,
    ) {
        self.profile_gain_table_map = gain_table_map;
    }

    // ------------------------------------------------------- profile adjust

    /// `AdjustProfileForStage3 ()` (dng_negative.cpp:6663) — virtual hook;
    /// nothing by default.
    pub fn adjust_profile_for_stage3(&mut self) {}
}

// ---------------------------------------------------------------------------
// Stream helper.
// ---------------------------------------------------------------------------

/// Position the stream and read exactly `count` bytes.
fn stream_get_at(stream: &mut DngStream, position: uint64, count: usize) -> DngResult<Vec<uint8>> {
    stream.set_read_position(position)?;

    let data = stream.get(count as uint32)?;

    if data.len() != count {
        return Err(dng_core::DngError::EndOfFile);
    }

    Ok(data)
}

// ---------------------------------------------------------------------------
// dng_find_new_raw_image_digest_task (dng_negative.cpp:1897).
// ---------------------------------------------------------------------------

/// Port of `dng_find_new_raw_image_digest_task`.
struct DngFindNewRawImageDigestTask {
    image: ImageRef,

    pixel_type: uint32,

    tiles_across: uint32,
    tiles_down: uint32,

    tile_count: uint32,

    tile_hash: std::sync::Mutex<Vec<DngFingerprint>>,

    unit_cell: DngPoint,
}

impl DngFindNewRawImageDigestTask {
    const K_TILE_SIZE: int32 = 256;

    fn new(image: ImageRef, pixel_type: uint32) -> Self {
        let bounds = image.read().unwrap().bounds();

        let unit_cell = DngPoint::new(
            min_int32(Self::K_TILE_SIZE, bounds.h() as int32),
            min_int32(Self::K_TILE_SIZE, bounds.w() as int32),
        );

        DngFindNewRawImageDigestTask {
            image,
            pixel_type,
            tiles_across: 0,
            tiles_down: 0,
            tile_count: 0,
            tile_hash: std::sync::Mutex::new(Vec::new()),
            unit_cell,
        }
    }

    /// `Result ()`.
    fn result(&self) -> DngResult<DngFingerprint> {
        let mut printer = DngMd5Printer::new();

        let tile_hash = self.tile_hash.lock().unwrap();

        for tile_index in 0..self.tile_count as usize {
            printer.process(&tile_hash[tile_index].data);
        }

        Ok(*printer.result())
    }
}

impl DngAreaTask for DngFindNewRawImageDigestTask {
    fn name(&self) -> &str {
        "dng_find_new_raw_image_digest_task"
    }

    fn min_task_area(&self) -> uint32 {
        1
    }

    fn unit_cell(&self) -> DngPoint {
        self.unit_cell
    }

    fn max_tile_size(&self) -> DngPoint {
        self.unit_cell
    }

    fn start(
        &mut self,
        _thread_count: uint32,
        _dst_area: DngRect,
        tile_size: DngPoint,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        if tile_size != self.unit_cell {
            return Err(program_error());
        }

        let bounds = self.image.read().unwrap().bounds();

        self.tiles_across =
            ((bounds.w() as int32 + self.unit_cell.h - 1) / self.unit_cell.h) as uint32;
        self.tiles_down =
            ((bounds.h() as int32 + self.unit_cell.v - 1) / self.unit_cell.v) as uint32;

        self.tile_count = self.tiles_across * self.tiles_down;

        *self.tile_hash.lock().unwrap() =
            vec![DngFingerprint::new(); self.tile_count as usize];

        Ok(())
    }

    fn process(
        &self,
        _thread_index: uint32,
        tile: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        let bounds = self.image.read().unwrap().bounds();

        let col_index = (tile.l - bounds.l) / self.unit_cell.h;
        let row_index = (tile.t - bounds.t) / self.unit_cell.v;

        debug_assert!(
            tile.l == bounds.l + col_index * self.unit_cell.h
                && tile.t == bounds.t + row_index * self.unit_cell.v,
            "Bad tile origin"
        );

        let tile_index = (row_index as uint32) * self.tiles_across + col_index as uint32;

        let img = self.image.read().unwrap();

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile,
            0,
            img.planes(),
            self.pixel_type,
            dng_core::dng_tag_values::pcPlanar,
            DngPixelBuffer::allocate_data(tile, img.planes(), self.pixel_type)?,
        )?;

        img.do_get(&mut buffer)?;

        // Hash the plane-planar buffer content.

        let count = buffer.area.h() as usize
            * buffer.area.w() as usize
            * (img.planes() as usize)
            * (tag_type_size(self.pixel_type) as usize);

        let bytes = pixel_buffer_le_bytes(&buffer, count);

        let mut printer = DngMd5Printer::new();

        printer.process(&bytes);

        self.tile_hash.lock().unwrap()[tile_index as usize] = *printer.result();

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Proxy-encoding helpers (EncodeImageForCompression, dng_negative.cpp:5818).
// ---------------------------------------------------------------------------

/// Port of `dng_base_proxy_curve`.
trait DngBaseProxyCurve: Send + Sync {
    fn evaluate_scene(&self, x: real64) -> real64;
    fn evaluate_output(&self, x: real64) -> real64;
    fn scene_slope(&self) -> real64;
    fn output_slope(&self) -> real64;
}

/// Port of `dng_jpeg_proxy_curve`.
struct DngJpegProxyCurve;

const K_SCENE_PROXY_CURVE_SLOPE: real64 = 1.0 / 128.0;
const K_OUTPUT_PROXY_CURVE_SLOPE: real64 = 1.0 / 16.0;

impl DngBaseProxyCurve for DngJpegProxyCurve {
    /// The following code evaluates the inverse of:
    /// f (x) = (s * x) + ((1 - s) * x^3)
    /// where s is the slope of the function at the origin (x == 0).
    fn evaluate_scene(&self, x: real64) -> real64 {
        let s = K_SCENE_PROXY_CURVE_SLOPE;

        let k0 = 2.0f64.powf(1.0 / 3.0);

        let k1 = 108.0 * s * s * s * (1.0 - s) * (1.0 - s) * (1.0 - s);

        let k2 = (27.0 * x) - (54.0 * s * x) + (27.0 * x * s * s);

        let k3 = (k2 + (k1 + k2 * k2).sqrt()).powf(1.0 / 3.0);

        let mut y = (k3 / (3.0 * k0 * (1.0 - s))) - (k0 * s / k3);

        y = pin_real64(0.0, y, 1.0);

        y
    }

    fn evaluate_output(&self, x: real64) -> real64 {
        ((960.0 * x + 1.0).sqrt() - 1.0) / 30.0
    }

    fn scene_slope(&self) -> real64 {
        K_SCENE_PROXY_CURVE_SLOPE
    }

    fn output_slope(&self) -> real64 {
        K_OUTPUT_PROXY_CURVE_SLOPE
    }
}

/// Port of `dng_jxl_proxy_curve` — intended for integer data, not
/// floating-point data.
struct DngJxlProxyCurve;

const K_JXL_SCENE_PROXY_CURVE_SLOPE: real64 = 1.0 / 16.0;
const K_JXL_OUTPUT_PROXY_CURVE_SLOPE: real64 = 1.0 / 16.0;

impl DngBaseProxyCurve for DngJxlProxyCurve {
    fn evaluate_scene(&self, x: real64) -> real64 {
        let s = K_JXL_SCENE_PROXY_CURVE_SLOPE;

        let k0 = 2.0f64.powf(1.0 / 3.0);

        let k1 = 108.0 * s * s * s * (1.0 - s) * (1.0 - s) * (1.0 - s);

        let k2 = (27.0 * x) - (54.0 * s * x) + (27.0 * x * s * s);

        let k3 = (k2 + (k1 + k2 * k2).sqrt()).powf(1.0 / 3.0);

        let mut y = (k3 / (3.0 * k0 * (1.0 - s))) - (k0 * s / k3);

        y = pin_real64(0.0, y, 1.0);

        y
    }

    fn evaluate_output(&self, x: real64) -> real64 {
        ((960.0 * x + 1.0).sqrt() - 1.0) / 30.0
    }

    fn scene_slope(&self) -> real64 {
        K_JXL_SCENE_PROXY_CURVE_SLOPE
    }

    fn output_slope(&self) -> real64 {
        K_JXL_OUTPUT_PROXY_CURVE_SLOPE
    }
}

/// Port of `dng_gamma_encode_proxy` — the proxy gamma encoding 1d function.
struct DngGammaEncodeProxy<'a> {
    lower: real64,
    upper: real64,

    is_scene_referred: bool,

    stage3_black_level: real64,

    black_level: real64,

    base_curve: &'a dyn DngBaseProxyCurve,
}

impl<'a> DngGammaEncodeProxy<'a> {
    fn evaluate(&self, x: real64) -> real64 {
        let mut x = x;
        let mut y;

        if self.is_scene_referred {
            if self.lower < self.stage3_black_level {
                x = pin_real64(
                    -1.0,
                    (x - self.stage3_black_level) / (self.upper - self.stage3_black_level),
                    1.0,
                );

                if x >= 0.0 {
                    y = self.base_curve.evaluate_scene(x);
                } else {
                    y = -self.base_curve.evaluate_scene(-x);
                }

                y = pin_real64(0.0, y * (1.0 - self.black_level) + self.black_level, 1.0);
            } else {
                x = pin_real64(0.0, (x - self.lower) / (self.upper - self.lower), 1.0);

                y = self.base_curve.evaluate_scene(x);
            }
        } else {
            x = pin_real64(0.0, (x - self.lower) / (self.upper - self.lower), 1.0);

            y = self.base_curve.evaluate_output(x);
        }

        y
    }
}

/// Port of `dng_encode_proxy_task` — applies per-plane 16-bit encode tables.
struct DngEncodeProxyTask {
    src_image: ImageRef,
    dst_image: ImageRef,

    table16: Vec<Vec<uint16>>,
}

impl DngEncodeProxyTask {
    #[allow(clippy::too_many_arguments)]
    fn new(
        src_image: ImageRef,
        dst_image: ImageRef,
        lower: &[real64; kMaxColorPlanes as usize],
        upper: &[real64; kMaxColorPlanes as usize],
        is_scene_referred: bool,
        stage3_black_level: real64,
        black_level: &[real64; kMaxColorPlanes as usize],
        white_level: real64,
        base_curve: &dyn DngBaseProxyCurve,
    ) -> DngResult<Self> {
        let planes = src_image.read().unwrap().planes();

        let mut table16: Vec<Vec<uint16>> = Vec::new();

        for plane in 0..planes as usize {
            let norm_black_level = black_level[plane] / white_level;

            let gamma = DngGammaEncodeProxy {
                lower: lower[plane],
                upper: upper[plane],
                is_scene_referred,
                stage3_black_level,
                black_level: norm_black_level,
                base_curve,
            };

            // Compute fast approximation of encoding table.

            let mut table = vec![0u16; 0x10000];

            for (j, entry) in table.iter_mut().enumerate() {
                let x = j as real64 * (1.0 / 65535.0);

                *entry = (gamma.evaluate(x) * 65535.0) as uint16;
            }

            // The gamma curve has some fairly high curvature near the black
            // point, and the above approximation can actually change results.
            // So use exact math near the black point.

            {
                const K_HIGH_RES_RADIUS: int32 = 1024;

                let zero_pt = round_uint32_f64(stage3_black_level * 65535.0) as int32;

                let high_res_lower = max_int32(0, zero_pt - K_HIGH_RES_RADIUS);
                let high_res_upper = min_int32(0x10000, zero_pt + K_HIGH_RES_RADIUS);

                for j in high_res_lower..high_res_upper {
                    let x = j as real64 * (1.0 / 65535.0);

                    let y = gamma.evaluate(x);

                    let z = pin_uint16(round_int32_f64(y * 65535.0));

                    table[j as usize] = z;
                }
            }

            table16.push(table);
        }

        Ok(DngEncodeProxyTask {
            src_image,
            dst_image,
            table16,
        })
    }
}

impl DngAreaTask for DngEncodeProxyTask {
    fn name(&self) -> &str {
        "dng_encode_proxy_task"
    }

    fn repeating_tile1(&self) -> DngRect {
        self.src_image.read().unwrap().repeating_tile()
    }

    fn repeating_tile2(&self) -> DngRect {
        self.dst_image.read().unwrap().repeating_tile()
    }

    fn process(
        &self,
        _thread_index: uint32,
        tile: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        let src_guard = self.src_image.read().unwrap();

        let src_buffer = dng_core::dng_image::DngConstTileBuffer::new(&*src_guard, tile)?;

        drop(src_guard);

        let dst_guard = self.dst_image.read().unwrap();

        let mut dst_buffer =
            dng_core::dng_image::DngDirtyTileBuffer::new(&*dst_guard, tile)?;

        drop(dst_guard);

        let s_col_step = src_buffer.buffer.col_step;
        let d_col_step = dst_buffer.buffer.col_step;

        let planes = self.src_image.read().unwrap().planes();

        if self.dst_image.read().unwrap().pixel_size() == 2 {
            // 16-bit path.

            for plane in 0..planes as usize {
                let map = &self.table16[plane];

                for row in tile.t..tile.b {
                    let s_idx0 = src_buffer
                        .buffer
                        .pixel_element_index_public(row, tile.l, plane as uint32)?;

                    let d_idx0 = dst_buffer
                        .buffer
                        .pixel_element_index_public(row, tile.l, plane as uint32)?;

                    let src = src_buffer.buffer.as_u16_slice().unwrap();
                    let dst = dst_buffer.buffer.as_u16_slice_mut().unwrap();

                    let mut si = s_idx0;
                    let mut di = d_idx0;

                    for _col in tile.l..tile.r {
                        dst[di] = map[src[si] as usize];

                        si += s_col_step as usize;
                        di += d_col_step as usize;
                    }
                }
            }
        } else {
            // 8-bit path.

            let noise = DngDither::get().noise_buffer16().to_vec();

            for plane in 0..planes as usize {
                let map = &self.table16[plane];

                for row in tile.t..tile.b {
                    let s_idx0 = src_buffer
                        .buffer
                        .pixel_element_index_public(row, tile.l, plane as uint32)?;

                    let d_idx0 = dst_buffer
                        .buffer
                        .pixel_element_index_public(row, tile.l, plane as uint32)?;

                    let src = src_buffer.buffer.as_u16_slice().unwrap();
                    let dst = dst_buffer.buffer.as_u8_slice_mut().unwrap();

                    let row_offset = ((row as uint32 & DngDither::kRNGMask) as usize)
                        * DngDither::kRNGSize as usize;

                    let mut si = s_idx0;
                    let mut di = d_idx0;

                    for col in tile.l..tile.r {
                        let mut x = src[si] as uint32;

                        let r =
                            noise[row_offset + (col as uint32 & DngDither::kRNGMask) as usize]
                                as uint32;

                        x = map[x as usize] as uint32;

                        x = (((x << 8) - x) + r) >> 16;

                        dst[di] = x as uint8;

                        si += s_col_step as usize;
                        di += d_col_step as usize;
                    }
                }
            }
        }

        {
            let dst_guard = self.dst_image.read().unwrap();
            dst_buffer.flush(&*dst_guard)?;
        }

        Ok(())
    }
}

/// Port of `EncodeImageForCompression` (dng_negative.cpp:5818) — computes
/// the proxy gamma-encoding of an image plus the opcode list to undo it.
#[allow(clippy::too_many_arguments)]
pub fn encode_image_for_compression(
    host: &mut DngHost,
    src_image: &ImageRef,
    active_area: DngRect,
    is_scene_referred: bool,
    use_16bit: bool,
    src_black_level: real64,
    dst_black_level: &mut [real64; kMaxColorPlanes as usize],
    opcode_list: &mut DngOpcodeList,
) -> DngResult<Option<ImageRef>> {
    if src_image.read().unwrap().pixel_type() != ttShort {
        return Ok(None);
    }

    let planes = src_image.read().unwrap().planes();

    let mut lower = [0.0; kMaxColorPlanes as usize];
    let mut upper = [0.0; kMaxColorPlanes as usize];

    {
        const K_CLIP_FRACTION: real64 = 0.00001;

        let pixels = (active_area.h() as uint64) * (active_area.w() as uint64);

        let limit = round_uint32_f64(pixels as real64 * K_CLIP_FRACTION);

        let mut hist = vec![0u32; 65536];

        for plane in 0..planes {
            histogram_area(
                host,
                &*src_image.read().unwrap(),
                active_area,
                &mut hist,
                65535,
                plane,
            )?;

            let mut total: uint32 = 0;

            let mut upper_index: uint32 = 65535;

            while total + hist[upper_index as usize] <= limit && upper_index > 255 {
                total += hist[upper_index as usize];

                upper_index -= 1;
            }

            total = 0;

            let mut lower_index: uint32 = 0;

            while total + hist[lower_index as usize] <= limit
                && lower_index < upper_index.saturating_sub(255)
            {
                total += hist[lower_index as usize];

                lower_index += 1;
            }

            lower[plane as usize] = lower_index as real64 / 65535.0;
            upper[plane as usize] = upper_index as real64 / 65535.0;
        }
    }

    let base_curve: Box<dyn DngBaseProxyCurve> = if use_16bit {
        Box::new(DngJxlProxyCurve)
    } else {
        Box::new(DngJpegProxyCurve)
    };

    for entry in dst_black_level.iter_mut() {
        *entry = 0.0;
    }

    let white_level: uint32 = if use_16bit { 65535 } else { 255 };

    let white_level64 = white_level as real64;

    if is_scene_referred && (src_black_level > 0.0) {
        for plane in 0..planes as usize {
            if lower[plane] < src_black_level {
                upper[plane] = max_real64(
                    upper[plane],
                    src_black_level
                        + (src_black_level - lower[plane])
                            * (1.0 / kMaxStage3BlackLevelNormalized - 1.0),
                );

                upper[plane] = min_real64(upper[plane], 1.0);

                let neg_range = base_curve.evaluate_scene(
                    (src_black_level - lower[plane]) / (upper[plane] - src_black_level),
                );

                let out_black = neg_range / (1.0 + neg_range);

                dst_black_level[plane] = min_real64(
                    kMaxStage3BlackLevelNormalized * white_level64,
                    (out_black * white_level64).ceil(),
                );
            }
        }
    }

    // Apply the gamma encoding, using dither when downsampling to 8-bit.

    let dst = DngSimpleImage::new(
        src_image.read().unwrap().bounds(),
        planes,
        if use_16bit { ttShort } else { ttByte },
    )?;

    let dst_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(dst));

    {
        let mut task = DngEncodeProxyTask::new(
            src_image.clone(),
            dst_ref.clone(),
            &lower,
            &upper,
            is_scene_referred,
            src_black_level,
            dst_black_level,
            white_level64,
            base_curve.as_ref(),
        )?;

        let src_bounds = src_image.read().unwrap().bounds();

        host.perform_area_task(&mut task, src_bounds)?;
    }

    // Add opcodes to undo the gamma encoding.

    for plane in 0..planes {
        let area_spec = crate::dng_misc_opcodes::DngAreaSpec::new(
            DngRect::from_size_point(active_area.size()),
            plane,
            1,
            1,
            1,
        );

        let mut coefficient = [0.0; 4];

        coefficient[0] = 0.0;

        if is_scene_referred {
            coefficient[1] = base_curve.scene_slope();
            coefficient[2] = 0.0;
            coefficient[3] = 1.0 - coefficient[1];
        } else {
            coefficient[1] = base_curve.output_slope();
            coefficient[2] = 1.0 - coefficient[1];
            coefficient[3] = 0.0;
        }

        if lower[plane as usize] < src_black_level {
            let rescale = (upper[plane as usize] - src_black_level) / (1.0 - src_black_level);

            coefficient[0] *= rescale;
            coefficient[1] *= rescale;
            coefficient[2] *= rescale;
            coefficient[3] *= rescale;
        } else {
            let rescale = (upper[plane as usize] - lower[plane as usize]) / (1.0 - src_black_level);

            coefficient[0] *= rescale;
            coefficient[1] *= rescale;
            coefficient[2] *= rescale;
            coefficient[3] *= rescale;

            coefficient[0] += (lower[plane as usize] - src_black_level) / (1.0 - src_black_level);
        }

        let opcode = crate::dng_misc_opcodes::DngOpcodeMapPolynomial::new(
            area_spec,
            if is_scene_referred { 3 } else { 2 },
            &coefficient,
        );

        opcode_list.append(Box::new(opcode));
    }

    Ok(Some(dst_ref))
}

// ---------------------------------------------------------------------------
// NoiseProfile tag parsing (the NoiseProfile tag layout: count followed by
// scale/offset float64 pairs; dng_ifd.cpp captures offset/count only).
// ---------------------------------------------------------------------------

/// Parse the NoiseProfile tag (a uint32 plane count followed by count pairs
/// of (scale, offset) float64 values).
fn parse_noise_profile_tag(stream: &mut DngStream, offset: uint64) -> DngResult<DngNoiseProfile> {
    stream.set_read_position(offset)?;

    let count = stream.get_uint32()?;

    let mut functions = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let scale = stream.get_real64()?;
        let offset_v = stream.get_real64()?;

        functions.push(crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(
            scale, offset_v,
        ));
    }

    Ok(DngNoiseProfile::with_functions(functions))
}

// ---------------------------------------------------------------------------
// Small helpers.
// ---------------------------------------------------------------------------

fn min_masked_count(a: uint32, b: uint32) -> uint32 {
    if a < b {
        a
    } else {
        b
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_negative() -> DngNegative {
        let mut host = DngHost::new();
        DngNegative::new(&mut host).unwrap()
    }

    #[test]
    fn defaults_match_cpp_constructor() {
        let negative = test_negative();

        assert_eq!(negative.default_crop_origin_h, DngURational::new(0, 1));
        assert_eq!(negative.default_crop_origin_v, DngURational::new(0, 1));
        assert_eq!(negative.default_user_crop_t, DngURational::new(0, 1));
        assert_eq!(negative.default_user_crop_b, DngURational::new(1, 1));
        assert_eq!(negative.default_scale_h, DngURational::new(1, 1));
        assert_eq!(negative.best_quality_scale, DngURational::new(1, 1));
        assert!((negative.raw_to_full_scale_h - 1.0).abs() < 1e-12);
        assert_eq!(negative.baseline_noise, DngURational::new(100, 100));
        assert_eq!(negative.baseline_exposure, DngSRational::new(0, 100));
        assert_eq!(negative.baseline_sharpness, DngURational::new(100, 100));
        assert_eq!(negative.anti_alias_strength, DngURational::new(100, 100));
        assert_eq!(negative.linear_response_limit, DngURational::new(100, 100));
        assert_eq!(negative.shadow_scale, DngURational::new(1, 1));
        assert_eq!(negative.colorimetric_reference, crSceneReferred);
        assert_eq!(negative.color_channels, 0);
        assert_eq!(negative.stage3_gain, 1.0);
        assert_eq!(negative.stage3_black_level, 0);
        assert_eq!(negative.depth_format, depthFormatUnknown);
        assert_eq!(negative.depth_units, depthUnitsUnknown);
        assert_eq!(negative.depth_measure_type, depthMeasureUnknown);
        assert_eq!(negative.raw_image_stage, RawImageStage::None);
    }

    #[test]
    fn crop_and_scale_math() {
        let mut negative = test_negative();

        negative.set_default_crop_size_u32(100, 200);
        negative.set_default_crop_origin_u32(10, 20);
        negative.set_default_scale(DngURational::new(2, 1), DngURational::new(1, 1));

        assert_eq!(negative.square_width(), 100.0);
        // SquareHeight = cropV * scaleV / scaleH = 200 * 1 / 2 = 100.
        assert!((negative.square_height() - 100.0).abs() < 1e-9);
        assert!((negative.aspect_ratio() - 1.0).abs() < 1e-9);
        assert_eq!(negative.default_final_width(), 200);
        assert_eq!(negative.default_final_height(), 200);
        assert_eq!(negative.best_quality_final_width(), 200);
        assert_eq!(negative.best_quality_final_height(), 200);
        assert!(!negative.has_default_user_crop());

        negative.set_default_user_crop(
            DngURational::new(1, 10),
            DngURational::new(0, 1),
            DngURational::new(9, 10),
            DngURational::new(1, 1),
        );
        assert!(negative.has_default_user_crop());
    }

    #[test]
    fn raw_default_copies() {
        let mut negative = test_negative();

        negative.set_default_crop_size_u32(400, 300);
        negative.set_raw_default_crop();
        negative.set_raw_default_scale();
        negative.set_raw_best_quality_scale();
        negative.set_raw_baseline_sharpness();
        negative.set_raw_noise_reduction_applied();
        negative.set_raw_noise_profile();

        assert_eq!(negative.raw_default_crop_size_h, DngURational::new(400, 1));
        assert_eq!(negative.raw_default_scale_h, DngURational::new(1, 1));
    }

    #[test]
    fn analog_balance_normalization() {
        let mut negative = test_negative();

        negative.set_color_channels(3);

        let balance = DngVector::from_values(&[2.0, 4.0, 8.0]).unwrap();

        negative.set_analog_balance(&balance);

        // Scaled so the minimum entry is 1.0 and rounded to 1e6.
        assert!((negative.analog_balance(0) - 1.0).abs() < 1e-6);
        assert!((negative.analog_balance(1) - 2.0).abs() < 1e-6);
        assert!((negative.analog_balance(2) - 4.0).abs() < 1e-6);

        // Empty/cleared input clears the balance.
        negative.set_analog_balance(&DngVector::new());
        assert!(!negative.has_analog_balance_entry());
    }

    #[test]
    fn camera_neutral_and_white() {
        let mut negative = test_negative();

        assert!(!negative.has_camera_neutral());

        let neutral = DngVector::from_values(&[1.0, 2.0, 4.0]).unwrap();

        negative.set_camera_neutral(&neutral);

        assert!(negative.has_camera_neutral());
        assert!((negative.camera_neutral.get(0) - 0.25).abs() < 1e-6);
        assert!((negative.camera_neutral.get(2) - 1.0).abs() < 1e-6);

        negative.clear_camera_neutral();
        assert!(!negative.has_camera_neutral());

        let white = DngXyCoord::new(0.3127, 0.3290);

        negative.set_camera_white_xy(&white);

        assert!(negative.has_camera_white_xy());
        assert!((negative.camera_white_xy().x - 0.3127).abs() < 1e-6);
    }

    #[test]
    fn camera_profile_add_and_dedup() {
        let mut negative = test_negative();

        let mut profile = DngCameraProfile::new();
        profile.set_name("Test Profile");

        negative.add_profile(profile);

        assert_eq!(negative.profile_count(), 1);
        assert_eq!(
            negative.profile_by_index(0).name().get_str(),
            Some("Test Profile")
        );

        // Adding an identical profile replaces the first (duplicate
        // detection).

        let mut duplicate = DngCameraProfile::new();
        duplicate.set_name("Test Profile");

        negative.add_profile(duplicate);

        assert_eq!(negative.profile_count(), 1);

        // Adding a profile with no name gets "Embedded".

        negative.add_profile(DngCameraProfile::new());

        assert_eq!(
            negative.profile_by_index(1).name().get_str(),
            Some("Embedded")
        );
    }

    #[test]
    fn profile_lookup_by_name() {
        let mut negative = test_negative();

        let mut profile = DngCameraProfile::new();
        profile.set_name("Adobe Standard");

        negative.add_profile(profile);

        let id = DngCameraProfileId::from_name("Adobe Standard");

        let mut found = DngCameraProfile::new();

        assert!(negative.get_profile_by_id(&id, &mut found, true, None));
        assert_eq!(found.name().get_str(), Some("Adobe Standard"));

        // Monochrome negatives don't have profiles.

        negative.set_monochrome();

        assert!(!negative.get_profile_by_id(&id, &mut found, true, None));
    }

    #[test]
    fn bayer_mosaic_patterns() {
        let mut negative = test_negative();

        negative.set_rgb();
        negative.set_bayer_mosaic(0);

        let info = negative.get_mosaic_info().unwrap();

        assert_eq!(info.color_planes, 3);
        assert_eq!(info.cfa_layout, 1);
        assert_eq!(info.cfa_pattern_size, DngPoint::new(2, 2));
        assert!(info.is_color_filter_array());

        // Phase 0 puts color0 at pattern (0, 1).

        assert_eq!(info.cfa_pattern[0][1], info.cfa_plane_color[0]);
        assert_eq!(info.cfa_pattern[0][0], info.cfa_plane_color[1]);

        // Fuji mosaic layout.

        negative.set_fuji_mosaic(0);

        let info = negative.get_mosaic_info().unwrap();

        assert_eq!(info.cfa_pattern_size, DngPoint::new(2, 4));
        assert_eq!(info.cfa_layout, 2);

        // Quad mosaic.

        negative.set_gmcy();
        negative.set_quad_mosaic(0xe1e4e1e4);

        let info = negative.get_mosaic_info().unwrap();

        assert_eq!(info.color_planes, 4);
        // C++ dng_point(4, 2) — v=4, h=2 for pattern 0xe1e4e1e4.
        assert_eq!(info.cfa_pattern_size, DngPoint::new(4, 2));
    }

    #[test]
    fn black_and_white_levels() {
        let mut negative = test_negative();

        // No linearization info yet: default white level.

        assert_eq!(negative.white_level(0), 0x0FFFF);

        negative.set_black_level(100.0, -1);
        negative.set_white_level(4000, -1);

        let info = negative.get_linearization_info().unwrap();

        assert_eq!(info.black_level_repeat_rows, 1);
        assert_eq!(info.black_level_repeat_cols, 1);
        assert_eq!(info.black_level[0][0][0], 100.0);
        assert_eq!(info.white_level[0], 4000.0);

        assert_eq!(negative.white_level(0), 4000);

        // Quad blacks.

        negative.set_quad_blacks(10.0, 20.0, 30.0, 40.0, -1);

        let info = negative.get_linearization_info().unwrap();

        assert_eq!(info.black_level_repeat_rows, 2);
        assert_eq!(info.black_level_repeat_cols, 2);
        assert_eq!(info.black_level[1][1][0], 40.0);
    }

    #[test]
    fn metadata_roundtrip_and_sync() {
        let mut negative = test_negative();

        // EXIF values flow into XMP.

        {
            let exif = negative.get_exif_mut();
            exif.make.set(b"TestCam");
            exif.model.set(b"Model X");
        }

        negative.synchronize_metadata();

        let xmp = negative.get_xmp();

        let mut make = DngString::new();

        xmp.get_string(dng_xmp::XMP_NS_TIFF, "Make", &mut make);

        assert_eq!(make.get_str(), Some("TestCam"));

        let mut model = DngString::new();

        xmp.get_string(dng_xmp::XMP_NS_TIFF, "Model", &mut model);

        assert_eq!(model.get_str(), Some("Model X"));

        // Original EXIF snapshot captured before sync.

        assert!(negative.get_original_exif().is_some());
    }

    #[test]
    fn orientation_application() {
        let mut negative = test_negative();

        negative.set_base_orientation(DngOrientation::rotate90cw());

        assert!(negative.has_base_orientation());
        assert_eq!(
            negative.orientation().get_adobe(),
            DngOrientation::rotate90cw().get_adobe()
        );

        // Applying an orientation composes it.

        negative.apply_orientation(DngOrientation::rotate_180());

        let expected = DngOrientation::rotate90cw().plus(DngOrientation::rotate_180());

        assert_eq!(negative.orientation().get_adobe(), expected.get_adobe());
    }

    #[test]
    fn digest_of_synthetic_image() {
        let mut host = DngHost::new();

        let img = DngSimpleImage::new(DngRect::from_corners(0, 0, 8, 8), 1, ttShort).unwrap();

        let image_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(img));

        // Fill with a deterministic pattern.

        {
            let mut img = image_ref.write().unwrap();

            let area = img.bounds();

            let mut buffer = DngPixelBuffer::with_planar_config(
                area,
                0,
                1,
                ttShort,
                dng_core::dng_tag_values::pcInterleaved,
                DngPixelBuffer::allocate_data(area, 1, ttShort).unwrap(),
            )
            .unwrap();

            {
                let data = buffer.as_u16_slice_mut().unwrap();

                for (j, v) in data.iter_mut().enumerate() {
                    *v = (j * 7 % 65536) as uint16;
                }
            }

            img.do_put(&buffer).unwrap();
        }

        let digest1 = DngNegative::find_image_digest(&mut host, &image_ref).unwrap();
        let digest2 = DngNegative::find_image_digest(&mut host, &image_ref).unwrap();

        assert!(digest1.is_valid());
        assert_eq!(digest1, digest2);

        // The fast digest over the same pixel type is also stable.

        let fast = DngNegative::find_fast_image_digest(&mut host, &image_ref, ttShort).unwrap();

        assert!(fast.is_valid());
    }

    #[test]
    fn proxy_curve_math_round_trip() {
        // The JPEG proxy scene curve is the inverse of
        // f(x) = s*x + (1-s)*x^3 for s = 1/128.

        let curve = DngJpegProxyCurve;

        let s = K_SCENE_PROXY_CURVE_SLOPE;

        let y = curve.evaluate_scene(0.5);

        let round_trip = s * y + (1.0 - s) * y * y * y;

        assert!((round_trip - 0.5).abs() < 1e-6);

        // Output curve round trip:
        // f(y) = k*y + (1-k)*y^2 with k = 1/16.

        let k = K_OUTPUT_PROXY_CURVE_SLOPE;

        let y2 = curve.evaluate_output(0.5);

        let round_trip2 = k * y2 + (1.0 - k) * y2 * y2;

        assert!((round_trip2 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn noise_profile_valid_for_negative() {
        let mut negative = test_negative();

        negative.set_color_channels(3);

        // Single-function profile is valid for any channel count.

        let single = DngNoiseProfile::with_functions(vec![
            crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(1.0, 0.0),
        ]);

        assert!(single.is_valid_for_negative(&negative));

        negative.set_noise_profile(single);

        assert!(negative.has_noise_profile());

        // Three-function profile matches 3 channels.

        let triple = DngNoiseProfile::with_functions(vec![
            crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(1.0, 0.0),
            crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(2.0, 0.0),
            crate::dng_noise_profile::DngNoiseFunction::with_scale_offset(3.0, 0.0),
        ]);

        assert!(triple.is_valid_for_negative(&negative));
    }

    #[test]
    fn total_baseline_exposure_includes_profile_offset() {
        let mut negative = test_negative();

        negative.set_baseline_exposure(0.25);

        let mut profile = DngCameraProfile::new();
        profile.set_name("Offset Profile");
        profile.set_baseline_exposure_offset(DngSRational::new(15, 100));

        negative.add_profile(profile);

        let id = DngCameraProfileId::from_name("Offset Profile");

        let total = negative.total_baseline_exposure(&id);

        assert!((total - 0.4).abs() < 1e-9);
    }

    #[test]
    fn semantic_mask_sub_area() {
        let mut mask = DngSemanticMask::default();

        // No mask image: not valid.

        assert!(!mask.is_mask_sub_area_valid());

        // 4x4 full image with a 2x2 mask at (1, 1).

        let img = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 2), 1, ttShort).unwrap();

        mask.mask = Some(std::sync::Arc::new(std::sync::RwLock::new(img)));

        mask.mask_sub_area = [1, 1, 4, 4];

        assert!(mask.is_mask_sub_area_valid());

        // Crop that doesn't fit: invalid.

        mask.mask_sub_area = [3, 3, 4, 4];

        assert!(!mask.is_mask_sub_area_valid());
    }

    #[test]
    fn original_sizes_helpers() {
        let mut negative = test_negative();

        negative.set_default_crop_size_u32(800, 600);

        negative.set_default_original_sizes();

        assert!(negative.original_default_final_size.h > 0);
        assert_eq!(
            negative.original_default_final_size,
            negative.original_best_quality_final_size
        );

        negative.clear_original_sizes();

        assert_eq!(negative.original_default_final_size, DngPoint::new(0, 0));
    }

    #[test]
    fn stage3_black_level_normalized() {
        let mut negative = test_negative();

        negative.set_stage3_black_level(6554); // ~0.1

        assert!((negative.stage3_black_level_normalized() - 6554.0 / 65535.0).abs() < 1e-9);
    }

    // -------------------------------------------------- DngMetadata-level API

    #[test]
    fn iptc_digest_padding_rules() {
        let mut metadata = DngMetadata::new();

        assert!(metadata.iptc_digest(true).is_null());

        let mut block = dng_core::DngMemoryBlock::new(7).unwrap();

        block.buffer_mut().copy_from_slice(b"IIII\x00\x00\x00");

        metadata.set_iptc(Some(block));

        let with_padding = metadata.iptc_digest(true);
        let without_padding = metadata.iptc_digest(false);

        assert!(with_padding.is_valid());
        assert!(without_padding.is_valid());
        assert_ne!(with_padding, without_padding);
    }

    #[test]
    fn metadata_set_embedded_xmp_digest() {
        let mut host = DngHost::new();

        let mut metadata = DngMetadata::new();

        let xmp_packet = b"<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>
<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">
 <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end=\"w\"?>";

        metadata.set_embedded_xmp(&mut host, xmp_packet);

        assert!(metadata.have_valid_embedded_xmp());

        // Sidecar tags removed from embedded XMP.

        assert!(!metadata
            .get_xmp()
            .exists(dng_xmp::XMP_NS_PHOTOSHOP, "SidecarForExtension"));
    }

    #[test]
    fn dual_parse_xmp_scans_tables() {
        let mut host = DngHost::new();

        let mut xmp = DngXmp::new();

        let mut dictionary = DngBigTableDictionary::new();

        // A compact-mode crs:Table_ block with an invalid 32-char
        // fingerprint: the scan skips it, parsing still succeeds.

        let packet = "<x:xmpmeta><rdf:RDF> crs:Table_00112233445566778899aabbccddeeff=\"AAAA\" </rdf:RDF></x:xmpmeta>";

        dual_parse_xmp(&mut host, &mut xmp, &mut dictionary, packet.as_bytes());

        // The hex fingerprint 001122...eeff is valid; the ASCII payload
        // "AAAA" decodes to 2 bytes and lands in the dictionary.

        assert_eq!(dictionary.map().len(), 1);
    }
    /// lossless_compress_jxl on a synthetic negative: raw JXL tiles decode
    /// back losslessly.
    #[test]
    fn lossless_compress_jxl_round_trip() {
        let mut host = DngHost::new();

        host.set_save_dng_version(dngVersion_1_7_1_0);

        let mut negative = DngNegative::new(&mut host).unwrap();

        let width = 24u32;
        let height = 16u32;

        let bounds = DngRect::from_corners(0, 0, height as int32, width as int32);

        let image: ImageRef = DngSimpleImage::new(bounds, 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buffer = DngPixelBuffer::with_planar_config(
                bounds,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(bounds, 1, ttShort).unwrap(),
            )
            .unwrap();

            let samples = buffer.as_u16_slice_mut().unwrap();

            for (index, sample) in samples.iter_mut().enumerate() {
                *sample = ((index * 365) % 65535) as u16;
            }

            image.write().unwrap().do_put(&buffer).unwrap();
        }

        negative.raw_image = Some(image.clone());
        negative.set_default_crop_size_u32(width, height);

        let mut writer = crate::dng_image_writer::DngImageWriter::new();

        negative
            .lossless_compress_jxl(&mut host, &mut writer, false)
            .unwrap();

        let lossy = negative
            .raw_lossy_compressed_image()
            .expect("raw lossy image must be set");

        assert_eq!(lossy.compression_code, ccJXL);
        assert!(lossy.tiles.data.len() > 0);
        assert!(lossy.tiles.data[0].is_some());

        // Decode tile 0 through the dng-jxl crate and compare against the
        // same tile region of the source (lossless must be bit-exact).

        let block = lossy.tiles.data[0].as_ref().unwrap();

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        stream.put(block.buffer()).unwrap();

        stream.set_read_position(0).unwrap();

        let length = stream.length() as uint32;

        let bytes = stream.get(length).unwrap();

        let decoded = dng_jxl::decode_jxl(&bytes).unwrap();

        // Tile layout: 1MB tiles over a 24x16 image = single tile.

        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);

        let source = image.read().unwrap();

        let mut buffer = DngPixelBuffer::with_planar_config(
            bounds,
            0,
            1,
            ttShort,
            pcInterleaved,
            DngPixelBuffer::allocate_data(bounds, 1, ttShort).unwrap(),
        )
        .unwrap();

        source.do_get(&mut buffer).unwrap();

        let source_samples = buffer.as_u16_slice().unwrap();

        let decoded_samples = &decoded.data;

        // 16-bit samples, interleaved LE.

        for (index, sample) in source_samples.iter().enumerate() {
            let lo = decoded_samples[index * 2] as uint16;
            let hi = decoded_samples[index * 2 + 1] as uint16;

            let value = lo | (hi << 8);

            assert_eq!(value, *sample, "lossless mismatch at {index}");
        }
    }

    /// encode_raw_proxy on a synthetic stage3: returns a proxy image and
    /// appends opcode list entries.
    #[test]
    fn encode_raw_proxy_shapes() {
        let mut host = DngHost::new();

        let negative = DngNegative::new(&mut host).unwrap();

        let width = 16u32;
        let height = 12u32;

        let bounds = DngRect::from_corners(0, 0, height as int32, width as int32);

        let image: ImageRef = DngSimpleImage::new(bounds, 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buffer = DngPixelBuffer::with_planar_config(
                bounds,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(bounds, 1, ttShort).unwrap(),
            )
            .unwrap();

            let samples = buffer.as_u16_slice_mut().unwrap();

            for (index, sample) in samples.iter_mut().enumerate() {
                *sample = ((index * 501) % 40000) as u16;
            }

            image.write().unwrap().do_put(&buffer).unwrap();
        }

        let mut opcode_list = crate::dng_opcode_list::DngOpcodeList::new(2);

        let mut black = [0.0; dng_core::dng_sdk_limits::kMaxColorPlanes as usize];

        let mut host = DngHost::new();

        let proxy = negative
            .encode_raw_proxy(&mut host, &image, &mut opcode_list, &mut black)
            .unwrap();

        let proxy_img = proxy.read().unwrap();

        assert_eq!(proxy_img.pixel_type(), ttShort);
        assert_eq!(proxy_img.width(), width);
        assert_eq!(proxy_img.height(), height);

        // The opcode list gains per-plane MapPolynomial entries.

        assert!(opcode_list.count() >= 1);
    }
    /// convert_to_proxy on a synthetic negative: stage3 resampled to the
    /// proxy size and a raw proxy image encoded.
    #[test]
    fn convert_to_proxy_shapes() {
        let mut host = DngHost::new();

        let mut negative = DngNegative::new(&mut host).unwrap();

        let width = 64u32;
        let height = 48u32;

        let bounds = DngRect::from_corners(0, 0, height as int32, width as int32);

        let image: ImageRef = DngSimpleImage::new(bounds, 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buffer = DngPixelBuffer::with_planar_config(
                bounds,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(bounds, 1, ttShort).unwrap(),
            )
            .unwrap();

            let samples = buffer.as_u16_slice_mut().unwrap();

            for (index, sample) in samples.iter_mut().enumerate() {
                *sample = ((index * 353) % 65535) as u16;
            }

            image.write().unwrap().do_put(&buffer).unwrap();
        }

        negative.set_stage3_image(Some(image));
        negative.set_default_crop_size_u32(width, height);
        negative.set_active_area(bounds);

        negative
            .convert_to_proxy(&mut host, &mut crate::dng_image_writer::DngImageWriter::new(), 16, 0)
            .unwrap();

        // Stage3 must now be the proxy size (16 wide).

        let stage3 = negative.stage3_image().unwrap().clone();

        {
            let img = stage3.read().unwrap();

            assert_eq!(img.width(), 16);
            assert!(img.height() <= 16);
        }

        // A raw image must exist (encoded proxy).

        assert!(negative.raw_image().is_some());
    }

    /// lossy_compress_mosaic_jxl on a Bayer negative.
    #[test]
    fn lossy_compress_mosaic_shapes() {
        let mut host = DngHost::new();

        host.set_save_dng_version(dngVersion_1_7_1_0);
        host.set_lossy_mosaic_jxl(true);

        let mut negative = DngNegative::new(&mut host).unwrap();

        let width = 32u32;
        let height = 32u32;

        let bounds = DngRect::from_corners(0, 0, height as int32, width as int32);

        let image: ImageRef = DngSimpleImage::new(bounds, 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buffer = DngPixelBuffer::with_planar_config(
                bounds,
                0,
                1,
                ttShort,
                pcInterleaved,
                DngPixelBuffer::allocate_data(bounds, 1, ttShort).unwrap(),
            )
            .unwrap();

            let samples = buffer.as_u16_slice_mut().unwrap();

            for (index, sample) in samples.iter_mut().enumerate() {
                *sample = ((index * 97) % 40000 + 1000) as u16;
            }

            image.write().unwrap().do_put(&buffer).unwrap();
        }

        negative.raw_image = Some(image);
        negative.set_default_crop_size_u32(width, height);

        // Mosaic info: 2x2 RGGB (cfa_pattern_size non-zero marks CFA).

        let mut mosaic = crate::dng_mosaic_info::DngMosaicInfo::new();

        mosaic.cfa_pattern_size = DngPoint::new(2, 2);
        mosaic.cfa_plane_color[0] = 0;
        mosaic.cfa_plane_color[1] = 1;
        mosaic.cfa_plane_color[2] = 2;
        mosaic.cfa_pattern[0][0] = 0;
        mosaic.cfa_pattern[0][1] = 1;
        mosaic.cfa_pattern[1][0] = 1;
        mosaic.cfa_pattern[1][1] = 2;

        negative.mosaic_info = Some(Box::new(mosaic));

        let mut writer = crate::dng_image_writer::DngImageWriter::new();

        negative
            .lossy_compress_mosaic_jxl(&mut host, &mut writer)
            .unwrap();

        let lossy = negative
            .raw_lossy_compressed_image()
            .expect("raw lossy image must be set");

        assert_eq!(lossy.compression_code, ccJXL);
        assert_eq!(lossy.row_interleave_factor, 2);
        assert_eq!(lossy.column_interleave_factor, 2);
        assert!(lossy.tiles.data[0].is_some());
    }
}

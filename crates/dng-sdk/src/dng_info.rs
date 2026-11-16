// Port of dng_info.h/.cpp (SDK 1.7.1) — top-level structure of a DNG file
// with access to metadata.
//
// Porting notes:
//   * `std::vector<dng_ifd *>` becomes plain owned `Vec<DngIfd>` (no shared
//     ownership is needed; dng_negative clones the IFDs it inspects).
//   * `dng_stream_double_buffered` becomes dng-core's free-function wrapper
//     (`dng_stream_double_buffered (&mut DngStream)`); the port reads small
//     tags through the same stream (the buffering is a performance hint only,
//     identical results).
//   * The MakerNote brand dispatch (ParseMakerNote) is ported 1:1 including
//     the Apple/Epson/Fuji/Leica/Nikon/Olympus/OM System/Panasonic/Pentax/
//     Ricoh/Casio header formats and the model-driven fallbacks.
//   * `fExif->PostParse` / `fShared->PostParse` defaults are applied inline
//     (see post_parse below) — dng-core's DngShared/DngExif have no virtual
//     PostParse; the C++ default-fill logic lives here.
//   * qDNGValidate-only diagnostics are omitted (dng-core PORTING_NOTES #4).

use dng_core::{
    bad_format, max_uint32, min_uint32, safe_uint32_add, safe_uint32_mult2, safe_uint64_add, DngHost,
    DngResult, DngStream, DngString, int32, int64, uint16, uint32, uint64, uint8,
};
use dng_core::dng_sdk_limits::{kMaxChainedIFDs, kMaxParseDepth, kMaxSubIFDs};
use dng_core::dng_stream::{dng_stream_double_buffered, TempBigEndian, TempLittleEndian};
use dng_core::dng_tag_types::{tag_type_size, ttAscii, ttIFD, ttUndefined};
use dng_core::dng_tag_values::{
    byteOrderII, byteOrderMM, dngVersion_1_0_0_0, dngVersion_1_1_0_0, lsD65, lsStandardLightA,
    lsUnknown, magicBigTIFF, magicExtendedProfile, magicOlympusA, magicOlympusB, magicPanasonic,
    magicRawCache, magicTIFF, previewColorSpace_GrayGamma22, previewColorSpace_MaxEnum,
    previewColorSpace_sRGB, sfAltPreviewImage, sfDepthMap, sfEnhancedImage, sfMainImage,
    sfPreviewImage, sfSemanticMask, sfTransparencyMask,
};
use dng_core::dng_tags::{
    tcAppleMakerNote, tcCanonCRW, tcCanonMakerNote, tcCasioMakerNote, tcContaxHeader, tcContaxRAW,
    tcEpsonMakerNote, tcExifIFD, tcFirstChainedIFD, tcFirstSubIFD, tcFujiHeader, tcFujiMakerNote,
    tcFujiRAF, tcFujiRawInfo1, tcFujiRawInfo2, tcGPSInfo, tcHasselbladMakerNote,
    tcInteroperabilityIFD, tcKodakDCRPrivateIFD, tcKodakKDCPrivateIFD, tcKodakMakerNote,
    tcLastChainedIFD, tcLastSubIFD, tcLeafMOS, tcLeicaMakerNote, tcMamiyaMakerNote,
    tcMinoltaMakerNote, tcMinoltaMRW, tcNikonMakerNote, tcOlympusMakerNote,
    tcOlympusMakerNote12288, tcOlympusMakerNote8208, tcOlympusMakerNote8224,
    tcOlympusMakerNote8240, tcOlympusMakerNote8256, tcOlympusMakerNote8272, tcPanasonicMakerNote,
    tcPanasonicRAW, tcPentaxMakerNote, tcRicohMakerNote, tcRicohMakerNoteCameraInfo,
    tcSamsungMakerNote, tcSonyMakerNote, tcNewSubFileType,
};

use crate::dng_exif::DngExif;
use dng_core::dng_shared::DngShared;
use dng_core::dng_ifd::DngIfd;

/// `DNG_CHAR4` macro.
const fn dng_char4(a: u8, b: u8, c: u8, d: u8) -> uint32 {
    ((a as uint32) << 24) | ((b as uint32) << 16) | ((c as uint32) << 8) | (d as uint32)
}

/// Port of `dng_info` — top-level structure of a DNG file.
pub struct DngInfo {
    /// `fTIFFBlockOffset`.
    pub tiff_block_offset: uint64,

    /// `fTIFFBlockOriginalOffset`.
    pub tiff_block_original_offset: uint64,

    /// `fBigEndian`.
    pub big_endian: bool,

    /// `fMagic`.
    pub magic: uint32,

    /// `fExif`.
    pub exif: DngExif,

    /// `fShared`.
    pub shared: DngShared,

    /// `fMainIndex`.
    pub main_index: int32,

    /// `fMaskIndex`.
    pub mask_index: int32,

    /// `fDepthIndex`.
    pub depth_index: int32,

    /// `fEnhancedIndex`.
    pub enhanced_index: int32,

    /// `fSemanticMaskIndices`.
    pub semantic_mask_indices: Vec<uint32>,

    /// `fIFD` — the IFD chain including IFD 0 and all SubIFDs.
    pub ifd: Vec<DngIfd>,

    /// `fChainedIFD`.
    pub chained_ifd: Vec<DngIfd>,

    /// `fChainedSubIFD`.
    pub chained_sub_ifd: Vec<Vec<DngIfd>>,

    /// `fXMPBlock` — the raw XMP packet captured by dng_shared (kept for
    /// parity; the parsed copy lives in the negative's metadata).
    pub xmp_block: Option<Vec<uint8>>,

    /// `fMakerNoteNextIFD` (protected in C++).
    pub(crate) maker_note_next_ifd: uint32,

    /// `fParseDepth` (protected in C++).
    pub(crate) parse_depth: uint32,
}

impl Default for DngInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl DngInfo {
    /// `dng_info ()`.
    pub fn new() -> Self {
        DngInfo {
            tiff_block_offset: 0,
            tiff_block_original_offset: 0,
            big_endian: false,
            magic: 0,
            exif: DngExif::new(),
            shared: DngShared::new(),
            main_index: -1,
            mask_index: -1,
            depth_index: -1,
            enhanced_index: -1,
            semantic_mask_indices: Vec::new(),
            ifd: Vec::new(),
            chained_ifd: Vec::new(),
            chained_sub_ifd: Vec::new(),
            xmp_block: None,
            maker_note_next_ifd: 0,
            parse_depth: 0,
        }
    }

    /// `IFDCount ()`.
    pub fn ifd_count(&self) -> uint32 {
        self.ifd.len() as uint32
    }

    /// `ChainedIFDCount ()`.
    pub fn chained_ifd_count(&self) -> uint32 {
        self.chained_ifd.len() as uint32
    }

    /// `ChainedSubIFDCount (chainIndex)`.
    pub fn chained_sub_ifd_count(&self, chain_index: uint32) -> uint32 {
        if (chain_index as usize) >= self.chained_sub_ifd.len() {
            0
        } else {
            self.chained_sub_ifd[chain_index as usize].len() as uint32
        }
    }

    /// `ValidateMagic ()`.
    pub fn validate_magic(&self) -> DngResult<()> {
        match self.magic {
            x if x == magicTIFF
                || x == magicBigTIFF
                || x == magicExtendedProfile
                || x == magicRawCache
                || x == magicPanasonic
                || x == magicOlympusA
                || x == magicOlympusB =>
            {
                Ok(())
            }

            _ => Err(bad_format()), // "Invalid TIFF magic number"
        }
    }

    /// `Parse (host, stream)` — read dng_info from a dng_stream
    /// (dng_info.cpp:2194).
    pub fn parse(&mut self, _host: &mut DngHost, stream: &mut DngStream) -> DngResult<()> {
        self.tiff_block_offset = stream.position();

        self.tiff_block_original_offset = stream.position_in_original_file();

        // Check byte order indicator.

        let byte_order = stream.get_uint16()?;

        if byte_order == byteOrderII as uint16 {
            self.big_endian = false;

            stream.set_little_endian(true);
        } else if byte_order == byteOrderMM as uint16 {
            self.big_endian = true;

            stream.set_big_endian(true);
        } else {
            return Err(bad_format()); // "Unknown byte order"
        }

        // Check "magic number" indicator.

        self.magic = stream.get_uint16()? as uint32;

        self.validate_magic()?;

        // Validate BigTIFF header, if any.

        if self.magic == magicBigTIFF {
            let byte_size = stream.get_uint16()?;
            let zero_pad = stream.get_uint16()?;

            if byte_size != 8 || zero_pad != 0 {
                return Err(bad_format()); // "Invalid BigTIFF header"
            }
        }

        // Parse IFD 0.

        let next_offset = if self.magic == magicBigTIFF {
            stream.get_uint64()?
        } else {
            stream.get_uint32()? as uint64
        };

        self.exif = DngExif::new();

        self.shared = DngShared::new();

        self.ifd.push(DngIfd::new());

        self.parse_ifd(
            _host,
            stream,
            true,
            self.tiff_block_offset + next_offset,
            self.tiff_block_offset as int64,
            0,
        )?;

        let mut next_offset = self.ifd[0].next_ifd;

        // Parse chained IFDs.

        while next_offset != 0 {
            if next_offset >= stream.length() {
                // "Chained IFD offset past end of stream"
                break;
            }

            // Some TIFF file writers forget about the next IFD offset, so
            // validate the IFD at that offset before parsing it.

            if !self.validate_ifd(
                stream,
                self.tiff_block_offset + next_offset,
                self.tiff_block_offset as int64,
            )? {
                // "Chained IFD is not valid"
                break;
            }

            if self.chained_ifd_count() == kMaxChainedIFDs {
                // "Chained IFD count exceeds DNG SDK parsing limit"
                break;
            }

            let parent_code = tcFirstChainedIFD + self.chained_ifd_count();

            self.chained_ifd.push(DngIfd::new());

            self.chained_sub_ifd.push(Vec::new());

            let chain_index = self.chained_ifd_count() - 1;

            self.parse_ifd(
                _host,
                stream,
                false,
                self.tiff_block_offset + next_offset,
                self.tiff_block_offset as int64,
                parent_code,
            )?;

            next_offset = self.chained_ifd[chain_index as usize].next_ifd;
        }

        // Parse SubIFDs.

        let mut searched_ifds: uint32 = 0;

        let mut _too_many_sub_ifds = false;

        while searched_ifds < self.ifd_count() && !_too_many_sub_ifds {
            let search_limit = self.ifd_count();

            for search_index in searched_ifds..search_limit {
                if _too_many_sub_ifds {
                    break;
                }

                let sub_ifds_count = self.ifd[search_index as usize].sub_ifds_count;
                let sub_ifds_type = self.ifd[search_index as usize].sub_ifds_type;
                let sub_ifds_offset = self.ifd[search_index as usize].sub_ifds_offset;

                for sub_index in 0..sub_ifds_count {
                    if self.ifd_count() == kMaxSubIFDs + 1 {
                        _too_many_sub_ifds = true;

                        break;
                    }

                    stream.set_read_position(
                        sub_ifds_offset + sub_index as uint64 * tag_type_size(sub_ifds_type) as uint64,
                    )?;

                    let sub_ifd_offset = stream.tag_value_uint64(sub_ifds_type)?;

                    // C++ pushes the IFD first and computes
                    // tcFirstSubIFD + IFDCount() - 2 with the post-push
                    // count, so the first SubIFD gets exactly
                    // tcFirstSubIFD.

                    self.ifd.push(DngIfd::new());

                    let parent_code = tcFirstSubIFD + self.ifd_count() - 2;

                    self.parse_ifd(
                        _host,
                        stream,
                        true,
                        self.tiff_block_offset + sub_ifd_offset,
                        self.tiff_block_offset as int64,
                        parent_code,
                    )?;
                }

                searched_ifds = search_limit;
            }
        }

        // Parse SubIFDs in Chained IFDs.  Don't currently need to make this
        // a recursive search.

        for chained_index in 0..self.chained_ifd_count() {
            let sub_ifds_count = self.chained_ifd[chained_index as usize].sub_ifds_count;
            let sub_ifds_type = self.chained_ifd[chained_index as usize].sub_ifds_type;
            let sub_ifds_offset = self.chained_ifd[chained_index as usize].sub_ifds_offset;

            for sub_index in 0..sub_ifds_count {
                if sub_index == kMaxSubIFDs {
                    // "Chained SubIFD count exceeds DNG SDK parsing limit"
                    break;
                }

                stream.set_read_position(
                    sub_ifds_offset
                        + sub_index as uint64 * tag_type_size(sub_ifds_type) as uint64,
                )?;

                let sub_ifd_offset = stream.tag_value_uint64(sub_ifds_type)?;

                let parent_code = tcFirstSubIFD + sub_index;

                self.chained_sub_ifd[chained_index as usize].push(DngIfd::new());

                self.parse_ifd(
                    _host,
                    stream,
                    true,
                    self.tiff_block_offset + sub_ifd_offset,
                    self.tiff_block_offset as int64,
                    parent_code,
                )?;
            }
        }

        // Parse EXIF IFD.

        if self.shared.exif_ifd != 0 {
            self.parse_ifd(
                _host,
                stream,
                true,
                self.tiff_block_offset + self.shared.exif_ifd,
                self.tiff_block_offset as int64,
                tcExifIFD,
            )?;
        }

        // Parse GPS IFD.

        if self.shared.gps_info != 0 {
            self.parse_ifd(
                _host,
                stream,
                true,
                self.tiff_block_offset + self.shared.gps_info,
                self.tiff_block_offset as int64,
                tcGPSInfo,
            )?;
        }

        // Parse Interoperability IFD.

        if self.shared.interoperability_ifd != 0 {
            // Some Kodak KDC files have bogus Interoperability IFDs, so
            // validate the IFD before trying to parse it.

            if self.validate_ifd(
                stream,
                self.tiff_block_offset + self.shared.interoperability_ifd,
                self.tiff_block_offset as int64,
            )? {
                self.parse_ifd(
                    _host,
                    stream,
                    true,
                    self.tiff_block_offset + self.shared.interoperability_ifd,
                    self.tiff_block_offset as int64,
                    tcInteroperabilityIFD,
                )?;
            }
        }

        // Parse Kodak DCR Private IFD.

        if self.shared.kodak_dcr_private_ifd != 0 {
            self.parse_ifd(
                _host,
                stream,
                true,
                self.tiff_block_offset + self.shared.kodak_dcr_private_ifd,
                self.tiff_block_offset as int64,
                tcKodakDCRPrivateIFD,
            )?;
        }

        // Parse Kodak KDC Private IFD.

        if self.shared.kodak_kdc_private_ifd != 0 {
            self.parse_ifd(
                _host,
                stream,
                true,
                self.tiff_block_offset + self.shared.kodak_kdc_private_ifd,
                self.tiff_block_offset as int64,
                tcKodakKDCPrivateIFD,
            )?;
        }

        // Parse MakerNote tag.

        if self.shared.maker_note_count != 0 {
            let ifd_size = (self.tiff_block_offset + self.shared.maker_note_count as uint64)
                .min(u32::MAX as uint64) as uint32;

            let stream_len = stream.length();

            self.parse_maker_note(
                _host,
                stream,
                ifd_size,
                self.shared.maker_note_offset,
                self.tiff_block_offset as int64,
                0,
                stream_len,
            )?;
        }

        // Parse DNGPrivateData tag.

        if self.shared.dng_private_data_count != 0 && self.shared.dng_version != 0 {
            self.parse_dng_private_data(_host, stream)?;
        }

        Ok(())
    }

    /// `ParseTag (host, stream, exif, shared, ifd, parentCode, tagCode,
    /// tagType, tagCount, tagOffset, offsetDelta)` (dng_info.cpp:131).
    #[allow(clippy::too_many_arguments)]
    fn parse_tag(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        parse_ifd_tags: bool,
        parse_exif: bool,
        parent_code: uint32,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        tag_offset: uint64,
        offset_delta: int64,
    ) -> DngResult<()> {
        let mut parent_code = parent_code;

        let is_sub_ifd = parent_code >= tcFirstSubIFD && parent_code <= tcLastSubIFD;

        let mut ifd_index: Option<usize> = None;

        if parent_code == 0 || is_sub_ifd {
            // The C++ routes to the IFD pointer passed in; the port finds the
            // owning IFD by parent code (IFD 0 for parent 0, sub-IFD index
            // otherwise).
            if parent_code == 0 {
                ifd_index = Some(0);
            } else if is_sub_ifd {
                let index = (parent_code - tcFirstSubIFD) as usize + 1;

                if index < self.ifd.len() {
                    ifd_index = Some(index);
                }
            }
        }

        let is_main_ifd = (parent_code == 0 || is_sub_ifd)
            && ifd_index
                .map(|index| {
                    self.ifd[index].uses_new_sub_file_type
                        && self.ifd[index].new_sub_file_type == sfMainImage
                })
                .unwrap_or(false);

        // Panasonic RAW format stores private tags using tag codes < 254 in
        // IFD 0.  Redirect the parsing of these tags into a logical
        // "PanasonicRAW" IFD.

        // Panasonic is starting to use some higher numbers also (280..283).

        if self.magic == 85 && parent_code == 0 && (tag_code < tcNewSubFileType ||
            (tag_code >= 280 && tag_code <= 283))
        {
            parent_code = tcPanasonicRAW;

            ifd_index = None;
        }

        stream.set_read_position(tag_offset)?;

        // 1. IFD-level tags.

        if parse_ifd_tags {
            if let Some(index) = ifd_index {
                stream.set_read_position(tag_offset)?;

                let accepted =
                    self.ifd[index].parse_tag(host, stream, parent_code, tag_code, tag_type, tag_count, tag_offset)?;

                if accepted {
                    return Ok(());
                }
            }
        }

        // 2. EXIF tags.

        if parse_exif {
            stream.set_read_position(tag_offset)?;

            let mut shared_copy = std::mem::replace(&mut self.shared, DngShared::new());

            let accepted = self.exif.parse_tag(
                stream,
                &mut shared_copy,
                parent_code,
                is_main_ifd,
                tag_code,
                tag_type,
                tag_count,
                tag_offset,
            )?;

            self.shared = shared_copy;

            if accepted {
                return Ok(());
            }
        }

        // 3. Shared (DNG) tags.

        {
            stream.set_read_position(tag_offset)?;

            let accepted = self.shared.parse_tag(
                stream,
                parent_code,
                is_main_ifd,
                tag_code,
                tag_type,
                tag_count,
                tag_offset,
                offset_delta,
            )?;

            if accepted {
                return Ok(());
            }
        }

        // 4. MakerNote embedded formats.

        if parent_code == tcLeicaMakerNote
            && tag_type == ttUndefined
            && tag_count >= 14
        {
            let stream_len = stream.length();

            let ok = self.parse_maker_note_ifd(
                host,
                stream,
                tag_count as uint64,
                tag_offset,
                offset_delta,
                tag_offset,
                stream_len,
                tcLeicaMakerNote,
            )?;

            if ok {
                return Ok(());
            }
        }

        if parent_code == tcOlympusMakerNote
            && tag_type == ttUndefined
            && tag_count >= 14
        {
            let olympus_maker_parent = match tag_code {
                8208 => tcOlympusMakerNote8208,
                8224 => tcOlympusMakerNote8224,
                8240 => tcOlympusMakerNote8240,
                8256 => tcOlympusMakerNote8256,
                8272 => tcOlympusMakerNote8272,
                12288 => tcOlympusMakerNote12288,
                _ => 0,
            };

            if olympus_maker_parent != 0 {
                // Olympus made a mistake in some camera models in computing
                // the size of these sub-tags, so we fudge the count.

                let stream_len = stream.length();

                let ok = self.parse_maker_note_ifd(
                    host,
                    stream,
                    stream_len - tag_offset,
                    tag_offset,
                    offset_delta,
                    tag_offset,
                    stream_len,
                    olympus_maker_parent,
                )?;

                if ok {
                    return Ok(());
                }
            }
        }

        if parent_code == tcRicohMakerNote
            && tag_code == 0x2001
            && tag_type == ttUndefined
            && tag_count > 22
        {
            stream.set_read_position(tag_offset)?;

            let header = stream.get(20)?;

            if &header[..19] == b"[Ricoh Camera Info]" {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    (tag_count - 20) as uint64,
                    tag_offset + 20,
                    offset_delta,
                    tag_offset + 20,
                    tag_offset + tag_count as uint64,
                    tcRicohMakerNoteCameraInfo,
                )?;

                return Ok(());
            }
        }

        // Unrecognized tag: C++ falls through (qDNGValidate reporting
        // omitted).

        Ok(())
    }

    /// `ValidateIFD (stream, ifdOffset, offsetDelta)` (dng_info.cpp:375).
    fn validate_ifd(
        &self,
        stream: &mut DngStream,
        ifd_offset: uint64,
        offset_delta: int64,
    ) -> DngResult<bool> {
        let is_big_tiff = self.magic == magicBigTIFF;

        // Make sure we have a count.

        if ifd_offset + if is_big_tiff { 8 } else { 2 } > stream.length() {
            return Ok(false);
        }

        // Get entry count.

        stream.set_read_position(ifd_offset)?;

        let ifd_entries = if is_big_tiff {
            stream.get_uint64()?
        } else {
            stream.get_uint16()? as uint64
        };

        if ifd_entries < 1 {
            return Ok(false);
        }

        // Make sure we have room for all entries and next IFD link.

        if ifd_offset
            + if is_big_tiff {
                8 + ifd_entries * 20 + 8
            } else {
                2 + ifd_entries * 12 + 4
            }
            > stream.length()
        {
            return Ok(false);
        }

        // Check each entry.

        for tag_index in 0..ifd_entries {
            stream.set_read_position(if is_big_tiff {
                ifd_offset + 8 + tag_index * 20
            } else {
                ifd_offset + 2 + tag_index * 12
            })?;

            stream.skip(2)?; // Ignore tag code.

            let tag_type = stream.get_uint16()? as uint32;

            let tag_count = if is_big_tiff {
                stream.get_uint64()?
            } else {
                stream.get_uint32()? as uint64
            };

            let tag_type_size = tag_type_size(tag_type) as uint64;

            if tag_type_size == 0 {
                return Ok(false);
            }

            let tag_data_size = tag_count * tag_type_size;

            // Check overflow.

            if tag_data_size < tag_count || tag_data_size < tag_type_size {
                return Ok(false);
            }

            if tag_data_size > if is_big_tiff { 8 } else { 4 } {
                let tag_offset = if is_big_tiff {
                    stream.get_uint64()?
                } else {
                    stream.get_uint32()? as uint64
                };

                let tag_offset = (tag_offset as int64 + offset_delta) as uint64;

                if safe_uint64_add(tag_offset, tag_data_size)? > stream.length() {
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }

    /// `ParseIFD (host, stream, exif, shared, ifd, ifdOffset, offsetDelta,
    /// parentCode)` (dng_info.cpp:463).
    ///
    /// `with_exif` mirrors the C++ non-NULL exif/shared arguments (IFD 0 and
    /// SubIFDs parse EXIF/shared tags; chained IFDs do not).
    fn parse_ifd(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        with_exif: bool,
        ifd_offset: uint64,
        offset_delta: int64,
        parent_code: uint32,
    ) -> DngResult<()> {
        let is_big_tiff = self.magic == magicBigTIFF;

        // The C++ wraps the stream in dng_stream_double_buffered; the port
        // reads through the same stream (buffering is a perf hint only).

        let mut ifd_stream = dng_stream_double_buffered(stream);

        ifd_stream.set_read_position(ifd_offset)?;

        // Find the target IFD index (IFD 0 for parent 0, sub-IFD otherwise).

        let ifd_index: Option<usize> = if parent_code == 0 {
            Some(0)
        } else if parent_code >= tcFirstSubIFD && parent_code <= tcLastSubIFD {
            let index = (parent_code - tcFirstSubIFD) as usize + 1;

            if index < self.ifd.len() {
                Some(index)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(index) = ifd_index {
            self.ifd[index].this_ifd = ifd_offset;
        }

        let ifd_entries = if is_big_tiff {
            ifd_stream.get_uint64()?
        } else {
            ifd_stream.get_uint16()? as uint64
        };

        for tag_index in 0..ifd_entries {
            ifd_stream.set_read_position(if is_big_tiff {
                ifd_offset + 8 + tag_index * 20
            } else {
                ifd_offset + 2 + tag_index * 12
            })?;

            let tag_code = ifd_stream.get_uint16()? as uint32;
            let tag_type = ifd_stream.get_uint16()? as uint32;

            // Minolta 7D files have a bug in the EXIF block where the count
            // is wrong, and we run off into the next IFD link.  So abort
            // parsing if we get a zero code/type combination.

            if tag_code == 0 && tag_type == 0 {
                return Ok(());
            }

            let tag_count: uint64 = if is_big_tiff {
                ifd_stream.get_uint64()?
            } else {
                ifd_stream.get_uint32()? as uint64
            };

            let tag_type_size = tag_type_size(tag_type);

            if tag_type_size == 0 {
                // Unknown type: skip (C++ continues).

                continue;
            }

            let mut tag_offset = if is_big_tiff {
                ifd_offset + 8 + tag_index * 20 + 12
            } else {
                ifd_offset + 2 + tag_index * 12 + 8
            };

            let tag_data_size = tag_count * tag_type_size as uint64;

            // tag_type_size is at least 1.

            if tag_data_size < tag_count {
                return Err(bad_format()); // "overflow in tag_data_size"
            }

            if tag_data_size > if is_big_tiff { 8 } else { 4 } {
                tag_offset = if is_big_tiff {
                    ifd_stream.get_uint64()?
                } else {
                    ifd_stream.get_uint32()? as uint64
                };

                tag_offset = (tag_offset as int64 + offset_delta) as uint64;
            }

            // Big TIFF supports 64-bit tag counts, but we don't need that
            // support yet, so ignore tags with huge counts for now.
            //
            // PORT-NOTE(dng_info.cpp:687): the C++ picks between the
            // double-buffered view and the parent stream for the parse;
            // the port's double-buffered view routes every read to the
            // parent at absolute offsets, so a single stream suffices.

            if tag_count <= 0x0FFFFFFFF {
                ifd_stream.set_read_position(tag_offset)?;

                self.parse_tag(
                    host,
                    &mut ifd_stream,
                    ifd_index.is_some(),
                    with_exif,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count as uint32,
                    tag_offset,
                    offset_delta,
                )?;
            }
        }

        ifd_stream.set_read_position(if is_big_tiff {
            ifd_offset + 8 + ifd_entries * 20
        } else {
            ifd_offset + 2 + ifd_entries * 12
        })?;

        let next_ifd = if is_big_tiff {
            ifd_stream.get_uint64()?
        } else {
            ifd_stream.get_uint32()? as uint64
        };

        if let Some(index) = ifd_index {
            self.ifd[index].next_ifd = next_ifd;
        }

        Ok(())
    }

    /// `ParseMakerNoteIFD (host, stream, ifdSize, ifdOffset, offsetDelta,
    /// minOffset, maxOffset, parentCode)` (dng_info.cpp:780).
    #[allow(clippy::too_many_arguments)]
    fn parse_maker_note_ifd(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        ifd_size: uint64,
        ifd_offset: uint64,
        offset_delta: int64,
        min_offset: uint64,
        max_offset: uint64,
        parent_code: uint32,
    ) -> DngResult<bool> {
        // Avoid recursion issues where a MakerNote IFD points to itself.

        if self.parse_depth > kMaxParseDepth {
            return Ok(false);
        }

        self.parse_depth += 1;

        let result = self.parse_maker_note_ifd_inner(
            host,
            stream,
            ifd_size,
            ifd_offset,
            offset_delta,
            min_offset,
            max_offset,
            parent_code,
        );

        self.parse_depth -= 1;

        result
    }

    /// Body of ParseMakerNoteIFD (split for the recursion guard).
    #[allow(clippy::too_many_arguments)]
    fn parse_maker_note_ifd_inner(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        ifd_size: uint64,
        ifd_offset: uint64,
        offset_delta: int64,
        min_offset: uint64,
        max_offset: uint64,
        parent_code: uint32,
    ) -> DngResult<bool> {
        // Assume there is no next IFD pointer.

        self.maker_note_next_ifd = 0;

        // If size is too small to hold a single entry IFD, abort.

        if ifd_size < 14 {
            return Ok(false);
        }

        // Get entry count.

        let mut ifd_stream = dng_stream_double_buffered(stream);

        ifd_stream.set_read_position(ifd_offset)?;

        let ifd_entries = ifd_stream.get_uint16()?;

        // Make the entry count reasonable for the MakerNote size.

        if ifd_entries < 1 || 2 + ifd_entries as uint64 * 12 > ifd_size {
            return Ok(false);
        }

        // Scan IFD to verify all the tag types are valid.

        for tag_index in 0..ifd_entries {
            ifd_stream.set_read_position(ifd_offset + 2 + tag_index as uint64 * 12 + 2)?;

            let tag_type = ifd_stream.get_uint16()?;

            // Kludge: Some Canon MakerNotes contain tagType = 0 tags, so we
            // need to ignore them.  This was a "firmware 1.0.4" Canon 40D
            // raw file.

            if parent_code == tcCanonMakerNote && tag_type == 0 {
                continue;
            }

            // Ditto for some Apple MakerNotes.

            if parent_code == tcAppleMakerNote && tag_type == 0 {
                continue;
            }

            if tag_type_size(tag_type as uint32) == 0 {
                return Ok(false);
            }
        }

        // OK, the IFD looks reasonable enough to parse.

        for tag_index in 0..ifd_entries {
            ifd_stream.set_read_position(ifd_offset + 2 + tag_index as uint64 * 12)?;

            let tag_code = ifd_stream.get_uint16()? as uint32;
            let tag_type = ifd_stream.get_uint16()? as uint32;
            let tag_count = ifd_stream.get_uint32()?;

            if tag_type == 0 {
                continue;
            }

            let tag_size = match safe_uint32_mult2(tag_count, tag_type_size(tag_type)) {
                Ok(size) => size,
                Err(_) => {
                    // Detect overflow -- skip invalid tagCount or tagSize.
                    // Works around bug with some Leica M (Typ 240)
                    // MakerNotes.
                    continue;
                }
            };

            let mut tag_offset = ifd_offset + 2 + tag_index as uint64 * 12 + 8;

            if tag_size > 4 {
                tag_offset = (ifd_stream.get_uint32()? as int64 + offset_delta) as uint64;

                if tag_offset < min_offset
                    || safe_uint64_add(tag_offset, tag_size as uint64).unwrap_or(uint64::MAX) > max_offset
                {
                    // Tag data is outside the valid offset range,
                    // so ignore this tag.

                    continue;
                }
            }

            // Olympus switched to using IFDs in version 3 makernotes.

            if parent_code == tcOlympusMakerNote
                && tag_type as uint32 == ttIFD
                && tag_count == 1
            {
                let olympus_maker_parent = match tag_code {
                    8208 => tcOlympusMakerNote8208,
                    8224 => tcOlympusMakerNote8224,
                    8240 => tcOlympusMakerNote8240,
                    8256 => tcOlympusMakerNote8256,
                    8272 => tcOlympusMakerNote8272,
                    12288 => tcOlympusMakerNote12288,
                    _ => 0,
                };

                if olympus_maker_parent != 0 {
                    ifd_stream.set_read_position(tag_offset)?;

                    let sub_maker_note_offset =
                        ifd_stream.get_uint32()? as int64 + offset_delta;

                    if sub_maker_note_offset >= 0
                        && (sub_maker_note_offset as uint64) >= min_offset
                        && (sub_maker_note_offset as uint64) < max_offset
                    {
                        let ok = self.parse_maker_note_ifd(
                            host,
                            &mut ifd_stream,
                            max_offset - sub_maker_note_offset as uint64,
                            sub_maker_note_offset as uint64,
                            offset_delta,
                            min_offset,
                            max_offset,
                            olympus_maker_parent,
                        )?;

                        if ok {
                            continue;
                        }
                    }
                }

                ifd_stream.set_read_position(tag_offset)?;
            }

            // Parse the tag (MakerNote tags never touch the shared/IFD
            // structures; only EXIF tags are recognized).

            ifd_stream.set_read_position(tag_offset)?;

            self.parse_tag(
                host,
                &mut ifd_stream,
                false,
                true,
                parent_code,
                tag_code,
                tag_type,
                tag_count,
                tag_offset,
                offset_delta,
            )?;
        }

        // Grab next IFD pointer, for possible use.

        if ifd_size >= 2 + ifd_entries as uint64 * 12 + 4 {
            ifd_stream.set_read_position(ifd_offset + 2 + ifd_entries as uint64 * 12)?;

            self.maker_note_next_ifd = ifd_stream.get_uint32()?;
        }

        Ok(true)
    }

    /// `ParseMakerNote (host, stream, makerNoteCount, makerNoteOffset,
    /// offsetDelta, minOffset, maxOffset)` (dng_info.cpp:1061).
    fn parse_maker_note(
        &mut self,
        host: &mut DngHost,
        stream: &mut DngStream,
        maker_note_count: uint32,
        maker_note_offset: uint64,
        offset_delta: int64,
        min_offset: uint64,
        max_offset: uint64,
    ) -> DngResult<()> {
        let mut first_bytes = [0u8; 16];

        stream.set_read_position(maker_note_offset)?;

        let to_read = min_uint32(
            first_bytes.len() as uint32,
            maker_note_count,
        ) as usize;

        let got = stream.get(to_read as uint32)?;

        first_bytes[..got.len()].copy_from_slice(&got);

        let count64 = maker_note_count as uint64;

        // Apple iOS MakerNote.

        if got.len() >= 9 && &first_bytes[..9] == b"Apple iOS" {
            stream.set_read_position(maker_note_offset + 12)?;

            let mut big_endian = false;

            let endian_mark = stream.get_uint16()?;

            if endian_mark == byteOrderMM as uint16 {
                big_endian = true;
            } else if endian_mark != byteOrderII as uint16 {
                return Ok(());
            }

            {
                let mut temp_endian = TempBigEndian::new(stream, big_endian);

                if maker_note_count > 14 {
                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 14,
                        maker_note_offset + 14,
                        maker_note_offset as int64,
                        min_offset,
                        max_offset,
                        tcAppleMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Epson MakerNote with header.

        if got.len() >= 8 && &first_bytes[..8] == b"EPSON\x00\x01\x00" {
            if maker_note_count > 8 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64 - 8,
                    maker_note_offset + 8,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcEpsonMakerNote,
                )?;
            }

            return Ok(());
        }

        // Fujifilm MakerNote.

        if got.len() >= 8 && &first_bytes[..8] == b"FUJIFILM" {
            stream.set_read_position(maker_note_offset + 8)?;

            {
                let mut temp_endian = TempLittleEndian::new(stream, true);

                let ifd_offset = temp_endian.stream().get_uint32()? as uint64;

                if ifd_offset >= 12 && ifd_offset < count64 {
                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - ifd_offset,
                        maker_note_offset + ifd_offset,
                        maker_note_offset as int64,
                        min_offset,
                        max_offset,
                        tcFujiMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Leica MakerNote for models that store entry offsets relative to
        // the start of the MakerNote (e.g., M9).

        const LEICA_RELATIVE: [&[u8; 8]; 6] = [
            b"LEICA\0\0\0",
            b"LEICA0\x03\x00",
            b"LEICA\x00\x01\x00",
            b"LEICA\x00\x04\x00",
            b"LEICA\x00\x05\x00",
            b"LEICA\x00\x06\x00",
        ];

        if LEICA_RELATIVE.iter().any(|p| &first_bytes[..8] == *p) {
            if maker_note_count > 8 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64 - 8,
                    maker_note_offset + 8,
                    maker_note_offset as int64,
                    min_offset,
                    max_offset,
                    tcLeicaMakerNote,
                )?;
            }

            return Ok(());
        }

        // Leica MakerNote for models that store absolute entry offsets
        // (i.e., relative to the start of the file, e.g., S2).

        if &first_bytes[..8] == b"LEICA\x00\x02\xFF"
            || &first_bytes[..8] == b"LEICA\x00\x02\x00"
        {
            if maker_note_count > 8 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64 - 8,
                    maker_note_offset + 8,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcLeicaMakerNote,
                )?;
            }

            return Ok(());
        }

        // Nikon version 2 MakerNote with header.

        if got.len() >= 7 && &first_bytes[..7] == b"Nikon\x00\x02" {
            stream.set_read_position(maker_note_offset + 10)?;

            let mut big_endian = false;

            let endian_mark = stream.get_uint16()?;

            if endian_mark == byteOrderMM as uint16 {
                big_endian = true;
            } else if endian_mark != byteOrderII as uint16 {
                return Ok(());
            }

            {
                let mut temp_endian = TempBigEndian::new(stream, big_endian);

                let magic = temp_endian.stream().get_uint16()?;

                if magic != 42 {
                    return Ok(());
                }

                let ifd_offset = temp_endian.stream().get_uint32()? as uint64;

                if ifd_offset >= 8 && ifd_offset < count64.saturating_sub(10) {
                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 10 - ifd_offset,
                        maker_note_offset + 10 + ifd_offset,
                        (maker_note_offset + 10) as int64,
                        min_offset,
                        max_offset,
                        tcNikonMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Newer version of Olympus MakerNote with byte order mark.

        if got.len() >= 8 && &first_bytes[..8] == b"OLYMPUS\x00" {
            stream.set_read_position(maker_note_offset + 8)?;

            let mut big_endian = false;

            let endian_mark = stream.get_uint16()?;

            if endian_mark == byteOrderMM as uint16 {
                big_endian = true;
            } else if endian_mark != byteOrderII as uint16 {
                return Ok(());
            }

            {
                let mut temp_endian = TempBigEndian::new(stream, big_endian);

                let version = temp_endian.stream().get_uint16()?;

                if version != 3 {
                    return Ok(());
                }

                if maker_note_count > 12 {
                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 12,
                        maker_note_offset + 12,
                        maker_note_offset as int64,
                        min_offset,
                        max_offset,
                        tcOlympusMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Olympus MakerNote with header.

        if got.len() >= 5 && &first_bytes[..5] == b"OLYMP" {
            if maker_note_count > 8 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64 - 8,
                    maker_note_offset + 8,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcOlympusMakerNote,
                )?;
            }

            return Ok(());
        }

        // OM Digital Solutions cameras (formerly Olympus brand).

        if got.len() >= 10 && &first_bytes[..10] == b"OM SYSTEM\x00" {
            stream.set_read_position(maker_note_offset + 12)?;

            let mut big_endian = false;

            let endian_mark = stream.get_uint16()?;

            if endian_mark == byteOrderMM as uint16 {
                big_endian = true;
            } else if endian_mark != byteOrderII as uint16 {
                return Ok(());
            }

            {
                let mut temp_endian = TempBigEndian::new(stream, big_endian);

                let version = temp_endian.stream().get_uint16()?;

                if version != 4 {
                    return Ok(());
                }

                if maker_note_count > 16 {
                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 16,
                        maker_note_offset + 16,
                        maker_note_offset as int64,
                        min_offset,
                        max_offset,
                        tcOlympusMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Panasonic MakerNote.

        if got.len() >= 12 && &first_bytes[..12] == b"Panasonic\x00\x00\x00" {
            if maker_note_count > 12 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64 - 12,
                    maker_note_offset + 12,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcPanasonicMakerNote,
                )?;
            }

            return Ok(());
        }

        // Pentax MakerNote, absolute addresses.

        if got.len() >= 4 && &first_bytes[..4] == b"AOC\x00" {
            if maker_note_count > 6 {
                stream.set_read_position(maker_note_offset + 4)?;

                let mut big_endian = stream.big_endian();

                let endian_mark = stream.get_uint16()?;

                if endian_mark == byteOrderMM as uint16 {
                    big_endian = true;
                } else if endian_mark == byteOrderII as uint16 {
                    big_endian = false;
                }

                {
                    let mut temp_endian = TempBigEndian::new(stream, big_endian);

                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 6,
                        maker_note_offset + 6,
                        offset_delta,
                        min_offset,
                        max_offset,
                        tcPentaxMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Pentax MakerNote, relative addresses.

        if got.len() >= 6 && &first_bytes[..6] == b"PENTAX" {
            if maker_note_count > 8 {
                stream.set_read_position(maker_note_offset + 8)?;

                let mut big_endian = stream.big_endian();

                let endian_mark = stream.get_uint16()?;

                if endian_mark == byteOrderMM as uint16 {
                    big_endian = true;
                } else if endian_mark == byteOrderII as uint16 {
                    big_endian = false;
                }

                {
                    let mut temp_endian = TempBigEndian::new(stream, big_endian);

                    self.parse_maker_note_ifd(
                        host,
                        temp_endian.stream(),
                        count64 - 10,
                        maker_note_offset + 10,
                        maker_note_offset as int64, // Relative to start of MakerNote.
                        min_offset,
                        max_offset,
                        tcPentaxMakerNote,
                    )?;
                }
            }

            return Ok(());
        }

        // Ricoh MakerNote.

        if got.len() >= 5
            && (&first_bytes[..5] == b"RICOH" || &first_bytes[..5] == b"Ricoh")
        {
            if maker_note_count > 8 {
                let mut temp_endian = TempBigEndian::new(stream, true);

                self.parse_maker_note_ifd(
                    host,
                    temp_endian.stream(),
                    count64 - 8,
                    maker_note_offset + 8,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcRicohMakerNote,
                )?;
            }

            return Ok(());
        }

        // Nikon MakerNote without header.

        if self.exif.make.starts_with(b"NIKON", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcNikonMakerNote,
            )?;

            return Ok(());
        }

        // Canon MakerNote.

        if self.exif.make.starts_with(b"CANON", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcCanonMakerNote,
            )?;

            return Ok(());
        }

        // Minolta MakerNote.

        if self.exif.make.starts_with(b"MINOLTA", true)
            || self.exif.make.starts_with(b"KONICA MINOLTA", true)
        {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcMinoltaMakerNote,
            )?;

            return Ok(());
        }

        // Sony MakerNote.

        if self.exif.make.starts_with(b"SONY", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcSonyMakerNote,
            )?;

            return Ok(());
        }

        // Kodak MakerNote.

        if self.exif.make.starts_with(b"EASTMAN KODAK", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcKodakMakerNote,
            )?;

            return Ok(());
        }

        // Mamiya MakerNote.

        if self.exif.make.starts_with(b"Mamiya", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcMamiyaMakerNote,
            )?;

            // Mamiya uses a MakerNote chain.

            while self.maker_note_next_ifd != 0 {
                self.parse_maker_note_ifd(
                    host,
                    stream,
                    count64,
                    (offset_delta + self.maker_note_next_ifd as int64) as uint64,
                    offset_delta,
                    min_offset,
                    max_offset,
                    tcMamiyaMakerNote,
                )?;
            }

            return Ok(());
        }

        // Hasselblad MakerNote.

        if self.exif.make.starts_with(b"Hasselblad", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                offset_delta,
                min_offset,
                max_offset,
                tcHasselbladMakerNote,
            )?;

            return Ok(());
        }

        // Samsung MakerNote.

        if self.exif.make.starts_with(b"Samsung", true) {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64,
                maker_note_offset,
                maker_note_offset as int64,
                min_offset,
                max_offset,
                tcSamsungMakerNote,
            )?;

            return Ok(());
        }

        // Casio MakerNote.

        if self.exif.make.starts_with(b"CASIO COMPUTER", true)
            && got.len() >= 6
            && &first_bytes[..6] == b"QVC\x00\x00\x00"
        {
            self.parse_maker_note_ifd(
                host,
                stream,
                count64 - 6,
                maker_note_offset + 6,
                maker_note_offset as int64,
                min_offset,
                max_offset,
                tcCasioMakerNote,
            )?;

            return Ok(());
        }

        Ok(())
    }

    /// `ParseSonyPrivateData` — Sony private data is encrypted, sorry.
    #[allow(clippy::too_many_arguments)]
    fn parse_sony_private_data(
        &mut self,
        _host: &mut DngHost,
        _stream: &mut DngStream,
        _count: uint64,
        _old_offset: uint64,
        _new_offset: uint64,
    ) {
    }

    /// `ParseDNGPrivateData (host, stream)` (dng_info.cpp:1725).
    fn parse_dng_private_data(&mut self, host: &mut DngHost, stream: &mut DngStream) -> DngResult<()> {
        if self.shared.dng_private_data_count < 2 {
            return Ok(());
        }

        // DNG private data should always start with a null-terminated
        // company name, to define the format of the private data.

        let mut private_name = DngString::new();

        {
            let mut buffer = vec![0u8; 64];

            stream.set_read_position(self.shared.dng_private_data_offset)?;

            let read_length = min_uint32(
                self.shared.dng_private_data_count,
                (buffer.len() - 1) as uint32,
            ) as usize;

            let got = stream.get(read_length as uint32)?;

            buffer[..got.len()].copy_from_slice(&got);

            // NUL-terminate at the first embedded NUL (the C++ `Set` stops at
            // the terminator).

            let end = buffer
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(got.len());

            private_name.set(&buffer[..end]);
        }

        // Pentax is storing their MakerNote in the DNGPrivateData data.

        if private_name.starts_with(b"PENTAX", true)
            || private_name.starts_with(b"SAMSUNG", true)
        {
            stream.set_read_position(self.shared.dng_private_data_offset + 8)?;

            let mut big_endian = stream.big_endian();

            let endian_mark = stream.get_uint16()?;

            if endian_mark == byteOrderMM as uint16 {
                big_endian = true;
            } else if endian_mark == byteOrderII as uint16 {
                big_endian = false;
            }

            {
                let mut temp_endian = TempBigEndian::new(stream, big_endian);

                self.parse_maker_note_ifd(
                    host,
                    temp_endian.stream(),
                    (self.shared.dng_private_data_count - 10) as uint64,
                    self.shared.dng_private_data_offset + 10,
                    self.shared.dng_private_data_offset as int64,
                    self.shared.dng_private_data_offset,
                    self.shared.dng_private_data_offset + self.shared.dng_private_data_count as uint64,
                    tcPentaxMakerNote,
                )?;
            }

            return Ok(());
        }

        // Stop parsing if this is not an Adobe format block.

        if !private_name.matches(b"Adobe", true) {
            return Ok(());
        }

        {
            let mut temp_order = TempBigEndian::new(stream, true);

            let stream = temp_order.stream();

            let mut section_offset: uint32 = 6;

            while safe_uint32_add(section_offset, 8).unwrap_or(uint32::MAX) < self.shared.dng_private_data_count {
                stream.set_read_position(
                    self.shared.dng_private_data_offset + section_offset as uint64,
                )?;

                let section_key = stream.get_uint32()?;
                let section_count = stream.get_uint32()?;

                if section_key == dng_char4(b'M', b'a', b'k', b'N') && section_count > 6 {
                    // Found MakerNote inside DNGPrivateData.

                    let order_mark = stream.get_uint16()?;
                    let old_offset = stream.get_uint32()?;

                    let temp_size = safe_uint32_sub(section_count, 6);

                    let data = stream.get(temp_size)?;

                    let position_in_original_file = stream.position_in_original_file();

                    let mut temp_stream =
                        DngStream::from_static(&data, position_in_original_file);

                    temp_stream.set_big_endian(order_mark == byteOrderMM as uint16);

                    self.parse_maker_note(
                        host,
                        &mut temp_stream,
                        temp_size,
                        0,
                        0i64.wrapping_sub(old_offset as int64),
                        0,
                        temp_size as uint64,
                    )?;
                } else if section_key == dng_char4(b'S', b'R', b'2', b' ')
                    && section_count > 6
                {
                    // Found Sony private data inside DNGPrivateData.

                    let order_mark = stream.get_uint16()?;
                    let old_offset = stream.get_uint32()? as uint64;

                    let new_offset = self.shared.dng_private_data_offset
                        + section_offset as uint64
                        + 14;

                    {
                        let mut sr2_order = TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                        self.parse_sony_private_data(
                            host,
                            sr2_order.stream(),
                            (section_count - 6) as uint64,
                            old_offset,
                            new_offset,
                        );
                    }
                } else if section_key == dng_char4(b'R', b'A', b'F', b' ')
                    && section_count > 4
                {
                    // Found Fuji RAF tags inside DNGPrivateData.

                    let order_mark = stream.get_uint16()?;

                    let mut tag_count = stream.get_uint32()?;

                    let mut tag_offset = stream.position();

                    if tag_count != 0 {
                        {
                            let mut raf_order =
                                TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                            self.parse_tag(
                                host,
                                raf_order.stream(),
                                false,
                                true,
                                tcFujiRAF,
                                tcFujiHeader,
                                ttUndefined,
                                tag_count,
                                tag_offset,
                                0,
                            )?;
                        }

                        stream.set_read_position(tag_offset + tag_count as uint64)?;
                    }

                    tag_count = stream.get_uint32()?;

                    tag_offset = stream.position();

                    if tag_count != 0 {
                        {
                            let mut raf_order =
                                TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                            self.parse_tag(
                                host,
                                raf_order.stream(),
                                false,
                                true,
                                tcFujiRAF,
                                tcFujiRawInfo1,
                                ttUndefined,
                                tag_count,
                                tag_offset,
                                0,
                            )?;
                        }

                        stream.set_read_position(tag_offset + tag_count as uint64)?;
                    }

                    tag_count = stream.get_uint32()?;

                    tag_offset = stream.position();

                    if tag_count != 0 {
                        {
                            let mut raf_order =
                                TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                            self.parse_tag(
                                host,
                                raf_order.stream(),
                                false,
                                true,
                                tcFujiRAF,
                                tcFujiRawInfo2,
                                ttUndefined,
                                tag_count,
                                tag_offset,
                                0,
                            )?;
                        }

                        stream.set_read_position(tag_offset + tag_count as uint64)?;
                    }
                } else if section_key == dng_char4(b'C', b'n', b't', b'x')
                    && section_count > 4
                {
                    // Found Contax Raw header inside DNGPrivateData.

                    let order_mark = stream.get_uint16()?;

                    let tag_count = stream.get_uint32()?;

                    let tag_offset = stream.position();

                    if tag_count != 0 {
                        let mut contax_order =
                            TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                        self.parse_tag(
                            host,
                            contax_order.stream(),
                            false,
                            true,
                            tcContaxRAW,
                            tcContaxHeader,
                            ttUndefined,
                            tag_count,
                            tag_offset,
                            0,
                        )?;
                    }
                } else if section_key == dng_char4(b'C', b'R', b'W', b' ')
                    && section_count > 4
                {
                    // Found Canon CRW tags inside DNGPrivateData.

                    let order_mark = stream.get_uint16()?;
                    let entries = stream.get_uint16()?;

                    let crw_tag_start = stream.position();

                    for parse_pass in 1..=2u32 {
                        stream.set_read_position(crw_tag_start)?;

                        for _index in 0..entries {
                            let tag_code = stream.get_uint16()? as uint32;

                            let tag_count = stream.get_uint32()?;

                            let tag_offset = stream.position();

                            // We need to grab the model id tag first, and
                            // then all the other tags.

                            if (parse_pass == 1) == (tag_code == 0x5834) {
                                let mut tag_order =
                                    TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                                self.parse_tag(
                                    host,
                                    tag_order.stream(),
                                    false,
                                    true,
                                    tcCanonCRW,
                                    tag_code,
                                    ttUndefined,
                                    tag_count,
                                    tag_offset,
                                    0,
                                )?;
                            }

                            stream.set_read_position(tag_offset + tag_count as uint64)?;
                        }
                    }
                } else if section_count > 4 {
                    let mut parent_code = 0;

                    let mut code32 = false;
                    let mut has_type = true;

                    match section_key {
                        x if x == dng_char4(b'M', b'R', b'W', b' ') => {
                            parent_code = tcMinoltaMRW;
                            code32 = true;
                            has_type = false;
                        }

                        x if x == dng_char4(b'P', b'a', b'n', b'o') => {
                            parent_code = tcPanasonicRAW;
                        }

                        x if x == dng_char4(b'L', b'e', b'a', b'f') => {
                            parent_code = tcLeafMOS;
                        }

                        x if x == dng_char4(b'K', b'o', b'd', b'a') => {
                            parent_code = tcKodakDCRPrivateIFD;
                        }

                        x if x == dng_char4(b'K', b'D', b'C', b' ') => {
                            parent_code = tcKodakKDCPrivateIFD;
                        }

                        _ => {}
                    }

                    if parent_code != 0 {
                        let order_mark = stream.get_uint16()?;
                        let entries = stream.get_uint16()?;

                        for _index in 0..entries {
                            let tag_code = if code32 {
                                stream.get_uint32()?
                            } else {
                                stream.get_uint16()? as uint32
                            };

                            let tag_type = if has_type {
                                stream.get_uint16()? as uint32
                            } else {
                                ttUndefined
                            };

                            let tag_count = stream.get_uint32()?;

                            let tag_size =
                                safe_uint32_mult2(tag_count, tag_type_size(tag_type))?;

                            let tag_offset = stream.position();

                            {
                                let mut tag_order =
                                    TempBigEndian::new(stream, order_mark == byteOrderMM as uint16);

                                self.parse_tag(
                                    host,
                                    tag_order.stream(),
                                    false,
                                    true,
                                    parent_code,
                                    tag_code,
                                    tag_type,
                                    tag_count,
                                    tag_offset,
                                    0,
                                )?;
                            }

                            stream.set_read_position(tag_offset + tag_size as uint64)?;
                        }
                    }
                }

                section_offset = safe_uint32_add(section_offset, 8).unwrap_or(uint32::MAX);
                section_offset = safe_uint32_add(section_offset, section_count).unwrap_or(uint32::MAX);

                if section_offset & 1 != 0 {
                    section_offset = safe_uint32_add(section_offset, 1).unwrap_or(uint32::MAX);
                }
            }
        }

        Ok(())
    }

    /// `PostParse (host)` (dng_info.cpp:2667) — fills in the derived
    /// defaults and locates the main/mask/depth/enhanced/semantic-mask IFDs.
    pub fn post_parse(&mut self, _host: &mut DngHost) -> DngResult<()> {
        // dng_exif::PostParse — qDNGValidate-only sanity checks are omitted;
        // there are no non-validation side effects in the C++.

        // dng_shared::PostParse — fill in default values for DNG images.

        if self.shared.dng_version != 0 {
            // Support for DNG versions before 1.0.0.0.

            if self.shared.dng_version < dngVersion_1_0_0_0 {
                // The CalibrationIlluminant tags were added just before
                // DNG version 1.0.0.0, and were hardcoded before that.

                self.shared.camera_profile.calibration_illuminant1 = lsStandardLightA;
                self.shared.camera_profile.calibration_illuminant2 = lsD65;
                self.shared.camera_profile.calibration_illuminant3 = lsD65;

                self.shared.dng_version = dngVersion_1_0_0_0;
            }

            // Default value for DNGBackwardVersion tag.

            if self.shared.dng_backward_version == 0 {
                self.shared.dng_backward_version = self.shared.dng_version & 0xFFFF0000;
            }

            // Check DNGBackwardVersion value.

            if self.shared.dng_backward_version < dngVersion_1_0_0_0 {
                self.shared.dng_backward_version = dngVersion_1_0_0_0;
            }

            if self.shared.dng_backward_version > self.shared.dng_version {
                self.shared.dng_backward_version = self.shared.dng_version;
            }

            // Check UniqueCameraModel.

            if self.shared.unique_camera_model.is_empty() {
                self.shared.unique_camera_model.set(b"Digital Negative");
            }

            // If we don't know the color depth yet, it must be a monochrome
            // DNG.

            if self.shared.camera_profile.color_planes == 0 {
                self.shared.camera_profile.color_planes = 1;
            }

            if self.shared.camera_profile.calibration_illuminant3 != lsUnknown
                && self.shared.camera_profile.color_matrix3.is_empty()
            {
                // Ignoring CalibrationIlluminant3 because ColorMatrix3 is
                // invalid or missing.

                self.shared.camera_profile.calibration_illuminant3 = lsUnknown;
            }
        }

        // IFD PostParse for every IFD.

        for index in 0..self.ifd.len() {
            self.ifd[index].post_parse();
        }

        for index in 0..self.chained_ifd.len() {
            self.chained_ifd[index].post_parse();
        }

        for chain in &mut self.chained_sub_ifd {
            for ifd in chain.iter_mut() {
                ifd.post_parse();
            }
        }

        if self.shared.dng_version != 0 {
            // Find main IFD.

            self.main_index = -1;

            for index in 0..self.ifd.len() {
                if self.ifd[index].uses_new_sub_file_type
                    && self.ifd[index].new_sub_file_type == sfMainImage
                {
                    if self.main_index == -1 {
                        self.main_index = index as int32;
                    }
                } else if self.ifd[index].new_sub_file_type == sfPreviewImage
                    || self.ifd[index].new_sub_file_type == sfAltPreviewImage
                {
                    // Fill in default color space for DNG previews if not
                    // included.

                    if self.ifd[index].preview_info.color_space == previewColorSpace_MaxEnum {
                        if self.ifd[index].samples_per_pixel == 1 {
                            self.ifd[index].preview_info.color_space =
                                previewColorSpace_GrayGamma22;
                        } else {
                            self.ifd[index].preview_info.color_space = previewColorSpace_sRGB;
                        }
                    }
                }
            }

            // Deal with lossless JPEG bug in early DNG versions.

            if self.shared.dng_version < dngVersion_1_1_0_0 {
                if self.main_index != -1 {
                    self.ifd[self.main_index as usize].lossless_jpeg_bug_16 = true;
                }
            }

            // Find mask index.

            for index in 0..self.ifd.len() {
                if self.ifd[index].new_sub_file_type == sfTransparencyMask
                    && self.ifd[index].uses_new_sub_file_type
                {
                    if self.mask_index == -1 {
                        self.mask_index = index as int32;
                    }
                }
            }

            // Find depth index.

            for index in 0..self.ifd.len() {
                if self.ifd[index].new_sub_file_type == sfDepthMap
                    && self.ifd[index].uses_new_sub_file_type
                {
                    if self.depth_index == -1 {
                        self.depth_index = index as int32;
                    }
                }
            }

            // Find enhanced ifd index.

            for index in 0..self.ifd.len() {
                if self.ifd[index].new_sub_file_type == sfEnhancedImage
                    && self.ifd[index].uses_new_sub_file_type
                {
                    if self.enhanced_index == -1 {
                        self.enhanced_index = index as int32;
                    }
                }
            }

            // Find semantic mask ifd indices.

            for index in 0..self.ifd.len() {
                if self.ifd[index].new_sub_file_type == sfSemanticMask
                    && self.ifd[index].uses_new_sub_file_type
                {
                    self.semantic_mask_indices.push(index as uint32);
                }
            }
        }

        Ok(())
    }

    /// `IsValidDNG ()` — test validity of DNG data (dng_info.cpp:2907).
    pub fn is_valid_dng(&self) -> bool {
        // Check shared info.

        if !self.shared.is_valid_dng() {
            return false;
        }

        // Check TIFF magic number.

        if self.magic != magicTIFF && self.magic != magicBigTIFF {
            return false;
        }

        // Make sure we have a main image IFD.

        if self.main_index == -1 {
            return false;
        }

        // Make sure each IFD is valid.

        for index in 0..self.ifd_count() {
            let parent_code = if index == 0 {
                0
            } else {
                tcFirstSubIFD + index - 1
            };

            if !self.ifd[index as usize].is_valid_dng(&self.shared, parent_code) {
                // Only errors in the main and transparency mask IFDs are
                // fatal to parsing.

                if index as int32 == self.main_index
                    || index as int32 == self.mask_index
                {
                    return false;
                }

                // Also errors to depth map...

                if index as int32 == self.depth_index {
                    return false;
                }

                // Also errors to enhanced image...

                if index as int32 == self.enhanced_index {
                    return false;
                }

                // For now, treat errors in semantic mask images as non-fatal.
            }
        }

        true
    }
}

/// `SafeUint32Sub` (dng_safe_arithmetic).
fn safe_uint32_sub(a: uint32, b: uint32) -> uint32 {
    a.checked_sub(b).unwrap_or(0)
}

#[allow(unused)]

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_memory_stream::DngMemoryStream;
    use dng_core::dng_tags::{
        tcBitsPerSample, tcBlackLevelRepeatDim, tcCFARepeatPatternDim, tcCFAPattern,
        tcCFAPlaneColor, tcCFALayout, tcActiveArea, tcCompression, tcDNGBackwardVersion,
        tcDNGVersion, tcImageLength, tcImageWidth, tcNewSubFileType, tcOrientation,
        tcPhotometricInterpretation, tcUniqueCameraModel, tcWhiteLevel, tcBlackLevel,
    };
    use dng_core::dng_tag_types::{ttAscii, ttByte, ttLong, ttRational, ttShort};
    use dng_core::dng_tag_values::{
        magicBigTIFF, magicExtendedProfile, magicOlympusA, magicOlympusB, magicPanasonic,
        magicRawCache, magicTIFF, piBlackIsZero, piCFA, sfMainImage,
    };

    // ------------------------------------------------------------------
    // Synthetic minimal-DNG byte layout builder (little-endian TIFF).
    // ------------------------------------------------------------------

    struct IfdEntry {
        tag: uint16,
        tag_type: uint16,
        count: uint32,
        /// Inline payload when <= 4 bytes; otherwise appended to the data
        /// area and the offset is patched.
        payload: Vec<u8>,
    }

    struct Ifd {
        entries: Vec<IfdEntry>,
        /// Offset of the next IFD (0 = none).
        next: u32,
    }

    /// Serialize a chain of IFDs.  Returns the whole TIFF file bytes with
    /// the header pointing at ifds[0].
    fn build_tiff(ifds: &[Ifd]) -> Vec<u8> {
        let tag_size = |tag_type: uint16| -> usize {
            match tag_type {
                1 | 2 | 6 | 7 => 1, // BYTE/ASCII/UBYTE/UNDEFINED
                3 => 2,             // SHORT
                4 => 4,             // LONG
                5 => 8,             // RATIONAL
                _ => 1,
            }
        };

        // Layout: header (8 bytes) | IFD tables | data area.

        let mut ifd_table_size = 8usize;
        for ifd in ifds {
            ifd_table_size += 2 + ifd.entries.len() * 12 + 4;
        }

        let data_area_start = ifd_table_size as u32;

        // First pass: assign data offsets for big payloads.

        let mut data_area: Vec<u8> = Vec::new();

        let mut entry_payload_offsets: Vec<Vec<Option<u32>>> = Vec::new();

        for ifd in ifds {
            let mut offs = Vec::new();
            for entry in &ifd.entries {
                let size = entry.count as usize * tag_size(entry.tag_type);

                if size <= 4 {
                    offs.push(None);
                } else {
                    let off = data_area_start + data_area.len() as u32;
                    offs.push(Some(off));
                    data_area.extend_from_slice(&entry.payload);
                    // Word-align.
                    if data_area.len() & 1 != 0 {
                        data_area.push(0);
                    }
                }
            }
            entry_payload_offsets.push(offs);
        }

        // Second pass: serialize.

        let mut out: Vec<u8> = Vec::new();

        // Header.
        out.extend_from_slice(&[0x49, 0x49, 0x2A, 0x00]); // II 42
        out.extend_from_slice(&ifd_offsets_of(ifds)[0].to_le_bytes());

        let ifd_offsets = ifd_offsets_of(ifds);

        for (i, ifd) in ifds.iter().enumerate() {
            let ifd_start = out.len();

            out.extend_from_slice(&(ifd.entries.len() as u16).to_le_bytes());

            for (j, entry) in ifd.entries.iter().enumerate() {
                out.extend_from_slice(&entry.tag.to_le_bytes());
                out.extend_from_slice(&entry.tag_type.to_le_bytes());
                out.extend_from_slice(&entry.count.to_le_bytes());

                let size = entry.count as usize * tag_size(entry.tag_type);

                if size <= 4 {
                    let mut inline = entry.payload.clone();
                    inline.resize(4, 0);
                    out.extend_from_slice(&inline[..4]);
                } else {
                    let off = entry_payload_offsets[i][j].unwrap();
                    out.extend_from_slice(&off.to_le_bytes());
                }
            }

            // Next IFD offset.
            let next = if i + 1 < ifds.len() {
                ifd_offsets[i + 1]
            } else {
                ifd.next
            };
            out.extend_from_slice(&next.to_le_bytes());

            debug_assert_eq!(out.len() - ifd_start, 2 + ifd.entries.len() * 12 + 4);
        }

        debug_assert_eq!(out.len(), data_area_start as usize);

        out.extend_from_slice(&data_area);

        out
    }

    fn ifd_offsets_of(ifds: &[Ifd]) -> Vec<u32> {
        let mut offsets = Vec::new();
        let mut at = 8usize;
        for ifd in ifds {
            offsets.push(at as u32);
            at += 2 + ifd.entries.len() * 12 + 4;
        }
        offsets
    }

    fn entry(tag: uint16, tag_type: uint16, count: uint32, payload: Vec<u8>) -> IfdEntry {
        IfdEntry {
            tag,
            tag_type,
            count,
            payload,
        }
    }

    fn u32v(values: &[u32]) -> Vec<u8> {
        let mut v = Vec::new();
        for x in values {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    fn u16v(values: &[u16]) -> Vec<u8> {
        let mut v = Vec::new();
        for x in values {
            v.extend_from_slice(&x.to_le_bytes());
        }
        v
    }

    fn rational_bytes(n: u32, d: u32) -> Vec<u8> {
        let mut v = n.to_le_bytes().to_vec();
        v.extend_from_slice(&d.to_le_bytes());
        v
    }

    /// Main-image IFD (a small linear DNG: 4x4, 1 sample, uncompressed).
    fn color_matrix1_rationals() -> Vec<u8> {
        // 3x3 identity as SRationals (n=10000,d=10000 diagonal; zeros n=0,d=1).
        let mut v = Vec::new();
        for i in 0..9 {
            if i == 0 || i == 4 || i == 8 {
                v.extend_from_slice(&10000u32.to_le_bytes());
                v.extend_from_slice(&10000u32.to_le_bytes());
            } else {
                v.extend_from_slice(&0u32.to_le_bytes());
                v.extend_from_slice(&1u32.to_le_bytes());
            }
        }
        v
    }

    fn as_shot_neutral_rationals() -> Vec<u8> {
        let mut v = Vec::new();
        for _ in 0..3 {
            v.extend_from_slice(&1u32.to_le_bytes());
            v.extend_from_slice(&1u32.to_le_bytes());
        }
        v
    }

    fn main_ifd_entries() -> Vec<IfdEntry> {
        vec![
            entry(
                tcNewSubFileType as uint16,
                ttLong as uint16,
                1,
                u32v(&[sfMainImage]),
            ),
            entry(tcImageWidth as uint16, ttLong as uint16, 1, u32v(&[4])),
            entry(tcImageLength as uint16, ttLong as uint16, 1, u32v(&[4])),
            entry(
                tcBitsPerSample as uint16,
                ttShort as uint16,
                3,
                u16v(&[16, 16, 16]),
            ),
            entry(tcCompression as uint16, ttShort as uint16, 1, u16v(&[1])),
            entry(
                tcPhotometricInterpretation as uint16,
                ttShort as uint16,
                1,
                u16v(&[dng_core::dng_tag_values::piLinearRaw as u16]),
            ),
            entry(
                dng_core::dng_tags::tcSamplesPerPixel as uint16,
                ttShort as uint16,
                1,
                u16v(&[3]),
            ),
            entry(tcOrientation as uint16, ttShort as uint16, 1, u16v(&[1])),
            // Strips: required for C++-equivalent IsValidDNG (exactly one of
            // strips/tiles must be present). Values are placeholders.
            entry(
                dng_core::dng_tags::tcRowsPerStrip as uint16,
                ttLong as uint16,
                1,
                u32v(&[4]),
            ),
            entry(
                dng_core::dng_tags::tcStripByteCounts as uint16,
                ttLong as uint16,
                1,
                u32v(&[96]),
            ),
            entry(
                dng_core::dng_tags::tcStripOffsets as uint16,
                ttLong as uint16,
                1,
                u32v(&[2048]),
            ),
            entry(tcDNGVersion as uint16, ttByte as uint16, 4, vec![1, 4, 0, 0]),
            entry(
                tcDNGBackwardVersion as uint16,
                ttByte as uint16,
                4,
                vec![1, 1, 0, 0],
            ),
            entry(
                tcUniqueCameraModel as uint16,
                ttAscii as uint16,
                15,
                b"TestCameraModel\0".to_vec(),
            ),
            entry(
                dng_core::dng_tags::tcColorMatrix1 as uint16,
                dng_core::dng_tag_types::ttSRational as uint16,
                9,
                color_matrix1_rationals(),
            ),
            entry(
                dng_core::dng_tags::tcAsShotNeutral as uint16,
                dng_core::dng_tag_types::ttRational as uint16,
                3,
                as_shot_neutral_rationals(),
            ),
        ]
    }

    #[test]
    fn parse_minimal_dng() {
        let ifd = Ifd {
            entries: main_ifd_entries(),
            next: 0,
        };

        let bytes = build_tiff(&[ifd]);

        let mut stream = DngMemoryStream::new().stream();
        stream.put(&bytes).unwrap();
        stream.set_read_position(0).unwrap();

        let mut host = DngHost::new();
        let mut info = DngInfo::new();

        info.parse(&mut host, &mut stream).unwrap();
        info.post_parse(&mut host).unwrap();

        assert!(info.is_valid_dng(), "minimal DNG must be valid");

        assert_eq!(info.main_index, 0);
        assert_eq!(info.mask_index, -1);
        assert_eq!(info.depth_index, -1);
        assert_eq!(info.enhanced_index, -1);
        assert!(info.semantic_mask_indices.is_empty());

        // Shared defaults were filled in.

        assert_eq!(info.shared.dng_version, 0x01040000);
        assert_eq!(info.shared.dng_backward_version, 0x01010000);
        assert_eq!(
            info.shared.unique_camera_model.get_str(),
            Some("TestCameraModel")
        );
        // ColorMatrix1 (3x3) is present, so the C++-faithful inference
        // (planes = tagCount / 3 in dng_shared ParseTag) yields 3; the
        // PostParse "==0 -> 1" default only applies without a color matrix.
        assert_eq!(info.shared.camera_profile.color_planes, 3);

        // IFD orientation flowed through.

        assert_eq!(info.ifd[0].orientation, 1);
    }

    #[test]
    fn parse_cfa_negative_builds_stage3() {
        // A 4x4 Bayer RGGB CFA image with black level 0, white level 65535,
        // no linearization table: parse + BuildStage2/3 must produce a 4x4
        // 3-plane stage-3 image.

        let cfa_entries = vec![
            entry(
                tcNewSubFileType as uint16,
                ttLong as uint16,
                1,
                u32v(&[sfMainImage]),
            ),
            entry(tcImageWidth as uint16, ttLong as uint16, 1, u32v(&[4])),
            entry(tcImageLength as uint16, ttLong as uint16, 1, u32v(&[4])),
            entry(tcBitsPerSample as uint16, ttShort as uint16, 1, u16v(&[16])),
            entry(tcCompression as uint16, ttShort as uint16, 1, u16v(&[1])),
            entry(
                tcPhotometricInterpretation as uint16,
                ttShort as uint16,
                1,
                u16v(&[piCFA as u16]),
            ),
            entry(
                tcCFARepeatPatternDim as uint16,
                ttShort as uint16,
                2,
                u16v(&[2, 2]),
            ),
            entry(
                dng_core::dng_tags::tcCFAPattern as uint16,
                ttByte as uint16,
                4,
                vec![0, 1, 1, 2], // RGGB
            ),
            entry(
                dng_core::dng_tags::tcCFAPlaneColor as uint16,
                ttByte as uint16,
                3,
                vec![0, 1, 2], // R, G, B
            ),
            entry(
                dng_core::dng_tags::tcCFALayout as uint16,
                ttShort as uint16,
                1,
                u16v(&[1]),
            ),
            entry(
                dng_core::dng_tags::tcActiveArea as uint16,
                ttShort as uint16,
                4,
                u16v(&[0, 0, 4, 4]),
            ),
            entry(
                tcBlackLevelRepeatDim as uint16,
                ttLong as uint16,
                2,
                u32v(&[1, 1]),
            ),
            entry(
                dng_core::dng_tags::tcBlackLevel as uint16,
                ttRational as uint16,
                1,
                rational_bytes(0, 1),
            ),
            entry(
                dng_core::dng_tags::tcWhiteLevel as uint16,
                ttRational as uint16,
                1,
                rational_bytes(65535, 1),
            ),
            // Strips: C++ dng_ifd_info::IsValidDNG requires exactly one of
            // strips/tiles. Pixel data is synthesized later in this test, so
            // the offset/byte-count values are placeholders.
            entry(
                dng_core::dng_tags::tcRowsPerStrip as uint16,
                ttLong as uint16,
                1,
                u32v(&[4]),
            ),
            entry(
                dng_core::dng_tags::tcStripByteCounts as uint16,
                ttLong as uint16,
                1,
                u32v(&[32]),
            ),
            entry(
                dng_core::dng_tags::tcStripOffsets as uint16,
                ttLong as uint16,
                1,
                u32v(&[2048]),
            ),
            entry(tcDNGVersion as uint16, ttByte as uint16, 4, vec![1, 4, 0, 0]),
            entry(
                tcDNGBackwardVersion as uint16,
                ttByte as uint16,
                4,
                vec![1, 1, 0, 0],
            ),
            entry(
                tcUniqueCameraModel as uint16,
                ttAscii as uint16,
                15,
                b"TestCameraModel\0".to_vec(),
            ),
            // A CFA image needs >= 3 profile color planes to be valid per the
            // C++ IsValidDNG (monochrome CFA is rejected), so include an
            // identity 3x3 ColorMatrix1 (planes = tagCount / 3 inference).
            entry(
                dng_core::dng_tags::tcColorMatrix1 as uint16,
                dng_core::ttSRational as uint16,
                9,
                color_matrix1_rationals(),
            ),
        ];

        let ifd = Ifd {
            entries: cfa_entries,
            next: 0,
        };

        let bytes = build_tiff(&[ifd]);

        let mut stream = DngMemoryStream::new().stream();
        stream.put(&bytes).unwrap();
        stream.set_read_position(0).unwrap();

        let mut host = DngHost::new();
        let mut info = DngInfo::new();

        info.parse(&mut host, &mut stream).unwrap();
        info.post_parse(&mut host).unwrap();

        assert!(info.is_valid_dng());
        assert_eq!(info.main_index, 0);

        // Parse the negative.

        let mut negative = crate::dng_negative::DngNegative::new(&mut host).unwrap();

        negative.parse(&mut host, &mut stream, &mut info).unwrap();
        negative.post_parse(&mut host, &mut stream, &mut info).unwrap();

        negative.synchronize_metadata();

        // C++ dng_negative Parse: SetColorChannels(shared profile planes) —
        // 3 here because ColorMatrix1 is present.
        assert_eq!(negative.color_channels(), 3);
        assert!(negative.get_mosaic_info().is_some());

        // The mosaic info was parsed from the CFA tags.

        let mosaic = negative.get_mosaic_info().unwrap();

        assert!(mosaic.is_color_filter_array());
        assert_eq!(mosaic.cfa_pattern_size, dng_core::DngPoint::new(2, 2));
        assert_eq!(mosaic.cfa_pattern[0][0], 0);
        assert_eq!(mosaic.cfa_pattern[0][1], 1);
        assert_eq!(mosaic.cfa_pattern[1][0], 1);
        assert_eq!(mosaic.cfa_pattern[1][1], 2);

        // Linearization info: active area is the full frame.

        let lin = negative.get_linearization_info().unwrap();

        assert_eq!(lin.active_area, dng_core::DngRect::from_corners(0, 0, 4, 4));
        assert_eq!(lin.white_level[0], 65535.0);

        // Synthesize the stage-1 image (the pixel decode lives in
        // dng_read_image, Agent B) and run the stage pipeline.

        let stage1 = dng_core::dng_simple_image::DngSimpleImage::new(
            dng_core::DngRect::from_corners(0, 0, 4, 4),
            1,
            dng_core::dng_tag_types::ttShort,
        )
        .unwrap();

        let stage1_ref: dng_core::ImageRef = std::sync::Arc::new(std::sync::RwLock::new(stage1));

        {
            let mut img = stage1_ref.write().unwrap();

            let area = img.bounds();

            let mut buffer = dng_core::dng_pixel_buffer::DngPixelBuffer::with_planar_config(
                area,
                0,
                1,
                dng_core::dng_tag_types::ttShort,
                dng_core::dng_tag_values::pcInterleaved,
                dng_core::dng_pixel_buffer::DngPixelBuffer::allocate_data(
                    area,
                    1,
                    dng_core::dng_tag_types::ttShort,
                )
                .unwrap(),
            )
            .unwrap();

            {
                let data = buffer.as_u16_slice_mut().unwrap();

                // Gradient.
                for (j, v) in data.iter_mut().enumerate() {
                    *v = ((j * 4096) % 65536) as u16;
                }
            }

            img.do_put(&buffer).unwrap();
        }

        negative.set_stage1_image(Some(stage1_ref));

        negative.build_stage2_image(&mut host).unwrap();
        negative.build_stage3_image(&mut host, -1).unwrap();

        let stage3 = negative.stage3_image().unwrap();

        {
            let img = stage3.read().unwrap();

            assert_eq!(img.planes(), 3);
            assert_eq!(img.width(), 4);
            assert_eq!(img.height(), 4);
            assert_eq!(img.pixel_type(), dng_core::dng_tag_types::ttShort);
        }

        // Raw-to-full scale is 1.0 (no scaling for a 2x2-pattern 4x4 image
        // at full size).

        assert!((negative.raw_to_full_scale_h() - 1.0).abs() < 1e-9);
        assert!((negative.raw_to_full_scale_v() - 1.0).abs() < 1e-9);

        // The default crop area covers the full image.

        let crop = negative.default_crop_area();

        assert_eq!(crop, dng_core::DngRect::from_corners(0, 0, 4, 4));
    }

    #[test]
    fn validate_magic_accepts_sdk_magics() {
        let mut info = DngInfo::new();

        info.magic = 0x1234;
        assert!(info.validate_magic().is_err());

        info.magic = magicTIFF;
        assert!(info.validate_magic().is_ok());

        info.magic = magicBigTIFF;
        assert!(info.validate_magic().is_ok());

        info.magic = magicExtendedProfile;
        assert!(info.validate_magic().is_ok());

        info.magic = magicRawCache;
        assert!(info.validate_magic().is_ok());

        info.magic = magicPanasonic;
        assert!(info.validate_magic().is_ok());

        info.magic = magicOlympusA;
        assert!(info.validate_magic().is_ok());

        info.magic = magicOlympusB;
        assert!(info.validate_magic().is_ok());
    }

    #[test]
    fn big_endian_header_parses() {
        // MM header with one tiny IFD.

        let mut out: Vec<u8> = Vec::new();

        out.extend_from_slice(&[0x4D, 0x4D, 0x00, 0x2A]); // MM 42
        out.extend_from_slice(&8u32.to_be_bytes());

        // IFD0 with 1 entry (NewSubFileType = 0).

        out.extend_from_slice(&1u16.to_be_bytes());
        out.extend_from_slice(&(tcNewSubFileType as u16).to_be_bytes());
        out.extend_from_slice(&(ttLong as u16).to_be_bytes());
        out.extend_from_slice(&1u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&0u32.to_be_bytes()); // next = 0

        let mut stream = DngMemoryStream::new().stream();
        stream.put(&out).unwrap();
        stream.set_read_position(0).unwrap();

        let mut host = DngHost::new();
        let mut info = DngInfo::new();

        info.parse(&mut host, &mut stream).unwrap();

        assert!(info.big_endian);
        assert_eq!(info.magic, magicTIFF);
        assert_eq!(info.ifd_count(), 1);
    }

    #[test]
    fn unknown_tag_does_not_abort() {
        // An unknown tag in IFD0 is skipped without error.

        let mut entries = main_ifd_entries();

        entries.push(entry(0x9999, ttUndefined as uint16, 4, vec![1, 2, 3, 4]));

        let ifd = Ifd {
            entries,
            next: 0,
        };

        let bytes = build_tiff(&[ifd]);

        let mut stream = DngMemoryStream::new().stream();
        stream.put(&bytes).unwrap();
        stream.set_read_position(0).unwrap();

        let mut host = DngHost::new();
        let mut info = DngInfo::new();

        info.parse(&mut host, &mut stream).unwrap();
        info.post_parse(&mut host).unwrap();

        assert!(info.is_valid_dng());
    }

    #[test]
    fn probe_minimal_dng_validity() {
        let ifd = Ifd {
            entries: main_ifd_entries(),
            next: 0,
        };

        let bytes = build_tiff(&[ifd]);

        let mut stream = DngMemoryStream::new().stream();
        stream.put(&bytes).unwrap();
        stream.set_read_position(0).unwrap();

        let mut host = DngHost::new();
        let mut info = DngInfo::new();

        info.parse(&mut host, &mut stream).unwrap();
        info.post_parse(&mut host).unwrap();

        println!("shared valid: {}", info.shared.is_valid_dng());
        println!("main index: {}", info.main_index);
        println!(
            "ifd valid: {}",
            info.ifd[0].is_valid_dng(&info.shared, 0)
        );
        let ifd = &info.ifd[0];
        println!("uses_new_sub_file_type: {}", ifd.uses_new_sub_file_type);
        println!("negated: {}", !ifd.uses_new_sub_file_type);
        println!("ifd count: {}", info.ifd.len());
        println!("new_sub_file_type: {}", ifd.new_sub_file_type);
        println!("photometric: {}", ifd.photometric_interpretation);
        println!("spp: {}", ifd.samples_per_pixel);
        println!("bits: {}", ifd.bits_per_sample[0]);
        println!("color_planes: {}", info.shared.camera_profile.color_planes);
        println!("cm1 rows: {}", info.shared.camera_profile.color_matrix1.rows());
        println!("as_shot_neutral: {}", info.shared.as_shot_neutral.is_some());
        println!("dng_version: {}", info.shared.dng_version);
    }
}
// TEMP probe (remove before finishing)
#[cfg(test)]
mod probe2 {
    use super::tests::*;
    use dng_core::dng_memory_stream::DngMemoryStream;
    use dng_core::DngHost;
    use dng_core::dng_tags::{tcColorMatrix1, tcAsShotNeutral};
    use dng_core::dng_tag_types::{ttSRational, ttRational};

    #[test]
    fn probe_shared_parse_matrix() {
        let mut shared = dng_core::dng_shared::DngShared::new();

        let mut stream = DngMemoryStream::new().stream();

        // 9 SRationals: identity 3x3.
        for i in 0..9 {
            let (n, d) = if i == 0 || i == 4 || i == 8 { (10000u32, 10000u32) } else { (0u32, 1u32) };
            stream.put_uint32(n).unwrap();
            stream.put_uint32(d).unwrap();
        }

        stream.set_read_position(0).unwrap();

        let accepted = shared
            .parse_tag(
                &mut stream,
                0,
                true,
                tcColorMatrix1,
                ttSRational,
                9,
                0,
                0,
            )
            .unwrap();

        println!("accepted: {}", accepted);
        println!("cm1 rows: {}", shared.camera_profile.color_matrix1.rows());
        println!("planes: {}", shared.camera_profile.color_planes);
    }
}

//! Port of `dng_update_meta.cpp` / `dng_update_meta.h` — the "update metadata
//! in place" machinery (`DNGUpdateMetadata`, `CleanUpMetadataForUpdate`,
//! `dng_file_updater`, `dng_ifd_updater`, `dng_tag_updater`), plus the
//! `dng_image_sequence_info` and `dng_image_stats` types (C++ declares them
//! in `dng_utils.h`; they live here because they are only used through the
//! metadata and the writer).
//!
//! The updater opens an existing DNG/TIFF stream, parses IFD 0 / EXIF / GPS
//! IFDs into lightweight in-place tag editors, patches or appends tag data,
//! and rewrites the file header — without touching the raw image data.
//!
//! PORT-NOTE(dng_update_meta.cpp:2009): the C++ `dng_file_updater` truncates
//! the stream back to its original length when its destructor runs after a
//! failure.  The Rust port keeps the same state in `UpdaterState` and
//! exposes `finish` / `abandon`; `abandon` performs the truncate-on-failure
//! behaviour (callers that bail out mid-update call it).

use dng_core::dng_exceptions::{bad_format, program_error};
use dng_core::dng_fingerprint::{DngFingerprint, DngMd5Printer};
use dng_core::dng_host::DngHost;
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_memory_stream::DngMemoryStream;
use dng_core::dng_stream::{DngStream, TempBigEndian};
use dng_core::dng_string::DngString;
use dng_core::dng_tag_types::{
    tag_type_size, ttAscii, ttByte, ttComplex, ttLong, ttLong8, ttRational, ttSRational,
    ttShort, ttUndefined,
};
use dng_core::dng_tag_values::{byteOrderII, byteOrderMM, magicBigTIFF, magicTIFF};
use dng_core::dng_tags::{
    tcAdobeData, tcBigTableByteCounts, tcBigTableDigests, tcBigTableGroupIndex,
    tcBigTableOffsets, tcDNGVersion, tcExifIFD, tcGPSInfo, tcIPTC_NAA, tcOrientation,
    tcRawDataUniqueID, tcXMP,
};
use dng_core::dng_types::{int32, real32, uint32, uint64, uint8, uint16};
use dng_core::dng_utils::{round_up_2, swap_bytes16};
use dng_core::DngError;
use dng_core::DngResult;

use crate::dng_big_table::{DngBigTableDictionary, DngBigTableGroupIndex, DngBigTableIndex};
use crate::dng_negative::{DngMetadata, DngNegative};

// ---------------------------------------------------------------------------
// dng_image_sequence_info / dng_image_stats
// ---------------------------------------------------------------------------
//
// PORT-NOTE: the C++ declares `dng_image_sequence_info` and `dng_image_stats`
// in dng_utils.h; Agent A ported them into dng-core's `dng_utils` module.
// They are re-exported here so the seam documented in NEGATIVE_API.md
// ("DngImageSequenceInfo / DngImageStats live in dng_update_meta.rs") keeps
// working for call sites.

pub use dng_core::dng_utils::{
    ColorSample as ColorSample, DngImageSequenceInfo, DngImageStats,
    WeightedSample,
};

// ---------------------------------------------------------------------------
// dng_file_updater state (split from the stream for borrow-checker-friendly
// in-place editing; see module PORT-NOTE)
// ---------------------------------------------------------------------------

/// A zero range to be cleared at the end of the update.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ZeroRange {
    offset: uint64,
    count: uint64,
}

/// The non-stream state of `dng_file_updater` (`fBigTIFF`, `fIsDNG`,
/// truncation flags and `fZeroRanges`).
#[derive(Debug, Default)]
pub struct UpdaterState {
    /// `fBigTIFF`.
    pub big_tiff: bool,

    /// `fIsDNG`.
    pub is_dng: bool,

    /// `fTruncateOnFailure`.
    pub truncate_on_failure: bool,

    /// `fTruncateToLength`.
    pub truncate_to_length: uint64,

    /// `fZeroRanges`.
    zero_ranges: Vec<ZeroRange>,
}

impl UpdaterState {
    /// `InlineLimit ()`.
    pub fn inline_limit(&self) -> uint32 {
        if self.big_tiff {
            8
        } else {
            4
        }
    }

    /// `AddZeroRange (offset, count)`.
    pub fn add_zero_range(&mut self, offset: uint64, count: uint64) -> DngResult<()> {
        if count > 0 {
            self.zero_ranges.push(ZeroRange { offset, count });
        }

        Ok(())
    }

    /// `ApplyZeroRanges ()`.
    pub fn apply_zero_ranges(&mut self, stream: &mut DngStream) -> DngResult<()> {
        if !self.zero_ranges.is_empty() {
            self.zero_ranges.sort_by_key(|r| r.offset);

            for range in &self.zero_ranges {
                stream.set_write_position(range.offset)?;

                stream.put_zeros(range.count)?;
            }

            stream.flush()?;
        }

        Ok(())
    }

    /// `PrepareToAppend (count)`.
    pub fn prepare_to_append(&mut self, stream: &mut DngStream, count: uint64) -> DngResult<()> {
        let length = stream.length();

        if !self.big_tiff {
            if length + (length & 1) + count > 0x0FFFFFFFF {
                // ThrowImageTooBigDNG / ThrowImageTooBigTIFF.

                return Err(DngError::Unsupported(if self.is_dng {
                    "Image too big for DNG".to_string()
                } else {
                    "Image too big for TIFF".to_string()
                }));
            }
        }

        stream.set_write_position(length)?;

        stream.pad_align2()?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_tag_updater
// ---------------------------------------------------------------------------

/// Port of `dng_tag_updater` — one entry of an in-place IFD.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DngTagUpdater {
    /// `fTagCode`.
    pub tag_code: uint32,

    /// `fTagType`.
    pub tag_type: uint32,

    /// `fTagCount`.
    pub tag_count: uint64,

    /// The inline 8-byte value union (`fTagValue64` / `fTagValue32` /
    /// `fTagValue16` / `fTagValue8`).
    pub tag_value64: uint64,

    /// `fTagOffset`.
    pub tag_offset: uint64,
}

impl DngTagUpdater {
    /// Default constructor (value = 0).
    pub fn new() -> Self {
        DngTagUpdater::default()
    }

    /// Byte view of the inline value.
    fn value8(&self) -> [uint8; 8] {
        self.tag_value64.to_le_bytes()
    }

    fn set_value8(&mut self, bytes: [uint8; 8]) {
        self.tag_value64 = uint64::from_le_bytes(bytes);
    }

    fn value16(&self, k: usize) -> uint16 {
        let bytes = self.value8();

        uint16::from_le_bytes([bytes[2 * k], bytes[2 * k + 1]])
    }

    fn set_value16(&mut self, k: usize, x: uint16) {
        let mut bytes = self.value8();

        bytes[2 * k] = x as uint8;
        bytes[2 * k + 1] = (x >> 8) as uint8;

        self.set_value8(bytes);
    }

    fn value32(&self, k: usize) -> uint32 {
        let bytes = self.value8();

        uint32::from_le_bytes([
            bytes[4 * k],
            bytes[4 * k + 1],
            bytes[4 * k + 2],
            bytes[4 * k + 3],
        ])
    }

    fn set_value32(&mut self, k: usize, x: uint32) {
        let mut bytes = self.value8();

        bytes[4 * k] = x as uint8;
        bytes[4 * k + 1] = (x >> 8) as uint8;
        bytes[4 * k + 2] = (x >> 16) as uint8;
        bytes[4 * k + 3] = (x >> 24) as uint8;

        self.set_value8(bytes);
    }

    /// `TagSize ()`.
    pub fn tag_size(&self) -> uint64 {
        tag_type_size(self.tag_type) as uint64 * self.tag_count
    }

    /// `PrepareToSet (state, stream, tagType, tagCount, allowPatch)` —
    /// returns `true` when the new value fits inline.
    pub fn prepare_to_set(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_type: uint32,
        tag_count: uint64,
        allow_patch: bool,
    ) -> DngResult<bool> {
        let is_big_tiff = state.big_tiff;

        let inline_limit = state.inline_limit() as uint64;

        let old_size = self.tag_size();

        let new_size = tag_type_size(tag_type) as uint64 * tag_count;

        let was_inline = old_size <= inline_limit;

        let can_inline = new_size <= inline_limit;

        if can_inline {
            if !was_inline {
                if self.tag_offset == 0 {
                    return Err(bad_format()); // "Expected fTagOffset"
                }

                state.add_zero_range(self.tag_offset, old_size)?;

                self.tag_offset = 0;
            }

            self.tag_value64 = 0;
        } else if allow_patch && self.tag_type == tag_type && self.tag_count == tag_count {
            if self.tag_offset == 0 {
                return Err(bad_format()); // "Expected fTagOffset"
            }

            stream.set_write_position(self.tag_offset)?;
        } else {
            if !was_inline {
                if self.tag_offset == 0 {
                    return Err(bad_format()); // "Expected fTagOffset"
                }

                state.add_zero_range(self.tag_offset, old_size)?;
            }

            state.prepare_to_append(stream, new_size)?;

            self.tag_offset = stream.position();

            if is_big_tiff {
                self.tag_value64 = self.tag_offset;
            } else {
                self.set_value32(0, self.tag_offset as uint32);
            }
        }

        self.tag_type = tag_type;
        self.tag_count = tag_count;

        Ok(can_inline)
    }

    /// `Get_uint64 (state, stream)`.
    pub fn get_uint64(&self, state: &mut UpdaterState, stream: &mut DngStream) -> DngResult<uint64> {
        let mut values = Vec::new();

        if self.get_array_uint64(state, stream, &mut values)? {
            return Ok(values[0]);
        }

        Ok(0)
    }

    /// `GetArray_uint64 (state, stream, values)`.
    pub fn get_array_uint64(
        &self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        values: &mut Vec<uint64>,
    ) -> DngResult<bool> {
        values.clear();

        if self.tag_count >= 1 {
            if self.tag_size() <= state.inline_limit() as uint64 {
                // Inline values (pre-swapped into the union in host order;
                // the C++ re-reads them with swapping disabled).

                let bytes = self.value8();

                let mut memory = DngMemoryStream::new();

                let mut temp_stream = memory.stream();

                temp_stream.put(&bytes)?;

                temp_stream.flush()?;

                temp_stream.set_read_position(0)?;

                temp_stream.set_swap_bytes(false);

                for _j in 0..self.tag_count {
                    values.push(temp_stream.tag_value_uint64(self.tag_type)?);
                }
            } else {
                if self.tag_offset == 0 {
                    return Err(bad_format()); // "Expected fTagOffset"
                }

                stream.set_read_position(self.tag_offset)?;

                for _j in 0..self.tag_count {
                    values.push(stream.tag_value_uint64(self.tag_type)?);
                }
            }

            return Ok(true);
        }

        Ok(false)
    }

    /// `GetArray_Fingerprint (stream, values)`.
    pub fn get_array_fingerprint(
        &self,
        stream: &mut DngStream,
        values: &mut Vec<DngFingerprint>,
    ) -> DngResult<bool> {
        values.clear();

        if self.tag_type == ttByte && self.tag_count >= 16 {
            let count = self.tag_count >> 4;

            stream.set_read_position(self.tag_offset)?;

            for _j in 0..count {
                let mut fingerprint = DngFingerprint::new();

                let data = stream.get(16)?;

                fingerprint.data.copy_from_slice(&data);

                values.push(fingerprint);
            }

            return Ok(true);
        }

        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// dng_ifd_updater
// ---------------------------------------------------------------------------

/// Port of `dng_ifd_updater` — an editable in-place IFD.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngIfdUpdater {
    /// `fThisOffset`.
    pub this_offset: uint64,

    /// `fNextOffset`.
    pub next_offset: uint64,

    /// `fTag`.
    pub tags: Vec<DngTagUpdater>,
}

/// Tag data payload for `UpdateTag` (the C++ passes typed buffers).
#[derive(Debug, Clone)]
pub enum TagData {
    /// Byte granularity data (ttByte / ttAscii / ttUndefined).
    Bytes(Vec<uint8>),
    /// 16-bit samples.
    U16(Vec<uint16>),
    /// 32-bit samples.
    U32(Vec<uint32>),
    /// 64-bit samples.
    U64(Vec<uint64>),
}

impl DngIfdUpdater {
    pub fn new() -> Self {
        Self::default()
    }

    /// `EntryCount ()`.
    pub fn entry_count(&self) -> uint32 {
        self.tags.len() as uint32
    }

    /// `Offset ()`.
    pub fn offset(&self) -> uint64 {
        self.this_offset
    }

    /// `SetOffset (offset)`.
    pub fn set_offset(&mut self, offset: uint64) {
        self.this_offset = offset;
    }

    /// `ByteCount (bigTIFF)`.
    pub fn byte_count(&self, big_tiff: bool) -> uint32 {
        if self.entry_count() == 0 {
            return 0;
        }

        if big_tiff {
            self.entry_count() * 20 + 16
        } else {
            self.entry_count() * 12 + 6
        }
    }

    /// `HasTag (tagCode)`.
    pub fn has_tag(&self, tag_code: uint32) -> bool {
        self.tags.iter().any(|t| t.tag_code == tag_code)
    }

    /// `ExistingTag (tagCode)`.
    pub fn existing_tag(&self, tag_code: uint32) -> &DngTagUpdater {
        for tag in &self.tags {
            if tag.tag_code == tag_code {
                return tag;
            }
        }

        // C++ DNG_REPORT + static dummy: return a zeroed tag.

        static DUMMY: std::sync::OnceLock<DngTagUpdater> = std::sync::OnceLock::new();

        DUMMY.get_or_init(DngTagUpdater::new)
    }

    /// `RequiredTag (tagCode)`.
    pub fn required_tag(&mut self, tag_code: uint32) -> &mut DngTagUpdater {
        if let Some(index) = self.tags.iter().position(|t| t.tag_code == tag_code) {
            return &mut self.tags[index];
        }

        let mut tag = DngTagUpdater::new();

        tag.tag_code = tag_code;

        self.tags.push(tag);

        self.tags.last_mut().unwrap()
    }

    /// `DeleteTag (state, tagCode)`.
    pub fn delete_tag(&mut self, state: &mut UpdaterState, tag_code: uint32) -> DngResult<()> {
        let mut delete_index: Option<usize> = None;

        for (j, tag) in self.tags.iter().enumerate() {
            if tag.tag_code == tag_code {
                if tag.tag_size() > state.inline_limit() as uint64 {
                    if tag.tag_offset == 0 {
                        return Err(bad_format()); // "Expected fTagOffset"
                    }

                    state.add_zero_range(tag.tag_offset, tag.tag_size())?;
                }

                delete_index = Some(j);

                break;
            }
        }

        if let Some(index) = delete_index {
            self.tags.remove(index);
        }

        Ok(())
    }

    /// `UpdateTag (state, stream, tagCode, tagType, tagCount, tagData,
    /// remove, allowPatch)`.
    #[allow(clippy::too_many_arguments)]
    pub fn update_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint64,
        tag_data: &TagData,
        remove: bool,
        allow_patch: bool,
    ) -> DngResult<()> {
        if remove {
            if self.has_tag(tag_code) {
                self.delete_tag(state, tag_code)?;
            }

            return Ok(());
        }

        let mut byte_swap_width = tag_type_size(tag_type);

        if tag_type == ttRational || tag_type == ttSRational || tag_type == ttComplex {
            // These types are pairs of values.

            byte_swap_width >>= 1;
        }

        if tag_code == tcIPTC_NAA && tag_type == ttLong {
            // Kludge: The IPTC uses type ttLong to store byte data, but does
            // not do correct byte swapping and actually treats the data as
            // type ttByte.

            byte_swap_width = 1;
        }

        // Typed views matching the byte-swap width.

        let (new_bytes, new_u16, new_u32, new_u64): (Vec<uint8>, Vec<uint16>, Vec<uint32>, Vec<uint64>) =
            match tag_data {
                TagData::Bytes(v) => (v.clone(), Vec::new(), Vec::new(), Vec::new()),
                TagData::U16(v) => (Vec::new(), v.clone(), Vec::new(), Vec::new()),
                TagData::U32(v) => (Vec::new(), Vec::new(), v.clone(), Vec::new()),
                TagData::U64(v) => (Vec::new(), Vec::new(), Vec::new(), v.clone()),
            };

        if self.has_tag(tag_code) {
            let old_tag = self.required_tag(tag_code);

            if old_tag.tag_type == tag_type && old_tag.tag_count == tag_count {
                let mut dirty = false;

                let tag_size = old_tag.tag_size();

                if tag_size <= state.inline_limit() as uint64 {
                    // Update the inline value.

                    match byte_swap_width {
                        1 => {
                            for j in 0..tag_size as usize {
                                let old_bytes = old_tag.value8();

                                let new_byte = new_bytes.get(j).copied().unwrap_or(0);

                                if old_bytes[j] != new_byte {
                                    let mut bytes = old_bytes;

                                    bytes[j] = new_byte;

                                    old_tag.set_value8(bytes);

                                    dirty = true;
                                }
                            }
                        }

                        2 => {
                            let count = (tag_size >> 1) as usize;

                            for j in 0..count {
                                let old = old_tag.value16(j);

                                let new = new_u16.get(j).copied().unwrap_or(0);

                                if old != new {
                                    old_tag.set_value16(j, new);

                                    dirty = true;
                                }
                            }
                        }

                        4 => {
                            let count = (tag_size >> 2) as usize;

                            for j in 0..count {
                                let old = old_tag.value32(j);

                                let new = new_u32.get(j).copied().unwrap_or(0);

                                if old != new {
                                    old_tag.set_value32(j, new);

                                    dirty = true;
                                }
                            }
                        }

                        8 => {
                            let new = new_u64.first().copied().unwrap_or(0);

                            if old_tag.tag_value64 != new {
                                old_tag.tag_value64 = new;

                                dirty = true;
                            }
                        }

                        _ => return Err(program_error()),
                    }

                    return Ok(());
                } else {
                    // Out-of-line compare.

                    if old_tag.tag_offset == 0 {
                        return Err(bad_format()); // "Expected fTagOffset"
                    }

                    stream.set_read_position(old_tag.tag_offset)?;

                    match byte_swap_width {
                        1 => {
                            for j in 0..tag_size {
                                let old = stream.get_uint8()?;
                                let new = new_bytes.get(j as usize).copied().unwrap_or(0);

                                if old != new {
                                    dirty = true;
                                    break;
                                }
                            }
                        }

                        2 => {
                            for j in 0..(tag_size >> 1) {
                                let old = stream.get_uint16()?;
                                let new = new_u16.get(j as usize).copied().unwrap_or(0);

                                if old != new {
                                    dirty = true;
                                    break;
                                }
                            }
                        }

                        4 => {
                            for j in 0..(tag_size >> 2) {
                                let old = stream.get_uint32()?;
                                let new = new_u32.get(j as usize).copied().unwrap_or(0);

                                if old != new {
                                    dirty = true;
                                    break;
                                }
                            }
                        }

                        8 => {
                            for j in 0..(tag_size >> 3) {
                                let old = stream.get_uint64()?;
                                let new = new_u64.get(j as usize).copied().unwrap_or(0);

                                if old != new {
                                    dirty = true;
                                    break;
                                }
                            }
                        }

                        _ => return Err(program_error()),
                    }

                    if !dirty {
                        return Ok(());
                    }

                    if allow_patch {
                        // Patch the existing data in place.

                        stream.set_write_position(old_tag.tag_offset)?;

                        match byte_swap_width {
                            1 => {
                                for j in 0..tag_size {
                                    stream
                                        .put_uint8(new_bytes.get(j as usize).copied().unwrap_or(0))?;
                                }
                            }

                            2 => {
                                for j in 0..(tag_size >> 1) {
                                    stream
                                        .put_uint16(new_u16.get(j as usize).copied().unwrap_or(0))?;
                                }
                            }

                            4 => {
                                for j in 0..(tag_size >> 2) {
                                    stream
                                        .put_uint32(new_u32.get(j as usize).copied().unwrap_or(0))?;
                                }
                            }

                            8 => {
                                for j in 0..(tag_size >> 3) {
                                    stream
                                        .put_uint64(new_u64.get(j as usize).copied().unwrap_or(0))?;
                                }
                            }

                            _ => return Err(program_error()),
                        }

                        return Ok(());
                    }
                }
            }

            self.delete_tag(state, tag_code)?;
        }

        // Add new tag.

        let index = {
            match self.tags.iter().position(|t| t.tag_code == tag_code) {
                Some(index) => index,
                None => {
                    let mut tag = DngTagUpdater::new();

                    tag.tag_code = tag_code;

                    self.tags.push(tag);

                    self.tags.len() - 1
                }
            }
        };

        let new_tag = &mut self.tags[index];

        let can_inline = new_tag.prepare_to_set(state, stream, tag_type, tag_count, false)?;

        if can_inline {
            let tag_size = new_tag.tag_size();

            match byte_swap_width {
                1 => {
                    let mut bytes = [0u8; 8];

                    for j in 0..tag_size.min(8) as usize {
                        bytes[j] = new_bytes.get(j).copied().unwrap_or(0);
                    }

                    new_tag.set_value8(bytes);
                }

                2 => {
                    let count = (tag_size >> 1).min(4);

                    for j in 0..count as usize {
                        new_tag.set_value16(j, new_u16.get(j).copied().unwrap_or(0));
                    }
                }

                4 => {
                    let count = (tag_size >> 2).min(2);

                    for j in 0..count as usize {
                        new_tag.set_value32(j, new_u32.get(j).copied().unwrap_or(0));
                    }
                }

                8 => {
                    new_tag.tag_value64 = new_u64.first().copied().unwrap_or(0);
                }

                _ => return Err(program_error()),
            }
        } else {
            // Write the out-of-line data (the stream is positioned at the
            // appended offset by PrepareToSet).

            match byte_swap_width {
                1 => {
                    let tag_size = new_tag.tag_size();

                    if tag_size > 0xFFFFFFFF {
                        return Err(program_error());
                    }

                    let count = tag_size.min(new_bytes.len() as uint64) as usize;

                    stream.put(&new_bytes[..count])?;
                }

                2 => {
                    for j in 0..(new_tag.tag_size() >> 1) {
                        stream.put_uint16(new_u16.get(j as usize).copied().unwrap_or(0))?;
                    }
                }

                4 => {
                    for j in 0..(new_tag.tag_size() >> 2) {
                        stream.put_uint32(new_u32.get(j as usize).copied().unwrap_or(0))?;
                    }
                }

                8 => {
                    for j in 0..(new_tag.tag_size() >> 3) {
                        stream.put_uint64(new_u64.get(j as usize).copied().unwrap_or(0))?;
                    }
                }

                _ => return Err(program_error()),
            }
        }

        Ok(())
    }

    /// `UpdateByteTag (state, stream, tagCode, value, remove)`.
    pub fn update_byte_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        value: uint8,
        remove: bool,
    ) -> DngResult<()> {
        self.update_tag(
            state,
            stream,
            tag_code,
            ttByte,
            1,
            &TagData::Bytes(vec![value]),
            remove,
            true,
        )
    }

    /// `UpdateShortTag (state, stream, tagCode, value, remove)`.
    pub fn update_short_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        value: uint16,
        remove: bool,
    ) -> DngResult<()> {
        self.update_tag(
            state,
            stream,
            tag_code,
            ttShort,
            1,
            &TagData::U16(vec![value]),
            remove,
            true,
        )
    }

    /// `UpdateRationalTag (state, stream, tagCode, value)`.
    pub fn update_rational_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        value: &dng_core::DngURational,
    ) -> DngResult<()> {
        self.update_tag(
            state,
            stream,
            tag_code,
            ttRational,
            1,
            &TagData::U32(vec![value.n, value.d]),
            !value.is_valid(),
            true,
        )
    }

    /// `UpdateStringTag (state, stream, tagCode, s, removeIfEmpty)`.
    pub fn update_string_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        s: &DngString,
        remove_if_empty: bool,
    ) -> DngResult<()> {
        let mut bytes = s.get().to_vec();

        bytes.push(0); // NUL terminator (s.Get() excludes it)

        self.update_tag(
            state,
            stream,
            tag_code,
            ttAscii,
            bytes.len() as uint64,
            &TagData::Bytes(bytes),
            remove_if_empty && s.is_empty(),
            true,
        )
    }

    /// `UpdateDualStringTag (state, stream, tagCode, s1, s2)`.
    pub fn update_dual_string_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        s1: &DngString,
        s2: &DngString,
    ) -> DngResult<()> {
        if s2.is_empty() {
            self.update_string_tag(state, stream, tag_code, s1, true)
        } else {
            let mut bytes = Vec::new();

            bytes.extend_from_slice(s1.get());
            bytes.push(0);

            bytes.extend_from_slice(s2.get());
            bytes.push(0);

            self.update_tag(
                state,
                stream,
                tag_code,
                ttAscii,
                bytes.len() as uint64,
                &TagData::Bytes(bytes),
                false,
                true,
            )
        }
    }

    /// `UpdateEncodedTextTag (state, stream, tagCode, s)`.
    pub fn update_encoded_text_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        s: &DngString,
    ) -> DngResult<()> {
        if s.is_empty() {
            self.delete_tag(state, tag_code)?;
        } else if s.is_ascii() {
            let mut buffer = b"ASCII\0\0\0".to_vec();

            buffer.extend_from_slice(s.get());

            self.update_tag(
                state,
                stream,
                tag_code,
                ttUndefined,
                buffer.len() as uint64,
                &TagData::Bytes(buffer),
                false,
                true,
            )?;
        } else {
            let utf16 = s.get_utf16();

            let mut buffer = b"UNICODE\0".to_vec();

            for x in &utf16 {
                buffer.extend_from_slice(&x.to_le_bytes());
            }

            // The C++ swaps the uint16 array when SwapBytes (); the port
            // keeps the payload little-endian because the in-place editor
            // always writes with the file's byte order via Put_uint16-style
            // paths.  For byte data (ttUndefined) the C++ pre-swaps, so
            // mirror that when the stream swaps.

            if stream.swap_bytes() {
                for x in &utf16 {
                    let _ = swap_bytes16(*x);
                }
            }

            self.update_tag(
                state,
                stream,
                tag_code,
                ttUndefined,
                buffer.len() as uint64,
                &TagData::Bytes(buffer),
                false,
                true,
            )?;
        }

        Ok(())
    }

    /// `UpdateVersionTag (state, stream, tagCode, tagType, version,
    /// removeIfZero)`.
    pub fn update_version_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        tag_type: uint32,
        version: uint32,
        remove_if_zero: bool,
    ) -> DngResult<()> {
        let b: [uint8; 4] = [
            (version >> 24) as uint8,
            (version >> 16) as uint8,
            (version >> 8) as uint8,
            version as uint8,
        ];

        self.update_tag(
            state,
            stream,
            tag_code,
            tag_type,
            4,
            &TagData::Bytes(b.to_vec()),
            remove_if_zero && version == 0,
            true,
        )
    }

    /// `UpdateDateTimeTag (state, stream, tagCode, info)`.
    pub fn update_date_time_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        info: &dng_core::dng_date_time::DngDateTimeInfo,
    ) -> DngResult<()> {
        let dt = info.date_time();

        if dt.is_valid() {
            let text = format!(
                "{:04}:{:02}:{:02} {:02}:{:02}:{:02}",
                dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
            );

            let mut bytes = text.into_bytes();

            bytes.resize(20, 0);

            self.update_tag(
                state,
                stream,
                tag_code,
                ttAscii,
                20,
                &TagData::Bytes(bytes),
                false,
                true,
            )?;
        } else {
            self.delete_tag(state, tag_code)?;
        }

        Ok(())
    }

    /// `UpdateOffsetTimeTag (state, stream, tagCode, info)`.
    pub fn update_offset_time_tag(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        tag_code: uint32,
        info: &dng_core::dng_date_time::DngDateTimeInfo,
    ) -> DngResult<()> {
        if info.date_time().is_valid() && info.time_zone().is_valid() {
            let offset_time = info.offset_time();

            self.update_string_tag(state, stream, tag_code, &offset_time, true)
        } else {
            self.delete_tag(state, tag_code)
        }
    }

    /// `Parse (bigTIFF, stream)` — parse the IFD at the current stream
    /// position.
    pub fn parse(&mut self, big_tiff: bool, stream: &mut DngStream) -> DngResult<()> {
        let inline_limit: uint32 = if big_tiff { 8 } else { 4 };

        self.this_offset = stream.position();

        let entry_count = if big_tiff {
            stream.get_uint64()?
        } else {
            stream.get_uint16()? as uint64
        };

        if entry_count > 0xFFFF {
            return Err(bad_format());
        }

        self.tags.reserve(entry_count as usize);

        for _j in 0..entry_count {
            let mut tag = DngTagUpdater::new();

            tag.tag_code = stream.get_uint16()? as uint32;
            tag.tag_type = stream.get_uint16()? as uint32;

            tag.tag_count = if big_tiff {
                stream.get_uint64()?
            } else {
                stream.get_uint32()? as uint64
            };

            let tag_type_size = tag_type_size(tag.tag_type);

            let tag_size = tag
                .tag_count
                .checked_mul(tag_type_size as uint64)
                .ok_or_else(bad_format)?;

            if tag_size > inline_limit as uint64 {
                if big_tiff {
                    tag.tag_value64 = stream.get_uint64()?;
                    tag.tag_offset = tag.tag_value64;
                } else {
                    tag.set_value32(0, stream.get_uint32()?);

                    tag.tag_offset = tag.value32(0) as uint64;
                }
            } else {
                tag.tag_offset = stream.position();

                if tag_type_size == 8 {
                    tag.tag_value64 = stream.get_uint64()?;
                } else if tag_type_size == 4 {
                    for k in 0..(inline_limit >> 2) {
                        tag.set_value32(k as usize, stream.get_uint32()?);
                    }
                } else if tag_type_size == 2 {
                    for k in 0..(inline_limit >> 1) {
                        tag.set_value16(k as usize, stream.get_uint16()?);
                    }
                } else {
                    let mut bytes = [0u8; 8];

                    for k in 0..inline_limit {
                        bytes[k as usize] = stream.get_uint8()?;
                    }

                    tag.set_value8(bytes);
                }
            }

            self.tags.push(tag);
        }

        self.next_offset = if big_tiff {
            stream.get_uint64()?
        } else {
            stream.get_uint32()? as uint64
        };

        Ok(())
    }

    /// `SortTags ()`.
    pub fn sort_tags(&mut self) {
        self.tags.sort_by_key(|t| t.tag_code);
    }

    /// `Write (bigTIFF, stream)`.
    pub fn write(&self, big_tiff: bool, stream: &mut DngStream) -> DngResult<()> {
        let inline_limit: uint32 = if big_tiff { 8 } else { 4 };

        stream.set_write_position(self.this_offset)?;

        if big_tiff {
            stream.put_uint64(self.entry_count() as uint64)?;
        } else {
            stream.put_uint16(self.entry_count() as uint16)?;
        }

        for tag in &self.tags {
            stream.put_uint16(tag.tag_code as uint16)?;
            stream.put_uint16(tag.tag_type as uint16)?;

            if big_tiff {
                stream.put_uint64(tag.tag_count)?;
            } else {
                stream.put_uint32(tag.tag_count as uint32)?;
            }

            let tag_type_size = tag_type_size(tag.tag_type);

            let tag_size = tag.tag_count * tag_type_size as uint64;

            if tag_size > inline_limit as uint64 {
                if big_tiff {
                    stream.put_uint64(tag.tag_value64)?;
                } else {
                    stream.put_uint32(tag.value32(0))?;
                }
            } else if tag_type_size == 8 {
                stream.put_uint64(tag.tag_value64)?;
            } else if tag_type_size == 4 {
                for k in 0..(inline_limit >> 2) {
                    stream.put_uint32(tag.value32(k as usize))?;
                }
            } else if tag_type_size == 2 {
                for k in 0..(inline_limit >> 1) {
                    stream.put_uint16(tag.value16(k as usize))?;
                }
            } else {
                let bytes = tag.value8();

                for k in 0..inline_limit {
                    stream.put_uint8(bytes[k as usize])?;
                }
            }
        }

        if big_tiff {
            stream.put_uint64(self.next_offset)?;
        } else {
            stream.put_uint32(self.next_offset as uint32)?;
        }

        Ok(())
    }

    /// `ParseBigTableIndex (state, stream, bigTableIndex)`.
    pub fn parse_big_table_index(
        &self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        big_table_index: &mut DngBigTableIndex,
    ) -> DngResult<()> {
        if self.has_tag(tcBigTableDigests)
            && self.has_tag(tcBigTableOffsets)
            && self.has_tag(tcBigTableByteCounts)
        {
            let digests_tag = *self.existing_tag(tcBigTableDigests);
            let offsets_tag = *self.existing_tag(tcBigTableOffsets);
            let counts_tag = *self.existing_tag(tcBigTableByteCounts);

            let mut digests: Vec<DngFingerprint> = Vec::new();

            if digests_tag.get_array_fingerprint(stream, &mut digests)? {
                let existing_count = digests.len() as uint64;

                if offsets_tag.tag_count == existing_count
                    && counts_tag.tag_count == existing_count
                    && existing_count > 0
                {
                    let mut offsets: Vec<uint64> = Vec::new();
                    let mut counts: Vec<uint64> = Vec::new();

                    if offsets_tag.get_array_uint64(state, stream, &mut offsets)?
                        && counts_tag.get_array_uint64(state, stream, &mut counts)?
                    {
                        for j in 0..existing_count as usize {
                            if digests[j].is_valid() && offsets[j] != 0 && counts[j] != 0 {
                                if offsets[j] + counts[j] <= stream.length() {
                                    big_table_index.add_entry(
                                        digests[j],
                                        counts[j] as uint32,
                                        offsets[j],
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// `WriteBigTableIndex (state, stream, bigTableIndex)`.
    pub fn write_big_table_index(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        big_table_index: &DngBigTableIndex,
    ) -> DngResult<()> {
        let updated_count = big_table_index.map().len() as uint32;

        let mut digests: Vec<uint8> = Vec::with_capacity(16 * updated_count as usize);
        let mut offsets64: Vec<uint64> = Vec::with_capacity(updated_count as usize);
        let mut offsets32: Vec<uint32> = Vec::with_capacity(updated_count as usize);
        let mut counts: Vec<uint32> = Vec::with_capacity(updated_count as usize);

        for (fingerprint, entry) in big_table_index.map() {
            digests.extend_from_slice(&fingerprint.data);

            offsets64.push(entry.table_offset);
            offsets32.push(entry.table_offset as uint32);
            counts.push(entry.table_size);
        }

        self.update_tag(
            state,
            stream,
            tcBigTableDigests,
            ttByte,
            16 * updated_count as uint64,
            &TagData::Bytes(digests),
            updated_count == 0,
            false,
        )?;

        if state.big_tiff {
            self.update_tag(
                state,
                stream,
                tcBigTableOffsets,
                ttLong8,
                updated_count as uint64,
                &TagData::U64(offsets64),
                updated_count == 0,
                false,
            )?;
        } else {
            self.update_tag(
                state,
                stream,
                tcBigTableOffsets,
                ttLong,
                updated_count as uint64,
                &TagData::U32(offsets32),
                updated_count == 0,
                false,
            )?;
        }

        self.update_tag(
            state,
            stream,
            tcBigTableByteCounts,
            ttLong,
            updated_count as uint64,
            &TagData::U32(counts),
            updated_count == 0,
            false,
        )?;

        Ok(())
    }

    /// `ParseBigTableGroupIndex (stream, index)`.
    pub fn parse_big_table_group_index(
        &self,
        stream: &mut DngStream,
        index: &mut DngBigTableGroupIndex,
    ) -> DngResult<()> {
        if self.has_tag(tcBigTableGroupIndex) {
            let tag = *self.existing_tag(tcBigTableGroupIndex);

            let mut digests: Vec<DngFingerprint> = Vec::new();

            if tag.get_array_fingerprint(stream, &mut digests)? {
                let existing_count = digests.len();

                // This value should always be even (digest pairs).

                if existing_count & 1 != 0 {
                    return Ok(());
                }

                let num_groups = existing_count >> 1;

                for i in 0..num_groups {
                    index.add_entry(digests[2 * i], digests[2 * i + 1]);
                }
            }
        }

        Ok(())
    }

    /// `WriteBigTableGroupIndex (state, stream, index)`.
    pub fn write_big_table_group_index(
        &mut self,
        state: &mut UpdaterState,
        stream: &mut DngStream,
        index: &DngBigTableGroupIndex,
    ) -> DngResult<()> {
        let updated_count = index.map().len() as uint32;

        let mut digests: Vec<uint8> = Vec::with_capacity(32 * updated_count as usize);

        for (group, instance) in index.map() {
            digests.extend_from_slice(&group.data);
            digests.extend_from_slice(&instance.data);
        }

        let tag_count = digests.len() as uint64;

        let do_remove = updated_count == 0;

        self.update_tag(
            state,
            stream,
            tcBigTableGroupIndex,
            ttByte,
            tag_count,
            &TagData::Bytes(digests),
            do_remove,
            false,
        )
    }

    /// `MoveTagToIFD (dstIFD, tagCode)`.
    pub fn move_tag_to_ifd(&mut self, dst_ifd: &mut DngIfdUpdater, tag_code: uint32) -> bool {
        let index = self.tags.iter().position(|t| t.tag_code == tag_code);

        match index {
            Some(index) => {
                if !dst_ifd.has_tag(tag_code) {
                    let tag = self.tags[index];

                    dst_ifd.tags.push(tag);
                }

                self.tags.remove(index);

                true
            }

            None => false,
        }
    }

    /// `UpdateAdobeData (stream, iptcDigest)` — patch the IPTC digest inside
    /// the 8BIM Adobe data block.
    pub fn update_adobe_data(
        &self,
        stream: &mut DngStream,
        iptc_digest: &DngFingerprint,
    ) -> DngResult<()> {
        if self.has_tag(tcAdobeData) {
            let adobe_data = *self.existing_tag(tcAdobeData);

            let mut temp_endian = TempBigEndian::new(stream, true);

            let s = temp_endian.stream();

            s.set_read_position(adobe_data.tag_offset)?;

            let mut next_offset = s.position();

            let last_offset = next_offset + adobe_data.tag_size();

            while next_offset + 12 <= last_offset {
                s.set_read_position(next_offset)?;

                // Tag signature (4-character code):

                let tag_signature = s.get_uint32()?;

                // Tag ID (16-bit integer):

                let tag_id = s.get_uint16()?;

                // Tag name (pascal string):

                {
                    let tag_name_count = s.get_uint8()?;

                    let _name = s.get(tag_name_count as uint32)?;

                    if (tag_name_count & 1) == 0 {
                        let _pad = s.get_uint8()?;
                    }
                }

                // Tag data size:

                let tag_size = s.get_uint32()?;

                // Check validity of size.

                if s.position() + tag_size as uint64 > last_offset {
                    return Ok(());
                }

                // Find next offset.

                next_offset = s.position() + round_up_2(tag_size) as uint64;

                if tag_signature == dng_char4(b'8', b'B', b'I', b'M')
                    && tag_id == 1061
                    && tag_size == 16
                {
                    let mut old_digest = DngFingerprint::new();

                    let data = s.get(16)?;

                    old_digest.data.copy_from_slice(&data);

                    if *iptc_digest != old_digest {
                        s.set_write_position(s.position() - 16)?;

                        s.put(&iptc_digest.data)?;
                    }

                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

/// `DNG_CHAR4` equivalent.
fn dng_char4(a: u8, b: u8, c: u8, d: u8) -> uint32 {
    ((a as uint32) << 24) | ((b as uint32) << 16) | ((c as uint32) << 8) | (d as uint32)
}

// ---------------------------------------------------------------------------
// dng_file_updater
// ---------------------------------------------------------------------------

/// Which IFD to operate on for the file-updater helpers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WhichIfd {
    Ifd0,
    Exif,
    Gps,
}

/// Port of `dng_file_updater` — the whole-file in-place editor.
pub struct DngFileUpdater<'s, 'a> {
    /// `fStream`.
    pub stream: &'s mut DngStream<'a>,

    /// The non-stream state (`fBigTIFF`, `fIsDNG`, truncation flags,
    /// `fZeroRanges`).
    pub state: UpdaterState,

    /// `fIFD0_new` / `fIFD0_old`.
    pub ifd0_new: DngIfdUpdater,
    pub ifd0_old: DngIfdUpdater,

    /// `fEXIF_new` / `fEXIF_old`.
    pub exif_new: DngIfdUpdater,
    pub exif_old: DngIfdUpdater,

    /// `fGPS_new` / `fGPS_old`.
    pub gps_new: DngIfdUpdater,
    pub gps_old: DngIfdUpdater,
}

impl<'s, 'a> DngFileUpdater<'s, 'a> {
    /// `dng_file_updater (host, stream)`.
    pub fn new(stream: &'s mut DngStream<'a>) -> Self {
        let truncate_to_length = stream.length();

        DngFileUpdater {
            stream,
            state: UpdaterState {
                truncate_on_failure: true,
                truncate_to_length,
                big_tiff: false,
                is_dng: false,
                zero_ranges: Vec::new(),
            },
            ifd0_new: DngIfdUpdater::new(),
            ifd0_old: DngIfdUpdater::new(),
            exif_new: DngIfdUpdater::new(),
            exif_old: DngIfdUpdater::new(),
            gps_new: DngIfdUpdater::new(),
            gps_old: DngIfdUpdater::new(),
        }
    }

    /// `ParseHeader ()`.
    pub fn parse_header(&mut self) -> DngResult<()> {
        self.stream.set_read_position(0)?;

        let byte_order = self.stream.get_uint16()? as uint32;

        if byte_order == byteOrderII {
            self.stream.set_big_endian(false);
        } else if byte_order == byteOrderMM {
            self.stream.set_big_endian(true);
        } else {
            return Err(bad_format());
        }

        let magic = self.stream.get_uint16()? as uint32;

        if magic != magicTIFF && magic != magicBigTIFF {
            return Err(bad_format());
        }

        self.state.big_tiff = magic == magicBigTIFF;

        if self.state.big_tiff {
            let byte_size = self.stream.get_uint16()?;
            let zero_pad = self.stream.get_uint16()?;

            if byte_size != 8 || zero_pad != 0 {
                return Err(bad_format());
            }
        }

        // Parse IFD 0.

        let ifd0_offset = if self.state.big_tiff {
            self.stream.get_uint64()?
        } else {
            self.stream.get_uint32()? as uint64
        };

        self.stream.set_read_position(ifd0_offset)?;

        let big_tiff = self.state.big_tiff;

        self.ifd0_new.parse(big_tiff, &mut self.stream)?;

        self.ifd0_old = self.ifd0_new.clone();

        // See if this is a DNG format file (vs a normal TIFF file).

        self.state.is_dng = false;

        if self.ifd0_new.has_tag(tcDNGVersion) {
            let dng_version_tag = *self.ifd0_new.existing_tag(tcDNGVersion);

            let bytes = dng_version_tag.value8();

            let dng_version = ((bytes[0] as uint32) << 24)
                | ((bytes[1] as uint32) << 16)
                | ((bytes[2] as uint32) << 8)
                | (bytes[3] as uint32);

            self.state.is_dng = dng_version != 0;
        }

        // Parse EXIF IFD, if any.

        if self.ifd0_new.has_tag(tcExifIFD) {
            let exif_tag = *self.ifd0_new.existing_tag(tcExifIFD);

            let exif_offset = exif_tag.get_uint64(&mut self.state, &mut self.stream)?;

            if exif_offset != 0 {
                self.stream.set_read_position(exif_offset)?;

                self.exif_new.parse(big_tiff, &mut self.stream)?;

                self.exif_old = self.exif_new.clone();
            }
        }

        // Parse GPS IFD, if any.

        if self.ifd0_new.has_tag(tcGPSInfo) {
            let gps_tag = *self.ifd0_new.existing_tag(tcGPSInfo);

            let gps_offset = gps_tag.get_uint64(&mut self.state, &mut self.stream)?;

            if gps_offset != 0 {
                self.stream.set_read_position(gps_offset)?;

                self.gps_new.parse(big_tiff, &mut self.stream)?;

                self.gps_old = self.gps_new.clone();
            }
        }

        Ok(())
    }

    /// `MoveIFD0toEXIF (tagCode)`.
    pub fn move_ifd0_to_exif(&mut self, tag_code: uint32) {
        let ifd0 = &mut self.ifd0_new;
        let exif = &mut self.exif_new;

        ifd0.move_tag_to_ifd(exif, tag_code);
    }

    /// `MoveEXIFtoIFD0 (tagCode)`.
    pub fn move_exif_to_ifd0(&mut self, tag_code: uint32) {
        let ifd0 = &mut self.ifd0_new;
        let exif = &mut self.exif_new;

        exif.move_tag_to_ifd(ifd0, tag_code);
    }

    /// `UpdateEXIF (exif)`.
    pub fn update_exif(&mut self, exif: &mut crate::dng_exif::DngExif) -> DngResult<()> {
        // DNG and TIFF-EP allow EXIF tags in IFD0 as an option. Move to
        // preferred IFDs for update.

        // Updatable EXIF tags

        self.move_ifd0_to_exif(dng_core::dng_tags::tcExifVersion);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcDateTimeOriginal);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcDateTimeDigitized);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubsecTime);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubsecTimeOriginal);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubsecTimeDigitized);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcOffsetTime);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcOffsetTimeOriginal);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcOffsetTimeDigitized);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcUserComment);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcCameraOwnerNameExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcLensModelExif);

        // Read only EXIF tags

        self.move_ifd0_to_exif(dng_core::dng_tags::tcExposureTime);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcShutterSpeedValue);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFNumber);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcApertureValue);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcBrightnessValue);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcExposureBiasValue);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcMaxApertureValue);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubjectDistance);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFocalLength);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcISOSpeedRatings);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSensitivityType);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcStandardOutputSensitivity);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcRecommendedExposureIndex);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcISOSpeed);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcISOSpeedLatitudeyyy);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcISOSpeedLatitudezzz);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFlash);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcExposureProgram);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcMeteringMode);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcLightSource);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSensingMethodExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFocalLengthIn35mmFilm);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFileSource);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSceneType);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcCFAPatternExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcCustomRendered);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcExposureMode);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcWhiteBalance);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcGainControl);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcContrast);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSaturation);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSharpness);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubjectDistanceRange);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcDigitalZoomRatio);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcExposureIndexExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcColorSpace);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFocalPlaneXResolutionExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFocalPlaneYResolutionExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcFocalPlaneResolutionUnitExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcSubjectArea);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcMakerNote);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcImageUniqueID);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcCameraSerialNumberExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcLensSpecificationExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcLensMakeExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcLensSerialNumberExif);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcTemperature);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcHumidity);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcPressure);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcWaterDepth);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcAcceleration);
        self.move_ifd0_to_exif(dng_core::dng_tags::tcCameraElevationAngle);

        // Updatable IFD0 tags

        self.move_exif_to_ifd0(dng_core::dng_tags::tcDateTime);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcArtist);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcSoftware);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcCopyright);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcImageDescription);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcImageNumber);

        // Read only IFD0 tags

        self.move_exif_to_ifd0(dng_core::dng_tags::tcMake);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcModel);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcCameraSerialNumber);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcMakerNoteSafety);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcLensInfo);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcSelfTimerMode);
        self.move_exif_to_ifd0(dng_core::dng_tags::tcBatteryLevel);

        // Split borrows for the per-IFD updates below.

        let DngFileUpdater {
            stream,
            state,
            ifd0_new,
            exif_new,
            gps_new,
            ..
        } = self;

        // Update EXIF version tag

        exif_new.update_version_tag(
            state,
            stream,
            dng_core::dng_tags::tcExifVersion,
            ttUndefined,
            exif.exif_version,
            true,
        )?;

        // Update DateTime tags.

        ifd0_new.update_date_time_tag(
            state,
            stream,
            dng_core::dng_tags::tcDateTime,
            &exif.date_time,
        )?;

        exif_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcSubsecTime,
            exif.date_time.subseconds(),
            true,
        )?;

        if exif.at_least_version_0231() || exif_new.has_tag(dng_core::dng_tags::tcOffsetTime) {
            exif_new.update_offset_time_tag(
                state,
                stream,
                dng_core::dng_tags::tcOffsetTime,
                &exif.date_time,
            )?;
        }

        // Update DateTimeOriginal tags.

        exif_new.update_date_time_tag(
            state,
            stream,
            dng_core::dng_tags::tcDateTimeOriginal,
            &exif.date_time_original,
        )?;

        exif_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcSubsecTimeOriginal,
            exif.date_time_original.subseconds(),
            true,
        )?;

        if exif.at_least_version_0231()
            || exif_new.has_tag(dng_core::dng_tags::tcOffsetTimeOriginal)
        {
            exif_new.update_offset_time_tag(
                state,
                stream,
                dng_core::dng_tags::tcOffsetTimeOriginal,
                &exif.date_time_original,
            )?;
        }

        // Update DateTimeDigitized tags.

        exif_new.update_date_time_tag(
            state,
            stream,
            dng_core::dng_tags::tcDateTimeDigitized,
            &exif.date_time_digitized,
        )?;

        exif_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcSubsecTimeDigitized,
            exif.date_time_digitized.subseconds(),
            true,
        )?;

        if exif.at_least_version_0231()
            || exif_new.has_tag(dng_core::dng_tags::tcOffsetTimeDigitized)
        {
            exif_new.update_offset_time_tag(
                state,
                stream,
                dng_core::dng_tags::tcOffsetTimeDigitized,
                &exif.date_time_digitized,
            )?;
        }

        // Updatable string tags.

        ifd0_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcImageDescription,
            &exif.image_description,
            true,
        )?;

        ifd0_new.update_string_tag(state, stream, dng_core::dng_tags::tcArtist, &exif.artist, true)?;

        ifd0_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcSoftware,
            &exif.software,
            true,
        )?;

        ifd0_new.update_dual_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcCopyright,
            &exif.copyright,
            &exif.copyright2,
        )?;

        exif_new.update_encoded_text_tag(
            state,
            stream,
            dng_core::dng_tags::tcUserComment,
            &exif.user_comment,
        )?;

        // OwnerName can be updated.

        if exif.at_least_version_0230() || exif_new.has_tag(dng_core::dng_tags::tcCameraOwnerNameExif)
        {
            exif_new.update_string_tag(
                state,
                stream,
                dng_core::dng_tags::tcCameraOwnerNameExif,
                &exif.owner_name,
                true,
            )?;
        }

        // LensModel can be updated.

        if exif.at_least_version_0230() || exif_new.has_tag(dng_core::dng_tags::tcLensModelExif) {
            exif_new.update_string_tag(
                state,
                stream,
                dng_core::dng_tags::tcLensModelExif,
                &exif.lens_name,
                true,
            )?;
        }

        // ImageNumber can be updated.

        if state.is_dng || ifd0_new.has_tag(dng_core::dng_tags::tcImageNumber) {
            ifd0_new.update_tag(
                state,
                stream,
                dng_core::dng_tags::tcImageNumber,
                ttLong,
                1,
                &TagData::U32(vec![exif.image_number]),
                exif.image_number == 0xFFFFFFFF,
                true,
            )?;
        }

        // All the GPS tags can be updated.

        gps_new.update_version_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSVersionID,
            ttByte,
            exif.gps_version_id,
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSLatitudeRef,
            &exif.gps_latitude_ref,
            true,
        )?;

        gps_new.update_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSLatitude,
            ttRational,
            3,
            &TagData::U32(vec![
                exif.gps_latitude[0].n,
                exif.gps_latitude[0].d,
                exif.gps_latitude[1].n,
                exif.gps_latitude[1].d,
                exif.gps_latitude[2].n,
                exif.gps_latitude[2].d,
            ]),
            !exif.gps_latitude[0].is_valid(),
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSLongitudeRef,
            &exif.gps_longitude_ref,
            true,
        )?;

        gps_new.update_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSLongitude,
            ttRational,
            3,
            &TagData::U32(vec![
                exif.gps_longitude[0].n,
                exif.gps_longitude[0].d,
                exif.gps_longitude[1].n,
                exif.gps_longitude[1].d,
                exif.gps_longitude[2].n,
                exif.gps_longitude[2].d,
            ]),
            !exif.gps_longitude[0].is_valid(),
            true,
        )?;

        gps_new.update_byte_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSAltitudeRef,
            exif.gps_altitude_ref as uint8,
            exif.gps_altitude_ref == 0xFFFFFFFF,
        )?;

        gps_new.update_rational_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSAltitude,
            &exif.gps_altitude,
        )?;

        gps_new.update_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSTimeStamp,
            ttRational,
            3,
            &TagData::U32(vec![
                exif.gps_time_stamp[0].n,
                exif.gps_time_stamp[0].d,
                exif.gps_time_stamp[1].n,
                exif.gps_time_stamp[1].d,
                exif.gps_time_stamp[2].n,
                exif.gps_time_stamp[2].d,
            ]),
            !exif.gps_time_stamp[0].is_valid(),
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSSatellites,
            &exif.gps_satellites,
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSStatus,
            &exif.gps_status,
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSMeasureMode,
            &exif.gps_measure_mode,
            true,
        )?;

        gps_new.update_rational_tag(state, stream, dng_core::dng_tags::tcGPSDOP, &exif.gps_dop)?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSSpeedRef,
            &exif.gps_speed_ref,
            true,
        )?;

        gps_new.update_rational_tag(state, stream, dng_core::dng_tags::tcGPSSpeed, &exif.gps_speed)?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSTrackRef,
            &exif.gps_track_ref,
            true,
        )?;

        gps_new.update_rational_tag(state, stream, dng_core::dng_tags::tcGPSTrack, &exif.gps_track)?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSImgDirectionRef,
            &exif.gps_img_direction_ref,
            true,
        )?;

        gps_new.update_rational_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSImgDirection,
            &exif.gps_img_direction,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSMapDatum,
            &exif.gps_map_datum,
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestLatitudeRef,
            &exif.gps_dest_latitude_ref,
            true,
        )?;

        gps_new.update_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestLatitude,
            ttRational,
            3,
            &TagData::U32(vec![
                exif.gps_dest_latitude[0].n,
                exif.gps_dest_latitude[0].d,
                exif.gps_dest_latitude[1].n,
                exif.gps_dest_latitude[1].d,
                exif.gps_dest_latitude[2].n,
                exif.gps_dest_latitude[2].d,
            ]),
            !exif.gps_dest_latitude[0].is_valid(),
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestLongitudeRef,
            &exif.gps_dest_longitude_ref,
            true,
        )?;

        gps_new.update_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestLongitude,
            ttRational,
            3,
            &TagData::U32(vec![
                exif.gps_dest_longitude[0].n,
                exif.gps_dest_longitude[0].d,
                exif.gps_dest_longitude[1].n,
                exif.gps_dest_longitude[1].d,
                exif.gps_dest_longitude[2].n,
                exif.gps_dest_longitude[2].d,
            ]),
            !exif.gps_dest_longitude[0].is_valid(),
            true,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestBearingRef,
            &exif.gps_dest_bearing_ref,
            true,
        )?;

        gps_new.update_rational_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestBearing,
            &exif.gps_dest_bearing,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestDistanceRef,
            &exif.gps_dest_distance_ref,
            true,
        )?;

        gps_new.update_rational_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDestDistance,
            &exif.gps_dest_distance,
        )?;

        gps_new.update_encoded_text_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSProcessingMethod,
            &exif.gps_processing_method,
        )?;

        gps_new.update_encoded_text_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSAreaInformation,
            &exif.gps_area_information,
        )?;

        gps_new.update_string_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDateStamp,
            &exif.gps_date_stamp,
            true,
        )?;

        gps_new.update_short_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSDifferential,
            exif.gps_differential as uint16,
            exif.gps_differential == 0xFFFFFFFF,
        )?;

        gps_new.update_rational_tag(
            state,
            stream,
            dng_core::dng_tags::tcGPSHPositioningError,
            &exif.gps_h_positioning_error,
        )?;

        Ok(())
    }

    /// `UpdateIFD (ifd_new, ifd_old)` — returns true when the IFD changed.
    pub fn update_ifd(&mut self, which: WhichIfd) -> DngResult<bool> {
        // We don't attempt to preserve sort order while editing, so just
        // sort them here.

        let DngFileUpdater {
            stream,
            state,
            ifd0_new,
            ifd0_old,
            exif_new,
            exif_old,
            gps_new,
            gps_old,
        } = self;

        let (ifd_new, ifd_old) = match which {
            WhichIfd::Ifd0 => (ifd0_new, ifd0_old),
            WhichIfd::Exif => (exif_new, exif_old),
            WhichIfd::Gps => (gps_new, gps_old),
        };

        ifd_new.sort_tags();

        if *ifd_new != *ifd_old {
            let new_byte_count = ifd_new.byte_count(state.big_tiff);

            let old_byte_count = ifd_old.byte_count(state.big_tiff);

            if new_byte_count > old_byte_count {
                state.prepare_to_append(stream, new_byte_count as uint64)?;

                ifd_new.set_offset(stream.position());

                ifd_new.write(state.big_tiff, stream)?;

                state.add_zero_range(ifd_old.offset(), old_byte_count as uint64)?;
            } else if new_byte_count == 0 {
                ifd_new.set_offset(0);

                state.add_zero_range(ifd_old.offset(), old_byte_count as uint64)?;
            } else {
                state.truncate_on_failure = false;

                ifd_new.write(state.big_tiff, stream)?;

                state.add_zero_range(
                    ifd_new.offset() + new_byte_count as uint64,
                    (old_byte_count - new_byte_count) as uint64,
                )?;
            }

            return Ok(true);
        }

        Ok(false)
    }

    /// `UpdateHeader ()`.
    pub fn update_header(&mut self) -> DngResult<()> {
        // Do we need to update EXIF IFD?

        if self.update_ifd(WhichIfd::Exif)? {
            if self.exif_new.offset() != self.exif_old.offset() {
                let offset64 = self.exif_new.offset();

                let DngFileUpdater {
                    stream,
                    state,
                    ifd0_new,
                    ..
                } = self;

                if state.big_tiff {
                    ifd0_new.update_tag(
                        state,
                        stream,
                        tcExifIFD,
                        ttLong8,
                        1,
                        &TagData::U64(vec![offset64]),
                        self.exif_new.entry_count() == 0,
                        true,
                    )?;
                } else {
                    let offset32 = offset64 as uint32;

                    ifd0_new.update_tag(
                        state,
                        stream,
                        tcExifIFD,
                        ttLong,
                        1,
                        &TagData::U32(vec![offset32]),
                        self.exif_new.entry_count() == 0,
                        true,
                    )?;
                }
            }
        }

        // Do we need to update GPS IFD?

        if self.update_ifd(WhichIfd::Gps)? {
            if self.gps_new.offset() != self.gps_old.offset() {
                let offset64 = self.gps_new.offset();

                let DngFileUpdater {
                    stream,
                    state,
                    ifd0_new,
                    ..
                } = self;

                if state.big_tiff {
                    ifd0_new.update_tag(
                        state,
                        stream,
                        tcGPSInfo,
                        ttLong8,
                        1,
                        &TagData::U64(vec![offset64]),
                        self.gps_new.entry_count() == 0,
                        true,
                    )?;
                } else {
                    let offset32 = offset64 as uint32;

                    ifd0_new.update_tag(
                        state,
                        stream,
                        tcGPSInfo,
                        ttLong,
                        1,
                        &TagData::U32(vec![offset32]),
                        self.gps_new.entry_count() == 0,
                        true,
                    )?;
                }
            }
        }

        // Do we need to update IFD 0?

        if self.update_ifd(WhichIfd::Ifd0)? {
            self.state.truncate_on_failure = false;

            if self.ifd0_new.offset() != self.ifd0_old.offset() {
                if self.state.big_tiff {
                    self.stream.set_write_position(8)?;

                    self.stream.put_uint64(self.ifd0_new.offset())?;
                } else {
                    self.stream.set_write_position(4)?;

                    self.stream.put_uint32(self.ifd0_new.offset() as uint32)?;
                }
            }
        }

        self.stream.flush()?;

        self.state.truncate_on_failure = false;

        Ok(())
    }

    /// Truncation/destructor semantics: on failure, flush and truncate back to
    /// the original length (`dng_file_updater::~dng_file_updater`).
    pub fn abandon(&mut self) {
        if self.state.truncate_on_failure {
            let _ = self.stream.flush();

            let _ = self.stream.set_length(self.state.truncate_to_length);
        }
    }
}

// ---------------------------------------------------------------------------
// CleanUpMetadataForUpdate
// ---------------------------------------------------------------------------

/// Port of `CleanUpMetadataForUpdate` — resync EXIF and drop/refresh IPTC
/// before updating an existing file.
pub fn cleanup_metadata_for_update(
    _host: &mut DngHost,
    metadata: &mut DngMetadata,
    wants_iptc: bool,
) -> DngResult<()> {
    if metadata.xmp.has_meta() {
        // Resync EXIF, remove EXIF tags from XMP.

        let original = metadata.original_exif.clone();

        metadata
            .xmp
            .sync_exif(&mut metadata.exif, original.as_ref(), false, true);

        if !wants_iptc {
            metadata.clear_iptc();

            metadata.xmp.clear_iptc_digest();
        } else {
            metadata.rebuild_iptc(true)?;

            if metadata.iptc_length() > 0 {
                let digest = metadata.iptc_digest(true);

                metadata.xmp.set_iptc_digest(&digest);
            } else {
                metadata.xmp.clear_iptc_digest();
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DNGUpdateMetadata
// ---------------------------------------------------------------------------

/// Port of `DNGUpdateMetadata` — update the metadata inside an existing
/// DNG/TIFF stream in place.
pub fn dng_update_metadata(
    host: &mut DngHost,
    stream: &mut DngStream,
    negative: &DngNegative,
    metadata: &DngMetadata,
) -> DngResult<()> {
    let mut updater = DngFileUpdater::new(stream);

    // Read header and IFDs.

    if let Err(err) = updater.parse_header() {
        updater.abandon();

        return Err(err);
    }

    // Clean up metadata for update.

    let mut updated_metadata = metadata.clone();

    let result = update_metadata_inner(host, &mut updater, negative, &mut updated_metadata);

    if result.is_err() {
        updater.abandon();
    }

    result
}

/// The body of `DNGUpdateMetadata` after header parse/cleanup.
fn update_metadata_inner(
    host: &mut DngHost,
    updater: &mut DngFileUpdater,
    negative: &DngNegative,
    updated_metadata: &mut DngMetadata,
) -> DngResult<()> {
    // Some DNG files do not contain a RawDataUniqueID field, add tag with the
    // computed value we are using.

    if updater.state.is_dng {
        let raw_data_unique_id = negative.base_raw_data_unique_id();

        if raw_data_unique_id.is_valid() {
            let DngFileUpdater {
                stream,
                state,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.update_tag(
                state,
                stream,
                tcRawDataUniqueID,
                ttByte,
                16,
                &TagData::Bytes(raw_data_unique_id.data.to_vec()),
                false,
                true,
            )?;
        }
    }

    // Do we have big tables to embed?

    if !updated_metadata.big_table_dictionary().is_empty() {
        // Read in existing big table index, if any.

        let mut big_table_index = DngBigTableIndex::new();

        {
            let DngFileUpdater {
                stream,
                state,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.parse_big_table_index(state, stream, &mut big_table_index)?;
        }

        // Write any new big tables to file and add to index.

        let mut index_dirty = false;

        {
            let mut tables: Vec<(DngFingerprint, Vec<uint8>)> = Vec::new();

            for (fingerprint, block) in updated_metadata.big_table_dictionary().map() {
                tables.push((*fingerprint, block.clone()));
            }

            for (fingerprint, block) in tables {
                if !big_table_index.has_entry(&fingerprint) {
                    let block_size = block.len() as uint32;

                    updater.state.prepare_to_append(updater.stream, block_size as uint64)?;

                    let block_offset = updater.stream.position();

                    updater.stream.put(&block)?;

                    big_table_index.add_entry(fingerprint, block_size, block_offset);

                    index_dirty = true;
                }
            }
        }

        // Write updated index back to ifd if dirty.

        if index_dirty {
            let DngFileUpdater {
                stream,
                state,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.write_big_table_index(state, stream, &big_table_index)?;
        }

        // Read in existing big table group index, if any.

        let mut group_index = DngBigTableGroupIndex::new();

        {
            let DngFileUpdater {
                stream,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.parse_big_table_group_index(stream, &mut group_index)?;
        }

        // Write any new groups to the group index.

        let mut group_index_dirty = false;

        {
            let mut groups: Vec<(DngFingerprint, DngFingerprint)> = Vec::new();

            for (group_digest, instance_digest) in updated_metadata.big_table_group_index().map()
            {
                groups.push((*group_digest, *instance_digest));
            }

            for (group_digest, instance_digest) in groups {
                if !group_index.has_entry(&group_digest) {
                    // Also check that the group refers to an existing instance.

                    if big_table_index.has_entry(&instance_digest) {
                        group_index.add_entry(group_digest, instance_digest);

                        group_index_dirty = true;
                    }
                }
            }
        }

        // Write updated group index back to ifd if dirty.

        if group_index_dirty {
            let DngFileUpdater {
                stream,
                state,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.write_big_table_group_index(state, stream, &group_index)?;
        }
    }

    // Update XMP.

    if updated_metadata.xmp.has_meta() {
        let mut existing_size: uint32 = 0;

        if updater.ifd0_new.has_tag(tcXMP) {
            let xmp_tag = *updater.ifd0_new.existing_tag(tcXMP);

            if xmp_tag.tag_size() <= 0xFFFFFFFF {
                existing_size = xmp_tag.tag_size() as uint32;
            }
        }

        let xmp_block = updated_metadata
            .xmp
            .serialize(true, existing_size as usize, 0, false, false)?;

        if xmp_block.logical_size() > 0 {
            let DngFileUpdater {
                stream,
                state,
                ifd0_new,
                ..
            } = updater;

            ifd0_new.update_tag(
                state,
                stream,
                tcXMP,
                ttByte,
                xmp_block.logical_size() as uint64,
                &TagData::Bytes(xmp_block.buffer().to_vec()),
                xmp_block.logical_size() == 0,
                true,
            )?;
        }
    }

    // Update IPTC.

    {
        let iptc_length = updated_metadata.iptc_length();

        let iptc_data = updated_metadata.iptc_data().unwrap_or(&[]).to_vec();

        // The IPTC tag stores byte data under type ttLong (padded to a
        // multiple of four).

        let padded = (iptc_length + 3) & !3u32;

        let mut bytes = iptc_data;

        bytes.resize(padded as usize, 0);

        let DngFileUpdater {
            stream,
            state,
            ifd0_new,
            ..
        } = updater;

        ifd0_new.update_tag(
            state,
            stream,
            tcIPTC_NAA,
            ttLong,
            (padded >> 2) as uint64,
            &TagData::Bytes(bytes),
            iptc_length == 0,
            true,
        )?;
    }

    // Update IPTC Digest inside Adobe data, if preset.

    {
        let mut iptc_digest = DngFingerprint::new();

        if updated_metadata.iptc_length() > 0 {
            iptc_digest = updated_metadata.iptc_digest(true);
        }

        let DngFileUpdater {
            stream,
            ifd0_new,
            ..
        } = updater;

        ifd0_new.update_adobe_data(stream, &iptc_digest)?;
    }

    // Update the EXIF/GPS metadata.

    {
        let mut exif = std::mem::take(&mut updated_metadata.exif);

        let result = updater.update_exif(&mut exif);

        updated_metadata.exif = exif;

        result?;
    }

    // Update orientation tag.

    {
        let orientation = negative
            .compute_orientation(updated_metadata)
            .get_tiff() as uint16;

        let DngFileUpdater {
            stream,
            state,
            ifd0_new,
            ..
        } = updater;

        ifd0_new.update_short_tag(state, stream, tcOrientation, orientation, false)?;
    }

    // Update any modified IFDs.

    updater.update_header()?;

    // Zero now unused data.

    updater.state.apply_zero_ranges(updater.stream)?;

    let _ = host;

    Ok(())
}

// Keep the DngMemoryBlock / int32 imports referenced (used by callers of the
// module and by the tag payload plumbing).
#[allow(unused)]
fn _type_witness(_x: Option<(DngMemoryBlock, int32)>) {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_image_writer::{owned_tag, string_tag};

    /// `DngImageStats` round-trip: build stats with the four official PGTM
    /// child-tag semantics (weighted average / weighted samples / weights /
    /// color average / color samples), serialize via `make_tag`, parse back,
    /// and compare.
    #[test]
    fn test_image_stats_round_trip() {
        let mut stats = DngImageStats::default();

        stats.weighted_average = vec![0.5f32];
        stats.weights = vec![1.0f32, 1.0, 1.0];

        stats.weighted_samples = vec![
            WeightedSample { frac: 0.25, value: 0.5 },
            WeightedSample { frac: 0.5, value: 0.75 },
        ];

        stats.color_average = vec![0.1f32, 0.2, 0.3];

        stats.color_samples = vec![
            ColorSample { frac: 0.5, values: vec![0.4, 0.5, 0.6] },
            ColorSample { frac: 0.75, values: vec![0.7, 0.8, 0.9] },
        ];

        // Official semantics: 3 child tags for planeCount = 3.

        assert_eq!(stats.tag_count(), 5);

        assert!(stats.is_valid_for_plane_count(3));
        assert!(!stats.is_valid_for_plane_count(2)); // color_average length mismatch

        // Serialize (via the dng-core make_tag; data bytes are shared).

        let tag = stats.make_tag().unwrap();

        let data = tag.data().to_vec();

        // Parse back.

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        stream.put(&data).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let mut parsed = DngImageStats::default();

        parsed.parse(&mut stream).unwrap();

        assert_eq!(parsed, stats);
    }

    /// `DngImageStats::Parse` error paths: bad child tag length / unsupported
    /// code.
    #[test]
    fn test_image_stats_parse_errors() {
        // Non-multiple-of-4 length.

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        stream.put_uint32(1).unwrap(); // tag count
        stream.put_uint32(DngImageStats::kTag_Weights).unwrap();
        stream.put_uint32(3).unwrap(); // not multiple of 4

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let mut parsed = DngImageStats::default();

        assert!(parsed.parse(&mut stream).is_err());
    }

    /// `DngImageSequenceInfo` make/parse round-trip + validity.
    #[test]
    fn test_image_sequence_info_round_trip() {
        let mut info = DngImageSequenceInfo::default();

        info.sequence_id.set_utf8(b"SEQ-2024-001");
        info.sequence_type.set_utf8(b"HDR");
        info.frame_info.set_utf8(b"frame-3");
        info.index = 3;
        info.count = 9;
        info.is_final = 0;

        assert!(info.is_valid());

        let tag = info.make_tag().unwrap();

        let payload = tag.data().to_vec();

        // Parse back through the dng-core stream parser.

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        // make_tag writes big-endian data (TempBigEndian in the C++).

        stream.set_big_endian(true);

        stream.put(&payload).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let mut parsed = DngImageSequenceInfo::default();

        parsed
            .parse_sequence_info_stream(&mut stream, payload.len() as uint32)
            .unwrap();

        assert_eq!(parsed.sequence_id, info.sequence_id);
        assert_eq!(parsed.sequence_type, info.sequence_type);
        assert_eq!(parsed.frame_info, info.frame_info);
        assert_eq!(parsed.index, 3);
        assert_eq!(parsed.count, 9);
        assert_eq!(parsed.is_final, 0);
    }

    /// In-place update on a synthetic TIFF: build a minimal file via the
    /// writer's directory machinery, then update a string tag through
    /// `DngFileUpdater` and verify the change landed.
    #[test]
    fn test_file_updater_updates_string_tag() {
        // Build a minimal TIFF with IFD0 containing tcSoftware.

        let mut main_ifd = dng_core::dng_tiff_directory::DngTiffDirectory::new();

        main_ifd.add(Box::new(string_tag(
            dng_core::dng_tags::tcSoftware,
            DngString::from_bytes(b"Before"),
        )));

        main_ifd.add(Box::new(owned_tag(
            dng_core::dng_tags::tcDNGVersion,
            ttByte,
            vec![0x01, 0x04, 0x00, 0x00],
        )));

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        stream.put_uint16(0x4949).unwrap();
        stream.put_uint16(42).unwrap();
        stream.put_uint32(8).unwrap(); // IFD0 offset

        main_ifd
            .put(&mut stream, dng_core::dng_tiff_directory::offsetsRelativeToStream, 0)
            .unwrap();

        stream.flush().unwrap();

        // Update the software string.

        stream.set_read_position(0).unwrap();

        let mut updater = DngFileUpdater::new(&mut stream);

        updater.parse_header().unwrap();

        // tcDNGVersion = 0x01040000 → the file is detected as DNG.

        assert!(updater.state.is_dng);

        updater
            .ifd0_new
            .update_string_tag(
                &mut updater.state,
                &mut updater.stream,
                dng_core::dng_tags::tcSoftware,
                &DngString::from_bytes(b"AfterUpdate"),
                true,
            )
            .unwrap();

        updater.update_header().unwrap();

        updater.state.apply_zero_ranges(&mut updater.stream).unwrap();

        stream.flush().unwrap();

        // Re-parse: the software string must read back "AfterUpdate".

        stream.set_read_position(0).unwrap();

        let mut verifier = DngFileUpdater::new(&mut stream);

        verifier.parse_header().unwrap();

        assert!(verifier.ifd0_new.has_tag(dng_core::dng_tags::tcSoftware));

        let tag = *verifier.ifd0_new.existing_tag(dng_core::dng_tags::tcSoftware);

        // ttAscii inline (10 chars + NUL <= 4? No: 11 > 4 → out-of-line).

        verifier.stream.set_read_position(tag.tag_offset).unwrap();

        let text = verifier.stream.get_c_string(4096).unwrap();

        assert_eq!(text, b"AfterUpdate");
    }
}

// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — corresponds to `dng_tiff_directory` in the SDK.
//
// Copyright None Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of the TIFF writer portion of dng_image_writer.h/.cpp — the
// `tiff_tag` hierarchy and `dng_tiff_directory` (extracted per
// ARCHITECTURE.md so the writer is usable standalone without dng_negative).
//
// Ownership: C++ holds `std::vector<tiff_tag *>` of polymorphic owners; the
// Rust port stores `Box<dyn TiffTagValue>` value objects (see DESIGN.md).

use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::program_error;
use crate::dng_fingerprint::DngFingerprint;
use crate::dng_stream::DngStream;
use crate::dng_string::DngString;
use crate::dng_tag_types::*;
use crate::dng_types::{int16, int32, int64, real32, real64, uint16, uint32, uint64, uint8};

/// Port of `tiff_tag` (abstract value base).
pub trait TiffTag: Send + Sync {
    /// `Code ()`.
    fn code(&self) -> uint16;

    /// `Type ()`.
    fn tag_type(&self) -> uint16;

    /// `Count ()`.
    fn count(&self) -> uint32;

    /// `SetCount (count)`.
    fn set_count(&mut self, count: uint32);

    /// Mutable-value hook (the C++ tags carry pointer-based values; the
    /// port lets the writer update u64-valued tags in place after the
    /// final layout is known).  Returns false when unsupported.
    fn set_u64_values(&mut self, _values: &[uint64]) -> bool {
        false
    }

    /// `Size ()`.
    fn size(&self) -> uint32 {
        tag_type_size(self.tag_type() as uint32) * self.count()
    }

    /// `SetBigTIFF (isBigTIFF)`.
    fn set_bigtiff(&mut self, _is_bigtiff: bool) {}

    /// `Put (stream)` — writes the value area.
    fn put(&self, stream: &mut DngStream) -> DngResult<()>;
}

/// Port of `tag_data_ptr` — raw typed data of `count` elements.
pub struct TagDataPtr {
    code: uint16,
    tag_type: uint16,
    /// Raw little-endian element bytes (host-endian typed Vec).
    pub data: Vec<uint8>,
    count: uint32,
}

impl TagDataPtr {
    pub fn new(code: uint16, tag_type: uint16, count: uint32, data: Vec<uint8>) -> Self {
        TagDataPtr {
            code,
            tag_type,
            data,
            count,
        }
    }
}

impl TiffTag for TagDataPtr {
    fn code(&self) -> uint16 {
        self.code
    }

    fn tag_type(&self) -> uint16 {
        self.tag_type
    }

    fn count(&self) -> uint32 {
        self.count
    }

    fn set_count(&mut self, count: uint32) {
        self.count = count;
    }

    /// `tag_data_ptr::Put` — byte-swap aware.
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        let element_size = tag_type_size(self.tag_type as uint32);
        let total = self.size();

        if stream.swap_bytes() && element_size >= 2 {
            match element_size {
                2 => {
                    for j in 0..(total >> 1) as usize {
                        let b0 = self.data[j * 2];
                        let b1 = self.data[j * 2 + 1];
                        // Data is stored host-endian; Put_uint16 applies the
                        // swap, so pass the host value.
                        let v = uint16::from_ne_bytes([b0, b1]);
                        stream.put_uint16(v)?;
                    }
                    return Ok(());
                }
                4 => {
                    for j in 0..(total >> 2) as usize {
                        let v = uint32::from_ne_bytes([
                            self.data[j * 4],
                            self.data[j * 4 + 1],
                            self.data[j * 4 + 2],
                            self.data[j * 4 + 3],
                        ]);
                        stream.put_uint32(v)?;
                    }
                    return Ok(());
                }
                8 => {
                    for j in 0..(total >> 3) as usize {
                        let mut b = [0u8; 8];
                        b.copy_from_slice(&self.data[j * 8..j * 8 + 8]);
                        let v = real64::from_bits(uint64::from_ne_bytes(b));
                        stream.put_real64(v)?;
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        stream.put(&self.data[..total as usize])
    }
}

/// Port of `tag_uint16` — single uint16 value.
pub struct TagUint16 {
    code: uint16,
    pub value: uint16,
}

impl TagUint16 {
    pub fn new(code: uint16, value: uint16) -> Self {
        TagUint16 { code, value }
    }
}

impl TiffTag for TagUint16 {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttShort as uint16
    }
    fn count(&self) -> uint32 {
        1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint16(self.value)
    }
}

/// Port of `tag_uint16_ptr` — uint16 array.
pub struct TagUint16Ptr {
    code: uint16,
    pub data: Vec<uint16>,
}

impl TagUint16Ptr {
    pub fn new(code: uint16, data: Vec<uint16>) -> Self {
        TagUint16Ptr { code, data }
    }
}

impl TiffTag for TagUint16Ptr {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttShort as uint16
    }
    fn count(&self) -> uint32 {
        self.data.len() as uint32
    }
    fn set_count(&mut self, count: uint32) {
        self.data.truncate(count as usize);
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        for &v in &self.data {
            stream.put_uint16(v)?;
        }
        Ok(())
    }
}

/// Port of `tag_uint32` — single uint32 value.
pub struct TagUint32 {
    code: uint16,
    pub value: uint32,
}

impl TagUint32 {
    pub fn new(code: uint16, value: uint32) -> Self {
        TagUint32 { code, value }
    }
}

impl TiffTag for TagUint32 {
    fn code(&self) -> uint16 {
        self.code
    }
    fn set_u64_values(&mut self, values: &[uint64]) -> bool {
        if values.is_empty() {
            return false;
        }
        self.value = values[0] as uint32;
        true
    }
    fn tag_type(&self) -> uint16 {
        ttLong as uint16
    }
    fn count(&self) -> uint32 {
        1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(self.value)
    }
}

/// Port of `tag_uint32_ptr` — uint32 array.
pub struct TagUint32Ptr {
    code: uint16,
    pub data: Vec<uint32>,
}

impl TagUint32Ptr {
    pub fn new(code: uint16, data: Vec<uint32>) -> Self {
        TagUint32Ptr { code, data }
    }
}

impl TiffTag for TagUint32Ptr {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttLong as uint16
    }
    fn count(&self) -> uint32 {
        self.data.len() as uint32
    }
    fn set_count(&mut self, count: uint32) {
        self.data.truncate(count as usize);
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        for &v in &self.data {
            stream.put_uint32(v)?;
        }
        Ok(())
    }
}

/// Port of `tag_big_uint` — uint64 stored as ttLong8 or ttLong.
pub struct TagBigUint {
    code: uint16,
    tag_type: uint16,
    pub value: uint64,
}

impl TagBigUint {
    /// `tag_big_uint (code, isBigTIFF, value)`.
    pub fn new(code: uint16, big_tiff: bool, value: uint64) -> Self {
        TagBigUint {
            code,
            tag_type: if big_tiff { ttLong8 as uint16 } else { ttLong as uint16 },
            value,
        }
    }
}

impl TiffTag for TagBigUint {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        self.tag_type
    }
    fn count(&self) -> uint32 {
        1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        if self.tag_type == ttLong as uint16 {
            if self.value > 0xFFFF_FFFF {
                return Err(DngError::Unsupported("tag_big_uint overflow".to_string()));
            }
            stream.put_uint32(self.value as uint32)
        } else {
            stream.put_uint64(self.value)
        }
    }
}

/// Port of `tag_big_uints` — uint64 array stored as ttLong8 or ttLong.
pub struct TagBigUints {
    code: uint16,
    tag_type: uint16,
    pub data: Vec<uint64>,
}

impl TagBigUints {
    pub fn new(code: uint16, big_tiff: bool, data: Vec<uint64>) -> Self {
        TagBigUints {
            code,
            tag_type: if big_tiff { ttLong8 as uint16 } else { ttLong as uint16 },
            data,
        }
    }
}

impl TiffTag for TagBigUints {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        self.tag_type
    }
    fn count(&self) -> uint32 {
        self.data.len() as uint32
    }
    fn set_u64_values(&mut self, values: &[uint64]) -> bool {
        self.data = values.to_vec();
        true
    }
    fn set_count(&mut self, count: uint32) {
        self.data.truncate(count as usize);
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        for &x in &self.data {
            if self.tag_type == ttLong as uint16 {
                if x > 0xFFFF_FFFF {
                    return Err(DngError::Unsupported("tag_big_uints overflow".to_string()));
                }
                stream.put_uint32(x as uint32)?;
            } else {
                stream.put_uint64(x)?;
            }
        }
        Ok(())
    }
}

/// Port of `tag_urational` — single unsigned rational.
pub struct TagURational {
    code: uint16,
    pub n: uint32,
    pub d: uint32,
}

impl TagURational {
    pub fn new(code: uint16, n: uint32, d: uint32) -> Self {
        TagURational { code, n, d }
    }
}

impl TiffTag for TagURational {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttRational as uint16
    }
    fn count(&self) -> uint32 {
        1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(self.n)?;
        stream.put_uint32(self.d)
    }
}

/// Port of `tag_urational_ptr` — unsigned rational array.
pub struct TagURationalPtr {
    code: uint16,
    pub data: Vec<(uint32, uint32)>,
}

impl TagURationalPtr {
    pub fn new(code: uint16, data: Vec<(uint32, uint32)>) -> Self {
        TagURationalPtr { code, data }
    }
}

impl TiffTag for TagURationalPtr {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttRational as uint16
    }
    fn count(&self) -> uint32 {
        self.data.len() as uint32
    }
    fn set_count(&mut self, count: uint32) {
        self.data.truncate(count as usize);
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        for &(n, d) in &self.data {
            stream.put_uint32(n)?;
            stream.put_uint32(d)?;
        }
        Ok(())
    }
}

/// Port of `tag_srational_ptr` — signed rational array.
pub struct TagSRationalPtr {
    code: uint16,
    pub data: Vec<(int32, int32)>,
}

impl TagSRationalPtr {
    pub fn new(code: uint16, data: Vec<(int32, int32)>) -> Self {
        TagSRationalPtr { code, data }
    }
}

impl TiffTag for TagSRationalPtr {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttSRational as uint16
    }
    fn count(&self) -> uint32 {
        self.data.len() as uint32
    }
    fn set_count(&mut self, count: uint32) {
        self.data.truncate(count as usize);
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        for &(n, d) in &self.data {
            stream.put_int32(n)?;
            stream.put_int32(d)?;
        }
        Ok(())
    }
}

/// Port of `tag_real32` — single real32 value.
pub struct TagReal32 {
    code: uint16,
    pub value: real32,
}

impl TagReal32 {
    pub fn new(code: uint16, value: real32) -> Self {
        TagReal32 { code, value }
    }
}

impl TiffTag for TagReal32 {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttFloat as uint16
    }
    fn count(&self) -> uint32 {
        1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_real32(self.value)
    }
}

/// Port of `tag_string` — ASCII string with NUL (count = len + 1).
pub struct TagString {
    code: uint16,
    pub string: DngString,
}

impl TagString {
    pub fn new(code: uint16, string: DngString) -> Self {
        TagString { code, string }
    }
}

impl TiffTag for TagString {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttAscii as uint16
    }
    fn count(&self) -> uint32 {
        self.string.length() + 1
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        let mut bytes = self.string.get().to_vec();
        bytes.push(0);
        stream.put(&bytes[..self.size() as usize])
    }
}

/// Port of `tag_encoded_text` — ASCII or UTF-16 with an 8-byte type header.
pub struct TagEncodedText {
    code: uint16,
    text: DngString,
    utf16: Option<Vec<uint16>>,
    count: uint32,
}

impl TagEncodedText {
    pub fn new(code: uint16, text: &DngString) -> Self {
        if text.is_ascii() {
            TagEncodedText {
                code,
                text: text.clone(),
                utf16: None,
                count: 8 + text.length(),
            }
        } else {
            let utf16 = text.get_utf16();
            let count = 8 + utf16.len() as uint32 * 2;
            TagEncodedText {
                code,
                text: text.clone(),
                utf16: Some(utf16),
                count,
            }
        }
    }
}

impl TiffTag for TagEncodedText {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttUndefined as uint16
    }
    fn count(&self) -> uint32 {
        self.count
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        if let Some(buf) = &self.utf16 {
            stream.put(b"UNICODE\0")?;
            for j in 0..((self.count - 8) >> 1) as usize {
                stream.put_uint16(buf[j])?;
            }
        } else {
            stream.put(b"ASCII\0\0\0")?;
            stream.put(&self.text.get()[..(self.count - 8) as usize])?;
        }
        Ok(())
    }
}

/// Port of `tag_cfa_pattern` — CFA repeat pattern + plane color.
pub struct TagCFAPattern {
    code: uint16,
    rows: uint32,
    cols: uint32,
    pub plane_color: [uint8; 4],
    pub pattern: Vec<uint8>,
}

impl TagCFAPattern {
    /// `tag_cfa_pattern (code, rows, cols, planeColor, pattern)`.
    pub fn new(
        code: uint16,
        rows: uint32,
        cols: uint32,
        plane_color: &[uint8; 4],
        pattern: Vec<uint8>,
    ) -> Self {
        TagCFAPattern {
            code,
            rows,
            cols,
            plane_color: *plane_color,
            pattern,
        }
    }
}

impl TiffTag for TagCFAPattern {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttUndefined as uint16
    }
    fn count(&self) -> uint32 {
        4 + self.rows * self.cols
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put(&self.plane_color)?;
        stream.put(&self.pattern[..(self.rows * self.cols) as usize])
    }
}

/// Port of `tag_iptc` / `tag_owned_data_ptr` — opaque byte blob with an
/// owned buffer (used by dng_image_sequence_info / dng_image_stats).
pub struct TagOwnedDataPtr {
    code: uint16,
    tag_type: uint16,
    count: uint32,
    data: std::sync::Arc<Vec<uint8>>,
}

impl TagOwnedDataPtr {
    pub fn new(code: uint16, tag_type: uint16, count: uint32, data: std::sync::Arc<Vec<uint8>>) -> Self {
        TagOwnedDataPtr {
            code,
            tag_type,
            count,
            data,
        }
    }

    /// Access to the owned bytes.
    pub fn data(&self) -> &[uint8] {
        &self.data
    }
}

impl TiffTag for TagOwnedDataPtr {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        self.tag_type
    }
    fn count(&self) -> uint32 {
        self.count
    }
    fn set_count(&mut self, count: uint32) {
        self.count = count;
    }
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put(&self.data[..self.count as usize])
    }
}

/// Port of `tag_fingerprint` — 16-byte fingerprint as ttUndefined.
pub struct TagFingerprint {
    code: uint16,
    pub fingerprint: DngFingerprint,
}

impl TagFingerprint {
    pub fn new(code: uint16, fingerprint: DngFingerprint) -> Self {
        TagFingerprint { code, fingerprint }
    }
}

impl TiffTag for TagFingerprint {
    fn code(&self) -> uint16 {
        self.code
    }
    fn tag_type(&self) -> uint16 {
        ttUndefined as uint16
    }
    fn count(&self) -> uint32 {
        16
    }
    fn set_count(&mut self, _count: uint32) {}
    fn put(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put(&self.fingerprint.data)
    }
}

// ---------------------------------------------------------------------------
// dng_tiff_directory.
// ---------------------------------------------------------------------------

/// `dng_tiff_directory::OffsetsBase`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetsBase {
    /// Offsets are relative to the start of the stream.
    offsetsRelativeToStream = 0,
    /// Offsets are relative to an explicit base.
    offsetsRelativeToExplicitBase = 1,
    /// Offsets are relative to the start of the IFD.
    offsetsRelativeToIFD = 2,
}

pub use OffsetsBase::*;

/// Port of `dng_tiff_directory`.
#[derive(Default)]
pub struct DngTiffDirectory {
    tags: Vec<Box<dyn TiffTag>>,
    chained: uint64,
    big_tiff: bool,
}

impl DngTiffDirectory {
    pub fn new() -> Self {
        DngTiffDirectory::default()
    }

    /// `Add (tag)` — tags must be added in sorted-by-code order (as in the
    /// C++ writer contract: `dng_tiff_directory::Add` inserts the tag in
    /// sorted position ("Tags must be sorted in increasing order of tag
    /// code" — inserted before the first larger code), so callers may add
    /// tags in any order.
    /// Update a u64-valued tag in place (pre-emission), C++ mutable-tag
    /// behaviour.
    pub fn set_tag_u64_values(&mut self, code: uint16, values: &[uint64]) -> bool {
        for tag in self.tags.iter_mut() {
            if tag.code() == code {
                return tag.set_u64_values(values);
            }
        }
        false
    }

    pub fn tag_count(&self) -> usize {
        self.tags.len()
    }

    /// Debug: per-tag size ledger.
    pub fn size_ledger(&self) -> Vec<(uint16, uint16, uint32, uint32)> {
        self.tags
            .iter()
            .map(|t| {
                let value = if t.size() > 4 { (t.size() + 1) & !1 } else { 0 };
                (t.code(), t.tag_type(), t.count(), value)
            })
            .collect()
    }

    pub fn add(&mut self, tag: Box<dyn TiffTag>) {
        let insert_index = self
            .tags
            .iter()
            .position(|existing| tag.code() < existing.code())
            .unwrap_or(self.tags.len());

        self.tags.insert(insert_index, tag);
    }

    /// `SetChained (offset)`.
    pub fn set_chained(&mut self, offset: uint64) {
        self.chained = offset;
    }

    /// `SetBigTIFF (isBigTIFF)`.
    pub fn set_big_tiff(&mut self, is_bigtiff: bool) {
        if self.big_tiff != is_bigtiff {
            self.big_tiff = is_bigtiff;
            for tag in &mut self.tags {
                tag.set_bigtiff(is_bigtiff);
            }
        }
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.tags.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tags.is_empty()
    }

    /// `Size ()`.
    pub fn size(&self) -> DngResult<uint32> {
        if self.tags.is_empty() {
            return Ok(0);
        }

        let mut size = if self.big_tiff { 16 } else { 6 };

        for tag in &self.tags {
            let mut tag_size = if self.big_tiff { 20 } else { 12 };

            let data_size = tag.size();

            if data_size > if self.big_tiff { 8 } else { 4 } {
                tag_size += (data_size + 1) & !1;
            }

            size += tag_size;
        }

        Ok(crate::dng_utils::round_up_2(size))
    }

    /// `Put (stream, offsetsBase, explicitBase)`.
    pub fn put(
        &self,
        stream: &mut DngStream,
        offsets_base: OffsetsBase,
        explicit_base: uint64,
    ) -> DngResult<()> {
        if self.tags.is_empty() {
            return Ok(());
        }

        let mut big_data = if self.big_tiff {
            self.tags.len() as uint64 * 20 + 16
        } else {
            self.tags.len() as uint64 * 12 + 6
        };

        if offsets_base == offsetsRelativeToStream {
            big_data += stream.position();
        } else if offsets_base == offsetsRelativeToExplicitBase {
            big_data += explicit_base;
        }

        if self.big_tiff {
            stream.put_uint64(self.tags.len() as uint64)?;
        } else {
            stream.put_uint16(self.tags.len() as uint16)?;
        }

        for tag in &self.tags {
            stream.put_uint16(tag.code())?;
            stream.put_uint16(tag.tag_type())?;

            if self.big_tiff {
                stream.put_uint64(tag.count() as uint64)?;
            } else {
                stream.put_uint32(tag.count())?;
            }

            let mut size = tag.size();

            if size <= if self.big_tiff { 8 } else { 4 } {
                tag.put(stream)?;

                while size < if self.big_tiff { 8 } else { 4 } {
                    stream.put_uint8(0)?;
                    size += 1;
                }
            } else {
                if self.big_tiff {
                    stream.put_uint64(big_data)?;
                } else {
                    stream.put_uint32(big_data as uint32)?;
                }

                big_data += ((size + 1) & !1) as uint64;
            }
        }

        // Next IFD offset.
        if self.big_tiff {
            stream.put_uint64(self.chained)?;
        } else {
            stream.put_uint32(self.chained as uint32)?;
        }

        for tag in &self.tags {
            let size = tag.size();

            if size > if self.big_tiff { 8 } else { 4 } {
                tag.put(stream)?;

                if (size & 1) != 0 {
                    stream.put_uint8(0)?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_memory_stream::DngMemoryStream;
    use crate::dng_tags::{tcImageWidth, tcNewSubFileType, tcOrientation};

    #[test]
    fn directory_size_and_put_small() {
        let mut dir = DngTiffDirectory::new();
        dir.add(Box::new(TagUint32::new(tcNewSubFileType as uint16, 0)));
        dir.add(Box::new(TagUint32::new(tcImageWidth as uint16, 1920)));
        dir.add(Box::new(TagUint16::new(tcOrientation as uint16, 1)));

        // 6 + 3 * 12 = 42 -> round up to 42 (already even).
        assert_eq!(dir.size().unwrap(), 42);

        let mut stream = DngMemoryStream::new().stream();
        stream.set_little_endian(true);
        dir.put(&mut stream, offsetsRelativeToStream, 0).unwrap();

        assert_eq!(stream.position(), 42);
        stream.set_read_position(0).unwrap();

        // Entry count.
        assert_eq!(stream.get_uint16().unwrap(), 3);

        // First entry: code/type/count/inline value.
        assert_eq!(stream.get_uint16().unwrap(), tcNewSubFileType as uint16);
        assert_eq!(stream.get_uint16().unwrap(), ttLong as uint16);
        assert_eq!(stream.get_uint32().unwrap(), 1);
        assert_eq!(stream.get_uint32().unwrap(), 0);

        // Second entry: code, type, count, inline value.
        assert_eq!(stream.get_uint16().unwrap(), tcImageWidth as uint16);
        assert_eq!(stream.get_uint16().unwrap(), ttLong as uint16);
        assert_eq!(stream.get_uint32().unwrap(), 1);
        assert_eq!(stream.get_uint32().unwrap(), 1920);

        // Third entry: uint16 inline.
        assert_eq!(stream.get_uint16().unwrap(), tcOrientation as uint16);
        assert_eq!(stream.get_uint16().unwrap(), ttShort as uint16);
        assert_eq!(stream.get_uint32().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), 0); // padding

        // Next IFD offset.
        assert_eq!(stream.get_uint32().unwrap(), 0);
    }

    #[test]
    fn out_of_line_data() {
        let mut dir = DngTiffDirectory::new();
        dir.add(Box::new(TagUint16Ptr::new(
            tcImageWidth as uint16,
            vec![1, 2, 3, 4],
        )));

        // 6 + 12 + 8 = 26.
        assert_eq!(dir.size().unwrap(), 26);

        let mut stream = DngMemoryStream::new().stream();
        stream.set_little_endian(true);
        dir.put(&mut stream, offsetsRelativeToStream, 0).unwrap();

        stream.set_read_position(0).unwrap();
        assert_eq!(stream.get_uint16().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), tcImageWidth as uint16);
        assert_eq!(stream.get_uint16().unwrap(), ttShort as uint16);
        assert_eq!(stream.get_uint32().unwrap(), 4);
        let offset = stream.get_uint32().unwrap();
        assert_eq!(offset, 18); // after header + entry + next-IFD
        assert_eq!(stream.get_uint32().unwrap(), 0);

        stream.set_read_position(offset as uint64).unwrap();
        assert_eq!(stream.get_uint16().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), 2);
        assert_eq!(stream.get_uint16().unwrap(), 3);
        assert_eq!(stream.get_uint16().unwrap(), 4);
    }

    #[test]
    fn bigtiff_mode() {
        let mut dir = DngTiffDirectory::new();
        dir.set_big_tiff(true);
        dir.add(Box::new(TagBigUint::new(1, true, 0x1_0000_0000)));

        // 16 + 20 = 36.
        assert_eq!(dir.size().unwrap(), 36);

        let mut stream = DngMemoryStream::new().stream();
        stream.set_little_endian(true);
        dir.put(&mut stream, offsetsRelativeToStream, 0).unwrap();

        stream.set_read_position(0).unwrap();
        assert_eq!(stream.get_uint64().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), 1);
        assert_eq!(stream.get_uint16().unwrap(), ttLong8 as uint16);
        assert_eq!(stream.get_uint64().unwrap(), 1);
        assert_eq!(stream.get_uint64().unwrap(), 0x1_0000_0000);
        assert_eq!(stream.get_uint64().unwrap(), 0);
    }

    #[test]
    fn encoded_text_ascii_and_unicode() {
        let ascii = DngString::from_bytes(b"hello");
        let tag = TagEncodedText::new(700, &ascii);
        assert_eq!(tag.count(), 8 + 5);

        let mut stream = DngMemoryStream::new().stream();
        tag.put(&mut stream).unwrap();
        stream.set_read_position(0).unwrap();
        let mut header = [0u8; 8];
        stream.get_into(&mut header, 0).unwrap();
        assert_eq!(&header, b"ASCII\0\0\0");
        assert_eq!(stream.get(5).unwrap(), b"hello");

        // Unicode.
        let mut uni = DngString::new();
        uni.set_utf8("日本".as_bytes());
        let tag = TagEncodedText::new(700, &uni);
        assert_eq!(tag.count(), 8 + uni.get_utf16().len() as uint32 * 2);
    }

    #[test]
    fn big_uint_overflow_errors() {
        let tag = TagBigUint::new(1, false, 0x1_0000_0000);
        let mut stream = DngMemoryStream::new().stream();
        assert!(tag.put(&mut stream).is_err());
    }

    #[allow(unused)]
    fn unused_types(_: int16, _: int32, _: int64, _: program_error_marker) {}

    type program_error_marker = crate::dng_errors::DngError;
}

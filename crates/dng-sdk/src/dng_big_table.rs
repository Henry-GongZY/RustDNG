// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_big_table.cpp/.h`.
//
// Copyright 2015-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_big_table.cpp` / `dng_big_table.h` — the "big table"
//! infrastructure: the abstract `dng_big_table` base (fingerprinting,
//! zlib-compressed binary codec, Z85-like ASCII codec for XMP),
//! `dng_big_table_dictionary`, `dng_big_table_index`,
//! `dng_big_table_group_index`, `dng_big_table_storage`, and the XMP
//! read/write glue.
//!
//! The specialized table subclasses (`dng_look_table`, `dng_rgb_table`,
//! `dng_image_table`, `dng_packed_image_table`, `dng_masked_rgb_tables`)
//! build on this base; see PORTING_NOTES.md for their scheduling.
//!
//! `DngImageSequenceInfo` / `DngImageStats` (PGTM) live in
//! `dng_core::dng_utils` and are re-exported here for the `dng_ifd` glue.

use dng_core::dng_exceptions::{bad_format, program_error};
use dng_core::dng_fingerprint::{DngFingerprint, DngMd5Printer};
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_memory_stream::DngMemoryStream;
use dng_core::dng_stream::DngStream;
use dng_core::dng_types::{uint32, uint64, uint8};
use dng_core::DngResult;
use dng_core::zlib;

// ---------------------------------------------------------------------------
// Big table type enum (BigTableTypeEnum)
// ---------------------------------------------------------------------------

/// `btt_LookTable`.
pub const btt_LookTable: uint32 = 0;
/// `btt_RGBTable`.
pub const btt_RGBTable: uint32 = 1;
/// `btt_ImageTable`.
pub const btt_ImageTable: uint32 = 2;
/// `btt_CompressedSettingsTable`.
pub const btt_CompressedSettingsTable: uint32 = 3;
/// `btt_UncompressedSettingsTable`.
pub const btt_UncompressedSettingsTable: uint32 = 4;
/// `btt_PackedImageTable`.
pub const btt_PackedImageTable: uint32 = 5;

// ---------------------------------------------------------------------------
// ASCII <-> binary codec (Z85-like, adjusted for XMP cleanliness)
// ---------------------------------------------------------------------------

/// `kEncodeTable` from `dng_big_table::EncodeAsString`.
pub const K_ENCODE_TABLE: &[u8; 85] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWX\
YZ.-:+=^!/*?`'|()[]{}@%$#";

/// `kDecodeTable` from `dng_big_table::ASCIItoBinary` (96 entries, ASCII
/// 32..127; 0xFF = invalid).
pub const K_DECODE_TABLE: [uint8; 96] = [
    0xFF, 0x44, 0xFF, 0x54, 0x53, 0x52, 0xFF, 0x49, 0x4B, 0x4C, 0x46, 0x41,
    0xFF, 0x3F, 0x3E, 0x45,
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09,
    0x40, 0xFF, 0xFF, 0x42, 0xFF, 0x47, 0x51,
    0x24, 0x25, 0x26, 0x27, 0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x2D, 0x2E, 0x2F,
    0x30, 0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3A, 0x3B, 0x3C, 0x3D,
    0x4D, 0xFF, 0x4E, 0x43, 0xFF, 0x48,
    0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17,
    0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23,
    0x4F, 0x4A, 0x50, 0xFF, 0xFF,
];

/// Port of `dng_big_table::ASCIItoBinary` — decodes the Z85-like text
/// encoding to binary. Invalid characters (outside ASCII 32..127 or not in
/// the encode table) are skipped, matching the C++.
pub fn ascii_to_binary(s: &[u8]) -> Vec<uint8> {
    let mut out = Vec::with_capacity((s.len() + 4) / 5 * 4);

    let mut phase: uint32 = 0;
    let mut value: uint32 = 0;

    for &e in s {
        if e < 32 || e > 127 {
            continue;
        }

        let d = K_DECODE_TABLE[(e - 32) as usize];

        if d > 85 {
            continue;
        }

        phase += 1;

        if phase == 1 {
            value = d as uint32;
        } else if phase == 2 {
            value += d as uint32 * 85;
        } else if phase == 3 {
            value += d as uint32 * (85 * 85);
        } else if phase == 4 {
            value += d as uint32 * (85 * 85 * 85);
        } else {
            value += d as uint32 * (85 * 85 * 85 * 85);

            out.push((value & 0xFF) as uint8);
            out.push(((value >> 8) & 0xFF) as uint8);
            out.push(((value >> 16) & 0xFF) as uint8);
            out.push(((value >> 24) & 0xFF) as uint8);

            phase = 0;
        }
    }

    // Trailing partial group: the C++ writes byte2, then byte1, then byte0.

    if phase > 3 {
        out.push(((value >> 16) & 0xFF) as uint8);
    }

    if phase > 2 {
        out.push(((value >> 8) & 0xFF) as uint8);
    }

    if phase > 1 {
        out.push((value & 0xFF) as uint8);
    }

    out
}

/// Port of the `EncodeAsString` binary-to-text loop (inverse of
/// [`ascii_to_binary`]). `data.len()` must be a multiple of 4; callers
/// zero-pad like the C++ block buffer.
pub fn binary_to_ascii(data: &[uint8]) -> Vec<uint8> {
    debug_assert!(
        data.len() % 4 == 0,
        "binary_to_ascii requires 4-byte multiples"
    );

    let mut out = Vec::with_capacity(data.len() / 4 * 5 + 1);

    for chunk in data.chunks_exact(4) {
        let x0 = chunk[0] as uint32
            | ((chunk[1] as uint32) << 8)
            | ((chunk[2] as uint32) << 16)
            | ((chunk[3] as uint32) << 24);

        let x1 = x0 / 85;
        out.push(K_ENCODE_TABLE[(x0 - x1 * 85) as usize]);

        let x2 = x1 / 85;
        out.push(K_ENCODE_TABLE[(x1 - x2 * 85) as usize]);

        let x3 = x2 / 85;
        out.push(K_ENCODE_TABLE[(x2 - x3 * 85) as usize]);

        let x4 = x3 / 85;
        out.push(K_ENCODE_TABLE[(x3 - x4 * 85) as usize]);
        out.push(K_ENCODE_TABLE[x4 as usize]);
    }

    out
}

// ---------------------------------------------------------------------------
// dng_big_table base
// ---------------------------------------------------------------------------

/// Port of `dng_big_table` (abstract base).
///
/// C++ stores a cache pointer and uses a global refcounted cache; the Rust
/// port drops the cache (PORT-NOTE — the negative's dictionary serves the
/// same dedup role; cache extraction is a performance feature).
pub trait DngBigTable: std::fmt::Debug {
    /// Set the stored fingerprint (used by the XMP read path after a
    /// successful decode; C++ mutates fFingerprint in RecomputeFingerprint).
    fn fingerprint_set(&mut self, _fingerprint: DngFingerprint) {}

    /// `IsMissing ()`.
    fn is_missing(&self) -> bool {
        false
    }

    /// `SetMissing ()`.
    fn set_missing(&mut self) {}

    /// `IsEmbedNever ()` — `GetFlags () & 1`.
    fn is_embed_never(&self) -> bool {
        self.get_flags() & 1 != 0
    }

    /// `SetEmbedNever ()`.
    fn set_embed_never(&mut self) {
        let flags = self.get_flags() | 1;
        self.set_flags(flags);
    }

    /// `IsValid ()`.
    fn is_valid(&self) -> bool;

    /// `Fingerprint ()`.
    fn fingerprint(&self) -> DngFingerprint;

    /// `GetStream (stream)` — read the table payload from an uncompressed
    /// little-endian stream. Returns false on a format error (C++ catches
    /// and returns false).
    fn get_stream(&mut self, stream: &mut DngStream<'static>) -> DngResult<bool>;

    /// `PutStream (stream, forFingerprint)` — write the table payload.
    fn put_stream(&self, stream: &mut DngStream<'static>, for_fingerprint: bool) -> DngResult<()>;

    /// `GetFlags ()`.
    fn get_flags(&self) -> uint32;

    /// `SetFlags (flags)`.
    fn set_flags(&mut self, flags: uint32);

    /// `UseCompression ()` — default true.
    fn use_compression(&self) -> bool {
        true
    }

    /// `ComputeFingerprint ()` — MD5 of the little-endian payload stream.
    fn compute_fingerprint(&self) -> DngResult<DngFingerprint> {
        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        stream.set_little_endian(true);

        self.put_stream(&mut stream, true)?;

        let data = stream.as_memory_block()?.buffer().to_vec();

        let mut printer = DngMd5Printer::new();
        printer.process(&data);

        Ok(printer.result().clone())
    }

    /// `DecodeFromBinary (host, compressedData, compressedSize, cache)`.
    ///
    /// Decompresses (4-byte LE uncompressed size prefix + zlib stream), then
    /// reads the table via `get_stream`. Returns false on any format error.
    fn decode_from_binary(&mut self, compressed_data: &[uint8]) -> DngResult<bool> {
        if self.use_compression() {
            if compressed_data.len() < 5 {
                return Ok(false);
            }

            // Uncompressed size is stored in first four bytes, little
            // endian order.

            let uncompressed_size = (compressed_data[0] as uint32)
                | ((compressed_data[1] as uint32) << 8)
                | ((compressed_data[2] as uint32) << 16)
                | ((compressed_data[3] as uint32) << 24);

            match zlib::uncompress(&compressed_data[4..], uncompressed_size) {
                Ok(uncompressed) => {
                    let mut stream = DngStream::from_static(&uncompressed, 0);
                    stream.set_little_endian(true);

                    Ok(self.get_stream(&mut stream).unwrap_or(false))
                }
                Err(_) => Ok(false),
            }
        } else {
            let mut stream = DngStream::from_static(compressed_data, 0);
            stream.set_little_endian(true);

            Ok(self.get_stream(&mut stream).unwrap_or(false))
        }
        // PORT-NOTE: RecomputeFingerprint is caller-side here (the concrete
        // table recomputes after a successful decode).
    }

    /// `DecodeFromString (host, block1)` — ASCII-decode then binary-decode.
    fn decode_from_string(&mut self, block1: &[u8]) -> DngResult<bool> {
        let binary = ascii_to_binary(block1);
        self.decode_from_binary(&binary)
    }

    /// `EncodeAsBinary (allocator, compressedSize)` — 4-byte LE size prefix
    /// + zlib stream (Z_DEFAULT_COMPRESSION).
    fn encode_as_binary(&self) -> DngResult<DngMemoryBlock> {
        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        stream.set_little_endian(true);

        self.put_stream(&mut stream, false)?;

        let block1 = stream.as_memory_block()?.buffer().to_vec();

        if !self.use_compression() {
            let mut block = DngMemoryBlock::new(block1.len() as uint32)?;
            block.buffer_mut().copy_from_slice(&block1);
            return Ok(block);
        }

        let uncompressed_size = block1.len() as uint32;

        let compressed = zlib::compress(&block1, -1)?;

        let mut block = DngMemoryBlock::new((compressed.len() + 4) as uint32)?;

        block.buffer_mut()[0] = (uncompressed_size & 0xFF) as uint8;
        block.buffer_mut()[1] = ((uncompressed_size >> 8) & 0xFF) as uint8;
        block.buffer_mut()[2] = ((uncompressed_size >> 16) & 0xFF) as uint8;
        block.buffer_mut()[3] = ((uncompressed_size >> 24) & 0xFF) as uint8;

        block.buffer_mut()[4..].copy_from_slice(&compressed);

        Ok(block)
    }

    /// `EncodeAsString (allocator)` — binary-encode then Z85-like encode.
    /// The C++ zero-pads the final 4-byte group via the block's zero-filled
    /// physical buffer; we pad explicitly.
    fn encode_as_string(&self) -> DngResult<Vec<uint8>> {
        let block2 = self.encode_as_binary()?;

        let data = block2.buffer();
        let padded_len = (data.len() + 3) / 4 * 4;

        let mut padded = Vec::with_capacity(padded_len);
        padded.extend_from_slice(data);
        padded.resize(padded_len, 0);

        Ok(binary_to_ascii(&padded))
    }
}

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

/// Port of `dng_big_table_dictionary` — fingerprint → table-data map.
/// C++ uses `dng_ref_counted_block`; the Rust port uses `Vec<u8>`.
#[derive(Debug, Default, Clone)]
pub struct DngBigTableDictionary {
    map: std::collections::BTreeMap<DngFingerprint, Vec<uint8>>,
}

impl DngBigTableDictionary {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Map ()`.
    pub fn map(&self) -> &std::collections::BTreeMap<DngFingerprint, Vec<uint8>> {
        &self.map
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `HasTable (fingerprint)`.
    pub fn has_table(&self, fingerprint: &DngFingerprint) -> bool {
        self.map.contains_key(fingerprint)
    }

    /// `GetTable (fingerprint, block)`.
    pub fn get_table(&self, fingerprint: &DngFingerprint) -> Option<&[uint8]> {
        self.map.get(fingerprint).map(|v| v.as_slice())
    }

    /// `AddTable (fingerprint, block)`.
    pub fn add_table(&mut self, fingerprint: DngFingerprint, block: &[uint8]) {
        self.map.insert(fingerprint, block.to_vec());
    }

    /// `CopyToDictionary (dictionary)` (dng_big_table_accessor).
    pub fn copy_to_dictionary(&self, dictionary: &mut DngBigTableDictionary) {
        for (k, v) in &self.map {
            dictionary.add_table(*k, v);
        }
    }
}

/// `dng_big_table_index::IndexEntry`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BigTableIndexEntry {
    /// `fTableSize`.
    pub table_size: uint32,
    /// `fTableOffset`.
    pub table_offset: uint64,
}

/// Port of `dng_big_table_index` — fingerprint → (size, offset) map.
#[derive(Debug, Default, Clone)]
pub struct DngBigTableIndex {
    map: std::collections::BTreeMap<DngFingerprint, BigTableIndexEntry>,
}

impl DngBigTableIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `Map ()`.
    pub fn map(&self) -> &std::collections::BTreeMap<DngFingerprint, BigTableIndexEntry> {
        &self.map
    }

    /// `HasEntry (fingerprint)`.
    pub fn has_entry(&self, fingerprint: &DngFingerprint) -> bool {
        self.map.contains_key(fingerprint)
    }

    /// `GetEntry (fingerprint, tableSize, tableOffset)`.
    pub fn get_entry(&self, fingerprint: &DngFingerprint) -> Option<BigTableIndexEntry> {
        self.map.get(fingerprint).copied()
    }

    /// `AddEntry (fingerprint, tableSize, tableOffset)` — first entry wins.
    pub fn add_entry(
        &mut self,
        fingerprint: DngFingerprint,
        table_size: uint32,
        table_offset: uint64,
    ) {
        self.map
            .entry(fingerprint)
            .or_insert(BigTableIndexEntry {
                table_size,
                table_offset,
            });
    }
}

/// Port of `dng_big_table_group_index` — group digest → instance digest.
#[derive(Debug, Default, Clone)]
pub struct DngBigTableGroupIndex {
    map: std::collections::BTreeMap<DngFingerprint, DngFingerprint>,
}

impl DngBigTableGroupIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// `Map ()`.
    pub fn map(&self) -> &std::collections::BTreeMap<DngFingerprint, DngFingerprint> {
        &self.map
    }

    /// `HasEntry (groupDigest)`.
    pub fn has_entry(&self, group_digest: &DngFingerprint) -> bool {
        self.map.contains_key(group_digest)
    }

    /// `GetEntry (groupDigest, instanceDigest)`.
    pub fn get_entry(&self, group_digest: &DngFingerprint) -> Option<DngFingerprint> {
        self.map.get(group_digest).copied()
    }

    /// `AddEntry (groupDigest, instanceDigest)` — first entry wins.
    pub fn add_entry(&mut self, group_digest: DngFingerprint, instance_digest: DngFingerprint) {
        self.map.entry(group_digest).or_insert(instance_digest);
    }
}

/// Port of `dng_big_table_storage` — virtual sink/source for table data.
/// Default implementation does nothing (all lookups fail), matching the
/// C++ base.
#[derive(Debug, Default)]
pub struct DngBigTableStorage;

impl DngBigTableStorage {
    pub fn new() -> Self {
        Self
    }

    /// `ReadTable (table, fingerprint, allocator)`.
    pub fn read_table(
        &self,
        _table: &mut dyn DngBigTable,
        _fingerprint: &DngFingerprint,
    ) -> DngResult<bool> {
        Ok(false)
    }

    /// `WriteTable (table, fingerprint, allocator)`.
    pub fn write_table(
        &self,
        _table: &dyn DngBigTable,
        _fingerprint: &DngFingerprint,
    ) -> DngResult<bool> {
        Ok(false)
    }

    /// `MissingTable (fingerprint)`.
    pub fn missing_table(&self, _fingerprint: &DngFingerprint) {}

    /// `GroupToInstance (groupDigest, instanceDigest)`.
    pub fn group_to_instance(
        &self,
        _group_digest: &DngFingerprint,
        _instance_digest: &mut DngFingerprint,
    ) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// btbl box payload (used by dng_bmff; see dng_bmff_io::UpdateBigTables)
// ---------------------------------------------------------------------------

/// The `btbl` box tag name.
pub const BIG_TABLE_TAG_NAME: &[u8; 4] = b"btbl";

/// Build a `btbl` box payload: version (u32 BE) + fingerprint (16 bytes) +
/// table data. Version 1 layout per `dng_bmff_io::UpdateBigTables`.
pub fn make_btbl_payload(fingerprint: &DngFingerprint, table_data: &[uint8]) -> Vec<uint8> {
    let mut out = Vec::with_capacity(20 + table_data.len());

    out.extend_from_slice(&1u32.to_be_bytes()); // version 1
    out.extend_from_slice(&fingerprint.data);
    out.extend_from_slice(table_data);

    out
}

/// Parse a `btbl` box payload: returns `(version, fingerprint, table_data)`.
pub fn parse_btbl_payload(payload: &[uint8]) -> DngResult<(uint32, DngFingerprint, &[uint8])> {
    if payload.len() < 4 {
        return Err(bad_format());
    }

    let version = u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]);

    if version == 1 {
        if payload.len() <= 20 {
            return Err(bad_format());
        }

        let mut fingerprint = DngFingerprint::new();
        fingerprint.data.copy_from_slice(&payload[4..20]);

        Ok((version, fingerprint, &payload[20..]))
    } else {
        // Future versions: data layout unknown.
        Err(bad_format())
    }
}

// ---------------------------------------------------------------------------
// XMP glue (ReadTableFromXMP / ReadFromXMP / WriteToXMP)
// ---------------------------------------------------------------------------

/// Port of `dng_big_table::ReadFromXMP` — reads the digest + ASCII table
/// from XMP string properties, decodes and verifies the fingerprint.
pub fn read_table_from_xmp(
    table: &mut dyn DngBigTable,
    xmp: &dng_xmp::XmpMeta,
    ns: &str,
    digest_path: &str,
    data_path: &str,
) -> DngResult<bool> {
    // Read the digest identifying the table.

    let digest_str = match xmp.get_property(ns, digest_path) {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(false),
    };

    let fingerprint = DngFingerprint::from_hex(digest_str.as_bytes());

    if fingerprint.is_null() {
        return Ok(false);
    }

    // Read the ASCII-encoded table.

    let block1 = match xmp.get_property(ns, data_path) {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(false),
    };

    if table.decode_from_string(block1.as_bytes())? {
        // Verify the decoded fingerprint matches the announced digest.

        let computed = table.compute_fingerprint()?;

        if computed == fingerprint {
            table.fingerprint_set(computed);
            return Ok(true);
        }
    }

    Ok(false)
}

/// Port of `dng_big_table::WriteToXMP` — writes the digest and ASCII table.
pub fn write_table_to_xmp(
    table: &dyn DngBigTable,
    xmp: &mut dng_xmp::XmpMeta,
    ns: &str,
    digest_path: &str,
    data_path: &str,
) -> DngResult<()> {
    let fingerprint = table.fingerprint();

    let ascii = table.encode_as_string()?;

    let ascii_str = String::from_utf8(ascii).map_err(|_| program_error())?;

    xmp.set_property(ns, digest_path, &fingerprint.to_utf8_hex_string());
    xmp.set_property(ns, data_path, &ascii_str);

    Ok(())
}

// ---------------------------------------------------------------------------
// Re-exports (PGTM / ImageSequenceInfo live in dng-core)
// ---------------------------------------------------------------------------

pub use dng_core::dng_utils::{DngImageSequenceInfo, DngImageStats};

// The int32 import guard (used by some table subclasses in batch 2).
#[allow(unused_imports)]
use dng_core::dng_types::int32;

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal concrete table for testing the base codec: payload =
    /// 4-byte LE count + count u16 samples.
    #[derive(Debug, Default)]
    struct TestTable {
        samples: Vec<u16>,
        flags: uint32,
        fingerprint: DngFingerprint,
        compressed: bool,
    }

    impl TestTable {
        fn new(samples: &[u16]) -> Self {
            let mut t = TestTable {
                samples: samples.to_vec(),
                flags: 0,
                fingerprint: DngFingerprint::new(),
                compressed: true,
            };
            t.fingerprint = t.compute_fingerprint().unwrap();
            t
        }

        fn new_uncompressed(samples: &[u16]) -> Self {
            let mut t = Self::new(samples);
            t.compressed = false;
            t
        }
    }

    impl DngBigTable for TestTable {
        fn is_valid(&self) -> bool {
            true
        }

        fn fingerprint(&self) -> DngFingerprint {
            self.fingerprint
        }

        fn get_stream(&mut self, stream: &mut DngStream<'static>) -> DngResult<bool> {
            let count = stream.get_uint32()?;
            self.samples.clear();
            for _ in 0..count {
                self.samples.push(stream.get_uint16()?);
            }
            Ok(true)
        }

        fn put_stream(
            &self,
            stream: &mut DngStream<'static>,
            _for_fingerprint: bool,
        ) -> DngResult<()> {
            stream.put_uint32(self.samples.len() as uint32)?;
            for &s in &self.samples {
                stream.put_uint16(s)?;
            }
            Ok(())
        }

        fn get_flags(&self) -> uint32 {
            self.flags
        }

        fn set_flags(&mut self, flags: uint32) {
            self.flags = flags;
            self.fingerprint = self.compute_fingerprint().unwrap();
        }

        fn fingerprint_set(&mut self, fingerprint: DngFingerprint) {
            self.fingerprint = fingerprint;
        }

        fn use_compression(&self) -> bool {
            self.compressed
        }
    }

    #[test]
    fn ascii_binary_roundtrip() {
        for len in [4usize, 8, 12, 16, 20, 100, 256, 1024] {
            let data: Vec<uint8> = (0..len)
                .map(|i| ((i * 37 + 11) & 0xFF) as uint8)
                .collect();

            let ascii = binary_to_ascii(&data);
            let decoded = ascii_to_binary(&ascii);

            assert_eq!(decoded, data, "len {len}");
        }
    }

    #[test]
    fn ascii_decode_skips_invalid() {
        let mut ascii = binary_to_ascii(&[0xDE, 0xAD, 0xBE, 0xEF]);
        ascii.insert(0, b' '); // space is invalid → skipped
        ascii.push(b'\n'); // newline invalid → skipped

        let decoded = ascii_to_binary(&ascii);
        assert_eq!(decoded, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn binary_roundtrip_compressed() {
        let table = TestTable::new(&[1000, 2000, 3000, 4000, 65535]);

        let binary = table.encode_as_binary().unwrap();

        // First 4 bytes: LE uncompressed size = 4 + 5*2 = 14.
        let size = u32::from_le_bytes([
            binary.buffer()[0],
            binary.buffer()[1],
            binary.buffer()[2],
            binary.buffer()[3],
        ]);
        assert_eq!(size, 14);

        // Decompressed content matches the payload.
        let uncompressed = zlib::uncompress(&binary.buffer()[4..], size).unwrap();
        assert_eq!(uncompressed.len(), 14);

        let mut decoded = TestTable::new(&[]); // compressed decode target
        assert!(decoded.decode_from_binary(binary.buffer()).unwrap());
        assert_eq!(decoded.samples, table.samples);

        // Fingerprint recomputed after decode matches the source.
        decoded.fingerprint = decoded.compute_fingerprint().unwrap();
        assert_eq!(decoded.fingerprint, table.fingerprint);
    }

    #[test]
    fn binary_roundtrip_uncompressed() {
        let table = TestTable::new(&[7, 8, 9]);

        let raw_payload = {
            let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
            stream.set_little_endian(true);
            table.put_stream(&mut stream, false).unwrap();
            stream.as_memory_block().unwrap().buffer().to_vec()
        };

        let mut decoded = TestTable::new_uncompressed(&[]);
        assert!(decoded.decode_from_binary(&raw_payload).unwrap());
        assert_eq!(decoded.samples, vec![7, 8, 9]);
    }

    #[test]
    fn string_roundtrip() {
        let table = TestTable::new(&[1, 2, 3, 0xFFFF]);

        let ascii = table.encode_as_string().unwrap();

        // All characters must be printable and in the encode table (clean
        // for XMP).
        for &c in &ascii {
            assert!(c >= 32 && c <= 127);
        }

        let mut decoded = TestTable::new(&[]); // compressed decode target
        assert!(decoded.decode_from_string(&ascii).unwrap());
        assert_eq!(decoded.samples, table.samples);

        decoded.fingerprint = decoded.compute_fingerprint().unwrap();
        assert_eq!(decoded.fingerprint, table.fingerprint);
    }

    #[test]
    fn fingerprint_depends_on_payload() {
        let t1 = TestTable::new(&[1, 2, 3]);
        let t2 = TestTable::new(&[1, 2, 4]);
        let t1b = TestTable::new(&[1, 2, 3]);

        assert_ne!(t1.fingerprint(), t2.fingerprint());
        assert_eq!(t1.fingerprint(), t1b.fingerprint());
    }

    #[test]
    fn embed_never_flag() {
        let mut table = TestTable::new(&[5]);

        assert!(!table.is_embed_never());
        table.set_embed_never();
        assert!(table.is_embed_never());
        assert_eq!(table.get_flags() & 1, 1);
    }

    #[test]
    fn dictionary_operations() {
        let mut dict = DngBigTableDictionary::new();

        assert!(dict.is_empty());

        let fp1 = TestTable::new(&[1]).fingerprint;
        let fp2 = TestTable::new(&[2]).fingerprint;

        dict.add_table(fp1, &[0xAA, 0xBB]);
        dict.add_table(fp2, &[0xCC]);

        assert!(!dict.is_empty());
        assert!(dict.has_table(&fp1));
        assert!(!dict.has_table(&DngFingerprint::new()));
        assert_eq!(dict.get_table(&fp1), Some(&[0xAA, 0xBB][..]));
        assert_eq!(dict.get_table(&fp2), Some(&[0xCC][..]));
        assert_eq!(dict.map().len(), 2);

        // CopyToDictionary.
        let mut dict2 = DngBigTableDictionary::new();
        dict.copy_to_dictionary(&mut dict2);
        assert_eq!(dict2.map().len(), 2);
        assert_eq!(dict2.get_table(&fp1), Some(&[0xAA, 0xBB][..]));
    }

    #[test]
    fn index_and_group_index() {
        let mut index = DngBigTableIndex::new();
        assert!(index.is_empty());

        let fp = TestTable::new(&[9]).fingerprint;
        index.add_entry(fp, 123, 456);
        index.add_entry(fp, 999, 999); // first entry wins

        assert!(index.has_entry(&fp));
        assert_eq!(
            index.get_entry(&fp),
            Some(BigTableIndexEntry {
                table_size: 123,
                table_offset: 456
            })
        );
        assert!(!index.has_entry(&DngFingerprint::new()));

        let mut group = DngBigTableGroupIndex::new();
        assert!(group.is_empty());

        let group_digest = TestTable::new(&[11]).fingerprint;
        let instance_digest = TestTable::new(&[12]).fingerprint;

        group.add_entry(group_digest, instance_digest);
        group.add_entry(group_digest, DngFingerprint::new()); // first wins

        assert!(group.has_entry(&group_digest));
        assert_eq!(group.get_entry(&group_digest), Some(instance_digest));
    }

    #[test]
    fn btbl_payload_roundtrip() {
        let fp = TestTable::new(&[42]).fingerprint;
        let table_data = vec![1u8, 2, 3, 4, 5];

        let payload = make_btbl_payload(&fp, &table_data);

        let (version, parsed_fp, data) = parse_btbl_payload(&payload).unwrap();
        assert_eq!(version, 1);
        assert_eq!(parsed_fp, fp);
        assert_eq!(data, table_data);

        // Invalid version.
        let mut bad = payload.clone();
        bad[0] = 2;
        assert!(parse_btbl_payload(&bad).is_err());

        // Too short.
        assert!(parse_btbl_payload(&payload[..10]).is_err());
    }

    #[test]
    fn xmp_roundtrip() {
        dng_xmp::register_namespace("crbtbl", "http://ns.adobe.com/bigtable-test/1.0/");

        let ns = "http://ns.adobe.com/bigtable-test/1.0/";
        let table = TestTable::new(&[10, 20, 30]);

        let mut xmp = dng_xmp::XmpMeta::new();
        write_table_to_xmp(&table, &mut xmp, ns, "BigTableDigest", "BigTable").unwrap();

        // Digest property present.
        let digest_str = xmp.get_property(ns, "BigTableDigest").unwrap();
        assert_eq!(digest_str, table.fingerprint().to_utf8_hex_string());

        // Round-trip through serialize → parse.
        let bytes = xmp
            .serialize_to_buffer(&dng_xmp::XmpSerializeOptions::default())
            .unwrap();
        let reparsed = dng_xmp::XmpMeta::parse_from_buffer(&bytes).unwrap();

        let mut decoded = TestTable::new(&[]); // compressed decode target
        assert!(
            read_table_from_xmp(
                &mut decoded,
                &reparsed,
                ns,
                "BigTableDigest",
                "BigTable"
            )
            .unwrap()
        );

        assert_eq!(decoded.samples, table.samples);
        decoded.fingerprint = decoded.compute_fingerprint().unwrap();
        assert_eq!(decoded.fingerprint, table.fingerprint);
    }

    #[test]
    fn storage_defaults_fail() {
        let storage = DngBigTableStorage::new();
        let mut table = TestTable::new(&[1]);
        let fp = table.fingerprint;

        assert!(!storage.read_table(&mut table, &fp).unwrap());
        assert!(!storage.write_table(&table, &fp).unwrap());
        assert!(!storage.group_to_instance(&fp, &mut DngFingerprint::new()));
    }
}

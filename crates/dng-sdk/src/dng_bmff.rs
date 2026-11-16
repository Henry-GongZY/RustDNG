// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_bmff.cpp/.h`.
//
// Copyright 2006-2022 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_bmff.cpp` / `dng_bmff.h` — basic support for the Base
//! Media File Format (ISO BMFF): `dng_bmff_box`, `dng_bmff_box_list`,
//! `dng_bmff_io` with `Read` / `Write` / `UpdateBigTables`.
//!
//! `UpdateBigTables` manages `btbl` boxes (registered at mp4ra.org):
//! version (u32 BE) + MD5 fingerprint (16 bytes) + table data (version 1).

use dng_core::dng_exceptions::bad_format;
use dng_core::dng_fingerprint::DngFingerprint;
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_memory_stream::DngMemoryStream;
use dng_core::dng_stream::DngStream;
use dng_core::dng_types::{uint32, uint64, uint8};
use dng_core::{DngError, DngResult};

use crate::dng_big_table::{DngBigTableDictionary, BIG_TABLE_TAG_NAME};

/// Port of `dng_bmff_box`.
#[derive(Debug, Default)]
pub struct DngBmffBox {
    /// Byte length of the box as stored in the file:
    /// 0 = last box, 1 = large box, other = compact box length.
    pub stored_length: uint32,

    /// Real byte length of the box, if known (0 for a last box).
    pub real_length: uint64,

    /// Offset of the box (first byte of the length field) in the file.
    pub offset: uint64,

    /// 4-char box name.
    pub name: [uint8; 4],

    /// Box contents, excluding the 8-byte (compact/last) or 16-byte (large)
    /// header.
    pub content: Option<DngMemoryBlock>,
}

impl DngBmffBox {
    /// Box name as a string (lossy for non-UTF-8 names).
    pub fn name_str(&self) -> String {
        self.name.iter().map(|&b| b as char).collect()
    }
}

/// Port of `dng_bmff_io` — box list + Read/Write/UpdateBigTables.
///
/// The C++ class is a virtual base with `ShouldReadBox`; the Rust port
/// exposes the predicate as a settable closure (default: read everything).
#[derive(Default)]
pub struct DngBmffIo {
    /// `fBoxes`.
    pub boxes: Vec<Option<DngBmffBox>>,

    should_read_box: Option<Box<dyn Fn(&str, uint64) -> bool>>,
}

impl DngBmffIo {
    pub fn new() -> Self {
        Self::default()
    }

    /// `ShouldReadBox` predicate override (protected virtual in C++).
    pub fn set_should_read_box(&mut self, predicate: Box<dyn Fn(&str, uint64) -> bool>) {
        self.should_read_box = Some(predicate);
    }

    fn should_read_box(&self, name: &str, length: uint64) -> bool {
        match &self.should_read_box {
            Some(f) => f(name, length),
            None => true,
        }
    }

    /// `Add (box)` — appends non-null boxes.
    pub fn add(&mut self, box_: Option<DngBmffBox>) {
        if let Some(box_) = box_ {
            self.boxes.push(Some(box_));
        }
    }

    /// `Read (host, stream)` — parse all boxes from a BMFF stream.
    ///
    /// Requires the first box to be `ftyp` (or `JXL ` for JXL containers).
    pub fn read(&mut self, stream: &mut DngStream<'static>) -> DngResult<()> {
        stream.set_big_endian(true);
        stream.set_read_position(0)?;

        // Skip first 4 bytes (length field of first box).

        stream.skip(4)?;

        // Quick check to see if this looks like a valid ISO BMFF or JXL
        // file.

        let first_name = Self::read_name(stream)?;

        if first_name != "ftyp" && first_name != "JXL " {
            return Err(bad_format());
        }

        // Reset to reading.

        stream.set_read_position(0)?;

        let mut total_offset: uint64 = 0;

        let stream_length = stream.length();

        while stream.position() < stream_length {
            let stored_length = stream.get_uint32()?;

            let mut length = stored_length as uint64;

            let name = Self::read_name(stream)?;

            let header_offset: uint64;

            if stored_length == 1 {
                // Large box.

                length = stream.get_uint64()?;

                if length < 16 {
                    return Err(bad_format());
                }

                header_offset = 16;
            } else if stored_length == 0 {
                // Last box: extends to EOF.

                header_offset = 8;

                length = header_offset + (stream_length - stream.position());
            } else {
                // Compact box.

                if length < 8 {
                    return Err(bad_format());
                }

                header_offset = 8;
            }

            if length < header_offset {
                return Err(bad_format());
            }

            let content_length = length - header_offset;

            // Store the box data if desired.

            if self.should_read_box(&name, length) {
                // Currently do not support contents above 4 GB.

                if content_length > uint64::from(0xFFFFFFFFu32) {
                    return Err(bad_format());
                }

                let content_length32 = content_length as uint32;

                let content = stream.get(content_length32)?;

                let mut block = DngMemoryBlock::new(content_length32)?;
                block.buffer_mut().copy_from_slice(&content);

                let mut box_ = DngBmffBox {
                    stored_length,
                    real_length: length,
                    offset: total_offset,
                    name: [0; 4],
                    content: Some(block),
                };

                let name_bytes = name.as_bytes();
                box_.name.copy_from_slice(&name_bytes[..4.min(name_bytes.len())]);

                self.boxes.push(Some(box_));
            }
            // Skip the box.
            else {
                stream.skip(content_length)?;
            }

            // Update total offset.

            total_offset += length;
        }

        Ok(())
    }

    fn read_name(stream: &mut DngStream<'static>) -> DngResult<String> {
        let bytes = stream.get(4)?;
        Ok(bytes.iter().map(|&b| b as char).collect())
    }

    /// `Write (host, stream)` — write all boxes back out, honoring the
    /// stored length forms (large / compact / last).
    pub fn write(&self, stream: &mut DngStream<'static>) -> DngResult<()> {
        stream.set_big_endian(true);

        for box_ in &self.boxes {
            // Skip missing/invalid boxes.

            let Some(box_) = box_ else { continue };

            if box_.name.len() != 4 {
                return Err(bad_format());
            }

            let data_len = box_
                .content
                .as_ref()
                .map(|c| c.logical_size() as uint64)
                .unwrap_or(0);

            let use_large_size = box_.stored_length == 1 || data_len + 8 > 0xFFFFFFFF;

            if use_large_size {
                stream.put_uint32(1)?; // use large size

                stream.put(&box_.name)?;

                stream.put_uint64(data_len + 16)?;

                if let Some(content) = &box_.content {
                    if data_len > 0 {
                        stream.put(content.buffer())?;
                    }
                }
            } else {
                // Compact or last box.

                if box_.stored_length == 0 {
                    stream.put_uint32(0)?; // last box
                } else {
                    stream.put_uint32((data_len + 8) as uint32)?; // compact
                }

                stream.put(&box_.name)?;

                if let Some(content) = &box_.content {
                    if data_len > 0 {
                        stream.put(content.buffer())?;
                    }
                }
            }
        }

        stream.flush()?;

        Ok(())
    }

    /// `UpdateBigTables (host, newTables, deleteUnused)`.
    ///
    /// Step 1: find existing `btbl` boxes, dedup, and (optionally) mark
    /// unused ones for deletion. Step 2: append boxes for new tables.
    pub fn update_big_tables(
        &mut self,
        new_tables: &DngBigTableDictionary,
        delete_unused: bool,
    ) -> DngResult<()> {
        // Step 1: Find all existing tables in the file. Along the way, we
        // clean up duplicate tables and no-longer-used tables.

        let dict_map = new_tables.map();

        let mut digests: std::collections::BTreeMap<DngFingerprint, usize> =
            std::collections::BTreeMap::new();

        // Mark-for-delete via a pass that nulls entries (C++ `box.reset()`).

        let mut to_delete: Vec<usize> = Vec::new();

        for (index, box_opt) in self.boxes.iter().enumerate() {
            let Some(box_) = box_opt else { continue };

            if box_.name != *BIG_TABLE_TAG_NAME {
                continue;
            }

            let Some(content) = &box_.content else { continue };

            if content.logical_size() <= 20 {
                continue;
            }

            let (version, fingerprint, _data) =
                match crate::dng_big_table::parse_btbl_payload(content.buffer()) {
                    Ok(x) => x,
                    Err(_) => continue,
                };

            let _ = version;

            if fingerprint.is_valid() && !digests.contains_key(&fingerprint) {
                // Not seen yet.

                if !dict_map.contains_key(&fingerprint) {
                    // Not found in newTables, so we don't need this table.
                    // If requested to delete unused tables, then mark for
                    // delete.

                    if delete_unused {
                        to_delete.push(index);
                    }
                } else {
                    // Found in newTables, so we do need this table.
                    // Remember that we saw it.

                    digests.insert(fingerprint, index);
                }
            } else {
                // Already seen this table earlier in this loop (or invalid
                // digest), so this is a duplicate.

                to_delete.push(index);
            }
        }

        for index in to_delete {
            self.boxes[index] = None;
        }

        // Step 2: Write new tables.

        for (fingerprint, block) in dict_map {
            // Skip tables that we already have.

            if digests.contains_key(&fingerprint) {
                continue;
            }

            // Add this table.

            let mut mem_stream = DngStream::new(Box::new(DngMemoryStream::new()));
            mem_stream.set_big_endian(true);

            // Write version.

            mem_stream.put_uint32(1)?;

            // Write fingerprint.

            mem_stream.put(&fingerprint.data)?;

            // Write table data.

            mem_stream.put(block)?;

            let content = mem_stream.as_memory_block()?;

            let mut temp = DngBmffBox {
                stored_length: 0xFFFFFFFF,
                real_length: 0,
                offset: 0,
                name: *BIG_TABLE_TAG_NAME,
                content: Some(content),
            };

            let _ = &mut temp;

            self.boxes.push(Some(temp));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_compact(name: &[u8; 4], content: &[u8]) -> DngBmffBox {
        let mut block = DngMemoryBlock::new(content.len() as uint32).unwrap();
        block.buffer_mut().copy_from_slice(content);

        DngBmffBox {
            stored_length: (content.len() + 8) as uint32,
            real_length: (content.len() + 8) as uint64,
            offset: 0,
            name: *name,
            content: Some(block),
        }
    }

    fn build_stream(bytes: &[u8]) -> DngStream<'static> {
        DngStream::from_static(bytes, 0)
    }

    #[test]
    fn read_write_compact_boxes() {
        // Build: ftyp (compact) + junk (compact).
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"\x00\x00\x00\x00jxl ")));
        io.add(Some(box_compact(b"free", b"payload!")));

        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        io.write(&mut stream).unwrap();

        let bytes = stream.as_memory_block().unwrap().buffer().to_vec();

        // Sizes: 8+8=16, 8+8=16 → total 32.
        assert_eq!(bytes.len(), 32);
        assert_eq!(&bytes[4..8], b"ftyp");
        assert_eq!(&bytes[20..24], b"free"); // second box header after first's 16 bytes

        // Read back.
        let mut io2 = DngBmffIo::new();
        let mut read_stream = build_stream(&bytes);
        io2.read(&mut read_stream).unwrap();

        assert_eq!(io2.boxes.len(), 2);
        let b0 = io2.boxes[0].as_ref().unwrap();
        assert_eq!(b0.name_str(), "ftyp");
        assert_eq!(b0.stored_length, 16);
        assert_eq!(b0.real_length, 16);
        assert_eq!(b0.offset, 0);
        assert_eq!(b0.content.as_ref().unwrap().logical_size(), 8);

        let b1 = io2.boxes[1].as_ref().unwrap();
        assert_eq!(b1.name_str(), "free");
        assert_eq!(b1.offset, 16);
        assert_eq!(b1.content.as_ref().unwrap().buffer(), b"payload!");
    }

    #[test]
    fn read_requires_ftyp_or_jxl() {
        // mdat first → BadFormat.
        let bytes = [
            0, 0, 0, 12, b'm', b'd', b'a', b't', 1, 2, 3, 4,
        ];
        let mut io = DngBmffIo::new();
        let mut stream = build_stream(&bytes);
        assert!(io.read(&mut stream).is_err());
    }

    #[test]
    fn read_last_box_form() {
        // ftyp then a "last box" (stored length 0) extending to EOF.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"jxl ");

        bytes.extend_from_slice(&0u32.to_be_bytes()); // last box
        bytes.extend_from_slice(b"mdat");
        bytes.extend_from_slice(b"CONTENT");

        let mut io = DngBmffIo::new();
        let mut stream = build_stream(&bytes);
        io.read(&mut stream).unwrap();

        assert_eq!(io.boxes.len(), 2);
        let last = io.boxes[1].as_ref().unwrap();
        assert_eq!(last.stored_length, 0);
        assert_eq!(last.real_length, 8 + 7);
        assert_eq!(last.content.as_ref().unwrap().buffer(), b"CONTENT");
    }

    #[test]
    fn read_large_box_form() {
        // Large box: stored length 1, then u64 real length.
        let content = vec![7u8; 100];

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&12u32.to_be_bytes());
        bytes.extend_from_slice(b"ftyp");
        bytes.extend_from_slice(b"jxl ");

        bytes.extend_from_slice(&1u32.to_be_bytes()); // large size marker
        bytes.extend_from_slice(b"btbl");
        bytes.extend_from_slice(&(16 + content.len() as u64).to_be_bytes());
        bytes.extend_from_slice(&content);

        let mut io = DngBmffIo::new();
        let mut stream = build_stream(&bytes);
        io.read(&mut stream).unwrap();

        assert_eq!(io.boxes.len(), 2);
        let big = io.boxes[1].as_ref().unwrap();
        assert_eq!(big.stored_length, 1);
        assert_eq!(big.real_length, 116);
        assert_eq!(big.content.as_ref().unwrap().logical_size(), 100);
    }

    #[test]
    fn write_large_box_form() {
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"jxl ")));

        let mut content = DngMemoryBlock::new(3).unwrap();
        content.buffer_mut().copy_from_slice(&[1, 2, 3]);
        let mut big = DngBmffBox {
            stored_length: 1, // force large form
            real_length: 8 + 16 + 3,
            offset: 0,
            name: *b"btbl",
            content: Some(content),
        };
        io.add(Some(big));

        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        io.write(&mut stream).unwrap();

        let bytes = stream.as_memory_block().unwrap().buffer().to_vec();

        // First box compact (12 bytes: 8 + 4 "jxl "), second large:
        // marker(4) + name(4) + u64 length(8) + content(3) = 19 bytes.
        assert_eq!(bytes.len(), 12 + 19);
        assert_eq!(u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]), 1);
        assert_eq!(&bytes[16..20], b"btbl");
        assert_eq!(
            u64::from_be_bytes([
                bytes[20], bytes[21], bytes[22], bytes[23], bytes[24], bytes[25], bytes[26],
                bytes[27]
            ]),
            16 + 3
        );
        assert_eq!(&bytes[28..31], &[1, 2, 3]);
    }

    #[test]
    fn should_read_box_predicate() {
        // Build a 2-box stream where the second box is skipped by predicate.
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"jxl ")));
        io.add(Some(box_compact(b"free", b"skip me")));

        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        io.write(&mut stream).unwrap();
        let bytes = stream.as_memory_block().unwrap().buffer().to_vec();

        let mut io2 = DngBmffIo::new();
        io2.set_should_read_box(Box::new(|name, _len| name == "ftyp"));

        let mut read_stream = build_stream(&bytes);
        io2.read(&mut read_stream).unwrap();

        // Only ftyp is stored.
        assert_eq!(io2.boxes.len(), 1);
        assert_eq!(io2.boxes[0].as_ref().unwrap().name_str(), "ftyp");
    }

    #[test]
    fn update_big_tables_add_and_dedup() {
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"jxl ")));

        let mut dict = DngBigTableDictionary::new();
        dict.add_table(DngFingerprint::from_hex(b"00112233445566778899aabbccddeeff"), &[1, 2, 3]);

        io.update_big_tables(&dict, false).unwrap();

        // ftyp + one btbl box.
        assert_eq!(io.boxes.len(), 2);
        let btbl = io.boxes[1].as_ref().unwrap();
        assert_eq!(btbl.name_str(), "btbl");

        let (version, fp, data) =
            crate::dng_big_table::parse_btbl_payload(btbl.content.as_ref().unwrap().buffer())
                .unwrap();
        assert_eq!(version, 1);
        assert_eq!(fp, DngFingerprint::from_hex(b"00112233445566778899aabbccddeeff"));
        assert_eq!(data, &[1, 2, 3]);

        // Running update again with the same dictionary must NOT duplicate
        // the box (dedup via fingerprint).
        io.update_big_tables(&dict, false).unwrap();
        assert_eq!(io.boxes.len(), 2);
    }

    #[test]
    fn update_big_tables_delete_unused() {
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"jxl ")));

        // Pre-existing btbl with fingerprint AA.
        let fp_aa = DngFingerprint::from_hex(b"aa112233445566778899aabbccddeeff");
        let payload = crate::dng_big_table::make_btbl_payload(&fp_aa, &[0xDE, 0xAD]);
        io.add(Some(box_compact(b"btbl", &payload)));

        // Update with a dictionary that only contains BB: AA is unused →
        // deleted when delete_unused = true. BB appended.
        let mut dict = DngBigTableDictionary::new();
        let fp_bb = DngFingerprint::from_hex(b"bb112233445566778899aabbccddeeff");
        dict.add_table(fp_bb, &[0xBE, 0xEF]);

        io.update_big_tables(&dict, true).unwrap();

        // ftyp + btbl(AA, marked deleted → None) + btbl(BB).
        assert_eq!(io.boxes.len(), 3);
        assert!(io.boxes[1].is_none(), "unused table marked for delete");

        let bb = io.boxes[2].as_ref().unwrap();
        assert_eq!(bb.name_str(), "btbl");
        let (_, fp, _) =
            crate::dng_big_table::parse_btbl_payload(bb.content.as_ref().unwrap().buffer()).unwrap();
        assert_eq!(fp, fp_bb);

        // Write skips the None box.
        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        io.write(&mut stream).unwrap();
        let bytes = stream.as_memory_block().unwrap().buffer().to_vec();

        // ftyp (12) + btbl BB (8 + 4 + 16 + 2 = 30) → total 42; the
        // deleted (None) box is skipped.
        assert_eq!(bytes.len(), 42);
    }

    #[test]
    fn update_big_tables_keep_when_not_deleting() {
        let mut io = DngBmffIo::new();
        io.add(Some(box_compact(b"ftyp", b"jxl ")));

        let fp_aa = DngFingerprint::from_hex(b"aa112233445566778899aabbccddeeff");
        let payload = crate::dng_big_table::make_btbl_payload(&fp_aa, &[0xDE, 0xAD]);
        io.add(Some(box_compact(b"btbl", &payload)));

        // Empty dictionary, delete_unused = false → AA kept.
        let dict = DngBigTableDictionary::new();

        io.update_big_tables(&dict, false).unwrap();

        assert_eq!(io.boxes.len(), 2);
        assert!(io.boxes[1].is_some());
    }
}

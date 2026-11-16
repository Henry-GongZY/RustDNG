// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_parse_utils.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_parse_utils.h/.cpp — tag validation and tag-parsing helpers.
//
// PORT-NOTE(dng_parse_utils.h:19): the qDNGValidate-only Lookup*/Dump* table
// functions are debug/validation output and are omitted (qDNGValidate is 0 in
// this build, matching the non-validate C++ code paths); the functional
// Check*/Parse* helpers are ported in full.

use crate::dng_date_time::DngDateTime;
use crate::dng_errors::DngResult;
use crate::dng_matrix::{DngMatrix, DngVector};
use crate::dng_stream::DngStream;
use crate::dng_string::DngString;
use crate::dng_tag_types::*;
use crate::dng_sdk_limits::kMaxColorPlanes;
use crate::dng_tag_values::*;
use crate::dng_types::{int32, real64, uint16, uint32, uint64, uint8};

/// `CheckTagType (parentCode, tagCode, tagType, validType0..3)` — the Rust
/// form takes a slice of valid types.
pub fn check_tag_type(
    _parent_code: uint32,
    _tag_code: uint32,
    tag_type: uint32,
    valid_types: &[uint16],
) -> bool {
    valid_types.contains(&(tag_type as uint16))
}

/// `CheckTagCount (parentCode, tagCode, tagCount, minCount, maxCount)` —
/// maxCount == 0 means "exactly minCount".
pub fn check_tag_count(
    _parent_code: uint32,
    _tag_code: uint32,
    tag_count: uint32,
    min_count: uint32,
    max_count: uint32,
) -> bool {
    let mut max_count = max_count;
    if max_count < min_count {
        max_count = min_count;
    }
    tag_count >= min_count && tag_count <= max_count
}

/// `CheckColorImage (parentCode, tagCode, colorPlanes)`.
pub fn check_color_image(_parent_code: uint32, _tag_code: uint32, color_planes: uint32) -> bool {
    color_planes >= 3 && color_planes <= kMaxColorPlanes
}

/// `CheckMainIFD (parentCode, tagCode, newSubFileType)`.
pub fn check_main_ifd(_parent_code: uint32, _tag_code: uint32, new_sub_file_type: uint32) -> bool {
    new_sub_file_type == sfMainImage
}

/// `CheckMainOrEnhancedIFD`.
pub fn check_main_or_enhanced_ifd(
    _parent_code: uint32,
    _tag_code: uint32,
    new_sub_file_type: uint32,
) -> bool {
    new_sub_file_type == sfMainImage || new_sub_file_type == sfEnhancedImage
}

/// `CheckRawIFD`.
pub fn check_raw_ifd(
    _parent_code: uint32,
    _tag_code: uint32,
    photometric_interpretation: uint32,
) -> bool {
    photometric_interpretation == piLinearRaw || photometric_interpretation == piCFA
}

/// `CheckCFA`.
pub fn check_cfa(_parent_code: uint32, _tag_code: uint32, photometric_interpretation: uint32) -> bool {
    photometric_interpretation == piCFA
}

/// `ParseStringTag (stream, parentCode, tagCode, tagCount, s, trimBlanks)`.
pub fn parse_string_tag(
    stream: &mut DngStream,
    _parent_code: uint32,
    _tag_code: uint32,
    tag_count: uint32,
    s: &mut DngString,
    trim_blanks: bool,
) -> DngResult<()> {
    if tag_count == 0 || tag_count == 0xFFFF_FFFF {
        s.clear();
        return Ok(());
    }

    let mut buffer = stream.get(tag_count)?;
    buffer.push(0); // null terminator safety

    // Make sure the string is null terminated (trailing byte handling as C++).
    if tag_count > 0 && buffer[(tag_count - 1) as usize] != 0 {
        // C++ null-terminates and (in validate builds) warns.  The value part
        // still parses up to the first NUL via dng_string::Set.
    }

    // Truncate at the first NUL.
    let nul = buffer.iter().position(|&b| b == 0).unwrap_or(tag_count as usize);
    let mut value = buffer[..nul].to_vec();

    if trim_blanks {
        // Trim trailing blanks.
        while let Some(&last) = value.last() {
            if last == b' ' {
                value.pop();
            } else {
                break;
            }
        }
        // Trim leading blanks.
        let lead = value.iter().take_while(|&&b| b == b' ').count();
        value.drain(..lead);
    }

    s.set(&value);

    Ok(())
}

/// `ParseDualStringTag (stream, ..., s1, s2)` — two NUL-separated strings.
pub fn parse_dual_string_tag(
    stream: &mut DngStream,
    parent_code: uint32,
    tag_code: uint32,
    tag_count: uint32,
    s1: &mut DngString,
    s2: &mut DngString,
) -> DngResult<()> {
    s1.clear();
    s2.clear();

    if tag_count == 0 || tag_count == 0xFFFF_FFFF {
        return Ok(());
    }

    let mut buffer = stream.get(tag_count)?;
    buffer.push(0);

    // Parse s1.
    let mut split = 0usize;
    while split < tag_count as usize && buffer[split] != 0 {
        split += 1;
    }

    parse_string_tag_from(&buffer[..split], s1);

    // Parse s2.
    if split < tag_count as usize {
        let rest = &buffer[split + 1..tag_count as usize];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        parse_string_tag_from(&rest[..end], s2);
    }

    // C++ warns on malformed dual strings; value behavior is the same.
    let _ = (parent_code, tag_code);

    Ok(())
}

fn parse_string_tag_from(bytes: &[u8], s: &mut DngString) {
    let mut value = bytes.to_vec();
    while let Some(&last) = value.last() {
        if last == b' ' {
            value.pop();
        } else {
            break;
        }
    }
    s.set(&value);
}

/// `ParseEncodedStringTag (stream, ..., s)` — ASCII\0 or UNICODE\0 form.
pub fn parse_encoded_string_tag(
    stream: &mut DngStream,
    _parent_code: uint32,
    _tag_code: uint32,
    tag_count: uint32,
    s: &mut DngString,
) -> DngResult<()> {
    s.clear();

    if tag_count > 8 {
        let mut header = [0u8; 8];
        stream.get_into(&mut header, 0)?;

        if &header == b"UNICODE\0" {
            let count = ((tag_count - 8) >> 1) as usize;
            let mut u16s = Vec::with_capacity(count);
            for _ in 0..count {
                u16s.push(stream.get_uint16()?);
            }
            s.set_utf16(&u16s);
        } else {
            let mut rest = stream.get(tag_count - 8)?;
            rest.retain(|&b| b != 0);
            s.set(&rest);
        }
    } else if tag_count > 0 {
        // Skip the (invalid) data.
        stream.skip(tag_count as uint64)?;
    }

    Ok(())
}

/// `ParseMatrixTag (stream, ..., rows, cols, m)` — returns false on mismatch.
pub fn parse_matrix_tag(
    stream: &mut DngStream,
    parent_code: uint32,
    tag_code: uint32,
    tag_type: uint32,
    tag_count: uint32,
    rows: uint32,
    cols: uint32,
) -> DngResult<Option<DngMatrix>> {
    if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16, ttDouble as uint16]) {
        return Ok(None);
    }

    if !check_tag_count(parent_code, tag_code, tag_count, rows * cols, 0) {
        return Ok(None);
    }

    let mut m = DngMatrix::with_dims(rows, cols)?;

    for row in 0..rows as usize {
        for col in 0..cols as usize {
            let value: real64 = match tag_type {
                x if x == ttSRational => {
                    let r = stream.tag_value_srational(ttSRational)?;
                    r.as_real64()
                }
                x if x == ttDouble => stream.get_real64()?,
                _ => return Ok(None),
            };
            m.set(row, col, value);
        }
    }

    Ok(Some(m))
}

/// `ParseVectorTag (stream, ..., count, v)`.
pub fn parse_vector_tag(
    stream: &mut DngStream,
    parent_code: uint32,
    tag_code: uint32,
    tag_type: uint32,
    tag_count: uint32,
    count: uint32,
) -> DngResult<Option<DngVector>> {
    if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16, ttDouble as uint16]) {
        return Ok(None);
    }

    if !check_tag_count(parent_code, tag_code, tag_count, count, 0) {
        return Ok(None);
    }

    let mut v = DngVector::with_count(count)?;

    for j in 0..count as usize {
        let value: real64 = match tag_type {
            x if x == ttSRational => {
                let r = stream.tag_value_srational(ttSRational)?;
                r.as_real64()
            }
            x if x == ttDouble => stream.get_real64()?,
            _ => return Ok(None),
        };
        v.set(j, value);
    }

    Ok(Some(v))
}

/// `ParseDateTimeTag (stream, ..., dt)` — returns whether the date parsed.
pub fn parse_date_time_tag(
    stream: &mut DngStream,
    parent_code: uint32,
    tag_code: uint32,
    tag_type: uint32,
    tag_count: uint32,
    dt: &mut DngDateTime,
) -> DngResult<bool> {
    if !check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]) {
        return Ok(false);
    }

    // Kludge: PaintShop Pro writes length 21 instead of 20; accept both.
    let _ = check_tag_count(parent_code, tag_code, tag_count, 20, 0);

    if tag_count < 20 {
        return Ok(false);
    }

    let s = stream.get(20)?;

    // See if this is a valid date/time string.
    if dt.parse(&s) {
        return Ok(true);
    }

    // Accept strings that contain only blanks, colons, and zeros as valid
    // "null" dates.
    if s.iter().all(|&c| c == b' ' || c == b':' || c == b'0') {
        dt.clear();
        return Ok(true);
    }

    // Skip any remainder.
    if tag_count > 20 {
        stream.skip((tag_count - 20) as uint64)?;
    }

    Ok(false)
}

/// `ParseIPTCBlock`-style helper used by dng_iptc (phase 2); the block is
/// returned raw here.
pub fn parse_iptc_block(stream: &mut DngStream, tag_count: uint32) -> DngResult<Vec<uint8>> {
    // The C++ scans for the 0x1C (IPTC marker) lead byte.
    let data = stream.get(tag_count)?;
    let _ = find_iptc_start(&data);
    Ok(data)
}

/// Finds the 0x1C02/0x1C05-style IPTC records start.
pub fn find_iptc_start(data: &[uint8]) -> Option<usize> {
    for i in 0..data.len().saturating_sub(1) {
        if data[i] == 0x1C && (data[i + 1] == 0x02 || data[i + 1] == 0x05) {
            return Some(i);
        }
    }
    None
}

/// int32 clamp helper used by several tag paths.
pub fn clamp_int32(x: int32, min: int32, max: int32) -> int32 {
    crate::dng_utils::pin_int32(min, x, max)
}

#[allow(unused)]
fn unused_types(_: uint16, _: uint32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_memory_stream::DngMemoryStream;

    #[test]
    fn check_helpers() {
        assert!(check_tag_type(0, 0, ttShort as uint32, &[ttShort as uint16, ttLong as uint16]));
        assert!(!check_tag_type(0, 0, ttAscii as uint32, &[ttShort as uint16]));

        assert!(check_tag_count(0, 0, 3, 3, 0));
        assert!(check_tag_count(0, 0, 5, 2, 6));
        assert!(!check_tag_count(0, 0, 7, 2, 6));
        assert!(!check_tag_count(0, 0, 1, 3, 0));

        assert!(check_color_image(0, 0, 3));
        assert!(!check_color_image(0, 0, 2));
        assert!(!check_color_image(0, 0, 5));

        assert!(check_main_ifd(0, 0, sfMainImage));
        assert!(!check_main_ifd(0, 0, sfPreviewImage));

        assert!(check_cfa(0, 0, piCFA));
        assert!(!check_cfa(0, 0, piLinearRaw));
    }

    #[test]
    fn string_tag() {
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"Hello World\0").unwrap();
        stream.set_read_position(0).unwrap();

        let mut s = DngString::new();
        parse_string_tag(&mut stream, 0, 0, 12, &mut s, true).unwrap();
        assert_eq!(s.get(), b"Hello World");
    }

    #[test]
    fn string_tag_trim_blanks() {
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"  padded  \0").unwrap();
        stream.set_read_position(0).unwrap();

        let mut s = DngString::new();
        parse_string_tag(&mut stream, 0, 0, 11, &mut s, true).unwrap();
        assert_eq!(s.get(), b"padded");

        // Without trim.
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"  padded  \0").unwrap();
        stream.set_read_position(0).unwrap();
        let mut s2 = DngString::new();
        parse_string_tag(&mut stream, 0, 0, 11, &mut s2, false).unwrap();
        assert_eq!(s2.get(), b"  padded  ");
    }

    #[test]
    fn dual_string_tag() {
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"first\0second\0").unwrap();
        stream.set_read_position(0).unwrap();

        let mut s1 = DngString::new();
        let mut s2 = DngString::new();
        parse_dual_string_tag(&mut stream, 0, 0, 13, &mut s1, &mut s2).unwrap();
        assert_eq!(s1.get(), b"first");
        assert_eq!(s2.get(), b"second");
    }

    #[test]
    fn encoded_string_tag() {
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"ASCII\0\0\0").unwrap();
        stream.put(b"test text\0").unwrap();
        stream.set_read_position(0).unwrap();

        let mut s = DngString::new();
        parse_encoded_string_tag(&mut stream, 0, 0, 8 + 10, &mut s).unwrap();
        assert_eq!(s.get(), b"test text");

        // UNICODE form.
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"UNICODE\0").unwrap();
        stream.put_uint16(0x3042).unwrap(); // あ
        stream.set_read_position(0).unwrap();
        let mut s2 = DngString::new();
        parse_encoded_string_tag(&mut stream, 0, 0, 8 + 2, &mut s2).unwrap();
        assert_eq!(s2.get(), "あ".as_bytes());
    }

    #[test]
    fn matrix_tag() {
        let mut stream = DngMemoryStream::new().stream();
        stream.set_big_endian(true);
        // SRational 2x2: 1/2, 1/4, 1/8, 1/16
        for (n, d) in [(1, 2), (1, 4), (1, 8), (1, 16)] {
            stream.put_int32(n).unwrap();
            stream.put_int32(d).unwrap();
        }
        stream.set_read_position(0).unwrap();

        let m = parse_matrix_tag(
            &mut stream, 0, 0, ttSRational as uint32, 4, 2, 2,
        )
        .unwrap()
        .unwrap();

        assert_eq!(m.get(0, 0), 0.5);
        assert_eq!(m.get(0, 1), 0.25);
        assert_eq!(m.get(1, 0), 0.125);
        assert_eq!(m.get(1, 1), 0.0625);

        // Wrong type -> None.
        let mut stream = DngMemoryStream::new().stream();
        let none = parse_matrix_tag(&mut stream, 0, 0, ttFloat as uint32, 4, 2, 2).unwrap();
        assert!(none.is_none());
    }

    #[test]
    fn date_time_tag() {
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"2003:08:13 18:34:51\0").unwrap();
        stream.set_read_position(0).unwrap();

        let mut dt = DngDateTime::new();
        assert!(parse_date_time_tag(&mut stream, 0, 0, ttAscii as uint32, 20, &mut dt).unwrap());
        assert_eq!(dt.year, 2003);

        // Null date (all blanks/colons/zeros).
        let mut stream = DngMemoryStream::new().stream();
        stream.put(b"00000000000000000000").unwrap();
        stream.set_read_position(0).unwrap();
        let mut dt2 = DngDateTime::new();
        assert!(parse_date_time_tag(&mut stream, 0, 0, ttAscii as uint32, 20, &mut dt2).unwrap());
        assert!(dt2.not_valid());
    }

    #[test]
    fn iptc_start_scan() {
        let block = [0u8, 0x1C, 0x02, 0x05, 0xAB];
        assert_eq!(find_iptc_start(&block), Some(1));
        assert_eq!(find_iptc_start(&[0u8; 8]), None);
    }
}

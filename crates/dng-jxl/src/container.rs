// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1.
//
// Copyright 2005-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! JPEG XL BMFF container (ISO BMFF boxes) parse + build.
//!
//! Container signature (identical to the one the DNG SDK checks in
//! `ParseJXL_Container`): `00 00 00 0C 4A 58 4C 20 0D 0A 87 0A`
//! (size=12, brand `JXL `, minor version 0, compat `0D 0A 87 0A`).
//!
//! Box types handled (mirroring `dng_jxl.cpp` Encode/Decode):
//! - `ftyp` (brand `jxl `)
//! - `jxlp` (partial codestream, 4-byte index prefix) and `jxlc` (complete
//!   codestream)
//! - `Exif` (EXIF block, leading 4-byte TIFF header offset — the DNG SDK
//!   writes `kNumLeadingZeroBytesForEXIF = 4` zero bytes)
//! - `xml ` (XMP packet; the SDK also accepts uppercase `XML ` on read)
//! - `colr` (ICC profile: 4-byte signature method + data, method 1 = raw ICC)
//! - `brob` (brotli-compressed boxes — parsed but not decompressed here;
//!   Exif/XMP decompression goes through jxl-oxide's aux box reader)
//! - unknown boxes are skipped but preserved in the box list.

/// Container signature bytes.
pub const CONTAINER_SIG: [u8; 12] = [0, 0, 0, 0xc, b'J', b'X', b'L', b' ', 0xd, 0xa, 0x87, 0xa];

/// Bare codestream signature.
pub const CODESTREAM_SIG: [u8; 2] = [0xff, 0x0a];

/// A parsed container box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Box {
    pub ty: [u8; 4],
    pub content: Vec<u8>,
}

impl Box {
    pub fn ty_str(&self) -> String {
        self.ty.iter().map(|&b| b as char).collect()
    }
}

/// Is this a JXL container (BMFF) stream?
pub fn is_container(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes[..12] == CONTAINER_SIG
}

/// Is this a bare JXL codestream?
pub fn is_bare_codestream(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[..2] == CODESTREAM_SIG
}

/// Parse all top-level boxes of a container.
///
/// The codestream is reconstructed from `jxlp` (partial) and `jxlc`
/// (complete) boxes in order.
pub struct ParsedContainer {
    pub boxes: Vec<Box>,
    /// Concatenated codestream bytes (from jxlp/jxlc boxes).
    pub codestream: Vec<u8>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    /// ICC profile from a `colr` box (method 1), if present.
    pub icc: Option<Vec<u8>>,
}

pub fn parse_container(bytes: &[u8]) -> Result<ParsedContainer, crate::JxlError> {
    if !is_container(bytes) {
        return Err(crate::JxlError::Container("missing container signature".into()));
    }

    let mut boxes = Vec::new();
    let mut codestream = Vec::new();
    let mut exif = None;
    let mut xmp = None;
    let mut icc = None;

    let mut pos = 12usize; // skip the signature (which is the ftyp box itself)
    // The 12-byte signature IS a complete ftyp box (size 12).
    boxes.push(Box {
        ty: *b"ftyp",
        content: CONTAINER_SIG[8..12].to_vec(),
    });

    while pos + 8 <= bytes.len() {
        let size32 = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]);
        let ty: [u8; 4] = [
            bytes[pos + 4],
            bytes[pos + 5],
            bytes[pos + 6],
            bytes[pos + 7],
        ];

        let (header_len, size) = if size32 == 1 {
            // 64-bit largesize.
            if pos + 16 > bytes.len() {
                return Err(crate::JxlError::Container("truncated largesize header".into()));
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&bytes[pos + 8..pos + 16]);
            let size = u64::from_be_bytes(b);
            if size < 16 {
                return Err(crate::JxlError::Container("invalid largesize".into()));
            }
            (16usize, size as usize)
        } else if size32 == 0 {
            // Box extends to end of file.
            (8usize, bytes.len() - pos)
        } else {
            if (size32 as usize) < 8 {
                return Err(crate::JxlError::Container("invalid box size".into()));
            }
            (8usize, size32 as usize)
        };

        if pos + size > bytes.len() {
            return Err(crate::JxlError::Container(format!(
                "box '{}' extends past end of stream",
                ty.iter().map(|&b| b as char).collect::<String>()
            )));
        }

        let content = bytes[pos + header_len..pos + size].to_vec();
        let ty_str: [u8; 4] = ty;

        match ty_str {
            [b'j', b'x', b'l', b'p'] => {
                // Partial codestream: 4-byte (u32 BE) index prefix.
                if content.len() >= 4 {
                    codestream.extend_from_slice(&content[4..]);
                }
            }
            [b'j', b'x', b'l', b'c'] => {
                codestream.extend_from_slice(&content);
            }
            [b'E', b'x', b'i', b'f'] => {
                if exif.is_none() {
                    exif = Some(content.clone());
                }
            }
            [b'x', b'm', b'l', b' '] | [b'X', b'M', b'L', b' '] => {
                if xmp.is_none() {
                    xmp = Some(content.clone());
                }
            }
            [b'c', b'o', b'l', b'r'] => {
                // colr box: 1-byte method (1 = raw ICC), 3-byte reserved/zero,
                // then profile data. Per the JXL container spec the ICC
                // profile starts at offset 4.
                if content.len() > 4 && content[0] == 1 && icc.is_none() {
                    icc = Some(content[4..].to_vec());
                }
            }
            _ => {}
        }

        boxes.push(Box { ty: ty_str, content });
        pos += size;
    }

    Ok(ParsedContainer {
        boxes,
        codestream,
        exif,
        xmp,
        icc,
    })
}

/// Build a container from a codestream and optional metadata payloads.
///
/// Box order mirrors what libjxl's `JxlEncoderAddBox` + `dng_jxl.cpp`
/// produce: `ftyp`, metadata boxes (`Exif`, `xml `, `colr`) *before* the
/// codestream (libjxl writes boxes added via AddBox first, then the
/// codestream follows as `jxlc`).
pub fn build_container(
    codestream: &[u8],
    exif: Option<&[u8]>,
    xmp: Option<&[u8]>,
    icc: Option<&[u8]>,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(codestream.len() + 128);

    // ftyp: the 12-byte signature is exactly the ftyp box.
    out.extend_from_slice(&CONTAINER_SIG);

    // Exif (leading 4-byte offset field per JXL spec; the DNG SDK writes
    // zeros there).
    if let Some(exif) = exif {
        write_box(&mut out, b"Exif", |b| {
            b.extend_from_slice(&[0, 0, 0, 0]);
            b.extend_from_slice(exif);
        });
    }

    // XMP as `xml ` (lowercase; mirrors dng_jxl.cpp).
    if let Some(xmp) = xmp {
        write_box(&mut out, b"xml ", |b| {
            b.extend_from_slice(xmp);
        });
    }

    // ICC profile as `colr` box: method 1 (raw), 3 reserved bytes.
    if let Some(icc) = icc {
        write_box(&mut out, b"colr", |b| {
            b.push(1);
            b.extend_from_slice(&[0, 0, 0]);
            b.extend_from_slice(icc);
        });
    }

    // Codestream as a single jxlc box.
    write_box(&mut out, b"jxlc", |b| {
        b.extend_from_slice(codestream);
    });

    out
}

fn write_box(out: &mut Vec<u8>, ty: &[u8; 4], f: impl FnOnce(&mut Vec<u8>)) {
    let mut content = Vec::new();
    f(&mut content);
    let size = (8 + content.len()) as u32;
    out.extend_from_slice(&size.to_be_bytes());
    out.extend_from_slice(ty);
    out.extend_from_slice(&content);
}

/// Detect the stream form and return the raw codestream bytes plus, for
/// containers, the metadata boxes.
pub fn split_input(bytes: &[u8]) -> Result<ParsedInput, crate::JxlError> {
    if is_bare_codestream(bytes) {
        Ok(ParsedInput {
            codestream: bytes.to_vec(),
            exif: None,
            xmp: None,
            icc: None,
        })
    } else if is_container(bytes) {
        let c = parse_container(bytes)?;
        Ok(ParsedInput {
            codestream: c.codestream,
            exif: c.exif,
            xmp: c.xmp,
            icc: c.icc,
        })
    } else {
        Err(crate::JxlError::BadFormat(
            "neither JXL codestream nor container signature found".into(),
        ))
    }
}

/// Result of [`split_input`].
pub struct ParsedInput {
    pub codestream: Vec<u8>,
    pub exif: Option<Vec<u8>>,
    pub xmp: Option<Vec<u8>>,
    pub icc: Option<Vec<u8>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_roundtrip() {
        let cs = [0xffu8, 0x0a, 1, 2, 3, 4, 5];
        let exif = b"EXIFDATA".to_vec();
        let xmp = b"<x:xmpmeta/>".to_vec();
        let icc = vec![0xEEu8; 132];
        let bytes = build_container(&cs, Some(&exif), Some(&xmp), Some(&icc));

        assert!(is_container(&bytes));
        let parsed = parse_container(&bytes).unwrap();
        assert_eq!(parsed.codestream, cs);
        // The Exif box content carries the 4-byte offset prefix written by
        // build_container.
        assert_eq!(parsed.exif.unwrap(), [0u8; 4].into_iter().chain(exif).collect::<Vec<u8>>());
        assert_eq!(parsed.xmp.unwrap(), xmp);
        assert_eq!(parsed.icc.unwrap(), icc);
        // Box order: ftyp, Exif, xml, colr, jxlc (mirrors dng_jxl.cpp order).
        assert_eq!(parsed.boxes.len(), 5);
        assert_eq!(parsed.boxes[1].ty_str(), "Exif");
        assert_eq!(parsed.boxes[2].ty_str(), "xml ");
        assert_eq!(parsed.boxes[3].ty_str(), "colr");
        assert_eq!(parsed.boxes[4].ty_str(), "jxlc");
    }

    #[test]
    fn jxlp_reassembly() {
        // Two jxlp partial boxes must be concatenated (skipping their
        // 4-byte index prefixes; last box has high bit set on index).
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&CONTAINER_SIG);
        let parts: [Vec<u8>; 2] = [vec![0xA, 0xB], vec![0xC]];
        for (idx, part) in parts.into_iter().enumerate() {
            let index = if idx == 1 { 0x8000_0000u32 | idx as u32 } else { idx as u32 };
            let content: Vec<u8> = index
                .to_be_bytes()
                .into_iter()
                .chain(part.into_iter())
                .collect();
            let size = (8 + content.len()) as u32;
            bytes.extend_from_slice(&size.to_be_bytes());
            bytes.extend_from_slice(b"jxlp");
            bytes.extend_from_slice(&content);
        }
        let parsed = parse_container(&bytes).unwrap();
        assert_eq!(parsed.codestream, vec![0xA, 0xB, 0xC]);
    }

    #[test]
    fn signature_detection() {
        assert!(is_bare_codestream(&[0xff, 0x0a]));
        assert!(is_container(&CONTAINER_SIG));
        assert!(!is_container(&CODESTREAM_SIG));
        assert!(!is_bare_codestream(&CONTAINER_SIG));
        assert!(split_input(&[0x12, 0x34]).is_err());
    }
}

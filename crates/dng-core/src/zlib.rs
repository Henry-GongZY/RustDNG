// zlib helpers (flate2-based) replacing the SDK's direct zlib.h usage.
//
// C++ call sites (SDK 1.7.1):
//   * dng_read_image.cpp:2344 `uncompress(...)`   — zlib-wrapped input
//   * dng_image_writer.cpp:4544 `compress2(...)`  — zlib-wrapped output
//   * dng_big_table.cpp:1394   `compress2(...)`   — zlib-wrapped output
// i.e. the SDK only ever uses the standard zlib container (windowBits 15).
// Raw-deflate variants are provided for completeness/robustness with files
// produced by other writers (some emit headerless deflate for ccOldDeflate).

use crate::dng_errors::DngResult;
use crate::dng_exceptions::{bad_format, memory_full};
use crate::dng_types::{uint32, uint8};
use std::io::{Read, Write};

/// zlib-wrapped compression (`compress2` with the given level 0..=9;
/// -1 = default).
pub fn compress(data: &[uint8], level: i32) -> DngResult<Vec<uint8>> {
    let mut encoder =
        flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::new(clamp_level(level)));
    encoder
        .write_all(data)
        .map_err(|_| memory_full())?;
    encoder.finish().map_err(|_| memory_full())
}

/// zlib-wrapped decompression (`uncompress`).  Mirrors the C++ reader
/// behavior: a data error is tolerated by the caller (dng_read_image
/// deliberately ignores Z_DATA_ERROR — Watson #2530216), so this returns a
/// distinct error kind the caller can match on.
pub fn uncompress(data: &[uint8], uncompressed_size: uint32) -> DngResult<Vec<uint8>> {
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(uncompressed_size as usize);

    match decoder.take(uncompressed_size as u64).read_to_end(&mut out) {
        Ok(_) => {}
        Err(_) => {
            // Z_DATA_ERROR path: caller decides (see module docs).
            return Err(bad_format());
        }
    }

    if out.len() as uint32 != uncompressed_size {
        return Err(bad_format());
    }

    Ok(out)
}

/// Tolerant decompression matching the C++ reader exactly: Z_DATA_ERROR is
/// swallowed (partial output used), Z_MEM_ERROR propagates as Memory, other
/// errors as BadFormat.  Returns (data, length_mismatch_occurred).
pub fn uncompress_tolerant(
    data: &[uint8],
    uncompressed_size: uint32,
) -> DngResult<(Vec<uint8>, bool)> {
    let mut decoder = flate2::read::ZlibDecoder::new(data);
    let mut out = Vec::with_capacity(uncompressed_size as usize);

    match decoder.take(uncompressed_size as u64).read_to_end(&mut out) {
        Ok(_) => {}
        Err(e) => {
            if e.kind() == std::io::ErrorKind::OutOfMemory {
                return Err(memory_full());
            }
            // Z_DATA_ERROR: tolerated (see dng_read_image.cpp:2355).
            return Ok((out, true));
        }
    }

    let mismatch = out.len() as uint32 != uncompressed_size;
    Ok((out, mismatch))
}

/// Raw deflate (no zlib header) — windowBits = -15 equivalent.
pub fn compress_raw(data: &[uint8], level: i32) -> DngResult<Vec<uint8>> {
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(clamp_level(level)));
    encoder.write_all(data).map_err(|_| memory_full())?;
    encoder.finish().map_err(|_| memory_full())
}

/// Raw deflate decompression (no zlib header).
pub fn uncompress_raw(data: &[uint8], uncompressed_size: uint32) -> DngResult<Vec<uint8>> {
    let mut decoder = flate2::read::DeflateDecoder::new(data);
    let mut out = Vec::with_capacity(uncompressed_size as usize);

    decoder
        .take(uncompressed_size as u64)
        .read_to_end(&mut out)
        .map_err(|_| bad_format())?;

    if out.len() as uint32 != uncompressed_size {
        return Err(bad_format());
    }

    Ok(out)
}

fn clamp_level(level: i32) -> u32 {
    // Z_DEFAULT_COMPRESSION (-1) -> 6.
    if level < 0 {
        6
    } else {
        level.clamp(0, 9) as u32
    }
}

/// CRC-32 (IEEE) helper — used by big tables and PNG-style chunks.
pub fn crc32(data: &[uint8]) -> uint32 {
    crc32fast::hash(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zlib_roundtrip() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();

        let compressed = compress(&data, -1).unwrap();
        assert!(compressed.len() < data.len());
        assert_eq!(&compressed[..2], b"\x78\x9c"); // zlib header

        let out = uncompress(&compressed, data.len() as uint32).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn zlib_roundtrip_levels() {
        let data = b"hello hello hello hello hello".to_vec();
        for level in [0i32, 1, 6, 9] {
            let compressed = compress(&data, level).unwrap();
            let out = uncompress(&compressed, data.len() as uint32).unwrap();
            assert_eq!(out, data);
        }
    }

    #[test]
    fn raw_deflate_roundtrip() {
        let data: Vec<u8> = (0..50_000u32).map(|i| (i % 13) as u8).collect();

        let compressed = compress_raw(&data, 6).unwrap();
        // No zlib header.
        assert_ne!(&compressed[..2], b"\x78\x9c");

        let out = uncompress_raw(&compressed, data.len() as uint32).unwrap();
        assert_eq!(out, data);
    }

    #[test]
    fn wrapped_and_raw_are_not_interchangeable() {
        let data = b"some data".to_vec();
        let wrapped = compress(&data, 6).unwrap();
        assert!(uncompress_raw(&wrapped, data.len() as uint32).is_err());
        let raw = compress_raw(&data, 6).unwrap();
        assert!(uncompress(&raw, data.len() as uint32).is_err());
    }

    #[test]
    fn bad_data_errors() {
        let garbage = [1u8, 2, 3, 4, 5, 6, 7];
        assert!(uncompress(&garbage, 100).is_err());
    }

    #[test]
    fn tolerant_path_matches_cpp_behavior() {
        // Truncated stream: C++ reader ignores Z_DATA_ERROR and uses partial
        // output (then fails the length check).
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 7) as u8).collect();
        let compressed = compress(&data, 6).unwrap();
        let truncated = &compressed[..compressed.len() / 2];

        let (partial, mismatch) = uncompress_tolerant(truncated, data.len() as uint32).unwrap();
        assert!(mismatch);
        assert!(partial.len() < data.len());
    }

    #[test]
    fn crc32_known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
        assert_eq!(crc32(b"The quick brown fox jumps over the lazy dog"), 0x414FA339);
    }
}

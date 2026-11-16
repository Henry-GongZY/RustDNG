// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_types.cpp/.h`.
//
// Copyright 2006-2022 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_types.h — standard integer/float type aliases.
// The aliases keep the C++ names for 1:1 traceability with the reference code.

pub type int8 = i8;
pub type int16 = i16;
pub type int32 = i32;
pub type int64 = i64;

pub type uint8 = u8;
pub type uint16 = u16;
pub type uint32 = u32;
pub type uint64 = u64;

pub type uintptr = usize;

pub type real32 = f32;
pub type real64 = f64;

/// Build a Macintosh style four-character constant.
/// C++ macro `DNG_CHAR4(a,b,c,d)`; arguments are byte values (u8 works via `as uint32`).
pub const fn dng_char4(a: uint8, b: uint8, c: uint8, d: uint8) -> uint32 {
    ((a as uint32) << 24) | ((b as uint32) << 16) | ((c as uint32) << 8) | (d as uint32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_char4() {
        assert_eq!(dng_char4(b'A', b'B', b'C', b'D'), 0x41424344);
        assert_eq!(dng_char4(b' ', b' ', b' ', 0), 0x20202000);
    }
}

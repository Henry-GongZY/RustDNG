// Port of dng_tag_types.h/.cpp — TIFF tag type codes and element sizes.

use crate::dng_types::uint32;

pub const ttByte: uint32 = 1;
pub const ttAscii: uint32 = 2;
pub const ttShort: uint32 = 3;
pub const ttLong: uint32 = 4;
pub const ttRational: uint32 = 5;
pub const ttSByte: uint32 = 6;
pub const ttUndefined: uint32 = 7;
pub const ttSShort: uint32 = 8;
pub const ttSLong: uint32 = 9;
pub const ttSRational: uint32 = 10;
pub const ttFloat: uint32 = 11;
pub const ttDouble: uint32 = 12;
pub const ttIFD: uint32 = 13;
pub const ttUnicode: uint32 = 14;
pub const ttComplex: uint32 = 15;

// Tag types added by BigTIFF:

pub const ttLong8: uint32 = 16;
pub const ttSLong8: uint32 = 17;
pub const ttIFD8: uint32 = 18;

/// Note that this is not an official TIFF tag type, and should not be used
/// in TIFF/DNG files.
pub const ttHalfFloat: uint32 = 19;

/// `TagTypeSize` — element size in bytes for a tag type (0 for unknown).
pub fn tag_type_size(tag_type: uint32) -> uint32 {
    match tag_type {
        ttByte | ttAscii | ttSByte | ttUndefined => 1,
        ttShort | ttSShort | ttUnicode | ttHalfFloat => 2,
        ttLong | ttSLong | ttFloat | ttIFD => 4,
        ttRational | ttDouble | ttSRational | ttComplex | ttLong8 | ttSLong8 | ttIFD8 => 8,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_match_cpp() {
        assert_eq!(tag_type_size(ttByte), 1);
        assert_eq!(tag_type_size(ttAscii), 1);
        assert_eq!(tag_type_size(ttUndefined), 1);
        assert_eq!(tag_type_size(ttShort), 2);
        assert_eq!(tag_type_size(ttSShort), 2);
        assert_eq!(tag_type_size(ttUnicode), 2);
        assert_eq!(tag_type_size(ttHalfFloat), 2);
        assert_eq!(tag_type_size(ttLong), 4);
        assert_eq!(tag_type_size(ttSLong), 4);
        assert_eq!(tag_type_size(ttFloat), 4);
        assert_eq!(tag_type_size(ttIFD), 4);
        assert_eq!(tag_type_size(ttRational), 8);
        assert_eq!(tag_type_size(ttDouble), 8);
        assert_eq!(tag_type_size(ttSRational), 8);
        assert_eq!(tag_type_size(ttComplex), 8);
        assert_eq!(tag_type_size(ttLong8), 8);
        assert_eq!(tag_type_size(ttSLong8), 8);
        assert_eq!(tag_type_size(ttIFD8), 8);
        assert_eq!(tag_type_size(0), 0);
        assert_eq!(tag_type_size(99), 0);
    }
}

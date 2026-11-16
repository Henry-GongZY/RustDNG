// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_orientation.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_orientation.h/.cpp — orientation encoding and math.
//
// The internal "Adobe" encoding differs from the TIFF/EXIF encoding; both
// conversions are ported exactly.  `CalcForwardMatrix3by3/4by4` return the
// matrix directly (the C++ out-parameter + bool pattern becomes
// Option<DngMatrix3by3>).

use crate::dng_matrix::{DngMatrix3by3, DngMatrix4by4};
use crate::dng_types::uint32;

pub const kNormal: uint32 = 0;
pub const kRotate90CW: uint32 = 1;
pub const kRotate180: uint32 = 2;
pub const kRotate90CCW: uint32 = 3;
pub const kMirror: uint32 = 4;
pub const kMirror90CW: uint32 = 5;
pub const kMirror180: uint32 = 6;
pub const kMirror90CCW: uint32 = 7;
pub const kUnknown: uint32 = 8;

/// Port of `dng_orientation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngOrientation {
    adobe_orientation: uint32,
}

impl DngOrientation {
    pub fn new() -> Self {
        DngOrientation {
            adobe_orientation: kNormal,
        }
    }

    /// `SetAdobe (adobe)`.
    pub fn set_adobe(&mut self, adobe: uint32) {
        self.adobe_orientation = adobe;
    }

    /// `GetAdobe ()`.
    pub fn get_adobe(&self) -> uint32 {
        self.adobe_orientation
    }

    /// `SetTIFF (tiff)`.
    pub fn set_tiff(&mut self, tiff: uint32) {
        self.adobe_orientation = match tiff {
            1 => kNormal,
            2 => kMirror,
            3 => kRotate180,
            4 => kMirror180,
            5 => kMirror90CCW,
            6 => kRotate90CW,
            7 => kMirror90CW,
            8 => kRotate90CCW,
            9 => kUnknown,
            _ => kNormal,
        };
    }

    /// `GetTIFF ()`.
    pub fn get_tiff(&self) -> uint32 {
        match self.adobe_orientation {
            kNormal => 1,
            kMirror => 2,
            kRotate180 => 3,
            kMirror180 => 4,
            kMirror90CCW => 5,
            kRotate90CW => 6,
            kMirror90CW => 7,
            kRotate90CCW => 8,
            kUnknown => 9,
            _ => 1,
        }
    }

    /// `AdobeToDNG (adobe)`.
    pub fn adobe_to_dng(adobe: uint32) -> Self {
        let mut result = DngOrientation::new();
        result.set_adobe(adobe);
        result
    }

    /// `TIFFtoDNG (tiff)`.
    pub fn tiff_to_dng(tiff: uint32) -> Self {
        let mut result = DngOrientation::new();
        result.set_tiff(tiff);
        result
    }

    pub fn normal() -> Self {
        Self::adobe_to_dng(kNormal)
    }

    pub fn rotate90cw() -> Self {
        Self::adobe_to_dng(kRotate90CW)
    }

    pub fn rotate_180() -> Self {
        Self::adobe_to_dng(kRotate180)
    }

    pub fn rotate90ccw() -> Self {
        Self::adobe_to_dng(kRotate90CCW)
    }

    pub fn mirror() -> Self {
        Self::adobe_to_dng(kMirror)
    }

    pub fn mirror90cw() -> Self {
        Self::adobe_to_dng(kMirror90CW)
    }

    pub fn mirror_180() -> Self {
        Self::adobe_to_dng(kMirror180)
    }

    pub fn mirror90ccw() -> Self {
        Self::adobe_to_dng(kMirror90CCW)
    }

    pub fn unknown() -> Self {
        Self::adobe_to_dng(kUnknown)
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.adobe_orientation < kUnknown
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `FlipD ()`.
    pub fn flip_d(&self) -> bool {
        (self.adobe_orientation & 1) != 0
    }

    /// `FlipH ()`.
    pub fn flip_h(&self) -> bool {
        if (self.adobe_orientation & 4) != 0 {
            (self.adobe_orientation & 2) == 0
        } else {
            (self.adobe_orientation & 2) != 0
        }
    }

    /// `FlipV ()`.
    pub fn flip_v(&self) -> bool {
        if (self.adobe_orientation & 4) != 0 {
            self.flip_d() == self.flip_h()
        } else {
            self.flip_d() != self.flip_h()
        }
    }

    /// `IsMirrored ()`.
    pub fn is_mirrored(&self) -> bool {
        (self.adobe_orientation & 4) != 0
    }

    /// `operator-` (unary inverse).
    pub fn neg(&self) -> DngOrientation {
        let mut x = self.get_adobe();

        if (x & 5) == 5 {
            x ^= 2;
        }

        DngOrientation::adobe_to_dng(((4u32.wrapping_sub(x)) & 3) | (x & 4))
    }

    /// `operator+` — compose orientations (see C++ comment about
    /// non-commutativity).
    pub fn plus(&self, b: DngOrientation) -> DngOrientation {
        let mut x = self.get_adobe();
        let y = b.get_adobe();

        if (y & 4) != 0 {
            if (x & 1) != 0 {
                x ^= 6;
            } else {
                x ^= 4;
            }
        }

        DngOrientation::adobe_to_dng(((x.wrapping_add(y)) & 3) | (x & 4))
    }

    /// `operator-` — `self + (-b)`.
    pub fn minus(&self, b: DngOrientation) -> DngOrientation {
        self.plus(b.neg())
    }

    /// `CalcForwardMatrix3by3 (matrix, horizontalFirstRow)` — returns
    /// `Some(matrix)` when the orientation applies (hasOrient).
    pub fn calc_forward_matrix3by3(&self, horizontal_first_row: bool) -> Option<DngMatrix3by3> {
        let mut has_orient = false;

        let mut orientation_matrix = DngMatrix3by3::identity();

        if self.flip_h() {
            has_orient = true;

            if horizontal_first_row {
                orientation_matrix = DngMatrix3by3::new(-1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0)
                    .multiply_3by3(&orientation_matrix);
            } else {
                orientation_matrix = DngMatrix3by3::new(1.0, 0.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0)
                    .multiply_3by3(&orientation_matrix);
            }
        }

        if self.flip_v() {
            has_orient = true;

            if horizontal_first_row {
                orientation_matrix = DngMatrix3by3::new(1.0, 0.0, 0.0, 0.0, -1.0, 1.0, 0.0, 0.0, 1.0)
                    .multiply_3by3(&orientation_matrix);
            } else {
                orientation_matrix = DngMatrix3by3::new(-1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0)
                    .multiply_3by3(&orientation_matrix);
            }
        }

        if self.flip_d() {
            has_orient = true;

            orientation_matrix = DngMatrix3by3::new(0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0)
                .multiply_3by3(&orientation_matrix);
        }

        if has_orient {
            Some(orientation_matrix)
        } else {
            None
        }
    }

    /// `CalcForwardMatrix4by4 (matrix, horizontalFirstRow)`.
    pub fn calc_forward_matrix4by4(&self, horizontal_first_row: bool) -> Option<DngMatrix4by4> {
        let mut has_orient = false;

        let mut orientation_matrix = DngMatrix4by4::identity();

        if self.flip_h() {
            has_orient = true;

            // Note: the C++ assigns (not multiplies) in this branch; that
            // exact behavior is preserved.
            if horizontal_first_row {
                orientation_matrix = DngMatrix4by4::new(
                    -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                );
            } else {
                orientation_matrix = DngMatrix4by4::new(
                    1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                );
            }
        }

        if self.flip_v() {
            has_orient = true;

            if horizontal_first_row {
                orientation_matrix = DngMatrix4by4::new(
                    1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                )
                .multiply_4by4(&orientation_matrix);
            } else {
                orientation_matrix = DngMatrix4by4::new(
                    -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
                )
                .multiply_4by4(&orientation_matrix);
            }
        }

        if self.flip_d() {
            has_orient = true;

            orientation_matrix = DngMatrix4by4::new(
                0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            )
            .multiply_4by4(&orientation_matrix);
        }

        if has_orient {
            Some(orientation_matrix)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiff_roundtrip() {
        for tiff in 1..=9u32 {
            let o = DngOrientation::tiff_to_dng(tiff);
            assert_eq!(o.get_tiff(), tiff);
        }
        // Unknown values map to normal.
        let o = DngOrientation::tiff_to_dng(0);
        assert_eq!(o.get_tiff(), 1);
    }

    #[test]
    fn flips() {
        // Normal: no flips.
        let o = DngOrientation::normal();
        assert!(!o.flip_h() && !o.flip_v() && !o.flip_d());
        assert!(!o.is_mirrored());

        // Mirror: FlipH only.
        let o = DngOrientation::mirror();
        assert!(o.flip_h());
        assert!(!o.flip_v());
        assert!(!o.flip_d());
        assert!(o.is_mirrored());

        // Rotate180: FlipH + FlipV.
        let o = DngOrientation::rotate_180();
        assert!(o.flip_h() && o.flip_v() && !o.flip_d());

        // Rotate90CW: FlipD + FlipV (transpose then vertical flip).
        let o = DngOrientation::rotate90cw();
        assert!(!o.flip_h() && o.flip_v() && o.flip_d());

        // Mirror90CW: FlipD + FlipH.
        let o = DngOrientation::mirror90cw();
        assert!(o.flip_d() && o.flip_h());
    }

    #[test]
    fn composition_roundtrip() {
        // a + (-a) == normal for all orientations.
        for adobe in 0..8u32 {
            let a = DngOrientation::adobe_to_dng(adobe);
            let inv = a.neg();
            let sum = a.plus(inv);
            assert_eq!(sum, DngOrientation::normal(), "adobe={adobe}");
        }
    }

    #[test]
    fn compose_is_correct_order() {
        // Applying rotate90cw twice equals rotate180.
        let a = DngOrientation::rotate90cw();
        let b = a.plus(a);
        assert_eq!(b, DngOrientation::rotate_180());
    }

    #[test]
    fn forward_matrix_3by3() {
        // Normal -> None.
        assert!(DngOrientation::normal().calc_forward_matrix3by3(true).is_none());

        // Mirror -> [-1 0 1; 0 1 0; 0 0 1] (horizontal first row).
        let m = DngOrientation::mirror()
            .calc_forward_matrix3by3(true)
            .unwrap();
        assert_eq!(m.get(0, 0), -1.0);
        assert_eq!(m.get(0, 2), 1.0);
        assert_eq!(m.get(1, 1), 1.0);
    }
}

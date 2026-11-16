// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_rect.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_rect.h/.cpp — rectangle types and rectangle algebra.
//
// Operators are methods (`and_`/`or_`/`add_offset`/`sub_offset`) per
// ARCHITECTURE.md; free functions are also provided where the C++ had
// globals (Transpose, HalfRect, ...).  Overflow in the checked constructors
// panics (C++ throws a program error which is never caught locally).

use crate::dng_point::{dot, DngPoint, DngPointReal64};
use crate::dng_safe_arithmetic::{convert_uint32_to_int32, safe_int32_sub};
use crate::dng_types::{int32, real64, uint32};
use crate::dng_utils::{
    lerp_real64, max_int32, max_real64, min_int32, min_real64, round_int32_f64,
};

/// Port of `dng_rect` (t = top, l = left, b = bottom, r = right).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngRect {
    pub t: int32,
    pub l: int32,
    pub b: int32,
    pub r: int32,
}

impl DngRect {
    /// Empty rect (all zeros).
    pub fn new() -> Self {
        DngRect::default()
    }

    /// Constructs from top-left/bottom-right corners.  Panics on overflow
    /// (C++ ThrowProgramError, never caught locally).
    pub fn from_corners(tt: int32, ll: int32, bb: int32, rr: int32) -> Self {
        // Overflow check mirroring the C++ constructor.
        let _ = safe_int32_sub(rr, ll).expect("Overflow in dng_rect constructor");
        let _ = safe_int32_sub(bb, tt).expect("Overflow in dng_rect constructor");
        DngRect { t: tt, l: ll, b: bb, r: rr }
    }

    /// Constructs a rect of size (h, w) anchored at the origin.
    pub fn from_hw(h: uint32, w: uint32) -> Self {
        let b = convert_uint32_to_int32(h).expect("Overflow in dng_rect constructor");
        let r = convert_uint32_to_int32(w).expect("Overflow in dng_rect constructor");
        DngRect { t: 0, l: 0, b, r }
    }

    /// Constructs from a size point (v = height, h = width).
    pub fn from_size_point(size: DngPoint) -> Self {
        DngRect { t: 0, l: 0, b: size.v, r: size.h }
    }

    pub fn clear(&mut self) {
        *self = DngRect::new();
    }

    pub fn is_zero(&self) -> bool {
        self.t == 0 && self.l == 0 && self.b == 0 && self.r == 0
    }

    pub fn not_zero(&self) -> bool {
        !self.is_zero()
    }

    pub fn is_empty(&self) -> bool {
        self.t >= self.b || self.l >= self.r
    }

    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    pub fn contains_rect(&self, rect: DngRect) -> bool {
        rect.is_empty()
            || (self.t <= rect.t
                && self.l <= rect.l
                && self.b >= rect.b
                && self.r >= rect.r)
    }

    pub fn does_not_contain_rect(&self, rect: DngRect) -> bool {
        !self.contains_rect(rect)
    }

    pub fn contains_point(&self, pt: DngPoint) -> bool {
        self.t <= pt.v && self.l <= pt.h && self.b > pt.v && self.r > pt.h
    }

    pub fn does_not_contain_point(&self, pt: DngPoint) -> bool {
        !self.contains_point(pt)
    }

    /// `W ()` — width, or 0 if r < l.  Panics on overflow (C++ throws).
    pub fn w(&self) -> uint32 {
        if self.r >= self.l {
            let width = safe_int32_sub(self.r, self.l)
                .expect("Overflow computing rectangle width");
            width as uint32
        } else {
            0
        }
    }

    /// `H ()` — height, or 0 if b < t.  Panics on overflow (C++ throws).
    pub fn h(&self) -> uint32 {
        if self.b >= self.t {
            let height = safe_int32_sub(self.b, self.t)
                .expect("Overflow computing rectangle height");
            height as uint32
        } else {
            0
        }
    }

    pub fn tl(&self) -> DngPoint {
        DngPoint::new(self.t, self.l)
    }

    pub fn tr(&self) -> DngPoint {
        DngPoint::new(self.t, self.r)
    }

    pub fn bl(&self) -> DngPoint {
        DngPoint::new(self.b, self.l)
    }

    pub fn br(&self) -> DngPoint {
        DngPoint::new(self.b, self.r)
    }

    /// `Size ()` — point (H, W).
    pub fn size(&self) -> DngPoint {
        DngPoint::new(self.h() as int32, self.w() as int32)
    }

    pub fn long_side(&self) -> uint32 {
        max_uint32(self.w(), self.h())
    }

    pub fn short_side(&self) -> uint32 {
        min_uint32(self.w(), self.h())
    }

    pub fn diagonal(&self) -> real64 {
        (self.w() as f64).hypot(self.h() as f64)
    }

    /// `operator&` — intersection (empty result collapses to the zero rect).
    pub fn and_(&self, other: DngRect) -> DngRect {
        let mut c = DngRect {
            t: max_int32(self.t, other.t),
            l: max_int32(self.l, other.l),
            b: min_int32(self.b, other.b),
            r: min_int32(self.r, other.r),
        };
        if c.is_empty() {
            c = DngRect::new();
        }
        c
    }

    /// `operator|` — union.
    pub fn or_(&self, other: DngRect) -> DngRect {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        DngRect {
            t: min_int32(self.t, other.t),
            l: min_int32(self.l, other.l),
            b: max_int32(self.b, other.b),
            r: max_int32(self.r, other.r),
        }
    }

    /// `operator+ (rect, point)` — translate.
    pub fn add_offset(&self, pt: DngPoint) -> DngRect {
        DngRect {
            t: self.t + pt.v,
            l: self.l + pt.h,
            b: self.b + pt.v,
            r: self.r + pt.h,
        }
    }

    /// `operator- (rect, point)` — translate.
    pub fn sub_offset(&self, pt: DngPoint) -> DngRect {
        DngRect {
            t: self.t - pt.v,
            l: self.l - pt.h,
            b: self.b - pt.v,
            r: self.r - pt.h,
        }
    }
}

fn max_uint32(a: uint32, b: uint32) -> uint32 {
    if a >= b { a } else { b }
}
fn min_uint32(a: uint32, b: uint32) -> uint32 {
    if a <= b { a } else { b }
}

/// Port of `dng_rect_real64`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DngRectReal64 {
    pub t: real64,
    pub l: real64,
    pub b: real64,
    pub r: real64,
}

impl DngRectReal64 {
    pub fn new() -> Self {
        DngRectReal64::default()
    }

    pub fn from_tlr_b(rr: real64, ll: real64, bb: real64, rright: real64) -> Self {
        DngRectReal64 { t: rr, l: ll, b: bb, r: rright }
    }

    /// `dng_rect_real64 (h, w)`.
    pub fn from_hw(h: real64, w: real64) -> Self {
        DngRectReal64 { t: 0.0, l: 0.0, b: h, r: w }
    }

    pub fn from_size_point(size: DngPointReal64) -> Self {
        DngRectReal64 { t: 0.0, l: 0.0, b: size.v, r: size.h }
    }

    pub fn from_points(pt1: DngPointReal64, pt2: DngPointReal64) -> Self {
        DngRectReal64 {
            t: min_real64(pt1.v, pt2.v),
            l: min_real64(pt1.h, pt2.h),
            b: max_real64(pt1.v, pt2.v),
            r: max_real64(pt1.h, pt2.h),
        }
    }

    pub fn from_rect(rect: DngRect) -> Self {
        DngRectReal64 {
            t: rect.t as real64,
            l: rect.l as real64,
            b: rect.b as real64,
            r: rect.r as real64,
        }
    }

    pub fn clear(&mut self) {
        *self = DngRectReal64::default();
    }

    pub fn is_zero(&self) -> bool {
        self.t == 0.0 && self.l == 0.0 && self.b == 0.0 && self.r == 0.0
    }

    pub fn not_zero(&self) -> bool {
        !self.is_zero()
    }

    pub fn is_empty(&self) -> bool {
        self.t >= self.b || self.l >= self.r
    }

    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    pub fn contains_rect(&self, rect: DngRectReal64) -> bool {
        rect.is_empty()
            || (self.t <= rect.t
                && self.l <= rect.l
                && self.b >= rect.b
                && self.r >= rect.r)
    }

    pub fn contains_point(&self, pt: DngPointReal64) -> bool {
        self.t <= pt.v && self.l <= pt.h && self.b > pt.v && self.r > pt.h
    }

    pub fn w(&self) -> real64 {
        max_real64(self.r - self.l, 0.0)
    }

    pub fn h(&self) -> real64 {
        max_real64(self.b - self.t, 0.0)
    }

    pub fn tl(&self) -> DngPointReal64 {
        DngPointReal64::new(self.t, self.l)
    }

    pub fn tr(&self) -> DngPointReal64 {
        DngPointReal64::new(self.t, self.r)
    }

    pub fn bl(&self) -> DngPointReal64 {
        DngPointReal64::new(self.b, self.l)
    }

    pub fn br(&self) -> DngPointReal64 {
        DngPointReal64::new(self.b, self.r)
    }

    pub fn size(&self) -> DngPointReal64 {
        DngPointReal64::new(self.h(), self.w())
    }

    /// `Round ()`.
    pub fn round(&self) -> DngRect {
        DngRect::from_corners(
            round_int32_f64(self.t),
            round_int32_f64(self.l),
            round_int32_f64(self.b),
            round_int32_f64(self.r),
        )
    }

    pub fn long_side(&self) -> real64 {
        max_real64(self.w(), self.h())
    }

    pub fn short_side(&self) -> real64 {
        min_real64(self.w(), self.h())
    }

    pub fn diagonal(&self) -> real64 {
        self.w().hypot(self.h())
    }

    pub fn center(&self) -> DngPointReal64 {
        DngPointReal64::new((self.t + self.b) * 0.5, (self.l + self.r) * 0.5)
    }

    /// `operator&` — intersection.
    pub fn and_(&self, other: DngRectReal64) -> DngRectReal64 {
        let mut c = DngRectReal64 {
            t: max_real64(self.t, other.t),
            l: max_real64(self.l, other.l),
            b: min_real64(self.b, other.b),
            r: min_real64(self.r, other.r),
        };
        if c.is_empty() {
            c = DngRectReal64::new();
        }
        c
    }

    /// `operator|` — union.
    pub fn or_(&self, other: DngRectReal64) -> DngRectReal64 {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return *self;
        }
        DngRectReal64 {
            t: min_real64(self.t, other.t),
            l: min_real64(self.l, other.l),
            b: max_real64(self.b, other.b),
            r: max_real64(self.r, other.r),
        }
    }

    /// `operator+ (rect, point)`.
    pub fn add_offset(&self, pt: DngPointReal64) -> DngRectReal64 {
        DngRectReal64 {
            t: self.t + pt.v,
            l: self.l + pt.h,
            b: self.b + pt.v,
            r: self.r + pt.h,
        }
    }

    /// `operator- (rect, point)`.
    pub fn sub_offset(&self, pt: DngPointReal64) -> DngRectReal64 {
        DngRectReal64 {
            t: self.t - pt.v,
            l: self.l - pt.h,
            b: self.b - pt.v,
            r: self.r - pt.h,
        }
    }
}

/// `operator&` free function.
pub fn intersect_rect(a: DngRect, b: DngRect) -> DngRect {
    a.and_(b)
}

/// `operator|` free function.
pub fn union_rect(a: DngRect, b: DngRect) -> DngRect {
    a.or_(b)
}

/// `Transpose (a)`.
pub fn transpose_rect(a: DngRect) -> DngRect {
    DngRect { t: a.l, l: a.t, b: a.r, r: a.b }
}

/// `Transpose (a)`.
pub fn transpose_rect_real64(a: DngRectReal64) -> DngRectReal64 {
    DngRectReal64 { t: a.l, l: a.t, b: a.r, r: a.b }
}

/// `HalfRect (rect)` — in place.
pub fn half_rect(rect: &mut DngRect) {
    rect.r = rect.l + (rect.w() >> 1) as int32;
    rect.b = rect.t + (rect.h() >> 1) as int32;
}

/// `DoubleRect (rect)` — in place.
pub fn double_rect(rect: &mut DngRect) {
    rect.r = rect.l + ((rect.w() << 1) as int32);
    rect.b = rect.t + ((rect.h() << 1) as int32);
}

/// `InnerPadRect (rect, pad)` — in place.
pub fn inner_pad_rect(rect: &mut DngRect, pad: int32) {
    rect.l += pad;
    rect.r -= pad;
    rect.t += pad;
    rect.b -= pad;
}

/// `OuterPadRect (rect, pad)` — in place.
pub fn outer_pad_rect(rect: &mut DngRect, pad: int32) {
    inner_pad_rect(rect, -pad);
}

/// `InnerPadRectH (rect, pad)` — in place.
pub fn inner_pad_rect_h(rect: &mut DngRect, pad: int32) {
    rect.l += pad;
    rect.r -= pad;
}

/// `InnerPadRectV (rect, pad)` — in place.
pub fn inner_pad_rect_v(rect: &mut DngRect, pad: int32) {
    rect.t += pad;
    rect.b -= pad;
}

/// `MakeHalfRect (rect)`.
pub fn make_half_rect(rect: DngRect) -> DngRect {
    let mut out = rect;
    half_rect(&mut out);
    out
}

/// `MakeDoubleRect (rect)`.
pub fn make_double_rect(rect: DngRect) -> DngRect {
    let mut out = rect;
    double_rect(&mut out);
    out
}

/// `MakeInnerPadRect (rect, pad)`.
pub fn make_inner_pad_rect(rect: DngRect, pad: int32) -> DngRect {
    let mut out = rect;
    inner_pad_rect(&mut out, pad);
    out
}

/// `MakeInnerPadRect` for real rects.
pub fn make_inner_pad_rect_real64(rect: DngRectReal64, pad: real64) -> DngRectReal64 {
    let mut result = rect;
    result.t += pad;
    result.l += pad;
    result.b -= pad;
    result.r -= pad;
    result
}

/// `MakeOuterPadRect (rect, pad)`.
pub fn make_outer_pad_rect(rect: DngRect, pad: int32) -> DngRect {
    let mut out = rect;
    outer_pad_rect(&mut out, pad);
    out
}

/// `MakeOuterPadRect` for real rects.
pub fn make_outer_pad_rect_real64(rect: DngRectReal64, pad: real64) -> DngRectReal64 {
    DngRectReal64 {
        t: rect.t - pad,
        l: rect.l - pad,
        b: rect.b + pad,
        r: rect.r + pad,
    }
}

/// `Lerp (a, b, t)` for real rects.
pub fn lerp_rect_real64(a: DngRectReal64, b: DngRectReal64, t: real64) -> DngRectReal64 {
    DngRectReal64 {
        t: lerp_real64(a.t, b.t, t),
        l: lerp_real64(a.l, b.l, t),
        b: lerp_real64(a.b, b.b, t),
        r: lerp_real64(a.r, b.r, t),
    }
}

/// `Bounds (a, b, c, d)` — axis-aligned bounds of four points.
pub fn bounds_of_points(
    a: DngPointReal64,
    b: DngPointReal64,
    c: DngPointReal64,
    d: DngPointReal64,
) -> DngRectReal64 {
    let x_min = min_real64(a.h, min_real64(b.h, min_real64(c.h, d.h)));
    let x_max = max_real64(a.h, max_real64(b.h, max_real64(c.h, d.h)));
    let y_min = min_real64(a.v, min_real64(b.v, min_real64(c.v, d.v)));
    let y_max = max_real64(a.v, max_real64(b.v, max_real64(c.v, d.v)));
    DngRectReal64 { t: y_min, l: x_min, b: y_max, r: x_max }
}

/// Port of `dng_oriented_bounding_box` — an oriented bounding box (a
/// dng_rect_real64 with rotation).
#[derive(Debug, Clone, Copy, Default)]
pub struct DngOrientedBoundingBox {
    /// The center of the OBB.
    pub center: DngPointReal64,
    /// The two normal vectors (perpendicular; length = half the edge length).
    pub vec1: DngPointReal64,
    pub vec2: DngPointReal64,
}

impl DngOrientedBoundingBox {
    /// Create an empty, invalid OBB.
    pub fn new() -> Self {
        DngOrientedBoundingBox::default()
    }

    /// Create an OBB from an axis-aligned bounding box (AABB).
    pub fn from_rect(rect: DngRectReal64) -> Self {
        let center = rect.center();
        let vec1 = crate::dng_point::scale_point(
            0.5,
            rect.tr().add(rect.br()),
        )
        .sub(center);
        let vec2 = crate::dng_point::scale_point(
            0.5,
            rect.bl().add(rect.br()),
        )
        .sub(center);
        DngOrientedBoundingBox { center, vec1, vec2 }
    }

    /// Create an OBB from four corners.
    pub fn from_corners(
        tl: DngPointReal64,
        tr: DngPointReal64,
        bl: DngPointReal64,
        br: DngPointReal64,
    ) -> Self {
        let center = crate::dng_point::scale_point(
            0.25,
            tl.add(tr).add(bl).add(br),
        );
        let vec1 = crate::dng_point::scale_point(0.5, tr.add(br)).sub(center);
        let vec2 = crate::dng_point::scale_point(0.5, bl.add(br)).sub(center);
        DngOrientedBoundingBox { center, vec1, vec2 }
    }

    /// `CalcCorners` — returns (tl, tr, bl, br).
    pub fn calc_corners(&self) -> (DngPointReal64, DngPointReal64, DngPointReal64, DngPointReal64) {
        let tl = self.center.sub(self.vec1).sub(self.vec2);
        let tr = self.center.add(self.vec1).sub(self.vec2);
        let bl = self.center.sub(self.vec1).add(self.vec2);
        let br = self.center.add(self.vec1).add(self.vec2);
        (tl, tr, bl, br)
    }
}

/// `Intersect (aBox, bBox)` — separating axis theorem, 4 axes in 2D.
pub fn intersect_obb(a_box: &DngOrientedBoundingBox, b_box: &DngOrientedBoundingBox) -> bool {
    let a = a_box.center;
    let b = b_box.center;
    let a1 = a_box.vec1;
    let a2 = a_box.vec2;
    let b1 = b_box.vec1;
    let b2 = b_box.vec2;

    // Case 1: project to a1.
    {
        let a1_radius_a = dot(a1, a1);
        let a1_radius_b = dot(b1, a1).abs() + dot(b2, a1).abs();
        let a1_dist = dot(b.sub(a), a1).abs();
        if a1_dist > a1_radius_a + a1_radius_b {
            return false;
        }
    }

    // Case 2: project to a2.
    {
        let a2_radius_a = dot(a2, a2);
        let a2_radius_b = dot(b1, a2).abs() + dot(b2, a2).abs();
        let a2_dist = dot(b.sub(a), a2).abs();
        if a2_dist > a2_radius_a + a2_radius_b {
            return false;
        }
    }

    // Case 3: project to b1.
    {
        let b1_radius_b = dot(b1, b1);
        let b1_radius_a = dot(a1, b1).abs() + dot(a2, b1).abs();
        let b1_dist = dot(b.sub(a), b1).abs();
        if b1_dist > b1_radius_a + b1_radius_b {
            return false;
        }
    }

    // Case 4: project to b2.
    {
        let b2_radius_b = dot(b2, b2);
        let b2_radius_a = dot(a1, b2).abs() + dot(a2, b2).abs();
        let b2_dist = dot(b.sub(a), b2).abs();
        if b2_dist > b2_radius_a + b2_radius_b {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_basics() {
        let r = DngRect::from_corners(10, 20, 30, 60);
        assert_eq!(r.w(), 40);
        assert_eq!(r.h(), 20);
        assert!(r.not_empty());
        assert!(r.contains_point(DngPoint::new(15, 25)));
        assert!(!r.contains_point(DngPoint::new(30, 25)));
    }

    #[test]
    fn rect_and_or() {
        let a = DngRect::from_corners(0, 0, 100, 100);
        let b = DngRect::from_corners(50, 50, 200, 200);
        assert_eq!(a.and_(b), DngRect::from_corners(50, 50, 100, 100));
        assert_eq!(a.or_(b), DngRect::from_corners(0, 0, 200, 200));
        let disjoint = DngRect::from_corners(200, 200, 300, 300);
        assert!(a.and_(disjoint).is_zero());
    }

    #[test]
    fn rect_real64_round() {
        let r = DngRectReal64 { t: 0.4, l: 1.6, b: 9.5, r: 10.5 };
        assert_eq!(r.round(), DngRect::from_corners(0, 2, 10, 11));
        let c = r.center();
        assert!((c.v - 4.95).abs() < 1e-12);
    }
}

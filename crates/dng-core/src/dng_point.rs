// Port of dng_point.h/.cpp — 2D point types (integer and real64).
//
// C++ `dng_point::operator+/-` throw a dng_exception on int32 overflow
// (program error).  The Rust methods panic in that case: an overflow here is
// a program bug exactly as in C++ (the exception is never caught locally).

use crate::dng_safe_arithmetic::{safe_int32_add, safe_int32_sub};
use crate::dng_types::{int32, real64};
use crate::dng_utils::{lerp_real64, round_int32_f64};

/// Port of `dng_point` (v = vertical/y, h = horizontal/x).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngPoint {
    pub v: int32,
    pub h: int32,
}

impl DngPoint {
    pub fn new(vv: int32, hh: int32) -> Self {
        DngPoint { v: vv, h: hh }
    }

    /// `Length ()` — hypot of (v, h).
    pub fn length(&self) -> real64 {
        (self.v as f64).hypot(self.h as f64)
    }

    /// `operator+` (overflow checked, as in C++).
    pub fn add(&self, other: DngPoint) -> DngPoint {
        DngPoint {
            v: safe_int32_add(self.v, other.v).expect("Overflow in dng_point operator+"),
            h: safe_int32_add(self.h, other.h).expect("Overflow in dng_point operator+"),
        }
    }

    /// `operator-` (overflow checked, as in C++).
    pub fn sub(&self, other: DngPoint) -> DngPoint {
        DngPoint {
            v: safe_int32_sub(self.v, other.v).expect("Overflow in dng_point operator-"),
            h: safe_int32_sub(self.h, other.h).expect("Overflow in dng_point operator-"),
        }
    }
}

/// Port of `dng_point_real64`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DngPointReal64 {
    pub v: real64,
    pub h: real64,
}

impl DngPointReal64 {
    pub fn new(vv: real64, hh: real64) -> Self {
        DngPointReal64 { v: vv, h: hh }
    }

    pub fn from_point(pt: DngPoint) -> Self {
        DngPointReal64 {
            v: pt.v as real64,
            h: pt.h as real64,
        }
    }

    /// `Round ()`.
    pub fn round(&self) -> DngPoint {
        DngPoint::new(round_int32_f64(self.v), round_int32_f64(self.h))
    }

    pub fn length(&self) -> real64 {
        self.v.hypot(self.h)
    }

    /// `Scale (scale)`.
    pub fn scale(&mut self, scale: real64) {
        self.v *= scale;
        self.h *= scale;
    }

    /// `Normalize ()`.
    pub fn normalize(&mut self) {
        let len = self.length();
        self.scale(1.0 / len);
    }

    pub fn add(&self, other: DngPointReal64) -> DngPointReal64 {
        DngPointReal64::new(self.v + other.v, self.h + other.h)
    }

    pub fn sub(&self, other: DngPointReal64) -> DngPointReal64 {
        DngPointReal64::new(self.v - other.v, self.h - other.h)
    }
}

impl std::ops::Add<DngPointReal64> for DngPointReal64 {
    type Output = DngPointReal64;
    fn add(self, rhs: DngPointReal64) -> DngPointReal64 {
        self.add(rhs)
    }
}

impl std::ops::Sub<DngPointReal64> for DngPointReal64 {
    type Output = DngPointReal64;
    fn sub(self, rhs: DngPointReal64) -> DngPointReal64 {
        self.sub(rhs)
    }
}

impl std::ops::Add<DngPoint> for DngPoint {
    type Output = DngPoint;
    fn add(self, rhs: DngPoint) -> DngPoint {
        self.add(rhs)
    }
}

impl std::ops::Sub<DngPoint> for DngPoint {
    type Output = DngPoint;
    fn sub(self, rhs: DngPoint) -> DngPoint {
        self.sub(rhs)
    }
}

/// `Distance (a, b)`.
pub fn distance(a: DngPointReal64, b: DngPointReal64) -> real64 {
    a.sub(b).length()
}

/// `DistanceSquared (a, b)`.
pub fn distance_squared(a: DngPointReal64, b: DngPointReal64) -> real64 {
    let diff = a.sub(b);
    diff.v * diff.v + diff.h * diff.h
}

/// `DistanceSquared (p, v, w)` — distance squared from point p to the line
/// segment from v to w.
pub fn distance_squared_to_segment(
    p: DngPointReal64,
    v: DngPointReal64,
    w: DngPointReal64,
) -> real64 {
    let len2 = distance_squared(v, w);
    if len2 == 0.0 {
        return distance_squared(p, v);
    }
    let t = ((p.h - v.h) * (w.h - v.h) + (p.v - v.v) * (w.v - v.v)) / len2;
    if t <= 0.0 {
        return distance_squared(p, v);
    }
    if t >= 1.0 {
        return distance_squared(p, w);
    }
    let z = DngPointReal64::new(v.v + t * (w.v - v.v), v.h + t * (w.h - v.h));
    distance_squared(p, z)
}

/// `Transpose (a)` for integer points.
pub fn transpose_point(a: DngPoint) -> DngPoint {
    DngPoint::new(a.h, a.v)
}

/// `Transpose (a)` for real points.
pub fn transpose_point_real64(a: DngPointReal64) -> DngPointReal64 {
    DngPointReal64::new(a.h, a.v)
}

/// `Lerp (a, b, t)`.
pub fn lerp_point_real64(a: DngPointReal64, b: DngPointReal64, t: real64) -> DngPointReal64 {
    DngPointReal64::new(lerp_real64(a.v, b.v, t), lerp_real64(a.h, b.h, t))
}

/// `Dot (a, b)`.
pub fn dot(a: DngPointReal64, b: DngPointReal64) -> real64 {
    a.h * b.h + a.v * b.v
}

/// `scale * pt` (C++ `operator*`).
pub fn scale_point(scale: real64, pt: DngPointReal64) -> DngPointReal64 {
    DngPointReal64::new(pt.v * scale, pt.h * scale)
}

/// `MakePerpendicular (pt)` for integer points: (-h, v).
pub fn make_perpendicular(pt: DngPoint) -> DngPoint {
    DngPoint::new(-pt.h, pt.v)
}

/// `MakePerpendicular (pt)` for real points: (-h, v).
pub fn make_perpendicular_real64(pt: DngPointReal64) -> DngPointReal64 {
    DngPointReal64::new(-pt.h, pt.v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_basics() {
        let p = DngPoint::new(3, 4);
        assert_eq!(p.length(), 5.0);
        let q = DngPoint::new(1, 1);
        assert_eq!(p.add(q), DngPoint::new(4, 5));
        assert_eq!(p.sub(q), DngPoint::new(2, 3));
    }

    #[test]
    fn distance_fns() {
        let a = DngPointReal64::new(0.0, 0.0);
        let b = DngPointReal64::new(3.0, 4.0);
        assert_eq!(distance(a, b), 5.0);
        assert_eq!(distance_squared(a, b), 25.0);
    }
}

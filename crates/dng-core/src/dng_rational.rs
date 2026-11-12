// Port of dng_rational.h/.cpp — rational number types.
//
// Note: C++ `Set_real64` assigns an initial (0,1) for zero/non-positive
// input and then FALLS THROUGH to the denominator-selection logic, so the
// final value is (0, dd), not (0, 1).  That exact behavior is preserved.
// `ReduceByFactor (0)` is UB in C++; the Rust port returns without change
// (documented in PORTING_NOTES.md).

use crate::dng_types::{int32, int64, real64, uint32, uint64};
use crate::dng_utils::{abs_real64, round_int32_f64, round_uint32_f64};

/// Port of `dng_srational` (i32/i32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngSRational {
    /// Numerator
    pub n: int32,
    /// Denominator
    pub d: int32,
}

impl DngSRational {
    pub fn new(nn: int32, dd: int32) -> Self {
        DngSRational { n: nn, d: dd }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.d = 0;
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.d != 0
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `As_real64 ()` — 0.0 when d == 0.
    pub fn as_real64(&self) -> real64 {
        if self.d != 0 {
            self.n as real64 / self.d as real64
        } else {
            0.0
        }
    }

    /// `Set_real64 (x, dd)`.
    pub fn set_real64(&mut self, x: real64, dd: int32) {
        if x == 0.0 {
            *self = DngSRational::new(0, 1);
        }

        let mut dd = dd;

        if dd == 0 {
            let y = abs_real64(x);

            if y >= 32768.0 {
                dd = 1;
            } else if y >= 1.0 {
                dd = 32768;
            } else {
                dd = 32768 * 32768;
            }
        }

        *self = DngSRational::new(round_int32_f64(x * dd as real64), dd);
    }

    /// `ReduceByFactor (factor)`.
    pub fn reduce_by_factor(&mut self, factor: int32) {
        if factor == 0 {
            return; // C++ UB; documented deviation.
        }
        while self.n % factor == 0 && self.d % factor == 0 && self.d >= factor {
            self.n /= factor;
            self.d /= factor;
        }
    }
}

/// Port of `dng_urational` (u32/u32).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngURational {
    /// Numerator
    pub n: uint32,
    /// Denominator
    pub d: uint32,
}

impl DngURational {
    pub fn new(nn: uint32, dd: uint32) -> Self {
        DngURational { n: nn, d: dd }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.d = 0;
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.d != 0
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `As_real64 ()` — 0.0 when d == 0.
    pub fn as_real64(&self) -> real64 {
        if self.d != 0 {
            self.n as real64 / self.d as real64
        } else {
            0.0
        }
    }

    /// `Set_real64 (x, dd)`.
    pub fn set_real64(&mut self, x: real64, dd: uint32) {
        if x <= 0.0 {
            *self = DngURational::new(0, 1);
        }

        let mut dd = dd;

        if dd == 0 {
            if x >= 32768.0 {
                dd = 1;
            } else if x >= 1.0 {
                dd = 32768;
            } else {
                dd = 32768 * 32768;
            }
        }

        *self = DngURational::new(round_uint32_f64(x * dd as real64), dd);
    }

    /// `ReduceByFactor (factor)`.
    pub fn reduce_by_factor(&mut self, factor: uint32) {
        if factor == 0 {
            return; // C++ UB; documented deviation.
        }
        while self.n % factor == 0 && self.d % factor == 0 && self.d >= factor {
            self.n /= factor;
            self.d /= factor;
        }
    }

    /// `ScaleBy (scale)`.
    pub fn scale_by(&mut self, scale: real64) {
        if scale <= 0.0 {
            *self = DngURational::new(0, 1);
            return;
        }

        if self.is_valid() {
            if self.d > 1 {
                let d = self.d;
                self.reduce_by_factor(d);
            }

            self.reduce_by_factor(2);
            self.reduce_by_factor(3);
            self.reduce_by_factor(5);

            loop {
                if scale == 1.0 {
                    break;
                } else if scale > 1.0 {
                    let x = round_uint32_f64(scale);

                    if scale == x as real64 {
                        if self.d % x == 0 {
                            self.d /= x;
                            break;
                        } else if self.n as uint64 * x as uint64 <= 0xFFFFFFFF {
                            self.n *= x;
                            break;
                        }
                    }
                } else {
                    let x = round_uint32_f64(1.0 / scale);

                    if scale == 1.0 / x as real64 {
                        if self.n % x == 0 {
                            self.n /= x;
                            break;
                        } else if self.d as uint64 * x as uint64 <= 0xFFFFFFFF {
                            self.d *= x;
                            break;
                        }
                    }
                }

                let v = self.as_real64() * scale;
                self.set_real64(v, 0);

                break;
            }

            self.reduce_by_factor(2);
        }
    }
}

/// 64-bit rational (`DngRational`, i64/i64).  Minimal extension required by
/// the ARCHITECTURE.md type contract (no 1.7.1 C++ counterpart); mirrors the
/// dng_urational/dng_srational method set with wider arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngRational {
    pub n: int64,
    pub d: int64,
}

impl DngRational {
    pub fn new(nn: int64, dd: int64) -> Self {
        DngRational { n: nn, d: dd }
    }

    pub fn clear(&mut self) {
        self.n = 0;
        self.d = 0;
    }

    pub fn is_valid(&self) -> bool {
        self.d != 0
    }

    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    pub fn as_real64(&self) -> real64 {
        if self.d != 0 {
            self.n as real64 / self.d as real64
        } else {
            0.0
        }
    }

    /// Set from a real with denominator selection identical to
    /// `dng_srational::Set_real64`.
    pub fn set_real64(&mut self, x: real64, dd: int64) {
        if x == 0.0 {
            *self = DngRational::new(0, 1);
        }

        let mut dd = dd;

        if dd == 0 {
            let y = abs_real64(x);
            if y >= 32768.0 {
                dd = 1;
            } else if y >= 1.0 {
                dd = 32768;
            } else {
                dd = 32768 * 32768;
            }
        }

        *self = DngRational::new((x * dd as real64).round() as int64, dd);
    }

    pub fn reduce_by_factor(&mut self, factor: int64) {
        if factor == 0 {
            return;
        }
        while self.n % factor == 0 && self.d % factor == 0 && self.d >= factor {
            self.n /= factor;
            self.d /= factor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srational_basics() {
        let r = DngSRational::new(1, 3);
        assert!((r.as_real64() - 1.0 / 3.0).abs() < 1e-15);
        assert!(r.is_valid());
        assert!(DngSRational::new(1, 0).not_valid());
        assert_eq!(DngSRational::new(1, 0).as_real64(), 0.0);
    }

    #[test]
    fn srational_set_real64() {
        // Value >= 1 uses denominator 32768.
        let mut r = DngSRational::new(0, 0);
        r.set_real64(2.0, 0);
        assert_eq!(r, DngSRational::new(65536, 32768));

        // Value < 1 uses denominator 32768*32768.
        r.set_real64(0.5, 0);
        assert_eq!(r, DngSRational::new(536870912, 1073741824));

        // Explicit denominator.
        r.set_real64(1.5, 2);
        assert_eq!(r, DngSRational::new(3, 2));

        // Zero: falls through the dd selection (C++ behavior).
        r.set_real64(0.0, 0);
        assert_eq!(r, DngSRational::new(0, 32768 * 32768));

        // Large value uses denominator 1.
        r.set_real64(40000.0, 0);
        assert_eq!(r, DngSRational::new(40000, 1));
    }

    #[test]
    fn urational_scale_by() {
        let mut r = DngURational::new(1, 3);
        r.scale_by(2.0);
        assert_eq!(r, DngURational::new(2, 3));

        let mut r = DngURational::new(2, 4);
        r.scale_by(1.0);
        assert_eq!(r, DngURational::new(1, 2));

        let mut r = DngURational::new(1, 3);
        r.scale_by(0.5);
        assert_eq!(r, DngURational::new(1, 6));

        // Invalid scale.
        let mut r = DngURational::new(1, 3);
        r.scale_by(-1.0);
        assert_eq!(r, DngURational::new(0, 1));

        // Reduce on scale-up when denominator divides.
        let mut r = DngURational::new(3, 9);
        r.scale_by(3.0);
        assert_eq!(r, DngURational::new(1, 1));
    }

    #[test]
    fn reduce_by_factor() {
        let mut r = DngSRational::new(12, 8);
        r.reduce_by_factor(2);
        assert_eq!(r, DngSRational::new(3, 2));
    }

    #[test]
    fn rational64() {
        let mut r = DngRational::new(0, 0);
        r.set_real64(1.0 / 3.0, 0);
        // i64 rounding to 32768*32768 denominator keeps ~1e-11 precision.
        assert!((r.as_real64() - 1.0 / 3.0).abs() < 1e-9);
    }
}

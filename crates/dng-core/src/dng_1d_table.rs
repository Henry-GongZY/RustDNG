// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_1d_table.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_1d_table.h/.cpp — lookup table with linear interpolation.

use crate::dng_1d_function::Dng1dFunction;
use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::bad_format;
use crate::dng_types::{real32, real64, uint32, uint16};
use crate::dng_utils::{abs_real32, max_real32};

/// Constant denoting minimum size of table.
pub const kMinTableSize: uint32 = 512;

/// Constant denoting default size of table.
const kDefaultTableSize: uint32 = 4096;

/// Port of `dng_1d_table`.
#[derive(Debug, Clone)]
pub struct Dng1dTable {
    table: Vec<real32>,
    table_count: uint32,
    table_count32: real32,
}

impl Dng1dTable {
    /// `dng_1d_table (count)` — count must be a power of two and at least
    /// kMinTableSize.
    pub fn new(count: uint32) -> DngResult<Self> {
        if count < kMinTableSize {
            return Err(DngError::Unsupported("count must be at least kMinTableSize".to_string()));
        }
        if (count & (count - 1)) != 0 {
            return Err(DngError::Unsupported("count must be power of 2".to_string()));
        }

        Ok(Dng1dTable {
            table: Vec::new(),
            table_count: count,
            table_count32: count as real32,
        })
    }

    /// Default 4096-entry table.
    pub fn default_table() -> DngResult<Self> {
        Self::new(kDefaultTableSize)
    }

    /// `Count ()`.
    pub fn count(&self) -> uint32 {
        self.table_count
    }

    /// `Initialize (function, subSample)`.
    pub fn initialize(
        &mut self,
        function: &dyn Dng1dFunction,
        sub_sample: bool,
    ) -> DngResult<()> {
        self.table = vec![0.0; (self.table_count + 2) as usize];

        if sub_sample {
            self.table[0] = function.evaluate(0.0) as real32;
            self.table[self.table_count as usize] = function.evaluate(1.0) as real32;

            let max_delta = max_real32(
                abs_real32(
                    self.table[self.table_count as usize] - self.table[0],
                ),
                1.0,
            ) * (1.0 / 256.0);

            self.sub_divide(function, 0, self.table_count, max_delta);
        } else {
            for j in 0..=self.table_count {
                let x = j as real64 * (1.0 / self.table_count as real64);
                let y = function.evaluate(x);
                self.table[j as usize] = crate::dng_safe_arithmetic::convert_double_to_float(y);
            }
        }

        self.table[(self.table_count + 1) as usize] = self.table[self.table_count as usize];

        Ok(())
    }

    /// `SubDivide` — adaptive sampling for subSample mode.
    fn sub_divide(
        &mut self,
        function: &dyn Dng1dFunction,
        lower: uint32,
        upper: uint32,
        max_delta: real32,
    ) {
        let range = upper - lower;

        let mut sub_divide = range > (self.table_count >> 8);

        if !sub_divide {
            let delta = abs_real32(self.table[upper as usize] - self.table[lower as usize]);
            if delta > max_delta {
                sub_divide = true;
            }
        }

        if sub_divide {
            let middle = (lower + upper) >> 1;

            self.table[middle as usize] = function
                .evaluate(middle as real64 * (1.0 / self.table_count as real64))
                as real32;

            if range > 2 {
                self.sub_divide(function, lower, middle, max_delta);
                self.sub_divide(function, middle, upper, max_delta);
            }
        } else {
            let mut y0 = self.table[lower as usize] as real64;
            let y1 = self.table[upper as usize] as real64;

            let delta = (y1 - y0) / range as real64;

            for j in (lower + 1)..upper {
                y0 += delta;
                self.table[j as usize] = y0 as real32;
            }
        }
    }

    /// `Interpolate (x)` — checked.
    pub fn interpolate(&self, x: real32) -> DngResult<real32> {
        let y = x * self.table_count32;

        let index = y as i32;

        if index < 0 || index > self.table_count as i32 {
            return Err(bad_format()); // "Index out of range."
        }

        Ok(self.interpolate_index(index, y))
    }

    /// `UnsafeInterpolate (x)` — for x known to be in 0.0..=1.0.  Values
    /// outside the table range clamp to the ends (the C++ reads out of
    /// bounds; the port clamps — see PORTING_NOTES.md).
    pub fn unsafe_interpolate(&self, x: real32) -> real32 {
        let y = x * self.table_count32;
        let index = y as i32;

        if index < 0 {
            return self.table[0];
        }
        if index > self.table_count as i32 {
            return self.table[self.table_count as usize];
        }

        self.interpolate_index(index, y)
    }

    fn interpolate_index(&self, index: i32, y: real32) -> real32 {
        let z = index as real32;
        let fract = y - z;

        self.table[index as usize] * (1.0 - fract) + self.table[index as usize + 1] * fract
    }

    /// `Table ()` — direct access.
    pub fn table(&self) -> &[real32] {
        &self.table
    }

    /// `Expand16 (table16)` — expand to a 16-bit to 16-bit table.
    pub fn expand16(&self, table16: &mut [uint16]) -> DngResult<()> {
        assert!(table16.len() >= 0x10000);

        let step = self.table_count as real64 / 65535.0;

        let mut y0 = self.table[0] as real64;
        let mut y1 = self.table[1] as real64;

        let mut base = y0 * 65535.0 + 0.5;
        let mut slope = (y1 - y0) * 65535.0;

        let mut index: uint32 = 1;
        let mut fract: real64 = 0.0;

        for j in 0..0x10000usize {
            table16[j] = (base + slope * fract) as uint16;

            fract += step;

            if fract > 1.0 {
                index += 1;
                fract -= 1.0;

                y0 = y1;
                y1 = self.table[index as usize] as real64;

                base = y0 * 65535.0 + 0.5;
                slope = (y1 - y0) * 65535.0;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_1d_function::DngPiecewiseLinear;

    fn gamma_function(gamma: real64) -> impl Dng1dFunction {
        struct Gamma(real64);
        impl Dng1dFunction for Gamma {
            fn evaluate(&self, x: real64) -> real64 {
                x.powf(self.0)
            }
        }
        Gamma(gamma)
    }

    #[test]
    fn initialize_and_interpolate() {
        let mut table = Dng1dTable::default_table().unwrap();
        table.initialize(&gamma_function(2.2), false).unwrap();

        assert_eq!(table.count(), 4096);
        assert_eq!(table.table().len(), 4098);

        // Endpoints exact.
        assert_eq!(table.interpolate(0.0).unwrap(), 0.0);
        assert!((table.interpolate(1.0).unwrap() - 1.0).abs() < 1e-6);

        // 0.5 -> 0.5^2.2 ~ 0.2176
        let v = table.interpolate(0.5).unwrap();
        assert!((v - 0.5f32.powf(2.2)).abs() < 1e-3, "v={v}");

        // Out-of-range errors.
        assert!(table.interpolate(-0.1).is_err());
        assert!(table.interpolate(1.1).is_err());

        // Unsafe clamps.
        assert_eq!(table.unsafe_interpolate(-1.0), 0.0);
    }

    #[test]
    fn subsample_matches_full() {
        let mut full = Dng1dTable::default_table().unwrap();
        full.initialize(&gamma_function(1.8), false).unwrap();

        let mut sub = Dng1dTable::default_table().unwrap();
        sub.initialize(&gamma_function(1.8), true).unwrap();

        // Max deviation bounded by the subdivision tolerance.
        let mut max_diff: f32 = 0.0;
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let a = full.unsafe_interpolate(x);
            let b = sub.unsafe_interpolate(x);
            max_diff = max_diff.max((a - b).abs());
        }
        assert!(max_diff < 0.01, "max_diff={max_diff}");
    }

    #[test]
    fn expand16() {
        let mut table = Dng1dTable::new(4096).unwrap();
        table.initialize(&gamma_function(1.0), false).unwrap();

        let mut table16 = vec![0u16; 0x10000];
        table.expand16(&mut table16).unwrap();

        // Identity: table16[j] ~ j.
        assert_eq!(table16[0], 0);
        assert_eq!(table16[0xFFFF], 0xFFFF);
        assert!((table16[0x8000] as i32 - 0x8000).abs() <= 1);
    }

    #[test]
    fn validation() {
        assert!(Dng1dTable::new(256).is_err()); // < kMinTableSize
        assert!(Dng1dTable::new(1000).is_err()); // not power of 2
        assert!(Dng1dTable::new(512).is_ok());
    }

    #[allow(unused)]
    fn unused_piecewise(p: &DngPiecewiseLinear) -> bool {
        p.is_valid()
    }
}

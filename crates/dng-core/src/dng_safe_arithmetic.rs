// Port of dng_safe_arithmetic.h/.cpp — overflow-guarded integer arithmetic.
//
// C++ throws a dng_exception (dng_error_unknown, i.e. program error) on
// overflow; the Rust `safe_*` functions return `DngResult<T>` and the
// `try_*` functions return Option/bool like the out-param C++ overloads.

use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::program_error;
use crate::dng_types::{int32, int64, uint32, uint64};

fn overflow() -> DngError {
    // C++ uses ThrowProgramError("...") for these — dng_error_unknown.
    program_error()
}

// ---------------------------------------------------------------- add

/// `SafeInt32Add` boolean form.
pub fn try_safe_int32_add(arg1: int32, arg2: int32) -> Option<int32> {
    arg1.checked_add(arg2)
}

/// `SafeInt32Add` throwing form.
pub fn safe_int32_add(arg1: int32, arg2: int32) -> DngResult<int32> {
    arg1.checked_add(arg2).ok_or_else(overflow)
}

pub fn safe_int64_add(arg1: int64, arg2: int64) -> DngResult<int64> {
    arg1.checked_add(arg2).ok_or_else(overflow)
}

pub fn try_safe_uint32_add(arg1: uint32, arg2: uint32) -> Option<uint32> {
    arg1.checked_add(arg2)
}

pub fn safe_uint32_add(arg1: uint32, arg2: uint32) -> DngResult<uint32> {
    arg1.checked_add(arg2).ok_or_else(overflow)
}

pub fn safe_uint64_add(arg1: uint64, arg2: uint64) -> DngResult<uint64> {
    arg1.checked_add(arg2).ok_or_else(overflow)
}

// ---------------------------------------------------------------- sub

pub fn try_safe_int32_sub(arg1: int32, arg2: int32) -> Option<int32> {
    arg1.checked_sub(arg2)
}

pub fn safe_int32_sub(arg1: int32, arg2: int32) -> DngResult<int32> {
    arg1.checked_sub(arg2).ok_or_else(overflow)
}

pub fn safe_uint32_sub(arg1: uint32, arg2: uint32) -> DngResult<uint32> {
    arg1.checked_sub(arg2).ok_or_else(overflow)
}

// ---------------------------------------------------------------- mult

pub fn try_safe_int32_mult(arg1: int32, arg2: int32) -> Option<int32> {
    arg1.checked_mul(arg2)
}

pub fn safe_int32_mult(arg1: int32, arg2: int32) -> DngResult<int32> {
    arg1.checked_mul(arg2).ok_or_else(overflow)
}

pub fn try_safe_uint32_mult2(arg1: uint32, arg2: uint32) -> Option<uint32> {
    arg1.checked_mul(arg2)
}

pub fn try_safe_uint32_mult3(arg1: uint32, arg2: uint32, arg3: uint32) -> Option<uint32> {
    arg1.checked_mul(arg2)?.checked_mul(arg3)
}

pub fn try_safe_uint32_mult4(
    arg1: uint32,
    arg2: uint32,
    arg3: uint32,
    arg4: uint32,
) -> Option<uint32> {
    arg1.checked_mul(arg2)?.checked_mul(arg3)?.checked_mul(arg4)
}

pub fn safe_uint32_mult2(arg1: uint32, arg2: uint32) -> DngResult<uint32> {
    try_safe_uint32_mult2(arg1, arg2).ok_or_else(overflow)
}

pub fn safe_uint32_mult3(arg1: uint32, arg2: uint32, arg3: uint32) -> DngResult<uint32> {
    try_safe_uint32_mult3(arg1, arg2, arg3).ok_or_else(overflow)
}

pub fn safe_uint32_mult4(arg1: uint32, arg2: uint32, arg3: uint32, arg4: uint32) -> DngResult<uint32> {
    try_safe_uint32_mult4(arg1, arg2, arg3, arg4).ok_or_else(overflow)
}

pub fn safe_int64_mult(arg1: int64, arg2: int64) -> DngResult<int64> {
    arg1.checked_mul(arg2).ok_or_else(overflow)
}

pub fn safe_sizet_mult(arg1: usize, arg2: usize) -> DngResult<usize> {
    arg1.checked_mul(arg2).ok_or_else(overflow)
}

// ---------------------------------------------------------------- divide

/// Divides arg1 by arg2 rounding up.  If arg2 is zero, program error.
pub fn safe_uint32_divide_up(arg1: uint32, arg2: uint32) -> DngResult<uint32> {
    if arg2 == 0 {
        return Err(overflow());
    }
    Ok(arg1.div_ceil(arg2))
}

/// Smallest multiple of `multiple_of` >= `val` (bool form).
pub fn try_round_up_uint32_to_multiple(val: uint32, multiple_of: uint32) -> Option<uint32> {
    if multiple_of == 0 {
        return None;
    }
    let (q, r) = (val / multiple_of, val % multiple_of);
    if r == 0 {
        Some(val)
    } else {
        q.checked_add(1)?.checked_mul(multiple_of)
    }
}

/// Throwing form of `RoundUpUint32ToMultiple`.
pub fn round_up_uint32_to_multiple(val: uint32, multiple_of: uint32) -> DngResult<uint32> {
    try_round_up_uint32_to_multiple(val, multiple_of).ok_or_else(overflow)
}

// ---------------------------------------------------------------- convert

pub fn try_convert_uint32_to_int32(val: uint32) -> Option<int32> {
    int32::try_from(val).ok()
}

pub fn convert_uint32_to_int32(val: uint32) -> DngResult<int32> {
    try_convert_uint32_to_int32(val).ok_or_else(overflow)
}

/// `ConvertDoubleToInt32` — truncating conversion; program error if out of range.
pub fn convert_double_to_int32(val: f64) -> DngResult<int32> {
    if val.is_nan() || val < -(2_147_483_648.0) || val >= 2_147_483_648.0 {
        return Err(overflow());
    }
    Ok(val as int32)
}

/// `ConvertDoubleToUint32` — truncating conversion; program error if out of range.
pub fn convert_double_to_uint32(val: f64) -> DngResult<uint32> {
    if val.is_nan() || val < 0.0 || val >= 4_294_967_296.0 {
        return Err(overflow());
    }
    Ok(val as uint32)
}

/// `ConvertDoubleToFloat` — values beyond f32 range become ±infinity, NaN stays NaN.
pub fn convert_double_to_float(val: f64) -> f32 {
    val as f32
}

// ---------------------------------------------------------------- wrappers

/// Port of `dng_safe_uint32` — a uint32 paired with checked arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DngSafeUint32 {
    value: uint32,
}

impl DngSafeUint32 {
    pub fn new(x: uint32) -> Self {
        DngSafeUint32 { value: x }
    }

    pub fn get(self) -> uint32 {
        self.value
    }

    /// `operator+=` — checked.
    pub fn checked_add(self, x: uint32) -> DngResult<Self> {
        Ok(DngSafeUint32::new(safe_uint32_add(self.value, x)?))
    }

    /// `operator*=` — checked.
    pub fn checked_mul(self, x: uint32) -> DngResult<Self> {
        Ok(DngSafeUint32::new(safe_uint32_mult2(self.value, x)?))
    }
}

/// Port of `dng_safe_int32`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DngSafeInt32 {
    value: int32,
}

impl DngSafeInt32 {
    pub fn new(x: int32) -> Self {
        DngSafeInt32 { value: x }
    }

    /// `explicit dng_safe_int32 (const dng_safe_uint32 &x)` — overflow checked.
    pub fn from_uint32(x: uint32) -> DngResult<Self> {
        Ok(DngSafeInt32::new(convert_uint32_to_int32(x)?))
    }

    pub fn get(self) -> int32 {
        self.value
    }

    /// `Set_uint32` — overflow checked.
    pub fn set_uint32(&mut self, x: uint32) -> DngResult<()> {
        self.value = convert_uint32_to_int32(x)?;
        Ok(())
    }

    pub fn checked_add(self, x: int32) -> DngResult<Self> {
        Ok(DngSafeInt32::new(safe_int32_add(self.value, x)?))
    }

    pub fn checked_sub(self, x: int32) -> DngResult<Self> {
        Ok(DngSafeInt32::new(safe_int32_sub(self.value, x)?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_mult_bounds() {
        assert_eq!(safe_int32_add(1, 2).unwrap(), 3);
        assert!(safe_int32_add(i32::MAX, 1).is_err());
        assert!(safe_uint32_mult2(u32::MAX, 2).is_err());
        assert_eq!(safe_uint32_mult2(3, 4).unwrap(), 12);
        assert_eq!(safe_uint32_mult4(2, 3, 4, 5).unwrap(), 120);
    }

    #[test]
    fn divide_up_and_round() {
        assert_eq!(safe_uint32_divide_up(7, 2).unwrap(), 4);
        assert_eq!(safe_uint32_divide_up(8, 2).unwrap(), 4);
        assert!(safe_uint32_divide_up(1, 0).is_err());
        assert_eq!(round_up_uint32_to_multiple(10, 8).unwrap(), 16);
        assert_eq!(round_up_uint32_to_multiple(16, 8).unwrap(), 16);
        assert!(round_up_uint32_to_multiple(u32::MAX - 1, 3).is_err()); // 4294967294/3 overflows on round-up
    }

    #[test]
    fn conversions() {
        assert_eq!(convert_uint32_to_int32(100).unwrap(), 100);
        assert!(convert_uint32_to_int32(3_000_000_000).is_err());
        assert_eq!(convert_double_to_int32(3.9).unwrap(), 3);
        assert_eq!(convert_double_to_uint32(-1.0).is_err(), true);
        assert_eq!(convert_double_to_uint32(4.2e9).unwrap(), 4_200_000_000u32);
    }
}

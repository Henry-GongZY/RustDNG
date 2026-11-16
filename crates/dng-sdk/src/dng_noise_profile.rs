// Port of the noise-model classes declared in dng_negative.h
// (dng_noise_function, dng_noise_profile) and implemented in
// dng_negative.cpp lines 83-220.
//
// PORT-NOTE(dng_negative.h): SDK 1.7.1 declares the noise model inside
// dng_negative.h; the port keeps it in its own module per the phase-2
// ownership table in NEGATIVE_API.md.

use dng_core::{real64, Dng1dFunction, uint32};
use crate::dng_negative::DngNegative;
use dng_core::dng_sdk_limits::{kMaxColorPlanes};

/// Port of `dng_noise_function` — noise model N(x) = sqrt (scale*x + offset).
///
/// C++ derives from `dng_1d_function`; the port implements the
/// `Dng1dFunction` trait from dng-core.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DngNoiseFunction {
    /// `fScale`.
    pub scale: real64,
    /// `fOffset`.
    pub offset: real64,
}

impl DngNoiseFunction {
    /// `dng_noise_function ()` — create empty and invalid noise function.
    pub fn new() -> Self {
        DngNoiseFunction {
            scale: 0.0,
            offset: 0.0,
        }
    }

    /// `dng_noise_function (scale, offset)`.
    pub fn with_scale_offset(scale: real64, offset: real64) -> Self {
        DngNoiseFunction { scale, offset }
    }

    /// `Scale ()` — the scale (slope, gain) of the noise function.
    pub fn scale(&self) -> real64 {
        self.scale
    }

    /// `Offset ()` — the offset (square of the noise floor) of the noise
    /// function.
    pub fn offset(&self) -> real64 {
        self.offset
    }

    /// `SetScale (scale)`.
    pub fn set_scale(&mut self, scale: real64) {
        self.scale = scale;
    }

    /// `SetOffset (offset)`.
    pub fn set_offset(&mut self, offset: real64) {
        self.offset = offset;
    }

    /// `IsValid ()` — is the noise function valid?
    pub fn is_valid(&self) -> bool {
        self.scale > 0.0 && self.offset >= 0.0
    }
}

impl Dng1dFunction for DngNoiseFunction {
    /// `Evaluate (x)` — compute noise (standard deviation) at the specified
    /// average signal level x.
    fn evaluate(&self, x: real64) -> real64 {
        (self.scale * x + self.offset).sqrt()
    }
}

/// Port of `dng_noise_profile` — noise profile for a negative.
///
/// For mosaiced negatives, the noise profile describes the approximate noise
/// characteristics of a mosaic negative after linearization, but prior to
/// demosaicing. For demosaiced negatives (i.e., linear DNGs), the noise
/// profile describes the approximate noise characteristics of the image data
/// immediately following the demosaic step, prior to the processing of
/// opcode list 3.
///
/// A noise profile may contain 1 or N noise functions, where N is the number
/// of color planes for the negative. Otherwise the noise profile is considered
/// to be invalid for that negative.
#[derive(Debug, Clone, Default)]
pub struct DngNoiseProfile {
    /// `fNoiseFunctions`.
    pub noise_functions: Vec<DngNoiseFunction>,
}

impl DngNoiseProfile {
    /// `dng_noise_profile ()` — create empty (invalid) noise profile.
    pub fn new() -> Self {
        DngNoiseProfile {
            noise_functions: Vec::new(),
        }
    }

    /// `dng_noise_profile (functions)` — create noise profile with the
    /// specified noise functions (1 per plane).
    pub fn with_functions(functions: Vec<DngNoiseFunction>) -> Self {
        DngNoiseProfile {
            noise_functions: functions,
        }
    }

    /// `IsValid ()` — is the noise profile valid?
    pub fn is_valid(&self) -> bool {
        if self.num_functions() == 0 || self.num_functions() > kMaxColorPlanes {
            return false;
        }

        for plane in 0..self.num_functions() {
            if !self.noise_function(plane).is_valid() {
                return false;
            }
        }

        true
    }

    /// `IsValidForNegative (negative)` — is the noise profile valid for the
    /// specified negative?
    pub fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        if !(self.num_functions() == 1 || self.num_functions() == negative.color_channels()) {
            return false;
        }

        self.is_valid()
    }

    /// `NoiseFunction (plane)` — the noise function for the specified plane.
    ///
    /// A single-function profile applies that function to every plane.
    pub fn noise_function(&self, plane: uint32) -> &DngNoiseFunction {
        if self.num_functions() == 1 {
            return &self.noise_functions[0];
        }

        debug_assert!(
            (plane as usize) < self.noise_functions.len(),
            "Bad plane index argument for NoiseFunction ()."
        );

        &self.noise_functions[plane as usize]
    }

    /// `NumFunctions ()` — the number of noise functions in this profile.
    pub fn num_functions(&self) -> uint32 {
        self.noise_functions.len() as uint32
    }
}

/// `operator==` — two profiles are equal when both invalid, or when both
/// valid with identical per-plane scale/offset pairs.
impl PartialEq for DngNoiseProfile {
    fn eq(&self, other: &DngNoiseProfile) -> bool {
        if self.is_valid() {
            if !other.is_valid() {
                return false;
            }

            if self.num_functions() != other.num_functions() {
                return false;
            }

            for plane in 0..self.num_functions() {
                let a = self.noise_function(plane);
                let b = other.noise_function(plane);

                if a.scale() != b.scale() || a.offset() != b.offset() {
                    return false;
                }
            }

            true
        } else {
            !other.is_valid()
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_evaluate_matches_cpp() {
        // N (x) = sqrt (scale * x + offset).
        let f = DngNoiseFunction::with_scale_offset(2.0, 4.0);
        assert!((f.evaluate(6.0) - 4.0).abs() < 1e-12);
        assert!((f.evaluate(0.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn function_validity() {
        assert!(!DngNoiseFunction::new().is_valid());
        assert!(!DngNoiseFunction::with_scale_offset(-1.0, 0.0).is_valid());
        assert!(!DngNoiseFunction::with_scale_offset(1.0, -0.5).is_valid());
        assert!(DngNoiseFunction::with_scale_offset(1.0, 0.0).is_valid());
    }

    #[test]
    fn profile_validity_and_plane_mapping() {
        let empty = DngNoiseProfile::new();
        assert!(!empty.is_valid());

        let single = DngNoiseProfile::with_functions(vec![DngNoiseFunction::with_scale_offset(
            1.0, 0.0,
        )]);
        assert!(single.is_valid());
        assert_eq!(single.num_functions(), 1);
        // Single-function profile maps every plane to the same function.
        assert_eq!(single.noise_function(2), single.noise_function(0));

        let too_many = DngNoiseProfile::with_functions(vec![
            DngNoiseFunction::with_scale_offset(1.0, 0.0),
            DngNoiseFunction::with_scale_offset(1.0, 0.0),
            DngNoiseFunction::with_scale_offset(1.0, 0.0),
            DngNoiseFunction::with_scale_offset(1.0, 0.0),
            DngNoiseFunction::with_scale_offset(1.0, 0.0),
        ]);
        assert!(!too_many.is_valid());
    }

    #[test]
    fn equality_semantics_match_cpp() {
        let valid_a = DngNoiseProfile::with_functions(vec![DngNoiseFunction::with_scale_offset(
            1.0, 0.0,
        )]);
        let valid_b = DngNoiseProfile::with_functions(vec![DngNoiseFunction::with_scale_offset(
            1.0, 0.0,
        )]);
        let invalid = DngNoiseProfile::new();

        assert_eq!(valid_a, valid_b);
        // Two invalid profiles compare equal.
        assert_eq!(invalid, DngNoiseProfile::new());
        // Valid vs invalid compares unequal (both directions).
        assert_ne!(valid_a, invalid);
        assert_ne!(invalid, valid_a);
    }
}

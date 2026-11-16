// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_sdk_limits.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_sdk_limits.h — SDK-wide limit constants.

use crate::dng_types::{real32, real64, uint32};

/// The maximum number of previews (in addition to the main IFD's thumbnail)
/// that we support embedded in a DNG.
pub const kMaxDNGPreviews: uint32 = 20;

/// Maximum supported number of semantic masks.
pub const kMaxSemanticMasks: uint32 = 100;

/// The maximum number of SubIFDs that will be parsed.
pub const kMaxSubIFDs: uint32 = kMaxDNGPreviews + kMaxSemanticMasks + 5;

/// The maximum number of chained IFDs that will be parsed.
pub const kMaxChainedIFDs: uint32 = 10;

/// The maximum number of samples per pixel.  (CMYK + transparency needs 5)
pub const kMaxSamplesPerPixel: uint32 = 5;

/// Maximum number of color planes.
pub const kMaxColorPlanes: uint32 = 4;

/// The maximum size of a CFA repeating pattern.
pub const kMaxCFAPattern: uint32 = 8;

/// The maximum size of a black level repeating pattern.
pub const kMaxBlackPattern: uint32 = 8;

/// The maximum number of masked area rectangles.
pub const kMaxMaskedAreas: uint32 = 4;

/// The maximum image size supported (pixels per side); qDNGBigImage path.
pub const kMaxImageSide: uint32 = if crate::dng_flags::qDNGBigImage {
    300000
} else {
    65000
};

/// The maximum number of tone curve points supported.
pub const kMaxToneCurvePoints: uint32 = 8192;

/// Maximum number of MP threads for dng_area_task operations (the Rust port
/// executes tasks serially; the constant is kept for API parity).
pub const kMaxMPThreads: uint32 = if crate::dng_flags::qDNG64Bit { 128 } else { 8 };

/// Maximum supported value of Stage3BlackLevelNormalized.
pub const kMaxStage3BlackLevelNormalized: real64 = 0.2;

/// Maximum number of points in a ProfileGainTableMap (64 megabytes).
pub const kMaxProfileGainTableMapPoints: uint32 = 16777216;

/// Minimum and maximum gain values in a ProfileGainTableMap.
pub const kProfileGainTableMap_MinGainValue: real32 = 0.000244140625; // 1 / 4096
pub const kProfileGainTableMap_MaxGainValue: real32 = 4096.0;

/// Minimum and maximum gamma parameter value in a ProfileGainTableMap2.
pub const kProfileGainTableMap_MinGamma: real32 = 0.125;
pub const kProfileGainTableMap_MaxGamma: real32 = 8.000;

pub const kMinSpectrumSamples: uint32 = 2;

/// The maximum number of spectral power samples for an illuminant.
pub const kMaxSpectrumSamples: uint32 = 1000;

/// Maximum recursion depth when parsing IFDs in dng_info.
pub const kMaxParseDepth: uint32 = 10;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_match_cpp() {
        assert_eq!(kMaxColorPlanes, 4);
        assert_eq!(kMaxBlackPattern, 8);
        assert_eq!(kMaxSamplesPerPixel, 5);
        assert_eq!(kMaxImageSide, 300000);
        assert_eq!(kMaxMPThreads, 128);
        assert_eq!(kMaxSubIFDs, 125);
    }
}

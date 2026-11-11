// Port of dng_tag_values.h — all value enums, transcribed verbatim from the
// C++ header by tools/transcribe_tags.py (values verified by spot checks and
// unit tests below).  Comments from the C++ header are preserved.

use crate::dng_types::uint32;

pub const sfMainImage: uint32 = 0;
pub const sfPreviewImage: uint32 = 1;
pub const sfTransparencyMask: uint32 = 4;
pub const sfPreviewMask: uint32 = 5;
pub const sfDepthMap: uint32 = 8;
pub const sfPreviewDepthMap: uint32 = 9;
pub const sfEnhancedImage: uint32 = 16;
pub const sfGainMap: uint32 = 32;
pub const sfAltPreviewImage: uint32 = 65537;
/// Added in DNG 1.6
pub const sfSemanticMask: uint32 = 65540;
pub const sfPreviewSemanticMask: uint32 = 65541;
pub const piWhiteIsZero: uint32 = 0;
pub const piBlackIsZero: uint32 = 1;
pub const piRGB: uint32 = 2;
pub const piRGBPalette: uint32 = 3;
pub const piTransparencyMask: uint32 = 4;
pub const piCMYK: uint32 = 5;
pub const piYCbCr: uint32 = 6;
pub const piCIELab: uint32 = 8;
pub const piICCLab: uint32 = 9;
/// TIFF-EP spec
pub const piCFA: uint32 = 32803;
pub const piLinearRaw: uint32 = 34892;
pub const piDepth: uint32 = 51177;
/// Added in DNG 1.6
pub const piPhotometricMask: uint32 = 52527;
/// Added in DNG 1.7
pub const piGainMap: uint32 = 52553;
pub const pcInterleaved: uint32 = 1;
pub const pcPlanar: uint32 = 2;
/// Internal use only
pub const pcRowInterleaved: uint32 = 100000;
/// Internal use only
pub const pcRowInterleavedAlignSIMD: uint32 = 100001;
pub const esUnspecified: uint32 = 0;
pub const esAssociatedAlpha: uint32 = 1;
pub const esUnassociatedAlpha: uint32 = 2;
pub const sfUnsignedInteger: uint32 = 1;
pub const sfSignedInteger: uint32 = 2;
pub const sfFloatingPoint: uint32 = 3;
pub const sfUndefined: uint32 = 4;
pub const ccUncompressed: uint32 = 1;
pub const ccLZW: uint32 = 5;
pub const ccOldJPEG: uint32 = 6;
pub const ccJPEG: uint32 = 7;
pub const ccDeflate: uint32 = 8;
pub const ccVc5: uint32 = 9;
pub const ccPackBits: uint32 = 32773;
pub const ccOldDeflate: uint32 = 32946;
pub const ccLossyJPEG: uint32 = 34892;
pub const ccJXL: uint32 = 52546;
pub const cpNullPredictor: uint32 = 1;
pub const cpHorizontalDifference: uint32 = 2;
pub const cpFloatingPoint: uint32 = 3;
pub const cpHorizontalDifferenceX2: uint32 = 34892;
pub const cpHorizontalDifferenceX4: uint32 = 34893;
pub const cpFloatingPointX2: uint32 = 34894;
pub const cpFloatingPointX4: uint32 = 34895;
pub const ruNone: uint32 = 1;
pub const ruInch: uint32 = 2;
pub const ruCM: uint32 = 3;
pub const ruMM: uint32 = 4;
pub const ruMicroM: uint32 = 5;
pub const lsUnknown: uint32 = 0;
pub const lsDaylight: uint32 = 1;
pub const lsFluorescent: uint32 = 2;
pub const lsTungsten: uint32 = 3;
pub const lsFlash: uint32 = 4;
pub const lsFineWeather: uint32 = 9;
pub const lsCloudyWeather: uint32 = 10;
pub const lsShade: uint32 = 11;
/// D  5700 - 7100K
pub const lsDaylightFluorescent: uint32 = 12;
/// N  4600 - 5500K
pub const lsDayWhiteFluorescent: uint32 = 13;
/// W  3800 - 4500K
pub const lsCoolWhiteFluorescent: uint32 = 14;
/// WW 3250 - 3800K
pub const lsWhiteFluorescent: uint32 = 15;
/// L  2600 - 3250K
pub const lsWarmWhiteFluorescent: uint32 = 16;
pub const lsStandardLightA: uint32 = 17;
pub const lsStandardLightB: uint32 = 18;
pub const lsStandardLightC: uint32 = 19;
pub const lsD55: uint32 = 20;
pub const lsD65: uint32 = 21;
pub const lsD75: uint32 = 22;
pub const lsD50: uint32 = 23;
pub const lsISOStudioTungsten: uint32 = 24;
pub const lsOther: uint32 = 255;
pub const epUnidentified: uint32 = 0;
pub const epManual: uint32 = 1;
pub const epProgramNormal: uint32 = 2;
pub const epAperturePriority: uint32 = 3;
pub const epShutterPriority: uint32 = 4;
pub const epProgramCreative: uint32 = 5;
pub const epProgramAction: uint32 = 6;
pub const epPortraitMode: uint32 = 7;
pub const epLandscapeMode: uint32 = 8;
pub const mmUnidentified: uint32 = 0;
pub const mmAverage: uint32 = 1;
pub const mmCenterWeightedAverage: uint32 = 2;
pub const mmSpot: uint32 = 3;
pub const mmMultiSpot: uint32 = 4;
pub const mmPattern: uint32 = 5;
pub const mmPartial: uint32 = 6;
pub const mmOther: uint32 = 255;
pub const colorKeyRed: uint32 = 0;
pub const colorKeyGreen: uint32 = 1;
pub const colorKeyBlue: uint32 = 2;
pub const colorKeyCyan: uint32 = 3;
pub const colorKeyMagenta: uint32 = 4;
pub const colorKeyYellow: uint32 = 5;
pub const colorKeyWhite: uint32 = 6;
pub const colorKeyMaxEnum: uint32 = 255;
pub const stUnknown: uint32 = 0;
pub const stStandardOutputSensitivity: uint32 = 1;
pub const stRecommendedExposureIndex: uint32 = 2;
pub const stISOSpeed: uint32 = 3;
pub const stSOSandREI: uint32 = 4;
pub const stSOSandISOSpeed: uint32 = 5;
pub const stREIandISOSpeed: uint32 = 6;
pub const stSOSandREIandISOSpeed: uint32 = 7;
pub const crSceneReferred: uint32 = 0;
pub const crICCProfilePCS: uint32 = 1;
pub const crOutputReferredHDR: uint32 = 2;
pub const pepAllowCopying: uint32 = 0;
pub const pepEmbedIfUsed: uint32 = 1;
pub const pepEmbedNever: uint32 = 2;
pub const pepNoRestrictions: uint32 = 3;
pub const encoding_Linear: uint32 = 0;
pub const encoding_sRGB: uint32 = 1;
pub const defaultBlackRender_Auto: uint32 = 0;
pub const defaultBlackRender_None: uint32 = 1;
pub const profileToneMethod_Unspecified: uint32 = 0;
pub const profileToneMethod_AdobePV5: uint32 = 1;
pub const profileToneMethod_AdobePV6: uint32 = 2;
pub const previewColorSpace_Unknown: uint32 = 0;
pub const previewColorSpace_GrayGamma22: uint32 = 1;
pub const previewColorSpace_sRGB: uint32 = 2;
pub const previewColorSpace_AdobeRGB: uint32 = 3;
pub const previewColorSpace_ProPhotoRGB: uint32 = 4;
pub const previewColorSpace_LastValid: uint32 = 4;
pub const previewColorSpace_MaxEnum: uint32 = 4294967295;
pub const cacheVersionMask: uint32 = 65535;
pub const cacheVersionDefault: uint32 = 256;
pub const cacheVersionDefloated: uint32 = 65536;
pub const cacheVersionFlattened: uint32 = 131072;
pub const cacheVersionFakeMerge: uint32 = 262144;
pub const depthFormatUnknown: uint32 = 0;
pub const depthFormatLinear: uint32 = 1;
pub const depthFormatInverse: uint32 = 2;
pub const depthUnitsUnknown: uint32 = 0;
pub const depthUnitsMeters: uint32 = 1;
pub const depthMeasureUnknown: uint32 = 0;
pub const depthMeasureOpticalAxis: uint32 = 1;
pub const depthMeasureOpticalRay: uint32 = 2;
/// 'II'
pub const byteOrderII: uint32 = 18761;
/// 'MM'
pub const byteOrderMM: uint32 = 19789;
/// TIFF (and DNG)
pub const magicTIFF: uint32 = 42;
/// BigTIFF (and "BigDNG")
pub const magicBigTIFF: uint32 = 43;
/// 'CR'
pub const magicExtendedProfile: uint32 = 17234;
/// Raw cache (fast load data)
pub const magicRawCache: uint32 = 1022;
pub const magicPanasonic: uint32 = 85;
pub const magicOlympusA: uint32 = 20306;
pub const magicOlympusB: uint32 = 21330;
pub const dngVersion_None: uint32 = 0;
pub const dngVersion_1_0_0_0: uint32 = 16777216;
pub const dngVersion_1_1_0_0: uint32 = 16842752;
pub const dngVersion_1_2_0_0: uint32 = 16908288;
pub const dngVersion_1_3_0_0: uint32 = 16973824;
pub const dngVersion_1_4_0_0: uint32 = 17039360;
pub const dngVersion_1_5_0_0: uint32 = 17104896;
pub const dngVersion_1_6_0_0: uint32 = 17170432;
pub const dngVersion_1_7_0_0: uint32 = 17235968;
pub const dngVersion_1_7_1_0: uint32 = 17236224;
pub const dngVersion_Current: uint32 = 17236224;
pub const dngVersion_SaveDefault: uint32 = 17236224;

/// All transcribed constant names (used by tests to assert completeness).
#[cfg(test)]
const NAMES: &[&str] = &[
    "sfMainImage",
    "sfPreviewImage",
    "sfTransparencyMask",
    "sfPreviewMask",
    "sfDepthMap",
    "sfPreviewDepthMap",
    "sfEnhancedImage",
    "sfGainMap",
    "sfAltPreviewImage",
    "sfSemanticMask",
    "sfPreviewSemanticMask",
    "piWhiteIsZero",
    "piBlackIsZero",
    "piRGB",
    "piRGBPalette",
    "piTransparencyMask",
    "piCMYK",
    "piYCbCr",
    "piCIELab",
    "piICCLab",
    "piCFA",
    "piLinearRaw",
    "piDepth",
    "piPhotometricMask",
    "piGainMap",
    "pcInterleaved",
    "pcPlanar",
    "pcRowInterleaved",
    "pcRowInterleavedAlignSIMD",
    "esUnspecified",
    "esAssociatedAlpha",
    "esUnassociatedAlpha",
    "sfUnsignedInteger",
    "sfSignedInteger",
    "sfFloatingPoint",
    "sfUndefined",
    "ccUncompressed",
    "ccLZW",
    "ccOldJPEG",
    "ccJPEG",
    "ccDeflate",
    "ccVc5",
    "ccPackBits",
    "ccOldDeflate",
    "ccLossyJPEG",
    "ccJXL",
    "cpNullPredictor",
    "cpHorizontalDifference",
    "cpFloatingPoint",
    "cpHorizontalDifferenceX2",
    "cpHorizontalDifferenceX4",
    "cpFloatingPointX2",
    "cpFloatingPointX4",
    "ruNone",
    "ruInch",
    "ruCM",
    "ruMM",
    "ruMicroM",
    "lsUnknown",
    "lsDaylight",
    "lsFluorescent",
    "lsTungsten",
    "lsFlash",
    "lsFineWeather",
    "lsCloudyWeather",
    "lsShade",
    "lsDaylightFluorescent",
    "lsDayWhiteFluorescent",
    "lsCoolWhiteFluorescent",
    "lsWhiteFluorescent",
    "lsWarmWhiteFluorescent",
    "lsStandardLightA",
    "lsStandardLightB",
    "lsStandardLightC",
    "lsD55",
    "lsD65",
    "lsD75",
    "lsD50",
    "lsISOStudioTungsten",
    "lsOther",
    "epUnidentified",
    "epManual",
    "epProgramNormal",
    "epAperturePriority",
    "epShutterPriority",
    "epProgramCreative",
    "epProgramAction",
    "epPortraitMode",
    "epLandscapeMode",
    "mmUnidentified",
    "mmAverage",
    "mmCenterWeightedAverage",
    "mmSpot",
    "mmMultiSpot",
    "mmPattern",
    "mmPartial",
    "mmOther",
    "colorKeyRed",
    "colorKeyGreen",
    "colorKeyBlue",
    "colorKeyCyan",
    "colorKeyMagenta",
    "colorKeyYellow",
    "colorKeyWhite",
    "colorKeyMaxEnum",
    "stUnknown",
    "stStandardOutputSensitivity",
    "stRecommendedExposureIndex",
    "stISOSpeed",
    "stSOSandREI",
    "stSOSandISOSpeed",
    "stREIandISOSpeed",
    "stSOSandREIandISOSpeed",
    "crSceneReferred",
    "crICCProfilePCS",
    "crOutputReferredHDR",
    "pepAllowCopying",
    "pepEmbedIfUsed",
    "pepEmbedNever",
    "pepNoRestrictions",
    "encoding_Linear",
    "encoding_sRGB",
    "defaultBlackRender_Auto",
    "defaultBlackRender_None",
    "profileToneMethod_Unspecified",
    "profileToneMethod_AdobePV5",
    "profileToneMethod_AdobePV6",
    "previewColorSpace_Unknown",
    "previewColorSpace_GrayGamma22",
    "previewColorSpace_sRGB",
    "previewColorSpace_AdobeRGB",
    "previewColorSpace_ProPhotoRGB",
    "previewColorSpace_LastValid",
    "previewColorSpace_MaxEnum",
    "cacheVersionMask",
    "cacheVersionDefault",
    "cacheVersionDefloated",
    "cacheVersionFlattened",
    "cacheVersionFakeMerge",
    "depthFormatUnknown",
    "depthFormatLinear",
    "depthFormatInverse",
    "depthUnitsUnknown",
    "depthUnitsMeters",
    "depthMeasureUnknown",
    "depthMeasureOpticalAxis",
    "depthMeasureOpticalRay",
    "byteOrderII",
    "byteOrderMM",
    "magicTIFF",
    "magicBigTIFF",
    "magicExtendedProfile",
    "magicRawCache",
    "magicPanasonic",
    "magicOlympusA",
    "magicOlympusB",
    "dngVersion_None",
    "dngVersion_1_0_0_0",
    "dngVersion_1_1_0_0",
    "dngVersion_1_2_0_0",
    "dngVersion_1_3_0_0",
    "dngVersion_1_4_0_0",
    "dngVersion_1_5_0_0",
    "dngVersion_1_6_0_0",
    "dngVersion_1_7_0_0",
    "dngVersion_1_7_1_0",
    "dngVersion_Current",
    "dngVersion_SaveDefault",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_checks_match_cpp() {
        // NewSubFileType
        assert_eq!(sfMainImage, 0);
        assert_eq!(sfPreviewImage, 1);
        assert_eq!(sfTransparencyMask, 4);
        assert_eq!(sfPreviewMask, 5);
        assert_eq!(sfSemanticMask, 0x10004);

        // PhotometricInterpretation
        assert_eq!(piWhiteIsZero, 0);
        assert_eq!(piBlackIsZero, 1);
        assert_eq!(piRGB, 2);
        assert_eq!(piCFA, 32803);
        assert_eq!(piPhotometricMask, 52527);
        assert_eq!(piGainMap, 52553);

        // Compression
        assert_eq!(ccUncompressed, 1);
        assert_eq!(ccLZW, 5);
        assert_eq!(ccJPEG, 7);
        assert_eq!(ccDeflate, 8);
        assert_eq!(ccVc5, 9);
        assert_eq!(ccLossyJPEG, 34892);
        assert_eq!(ccJXL, 52546);

        // Sample format
        assert_eq!(sfUnsignedInteger, 1);
        assert_eq!(sfSignedInteger, 2);
        assert_eq!(sfFloatingPoint, 3);

        // DNG versions.
        assert_eq!(dngVersion_None, 0);
        assert_eq!(dngVersion_1_4_0_0, 0x01040000);
        assert_eq!(dngVersion_1_7_1_0, 0x01070100);

        // Internal planar configs.
        assert_eq!(pcRowInterleaved, 100000);
        assert_eq!(pcRowInterleavedAlignSIMD, 100001);

        // Preview color space enum.
        assert_eq!(previewColorSpace_sRGB, 2);
        assert_eq!(previewColorSpace_LastValid, 4);
    }

    #[test]
    fn no_duplicate_names() {
        // The transcription tool would have overwritten duplicates; verify
        // count matches the C++ header (168 entries in 1.7.1).
        let names: Vec<&str> = NAMES.iter().copied().collect();
        let set: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), set.len());
        assert_eq!(names.len(), 168);
    }
}

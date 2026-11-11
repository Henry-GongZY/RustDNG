// Port of dng_tag_codes.h — all tag code constants (tcXXX) and JPEG marker
// codes, transcribed verbatim by tools/transcribe_tags.py.
//
// PORT-NOTE(dng_tag_codes.h): the ARCHITECTURE.md contract mentions
// `dng_tag_code`/`dng_tag_signature` helper types; SDK 1.7.1 contains no
// such types (the header is enums only), so nothing further to port.

use crate::dng_types::uint32;

pub const tcNewSubFileType: uint32 = 254;
pub const tcSubFileType: uint32 = 255;
pub const tcImageWidth: uint32 = 256;
pub const tcImageLength: uint32 = 257;
pub const tcBitsPerSample: uint32 = 258;
pub const tcCompression: uint32 = 259;
pub const tcPhotometricInterpretation: uint32 = 262;
pub const tcThresholding: uint32 = 263;
pub const tcCellWidth: uint32 = 264;
pub const tcCellLength: uint32 = 265;
pub const tcFillOrder: uint32 = 266;
pub const tcImageDescription: uint32 = 270;
pub const tcMake: uint32 = 271;
pub const tcModel: uint32 = 272;
pub const tcStripOffsets: uint32 = 273;
pub const tcOrientation: uint32 = 274;
pub const tcSamplesPerPixel: uint32 = 277;
pub const tcRowsPerStrip: uint32 = 278;
pub const tcStripByteCounts: uint32 = 279;
pub const tcMinSampleValue: uint32 = 280;
pub const tcMaxSampleValue: uint32 = 281;
pub const tcXResolution: uint32 = 282;
pub const tcYResolution: uint32 = 283;
pub const tcPlanarConfiguration: uint32 = 284;
pub const tcFreeOffsets: uint32 = 285;
pub const tcFreeByteCounts: uint32 = 286;
pub const tcGrayResponseUnit: uint32 = 290;
pub const tcGrayResponseCurve: uint32 = 291;
pub const tcResolutionUnit: uint32 = 296;
pub const tcTransferFunction: uint32 = 301;
pub const tcSoftware: uint32 = 305;
pub const tcDateTime: uint32 = 306;
pub const tcArtist: uint32 = 315;
pub const tcHostComputer: uint32 = 316;
pub const tcPredictor: uint32 = 317;
pub const tcWhitePoint: uint32 = 318;
pub const tcPrimaryChromaticities: uint32 = 319;
pub const tcColorMap: uint32 = 320;
pub const tcTileWidth: uint32 = 322;
pub const tcTileLength: uint32 = 323;
pub const tcTileOffsets: uint32 = 324;
pub const tcTileByteCounts: uint32 = 325;
pub const tcSubIFDs: uint32 = 330;
pub const tcExtraSamples: uint32 = 338;
pub const tcSampleFormat: uint32 = 339;
pub const tcJPEGTables: uint32 = 347;
pub const tcJPEGProc: uint32 = 512;
pub const tcJPEGInterchangeFormat: uint32 = 513;
pub const tcJPEGInterchangeFormatLength: uint32 = 514;
pub const tcYCbCrCoefficients: uint32 = 529;
pub const tcYCbCrSubSampling: uint32 = 530;
pub const tcYCbCrPositioning: uint32 = 531;
pub const tcReferenceBlackWhite: uint32 = 532;
pub const tcXMP: uint32 = 700;
pub const tcKodakCameraSerialNumber: uint32 = 33405;
pub const tcCFARepeatPatternDim: uint32 = 33421;
pub const tcCFAPattern: uint32 = 33422;
pub const tcBatteryLevel: uint32 = 33423;
pub const tcKodakDCRPrivateIFD: uint32 = 33424;
pub const tcCopyright: uint32 = 33432;
pub const tcExposureTime: uint32 = 33434;
pub const tcFNumber: uint32 = 33437;
pub const tcIPTC_NAA: uint32 = 33723;
pub const tcLeafPKTS: uint32 = 34310;
pub const tcAdobeData: uint32 = 34377;
pub const tcExifIFD: uint32 = 34665;
pub const tcICCProfile: uint32 = 34675;
pub const tcExposureProgram: uint32 = 34850;
pub const tcSpectralSensitivity: uint32 = 34852;
pub const tcGPSInfo: uint32 = 34853;
/// EXIF 2.3: PhotographicSensitivity.
pub const tcISOSpeedRatings: uint32 = 34855;
pub const tcOECF: uint32 = 34856;
pub const tcInterlace: uint32 = 34857;
pub const tcTimeZoneOffset: uint32 = 34858;
pub const tcSelfTimerMode: uint32 = 34859;
pub const tcSensitivityType: uint32 = 34864;
pub const tcStandardOutputSensitivity: uint32 = 34865;
pub const tcRecommendedExposureIndex: uint32 = 34866;
pub const tcISOSpeed: uint32 = 34867;
pub const tcISOSpeedLatitudeyyy: uint32 = 34868;
pub const tcISOSpeedLatitudezzz: uint32 = 34869;
pub const tcExifVersion: uint32 = 36864;
pub const tcDateTimeOriginal: uint32 = 36867;
pub const tcDateTimeDigitized: uint32 = 36868;
pub const tcOffsetTime: uint32 = 36880;
pub const tcOffsetTimeOriginal: uint32 = 36881;
pub const tcOffsetTimeDigitized: uint32 = 36882;
pub const tcComponentsConfiguration: uint32 = 37121;
pub const tcCompressedBitsPerPixel: uint32 = 37122;
pub const tcShutterSpeedValue: uint32 = 37377;
pub const tcApertureValue: uint32 = 37378;
pub const tcBrightnessValue: uint32 = 37379;
pub const tcExposureBiasValue: uint32 = 37380;
pub const tcMaxApertureValue: uint32 = 37381;
pub const tcSubjectDistance: uint32 = 37382;
pub const tcMeteringMode: uint32 = 37383;
pub const tcLightSource: uint32 = 37384;
pub const tcFlash: uint32 = 37385;
pub const tcFocalLength: uint32 = 37386;
pub const tcFlashEnergy: uint32 = 37387;
pub const tcSpatialFrequencyResponse: uint32 = 37388;
pub const tcNoise: uint32 = 37389;
pub const tcFocalPlaneXResolution: uint32 = 37390;
pub const tcFocalPlaneYResolution: uint32 = 37391;
pub const tcFocalPlaneResolutionUnit: uint32 = 37392;
pub const tcImageNumber: uint32 = 37393;
pub const tcSecurityClassification: uint32 = 37394;
pub const tcImageHistory: uint32 = 37395;
pub const tcSubjectArea: uint32 = 37396;
pub const tcExposureIndex: uint32 = 37397;
pub const tcTIFF_EP_StandardID: uint32 = 37398;
pub const tcSensingMethod: uint32 = 37399;
pub const tcMakerNote: uint32 = 37500;
pub const tcUserComment: uint32 = 37510;
pub const tcSubsecTime: uint32 = 37520;
pub const tcSubsecTimeOriginal: uint32 = 37521;
pub const tcSubsecTimeDigitized: uint32 = 37522;
pub const tcAdobeLayerData: uint32 = 37724;
pub const tcTemperature: uint32 = 37888;
pub const tcHumidity: uint32 = 37889;
pub const tcPressure: uint32 = 37890;
pub const tcWaterDepth: uint32 = 37891;
pub const tcAcceleration: uint32 = 37892;
pub const tcCameraElevationAngle: uint32 = 37893;
pub const tcFlashPixVersion: uint32 = 40960;
pub const tcColorSpace: uint32 = 40961;
pub const tcPixelXDimension: uint32 = 40962;
pub const tcPixelYDimension: uint32 = 40963;
pub const tcRelatedSoundFile: uint32 = 40964;
pub const tcInteroperabilityIFD: uint32 = 40965;
pub const tcFlashEnergyExif: uint32 = 41483;
pub const tcSpatialFrequencyResponseExif: uint32 = 41484;
pub const tcFocalPlaneXResolutionExif: uint32 = 41486;
pub const tcFocalPlaneYResolutionExif: uint32 = 41487;
pub const tcFocalPlaneResolutionUnitExif: uint32 = 41488;
pub const tcSubjectLocation: uint32 = 41492;
pub const tcExposureIndexExif: uint32 = 41493;
pub const tcSensingMethodExif: uint32 = 41495;
pub const tcFileSource: uint32 = 41728;
pub const tcSceneType: uint32 = 41729;
pub const tcCFAPatternExif: uint32 = 41730;
pub const tcCustomRendered: uint32 = 41985;
pub const tcExposureMode: uint32 = 41986;
pub const tcWhiteBalance: uint32 = 41987;
pub const tcDigitalZoomRatio: uint32 = 41988;
pub const tcFocalLengthIn35mmFilm: uint32 = 41989;
pub const tcSceneCaptureType: uint32 = 41990;
pub const tcGainControl: uint32 = 41991;
pub const tcContrast: uint32 = 41992;
pub const tcSaturation: uint32 = 41993;
pub const tcSharpness: uint32 = 41994;
pub const tcDeviceSettingDescription: uint32 = 41995;
pub const tcSubjectDistanceRange: uint32 = 41996;
pub const tcImageUniqueID: uint32 = 42016;
pub const tcCameraOwnerNameExif: uint32 = 42032;
pub const tcCameraSerialNumberExif: uint32 = 42033;
pub const tcLensSpecificationExif: uint32 = 42034;
pub const tcLensMakeExif: uint32 = 42035;
pub const tcLensModelExif: uint32 = 42036;
pub const tcLensSerialNumberExif: uint32 = 42037;
pub const tcGamma: uint32 = 42240;
pub const tcPrintImageMatchingInfo: uint32 = 50341;
pub const tcDNGVersion: uint32 = 50706;
pub const tcDNGBackwardVersion: uint32 = 50707;
pub const tcUniqueCameraModel: uint32 = 50708;
pub const tcLocalizedCameraModel: uint32 = 50709;
pub const tcCFAPlaneColor: uint32 = 50710;
pub const tcCFALayout: uint32 = 50711;
pub const tcLinearizationTable: uint32 = 50712;
pub const tcBlackLevelRepeatDim: uint32 = 50713;
pub const tcBlackLevel: uint32 = 50714;
pub const tcBlackLevelDeltaH: uint32 = 50715;
pub const tcBlackLevelDeltaV: uint32 = 50716;
pub const tcWhiteLevel: uint32 = 50717;
pub const tcDefaultScale: uint32 = 50718;
pub const tcDefaultCropOrigin: uint32 = 50719;
pub const tcDefaultCropSize: uint32 = 50720;
pub const tcColorMatrix1: uint32 = 50721;
pub const tcColorMatrix2: uint32 = 50722;
pub const tcCameraCalibration1: uint32 = 50723;
pub const tcCameraCalibration2: uint32 = 50724;
pub const tcReductionMatrix1: uint32 = 50725;
pub const tcReductionMatrix2: uint32 = 50726;
pub const tcAnalogBalance: uint32 = 50727;
pub const tcAsShotNeutral: uint32 = 50728;
pub const tcAsShotWhiteXY: uint32 = 50729;
pub const tcBaselineExposure: uint32 = 50730;
pub const tcBaselineNoise: uint32 = 50731;
pub const tcBaselineSharpness: uint32 = 50732;
pub const tcBayerGreenSplit: uint32 = 50733;
pub const tcLinearResponseLimit: uint32 = 50734;
pub const tcCameraSerialNumber: uint32 = 50735;
pub const tcLensInfo: uint32 = 50736;
pub const tcChromaBlurRadius: uint32 = 50737;
pub const tcAntiAliasStrength: uint32 = 50738;
pub const tcShadowScale: uint32 = 50739;
pub const tcDNGPrivateData: uint32 = 50740;
pub const tcMakerNoteSafety: uint32 = 50741;
pub const tcCalibrationIlluminant1: uint32 = 50778;
pub const tcCalibrationIlluminant2: uint32 = 50779;
pub const tcBestQualityScale: uint32 = 50780;
pub const tcRawDataUniqueID: uint32 = 50781;
pub const tcOriginalRawFileName: uint32 = 50827;
pub const tcOriginalRawFileData: uint32 = 50828;
pub const tcActiveArea: uint32 = 50829;
pub const tcMaskedAreas: uint32 = 50830;
pub const tcAsShotICCProfile: uint32 = 50831;
pub const tcAsShotPreProfileMatrix: uint32 = 50832;
pub const tcCurrentICCProfile: uint32 = 50833;
pub const tcCurrentPreProfileMatrix: uint32 = 50834;
pub const tcColorimetricReference: uint32 = 50879;
pub const tcCameraCalibrationSignature: uint32 = 50931;
pub const tcProfileCalibrationSignature: uint32 = 50932;
pub const tcExtraCameraProfiles: uint32 = 50933;
pub const tcAsShotProfileName: uint32 = 50934;
pub const tcNoiseReductionApplied: uint32 = 50935;
pub const tcProfileName: uint32 = 50936;
pub const tcProfileHueSatMapDims: uint32 = 50937;
pub const tcProfileHueSatMapData1: uint32 = 50938;
pub const tcProfileHueSatMapData2: uint32 = 50939;
pub const tcProfileToneCurve: uint32 = 50940;
pub const tcProfileEmbedPolicy: uint32 = 50941;
pub const tcProfileCopyright: uint32 = 50942;
pub const tcForwardMatrix1: uint32 = 50964;
pub const tcForwardMatrix2: uint32 = 50965;
pub const tcPreviewApplicationName: uint32 = 50966;
pub const tcPreviewApplicationVersion: uint32 = 50967;
pub const tcPreviewSettingsName: uint32 = 50968;
pub const tcPreviewSettingsDigest: uint32 = 50969;
pub const tcPreviewColorSpace: uint32 = 50970;
pub const tcPreviewDateTime: uint32 = 50971;
pub const tcRawImageDigest: uint32 = 50972;
pub const tcOriginalRawFileDigest: uint32 = 50973;
pub const tcSubTileBlockSize: uint32 = 50974;
pub const tcRowInterleaveFactor: uint32 = 50975;
pub const tcProfileLookTableDims: uint32 = 50981;
pub const tcProfileLookTableData: uint32 = 50982;
pub const tcOpcodeList1: uint32 = 51008;
pub const tcOpcodeList2: uint32 = 51009;
pub const tcOpcodeList3: uint32 = 51022;
pub const tcNoiseProfile: uint32 = 51041;
pub const tcOriginalDefaultFinalSize: uint32 = 51089;
pub const tcOriginalBestQualityFinalSize: uint32 = 51090;
pub const tcOriginalDefaultCropSize: uint32 = 51091;
pub const tcProfileHueSatMapEncoding: uint32 = 51107;
pub const tcProfileLookTableEncoding: uint32 = 51108;
pub const tcBaselineExposureOffset: uint32 = 51109;
pub const tcDefaultBlackRender: uint32 = 51110;
pub const tcNewRawImageDigest: uint32 = 51111;
pub const tcRawToPreviewGain: uint32 = 51112;
pub const tcCacheBlob: uint32 = 51113;
pub const tcCacheVersion: uint32 = 51114;
pub const tcDefaultUserCrop: uint32 = 51125;
pub const tcDepthFormat: uint32 = 51177;
pub const tcDepthNear: uint32 = 51178;
pub const tcDepthFar: uint32 = 51179;
pub const tcDepthUnits: uint32 = 51180;
pub const tcDepthMeasureType: uint32 = 51181;
pub const tcEnhanceParams: uint32 = 51182;
/// Added in DNG 1.6
pub const tcProfileGainTableMap: uint32 = 52525;
/// Added in DNG 1.6
pub const tcSemanticName: uint32 = 52526;
/// Added in DNG 1.6
pub const tcSemanticInstanceID: uint32 = 52528;
pub const tcCalibrationIlluminant3: uint32 = 52529;
pub const tcCameraCalibration3: uint32 = 52530;
pub const tcColorMatrix3: uint32 = 52531;
pub const tcForwardMatrix3: uint32 = 52532;
pub const tcIlluminantData1: uint32 = 52533;
pub const tcIlluminantData2: uint32 = 52534;
pub const tcIlluminantData3: uint32 = 52535;
pub const tcMaskSubArea: uint32 = 52536;
pub const tcProfileHueSatMapData3: uint32 = 52537;
pub const tcReductionMatrix3: uint32 = 52538;
/// Added in earlier draft of DNG 1.6
pub const tcRGBTablesDraft: uint32 = 52539;
/// Added in DNG 1.6
pub const tcRGBTables: uint32 = 52543;
pub const tcBigTableDigests: uint32 = 52540;
pub const tcBigTableOffsets: uint32 = 52541;
pub const tcBigTableByteCounts: uint32 = 52542;
pub const tcProfileGainTableMap2: uint32 = 52544;
pub const tcColumnInterleaveFactor: uint32 = 52547;
pub const tcImageSequenceInfo: uint32 = 52548;
pub const tcProfileToneMethod: uint32 = 52549;
pub const tcImageStats: uint32 = 52550;
pub const tcProfileDynamicRange: uint32 = 52551;
pub const tcProfileGroupName: uint32 = 52552;
pub const tcJXLDistance: uint32 = 52553;
pub const tcJXLEffort: uint32 = 52554;
pub const tcJXLDecodeSpeed: uint32 = 52555;
pub const tcBigTableGroupIndex: uint32 = 52556;
pub const tcKodakKDCPrivateIFD: uint32 = 65024;
pub const tcFirstSubIFD: uint32 = 65536;
pub const tcLastSubIFD: uint32 = 131071;
pub const tcFirstChainedIFD: uint32 = 131072;
pub const tcLastChainedIFD: uint32 = 196607;
pub const tcFirstMakerNoteIFD: uint32 = 196608;
pub const tcLastMakerNoteIFD: uint32 = 262143;
pub const tcAppleMakerNote: uint32 = 196608;
pub const tcCanonMakerNote: uint32 = 196609;
pub const tcCasioMakerNote: uint32 = 196610;
pub const tcEpsonMakerNote: uint32 = 196611;
pub const tcFujiMakerNote: uint32 = 196612;
pub const tcHasselbladMakerNote: uint32 = 196613;
pub const tcKodakMakerNote: uint32 = 196614;
pub const tcKodakMakerNote65280: uint32 = 196615;
pub const tcLeicaMakerNote: uint32 = 196616;
pub const tcMamiyaMakerNote: uint32 = 196617;
pub const tcMinoltaMakerNote: uint32 = 196618;
pub const tcNikonMakerNote: uint32 = 196619;
pub const tcOlympusMakerNote: uint32 = 196620;
pub const tcOlympusMakerNote8208: uint32 = 196621;
pub const tcOlympusMakerNote8224: uint32 = 196622;
pub const tcOlympusMakerNote8240: uint32 = 196623;
pub const tcOlympusMakerNote8256: uint32 = 196624;
pub const tcOlympusMakerNote8272: uint32 = 196625;
pub const tcOlympusMakerNote12288: uint32 = 196626;
pub const tcPanasonicMakerNote: uint32 = 196627;
pub const tcPentaxMakerNote: uint32 = 196628;
pub const tcPhaseOneMakerNote: uint32 = 196629;
pub const tcRicohMakerNote: uint32 = 196630;
pub const tcRicohMakerNoteCameraInfo: uint32 = 196631;
pub const tcSamsungMakerNote: uint32 = 196632;
pub const tcSIGMAMakerNote: uint32 = 196633;
pub const tcSonyMakerNote: uint32 = 196634;
pub const tcSonyMakerNoteSubInfo: uint32 = 196635;
pub const tcSonyPrivateIFD1: uint32 = 196636;
pub const tcSonyPrivateIFD2: uint32 = 196637;
pub const tcSonyPrivateIFD3A: uint32 = 196638;
pub const tcSonyPrivateIFD3B: uint32 = 196639;
pub const tcSonyPrivateIFD3C: uint32 = 196640;
pub const tcCanonCRW: uint32 = 262144;
pub const tcContaxRAW: uint32 = 262145;
pub const tcContaxHeader: uint32 = 262146;
pub const tcFujiRAF: uint32 = 262147;
pub const tcFujiHeader: uint32 = 262148;
pub const tcFujiRawInfo1: uint32 = 262149;
pub const tcFujiRawInfo2: uint32 = 262150;
pub const tcLeafMOS: uint32 = 262151;
pub const tcMinoltaMRW: uint32 = 262152;
pub const tcPanasonicRAW: uint32 = 262153;
pub const tcFoveonX3F: uint32 = 262154;
pub const tcJPEG: uint32 = 262155;
pub const tcAdobePSD: uint32 = 262156;
pub const tcPNG: uint32 = 262157;
pub const tcHEIC: uint32 = 262158;
pub const tcCanonCR3: uint32 = 262159;
pub const tcJXL: uint32 = 262160;
pub const tcAVIF: uint32 = 262161;
pub const tcGPSVersionID: uint32 = 0;
pub const tcGPSLatitudeRef: uint32 = 1;
pub const tcGPSLatitude: uint32 = 2;
pub const tcGPSLongitudeRef: uint32 = 3;
pub const tcGPSLongitude: uint32 = 4;
pub const tcGPSAltitudeRef: uint32 = 5;
pub const tcGPSAltitude: uint32 = 6;
pub const tcGPSTimeStamp: uint32 = 7;
pub const tcGPSSatellites: uint32 = 8;
pub const tcGPSStatus: uint32 = 9;
pub const tcGPSMeasureMode: uint32 = 10;
pub const tcGPSDOP: uint32 = 11;
pub const tcGPSSpeedRef: uint32 = 12;
pub const tcGPSSpeed: uint32 = 13;
pub const tcGPSTrackRef: uint32 = 14;
pub const tcGPSTrack: uint32 = 15;
pub const tcGPSImgDirectionRef: uint32 = 16;
pub const tcGPSImgDirection: uint32 = 17;
pub const tcGPSMapDatum: uint32 = 18;
pub const tcGPSDestLatitudeRef: uint32 = 19;
pub const tcGPSDestLatitude: uint32 = 20;
pub const tcGPSDestLongitudeRef: uint32 = 21;
pub const tcGPSDestLongitude: uint32 = 22;
pub const tcGPSDestBearingRef: uint32 = 23;
pub const tcGPSDestBearing: uint32 = 24;
pub const tcGPSDestDistanceRef: uint32 = 25;
pub const tcGPSDestDistance: uint32 = 26;
pub const tcGPSProcessingMethod: uint32 = 27;
pub const tcGPSAreaInformation: uint32 = 28;
pub const tcGPSDateStamp: uint32 = 29;
pub const tcGPSDifferential: uint32 = 30;
pub const tcGPSHPositioningError: uint32 = 31;
pub const tcInteroperabilityIndex: uint32 = 1;
pub const tcInteroperabilityVersion: uint32 = 2;
pub const tcRelatedImageFileFormat: uint32 = 4096;
pub const tcRelatedImageWidth: uint32 = 4097;
pub const tcRelatedImageLength: uint32 = 4098;
pub const M_TEM: uint32 = 1;
pub const M_SOF0: uint32 = 192;
pub const M_SOF1: uint32 = 193;
pub const M_SOF2: uint32 = 194;
pub const M_SOF3: uint32 = 195;
pub const M_DHT: uint32 = 196;
pub const M_SOF5: uint32 = 197;
pub const M_SOF6: uint32 = 198;
pub const M_SOF7: uint32 = 199;
pub const M_JPG: uint32 = 200;
pub const M_SOF9: uint32 = 201;
pub const M_SOF10: uint32 = 202;
pub const M_SOF11: uint32 = 203;
pub const M_DAC: uint32 = 204;
pub const M_SOF13: uint32 = 205;
pub const M_SOF14: uint32 = 206;
pub const M_SOF15: uint32 = 207;
pub const M_RST0: uint32 = 208;
pub const M_RST1: uint32 = 209;
pub const M_RST2: uint32 = 210;
pub const M_RST3: uint32 = 211;
pub const M_RST4: uint32 = 212;
pub const M_RST5: uint32 = 213;
pub const M_RST6: uint32 = 214;
pub const M_RST7: uint32 = 215;
pub const M_SOI: uint32 = 216;
pub const M_EOI: uint32 = 217;
pub const M_SOS: uint32 = 218;
pub const M_DQT: uint32 = 219;
pub const M_DNL: uint32 = 220;
pub const M_DRI: uint32 = 221;
pub const M_DHP: uint32 = 222;
pub const M_EXP: uint32 = 223;
pub const M_APP0: uint32 = 224;
pub const M_APP1: uint32 = 225;
pub const M_APP2: uint32 = 226;
pub const M_APP3: uint32 = 227;
pub const M_APP4: uint32 = 228;
pub const M_APP5: uint32 = 229;
pub const M_APP6: uint32 = 230;
pub const M_APP7: uint32 = 231;
pub const M_APP8: uint32 = 232;
pub const M_APP9: uint32 = 233;
pub const M_APP10: uint32 = 234;
pub const M_APP11: uint32 = 235;
pub const M_APP12: uint32 = 236;
pub const M_APP13: uint32 = 237;
pub const M_APP14: uint32 = 238;
pub const M_APP15: uint32 = 239;
pub const M_JPG0: uint32 = 240;
pub const M_JPG1: uint32 = 241;
pub const M_JPG2: uint32 = 242;
pub const M_JPG3: uint32 = 243;
pub const M_JPG4: uint32 = 244;
pub const M_JPG5: uint32 = 245;
pub const M_JPG6: uint32 = 246;
pub const M_JPG7: uint32 = 247;
pub const M_JPG8: uint32 = 248;
pub const M_JPG9: uint32 = 249;
pub const M_JPG10: uint32 = 250;
pub const M_JPG11: uint32 = 251;
pub const M_JPG12: uint32 = 252;
pub const M_JPG13: uint32 = 253;
pub const M_COM: uint32 = 254;
pub const M_ERROR: uint32 = 256;

/// All transcribed constant names (used by tests to assert completeness).
#[cfg(test)]
const NAMES: &[&str] = &[
    "tcNewSubFileType",
    "tcSubFileType",
    "tcImageWidth",
    "tcImageLength",
    "tcBitsPerSample",
    "tcCompression",
    "tcPhotometricInterpretation",
    "tcThresholding",
    "tcCellWidth",
    "tcCellLength",
    "tcFillOrder",
    "tcImageDescription",
    "tcMake",
    "tcModel",
    "tcStripOffsets",
    "tcOrientation",
    "tcSamplesPerPixel",
    "tcRowsPerStrip",
    "tcStripByteCounts",
    "tcMinSampleValue",
    "tcMaxSampleValue",
    "tcXResolution",
    "tcYResolution",
    "tcPlanarConfiguration",
    "tcFreeOffsets",
    "tcFreeByteCounts",
    "tcGrayResponseUnit",
    "tcGrayResponseCurve",
    "tcResolutionUnit",
    "tcTransferFunction",
    "tcSoftware",
    "tcDateTime",
    "tcArtist",
    "tcHostComputer",
    "tcPredictor",
    "tcWhitePoint",
    "tcPrimaryChromaticities",
    "tcColorMap",
    "tcTileWidth",
    "tcTileLength",
    "tcTileOffsets",
    "tcTileByteCounts",
    "tcSubIFDs",
    "tcExtraSamples",
    "tcSampleFormat",
    "tcJPEGTables",
    "tcJPEGProc",
    "tcJPEGInterchangeFormat",
    "tcJPEGInterchangeFormatLength",
    "tcYCbCrCoefficients",
    "tcYCbCrSubSampling",
    "tcYCbCrPositioning",
    "tcReferenceBlackWhite",
    "tcXMP",
    "tcKodakCameraSerialNumber",
    "tcCFARepeatPatternDim",
    "tcCFAPattern",
    "tcBatteryLevel",
    "tcKodakDCRPrivateIFD",
    "tcCopyright",
    "tcExposureTime",
    "tcFNumber",
    "tcIPTC_NAA",
    "tcLeafPKTS",
    "tcAdobeData",
    "tcExifIFD",
    "tcICCProfile",
    "tcExposureProgram",
    "tcSpectralSensitivity",
    "tcGPSInfo",
    "tcISOSpeedRatings",
    "tcOECF",
    "tcInterlace",
    "tcTimeZoneOffset",
    "tcSelfTimerMode",
    "tcSensitivityType",
    "tcStandardOutputSensitivity",
    "tcRecommendedExposureIndex",
    "tcISOSpeed",
    "tcISOSpeedLatitudeyyy",
    "tcISOSpeedLatitudezzz",
    "tcExifVersion",
    "tcDateTimeOriginal",
    "tcDateTimeDigitized",
    "tcOffsetTime",
    "tcOffsetTimeOriginal",
    "tcOffsetTimeDigitized",
    "tcComponentsConfiguration",
    "tcCompressedBitsPerPixel",
    "tcShutterSpeedValue",
    "tcApertureValue",
    "tcBrightnessValue",
    "tcExposureBiasValue",
    "tcMaxApertureValue",
    "tcSubjectDistance",
    "tcMeteringMode",
    "tcLightSource",
    "tcFlash",
    "tcFocalLength",
    "tcFlashEnergy",
    "tcSpatialFrequencyResponse",
    "tcNoise",
    "tcFocalPlaneXResolution",
    "tcFocalPlaneYResolution",
    "tcFocalPlaneResolutionUnit",
    "tcImageNumber",
    "tcSecurityClassification",
    "tcImageHistory",
    "tcSubjectArea",
    "tcExposureIndex",
    "tcTIFF_EP_StandardID",
    "tcSensingMethod",
    "tcMakerNote",
    "tcUserComment",
    "tcSubsecTime",
    "tcSubsecTimeOriginal",
    "tcSubsecTimeDigitized",
    "tcAdobeLayerData",
    "tcTemperature",
    "tcHumidity",
    "tcPressure",
    "tcWaterDepth",
    "tcAcceleration",
    "tcCameraElevationAngle",
    "tcFlashPixVersion",
    "tcColorSpace",
    "tcPixelXDimension",
    "tcPixelYDimension",
    "tcRelatedSoundFile",
    "tcInteroperabilityIFD",
    "tcFlashEnergyExif",
    "tcSpatialFrequencyResponseExif",
    "tcFocalPlaneXResolutionExif",
    "tcFocalPlaneYResolutionExif",
    "tcFocalPlaneResolutionUnitExif",
    "tcSubjectLocation",
    "tcExposureIndexExif",
    "tcSensingMethodExif",
    "tcFileSource",
    "tcSceneType",
    "tcCFAPatternExif",
    "tcCustomRendered",
    "tcExposureMode",
    "tcWhiteBalance",
    "tcDigitalZoomRatio",
    "tcFocalLengthIn35mmFilm",
    "tcSceneCaptureType",
    "tcGainControl",
    "tcContrast",
    "tcSaturation",
    "tcSharpness",
    "tcDeviceSettingDescription",
    "tcSubjectDistanceRange",
    "tcImageUniqueID",
    "tcCameraOwnerNameExif",
    "tcCameraSerialNumberExif",
    "tcLensSpecificationExif",
    "tcLensMakeExif",
    "tcLensModelExif",
    "tcLensSerialNumberExif",
    "tcGamma",
    "tcPrintImageMatchingInfo",
    "tcDNGVersion",
    "tcDNGBackwardVersion",
    "tcUniqueCameraModel",
    "tcLocalizedCameraModel",
    "tcCFAPlaneColor",
    "tcCFALayout",
    "tcLinearizationTable",
    "tcBlackLevelRepeatDim",
    "tcBlackLevel",
    "tcBlackLevelDeltaH",
    "tcBlackLevelDeltaV",
    "tcWhiteLevel",
    "tcDefaultScale",
    "tcDefaultCropOrigin",
    "tcDefaultCropSize",
    "tcColorMatrix1",
    "tcColorMatrix2",
    "tcCameraCalibration1",
    "tcCameraCalibration2",
    "tcReductionMatrix1",
    "tcReductionMatrix2",
    "tcAnalogBalance",
    "tcAsShotNeutral",
    "tcAsShotWhiteXY",
    "tcBaselineExposure",
    "tcBaselineNoise",
    "tcBaselineSharpness",
    "tcBayerGreenSplit",
    "tcLinearResponseLimit",
    "tcCameraSerialNumber",
    "tcLensInfo",
    "tcChromaBlurRadius",
    "tcAntiAliasStrength",
    "tcShadowScale",
    "tcDNGPrivateData",
    "tcMakerNoteSafety",
    "tcCalibrationIlluminant1",
    "tcCalibrationIlluminant2",
    "tcBestQualityScale",
    "tcRawDataUniqueID",
    "tcOriginalRawFileName",
    "tcOriginalRawFileData",
    "tcActiveArea",
    "tcMaskedAreas",
    "tcAsShotICCProfile",
    "tcAsShotPreProfileMatrix",
    "tcCurrentICCProfile",
    "tcCurrentPreProfileMatrix",
    "tcColorimetricReference",
    "tcCameraCalibrationSignature",
    "tcProfileCalibrationSignature",
    "tcExtraCameraProfiles",
    "tcAsShotProfileName",
    "tcNoiseReductionApplied",
    "tcProfileName",
    "tcProfileHueSatMapDims",
    "tcProfileHueSatMapData1",
    "tcProfileHueSatMapData2",
    "tcProfileToneCurve",
    "tcProfileEmbedPolicy",
    "tcProfileCopyright",
    "tcForwardMatrix1",
    "tcForwardMatrix2",
    "tcPreviewApplicationName",
    "tcPreviewApplicationVersion",
    "tcPreviewSettingsName",
    "tcPreviewSettingsDigest",
    "tcPreviewColorSpace",
    "tcPreviewDateTime",
    "tcRawImageDigest",
    "tcOriginalRawFileDigest",
    "tcSubTileBlockSize",
    "tcRowInterleaveFactor",
    "tcProfileLookTableDims",
    "tcProfileLookTableData",
    "tcOpcodeList1",
    "tcOpcodeList2",
    "tcOpcodeList3",
    "tcNoiseProfile",
    "tcOriginalDefaultFinalSize",
    "tcOriginalBestQualityFinalSize",
    "tcOriginalDefaultCropSize",
    "tcProfileHueSatMapEncoding",
    "tcProfileLookTableEncoding",
    "tcBaselineExposureOffset",
    "tcDefaultBlackRender",
    "tcNewRawImageDigest",
    "tcRawToPreviewGain",
    "tcCacheBlob",
    "tcCacheVersion",
    "tcDefaultUserCrop",
    "tcDepthFormat",
    "tcDepthNear",
    "tcDepthFar",
    "tcDepthUnits",
    "tcDepthMeasureType",
    "tcEnhanceParams",
    "tcProfileGainTableMap",
    "tcSemanticName",
    "tcSemanticInstanceID",
    "tcCalibrationIlluminant3",
    "tcCameraCalibration3",
    "tcColorMatrix3",
    "tcForwardMatrix3",
    "tcIlluminantData1",
    "tcIlluminantData2",
    "tcIlluminantData3",
    "tcMaskSubArea",
    "tcProfileHueSatMapData3",
    "tcReductionMatrix3",
    "tcRGBTablesDraft",
    "tcRGBTables",
    "tcBigTableDigests",
    "tcBigTableOffsets",
    "tcBigTableByteCounts",
    "tcProfileGainTableMap2",
    "tcColumnInterleaveFactor",
    "tcImageSequenceInfo",
    "tcProfileToneMethod",
    "tcImageStats",
    "tcProfileDynamicRange",
    "tcProfileGroupName",
    "tcJXLDistance",
    "tcJXLEffort",
    "tcJXLDecodeSpeed",
    "tcBigTableGroupIndex",
    "tcKodakKDCPrivateIFD",
    "tcFirstSubIFD",
    "tcLastSubIFD",
    "tcFirstChainedIFD",
    "tcLastChainedIFD",
    "tcFirstMakerNoteIFD",
    "tcLastMakerNoteIFD",
    "tcAppleMakerNote",
    "tcCanonMakerNote",
    "tcCasioMakerNote",
    "tcEpsonMakerNote",
    "tcFujiMakerNote",
    "tcHasselbladMakerNote",
    "tcKodakMakerNote",
    "tcKodakMakerNote65280",
    "tcLeicaMakerNote",
    "tcMamiyaMakerNote",
    "tcMinoltaMakerNote",
    "tcNikonMakerNote",
    "tcOlympusMakerNote",
    "tcOlympusMakerNote8208",
    "tcOlympusMakerNote8224",
    "tcOlympusMakerNote8240",
    "tcOlympusMakerNote8256",
    "tcOlympusMakerNote8272",
    "tcOlympusMakerNote12288",
    "tcPanasonicMakerNote",
    "tcPentaxMakerNote",
    "tcPhaseOneMakerNote",
    "tcRicohMakerNote",
    "tcRicohMakerNoteCameraInfo",
    "tcSamsungMakerNote",
    "tcSIGMAMakerNote",
    "tcSonyMakerNote",
    "tcSonyMakerNoteSubInfo",
    "tcSonyPrivateIFD1",
    "tcSonyPrivateIFD2",
    "tcSonyPrivateIFD3A",
    "tcSonyPrivateIFD3B",
    "tcSonyPrivateIFD3C",
    "tcCanonCRW",
    "tcContaxRAW",
    "tcContaxHeader",
    "tcFujiRAF",
    "tcFujiHeader",
    "tcFujiRawInfo1",
    "tcFujiRawInfo2",
    "tcLeafMOS",
    "tcMinoltaMRW",
    "tcPanasonicRAW",
    "tcFoveonX3F",
    "tcJPEG",
    "tcAdobePSD",
    "tcPNG",
    "tcHEIC",
    "tcCanonCR3",
    "tcJXL",
    "tcAVIF",
    "tcGPSVersionID",
    "tcGPSLatitudeRef",
    "tcGPSLatitude",
    "tcGPSLongitudeRef",
    "tcGPSLongitude",
    "tcGPSAltitudeRef",
    "tcGPSAltitude",
    "tcGPSTimeStamp",
    "tcGPSSatellites",
    "tcGPSStatus",
    "tcGPSMeasureMode",
    "tcGPSDOP",
    "tcGPSSpeedRef",
    "tcGPSSpeed",
    "tcGPSTrackRef",
    "tcGPSTrack",
    "tcGPSImgDirectionRef",
    "tcGPSImgDirection",
    "tcGPSMapDatum",
    "tcGPSDestLatitudeRef",
    "tcGPSDestLatitude",
    "tcGPSDestLongitudeRef",
    "tcGPSDestLongitude",
    "tcGPSDestBearingRef",
    "tcGPSDestBearing",
    "tcGPSDestDistanceRef",
    "tcGPSDestDistance",
    "tcGPSProcessingMethod",
    "tcGPSAreaInformation",
    "tcGPSDateStamp",
    "tcGPSDifferential",
    "tcGPSHPositioningError",
    "tcInteroperabilityIndex",
    "tcInteroperabilityVersion",
    "tcRelatedImageFileFormat",
    "tcRelatedImageWidth",
    "tcRelatedImageLength",
    "M_TEM",
    "M_SOF0",
    "M_SOF1",
    "M_SOF2",
    "M_SOF3",
    "M_DHT",
    "M_SOF5",
    "M_SOF6",
    "M_SOF7",
    "M_JPG",
    "M_SOF9",
    "M_SOF10",
    "M_SOF11",
    "M_DAC",
    "M_SOF13",
    "M_SOF14",
    "M_SOF15",
    "M_RST0",
    "M_RST1",
    "M_RST2",
    "M_RST3",
    "M_RST4",
    "M_RST5",
    "M_RST6",
    "M_RST7",
    "M_SOI",
    "M_EOI",
    "M_SOS",
    "M_DQT",
    "M_DNL",
    "M_DRI",
    "M_DHP",
    "M_EXP",
    "M_APP0",
    "M_APP1",
    "M_APP2",
    "M_APP3",
    "M_APP4",
    "M_APP5",
    "M_APP6",
    "M_APP7",
    "M_APP8",
    "M_APP9",
    "M_APP10",
    "M_APP11",
    "M_APP12",
    "M_APP13",
    "M_APP14",
    "M_APP15",
    "M_JPG0",
    "M_JPG1",
    "M_JPG2",
    "M_JPG3",
    "M_JPG4",
    "M_JPG5",
    "M_JPG6",
    "M_JPG7",
    "M_JPG8",
    "M_JPG9",
    "M_JPG10",
    "M_JPG11",
    "M_JPG12",
    "M_JPG13",
    "M_COM",
    "M_ERROR",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_checks_match_cpp() {
        assert_eq!(tcNewSubFileType, 254);
        assert_eq!(tcSubFileType, 255);
        assert_eq!(tcImageWidth, 256);
        assert_eq!(tcOrientation, 274);
        assert_eq!(tcExifIFD, 34665);
        assert_eq!(tcGPSInfo, 34853);
        assert_eq!(tcXMP, 700);
        assert_eq!(tcImageUniqueID, 42020);

        // Maker-note IFD parent codes.
        assert_eq!(tcFirstSubIFD, 0x10000);
        assert_eq!(tcLastSubIFD, 0x1FFFF);
        assert_eq!(tcFirstMakerNoteIFD, 0x30000);
        assert_eq!(tcAppleMakerNote, 0x30000);
        assert_eq!(tcCanonMakerNote, 0x30001);
        assert_eq!(tcCanonCRW, 0x40000);
        assert_eq!(tcContaxRAW, 0x40001);
        assert_eq!(tcJXL, 0x40013);

        // GPS.
        assert_eq!(tcGPSVersionID, 0);
        assert_eq!(tcGPSHPositioningError, 31);

        // Interoperability.
        assert_eq!(tcInteroperabilityIndex, 1);

        // JPEG markers.
        assert_eq!(M_TEM, 0x01);
        assert_eq!(M_SOF0, 0xc0);
        assert_eq!(M_SOF2, 0xc2);
        assert_eq!(M_DHT, 0xc4);
        assert_eq!(M_DQT, 0xdb);
        assert_eq!(M_DRI, 0xdd);
        assert_eq!(M_SOS, 0xda);
        assert_eq!(M_EOI, 0xd9);
    }

    #[test]
    fn count_and_uniqueness() {
        let names: Vec<&str> = NAMES.iter().copied().collect();
        let set: std::collections::HashSet<&str> = names.iter().copied().collect();
        assert_eq!(names.len(), set.len());
        assert_eq!(names.len(), 448);
    }
}

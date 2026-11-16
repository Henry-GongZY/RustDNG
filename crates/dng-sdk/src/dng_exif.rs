// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_exif.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_exif.h/.cpp — EXIF metadata fields, tag parsing and the
// exposure-time/f-number encode helpers.
//
// PORT-NOTE(dng_exif.cpp PostParse): the entire C++ body is `#if qDNGValidate`
// diagnostics (MaxApertureValue/LensInfo consistency warnings); with
// qDNGValidate=0 the method is a no-op, and the port mirrors that.
//
// PORT-NOTE(dng_exif.cpp ParseTag): the `#if qDNGValidate` dump blocks are
// omitted; every functional branch is ported.

use dng_core::dng_exceptions::bad_format;
use dng_core::{
    check_color_image, check_tag_count, check_tag_type, dng_char4, parse_date_time_tag,
    parse_dual_string_tag, parse_encoded_string_tag, parse_string_tag, DngDateTime, DngDateTimeInfo,
    DngDateTimeStorageInfo, DngFingerprint, DngResult, DngSRational, DngShared, DngStream,
    DngURational, kMaxCFAPattern, kMaxSamplesPerPixel, tcAcceleration, tcApertureValue, tcArtist,
    tcBatteryLevel, tcBrightnessValue, tcCameraElevationAngle, tcCameraOwnerNameExif,
    tcCameraSerialNumber, tcCameraSerialNumberExif, tcCFAPatternExif, tcColorSpace,
    tcComponentsConfiguration, tcCompressedBitsPerPixel, tcContrast, tcCopyright,
    tcCustomRendered, tcDateTime, tcDateTimeDigitized, tcDateTimeOriginal, tcDigitalZoomRatio,
    tcExifIFD, tcExifVersion, tcExposureBiasValue, tcExposureIndex, tcExposureIndexExif, tcExposureMode,
    tcExposureProgram, tcExposureTime, tcFileSource, tcFlash, tcFlashPixVersion, tcFNumber,
    tcFocalLength, tcFocalLengthIn35mmFilm, tcFocalPlaneResolutionUnit,
    tcFocalPlaneResolutionUnitExif, tcFocalPlaneXResolution, tcFocalPlaneXResolutionExif,
    tcFocalPlaneYResolution, tcFocalPlaneYResolutionExif, tcGainControl, tcGPSAltitude,
    tcGPSAltitudeRef, tcGPSAreaInformation, tcGPSDateStamp, tcGPSDestBearing,
    tcGPSDestBearingRef, tcGPSDestDistance, tcGPSDestDistanceRef, tcGPSDestLatitude,
    tcGPSDestLatitudeRef, tcGPSDestLongitude, tcGPSDestLongitudeRef, tcGPSDifferential, tcGPSDOP,
    tcGPSHPositioningError, tcGPSImgDirection, tcGPSImgDirectionRef, tcGPSInfo, tcGPSLatitude,
    tcGPSLatitudeRef, tcGPSLongitude, tcGPSLongitudeRef, tcGPSMapDatum, tcGPSMeasureMode, tcGPSProcessingMethod,
    tcGPSSatellites, tcGPSSpeed, tcGPSSpeedRef, tcGPSStatus, tcGPSTimeStamp, tcGPSTrack,
    tcGPSTrackRef, tcGPSVersionID, tcHumidity, tcImageDescription, tcImageNumber, tcImageUniqueID,
    tcInteroperabilityIFD, tcISOSpeed, tcISOSpeedLatitudeyyy, tcISOSpeedLatitudezzz,
    tcISOSpeedRatings, tcKodakCameraSerialNumber, tcLensInfo, tcLensMakeExif, tcLensModelExif,
    tcLensSerialNumberExif, tcLensSpecificationExif, tcLightSource, tcMake, tcMaxApertureValue,
    tcMeteringMode, tcModel, tcOffsetTime, tcOffsetTimeDigitized, tcOffsetTimeOriginal,
    tcPixelXDimension, tcPixelYDimension, tcPressure,
    tcRecommendedExposureIndex, tcRelatedImageFileFormat, tcRelatedImageLength,
    tcRelatedImageWidth, tcSaturation, tcSceneCaptureType, tcSceneType, tcSelfTimerMode,
    tcSensingMethod, tcSensingMethodExif, tcSensitivityType,
    tcSharpness, tcShutterSpeedValue, tcSoftware, tcStandardOutputSensitivity, tcSubjectArea,
    tcSubjectDistance, tcSubjectDistanceRange, tcSubjectLocation, tcSubsecTime,
    tcSubsecTimeDigitized, tcSubsecTimeOriginal, tcTemperature, tcTIFF_EP_StandardID,
    tcTimeZoneOffset, tcUserComment, tcWaterDepth, tcWhiteBalance, ttAscii, ttByte, ttLong,
    ttRational, ttShort, ttSRational, ttUndefined,
};
use dng_core::dng_tags::tcInteroperabilityIndex;
use dng_core::dng_tags::tcInteroperabilityVersion;
use dng_core::dng_tags::tcRelatedImageFileFormat as _ri;
use dng_core::dng_types::{real64, uint8, uint16, uint32, uint64};
use dng_core::DngError;

/// Port of `dng_exif`.
#[derive(Debug, Clone, Default)]
pub struct DngExif {
    pub image_description: dng_core::DngString,
    pub make: dng_core::DngString,
    pub model: dng_core::DngString,
    pub software: dng_core::DngString,
    pub artist: dng_core::DngString,
    pub copyright: dng_core::DngString,
    pub copyright2: dng_core::DngString,
    pub user_comment: dng_core::DngString,

    pub date_time: DngDateTimeInfo,
    pub date_time_storage_info: DngDateTimeStorageInfo,
    pub date_time_original: DngDateTimeInfo,
    pub date_time_original_storage_info: DngDateTimeStorageInfo,
    pub date_time_digitized: DngDateTimeInfo,
    pub date_time_digitized_storage_info: DngDateTimeStorageInfo,

    pub tiff_ep_standard_id: uint32,
    pub exif_version: uint32,
    pub flash_pix_version: uint32,

    pub exposure_time: DngURational,
    pub fnumber: DngURational,
    pub shutter_speed_value: DngSRational,
    pub aperture_value: DngURational,
    pub brightness_value: DngSRational,
    pub exposure_bias_value: DngSRational,
    pub max_aperture_value: DngURational,
    pub focal_length: DngURational,
    pub digital_zoom_ratio: DngURational,
    pub exposure_index: DngURational,
    pub subject_distance: DngURational,
    pub gamma: DngURational,
    pub battery_level_r: DngURational,
    pub battery_level_a: dng_core::DngString,

    pub exposure_program: uint32,
    pub metering_mode: uint32,
    pub light_source: uint32,
    pub flash: uint32,
    pub flash_mask: uint32,
    pub sensing_method: uint32,
    pub color_space: uint32,
    pub file_source: uint32,
    pub scene_type: uint32,
    pub custom_rendered: uint32,
    pub exposure_mode: uint32,
    pub white_balance: uint32,
    pub scene_capture_type: uint32,
    pub gain_control: uint32,
    pub contrast: uint32,
    pub saturation: uint32,
    pub sharpness: uint32,
    pub subject_distance_range: uint32,
    pub self_timer_mode: uint32,
    pub image_number: uint32,
    pub focal_length_in_35mm_film: uint32,

    pub iso_speed_ratings: [uint32; 3],
    pub sensitivity_type: uint32,
    pub standard_output_sensitivity: uint32,
    pub recommended_exposure_index: uint32,
    pub iso_speed: uint32,
    pub iso_speed_latitude_yyy: uint32,
    pub iso_speed_latitude_zzz: uint32,

    pub subject_area_count: uint32,
    pub subject_area: [uint32; 4],

    pub components_configuration: uint32,
    pub compressed_bits_per_pixel: DngURational,

    pub pixel_x_dimension: uint32,
    pub pixel_y_dimension: uint32,

    pub focal_plane_x_resolution: DngURational,
    pub focal_plane_y_resolution: DngURational,
    pub focal_plane_resolution_unit: uint32,

    pub cfa_repeat_pattern_rows: uint32,
    pub cfa_repeat_pattern_cols: uint32,
    pub cfa_pattern: [[uint8; kMaxCFAPattern as usize]; kMaxCFAPattern as usize],

    pub image_unique_id: DngFingerprint,

    pub gps_version_id: uint32,
    pub gps_latitude_ref: dng_core::DngString,
    pub gps_latitude: [DngURational; 3],
    pub gps_longitude_ref: dng_core::DngString,
    pub gps_longitude: [DngURational; 3],
    pub gps_altitude_ref: uint32,
    pub gps_altitude: DngURational,
    pub gps_time_stamp: [DngURational; 3],
    pub gps_satellites: dng_core::DngString,
    pub gps_status: dng_core::DngString,
    pub gps_measure_mode: dng_core::DngString,
    pub gps_dop: DngURational,
    pub gps_speed_ref: dng_core::DngString,
    pub gps_speed: DngURational,
    pub gps_track_ref: dng_core::DngString,
    pub gps_track: DngURational,
    pub gps_img_direction_ref: dng_core::DngString,
    pub gps_img_direction: DngURational,
    pub gps_map_datum: dng_core::DngString,
    pub gps_dest_latitude_ref: dng_core::DngString,
    pub gps_dest_latitude: [DngURational; 3],
    pub gps_dest_longitude_ref: dng_core::DngString,
    pub gps_dest_longitude: [DngURational; 3],
    pub gps_dest_bearing_ref: dng_core::DngString,
    pub gps_dest_bearing: DngURational,
    pub gps_dest_distance_ref: dng_core::DngString,
    pub gps_dest_distance: DngURational,
    pub gps_processing_method: dng_core::DngString,
    pub gps_area_information: dng_core::DngString,
    pub gps_date_stamp: dng_core::DngString,
    pub gps_differential: uint32,
    pub gps_h_positioning_error: DngURational,

    pub interoperability_index: dng_core::DngString,
    pub interoperability_version: uint32,
    pub related_image_file_format: dng_core::DngString,
    pub related_image_width: uint32,
    pub related_image_length: uint32,

    pub camera_serial_number: dng_core::DngString,
    pub lens_info: [DngURational; 4],
    pub lens_id: dng_core::DngString,
    pub lens_make: dng_core::DngString,
    pub lens_name: dng_core::DngString,
    pub lens_serial_number: dng_core::DngString,
    pub lens_name_was_read_from_exif: bool,

    pub approx_focus_distance: DngURational,
    pub flash_compensation: DngSRational,

    pub owner_name: dng_core::DngString,
    pub firmware: dng_core::DngString,

    pub temperature: DngSRational,
    pub humidity: DngURational,
    pub pressure: DngURational,
    pub water_depth: DngSRational,
    pub acceleration: DngURational,
    pub camera_elevation_angle: DngSRational,

    pub title: dng_core::DngString,

    pub lens_distort_info: [DngSRational; 4],
    pub neutral_density_factor: DngURational,

    pub subsec_time: dng_core::DngString,
    pub subsec_time_original: dng_core::DngString,
    pub subsec_time_digitized: dng_core::DngString,

    /// EXIF 2.3 offset-time strings ("+HH:MM").
    pub offset_time: dng_core::DngString,
    pub offset_time_original: dng_core::DngString,
    pub offset_time_digitized: dng_core::DngString,
}

impl DngExif {
    /// `dng_exif ()` — exact C++ defaults.
    pub fn new() -> Self {
        let mut e = DngExif {
            tiff_ep_standard_id: 0,
            exif_version: 0,
            flash_pix_version: 0,
            exposure_program: 0xFFFFFFFF,
            metering_mode: 0xFFFFFFFF,
            light_source: 0xFFFFFFFF,
            flash: 0xFFFFFFFF,
            flash_mask: 0x0000_FFFF,
            sensing_method: 0xFFFFFFFF,
            color_space: 0xFFFFFFFF,
            file_source: 0xFFFFFFFF,
            scene_type: 0xFFFFFFFF,
            custom_rendered: 0xFFFFFFFF,
            exposure_mode: 0xFFFFFFFF,
            white_balance: 0xFFFFFFFF,
            scene_capture_type: 0xFFFFFFFF,
            gain_control: 0xFFFFFFFF,
            contrast: 0xFFFFFFFF,
            saturation: 0xFFFFFFFF,
            sharpness: 0xFFFFFFFF,
            subject_distance_range: 0xFFFFFFFF,
            self_timer_mode: 0xFFFFFFFF,
            image_number: 0xFFFFFFFF,
            focal_length_in_35mm_film: 0,
            sensitivity_type: 0,
            standard_output_sensitivity: 0,
            recommended_exposure_index: 0,
            iso_speed: 0,
            iso_speed_latitude_yyy: 0,
            iso_speed_latitude_zzz: 0,
            subject_area_count: 0,
            components_configuration: 0,
            pixel_x_dimension: 0,
            pixel_y_dimension: 0,
            focal_plane_resolution_unit: 0,
            cfa_repeat_pattern_rows: 0,
            cfa_repeat_pattern_cols: 0,
            gps_version_id: 0,
            gps_altitude_ref: 0xFFFFFFFF,
            gps_differential: 0xFFFFFFFF,
            interoperability_version: 0,
            related_image_width: 0,
            related_image_length: 0,
            lens_name_was_read_from_exif: false,
            ..Default::default()
        };

        // Neutral rationals default to (0, 0) = invalid; srational too.
        e.flash_compensation = DngSRational::new(0, 0);
        e.approx_focus_distance = DngURational::new(0, 0);

        e
    }

    /// `Clone ()`.
    pub fn cloned(&self) -> Self {
        self.clone()
    }

    /// `SetEmpty ()`.
    pub fn set_empty(&mut self) {
        *self = DngExif::new();
    }

    /// `CopyGPSFrom (exif)`.
    pub fn copy_gps_from(&mut self, exif: &DngExif) {
        self.gps_version_id = exif.gps_version_id;
        self.gps_latitude_ref = exif.gps_latitude_ref.clone();
        self.gps_latitude = exif.gps_latitude;
        self.gps_longitude_ref = exif.gps_longitude_ref.clone();
        self.gps_longitude = exif.gps_longitude;
        self.gps_altitude_ref = exif.gps_altitude_ref;
        self.gps_altitude = exif.gps_altitude;
        self.gps_time_stamp = exif.gps_time_stamp;
        self.gps_satellites = exif.gps_satellites.clone();
        self.gps_status = exif.gps_status.clone();
        self.gps_measure_mode = exif.gps_measure_mode.clone();
        self.gps_dop = exif.gps_dop;
        self.gps_speed_ref = exif.gps_speed_ref.clone();
        self.gps_speed = exif.gps_speed;
        self.gps_track_ref = exif.gps_track_ref.clone();
        self.gps_track = exif.gps_track;
        self.gps_img_direction_ref = exif.gps_img_direction_ref.clone();
        self.gps_img_direction = exif.gps_img_direction;
        self.gps_map_datum = exif.gps_map_datum.clone();
        self.gps_dest_latitude_ref = exif.gps_dest_latitude_ref.clone();
        self.gps_dest_latitude = exif.gps_dest_latitude;
        self.gps_dest_longitude_ref = exif.gps_dest_longitude_ref.clone();
        self.gps_dest_longitude = exif.gps_dest_longitude;
        self.gps_dest_bearing_ref = exif.gps_dest_bearing_ref.clone();
        self.gps_dest_bearing = exif.gps_dest_bearing;
        self.gps_dest_distance_ref = exif.gps_dest_distance_ref.clone();
        self.gps_dest_distance = exif.gps_dest_distance;
        self.gps_processing_method = exif.gps_processing_method.clone();
        self.gps_area_information = exif.gps_area_information.clone();
        self.gps_date_stamp = exif.gps_date_stamp.clone();
        self.gps_differential = exif.gps_differential;
        self.gps_h_positioning_error = exif.gps_h_positioning_error;
    }

    /// `CopyDateFrom (exif)`.
    pub fn copy_date_from(&mut self, exif: &DngExif) {
        self.date_time_original = exif.date_time_original.clone();
        self.date_time_original_storage_info = exif.date_time_original_storage_info;
        self.date_time_digitized = exif.date_time_digitized.clone();
        self.date_time_digitized_storage_info = exif.date_time_digitized_storage_info;
        self.date_time = exif.date_time.clone();
        self.date_time_storage_info = exif.date_time_storage_info;
    }

    /// `SnapExposureTime (et)`.
    pub fn snap_exposure_time(mut et: real64) -> real64 {
        // Protection against invalid values.
        if et <= 0.0 {
            return 0.0;
        }

        #[rustfmt::skip]
        static kStandardSpeed: [real64; 70] = [
            30.0, 25.0, 20.0, 15.0, 13.0, 10.0, 8.0, 6.0, 5.0, 4.0,
            3.2, 3.0, 2.5, 2.0, 1.6, 1.5, 1.3, 1.0, 0.8, 0.7,
            0.6, 0.5, 0.4, 0.3,
            1.0 / 4.0, 1.0 / 5.0, 1.0 / 6.0, 1.0 / 8.0, 1.0 / 10.0,
            1.0 / 13.0, 1.0 / 15.0, 1.0 / 20.0, 1.0 / 25.0, 1.0 / 30.0,
            1.0 / 40.0, 1.0 / 45.0, 1.0 / 50.0, 1.0 / 60.0, 1.0 / 80.0,
            1.0 / 90.0, 1.0 / 100.0, 1.0 / 125.0, 1.0 / 160.0, 1.0 / 180.0,
            1.0 / 200.0, 1.0 / 250.0, 1.0 / 320.0, 1.0 / 350.0, 1.0 / 400.0,
            1.0 / 500.0, 1.0 / 640.0, 1.0 / 750.0, 1.0 / 800.0, 1.0 / 1000.0,
            1.0 / 1250.0, 1.0 / 1500.0, 1.0 / 1600.0, 1.0 / 2000.0,
            1.0 / 2500.0, 1.0 / 3000.0, 1.0 / 3200.0, 1.0 / 4000.0,
            1.0 / 5000.0, 1.0 / 6000.0, 1.0 / 6400.0, 1.0 / 8000.0,
            1.0 / 10000.0, 1.0 / 12000.0, 1.0 / 12800.0, 1.0 / 16000.0,
        ];

        for pass in 1..=2 {
            for fudge in 0..=1 {
                let mut test_speed = et;

                if fudge == 1 {
                    // Often APEX values are rounded to a power of two, which
                    // results in non-standard shutter speeds.
                    if et >= 0.1 {
                        // No fudging slower than 1/10 second.
                        break;
                    } else if et >= 0.01 {
                        // 1/15, 1/30, 1/60 misencoded as 1/16, 1/32, 1/64.
                        test_speed *= 16.0 / 15.0;
                    } else {
                        // 1/125, 1/250, 1/500 misencoded as 1/128, 1/256, 1/512.
                        test_speed *= 128.0 / 125.0;
                    }
                }

                for standard in kStandardSpeed.iter() {
                    if test_speed >= standard * 0.98 && test_speed <= standard * 1.02 {
                        return *standard;
                    }
                }
            }

            // Not near any standard speed: round to something reasonable.
            if pass == 1 {
                if et >= 10.0 {
                    et = (et + 0.5).floor();
                } else if et >= 0.5 {
                    et = (et * 10.0 + 0.5).floor() * 0.1;
                } else if et >= 1.0 / 20.0 {
                    et = 1.0 / (1.0 / et + 0.5).floor();
                } else if et >= 1.0 / 130.0 {
                    et = 0.2 / (0.2 / et + 0.5).floor();
                } else if et >= 1.0 / 750.0 {
                    et = 0.1 / (0.1 / et + 0.5).floor();
                } else if et >= 1.0 / 1300.0 {
                    et = 0.02 / (0.02 / et + 0.5).floor();
                } else if et >= 1.0 / 15000.0 {
                    et = 0.01 / (0.01 / et + 0.5).floor();
                } else {
                    et = 0.001 / (0.001 / et + 0.5).floor();
                }
            }
        }

        et
    }

    /// `SetExposureTime (et, snap)`.
    pub fn set_exposure_time(&mut self, et: real64, snap: bool) {
        use dng_core::dng_rational::DngURational as UR;

        self.exposure_time.clear();
        self.shutter_speed_value.clear();

        let mut et = et;
        if snap {
            et = Self::snap_exposure_time(et);
        }

        if (1.0 / 1073741824.0..=1073741824.0).contains(&et) {
            if et >= 100.0 {
                self.exposure_time.set_real64(et, 1);
            } else if et >= 1.0 {
                self.exposure_time.set_real64(et, 10);
                self.exposure_time.reduce_by_factor(10);
            } else if et <= 0.1 {
                self.exposure_time = UR::new(1, dng_core::dng_utils::round_uint32_f64(1.0 / et));
            } else {
                self.exposure_time.set_real64(et, 100);
                self.exposure_time.reduce_by_factor(10);

                for f in 2..=9u32 {
                    let z = 1.0 / f as real64 / et;
                    if (0.99..=1.01).contains(&z) {
                        self.exposure_time = UR::new(1, f);
                        break;
                    }
                }
            }

            // Mirror to the ShutterSpeedValue field.
            let et = self.exposure_time.as_real64();

            self.shutter_speed_value
                .set_real64(-(et.ln()) / 2.0f64.ln(), 1_000_000);

            for _ in 0..6 {
                self.shutter_speed_value.reduce_by_factor(10);
            }
        }
    }

    /// `SetShutterSpeedValue (ss)`.
    pub fn set_shutter_speed_value(&mut self, ss: real64) {
        if self.exposure_time.not_valid() {
            let et = 2.0f64.powf(-ss);
            self.set_exposure_time(et, true);
        }
    }

    /// `EncodeFNumber (fs)`.
    pub fn encode_fnumber(fs: real64) -> DngURational {
        let mut y = DngURational::default();

        if fs > 10.0 {
            y.set_real64(fs, 1);
        } else if fs < 1.0 {
            y.set_real64(fs, 100);
            y.reduce_by_factor(10);
            y.reduce_by_factor(10);
        } else {
            y.set_real64(fs, 10);
            y.reduce_by_factor(10);
        }

        y
    }

    /// `SetFNumber (fs)`.
    pub fn set_fnumber(&mut self, fs: real64) {
        self.fnumber.clear();
        self.aperture_value.clear();

        // Allow f/0.95-style values even though the APEX equivalent is
        // negative (ApertureValue is omitted for fs < 1.0).
        if fs > 0.0 && fs <= 32768.0 {
            self.fnumber = Self::encode_fnumber(fs);

            let av = Self::fnumber_to_aperture_value(self.fnumber.as_real64());

            if (0.0..=99.99).contains(&av) {
                self.aperture_value.set_real64(av, 1_000_000);
                for _ in 0..6 {
                    self.aperture_value.reduce_by_factor(10);
                }
            }
        }
    }

    /// `SetApertureValue (av)`.
    pub fn set_aperture_value(&mut self, av: real64) {
        if self.fnumber.not_valid() {
            self.set_fnumber(Self::aperture_value_to_fnumber(av));
        }
    }

    /// `ApertureValueToFNumber (av)`.
    pub fn aperture_value_to_fnumber(av: real64) -> real64 {
        (0.5 * av).exp2()
    }

    /// `ApertureValueToFNumber (const dng_urational &)`.
    pub fn aperture_value_to_fnumber_urational(av: &DngURational) -> real64 {
        Self::aperture_value_to_fnumber(av.as_real64())
    }

    /// `FNumberToApertureValue (fNumber)`.
    pub fn fnumber_to_aperture_value(fnumber: real64) -> real64 {
        2.0 * fnumber.ln() / 2.0f64.ln()
    }

    /// `FNumberToApertureValue (const dng_urational &)`.
    pub fn fnumber_to_aperture_value_urational(fnumber: &DngURational) -> real64 {
        Self::fnumber_to_aperture_value(fnumber.as_real64())
    }

    /// `UpdateDateTime (dt)`.
    pub fn update_date_time(&mut self, dt: &DngDateTimeInfo) {
        self.date_time = dt.clone();
    }

    /// `AtLeastVersion0230 ()`.
    pub fn at_least_version_0230(&self) -> bool {
        self.exif_version >= dng_char4(b'0', b'2', b'3', b'0')
    }

    /// `AtLeastVersion0231 ()`.
    pub fn at_least_version_0231(&self) -> bool {
        self.exif_version >= dng_char4(b'0', b'2', b'3', b'1')
    }

    /// `SetVersion0231 ()`.
    pub fn set_version_0231(&mut self) {
        self.exif_version = dng_char4(b'0', b'2', b'3', b'1');
    }

    /// `HasLensDistortInfo ()`.
    pub fn has_lens_distort_info(&self) -> bool {
        self.lens_distort_info.iter().all(|r| r.is_valid())
    }

    /// `SetLensDistortInfo (params)`.
    pub fn set_lens_distort_info(&mut self, params: &dng_core::dng_matrix::DngVector) {
        if params.count() != 4 {
            return;
        }

        for (i, r) in self.lens_distort_info.iter_mut().enumerate() {
            r.set_real64(params.get(i), 0);
        }
    }

    /// `ParseTag (stream, shared, parentCode, isMainIFD, tagCode, tagType,
    /// tagCount, tagOffset)` — dispatches to the sub-parse functions per the
    /// C++ structure.
    #[allow(clippy::too_many_arguments)]
    pub fn parse_tag(
        &mut self,
        stream: &mut DngStream,
        shared: &mut DngShared,
        parent_code: uint32,
        is_main_ifd: bool,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        tag_offset: uint64,
    ) -> DngResult<bool> {
        if tag_code == tcExifIFD || tag_code == tcGPSInfo || tag_code == tcInteroperabilityIFD {
            // IFD pointers are consumed by dng_info/dng_shared.
            return Ok(false);
        }

        if tag_code <= 0xFFFF {
            // ifd0 range (including main-IFD photo tags, which the C++
            // routes via parentCode checks).
            let routed_main = self.parse_ifd0_main(
                stream,
                parent_code,
                is_main_ifd,
                tag_code,
                tag_type,
                tag_count,
                tag_offset,
            )?;
            if routed_main {
                return Ok(true);
            }
        }

        // EXIF sub-IFD tags.
        if self.parse_ifd0_exif(
            stream,
            parent_code,
            is_main_ifd,
            tag_code,
            tag_type,
            tag_count,
            tag_offset,
        )? {
            return Ok(true);
        }

        // GPS tags.
        if self.parse_gps(stream, parent_code, tag_code, tag_type, tag_count, tag_offset)? {
            return Ok(true);
        }

        // Interoperability tags.
        if self.parse_interoperability(stream, parent_code, tag_code, tag_type, tag_count, tag_offset)? {
            return Ok(true);
        }

        // Remaining ifd0 tags.
        self.parse_ifd0(
            stream,
            parent_code,
            is_main_ifd,
            tag_code,
            tag_type,
            tag_count,
            tag_offset,
        )
    }

    /// `Parse_ifd0_main` — photo tags valid in the main IFD.
    #[allow(clippy::too_many_arguments)]
    fn parse_ifd0_main(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        _is_main_ifd: bool,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        _tag_offset: uint64,
    ) -> DngResult<bool> {
        match tag_code {
            x if x == tcFocalPlaneXResolution => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_x_resolution = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFocalPlaneYResolution => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_y_resolution = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFocalPlaneResolutionUnit => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_resolution_unit = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcSensingMethod => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.sensing_method = stream.tag_value_uint32(tag_type)?;
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// `Parse_ifd0_exif` — the EXIF sub-IFD tag set.
    #[allow(clippy::too_many_lines)]
    fn parse_ifd0_exif(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        _is_main_ifd: bool,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        _tag_offset: uint64,
    ) -> DngResult<bool> {
        match tag_code {
            x if x == tcExposureTime => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);

                let et = stream.tag_value_urational(ttRational)?;
                self.set_exposure_time(et.as_real64(), true);
            }
            x if x == tcShutterSpeedValue => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.shutter_speed_value = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcFNumber => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                let fs = stream.tag_value_urational(ttRational)?;
                self.set_fnumber(fs.as_real64());
            }
            x if x == tcApertureValue => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.aperture_value = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcBrightnessValue => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.brightness_value = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcExposureBiasValue => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.exposure_bias_value = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcMaxApertureValue => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.max_aperture_value = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcSubjectDistance => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.subject_distance = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcExposureProgram => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.exposure_program = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcMeteringMode => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.metering_mode = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcLightSource => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.light_source = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcFlash => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.flash = stream.tag_value_uint32(tag_type)?;
                // The C++ keeps a mask of valid bits (0x0000FFFF via fFlashMask
                // and &= in the parse; preserve both).
            }
            x if x == tcFocalLength => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_length = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcImageNumber => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16, ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.image_number = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcExposureIndex || x == tcExposureIndexExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.exposure_index = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcBatteryLevel => {
                if tag_type == ttRational as uint32 {
                    check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                    self.battery_level_r = stream.tag_value_urational(ttRational)?;
                } else {
                    check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                    parse_string_tag(
                        stream,
                        parent_code,
                        tag_code,
                        tag_count,
                        &mut self.battery_level_a,
                        true,
                    )?;
                }
            }
            x if x == tcISOSpeedRatings => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                if !check_tag_count(parent_code, tag_code, tag_count, 0, 3) {
                    return Ok(false);
                }
                for j in 0..tag_count as usize {
                    self.iso_speed_ratings[j] = stream.tag_value_uint32(tag_type)?;
                }
            }
            x if x == tcSensitivityType => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.sensitivity_type = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcStandardOutputSensitivity => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.standard_output_sensitivity = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcRecommendedExposureIndex => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.recommended_exposure_index = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcISOSpeed => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.iso_speed = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcISOSpeedLatitudeyyy => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.iso_speed_latitude_yyy = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcISOSpeedLatitudezzz => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.iso_speed_latitude_zzz = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcTimeZoneOffset => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                // EXIF 2.3 TimeZoneOffset feeds the date_time zone.
                let sr = stream.tag_value_srational(ttSRational)?;
                let mut zone = dng_core::dng_date_time::DngTimeZone::new();
                zone.set_offset_minutes(sr.as_real64().round() as i32 * 60);
                if zone.is_valid() {
                    self.date_time.set_zone(zone);
                }
            }
            x if x == tcSelfTimerMode => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.self_timer_mode = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcExifVersion => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16, ttByte as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.exif_version = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == tcDateTimeOriginal => {
                let tag_position = stream.position_in_original_file();

                let mut dt = DngDateTime::new();
                if !parse_date_time_tag(
                    stream, parent_code, tag_code, tag_type, tag_count, &mut dt,
                )? {
                    return Ok(false);
                }

                self.date_time_original.set_date_time(dt);
                self.date_time_original_storage_info = DngDateTimeStorageInfo::from_offset(
                    tag_position,
                    dng_core::dng_date_time::DngDateTimeFormat::Exif,
                );
            }
            x if x == tcDateTimeDigitized => {
                let tag_position = stream.position_in_original_file();

                self.date_time_digitized_storage_info = DngDateTimeStorageInfo::from_offset(
                    tag_position,
                    dng_core::dng_date_time::DngDateTimeFormat::Exif,
                );

                let mut dt = DngDateTime::new();
                if !parse_date_time_tag(
                    stream, parent_code, tag_code, tag_type, tag_count, &mut dt,
                )? {
                    return Ok(false);
                }

                let mut info = DngDateTimeInfo::new();
                info.set_date_time(dt);
                self.date_time_digitized = info;
            }
            x if x == tcOffsetTime || x == tcOffsetTimeOriginal || x == tcOffsetTimeDigitized => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                let mut s = match tag_code {
                    x if x == tcOffsetTime => std::mem::take(&mut self.offset_time),
                    x if x == tcOffsetTimeOriginal => std::mem::take(&mut self.offset_time_original),
                    _ => std::mem::take(&mut self.offset_time_digitized),
                };
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, false)?;
                match tag_code {
                    x if x == tcOffsetTime => self.offset_time = s,
                    x if x == tcOffsetTimeOriginal => self.offset_time_original = s,
                    _ => self.offset_time_digitized = s,
                }
            }
            x if x == tcSubsecTime || x == tcSubsecTimeOriginal || x == tcSubsecTimeDigitized => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                let mut s = match tag_code {
                    x if x == tcSubsecTime => std::mem::take(&mut self.subsec_time),
                    x if x == tcSubsecTimeOriginal => std::mem::take(&mut self.subsec_time_original),
                    _ => std::mem::take(&mut self.subsec_time_digitized),
                };
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, true)?;
                match tag_code {
                    x if x == tcSubsecTime => self.subsec_time = s,
                    x if x == tcSubsecTimeOriginal => self.subsec_time_original = s,
                    _ => self.subsec_time_digitized = s,
                }
            }
            x if x == tcUserComment => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                parse_encoded_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.user_comment,
                )?;
            }
            x if x == tcComponentsConfiguration => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.components_configuration =
                    (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == tcCompressedBitsPerPixel => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.compressed_bits_per_pixel = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFlashPixVersion => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16, ttByte as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.flash_pix_version = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == tcColorSpace => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.color_space = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcPixelXDimension => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16, ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.pixel_x_dimension = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcPixelYDimension => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16, ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.pixel_y_dimension = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcFocalPlaneXResolutionExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_x_resolution = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFocalPlaneYResolutionExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_y_resolution = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFocalPlaneResolutionUnitExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_plane_resolution_unit = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcSensingMethodExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.sensing_method = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcFileSource => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.file_source = stream.get_uint8()? as uint32;
            }
            x if x == tcSceneType => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.scene_type = stream.get_uint8()? as uint32;
            }
            x if x == tcCFAPatternExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);

                // 2 N + N*N layout: dims then pattern.
                if tag_count < 2 {
                    return Ok(false);
                }

                let rows = stream.get_uint16()? as uint32;
                let cols = stream.get_uint16()? as uint32;

                if (rows < 1 || rows > kMaxCFAPattern) || (cols < 1 || cols > kMaxCFAPattern) {
                    return Ok(false);
                }

                if !check_tag_count(
                    parent_code,
                    tag_code,
                    tag_count,
                    2 + 2 + rows * cols,
                    0,
                ) {
                    return Ok(false);
                }

                self.cfa_repeat_pattern_rows = rows;
                self.cfa_repeat_pattern_cols = cols;

                for j in 0..rows as usize {
                    for k in 0..cols as usize {
                        self.cfa_pattern[j][k] = stream.get_uint8()?;
                    }
                }
            }
            x if x == tcCustomRendered => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.custom_rendered = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcExposureMode => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.exposure_mode = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcWhiteBalance => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.white_balance = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcDigitalZoomRatio => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.digital_zoom_ratio = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcFocalLengthIn35mmFilm => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.focal_length_in_35mm_film = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcSubjectArea || x == tcSubjectLocation => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                if !(2..=4).contains(&tag_count) {
                    return Ok(false);
                }
                self.subject_area_count = tag_count;
                for j in 0..tag_count as usize {
                    self.subject_area[j] = stream.tag_value_uint32(tag_type)?;
                }
            }
            x if x == tcTemperature => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.temperature = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcHumidity => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.humidity = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcPressure => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.pressure = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcWaterDepth => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.water_depth = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcAcceleration => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.acceleration = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcCameraElevationAngle => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.camera_elevation_angle = stream.tag_value_srational(ttSRational)?;
            }
            x if x == tcCameraOwnerNameExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.owner_name, true)?;
            }
            x if x == tcCameraSerialNumberExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.camera_serial_number,
                    true,
                )?;
            }
            x if x == tcLensSpecificationExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                if !check_tag_count(parent_code, tag_code, tag_count, 4, 0) {
                    return Ok(false);
                }
                for j in 0..4usize {
                    self.lens_info[j] = stream.tag_value_urational(ttRational)?;
                }
            }
            x if x == tcLensMakeExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.lens_make, true)?;
            }
            x if x == tcLensModelExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.lens_name, true)?;
                self.lens_name_was_read_from_exif = true;
            }
            x if x == tcLensSerialNumberExif => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.lens_serial_number,
                    true,
                )?;
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    /// `Parse_gps` — the GPS sub-IFD tag set.
    #[allow(clippy::too_many_lines)]
    fn parse_gps(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        _tag_offset: uint64,
    ) -> DngResult<bool> {
        // GPS tags live in 0x0000..0x001F.
        if tag_code > 0x1F {
            return Ok(false);
        }

        match tag_code {
            x if x == tcGPSVersionID => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.gps_version_id = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == tcGPSLatitudeRef || x == tcGPSLongitudeRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                let mut s = dng_core::DngString::new();
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, false)?;
                if tag_code == tcGPSLatitudeRef {
                    self.gps_latitude_ref = s;
                } else {
                    self.gps_longitude_ref = s;
                }
            }
            x if x == tcGPSLatitude || x == tcGPSLongitude => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 3, 0);
                for j in 0..3usize {
                    let v = stream.tag_value_urational(ttRational)?;
                    if tag_code == tcGPSLatitude {
                        self.gps_latitude[j] = v;
                    } else {
                        self.gps_longitude[j] = v;
                    }
                }
            }
            x if x == tcGPSAltitudeRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_altitude_ref = stream.get_uint8()? as uint32;
            }
            x if x == tcGPSAltitude => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_altitude = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSTimeStamp => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 3, 0);
                for j in 0..3usize {
                    self.gps_time_stamp[j] = stream.tag_value_urational(ttRational)?;
                }
            }
            x if x == tcGPSSatellites => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_satellites,
                    false,
                )?;
            }
            x if x == tcGPSStatus => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.gps_status, false)?;
            }
            x if x == tcGPSMeasureMode => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_measure_mode,
                    false,
                )?;
            }
            x if x == tcGPSDOP => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_dop = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSSpeedRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.gps_speed_ref, false)?;
            }
            x if x == tcGPSSpeed => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_speed = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSTrackRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.gps_track_ref, false)?;
            }
            x if x == tcGPSTrack => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_track = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSImgDirectionRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_img_direction_ref,
                    false,
                )?;
            }
            x if x == tcGPSImgDirection => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_img_direction = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSMapDatum => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.gps_map_datum, false)?;
            }
            x if x == tcGPSDestLatitudeRef || x == tcGPSDestLongitudeRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                let mut s = dng_core::DngString::new();
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, false)?;
                if tag_code == tcGPSDestLatitudeRef {
                    self.gps_dest_latitude_ref = s;
                } else {
                    self.gps_dest_longitude_ref = s;
                }
            }
            x if x == tcGPSDestLatitude || x == tcGPSDestLongitude => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 3, 0);
                for j in 0..3usize {
                    let v = stream.tag_value_urational(ttRational)?;
                    if tag_code == tcGPSDestLatitude {
                        self.gps_dest_latitude[j] = v;
                    } else {
                        self.gps_dest_longitude[j] = v;
                    }
                }
            }
            x if x == tcGPSDestBearingRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_dest_bearing_ref,
                    false,
                )?;
            }
            x if x == tcGPSDestBearing => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_dest_bearing = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSDestDistanceRef => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 2, 0);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_dest_distance_ref,
                    false,
                )?;
            }
            x if x == tcGPSDestDistance => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_dest_distance = stream.tag_value_urational(ttRational)?;
            }
            x if x == tcGPSProcessingMethod => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                parse_encoded_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_processing_method,
                )?;
            }
            x if x == tcGPSAreaInformation => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                parse_encoded_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.gps_area_information,
                )?;
            }
            x if x == tcGPSDateStamp => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.gps_date_stamp, false)?;
            }
            x if x == tcGPSDifferential => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_differential = stream.tag_value_uint32(tag_type)?;
            }
            x if x == tcGPSHPositioningError => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.gps_h_positioning_error = stream.tag_value_urational(ttRational)?;
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    /// `Parse_interoperability` — the Interoperability IFD tag set.
    fn parse_interoperability(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        _tag_offset: uint64,
    ) -> DngResult<bool> {
        match tag_code {
            x if x == dng_core::dng_tags::tcInteroperabilityIndex => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.interoperability_index,
                    false,
                )?;
            }
            x if x == dng_core::dng_tags::tcInteroperabilityVersion => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.interoperability_version = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == dng_core::dng_tags::tcRelatedImageFileFormat => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.related_image_file_format,
                    true,
                )?;
            }
            x if x == dng_core::dng_tags::tcRelatedImageWidth => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16, ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.related_image_width = stream.tag_value_uint32(tag_type)?;
            }
            x if x == dng_core::dng_tags::tcRelatedImageLength => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16, ttLong as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 1, 0);
                self.related_image_length = stream.tag_value_uint32(tag_type)?;
            }
            _ => return Ok(false),
        }

        Ok(true)
    }

    /// `Parse_ifd0` — the main-IFD strings and camera-identity tags.
    fn parse_ifd0(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        _is_main_ifd: bool,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        _tag_offset: uint64,
    ) -> DngResult<bool> {
        match tag_code {
            x if x == tcImageDescription => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16, ttByte as uint16]);
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.image_description,
                    true,
                )?;
            }
            x if x == tcMake => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16, ttByte as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.make, true)?;
            }
            x if x == tcModel => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16, ttByte as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.model, true)?;
            }
            x if x == tcSoftware => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16, ttByte as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.software, true)?;
            }
            x if x == tcArtist => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16, ttByte as uint16]);
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut self.artist, true)?;
            }
            x if x == tcCopyright => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]);
                parse_dual_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.copyright,
                    &mut self.copyright2,
                )?;
            }
            x if x == tcTIFF_EP_StandardID => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]);
                check_tag_count(parent_code, tag_code, tag_count, 4, 0);
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.tiff_ep_standard_id = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }
            x if x == tcCameraSerialNumber || x == tcKodakCameraSerialNumber => {
                if tag_type == ttAscii as uint32 {
                    parse_string_tag(
                        stream,
                        parent_code,
                        tag_code,
                        tag_count,
                        &mut self.camera_serial_number,
                        true,
                    )?;
                } else {
                    check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]);
                    // Kodak packs 6 shorts of BCD digits.
                    check_tag_count(parent_code, tag_code, tag_count, 6, 0);
                    let mut s = dng_core::DngString::new();
                    let mut text: Vec<u8> = Vec::with_capacity(12);
                    for _ in 0..6 {
                        let x = stream.get_uint16()?;
                        text.push(b'0' + ((x >> 12) & 15) as u8);
                        text.push(b'0' + ((x >> 8) & 15) as u8);
                        text.push(b'0' + ((x >> 4) & 15) as u8);
                        text.push(b'0' + (x & 15) as u8);
                    }
                    s.set(&text);
                    self.camera_serial_number = s;
                }
            }
            x if x == tcLensInfo => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                if !check_tag_count(parent_code, tag_code, tag_count, 4, 0) {
                    return Ok(false);
                }
                for j in 0..4usize {
                    self.lens_info[j] = stream.tag_value_urational(ttRational)?;
                }
            }
            x if x == tcImageUniqueID => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 33, 0) {
                    return Ok(false);
                }

                let mut s = dng_core::DngString::new();
                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, false)?;

                if s.length() != 32 {
                    return Ok(false);
                }

                let mut fp = DngFingerprint::new();
                for j in 0..32usize {
                    let c = dng_core::dng_utils::force_uppercase(s.get()[j]);

                    let digit: Option<u32> = match c {
                        b'0'..=b'9' => Some((c - b'0') as u32),
                        b'A'..=b'F' => Some((c - b'A') as u32 + 10),
                        _ => None,
                    };

                    let digit = match digit {
                        Some(d) => d,
                        None => return Ok(false),
                    };

                    fp.data[j / 2] |= (digit << (4 * (1 - (j & 1)))) as u8;
                }

                self.image_unique_id = fp;
            }
            x if x == tcDateTime => {
                let tag_position = stream.position_in_original_file();

                self.date_time_storage_info = DngDateTimeStorageInfo::from_offset(
                    tag_position,
                    dng_core::dng_date_time::DngDateTimeFormat::Exif,
                );

                let mut dt = DngDateTime::new();
                if !parse_date_time_tag(
                    stream, parent_code, tag_code, tag_type, tag_count, &mut dt,
                )? {
                    return Ok(false);
                }

                let mut info = DngDateTimeInfo::new();
                info.set_date_time(dt);
                self.date_time = info;
            }
            _ => return Ok(false),
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_cpp() {
        let e = DngExif::new();
        assert_eq!(e.exposure_program, 0xFFFFFFFF);
        assert_eq!(e.flash_mask, 0x0000_FFFF);
        assert_eq!(e.flash, 0xFFFFFFFF);
        assert_eq!(e.gps_altitude_ref, 0xFFFFFFFF);
        assert_eq!(e.gps_differential, 0xFFFFFFFF);
        assert_eq!(e.focal_length_in_35mm_film, 0);
        assert!(!e.lens_name_was_read_from_exif);
        assert!(e.exposure_time.not_valid());
        assert!(e.date_time.not_valid());
    }

    #[test]
    fn snap_exposure_time_standard() {
        assert_eq!(DngExif::snap_exposure_time(1.0 / 125.0), 1.0 / 125.0);
        assert_eq!(DngExif::snap_exposure_time(1.0 / 127.0), 1.0 / 125.0);
        assert_eq!(DngExif::snap_exposure_time(0.999), 1.0);
        // Fudge: 1/64 -> 1/60 (within 2% after x16/15 fudge).
        assert_eq!(DngExif::snap_exposure_time(1.0 / 64.0), 1.0 / 60.0);
        // 1/97: fudge-1 pass (x16/15) lands within 2% of 1/90.
        assert_eq!(DngExif::snap_exposure_time(1.0 / 97.0), 1.0 / 90.0);
        // 1/103 is caught by the 128/125 fudge (misencoded 1/128) -> 1/100.
        assert_eq!(DngExif::snap_exposure_time(1.0 / 103.0), 1.0 / 100.0);
        // 1/111 is genuinely non-standard: rounds inverse to multiple of 5.
        let x = DngExif::snap_exposure_time(1.0 / 111.0);
        assert!((x - 1.0 / 110.0).abs() < 1e-12, "x={x}");
        assert_eq!(DngExif::snap_exposure_time(0.0), 0.0);
    }

    #[test]
    fn set_exposure_time_mirrors_shutter_speed() {
        let mut e = DngExif::new();
        e.set_exposure_time(1.0 / 125.0, true);
        assert_eq!(e.exposure_time, DngURational::new(1, 125));
        // APEX: -log2(1/125) ~= 6.966
        let apex = e.shutter_speed_value.as_real64();
        assert!((apex - (125.0f64).log2()).abs() < 1e-6, "apex={apex}");
    }

    #[test]
    fn fnumber_apex_roundtrip() {
        let mut e = DngExif::new();
        e.set_fnumber(2.8);
        assert_eq!(e.fnumber, DngExif::encode_fnumber(2.8));
        assert!((e.aperture_value.as_real64() - 2.0 * 2.8f64.ln() / 2.0f64.ln()).abs() < 1e-6);

        // Round trip through APEX.
        let fs = DngExif::aperture_value_to_fnumber(e.aperture_value.as_real64());
        assert!((fs - 2.8).abs() < 1e-6);

        // EncodeFNumber denominator rules.
        assert_eq!(DngExif::encode_fnumber(22.0), DngURational::new(22, 1));
        assert_eq!(DngExif::encode_fnumber(2.8), DngURational::new(28, 10));
    }

    #[test]
    fn version_checks() {
        let mut e = DngExif::new();
        assert!(!e.at_least_version_0230());
        e.exif_version = dng_char4(b'0', b'2', b'3', b'0');
        assert!(e.at_least_version_0230());
        assert!(!e.at_least_version_0231());
        e.set_version_0231();
        assert!(e.at_least_version_0231());
    }

    #[test]
    fn lens_distort_info() {
        let mut e = DngExif::new();
        assert!(!e.has_lens_distort_info());
        let params = dng_core::dng_matrix::DngVector::from_values(&[
            0.001, -0.002, 0.003, -0.004,
        ])
        .unwrap();
        e.set_lens_distort_info(&params);
        assert!(e.has_lens_distort_info());
        // Set_real64 quantizes through the 32768^2 denominator.
        assert!((e.lens_distort_info[0].as_real64() - 0.001).abs() < 1e-9);
    }

    #[test]
    fn parse_exposure_time_tag() {
        let mut e = DngExif::new();
        let mut shared = DngShared::new();
        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();
        stream.set_big_endian(true);
        stream.put_uint32(1).unwrap();
        stream.put_uint32(125).unwrap();
        stream.set_read_position(0).unwrap();

        assert!(e
            .parse_tag(
                &mut stream,
                &mut shared,
                0,
                false,
                tcExposureTime,
                ttRational,
                1,
                0
            )
            .unwrap());
        assert_eq!(e.exposure_time, DngURational::new(1, 125));
        assert_eq!(e.shutter_speed_value.not_valid(), false);
    }

    #[test]
    fn parse_gps_tags() {
        let mut e = DngExif::new();
        let mut shared = DngShared::new();
        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        // GPSLatitudeRef "N\0".
        stream.put(b"N\0").unwrap();
        stream.set_read_position(0).unwrap();
        assert!(e
            .parse_tag(&mut stream, &mut shared, 0, false, tcGPSLatitudeRef, ttAscii, 2, 0)
            .unwrap());
        assert_eq!(e.gps_latitude_ref.get(), b"N");

        // GPSLatitude 3 rationals.
        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();
        for _ in 0..3 {
            stream.put_uint32(37).unwrap();
            stream.put_uint32(1).unwrap();
        }
        stream.set_read_position(0).unwrap();
        assert!(e
            .parse_tag(&mut stream, &mut shared, 0, false, tcGPSLatitude, ttRational, 3, 0)
            .unwrap());
        assert_eq!(e.gps_latitude[0], DngURational::new(37, 1));
    }

    #[test]
    fn unhandled_tag_returns_false() {
        let mut e = DngExif::new();
        let mut shared = DngShared::new();
        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();
        assert!(!e
            .parse_tag(&mut stream, &mut shared, 0, false, 0xDEAD, ttLong, 1, 0)
            .unwrap());
    }
}

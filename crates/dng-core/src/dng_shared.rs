// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_shared.cpp/.h`.
//
// Copyright 2006-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_shared.h/.cpp — the "shared" IFD metadata (DNG version,
// camera-profile info, calibration matrices, ...) with the ParseTag driver
// covering the tags dng_shared owns.
//
// PORT-NOTE(dng_shared.cpp:1795): the C++ ParseTag handles ~90 tags and
// delegates EXIF tags to dng_exif (phase 2) via dng_info.  The phase-1 port
// covers every tag dng_shared owns (DNG-version, camera calibration, analog
// balance, as-shot neutral/white, baselines, private data, original raw,
// pre-profile matrices, colorimetric reference, depth format, big tables,
// ImageSequenceInfo); EXIF-specific tags remain in dng_exif (phase 2) as in
// the C++ split.  `fProfileGainTableMap` / `fMaskedRGBTables` shared pointers
// belong to phase-2 types and are represented by their raw offsets here.

use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::bad_format;
use crate::dng_fingerprint::DngFingerprint;
use crate::dng_matrix::DngMatrix;
use crate::dng_parse_utils::*;
use crate::dng_rational::{DngSRational, DngURational};
use crate::dng_string::DngString;
use crate::dng_stream::DngStream;
use crate::dng_tags::*;
use crate::dng_tag_types::*;
use crate::dng_tag_values::*;
use crate::dng_types::{int64, real32, real64, uint16, uint32, uint64};
use crate::dng_utils::Pin_uint64_unused;
use crate::dng_utils::pin_uint32;
use crate::dng_xy_coord::{DngIlluminantData, DngXyCoord};

/// Port of `dng_camera_profile_dynamic_range`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DngCameraProfileDynamicRange {
    pub version: uint16,
    pub dynamic_range: uint16,
    pub hint_max_output_value: real32,
}

impl DngCameraProfileDynamicRange {
    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.version == 1 && self.hint_max_output_value >= 0.0
    }

    /// `IsSDR ()`.
    pub fn is_sdr(&self) -> bool {
        self.dynamic_range == 0
    }

    /// `IsHDR ()`.
    pub fn is_hdr(&self) -> bool {
        !self.is_sdr()
    }

    /// `PutStream (stream)`.
    pub fn put_stream(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint16(self.version)?;
        stream.put_uint16(self.dynamic_range)?;
        stream.put_real32(self.hint_max_output_value)
    }
}

/// Port of `dng_camera_profile_info`.
#[derive(Debug, Clone, Default)]
pub struct DngCameraProfileInfo {
    pub big_endian: bool,
    pub color_planes: uint32,
    pub calibration_illuminant1: uint32,
    pub calibration_illuminant2: uint32,
    pub calibration_illuminant3: uint32,
    pub illuminant_data1: DngIlluminantData,
    pub illuminant_data2: DngIlluminantData,
    pub illuminant_data3: DngIlluminantData,
    pub color_matrix1: DngMatrix,
    pub color_matrix2: DngMatrix,
    pub color_matrix3: DngMatrix,
    pub forward_matrix1: DngMatrix,
    pub forward_matrix2: DngMatrix,
    pub forward_matrix3: DngMatrix,
    pub reduction_matrix1: DngMatrix,
    pub reduction_matrix2: DngMatrix,
    pub reduction_matrix3: DngMatrix,
    pub profile_calibration_signature: DngString,
    pub profile_name: DngString,
    pub profile_copyright: DngString,
    pub embed_policy: uint32,
    pub profile_hues: uint32,
    pub profile_sats: uint32,
    pub profile_vals: uint32,
    pub hue_sat_deltas1_offset: uint64,
    pub hue_sat_deltas1_count: uint32,
    pub hue_sat_deltas2_offset: uint64,
    pub hue_sat_deltas2_count: uint32,
    pub hue_sat_deltas3_offset: uint64,
    pub hue_sat_deltas3_count: uint32,
    pub hue_sat_map_encoding: uint32,
    pub look_table_hues: uint32,
    pub look_table_sats: uint32,
    pub look_table_vals: uint32,
    pub look_table_offset: uint64,
    pub look_table_count: uint32,
    pub look_table_encoding: uint32,
    pub baseline_exposure_offset: DngSRational,
    pub default_black_render: uint32,
    pub tone_curve_offset: uint64,
    pub tone_curve_count: uint32,
    pub tone_method: uint32,
    pub unique_camera_model: DngString,
    pub profile_group_name: DngString,
    pub profile_dynamic_range: DngCameraProfileDynamicRange,

    /// tcProfileGainTableMap2: parsed by the SDK layer from this offset
    /// (the `DngGainTableMap` type lives in dng-sdk).
    pub profile_gain_table_map_offset: uint64,
    pub profile_gain_table_map_count: uint32,
    pub profile_gain_table_map_tag_version: uint32,

    /// tcRGBTables / tcRGBTablesDraft: offset+count for the SDK layer.
    pub masked_rgb_tables_offset: uint64,
    pub masked_rgb_tables_count: uint32,
    pub masked_rgb_tables_is_draft: bool,
}

impl DngCameraProfileInfo {
    pub fn new() -> Self {
        DngCameraProfileInfo {
            color_planes: 0,
            calibration_illuminant1: 0xFFFFFFFF,
            calibration_illuminant2: 0xFFFFFFFF,
            calibration_illuminant3: 0xFFFFFFFF,
            masked_rgb_tables_offset: 0,
            masked_rgb_tables_count: 0,
            masked_rgb_tables_is_draft: false,
            ..Default::default()
        }
    }

    /// `ParseTag (stream, parentCode, tagCode, tagType, tagCount, tagOffset)`.
    pub fn parse_tag(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        tag_offset: uint64,
    ) -> DngResult<bool> {
        match tag_code {
            x if x == tcCalibrationIlluminant1 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.calibration_illuminant1 = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcCalibrationIlluminant2 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.calibration_illuminant2 = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcCalibrationIlluminant3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.calibration_illuminant3 = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcColorMatrix1 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }

                // C++ (dng_shared.cpp): infer the color-plane count from the
                // tag count when unknown, then require tagCount == planes*3.

                if self.color_planes == 0 {
                    self.color_planes = crate::dng_utils::pin_uint32(
                        0,
                        tag_count / 3,
                        crate::dng_sdk_limits::kMaxColorPlanes,
                    );
                }

                if !check_color_image(parent_code, tag_code, self.color_planes) {
                    return Ok(false);
                }

                if tag_count != self.color_planes * 3 {
                    return Ok(false);
                }

                if let Some(m) = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    self.color_planes,
                    3,
                )? {
                    self.color_matrix1 = m;
                }
            }

            x if x == tcColorMatrix2 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }

                // Kludge - Hasselblad FFF files are very DNG-like, but
                // sometimes only have a ColorMatrix2 tag and no ColorMatrix1
                // tag (dng_shared.cpp tcColorMatrix2).

                let hasselblad_hack = self.color_planes == 0;

                if hasselblad_hack {
                    self.color_planes = crate::dng_utils::pin_uint32(
                        0,
                        tag_count / 3,
                        crate::dng_sdk_limits::kMaxColorPlanes,
                    );
                }

                if !check_color_image(parent_code, tag_code, self.color_planes) {
                    return Ok(false);
                }

                if tag_count != self.color_planes * 3 {
                    return Ok(false);
                }

                if let Some(m) = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    self.color_planes,
                    3,
                )? {
                    self.color_matrix2 = m;
                }

                if hasselblad_hack {
                    self.color_matrix1 = self.color_matrix2.clone();
                    self.color_matrix2 = DngMatrix::default();
                }
            }

            x if x == tcColorMatrix3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }

                if self.color_planes == 0 {
                    self.color_planes = crate::dng_utils::pin_uint32(
                        0,
                        tag_count / 3,
                        crate::dng_sdk_limits::kMaxColorPlanes,
                    );
                }

                if !check_color_image(parent_code, tag_code, self.color_planes) {
                    return Ok(false);
                }

                if tag_count != self.color_planes * 3 {
                    return Ok(false);
                }

                if let Some(m) = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    self.color_planes,
                    3,
                )? {
                    self.color_matrix3 = m;
                }
            }

            x if x == tcForwardMatrix1 || x == tcForwardMatrix2 || x == tcForwardMatrix3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }

                if !check_color_image(parent_code, tag_code, self.color_planes) {
                    return Ok(false);
                }

                // C++ ParseMatrixTag(stream, ..., 3, fColorPlanes, matrix).

                if let Some(m) = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    self.color_planes,
                )? {
                    match tag_code {
                        tcForwardMatrix1 => self.forward_matrix1 = m,
                        tcForwardMatrix2 => self.forward_matrix2 = m,
                        _ => self.forward_matrix3 = m,
                    }
                }
            }

            x if x == tcReductionMatrix1 || x == tcReductionMatrix2 || x == tcReductionMatrix3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }

                if !check_color_image(parent_code, tag_code, self.color_planes) {
                    return Ok(false);
                }

                if let Some(m) = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    self.color_planes,
                )? {
                    match tag_code {
                        tcReductionMatrix1 => self.reduction_matrix1 = m,
                        tcReductionMatrix2 => self.reduction_matrix2 = m,
                        _ => self.reduction_matrix3 = m,
                    }
                }
            }

            x if x == tcIlluminantData1 || x == tcIlluminantData2 || x == tcIlluminantData3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 10000) {
                    return Ok(false);
                }

                let data = match tag_code {
                    tcIlluminantData1 => &mut self.illuminant_data1,
                    tcIlluminantData2 => &mut self.illuminant_data2,
                    _ => &mut self.illuminant_data3,
                };

                data.get_from_stream(stream, tag_count)?;
            }

            x if x == tcProfileEmbedPolicy => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.embed_policy = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcProfileHueSatMapData1 || x == tcProfileHueSatMapData2 || x == tcProfileHueSatMapData3 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttFloat as uint16]) {
                    return Ok(false);
                }
                if self.profile_sats == 0 {
                    return Ok(false);
                }

                // Either the full hue*sat*val*3 table, or a table with the
                // sat==0 row skipped (dng_shared.cpp).

                let hue_count = self.profile_hues;
                let val_count = self.profile_vals;

                let skip_sat0 = tag_count
                    == hue_count
                        .checked_mul(self.profile_sats.wrapping_sub(1))
                        .unwrap_or(0)
                        .checked_mul(val_count * 3)
                        .unwrap_or(0);

                if !skip_sat0 {
                    let expected = hue_count
                        .checked_mul(val_count)
                        .and_then(|v| v.checked_mul(self.profile_sats))
                        .and_then(|v| v.checked_mul(3))
                        .unwrap_or(u32::MAX);

                    if !check_tag_count(parent_code, tag_code, tag_count, expected, 0) {
                        return Ok(false);
                    }
                }

                self.big_endian = stream.big_endian();

                let (offset_field, count_field) = match tag_code {
                    tcProfileHueSatMapData1 => (
                        &mut self.hue_sat_deltas1_offset,
                        &mut self.hue_sat_deltas1_count,
                    ),
                    tcProfileHueSatMapData2 => (
                        &mut self.hue_sat_deltas2_offset,
                        &mut self.hue_sat_deltas2_count,
                    ),
                    _ => (
                        &mut self.hue_sat_deltas3_offset,
                        &mut self.hue_sat_deltas3_count,
                    ),
                };

                *offset_field = tag_offset;
                *count_field = tag_count;
            }

            x if x == tcProfileHueSatMapEncoding => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.hue_sat_map_encoding = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcProfileLookTableDims => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16, ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 3) {
                    return Ok(false);
                }
                self.look_table_hues = stream.tag_value_uint32(tag_type)?;
                self.look_table_sats = stream.tag_value_uint32(tag_type)?;
                if tag_count > 2 {
                    self.look_table_vals = stream.tag_value_uint32(tag_type)?;
                } else {
                    self.look_table_vals = 1;
                }
            }

            x if x == tcProfileLookTableData => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttFloat as uint16]) {
                    return Ok(false);
                }
                if self.look_table_sats == 0 {
                    return Ok(false);
                }

                let hue_count = self.look_table_hues;
                let val_count = self.look_table_vals;

                let skip_sat0 = tag_count
                    == hue_count
                        .checked_mul(self.look_table_sats.wrapping_sub(1))
                        .unwrap_or(0)
                        .checked_mul(val_count * 3)
                        .unwrap_or(0);

                if !skip_sat0 {
                    let expected = hue_count
                        .checked_mul(val_count)
                        .and_then(|v| v.checked_mul(self.look_table_sats))
                        .and_then(|v| v.checked_mul(3))
                        .unwrap_or(u32::MAX);

                    if !check_tag_count(parent_code, tag_code, tag_count, expected, 0) {
                        return Ok(false);
                    }
                }

                self.big_endian = stream.big_endian();

                self.look_table_offset = tag_offset;
                self.look_table_count = tag_count;
            }

            x if x == tcProfileLookTableEncoding => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.look_table_encoding = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcBaselineExposureOffset => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.baseline_exposure_offset = stream.tag_value_srational(tag_type)?;
            }

            x if x == tcDefaultBlackRender => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.default_black_render = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcProfileToneMethod => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.tone_method = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcUniqueCameraModel => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]) {
                    return Ok(false);
                }
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.unique_camera_model,
                    false,
                )?;
            }

            x if x == tcProfileGroupName => {
                if !check_tag_type(
                    parent_code,
                    tag_code,
                    tag_type,
                    &[ttAscii as uint16, ttByte as uint16],
                ) {
                    return Ok(false);
                }
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.profile_group_name,
                    false,
                )?;
            }

            x if x == tcProfileGainTableMap2 => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]) {
                    return Ok(false);
                }

                // The C++ parses the gain table map inline via
                // dng_gain_table_map::GetStream; the type lives in dng-sdk,
                // so record the extent and let the SDK layer parse it.

                self.profile_gain_table_map_offset = tag_offset;
                self.profile_gain_table_map_count = tag_count;
            }

            x if x == tcProfileDynamicRange => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 8, 0) {
                    return Ok(false);
                }

                let version = stream.get_uint16()?;

                if version != 1 {
                    return Err(bad_format()); // "Unsupported version in ProfileDynamicRange"
                }

                self.profile_dynamic_range.version = version;
                self.profile_dynamic_range.dynamic_range = stream.get_uint16()?;
                self.profile_dynamic_range.hint_max_output_value = stream.get_real32()?;
            }

            x if x == tcRGBTablesDraft || x == tcRGBTables => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttUndefined as uint16]) {
                    return Ok(false);
                }

                // Parsed inline by the C++ (dng_masked_rgb_tables::GetStream);
                // the type lives in dng-sdk, so record the extent.

                self.masked_rgb_tables_offset = tag_offset;
                self.masked_rgb_tables_count = tag_count;
                self.masked_rgb_tables_is_draft = tag_code == tcRGBTablesDraft;
            }

            x if x == tcProfileCalibrationSignature => {
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.profile_calibration_signature,
                    false,
                )?;
            }

            x if x == tcProfileName => {
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.profile_name,
                    false,
                )?;
            }

            x if x == tcProfileCopyright => {
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.profile_copyright,
                    false,
                )?;
            }

            x if x == tcProfileHueSatMapDims => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16, ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 3) {
                    return Ok(false);
                }
                self.profile_hues = stream.tag_value_uint32(tag_type)?;
                self.profile_sats = stream.tag_value_uint32(tag_type)?;

                if tag_count > 2 {
                    self.profile_vals = stream.tag_value_uint32(tag_type)?;
                } else {
                    self.profile_vals = 1;
                }
            }

            x if x == tcProfileToneCurve => {
                self.tone_curve_offset = tag_offset;
                self.tone_curve_count = tag_count;
            }

            _ => {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// `ParseExtended (stream)` — parse a standalone extended camera-profile
    /// block ("skipped" data referenced by offsets).
    ///
    /// `ParseExtended (stream)` — parse an embedded "extended" camera-profile
    /// block: a mini TIFF directory whose byte-order marker comes first and
    /// whose magic is 0x4352 'CR' (dng_shared.cpp:1572).
    pub fn parse_extended(&mut self, stream: &mut DngStream) -> DngResult<bool> {
        let start_position = stream.get_position();

        let byte_order = stream.get_uint16()?;

        let big_endian = if byte_order == byteOrderMM as uint16 {
            true
        } else if byte_order == byteOrderII as uint16 {
            false
        } else {
            return Ok(false);
        };

        let saved_endian = stream.big_endian();
        stream.set_big_endian(big_endian);

        let result = (|| -> DngResult<bool> {
            let magic_number = stream.get_uint16()?;
            if magic_number != magicExtendedProfile as uint16 {
                return Ok(false);
            }

            let offset = stream.get_uint32()?;

            stream.skip(crate::dng_safe_arithmetic::safe_uint32_sub(offset, 8)? as uint64)?;

            let ifd_entries = stream.get_uint16()?;
            if ifd_entries < 1 {
                return Ok(false);
            }

            for tag_index in 0..ifd_entries {
                stream.set_position(start_position + 8 + 2 + tag_index as uint64 * 12)?;

                let tag_code = stream.get_uint16()? as uint32;
                let tag_type = stream.get_uint16()? as uint32;
                let tag_count = stream.get_uint32()?;
                let mut tag_offset = stream.get_position();

                if tag_type_size(tag_type) as uint64 * tag_count as uint64 > 4 {
                    tag_offset = start_position + stream.get_uint32()? as uint64;
                    stream.set_position(tag_offset)?;
                }

                // C++ ignores per-tag failures in non-validate builds (the
                // loop just continues).

                let _ = self.parse_tag(stream, 0, tag_code, tag_type, tag_count, tag_offset)?;
            }

            Ok(true)
        })();

        stream.set_big_endian(saved_endian);
        result
    }
}

/// C++ Hasselblad hack for tcAsShotNeutral without ColorMatrix1: infer the
/// plane count from the tag count.
fn shared_color_planes_hack(color_planes: uint32, tag_count: uint32) -> uint32 {
    if color_planes == 0 {
        pin_uint32(0, tag_count, crate::dng_sdk_limits::kMaxColorPlanes)
    } else {
        color_planes
    }
}

/// Port of `dng_shared`.
#[derive(Debug, Clone, Default)]
pub struct DngShared {
    pub exif_ifd: uint64,
    pub gps_info: uint64,
    pub interoperability_ifd: uint64,
    pub kodak_dcr_private_ifd: uint64,
    pub kodak_kdc_private_ifd: uint64,

    pub xmp_count: uint32,
    pub xmp_offset: uint64,

    pub iptc_naa_count: uint32,
    pub iptc_naa_offset: uint64,

    pub maker_note_count: uint32,
    pub maker_note_offset: uint64,
    pub maker_note_safety: uint32,

    pub dng_version: uint32,
    pub dng_backward_version: uint32,

    pub unique_camera_model: DngString,
    pub localized_camera_model: DngString,

    pub camera_profile: DngCameraProfileInfo,
    pub extra_camera_profiles: Vec<DngCameraProfileInfo>,

    pub camera_calibration1: DngMatrix,
    pub camera_calibration2: DngMatrix,
    pub camera_calibration3: DngMatrix,

    pub camera_calibration_signature: DngString,

    pub analog_balance: Option<crate::dng_matrix::DngVector>,
    pub as_shot_neutral: Option<crate::dng_matrix::DngVector>,
    pub as_shot_white_xy: DngXyCoord,

    pub baseline_exposure: DngSRational,
    pub baseline_noise: DngURational,
    pub baseline_sharpness: DngURational,
    pub linear_response_limit: DngURational,
    pub shadow_scale: DngSRational,

    pub has_baseline_exposure: bool,
    pub has_shadow_scale: bool,

    pub dng_private_data_count: uint32,
    pub dng_private_data_offset: uint64,

    pub raw_image_digest: DngFingerprint,
    pub new_raw_image_digest: DngFingerprint,
    pub raw_data_unique_id: DngFingerprint,

    pub original_raw_file_name: DngString,
    pub original_raw_file_data_count: uint32,
    pub original_raw_file_data_offset: uint64,
    pub original_raw_file_digest: DngFingerprint,

    pub as_shot_icc_profile_count: uint32,
    pub as_shot_icc_profile_offset: uint64,
    pub as_shot_pre_profile_matrix: DngMatrix,

    pub current_icc_profile_count: uint32,
    pub current_icc_profile_offset: uint64,
    pub current_pre_profile_matrix: DngMatrix,

    pub colorimetric_reference: uint32,

    pub as_shot_profile_name: DngString,

    pub original_default_final_size: crate::dng_point::DngPoint,
    pub original_best_quality_final_size: crate::dng_point::DngPoint,
    pub original_default_crop_size_h: DngURational,
    pub original_default_crop_size_v: DngURational,

    pub depth_format: uint32,
    pub depth_near: DngURational,
    pub depth_far: DngURational,
    pub depth_units: uint32,
    pub depth_measure_type: uint32,

    pub big_table_digests: Vec<DngFingerprint>,
    pub big_table_offsets: Vec<uint64>,
    pub big_table_byte_counts: Vec<uint32>,

    /// Group index pairs for BigTable resources (dng_big_table group index).
    pub big_table_group_index: Vec<(DngFingerprint, DngFingerprint)>,

    pub image_sequence_info: crate::dng_utils::DngImageSequenceInfo,
}

impl DngShared {
    pub fn new() -> Self {
        DngShared {
            exif_ifd: 0,
            gps_info: 0,
            interoperability_ifd: 0,
            kodak_dcr_private_ifd: 0,
            kodak_kdc_private_ifd: 0,
            camera_profile: DngCameraProfileInfo::new(),
            baseline_noise: DngURational::new(1, 1),
            baseline_sharpness: DngURational::new(1, 1),
            linear_response_limit: DngURational::new(1, 1),
            colorimetric_reference: 0,
            ..Default::default()
        }
    }

    /// `ParseTag (stream, exif, parentCode, isMainIFD, tagCode, tagType,
    /// tagCount, tagOffset, offsetDelta)` — the dng_shared-owned tags.
    /// `offset_delta` mirrors the C++ (used for maker-note pointer fixups;
    /// phase-1 keeps the value for callers).
    #[allow(clippy::too_many_arguments)]
    pub fn parse_tag(
        &mut self,
        stream: &mut DngStream,
        parent_code: uint32,
        is_main_ifd: bool,
        tag_code: uint32,
        tag_type: uint32,
        tag_count: uint32,
        tag_offset: uint64,
        offset_delta: int64,
    ) -> DngResult<bool> {
        let _ = (is_main_ifd, offset_delta);

        // Camera-profile tags live on fCameraProfile in the C++
        // (dng_shared::ParseTag handles them inline on the embedded
        // profile-info struct); delegate those first.
        match tag_code {
            x if x == tcCalibrationIlluminant1
                || x == tcCalibrationIlluminant2
                || x == tcCalibrationIlluminant3
                || x == tcColorMatrix1
                || x == tcColorMatrix2
                || x == tcProfileCalibrationSignature
                || x == tcProfileCopyright
                || x == tcProfileHueSatMapDims
                || x == tcProfileName
                || x == tcProfileToneCurve =>
            {
                return self
                    .camera_profile
                    .parse_tag(stream, parent_code, tag_code, tag_type, tag_count, tag_offset);
            }
            _ => {}
        }

        match tag_code {
            x if x == tcDNGVersion => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 4, 0) {
                    return Ok(false);
                }
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.dng_version = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }

            x if x == tcDNGBackwardVersion => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 4, 0) {
                    return Ok(false);
                }
                let b0 = stream.get_uint8()? as uint32;
                let b1 = stream.get_uint8()? as uint32;
                let b2 = stream.get_uint8()? as uint32;
                let b3 = stream.get_uint8()? as uint32;
                self.dng_backward_version = (b0 << 24) | (b1 << 16) | (b2 << 8) | b3;
            }

            x if x == tcUniqueCameraModel || x == tcLocalizedCameraModel =>
            {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttAscii as uint16]) {
                    return Ok(false);
                }

                let mut s = if tag_code == tcUniqueCameraModel {
                    std::mem::take(&mut self.unique_camera_model)
                } else {
                    std::mem::take(&mut self.localized_camera_model)
                };

                parse_string_tag(stream, parent_code, tag_code, tag_count, &mut s, true)?;

                if tag_code == tcUniqueCameraModel {
                    self.unique_camera_model = s;
                } else {
                    self.localized_camera_model = s;
                }
            }

            x if x == tcCameraCalibrationSignature => {
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.camera_calibration_signature,
                    false,
                )?;
            }

            x if x == tcAnalogBalance => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);
                if !check_color_image(parent_code, tag_code, tag_count) {
                    return Ok(false);
                }
                self.analog_balance = parse_vector_tag(
                    stream,
                    parent_code,
                    tag_code,
                    ttSRational,
                    tag_count,
                    tag_count,
                )?;
            }

            x if x == tcAsShotNeutral => {
                check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]);

                // Kludge - Hasselblad FFF files are very DNG-like, but
                // sometimes they have no ColorMatrix tags.
                if self.dng_version == 0 && self.camera_profile.color_planes == 0 {
                    self.camera_profile.color_planes =
                        crate::dng_utils::pin_uint32(0, tag_count, crate::dng_sdk_limits::kMaxColorPlanes);
                }

                if !check_color_image(parent_code, tag_code, self.camera_profile.color_planes) {
                    return Ok(false);
                }
                self.as_shot_neutral = parse_vector_tag(
                    stream,
                    parent_code,
                    tag_code,
                    ttSRational,
                    tag_count,
                    self.camera_profile.color_planes,
                )?;
            }

            x if x == tcAsShotWhiteXY => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 0) {
                    return Ok(false);
                }
                let x = stream.tag_value_urational(ttRational)?;
                let y = stream.tag_value_urational(ttRational)?;
                self.as_shot_white_xy = DngXyCoord::new(x.as_real64(), y.as_real64());
            }

            x if x == tcBaselineExposure => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.baseline_exposure = stream.tag_value_srational(ttSRational)?;
                self.has_baseline_exposure = true;
            }

            x if x == tcBaselineNoise => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.baseline_noise = stream.tag_value_urational(ttRational)?;
            }

            x if x == tcBaselineSharpness => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.baseline_sharpness = stream.tag_value_urational(ttRational)?;
            }

            x if x == tcLinearResponseLimit => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.linear_response_limit = stream.tag_value_urational(ttRational)?;
            }

            x if x == tcShadowScale => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttSRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.shadow_scale = stream.tag_value_srational(ttSRational)?;
                self.has_shadow_scale = true;
            }

            x if x == tcExifIFD || x == tcGPSInfo || x == tcInteroperabilityIFD => {
                if !check_tag_type(
                    parent_code,
                    tag_code,
                    tag_type,
                    &[ttLong as uint16, ttLong8 as uint16],
                ) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }

                let value = stream.tag_value_uint64(tag_type)?;

                match tag_code {
                    tcExifIFD => self.exif_ifd = value,
                    tcGPSInfo => self.gps_info = value,
                    _ => self.interoperability_ifd = value,
                }
            }

            x if x == tcDNGPrivateData => {
                self.dng_private_data_count = tag_count;
                self.dng_private_data_offset = tag_offset;
                // Data read lazily (offset preserved), as in C++.
                stream.skip(tag_count as uint64)?;
            }

            x if x == tcRawImageDigest || x == tcNewRawImageDigest => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 16, 0) {
                    return Ok(false);
                }
                let mut digest = DngFingerprint::new();
                stream.get_into(&mut digest.data, 0)?;
                if tag_code == tcRawImageDigest {
                    self.raw_image_digest = digest;
                } else {
                    self.new_raw_image_digest = digest;
                }
            }

            x if x == tcRawDataUniqueID => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 16, 0) {
                    return Ok(false);
                }
                let mut digest = DngFingerprint::new();
                stream.get_into(&mut digest.data, 0)?;
                self.raw_data_unique_id = digest;
            }

            x if x == tcOriginalRawFileName => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16, ttAscii as uint16]) {
                    return Ok(false);
                }
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.original_raw_file_name,
                    true,
                )?;
            }

            x if x == tcOriginalRawFileData => {
                self.original_raw_file_data_count = tag_count;
                self.original_raw_file_data_offset = tag_offset;
                stream.skip(tag_count as uint64)?;
            }

            x if x == tcOriginalRawFileDigest => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttByte as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 16, 0) {
                    return Ok(false);
                }
                let mut digest = DngFingerprint::new();
                stream.get_into(&mut digest.data, 0)?;
                self.original_raw_file_digest = digest;
            }

            x if x == tcAsShotICCProfile => {
                self.as_shot_icc_profile_count = tag_count;
                self.as_shot_icc_profile_offset = tag_offset;
                stream.skip(tag_count as uint64)?;
            }

            x if x == tcAsShotPreProfileMatrix => {
                if tag_count % 3 != 0 {
                    return Ok(false);
                }
                self.as_shot_pre_profile_matrix = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    tag_count / 3,
                )?
                .unwrap_or_default();
            }

            x if x == tcCurrentICCProfile => {
                self.current_icc_profile_count = tag_count;
                self.current_icc_profile_offset = tag_offset;
                stream.skip(tag_count as uint64)?;
            }

            x if x == tcCurrentPreProfileMatrix => {
                if tag_count % 3 != 0 {
                    return Ok(false);
                }
                self.current_pre_profile_matrix = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    tag_count / 3,
                )?
                .unwrap_or_default();
            }

            x if x == tcColorimetricReference => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.colorimetric_reference = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcAsShotProfileName => {
                parse_string_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_count,
                    &mut self.as_shot_profile_name,
                    false,
                )?;
            }

            x if x == tcOriginalDefaultFinalSize => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 0) {
                    return Ok(false);
                }
                let y = stream.tag_value_uint32(tag_type)?;
                let x = stream.tag_value_uint32(tag_type)?;
                self.original_default_final_size = crate::dng_point::DngPoint::new(y as i32, x as i32);
            }

            x if x == tcOriginalBestQualityFinalSize => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 0) {
                    return Ok(false);
                }
                let y = stream.tag_value_uint32(tag_type)?;
                let x = stream.tag_value_uint32(tag_type)?;
                self.original_best_quality_final_size =
                    crate::dng_point::DngPoint::new(y as i32, x as i32);
            }

            x if x == tcOriginalDefaultCropSize => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttLong as uint16, ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 2, 0) {
                    return Ok(false);
                }
                self.original_default_crop_size_h = stream.tag_value_urational(tag_type)?;
                self.original_default_crop_size_v = stream.tag_value_urational(tag_type)?;
            }

            x if x == tcDepthFormat => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.depth_format = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcDepthNear => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.depth_near = stream.tag_value_urational(ttRational)?;
            }

            x if x == tcDepthFar => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttRational as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.depth_far = stream.tag_value_urational(ttRational)?;
            }

            x if x == tcDepthUnits => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.depth_units = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcDepthMeasureType => {
                if !check_tag_type(parent_code, tag_code, tag_type, &[ttShort as uint16]) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, 0) {
                    return Ok(false);
                }
                self.depth_measure_type = stream.tag_value_uint32(tag_type)?;
            }

            x if x == tcImageSequenceInfo => {
                // Parse directly (this tag data is inline).
                self.image_sequence_info
                    .parse_sequence_info_stream(stream, tag_count)?;
            }

            x if x == tcExtraCameraProfiles => {
                if !check_tag_type(
                    parent_code,
                    tag_code,
                    tag_type,
                    &[ttLong as uint16, ttLong8 as uint16],
                ) {
                    return Ok(false);
                }
                if !check_tag_count(parent_code, tag_code, tag_count, 1, tag_count) {
                    return Ok(false);
                }

                // Each entry is an absolute offset to an embedded "extended"
                // camera-profile block (a mini TIFF directory).

                for _index in 0..tag_count {
                    let profile_offset = stream.tag_value_uint64(tag_type)?;

                    let saved_position = stream.get_position();
                    stream.set_position(profile_offset)?;

                    let mut profile_info = DngCameraProfileInfo::new();

                    let parsed = profile_info.parse_extended(stream)?;

                    if parsed {
                        self.extra_camera_profiles.push(profile_info);
                    }

                    stream.set_position(saved_position)?;
                }
            }

            x if x == tcCameraCalibration1 => {
                if tag_count % 3 != 0 {
                    return Ok(false);
                }
                self.camera_calibration1 = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    tag_count / 3,
                )?
                .unwrap_or_default();
            }

            x if x == tcCameraCalibration2 => {
                if tag_count % 3 != 0 {
                    return Ok(false);
                }
                self.camera_calibration2 = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    tag_count / 3,
                )?
                .unwrap_or_default();
            }

            x if x == tcCameraCalibration3 => {
                if tag_count % 3 != 0 {
                    return Ok(false);
                }
                self.camera_calibration3 = parse_matrix_tag(
                    stream,
                    parent_code,
                    tag_code,
                    tag_type,
                    tag_count,
                    3,
                    tag_count / 3,
                )?
                .unwrap_or_default();
            }

            _ => {
                // Not a dng_shared tag (EXIF/IPTC handled by dng_info/phase 2).
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// `dng_shared::IsValidDNG ()` — C++-faithful: checks the DNGVersion
    /// floor, the DNGBackwardVersion ceiling, and (for multi-plane images)
    /// the size/invertibility of the optional camera calibrations and the
    /// analog balance. Note the C++ deliberately does NOT require a color
    /// matrix or AsShotNeutral here (monochrome DNGs have neither).
    pub fn is_valid_dng(&self) -> bool {
        // Check DNGVersion value.

        if self.dng_version < dngVersion_1_0_0_0 {
            return false;
        }

        // Check DNGBackwardVersion value. The C++ throws
        // ThrowUnsupportedDNG here; this bool predicate reports it as
        // invalid (PORT-NOTE: throw -> false).

        if self.dng_backward_version > dngVersion_Current {
            return false;
        }

        // Check color transform info.

        if self.camera_profile.color_planes > 1 {
            let planes = self.camera_profile.color_planes;

            // CameraCalibration1/2/3 are optional, but each must be the
            // right size and invertible if present.

            for calib in [
                &self.camera_calibration1,
                &self.camera_calibration2,
                &self.camera_calibration3,
            ] {
                if calib.cols() != 0 || calib.rows() != 0 {
                    if calib.cols() != planes || calib.rows() != planes {
                        return false;
                    }

                    // Make sure it is invertible.

                    if calib.inverted().is_err() {
                        return false;
                    }
                }
            }

            // Check analog balance.

            if let Some(analog_balance) = &self.analog_balance {
                if !analog_balance.is_empty() {
                    match analog_balance.as_diagonal() {
                        Ok(diag) => {
                            if diag.inverted().is_err() {
                                return false;
                            }
                        }
                        Err(_) => return false,
                    }
                }
            }
        }

        true
    }
}

#[allow(unused)]
fn unused(_: Pin_uint64_unused, _: real32) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_memory_stream::DngMemoryStream;

    #[test]
    fn dng_version_parse() {
        let mut shared = DngShared::new();
        let mut stream = DngMemoryStream::new().stream();

        // 1.4.0.0
        stream.put_uint8(1).unwrap();
        stream.put_uint8(4).unwrap();
        stream.put_uint8(0).unwrap();
        stream.put_uint8(0).unwrap();
        stream.set_read_position(0).unwrap();

        assert!(shared
            .parse_tag(&mut stream, 0, false, tcDNGVersion, ttByte as uint32, 4, 0, 0)
            .unwrap());
        assert_eq!(shared.dng_version, dngVersion_1_4_0_0);
    }

    #[test]
    fn as_shot_neutral_parse() {
        let mut shared = DngShared::new();
        let mut stream = DngMemoryStream::new().stream();
        stream.set_big_endian(true);

        // Three rationals 1/1.
        for _ in 0..3 {
            stream.put_uint32(1).unwrap();
            stream.put_uint32(1).unwrap();
        }
        stream.set_read_position(0).unwrap();

        assert!(shared
            .parse_tag(
                &mut stream, 0, false, tcAsShotNeutral, ttRational as uint32, 3, 0, 0
            )
            .unwrap());

        let v = shared.as_shot_neutral.as_ref().unwrap();
        assert_eq!(v.count(), 3);
        assert_eq!(v.get(0), 1.0);
    }

    #[test]
    fn unhandled_tag_returns_false() {
        let mut shared = DngShared::new();
        let mut stream = DngMemoryStream::new().stream();
        assert!(!shared
            .parse_tag(&mut stream, 0, false, 0xDEAD, ttLong as uint32, 1, 0, 0)
            .unwrap());
    }

    #[test]
    fn is_valid_dng_requirements() {
        let mut shared = DngShared::new();
        assert!(!shared.is_valid_dng()); // no version (C++: < dngVersion_1_0_0_0)

        // Version alone is enough: the C++ IsValidDNG does not require a
        // color matrix or AsShotNeutral (monochrome DNGs have neither).
        shared.dng_version = dngVersion_1_4_0_0;
        assert!(shared.is_valid_dng());

        shared.camera_profile.color_planes = 3;
        shared.camera_profile.color_matrix1 =
            DngMatrix::from_rows(3, 3, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        shared.as_shot_neutral =
            Some(crate::dng_matrix::DngVector::from_values(&[1.0, 1.0, 1.0]).unwrap());
        assert!(shared.is_valid_dng());

        // A wrong-size or non-invertible camera calibration invalidates.

        shared.camera_calibration1 =
            DngMatrix::from_rows(3, 3, &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
        assert!(!shared.is_valid_dng()); // singular calibration1
    }

    #[test]
    fn dynamic_range_helpers() {
        let mut dr = DngCameraProfileDynamicRange::default();
        dr.version = 1;
        assert!(dr.is_valid());
        assert!(dr.is_sdr());
        dr.dynamic_range = 1000;
        assert!(dr.is_hdr());
    }
}

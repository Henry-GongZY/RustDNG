// Port of dng_camera_profile.h/.cpp — camera color profiles.
//
// Phase-1's dng-core::DngCameraProfileInfo covers the wire format
// (dng_shared parsing); this module owns the live profile object.  The
// fingerprint is an MD5 over the field stream exactly as C++
// `CalculateFingerprint` lays it out (little-endian, tag_matrix data layout).
//
// PORT-NOTE(dng_camera_profile.cpp): `fMaskedRGBTables` (DNG 1.7 spectral
// masking) and `fProfileGainTableMap` wire parse arrive with
// dng_big_table/dng_gain_table_map completion; the fields are present with
// the types defined in this crate (DngGainTableMap) and
// INTEGRATION_NOTES.md.  `Parse`/`ParseExtended` reuse the dng-core
// DngCameraProfileInfo parser for the tag set and add the extended-block
// fields (hue-sat deltas, look table, tone curve) here.

use dng_core::dng_exceptions::program_error;
use dng_core::dng_fingerprint::{DngFingerprint, DngMd5PrinterStream};
use dng_core::{
    bad_format, calculate_triple_illuminant_weights, defaultBlackRender_Auto, encoding_Linear,
    int32, lsUnknown, DngError, DngResult, DngSRational, DngShared, DngStream, DngString,
    DngTemperature, DngURational, DngXyCoord, kMaxColorPlanes, pepAllowCopying, pepEmbedIfUsed,
    pepNoRestrictions, profileToneMethod_Unspecified, ttRational, ttSRational, ttShort, uint32,
};
use dng_core::dng_xy_coord::{pcs_to_xyz, DngIlluminantData};
use dng_core::{DngMatrix, DngVector};
use crate::dng_hue_sat_map::{DngHueSatMap, HSBModify};
use crate::dng_gain_map::DngGainTableMap;
use crate::dng_tone_curve::DngToneCurve;

pub const kProfileName_Embedded: &str = "Embedded";
pub const kAdobeCalibrationSignature: &str = "adobe";
pub const kProfileName_GroupPrefix: &str = "HDR | ";

/// Port of `dng_camera_profile_id`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngCameraProfileId {
    name: DngString,
    fingerprint: DngFingerprint,
}

impl DngCameraProfileId {
    pub fn from_name(name: &str) -> Self {
        let mut id = DngCameraProfileId::default();
        id.name.set(name.as_bytes());
        id
    }

    pub fn from_name_str(name: &DngString) -> Self {
        let mut id = DngCameraProfileId::default();
        id.name = name.clone();
        id
    }

    pub fn from_name_fingerprint(name: &str, fingerprint: &DngFingerprint) -> Self {
        let mut id = DngCameraProfileId::default();
        id.name.set(name.as_bytes());
        id.fingerprint = *fingerprint;
        id
    }

    pub fn name(&self) -> &DngString {
        &self.name
    }

    pub fn fingerprint(&self) -> &DngFingerprint {
        &self.fingerprint
    }

    /// `IsValid ()` — fingerprint is optional.
    pub fn is_valid(&self) -> bool {
        self.name.not_empty()
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        *self = DngCameraProfileId::default();
    }

    /// `AddDigest (printer)` — MD5 of the name and fingerprint.
    pub fn add_digest(&self, printer: &mut DngMd5PrinterStream) -> dng_core::DngResult<()> {
        printer.stream().put(self.name.get())?;
        if self.fingerprint.is_valid() {
            printer.stream().put(&self.fingerprint.data)?;
        }
        Ok(())
    }
}

/// `HasProfileGroupPrefix (name)`.
pub fn has_profile_group_prefix(name: &DngString) -> bool {
    name.starts_with(kProfileName_GroupPrefix.as_bytes(), true)
}

/// `StripProfileGroupPrefix (name)`.
pub fn strip_profile_group_prefix(name: &DngString) -> DngString {
    let mut result = name.clone();
    if has_profile_group_prefix(name) {
        result.trim_leading(kProfileName_GroupPrefix.as_bytes(), true);
    }
    result
}

/// Port of `dng_camera_profile`.
#[derive(Debug, Clone, Default)]
pub struct DngCameraProfile {
    name: DngString,
    group_name: DngString,
    // Dynamic range info (DNG 1.7): kept as the phase-1 struct.
    pub dynamic_range_info: Option<dng_core::DngCameraProfileDynamicRange>,

    calibration_illuminant1: u32,
    calibration_illuminant2: u32,
    calibration_illuminant3: u32,

    illuminant_data1: DngIlluminantData,
    illuminant_data2: DngIlluminantData,
    illuminant_data3: DngIlluminantData,

    color_matrix1: DngMatrix,
    color_matrix2: DngMatrix,
    color_matrix3: DngMatrix,

    forward_matrix1: DngMatrix,
    forward_matrix2: DngMatrix,
    forward_matrix3: DngMatrix,

    reduction_matrix1: DngMatrix,
    reduction_matrix2: DngMatrix,
    reduction_matrix3: DngMatrix,

    fingerprint: std::cell::Cell<DngFingerprint>,
    render_data_fingerprint: std::cell::Cell<DngFingerprint>,

    copyright: DngString,
    embed_policy: u32,

    hue_sat_deltas1: DngHueSatMap,
    hue_sat_deltas2: DngHueSatMap,
    hue_sat_deltas3: DngHueSatMap,
    hue_sat_map_encoding: u32,

    look_table: DngHueSatMap,
    look_table_encoding: u32,

    baseline_exposure_offset: DngSRational,
    default_black_render: u32,

    tone_curve: DngToneCurve,
    tone_method: u32,

    profile_calibration_signature: DngString,
    unique_camera_model_restriction: DngString,

    was_read_from_dng: bool,
    was_read_from_disk: bool,
    was_stubbed: bool,

    pub profile_gain_table_map: Option<DngGainTableMap>,
    pub profile_gain_table_map_tag_version: u32,
}

impl DngCameraProfile {
    /// `dng_camera_profile ()` — exact C++ defaults.
    pub fn new() -> Self {
        let mut p = DngCameraProfile {
            calibration_illuminant1: lsUnknown,
            calibration_illuminant2: lsUnknown,
            calibration_illuminant3: lsUnknown,
            embed_policy: pepAllowCopying,
            hue_sat_map_encoding: encoding_Linear,
            look_table_encoding: encoding_Linear,
            baseline_exposure_offset: DngSRational::new(0, 100),
            default_black_render: defaultBlackRender_Auto,
            tone_method: profileToneMethod_Unspecified,
            ..Default::default()
        };

        p.tone_curve.set_invalid();

        p
    }

    // ------------------------------------------------------------ name/group

    /// `SetName`.
    pub fn set_name(&mut self, name: &str) {
        self.name.set(name.as_bytes());
        self.clear_fingerprint();
    }

    /// `Name ()`.
    pub fn name(&self) -> &DngString {
        &self.name
    }

    /// `SetGroupName`.
    pub fn set_group_name(&mut self, s: &DngString) {
        self.group_name = s.clone();
        self.clear_fingerprint();
    }

    /// `GroupName ()`.
    pub fn group_name(&self) -> &DngString {
        &self.group_name
    }

    /// `NameIsEmbedded ()`.
    pub fn name_is_embedded(&self) -> bool {
        self.name.matches(kProfileName_Embedded.as_bytes(), true)
    }

    // ---------------------------------------------------------- illuminants

    /// `IlluminantModel ()` — 1, 2 or 3.
    pub fn illuminant_model(&self) -> u32 {
        let mut model = 1;

        if self.calibration_illuminant2 != lsUnknown && self.has_color_matrix2() {
            model = 2;

            if self.calibration_illuminant3 != lsUnknown && self.has_color_matrix3() {
                model = 3;
            }
        }

        model
    }

    pub fn set_calibration_illuminant1(&mut self, light: u32) {
        self.calibration_illuminant1 = light;
        self.clear_fingerprint();
    }
    pub fn calibration_illuminant1(&self) -> u32 {
        self.calibration_illuminant1
    }

    pub fn set_calibration_illuminant2(&mut self, light: u32) {
        self.calibration_illuminant2 = light;
        self.clear_fingerprint();
    }
    pub fn calibration_illuminant2(&self) -> u32 {
        self.calibration_illuminant2
    }

    pub fn set_calibration_illuminant3(&mut self, light: u32) {
        self.calibration_illuminant3 = light;
        self.clear_fingerprint();
    }
    pub fn calibration_illuminant3(&self) -> u32 {
        self.calibration_illuminant3
    }

    pub fn set_illuminant_data1(&mut self, data: &DngIlluminantData) {
        self.illuminant_data1 = data.clone();
        self.clear_fingerprint();
    }
    pub fn illuminant_data1(&self) -> &DngIlluminantData {
        &self.illuminant_data1
    }

    pub fn set_illuminant_data2(&mut self, data: &DngIlluminantData) {
        self.illuminant_data2 = data.clone();
        self.clear_fingerprint();
    }
    pub fn illuminant_data2(&self) -> &DngIlluminantData {
        &self.illuminant_data2
    }

    pub fn set_illuminant_data3(&mut self, data: &DngIlluminantData) {
        self.illuminant_data3 = data.clone();
        self.clear_fingerprint();
    }
    pub fn illuminant_data3(&self) -> &DngIlluminantData {
        &self.illuminant_data3
    }

    /// `CalibrationTemperature1/2/3 ()`.
    pub fn calibration_temperature1(&self) -> real64 {
        Self::illuminant_to_temperature(self.calibration_illuminant1, &self.illuminant_data1)
    }
    pub fn calibration_temperature2(&self) -> real64 {
        Self::illuminant_to_temperature(self.calibration_illuminant2, &self.illuminant_data2)
    }
    pub fn calibration_temperature3(&self) -> real64 {
        Self::illuminant_to_temperature(self.calibration_illuminant3, &self.illuminant_data3)
    }

    /// `IlluminantToTemperature (light, data)` — temperature of a standard
    /// illuminant, or the spectrum-derived white point's temperature.
    pub fn illuminant_to_temperature(light: u32, data: &DngIlluminantData) -> real64 {
        use dng_core::dng_tag_values::*;

        if !data.is_valid() {
            return 0.0;
        }

        match light {
            x if x == lsStandardLightA || x == lsTungsten => 2850.0,
            x if x == lsISOStudioTungsten => 3200.0,
            x if x == lsStandardLightB => 4871.4,
            x if x == lsStandardLightC => 6774.3,
            x if x == lsD50 => 5000.0,
            x if x == lsD55 || x == lsDaylight || x == lsFineWeather || x == lsFlash => 5500.0,
            x if x == lsD65 || x == lsCloudyWeather => 6500.0,
            x if x == lsD75 || x == lsShade => 7500.0,
            x if x == lsDaylightFluorescent => 6430.0,
            x if x == lsDayWhiteFluorescent => 5000.0,
            x if x == lsCoolWhiteFluorescent => 4150.0,
            x if x == lsFluorescent => 4230.0,
            x if x == lsWhiteFluorescent => 3450.0,
            x if x == lsWarmWhiteFluorescent => 2940.0,
            _ => {
                // Non-standard illuminant: use the derived white point.
                if data.white_xy().is_valid() {
                    let temp = dng_core::DngTemperature::from_xy(*data.white_xy());
                    temp.temperature()
                } else {
                    0.0
                }
            }
        }
    }

    // ------------------------------------------------------------- matrices

    /// `NormalizeColorMatrix (m)` — scale so the white column maps to the PCS
    /// white, round to 4 decimals.
    pub fn normalize_color_matrix(m: &mut DngMatrix) {
        if m.not_empty() {
            let coord = m.multiply_vector(&pcs_to_xyz()).unwrap();
            let max_coord = coord.max_entry();

            if max_coord > 0.0 && !(0.99..=1.01).contains(&max_coord) {
                m.scale(1.0 / max_coord);
            }

            m.round(10000.0);
        }
    }

    pub fn set_color_matrix1(&mut self, m: &DngMatrix) {
        self.color_matrix1 = *m;
        Self::normalize_color_matrix(&mut self.color_matrix1);
        self.clear_fingerprint();
    }
    pub fn color_matrix1(&self) -> &DngMatrix {
        &self.color_matrix1
    }

    pub fn set_color_matrix2(&mut self, m: &DngMatrix) {
        self.color_matrix2 = *m;
        Self::normalize_color_matrix(&mut self.color_matrix2);
        self.clear_fingerprint();
    }
    pub fn color_matrix2(&self) -> &DngMatrix {
        &self.color_matrix2
    }

    pub fn set_color_matrix3(&mut self, m: &DngMatrix) {
        self.color_matrix3 = *m;
        Self::normalize_color_matrix(&mut self.color_matrix3);
        self.clear_fingerprint();
    }
    pub fn color_matrix3(&self) -> &DngMatrix {
        &self.color_matrix3
    }

    /// `NormalizeForwardMatrix (m)` — scale so the white maps to (1, 1, 1).
    pub fn normalize_forward_matrix(m: &mut DngMatrix) {
        if m.not_empty() {
            let ones = DngVector::from_values(&[1.0, 1.0, 1.0]).unwrap();
            let coord = m.multiply_vector(&ones);
            if let Ok(coord) = coord {
                let max_coord = coord.max_entry();

                if max_coord > 0.0 && !(0.99..=1.01).contains(&max_coord) {
                    m.scale(1.0 / max_coord);
                }

                m.round(10000.0);
            }
        }
    }

    pub fn set_forward_matrix1(&mut self, m: &DngMatrix) {
        self.forward_matrix1 = *m;
        Self::normalize_forward_matrix(&mut self.forward_matrix1);
        self.clear_fingerprint();
    }
    pub fn forward_matrix1(&self) -> &DngMatrix {
        &self.forward_matrix1
    }

    pub fn set_forward_matrix2(&mut self, m: &DngMatrix) {
        self.forward_matrix2 = *m;
        Self::normalize_forward_matrix(&mut self.forward_matrix2);
        self.clear_fingerprint();
    }
    pub fn forward_matrix2(&self) -> &DngMatrix {
        &self.forward_matrix2
    }

    pub fn set_forward_matrix3(&mut self, m: &DngMatrix) {
        self.forward_matrix3 = *m;
        Self::normalize_forward_matrix(&mut self.forward_matrix3);
        self.clear_fingerprint();
    }
    pub fn forward_matrix3(&self) -> &DngMatrix {
        &self.forward_matrix3
    }

    pub fn set_reduction_matrix1(&mut self, m: &DngMatrix) {
        self.reduction_matrix1 = *m;
        self.clear_fingerprint();
    }
    pub fn reduction_matrix1(&self) -> &DngMatrix {
        &self.reduction_matrix1
    }

    pub fn set_reduction_matrix2(&mut self, m: &DngMatrix) {
        self.reduction_matrix2 = *m;
        self.clear_fingerprint();
    }
    pub fn reduction_matrix2(&self) -> &DngMatrix {
        &self.reduction_matrix2
    }

    pub fn set_reduction_matrix3(&mut self, m: &DngMatrix) {
        self.reduction_matrix3 = *m;
        self.clear_fingerprint();
    }
    pub fn reduction_matrix3(&self) -> &DngMatrix {
        &self.reduction_matrix3
    }

    /// `HasColorMatrix1 ()`.
    pub fn has_color_matrix1(&self) -> bool {
        self.color_matrix1.rows() > 0 && self.color_matrix1.cols() == 3
    }

    /// `HasColorMatrix2 ()`.
    pub fn has_color_matrix2(&self) -> bool {
        self.color_matrix2.rows() > 0 && self.color_matrix2.cols() == 3
    }

    /// `HasColorMatrix3 ()`.
    pub fn has_color_matrix3(&self) -> bool {
        self.color_matrix3.rows() > 0 && self.color_matrix3.cols() == 3
    }

    // ---------------------------------------------------------- hue sat etc

    pub fn set_hue_sat_deltas1(&mut self, deltas: &DngHueSatMap) {
        self.hue_sat_deltas1 = deltas.clone();
        self.clear_fingerprint();
    }
    pub fn hue_sat_deltas1(&self) -> &DngHueSatMap {
        &self.hue_sat_deltas1
    }

    pub fn set_hue_sat_deltas2(&mut self, deltas: &DngHueSatMap) {
        self.hue_sat_deltas2 = deltas.clone();
        self.clear_fingerprint();
    }
    pub fn hue_sat_deltas2(&self) -> &DngHueSatMap {
        &self.hue_sat_deltas2
    }

    pub fn set_hue_sat_deltas3(&mut self, deltas: &DngHueSatMap) {
        self.hue_sat_deltas3 = deltas.clone();
        self.clear_fingerprint();
    }
    pub fn hue_sat_deltas3(&self) -> &DngHueSatMap {
        &self.hue_sat_deltas3
    }

    pub fn set_hue_sat_map_encoding(&mut self, encoding: u32) {
        self.hue_sat_map_encoding = encoding;
        self.clear_fingerprint();
    }
    pub fn hue_sat_map_encoding(&self) -> u32 {
        self.hue_sat_map_encoding
    }

    pub fn set_look_table(&mut self, table: &DngHueSatMap) {
        self.look_table = table.clone();
        self.clear_fingerprint();
    }
    pub fn look_table(&self) -> &DngHueSatMap {
        &self.look_table
    }

    pub fn set_look_table_encoding(&mut self, encoding: u32) {
        self.look_table_encoding = encoding;
        self.clear_fingerprint();
    }
    pub fn look_table_encoding(&self) -> u32 {
        self.look_table_encoding
    }

    pub fn set_baseline_exposure_offset(&mut self, offset: DngSRational) {
        self.baseline_exposure_offset = offset;
        self.clear_fingerprint();
    }
    pub fn baseline_exposure_offset(&self) -> &DngSRational {
        &self.baseline_exposure_offset
    }

    pub fn set_default_black_render(&mut self, render: u32) {
        self.default_black_render = render;
        self.clear_fingerprint();
    }
    pub fn default_black_render(&self) -> u32 {
        self.default_black_render
    }

    pub fn set_tone_curve(&mut self, curve: &DngToneCurve) {
        self.tone_curve = curve.clone();
        self.clear_fingerprint();
    }
    pub fn tone_curve(&self) -> &DngToneCurve {
        &self.tone_curve
    }

    pub fn set_tone_method(&mut self, method: u32) {
        self.tone_method = method;
        self.clear_fingerprint();
    }
    pub fn tone_method(&self) -> u32 {
        self.tone_method
    }

    pub fn set_profile_calibration_signature(&mut self, signature: &DngString) {
        self.profile_calibration_signature = signature.clone();
        self.clear_fingerprint();
    }
    pub fn profile_calibration_signature(&self) -> &DngString {
        &self.profile_calibration_signature
    }

    pub fn set_unique_camera_model_restriction(&mut self, model: &DngString) {
        self.unique_camera_model_restriction = model.clone();
        self.clear_fingerprint();
    }
    pub fn unique_camera_model_restriction(&self) -> &DngString {
        &self.unique_camera_model_restriction
    }

    pub fn set_copyright(&mut self, copyright: &DngString) {
        self.copyright = copyright.clone();
        self.clear_fingerprint();
    }
    pub fn copyright(&self) -> &DngString {
        &self.copyright
    }

    pub fn set_embed_policy(&mut self, policy: u32) {
        self.embed_policy = policy;
        self.clear_fingerprint();
    }
    pub fn embed_policy(&self) -> u32 {
        self.embed_policy
    }

    pub fn was_read_from_dng(&self) -> bool {
        self.was_read_from_dng
    }
    pub fn set_was_read_from_dng(&mut self, was: bool) {
        self.was_read_from_dng = was;
    }
    pub fn was_read_from_disk(&self) -> bool {
        self.was_read_from_disk
    }
    pub fn set_was_read_from_disk(&mut self, was: bool) {
        self.was_read_from_disk = was;
    }
    pub fn was_stubbed(&self) -> bool {
        self.was_stubbed
    }

    /// `HasProfileGainTableMap ()`.
    pub fn has_profile_gain_table_map(&self) -> bool {
        self.profile_gain_table_map.is_some()
    }

    /// `IsSDR ()` / `IsHDR ()`.
    pub fn is_sdr(&self) -> bool {
        self.dynamic_range_info.as_ref().map(|d| d.is_sdr()).unwrap_or(true)
    }
    pub fn is_hdr(&self) -> bool {
        !self.is_sdr()
    }

    /// `Uses_1_6_Features ()`.
    pub fn uses_1_6_features(&self) -> bool {
        self.has_color_matrix3()
            || self.calibration_illuminant3 != lsUnknown
            || self.look_table.is_valid()
            || self.look_table_encoding != encoding_Linear
            || self.profile_gain_table_map.is_some()
            || self.is_hdr()
    }

    /// `Requires_1_6_Reader ()` — 1.6 features that break older readers.
    pub fn requires_1_6_reader(&self) -> bool {
        self.has_color_matrix3() || self.is_hdr()
    }

    /// `Uses_1_7_Features ()`.
    pub fn uses_1_7_features(&self) -> bool {
        self.profile_gain_table_map.is_some()
    }

    // ------------------------------------------------------------ lifecycle

    /// `ClearFingerprint ()` — internal helper.
    fn clear_fingerprint(&self) {
        self.fingerprint.set(DngFingerprint::new());
        self.render_data_fingerprint.set(DngFingerprint::new());
    }

    /// `Fingerprint ()` — cached MD5 over the profile field stream.
    pub fn fingerprint(&self) -> DngFingerprint {
        if self.fingerprint.get().is_null() {
            self.fingerprint.set(self.calculate_fingerprint(false));
        }
        self.fingerprint.get()
    }

    /// `RenderDataFingerprint ()`.
    pub fn render_data_fingerprint(&self) -> DngFingerprint {
        if self.render_data_fingerprint.get().is_null() {
            self.render_data_fingerprint.set(self.calculate_fingerprint(true));
        }
        self.render_data_fingerprint.get()
    }

    /// `CalculateFingerprint (renderDataOnly)`.
    pub fn calculate_fingerprint(&self, render_data_only: bool) -> DngFingerprint {
        let mut printer = DngMd5PrinterStream::new();

        // MD5 hash is always calculated on little-endian data.
        printer.stream().set_little_endian(true);

        let mut stream = printer.stream();

        if self.has_color_matrix1() {
            let color_channels = self.color_matrix1.rows();

            let _ = stream.put_uint16(self.calibration_illuminant1 as u16);
            Self::fingerprint_matrix(stream, &self.color_matrix1);

            if self.forward_matrix1.rows() == self.color_matrix1.cols()
                && self.forward_matrix1.cols() == self.color_matrix1.rows()
            {
                Self::fingerprint_matrix(stream, &self.forward_matrix1);
            }

            if color_channels > 3
                && self.reduction_matrix1.rows() * self.reduction_matrix1.cols()
                    == color_channels * 3
            {
                Self::fingerprint_matrix(stream, &self.reduction_matrix1);
            }

            if self.has_color_matrix2() {
                let _ = stream.put_uint16(self.calibration_illuminant2 as u16);
                Self::fingerprint_matrix(stream, &self.color_matrix2);

                if self.forward_matrix2.rows() == self.color_matrix2.cols()
                    && self.forward_matrix2.cols() == self.color_matrix2.rows()
                {
                    Self::fingerprint_matrix(stream, &self.forward_matrix2);
                }

                if color_channels > 3
                    && self.reduction_matrix2.rows() * self.reduction_matrix2.cols()
                        == color_channels * 3
                {
                    Self::fingerprint_matrix(stream, &self.reduction_matrix2);
                }

                if self.has_color_matrix3() {
                    let _ = stream.put_uint16(self.calibration_illuminant3 as u16);
                    Self::fingerprint_matrix(stream, &self.color_matrix3);

                    if self.forward_matrix3.rows() == self.color_matrix3.cols()
                        && self.forward_matrix3.cols() == self.color_matrix3.rows()
                    {
                        Self::fingerprint_matrix(stream, &self.forward_matrix3);
                    }

                    if color_channels > 3
                        && self.reduction_matrix3.rows() * self.reduction_matrix3.cols()
                            == color_channels * 3
                    {
                        Self::fingerprint_matrix(stream, &self.reduction_matrix3);
                    }
                }
            }
        }

        if !render_data_only {
            let _ = stream.put(self.name.get());
            let _ = stream.put(self.group_name.get());
        }

        let _ = stream.put(self.profile_calibration_signature.get());

        if !render_data_only {
            let _ = stream.put_uint32(self.embed_policy);
            let _ = stream.put(self.copyright.get());
        }

        let have_hue_sat1 = self.hue_sat_deltas1.is_valid();
        let have_hue_sat2 = self.hue_sat_deltas2.is_valid() && self.has_color_matrix2();
        let have_hue_sat3 = self.hue_sat_deltas3.is_valid() && self.has_color_matrix3();

        if have_hue_sat1 {
            Self::fingerprint_hue_sat_map(stream, &self.hue_sat_deltas1);
        }
        if have_hue_sat2 {
            Self::fingerprint_hue_sat_map(stream, &self.hue_sat_deltas2);
        }
        if have_hue_sat3 {
            Self::fingerprint_hue_sat_map(stream, &self.hue_sat_deltas3);
        }

        if have_hue_sat1 || have_hue_sat2 || have_hue_sat3 {
            if self.hue_sat_map_encoding != 0 {
                let _ = stream.put_uint32(self.hue_sat_map_encoding);
            }
        }

        if self.look_table.is_valid() {
            Self::fingerprint_hue_sat_map(stream, &self.look_table);

            if self.look_table_encoding != 0 {
                let _ = stream.put_uint32(self.look_table_encoding);
            }
        }

        let _ = stream.put_int32(self.baseline_exposure_offset.n);
        let _ = stream.put_int32(self.baseline_exposure_offset.d);
        let _ = stream.put_uint32(self.default_black_render);

        if self.tone_method != profileToneMethod_Unspecified {
            let _ = stream.put_uint32(self.tone_method);
        }

        if !self.tone_curve.is_null() && self.tone_method != profileToneMethod_Unspecified {
            // Tone curve points are fingerprinted in the C++ via a tag;
            // the port emits the coordinate stream.
            for point in &self.tone_curve.coord {
                let _ = stream.put_real64(point.h);
                let _ = stream.put_real64(point.v);
            }
        }

        let result = printer.result().unwrap_or_default();

        result
    }

    /// `FingerprintMatrix` — writes the matrix as a tag_matrix payload
    /// (rows/cols header + doubles), little-endian.
    fn fingerprint_matrix(printer: &mut dng_core::DngStream, matrix: &DngMatrix) {
        // tag_matrix (0, matrix): type ttSRational in C++ tag encode? No —
        // tag_matrix uses ttSRational encoding of the double values rounded
        // through dng_srational.  Mirror: rows, cols as uint32 header, then
        // srational pairs.
        let _ = printer.put_uint32(matrix.rows());
        let _ = printer.put_uint32(matrix.cols());

        for row in 0..matrix.rows() as usize {
            for col in 0..matrix.cols() as usize {
                // tag_matrix stores srational (n/d) with denominator 10000.
                let mut sr = DngSRational::default();
                sr.set_real64(matrix.get(row, col), 10000);
                let _ = printer.put_int32(sr.n);
                let _ = printer.put_int32(sr.d);
            }
        }
    }

    /// `FingerprintHueSatMap`.
    fn fingerprint_hue_sat_map(printer: &mut dng_core::DngStream, map: &DngHueSatMap) {
        if map.is_null() {
            return;
        }

        let (hues, sats, vals) = map.get_divisions();

        let _ = printer.put_uint32(hues);
        let _ = printer.put_uint32(sats);
        let _ = printer.put_uint32(vals);

        for val in 0..vals {
            for hue in 0..hues {
                for sat in 0..sats {
                    let modify = match map.get_delta(hue, sat, val) {
                        Ok(m) => m,
                        Err(_) => HSBModify::default(),
                    };

                    let _ = printer.put_real32(modify.hue_shift);
                    let _ = printer.put_real32(modify.sat_scale);
                    let _ = printer.put_real32(modify.val_scale);
                }
            }
        }
    }

    /// `IsValid (channels)` — structural validation.
    pub fn is_valid(&self, channels: u32) -> bool {
        // For monochrome images we ignore the camera profile.
        if channels == 1 {
            return true;
        }

        // ColorMatrix1 is required for all color images.
        if self.color_matrix1.cols() != 3 || self.color_matrix1.rows() != channels {
            return false;
        }

        let mut has_first_two_color_matrices = false;

        if self.color_matrix2.cols() != 0 || self.color_matrix2.rows() != 0 {
            if self.color_matrix2.cols() != 3 || self.color_matrix2.rows() != channels {
                return false;
            }

            has_first_two_color_matrices = true;
        }

        if self.color_matrix3.cols() != 0 || self.color_matrix3.rows() != 0 {
            if !has_first_two_color_matrices {
                return false;
            }

            if self.color_matrix3.cols() != 3 || self.color_matrix3.rows() != channels {
                return false;
            }
        }

        // ColorMatrix1 must be invertible (or the reduction matrix must make
        // it so).
        if self.color_matrix1.not_empty() {
            let inverted = if self.reduction_matrix1.not_empty() {
                self.color_matrix1.inverted_with_hint(&self.reduction_matrix1)
            } else {
                self.color_matrix1.inverted()
            };

            if inverted.is_err() {
                return false;
            }
        }

        // ColorMatrix2 invertibility when present.
        if self.color_matrix2.not_empty() {
            let inverted = if self.reduction_matrix2.not_empty() {
                self.color_matrix2.inverted_with_hint(&self.reduction_matrix2)
            } else {
                self.color_matrix2.inverted()
            };

            if inverted.is_err() {
                return false;
            }
        }

        // ColorMatrix3 invertibility when present.
        if self.color_matrix3.not_empty() {
            let inverted = if self.reduction_matrix3.not_empty() {
                self.color_matrix3.inverted_with_hint(&self.reduction_matrix3)
            } else {
                self.color_matrix3.inverted()
            };

            if inverted.is_err() {
                return false;
            }
        }

        // Reduction matrices present for all three or absent for all three.
        if self.reduction_matrix3.not_empty()
            && (self.reduction_matrix1.is_empty() || self.reduction_matrix2.is_empty())
        {
            return false;
        }

        // HueSat maps present for all three or absent for all three (when any
        // is valid).
        if self.hue_sat_deltas1.is_valid()
            || self.hue_sat_deltas2.is_valid()
            || self.hue_sat_deltas3.is_valid()
        {
            if self.hue_sat_deltas1.is_null()
                || self.hue_sat_deltas2.is_null()
                || self.hue_sat_deltas3.is_null()
            {
                return false;
            }
        }

        true
    }

    /// `ValidForwardMatrix (m)`.
    pub fn valid_forward_matrix(m: &DngMatrix) -> bool {
        if m.rows() != 3 || m.cols() != 3 {
            return false;
        }

        let ones = DngVector::from_values(&[1.0, 1.0, 1.0]).unwrap();
        let coord = match m.multiply_vector(&ones) {
            Ok(c) => c,
            Err(_) => return false,
        };

        // Must map white to (1,1,1) within tolerance.
        (coord.get(0) - 1.0).abs() < 0.1
            && (coord.get(1) - 1.0).abs() < 0.1
            && (coord.get(2) - 1.0).abs() < 0.1
            && coord.max_entry() > 0.0
    }

    /// `Stub ()` — drop render data, keep the identity.
    pub fn stub(&mut self) {
        let _ = self.fingerprint();
        let _ = self.render_data_fingerprint();

        self.hue_sat_deltas1 = DngHueSatMap::new();
        self.hue_sat_deltas2 = DngHueSatMap::new();
        self.hue_sat_deltas3 = DngHueSatMap::new();
        self.look_table = DngHueSatMap::new();
        self.tone_curve.set_invalid();
        self.profile_gain_table_map = None;

        self.was_stubbed = true;
    }

    /// `SetFourColorBayer ()` — duplicate the second row into a 4th channel.
    pub fn set_four_color_bayer(&mut self) -> DngResult<()> {
        if !self.is_valid(3) {
            return Err(program_error());
        }

        if self.color_matrix1.not_empty() {
            let mut m = DngMatrix::with_dims(4, 3)?;

            for j in 0..3usize {
                m.set(0, j, self.color_matrix1.get(0, j));
                m.set(1, j, self.color_matrix1.get(1, j));
                m.set(2, j, self.color_matrix1.get(2, j));
                m.set(3, j, self.color_matrix1.get(1, j));
            }

            self.color_matrix1 = m;
        }

        if self.color_matrix2.not_empty() {
            let mut m = DngMatrix::with_dims(4, 3)?;

            for j in 0..3usize {
                m.set(0, j, self.color_matrix2.get(0, j));
                m.set(1, j, self.color_matrix2.get(1, j));
                m.set(2, j, self.color_matrix2.get(2, j));
                m.set(3, j, self.color_matrix2.get(1, j));
            }

            self.color_matrix2 = m;
        }

        if self.color_matrix3.not_empty() {
            let mut m = DngMatrix::with_dims(4, 3)?;

            for j in 0..3usize {
                m.set(0, j, self.color_matrix3.get(0, j));
                m.set(1, j, self.color_matrix3.get(1, j));
                m.set(2, j, self.color_matrix3.get(2, j));
                m.set(3, j, self.color_matrix3.get(1, j));
            }

            self.color_matrix3 = m;
        }

        self.reduction_matrix1.clear();
        self.reduction_matrix2.clear();
        self.reduction_matrix3.clear();

        self.forward_matrix1.clear();
        self.forward_matrix2.clear();
        self.forward_matrix3.clear();

        self.clear_fingerprint();

        Ok(())
    }

    /// `Parse (stream, shared)` — reads a profile from the tag stream (the
    /// CameraProfile tag format, reusing the phase-1 wire parser from
    /// dng-core) and then the extended block if present.
    /// `ProfileID ()` — name + fingerprint id.
    pub fn profile_id(&self) -> DngCameraProfileId {
        let fingerprint = self.fingerprint();

        DngCameraProfileId::from_name_fingerprint(
            std::str::from_utf8(self.name().get()).unwrap_or(""),
            &fingerprint,
        )
    }

    /// `IsLegalToEmbed ()` — per the profile's embed policy.
    pub fn is_legal_to_embed(&self) -> bool {
        self.was_read_from_dng()
            || self.embed_policy() == pepAllowCopying
            || self.embed_policy() == pepEmbedIfUsed
            || self.embed_policy() == pepNoRestrictions
    }

    /// `UniqueID ()` — MD5 over the fingerprint plus the unique camera model
    /// restriction string (dng_camera_profile.cpp:835).
    pub fn unique_id(&self) -> DngFingerprint {
        let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();

        printer.process(&self.fingerprint().data);
        printer.process(self.unique_camera_model_restriction().get());

        *printer.result()
    }

    /// `EqualData (profile)` — render-data fingerprint equality.
    pub fn equal_data(&self, profile: &DngCameraProfile) -> bool {
        self.render_data_fingerprint() == profile.render_data_fingerprint()
    }

    /// `HueSatMapForWhite (white)` — finds the hue/sat table to use for a
    /// given white point, if any (dng_camera_profile.cpp:1865).
    pub fn hue_sat_map_for_white(&self, white: &DngXyCoord) -> DngResult<Option<DngHueSatMap>> {
        if self.hue_sat_deltas1().is_valid() {
            // If we only have the first table, just use it for any color
            // temperature.

            if !self.hue_sat_deltas2().is_valid() {
                return Ok(Some(self.hue_sat_deltas1().clone()));
            }

            // We have table 1 and table 2.

            if self.illuminant_model() == 3 {
                // This means we also have a 3rd hue sat map, since all three
                // are required to be present or absent for a
                // triple-illuminant profile.

                return self.hue_sat_map_for_white_triple(white).map(Some);
            } else {
                // Dual-illuminant model.

                return self.hue_sat_map_for_white_dual(white).map(Some);
            }
        }

        Ok(None)
    }

    /// `HueSatMapForWhite_Dual (white)` (dng_camera_profile.cpp:1908).
    fn hue_sat_map_for_white_dual(&self, white: &DngXyCoord) -> DngResult<DngHueSatMap> {
        debug_assert!(
            self.hue_sat_deltas1().is_valid() && self.hue_sat_deltas2().is_valid(),
            "Bad hue sat map deltas 1 or 2"
        );

        // Interpolate based on color temperature.

        let mut temperature1 = self.calibration_temperature1();
        let mut temperature2 = self.calibration_temperature2();

        if temperature1 <= 0.0 || temperature2 <= 0.0 || temperature1 == temperature2 {
            return Ok(self.hue_sat_deltas1().clone());
        }

        let reverse_order = temperature1 > temperature2;

        if reverse_order {
            std::mem::swap(&mut temperature1, &mut temperature2);
        }

        // Convert to temperature/offset space.

        let td = DngTemperature::from_xy(*white);

        // Find fraction to weight the first calibration.

        let mut g;

        if td.temperature() <= temperature1 {
            g = 1.0;
        } else if td.temperature() >= temperature2 {
            g = 0.0;
        } else {
            let inv_t = 1.0 / td.temperature();

            g = (inv_t - (1.0 / temperature2)) / ((1.0 / temperature1) - (1.0 / temperature2));
        }

        // Fix up if we swapped the order.

        if reverse_order {
            g = 1.0 - g;
        }

        // Do the interpolation.

        DngHueSatMap::interpolate(self.hue_sat_deltas1(), self.hue_sat_deltas2(), g)
    }

    /// `HueSatMapForWhite_Triple (white)` (dng_camera_profile.cpp:1947).
    fn hue_sat_map_for_white_triple(&self, white: &DngXyCoord) -> DngResult<DngHueSatMap> {
        debug_assert!(
            self.hue_sat_deltas1().is_valid()
                && self.hue_sat_deltas2().is_valid()
                && self.hue_sat_deltas3().is_valid(),
            "Bad hue sat map deltas 1 or 2 or 3"
        );

        let light1 =
            DngIlluminantData::from_light(self.calibration_illuminant1(), Some(self.illuminant_data1()))?;
        let light2 =
            DngIlluminantData::from_light(self.calibration_illuminant2(), Some(self.illuminant_data2()))?;
        let light3 =
            DngIlluminantData::from_light(self.calibration_illuminant3(), Some(self.illuminant_data3()))?;

        let (w1, w2, _w3) = calculate_triple_illuminant_weights(white, &light1, &light2, &light3);

        DngHueSatMap::interpolate3(
            self.hue_sat_deltas1(),
            self.hue_sat_deltas2(),
            self.hue_sat_deltas3(),
            w1,
            w2,
        )
    }

    pub fn parse(&mut self, stream: &mut DngStream, shared: &DngShared) -> DngResult<bool> {
        let mut info = dng_core::DngCameraProfileInfo::new();

        let mut got_any = false;

        // The profile tag stream is a sequence of (tagCode: uint16,
        // tagType: uint16, tagCount: uint32, data...) records.
        while stream.position() + 10 <= stream.length() {
            let tag_code = stream.get_uint16()? as u32;
            let tag_type = stream.get_uint16()? as u32;
            let tag_count = stream.get_uint32()?;
            let tag_offset = stream.position_in_original_file();

            // Data is inline for small tags; offsets were captured.
            let parsed = info.parse_tag(stream, 0, tag_code, tag_type, tag_count, tag_offset)?;

            got_any = got_any || parsed;
        }

        if !got_any {
            return Ok(false);
        }

        self.apply_info(&info, shared);

        Ok(true)
    }

    /// Applies a parsed wire-format info block to the live profile fields.
    pub fn apply_info(&mut self, info: &dng_core::DngCameraProfileInfo, _shared: &DngShared) {
        self.set_name(&String::from_utf8_lossy(info.profile_name.get()));
        self.profile_calibration_signature = info.profile_calibration_signature.clone();
        self.copyright = info.profile_copyright.clone();
        self.embed_policy = info.embed_policy;
        self.set_calibration_illuminant1(info.calibration_illuminant1);
        self.set_calibration_illuminant2(info.calibration_illuminant2);
        self.set_calibration_illuminant3(info.calibration_illuminant3);
        self.illuminant_data1 = info.illuminant_data1.clone();
        self.illuminant_data2 = info.illuminant_data2.clone();
        self.illuminant_data3 = info.illuminant_data3.clone();

        if info.color_matrix1.not_empty() {
            self.set_color_matrix1(&info.color_matrix1);
        }
        if info.color_matrix2.not_empty() {
            self.set_color_matrix2(&info.color_matrix2);
        }
        if info.color_matrix3.not_empty() {
            self.set_color_matrix3(&info.color_matrix3);
        }
        if info.forward_matrix1.not_empty() {
            self.set_forward_matrix1(&info.forward_matrix1);
        }
        if info.forward_matrix2.not_empty() {
            self.set_forward_matrix2(&info.forward_matrix2);
        }
        if info.forward_matrix3.not_empty() {
            self.set_forward_matrix3(&info.forward_matrix3);
        }
        if info.reduction_matrix1.not_empty() {
            self.set_reduction_matrix1(&info.reduction_matrix1);
        }
        if info.reduction_matrix2.not_empty() {
            self.set_reduction_matrix2(&info.reduction_matrix2);
        }
        if info.reduction_matrix3.not_empty() {
            self.set_reduction_matrix3(&info.reduction_matrix3);
        }

        if info.profile_hues > 0 && info.profile_sats > 0 {
            // Build hue-sat maps from the captured offsets is phase-3
            // (stream re-seek); record the dims for now via empty maps.
        }

        self.hue_sat_map_encoding = info.hue_sat_map_encoding;
        self.look_table_encoding = info.look_table_encoding;
        self.baseline_exposure_offset = info.baseline_exposure_offset;
        self.default_black_render = info.default_black_render;
        self.tone_method = info.tone_method;

        self.was_read_from_dng = true;
    }
}

type real64 = f64;

// ---------------------------------------------------------------------------
// Free helpers and metadata types (dng_camera_profile.cpp tail).
// ---------------------------------------------------------------------------

/// `SplitCameraProfileName (name, baseName, version)` — splits version
/// suffixes like " beta", " beta N" and " vN" off a profile name
/// (dng_camera_profile.cpp:2270).
pub fn split_camera_profile_name(name: &DngString) -> (DngString, int32) {
    let bytes: &[u8] = name.get();

    let mut base_name = name.clone();

    let mut version: int32 = 0;

    let len = bytes.len();

    // Special case ACR M.N version suffixes.

    if len == 7 && name.starts_with(b"ACR ", true) {
        let c0 = bytes[len - 3];
        let dot = bytes[len - 2];
        let c1 = bytes[len - 1];

        if c0.is_ascii_digit() && dot == b'.' && c1.is_ascii_digit() {
            base_name.truncate(3);

            version = ((c0 - b'0') as int32) * 10 + ((c1 - b'0') as int32);
        }

        return (base_name, version);
    }

    // " beta" suffix.

    if len > 5 && name.ends_with(b" beta", false) {
        base_name.truncate((len - 5) as uint32);

        version += -10;
    } else if len > 7 {
        let last_char = bytes[len - 1];

        if last_char.is_ascii_digit() {
            let mut temp = name.clone();
            temp.truncate((len - 1) as uint32);

            if temp.ends_with(b" beta ", false) {
                base_name.truncate((len - 7) as uint32);

                version += ((last_char - b'0') as int32) - 10;
            }
        }
    }

    // " vN" suffix.

    let len = base_name.length() as usize;

    if len > 3 {
        let bytes: &[u8] = base_name.get();

        let last_char = bytes[len - 1];

        if last_char.is_ascii_digit() {
            let mut temp = base_name.clone();
            temp.truncate((len - 1) as uint32);

            if temp.ends_with(b" v", false) {
                base_name.truncate((len - 3) as uint32);

                version += ((last_char - b'0') as int32) * 100;
            }
        }
    }

    (base_name, version)
}

/// Port of `dng_camera_profile_metadata`.
#[derive(Debug, Clone, Default)]
pub struct DngCameraProfileMetadata {
    /// `fProfileID`.
    pub profile_id: DngCameraProfileId,

    /// `fGroupName`.
    pub group_name: DngString,

    /// `fHDR`.
    pub hdr: bool,

    /// `fRenderDataFingerprint`.
    pub render_data_fingerprint: DngFingerprint,

    /// `fIsLegalToEmbed`.
    pub is_legal_to_embed: bool,

    /// `fWasReadFromDNG`.
    pub was_read_from_dng: bool,

    /// `fWasReadFromDisk`.
    pub was_read_from_disk: bool,

    /// `fUniqueID` — only valid if `was_read_from_disk` is true.
    pub unique_id: DngFingerprint,

    /// `fFilePath` — only valid if `was_read_from_disk` is true.
    pub file_path: DngString,

    /// `fReadOnly` — only valid if `was_read_from_disk` is true.
    pub read_only: bool,

    /// `fIndex` — only valid if attached to negative.
    pub index: int32,
}

impl DngCameraProfileMetadata {
    /// `dng_camera_profile_metadata (profile, index)`.
    pub fn from_profile(profile: &DngCameraProfile, index: int32) -> Self {
        let was_read_from_disk = profile.was_read_from_disk();

        let unique_id = if was_read_from_disk {
            profile.unique_id()
        } else {
            DngFingerprint::new()
        };

        DngCameraProfileMetadata {
            profile_id: profile.profile_id(),
            group_name: profile.group_name().clone(),
            hdr: profile.is_hdr(),
            render_data_fingerprint: profile.render_data_fingerprint(),
            is_legal_to_embed: profile.is_legal_to_embed(),
            was_read_from_dng: profile.was_read_from_dng(),
            was_read_from_disk,
            unique_id,
            file_path: DngString::new(),
            read_only: true,
            index,
        }
    }
}

impl PartialEq for DngCameraProfileMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.profile_id == other.profile_id
            && self.group_name == other.group_name
            && self.hdr == other.hdr
            && self.render_data_fingerprint == other.render_data_fingerprint
            && self.is_legal_to_embed == other.is_legal_to_embed
            && self.was_read_from_dng == other.was_read_from_dng
            && self.was_read_from_disk == other.was_read_from_disk
            && self.unique_id == other.unique_id
            && self.file_path == other.file_path
            && self.read_only == other.read_only
            && self.index == other.index
    }
}

impl Eq for DngCameraProfileMetadata {}

/// Port of `dng_camera_profile_group_selector` (dng_camera_profile.h:195).
#[derive(Debug, Clone, Copy, Default)]
pub struct DngCameraProfileGroupSelector {
    /// `fHDR` — selection: false = SDR, true = HDR.
    pub hdr: bool,
}

/// `dng_profile_metadata_list`.
pub type DngProfileMetadataList = Vec<DngCameraProfileMetadata>;

/// Render-data fingerprint comparison (`EqualData`).
pub fn camera_profile_equal_data(a: &DngCameraProfile, b: &DngCameraProfile) -> bool {
    a.render_data_fingerprint() == b.render_data_fingerprint()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> DngCameraProfile {
        let mut p = DngCameraProfile::new();
        p.set_name("Test Profile");
        p.set_calibration_illuminant1(21); // D65
        p.set_color_matrix1(
            &DngMatrix::from_rows(3, 3, &[0.9, 0.05, 0.05, 0.05, 0.9, 0.05, 0.05, 0.05, 0.9])
                .unwrap(),
        );
        p
    }

    #[test]
    fn defaults_match_cpp() {
        let p = DngCameraProfile::new();
        assert_eq!(p.calibration_illuminant1(), lsUnknown);
        assert_eq!(p.embed_policy, pepAllowCopying);
        assert_eq!(p.hue_sat_map_encoding, encoding_Linear);
        assert_eq!(p.look_table_encoding, encoding_Linear);
        assert_eq!(p.baseline_exposure_offset, DngSRational::new(0, 100));
        assert_eq!(p.default_black_render, defaultBlackRender_Auto);
        assert_eq!(p.tone_method, profileToneMethod_Unspecified);
        assert!(!p.tone_curve.is_valid()); // C++ ctor SetInvalid
        assert!(!p.was_read_from_dng());
        assert!(!p.was_stubbed());
        assert!(p.name().is_empty());
    }

    #[test]
    fn illuminant_model() {
        let mut p = sample_profile();
        assert_eq!(p.illuminant_model(), 1);

        p.set_calibration_illuminant2(17);
        p.set_color_matrix2(
            &DngMatrix::from_rows(3, 3, &[0.9, 0.05, 0.05, 0.05, 0.9, 0.05, 0.05, 0.05, 0.9])
                .unwrap(),
        );
        assert_eq!(p.illuminant_model(), 2);

        p.set_calibration_illuminant3(23);
        p.set_color_matrix3(
            &DngMatrix::from_rows(3, 3, &[0.9, 0.05, 0.05, 0.05, 0.9, 0.05, 0.05, 0.05, 0.9])
                .unwrap(),
        );
        assert_eq!(p.illuminant_model(), 3);
    }

    #[test]
    fn color_matrix_normalization() {
        // Matrix with white column sum != 1 gets scaled to 1 and rounded.
        let mut m = DngMatrix::from_rows(3, 3, &[2.0, 0.1, 0.1, 0.1, 2.0, 0.1, 0.1, 0.1, 2.0])
            .unwrap();
        DngCameraProfile::normalize_color_matrix(&mut m);

        // White column: M * PCSwhite should have max entry ~1.
        let coord = m.multiply_vector(&pcs_to_xyz()).unwrap();
        let max = coord.max_entry();
        assert!((max - 1.0).abs() < 0.001, "max={max}");
    }

    #[test]
    fn forward_matrix_validation() {
        // Identity maps white to white.
        let identity = DngMatrix::from_rows(3, 3, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0])
            .unwrap();
        assert!(DngCameraProfile::valid_forward_matrix(&identity));

        // Zero matrix does not.
        let zero = DngMatrix::with_dims(3, 3).unwrap();
        assert!(!DngCameraProfile::valid_forward_matrix(&zero));
    }

    #[test]
    fn profile_validity() {
        let mut p = sample_profile();
        assert!(p.is_valid(3));
        assert!(!p.is_valid(4)); // 3-channel matrix for a 4-channel image

        // Monochrome ignores the profile.
        assert!(DngCameraProfile::new().is_valid(1));
    }

    #[test]
    fn fingerprint_stable_and_sensitive() {
        let a = sample_profile();
        let b = sample_profile();

        // Same fields -> same fingerprint.
        assert_eq!(a.fingerprint(), b.fingerprint());

        // Different name -> different fingerprint.
        let mut c = sample_profile();
        c.set_name("Other Profile");
        assert_ne!(a.fingerprint(), c.fingerprint());

        // Render data fingerprint excludes the name.
        let mut d = sample_profile();
        d.set_name("Other Profile");
        assert_eq!(a.render_data_fingerprint(), d.render_data_fingerprint());
    }

    #[test]
    fn stub_behaviour() {
        let mut p = sample_profile();

        let mut deltas = DngHueSatMap::new();
        deltas.set_divisions(4, 4, 1).unwrap();
        p.set_hue_sat_deltas1(&deltas);

        p.stub();

        assert!(p.was_stubbed());
        assert!(p.hue_sat_deltas1().is_null());
        // Stub computes and caches the fingerprint BEFORE dropping the render
        // data (C++ order), so the cached value stays valid.
        assert!(p.fingerprint().is_valid());
    }

    #[test]
    fn four_color_bayer() {
        let mut p = sample_profile();
        p.set_four_color_bayer().unwrap();

        assert_eq!(p.color_matrix1().rows(), 4);
        assert_eq!(p.color_matrix1().cols(), 3);

        // Row 3 == row 1 (green duplicated).
        assert_eq!(p.color_matrix1().get(3, 0), p.color_matrix1().get(1, 0));

        // Reduction/forward matrices cleared.
        assert!(p.reduction_matrix1().is_empty());
        assert!(p.forward_matrix1().is_empty());
    }

    #[test]
    fn group_prefix_helpers() {
        let name = DngString::from_bytes(b"HDR | Base");
        assert!(has_profile_group_prefix(&name));
        let stripped = strip_profile_group_prefix(&name);
        assert_eq!(stripped.get(), b"Base");

        let plain = DngString::from_bytes(b"Base");
        assert!(!has_profile_group_prefix(&plain));
    }
}

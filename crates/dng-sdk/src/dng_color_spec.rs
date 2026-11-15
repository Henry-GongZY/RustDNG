// Port of dng_color_spec.h/.cpp — the camera-to-PCS color transform
// pipeline for single/dual/triple-illuminant profiles.
//
// PORT-NOTE(dng_color_spec.cpp:56): the C++ ctor reads camera calibrations
// and analog balance from a `dng_negative`.  DngNegative is assembled later
// in this phase, so the port takes those inputs via a `ColorSpecInputs`
// bundle (same fields, same defaults: identity calibrations, unit analog
// balance) and the dng_negative glue constructs it.  All downstream math is
// a faithful port.

use crate::dng_camera_profile::DngCameraProfile;
use crate::dng_core::dng_temperature::DngTemperature;
use crate::dng_core::{
    abs_real64, bad_format, calculate_triple_illuminant_weights, d50_xy_coord, invert,
    invert_with_hint, max_real64, pcs_to_xy, pcs_to_xyz, pin_real64, program_error, xy_to_xyz,
    DngError, DngIlluminantData, DngMatrix, DngMatrix3by3, DngResult, DngSRational, DngVector,
    DngXyCoord,
};

/// `MapWhiteMatrix (white1, white2)` — linearized Bradford adaptation.
pub fn map_white_matrix(white1: &DngXyCoord, white2: &DngXyCoord) -> DngResult<DngMatrix3by3> {
    // Linearized Bradford adaptation matrix.
    let mb = DngMatrix3by3::new(
        0.8951, 0.2664, -0.1614, -0.7502, 1.7135, 0.0367, 0.0389, -0.0685, 1.0296,
    );

    let mut w1 = mb.0.multiply_vector(&xy_to_xyz(white1))?;
    let mut w2 = mb.0.multiply_vector(&xy_to_xyz(white2))?;

    // Negative white coordinates are kind of meaningless.
    for j in 0..3usize {
        w1.set(j, max_real64(w1.get(j), 0.0));
        w2.set(j, max_real64(w2.get(j), 0.0));
    }

    // Limit scaling to something reasonable.
    let mut a = DngMatrix3by3::new(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);

    for j in 0..3usize {
        let scale = if w1.get(j) > 0.0 {
            w2.get(j) / w1.get(j)
        } else {
            10.0
        };
        a.set(j, j, pin_real64(0.1, scale, 10.0));
    }

    let mb_inv = mb.0.inverted()?;

    Ok(DngMatrix3by3(
        mb_inv.multiply(&a.0)?.multiply(&mb.0)?,
    ))
}

/// The negative-derived inputs the C++ ctor pulls from `dng_negative`.
#[derive(Debug, Clone, Default)]
pub struct ColorSpecInputs {
    /// Per-channel analog balance (defaults to 1.0).
    pub analog_balance: Vec<f64>,
    /// Camera calibrations 1..3 (defaults to identity).
    pub camera_calibration1: Option<DngMatrix>,
    pub camera_calibration2: Option<DngMatrix>,
    pub camera_calibration3: Option<DngMatrix>,
    /// Whether the negative's calibration signature matches the profile's.
    pub calibration_signature_matches: bool,
}

/// Port of `dng_color_spec`.
#[derive(Debug, Clone)]
pub struct DngColorSpec {
    channels: u32,
    temperature1: f64,
    temperature2: f64,

    light1: crate::dng_core::DngIlluminantData,
    light2: crate::dng_core::DngIlluminantData,
    light3: crate::dng_core::DngIlluminantData,

    color_matrix1: DngMatrix,
    color_matrix2: DngMatrix,
    color_matrix3: DngMatrix,

    forward_matrix1: DngMatrix,
    forward_matrix2: DngMatrix,
    forward_matrix3: DngMatrix,

    reduction_matrix1: DngMatrix,
    reduction_matrix2: DngMatrix,
    reduction_matrix3: DngMatrix,

    camera_calibration1: DngMatrix,
    camera_calibration2: DngMatrix,
    camera_calibration3: DngMatrix,

    analog_balance: DngMatrix,

    white_xy: DngXyCoord,
    camera_white: DngVector,
    camera_to_pcs: DngMatrix,
    pcs_to_camera: DngMatrix,
    num_illuminants: u32,
}

impl DngColorSpec {
    /// `dng_color_spec (negative, profile, allowStubbed)` — the negative
    /// inputs are passed as a bundle (see module PORT-NOTE).
    pub fn new(
        channels: u32,
        profile: Option<&DngCameraProfile>,
        allow_stubbed: bool,
        inputs: &ColorSpecInputs,
    ) -> DngResult<Self> {
        let mut spec = DngColorSpec {
            channels,
            temperature1: 0.0,
            temperature2: 0.0,
            light1: crate::dng_core::DngIlluminantData::default(),
            light2: crate::dng_core::DngIlluminantData::default(),
            light3: crate::dng_core::DngIlluminantData::default(),
            color_matrix1: DngMatrix::new(),
            color_matrix2: DngMatrix::new(),
            color_matrix3: DngMatrix::new(),
            forward_matrix1: DngMatrix::new(),
            forward_matrix2: DngMatrix::new(),
            forward_matrix3: DngMatrix::new(),
            reduction_matrix1: DngMatrix::new(),
            reduction_matrix2: DngMatrix::new(),
            reduction_matrix3: DngMatrix::new(),
            camera_calibration1: DngMatrix::new(),
            camera_calibration2: DngMatrix::new(),
            camera_calibration3: DngMatrix::new(),
            analog_balance: DngMatrix::new(),
            white_xy: DngXyCoord::default(),
            camera_white: DngVector::new(),
            camera_to_pcs: DngMatrix::new(),
            pcs_to_camera: DngMatrix::new(),
            num_illuminants: 1,
        };

        if channels > 1 {
            let profile = match profile {
                Some(p) if p.is_valid(channels) => p,
                _ => return Err(bad_format()),
            };

            if profile.was_stubbed() && !allow_stubbed {
                return Err(DngError::Unsupported("Using stubbed profile".to_string()));
            }

            spec.temperature1 = profile.calibration_temperature1();
            spec.temperature2 = profile.calibration_temperature2();

            spec.light1 = profile.illuminant_data1().clone();
            spec.light2 = profile.illuminant_data2().clone();
            spec.light3 = profile.illuminant_data3().clone();

            spec.color_matrix1 = *profile.color_matrix1();
            spec.color_matrix2 = *profile.color_matrix2();
            spec.color_matrix3 = *profile.color_matrix3();

            spec.forward_matrix1 = *profile.forward_matrix1();
            spec.forward_matrix2 = *profile.forward_matrix2();
            spec.forward_matrix3 = *profile.forward_matrix3();

            spec.reduction_matrix1 = *profile.reduction_matrix1();
            spec.reduction_matrix2 = *profile.reduction_matrix2();
            spec.reduction_matrix3 = *profile.reduction_matrix3();

            spec.camera_calibration1.set_identity(channels)?;
            spec.camera_calibration2.set_identity(channels)?;
            spec.camera_calibration3.set_identity(channels)?;

            if inputs.calibration_signature_matches {
                if let Some(m) = &inputs.camera_calibration1 {
                    if m.rows() == channels && m.cols() == channels {
                        spec.camera_calibration1 = *m;
                    }
                }
                if let Some(m) = &inputs.camera_calibration2 {
                    if m.rows() == channels && m.cols() == channels {
                        spec.camera_calibration2 = *m;
                    }
                }
                if let Some(m) = &inputs.camera_calibration3 {
                    if m.rows() == channels && m.cols() == channels {
                        spec.camera_calibration3 = *m;
                    }
                }
            }

            spec.analog_balance = DngMatrix::with_dims(channels, channels)?;

            for j in 0..channels as usize {
                let balance = inputs
                    .analog_balance
                    .get(j)
                    .copied()
                    .unwrap_or(1.0);
                spec.analog_balance.set(j, j, balance);
            }

            // NormalizeForwardMatrix(fForwardMatrix1) — the C++ call here
            // uses the profile's static helper.
            Self::normalize_forward_matrix(&mut spec.forward_matrix1);

            // Compose the working color matrices.
            spec.color_matrix1 = spec
                .analog_balance
                .multiply(&spec.camera_calibration1)?
                .multiply(&spec.color_matrix1)?;

            if !profile.has_color_matrix2()
                || spec.temperature1 <= 0.0
                || spec.temperature2 <= 0.0
                || spec.temperature1 == spec.temperature2
            {
                // Single-illuminant profile: use only the 1st set.
                spec.temperature1 = 5000.0;
                spec.temperature2 = 5000.0;

                spec.color_matrix2 = spec.color_matrix1;
                spec.forward_matrix2 = spec.forward_matrix1;
                spec.reduction_matrix2 = spec.reduction_matrix1;
                spec.camera_calibration2 = spec.camera_calibration1;

                spec.color_matrix3 = spec.color_matrix1;
                spec.forward_matrix3 = spec.forward_matrix1;
                spec.reduction_matrix3 = spec.reduction_matrix1;
                spec.camera_calibration3 = spec.camera_calibration1;

                spec.num_illuminants = 1;
            } else {
                // Dual-illuminant profile.
                spec.color_matrix2 = spec
                    .analog_balance
                    .multiply(&spec.camera_calibration2)?
                    .multiply(&spec.color_matrix2)?;

                if spec.temperature2 < spec.temperature1 {
                    // Illuminant 2 is warmer than illuminant 1: swap the sets.
                    std::mem::swap(&mut spec.temperature1, &mut spec.temperature2);
                    std::mem::swap(&mut spec.color_matrix1, &mut spec.color_matrix2);
                    std::mem::swap(&mut spec.forward_matrix1, &mut spec.forward_matrix2);
                    std::mem::swap(&mut spec.reduction_matrix1, &mut spec.reduction_matrix2);
                    std::mem::swap(&mut spec.camera_calibration1, &mut spec.camera_calibration2);
                    std::mem::swap(&mut spec.light1, &mut spec.light2);
                }

                spec.num_illuminants = 2;

                if profile.has_color_matrix3() && profile.calibration_illuminant3() != 0 {
                    spec.color_matrix3 = spec
                        .analog_balance
                        .multiply(&spec.camera_calibration3)?
                        .multiply(&spec.color_matrix3)?;

                    spec.num_illuminants = 3;
                } else {
                    spec.color_matrix3 = spec.color_matrix2;
                    spec.forward_matrix3 = spec.forward_matrix2;
                    spec.reduction_matrix3 = spec.reduction_matrix2;
                    spec.camera_calibration3 = spec.camera_calibration2;
                    spec.light3 = spec.light2.clone();
                }
            }
        } else {
            // Monochrome: single-illuminant defaults.
            spec.temperature1 = 5000.0;
            spec.temperature2 = 5000.0;
            spec.num_illuminants = 1;
        }

        Ok(spec)
    }

    /// `Channels ()`.
    pub fn channels(&self) -> u32 {
        self.channels
    }

    /// `SetWhiteXY (white)`.
    pub fn set_white_xy(&mut self, white: &DngXyCoord) -> DngResult<()> {
        self.white_xy = *white;

        // Deal with monochrome cameras.
        if self.channels == 1 {
            self.camera_white.set_identity(1)?;
            self.camera_to_pcs = pcs_to_xyz().as_column()?;
            return Ok(());
        }

        // Interpolate matrix values for this white point.
        let mut forward_matrix = DngMatrix::new();
        let mut reduction_matrix = DngMatrix::new();
        let mut camera_calibration = DngMatrix::new();

        let color_matrix = self.find_xyz_to_camera(
            &self.white_xy,
            Some(&mut forward_matrix),
            Some(&mut reduction_matrix),
            Some(&mut camera_calibration),
        )?;

        // Find the camera white values.
        self.camera_white = color_matrix.multiply_vector(&xy_to_xyz(&self.white_xy))?;

        let camera_white_max_entry = self.camera_white.max_entry();

        if camera_white_max_entry == 0.0 {
            return Err(bad_format());
        }

        let white_scale = 1.0 / camera_white_max_entry;

        for j in 0..self.channels as usize {
            // We don't support non-positive camera neutral values.
            self.camera_white.set(
                j,
                pin_real64(0.001, white_scale * self.camera_white.get(j), 1.0),
            );
        }

        // PCS to Camera transform, scaled so PCS white saturates channel 1.
        let white_map = map_white_matrix(&pcs_to_xy(), &self.white_xy)?;
        self.pcs_to_camera = color_matrix.multiply(&white_map.0)?;

        let scale = self.pcs_to_camera.multiply(&pcs_to_xyz().as_column()?)?.max_entry();

        if scale == 0.0 {
            return Err(bad_format());
        }

        self.pcs_to_camera.scale(1.0 / scale);

        // Forward matrix path.
        if forward_matrix.not_empty() {
            let individual_to_reference =
                invert(&self.analog_balance.multiply(&camera_calibration)?)?;

            let ref_camera_white = individual_to_reference.multiply_vector(&self.camera_white)?;

            self.camera_to_pcs = forward_matrix
                .multiply(&ref_camera_white.as_diagonal()?)?
                .multiply(&individual_to_reference)?;
        } else {
            // Adapt-in-XYZ method.
            self.camera_to_pcs = invert_with_hint(&self.pcs_to_camera, &reduction_matrix)?;
        }

        Ok(())
    }

    /// `WhiteXY ()`.
    pub fn white_xy(&self) -> &DngXyCoord {
        &self.white_xy
    }

    /// `CameraWhite ()`.
    pub fn camera_white(&self) -> &DngVector {
        &self.camera_white
    }

    /// `CameraToPCS ()`.
    pub fn camera_to_pcs(&self) -> &DngMatrix {
        &self.camera_to_pcs
    }

    /// `PCStoCamera ()`.
    pub fn pcs_to_camera(&self) -> &DngMatrix {
        &self.pcs_to_camera
    }

    /// `NeutralToXY (neutral)` — iterative white-balance solve.
    pub fn neutral_to_xy(&self, neutral: &DngVector) -> DngResult<DngXyCoord> {
        const kMaxPasses: u32 = 30;

        if self.channels == 1 {
            return Ok(pcs_to_xy());
        }

        let mut last = d50_xy_coord();

        for pass in 0..kMaxPasses {
            let xyz_to_camera = self.find_xyz_to_camera(&last, None, None, None)?;

            let next = crate::dng_core::dng_xy_coord::xyz_to_xy(
                &invert(&xyz_to_camera)?.multiply_vector(neutral)?,
            );

            if abs_real64(next.x - last.x) + abs_real64(next.y - last.y) < 0.0000001 {
                return Ok(next);
            }

            // Oscillation guard: average the last two estimates.
            if pass == kMaxPasses - 1 {
                let mut averaged = next;
                averaged.x = (last.x + next.x) * 0.5;
                averaged.y = (last.y + next.y) * 0.5;
                return Ok(averaged);
            }

            last = next;
        }

        Ok(last)
    }

    /// `FindXYZtoCamera (white, ...)`.
    pub fn find_xyz_to_camera(
        &self,
        white: &DngXyCoord,
        mut forward_matrix: Option<&mut DngMatrix>,
        mut reduction_matrix: Option<&mut DngMatrix>,
        mut camera_calibration: Option<&mut DngMatrix>,
    ) -> DngResult<DngMatrix> {
        if self.num_illuminants <= 2 {
            self.find_xyz_to_camera_single_or_dual(
                white,
                forward_matrix.as_deref_mut(),
                reduction_matrix.as_deref_mut(),
                camera_calibration.as_deref_mut(),
            )
        } else {
            self.find_xyz_to_camera_triple(
                white,
                forward_matrix.as_deref_mut(),
                reduction_matrix.as_deref_mut(),
                camera_calibration.as_deref_mut(),
            )
        }
    }

    /// `FindXYZtoCamera_SingleOrDual`.
    fn find_xyz_to_camera_single_or_dual(
        &self,
        white: &DngXyCoord,
        mut forward_matrix: Option<&mut DngMatrix>,
        mut reduction_matrix: Option<&mut DngMatrix>,
        mut camera_calibration: Option<&mut DngMatrix>,
    ) -> DngResult<DngMatrix> {
        let td = DngTemperature::from_xy(*white);

        // Fraction weighting the first calibration.
        let g = if td.temperature() <= self.temperature1 {
            1.0
        } else if td.temperature() >= self.temperature2 {
            0.0
        } else {
            let inv_t = 1.0 / td.temperature();
            (inv_t - (1.0 / self.temperature2))
                / ((1.0 / self.temperature1) - (1.0 / self.temperature2))
        };

        // Interpolate the color matrix.
        let color_matrix = if g >= 1.0 {
            self.color_matrix1
        } else if g <= 0.0 {
            self.color_matrix2
        } else {
            let mut m = self.color_matrix1.scaled(g)?;
            let scaled2 = self.color_matrix2.scaled(1.0 - g)?;
            m.add(&scaled2)?
        };

        // Forward matrix interpolation.
        if let Some(fm) = forward_matrix.as_deref_mut() {
            let has1 = self.forward_matrix1.not_empty();
            let has2 = self.forward_matrix2.not_empty();

            if has1 && has2 {
                if g >= 1.0 {
                    *fm = self.forward_matrix1;
                } else if g <= 0.0 {
                    *fm = self.forward_matrix2;
                } else {
                    let mut m = self.forward_matrix1.scaled(g)?;
                    let scaled2 = self.forward_matrix2.scaled(1.0 - g)?;
                    *fm = m.add(&scaled2)?;
                }
            } else if has1 {
                *fm = self.forward_matrix1;
            } else if has2 {
                *fm = self.forward_matrix2;
            } else {
                fm.clear();
            }
        }

        // Reduction matrix interpolation.
        if let Some(rm) = reduction_matrix.as_deref_mut() {
            let has1 = self.reduction_matrix1.not_empty();
            let has2 = self.reduction_matrix2.not_empty();

            if has1 && has2 {
                if g >= 1.0 {
                    *rm = self.reduction_matrix1;
                } else if g <= 0.0 {
                    *rm = self.reduction_matrix2;
                } else {
                    let mut m = self.reduction_matrix1.scaled(g)?;
                    let scaled2 = self.reduction_matrix2.scaled(1.0 - g)?;
                    *rm = m.add(&scaled2)?;
                }
            } else if has1 {
                *rm = self.reduction_matrix1;
            } else if has2 {
                *rm = self.reduction_matrix2;
            } else {
                rm.clear();
            }
        }

        // Camera calibration interpolation.
        if let Some(cc) = camera_calibration.as_deref_mut() {
            let has1 = self.camera_calibration1.not_empty();
            let has2 = self.camera_calibration2.not_empty();

            if has1 && has2 {
                if g >= 1.0 {
                    *cc = self.camera_calibration1;
                } else if g <= 0.0 {
                    *cc = self.camera_calibration2;
                } else {
                    let mut m = self.camera_calibration1.scaled(g)?;
                    let scaled2 = self.camera_calibration2.scaled(1.0 - g)?;
                    *cc = m.add(&scaled2)?;
                }
            } else if has1 {
                *cc = self.camera_calibration1;
            } else if has2 {
                *cc = self.camera_calibration2;
            } else {
                cc.clear();
            }
        }

        Ok(color_matrix)
    }

    /// `FindXYZtoCamera_Triple` — weighted blend of three matrices.
    fn find_xyz_to_camera_triple(
        &self,
        white: &DngXyCoord,
        mut forward_matrix: Option<&mut DngMatrix>,
        mut reduction_matrix: Option<&mut DngMatrix>,
        mut camera_calibration: Option<&mut DngMatrix>,
    ) -> DngResult<DngMatrix> {
        let (w1, w2, w3) = self.calculate_triple_illuminant_weights(white);

        // Interpolate the color matrices.
        let mut color_matrix = self.color_matrix1.scaled(w1)?;
        let m2 = self.color_matrix2.scaled(w2)?;
        let m3 = self.color_matrix3.scaled(w3)?;
        color_matrix = color_matrix.add(&m2)?.add(&m3)?;

        // Forward matrices (all three or none, per C++).
        if let Some(fm) = forward_matrix.as_deref_mut() {
            let has = self.forward_matrix1.not_empty()
                && self.forward_matrix2.not_empty()
                && self.forward_matrix3.not_empty();

            if has {
                let mut m = self.forward_matrix1.scaled(w1)?;
                m = m.add(&self.forward_matrix2.scaled(w2)?)?;
                m = m.add(&self.forward_matrix3.scaled(w3)?)?;
                *fm = m;
            } else {
                fm.clear();
            }
        }

        if let Some(rm) = reduction_matrix.as_deref_mut() {
            let has = self.reduction_matrix1.not_empty()
                && self.reduction_matrix2.not_empty()
                && self.reduction_matrix3.not_empty();

            if has {
                let mut m = self.reduction_matrix1.scaled(w1)?;
                m = m.add(&self.reduction_matrix2.scaled(w2)?)?;
                m = m.add(&self.reduction_matrix3.scaled(w3)?)?;
                *rm = m;
            } else {
                rm.clear();
            }
        }

        if let Some(cc) = camera_calibration.as_deref_mut() {
            let has = self.camera_calibration1.not_empty()
                && self.camera_calibration2.not_empty()
                && self.camera_calibration3.not_empty();

            if has {
                let mut m = self.camera_calibration1.scaled(w1)?;
                m = m.add(&self.camera_calibration2.scaled(w2)?)?;
                m = m.add(&self.camera_calibration3.scaled(w3)?)?;
                *cc = m;
            } else {
                cc.clear();
            }
        }

        Ok(color_matrix)
    }

    /// `CalculateTripleIlluminantWeights` (wraps dng_xy_coord helper).
    fn calculate_triple_illuminant_weights(&self, white: &DngXyCoord) -> (f64, f64, f64) {
        crate::dng_core::dng_xy_coord::calculate_triple_illuminant_weights(
            white,
            &self.light1,
            &self.light2,
            &self.light3,
        )
    }

    // C++ NormalizeColorMatrix/NormalizeForwardMatrix live on the profile;
    // expose the forward one for the ctor path.
    fn normalize_forward_matrix(m: &mut DngMatrix) {
        crate::dng_camera_profile::DngCameraProfile::normalize_forward_matrix(m);
    }
}

/// Marker so dng_color_spec compiles even when SRational import is only used
/// in doc positions.
#[allow(unused)]
fn _doc(_: DngSRational, _: DngError) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_core::dng_xy_coord::d65_xy_coord;

    fn srgb_like_profile() -> DngCameraProfile {
        let mut p = DngCameraProfile::new();
        p.set_name("sRGB-like");
        p.set_calibration_illuminant1(21); // D65
        p.set_color_matrix1(
            &DngMatrix::from_rows(3, 3, &[0.4361, 0.3851, 0.1431, 0.2225, 0.7169, 0.0606, 0.0139, 0.0971, 0.7141])
                .unwrap(),
        );
        p.set_forward_matrix1(
            &DngMatrix::from_rows(3, 3, &[0.9, 0.05, 0.05, 0.05, 0.9, 0.05, 0.05, 0.05, 0.9]).unwrap(),
        );
        p
    }

    #[test]
    fn map_white_matrix_identity_for_same_white() {
        let d65 = d65_xy_coord();
        let m = map_white_matrix(&d65, &d65).unwrap();

        // Adapting white to itself must be (near) identity.
        let v = m.0.multiply_vector(&xy_to_xyz(&d65)).unwrap();
        let expected = xy_to_xyz(&d65);
        assert!((v.get(0) - expected.get(0)).abs() < 1e-9);
        assert!((v.get(2) - expected.get(2)).abs() < 1e-9);
    }

    #[test]
    fn spec_single_illuminant() {
        let profile = srgb_like_profile();
        let inputs = ColorSpecInputs::default();

        let mut spec = DngColorSpec::new(3, Some(&profile), false, &inputs).unwrap();
        assert_eq!(spec.channels(), 3);
        assert_eq!(spec.num_illuminants, 1);

        // Set the white point to D65 (the calibration illuminant).
        spec.set_white_xy(&d65_xy_coord()).unwrap();

        assert!(spec.camera_white().count() == 3);
        // Camera white should be normalized to max 1.0.
        assert!((spec.camera_white().max_entry() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn neutral_to_xy_converges() {
        let profile = srgb_like_profile();
        let inputs = ColorSpecInputs::default();

        let mut spec = DngColorSpec::new(3, Some(&profile), false, &inputs).unwrap();
        spec.set_white_xy(&d65_xy_coord()).unwrap();

        // Camera neutral at the calibrated white should round-trip to D65.
        let neutral = spec.camera_white().clone();
        let xy = spec.neutral_to_xy(&neutral).unwrap();

        assert!((xy.x - d65_xy_coord().x).abs() < 1e-4, "x={}", xy.x);
        assert!((xy.y - d65_xy_coord().y).abs() < 1e-4, "y={}", xy.y);
    }

    #[test]
    fn monochrome_spec() {
        let inputs = ColorSpecInputs::default();
        let mut spec = DngColorSpec::new(1, None, false, &inputs).unwrap();

        spec.set_white_xy(&d65_xy_coord()).unwrap();
        assert_eq!(spec.camera_white().get(0), 1.0);
        assert_eq!(spec.neutral_to_xy(&DngVector::from_values(&[1.0]).unwrap()).unwrap(), pcs_to_xy());
    }

    #[test]
    fn requires_valid_profile_for_color() {
        let inputs = ColorSpecInputs::default();
        assert!(DngColorSpec::new(3, None, false, &inputs).is_err());
        assert!(DngColorSpec::new(3, Some(&DngCameraProfile::new()), false, &inputs).is_err());
    }
}

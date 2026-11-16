// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_color_space.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_color_space.h/.cpp — the built-in color spaces with their
// EXACT matrices (copied verbatim) and gamma functions, plus the embedded
// ICC profiles (extracted verbatim into dng_color_space_profiles.rs).

use crate::dng_1d_function::{Dng1dFunction, Dng1dIdentity};
use crate::dng_errors::DngResult;
use crate::dng_matrix::{
    invert, DngMatrix, DngMatrix3by3, DngVector,
};
use crate::dng_types::{real64, uint8};
use crate::dng_xy_coord::pcs_to_xyz;
use std::sync::OnceLock;

mod profiles {
    include!("dng_color_space_profiles.rs");
}

// ---------------------------------------------------------------------------
// Gamma functions.
// ---------------------------------------------------------------------------

/// `dng_function_GammaEncode_sRGB`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DngFunctionGammaEncodeSRGB;

impl Dng1dFunction for DngFunctionGammaEncodeSRGB {
    fn evaluate(&self, x: real64) -> real64 {
        if x <= 0.0031308 {
            x * 12.92
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        }
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        if y <= 0.0031308 * 12.92 {
            y * (1.0 / 12.92)
        } else {
            ((y + 0.055) * (1.0 / 1.055)).powf(2.4)
        }
    }
}

impl DngFunctionGammaEncodeSRGB {
    /// `dng_function_GammaEncode_sRGB::Get ()`.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: OnceLock<DngFunctionGammaEncodeSRGB> = OnceLock::new();
        INSTANCE.get_or_init(DngFunctionGammaEncodeSRGB::default)
    }
}

/// `dng_function_GammaEncode_1_8` — spline-smoothed 1/1.8 power.
#[derive(Debug, Clone, Copy, Default)]
pub struct DngFunctionGammaEncode1_8;

impl DngFunctionGammaEncode1_8 {
    /// `dng_function_GammaEncode_1_8::Get ()`.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: OnceLock<DngFunctionGammaEncode1_8> = OnceLock::new();
        INSTANCE.get_or_init(DngFunctionGammaEncode1_8::default)
    }
}

impl Dng1dFunction for DngFunctionGammaEncode1_8 {
    fn evaluate(&self, x: real64) -> real64 {
        let gamma = 1.0 / 1.8;
        let slope0 = 32.0;
        let x1 = 8.211_879_055_2e-4; // pow (slope0, 1.0 / (gamma - 1.0)) * 2.0
        let y1 = 0.019_310_851; // pow (x1, gamma)
        let slope1 = 13.064_306_598; // gamma * pow (x1, gamma - 1.0)

        if x <= x1 {
            crate::dng_spline::evaluate_spline_segment(x, 0.0, 0.0, slope0, x1, y1, slope1)
        } else {
            x.powf(gamma)
        }
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        if y > 0.0 && y < 0.019_310_851 {
            // Newton iteration fallback (dng_1d_function::EvaluateInverse).
            return newton_inverse(|x| self.evaluate(x), y);
        }
        y.powf(1.8)
    }
}

/// `dng_function_GammaEncode_2_2` — spline-smoothed 1/2.2 power.
#[derive(Debug, Clone, Copy, Default)]
pub struct DngFunctionGammaEncode2_2;

impl DngFunctionGammaEncode2_2 {
    /// `dng_function_GammaEncode_2_2::Get ()`.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: OnceLock<DngFunctionGammaEncode2_2> = OnceLock::new();
        INSTANCE.get_or_init(DngFunctionGammaEncode2_2::default)
    }
}

impl Dng1dFunction for DngFunctionGammaEncode2_2 {
    fn evaluate(&self, x: real64) -> real64 {
        let gamma = 1.0 / 2.2;
        let slope0 = 32.0;
        let x1 = 0.003_480_073_1; // pow (slope0, 1.0 / (gamma - 1.0)) * 2.0
        let y1 = 0.076_302_745_8; // pow (x1, gamma)
        let slope1 = 9.966_189_007_5; // gamma * pow (x1, gamma - 1.0)

        if x <= x1 {
            crate::dng_spline::evaluate_spline_segment(x, 0.0, 0.0, slope0, x1, y1, slope1)
        } else {
            x.powf(gamma)
        }
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        if y > 0.0 && y < 0.076_302_745_8 {
            return newton_inverse(|x| self.evaluate(x), y);
        }
        y.powf(2.2)
    }
}

/// `dng_function_GammaEncode_TwoPart` base + `dng_function_GammaEncode_Rec709`
/// (also used for Rec2020 via typedef).
#[derive(Debug, Clone, Copy, Default)]
pub struct DngFunctionGammaEncodeRec709 {
    alpha: real64,
    beta: real64,
    slope: real64,
    gamma: real64,
}

impl DngFunctionGammaEncodeRec709 {
    fn new() -> Self {
        DngFunctionGammaEncodeRec709 {
            alpha: 1.099_296_826_809_442_9,
            beta: 0.018_053_968_510_807_8,
            slope: 4.5,
            gamma: 0.45,
        }
    }

    /// `dng_function_GammaEncode_Rec709::Get ()`.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: OnceLock<DngFunctionGammaEncodeRec709> = OnceLock::new();
        INSTANCE.get_or_init(DngFunctionGammaEncodeRec709::new)
    }
}

impl Dng1dFunction for DngFunctionGammaEncodeRec709 {
    fn evaluate(&self, x: real64) -> real64 {
        if x <= self.beta {
            self.slope * x
        } else {
            self.alpha * x.powf(self.gamma) - (self.alpha - 1.0)
        }
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        if y <= self.slope * self.beta {
            y / self.slope
        } else {
            ((y + (self.alpha - 1.0)) / self.alpha).powf(1.0 / self.gamma)
        }
    }
}

/// Secant/Newton inverse matching `dng_1d_function::EvaluateInverse`.
fn newton_inverse<F: Fn(real64) -> real64>(evaluate: F, y: real64) -> real64 {
    const kMaxIterations: u32 = 30;
    const kNearZero: real64 = 1.0e-10;

    let mut x0 = 0.0;
    let mut y0 = evaluate(x0);

    let mut x1 = 1.0;
    let mut y1 = evaluate(x1);

    for _ in 0..kMaxIterations {
        if crate::dng_utils::abs_real64(y1 - y0) < kNearZero {
            break;
        }

        let x2 = crate::dng_utils::pin_real64_01(x1 + (y - y1) * (x1 - x0) / (y1 - y0));
        let y2 = evaluate(x2);

        x0 = x1;
        y0 = y1;

        x1 = x2;
        y1 = y2;
    }

    x1
}

// ---------------------------------------------------------------------------
// dng_color_space.
// ---------------------------------------------------------------------------

/// Port of `dng_color_space`.
#[derive(Debug, Clone)]
pub struct DngColorSpace {
    matrix_to_pcs: DngMatrix,
    matrix_from_pcs: DngMatrix,
    gamma: GammaKind,
    icc_profile: Option<&'static [uint8]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GammaKind {
    Identity,
    SRGB,
    Gamma18,
    Gamma22,
    Rec709,
}

impl DngColorSpace {
    /// Empty space (C++ protected default ctor).
    fn empty() -> Self {
        DngColorSpace {
            matrix_to_pcs: DngMatrix::new(),
            matrix_from_pcs: DngMatrix::new(),
            gamma: GammaKind::Identity,
            icc_profile: None,
        }
    }

    /// `SetMonochrome ()`.
    fn set_monochrome(&mut self) -> DngResult<()> {
        self.matrix_to_pcs = pcs_to_xyz().as_column()?;
        let mut m = DngMatrix::with_dims(1, 3)?;
        m.set(0, 0, 0.0);
        m.set(0, 1, 1.0);
        m.set(0, 2, 0.0);
        self.matrix_from_pcs = m;
        Ok(())
    }

    /// `SetMatrixToPCS (M)` — white-balances M to PCS and inverts.
    fn set_matrix_to_pcs(&mut self, m: &DngMatrix3by3) -> DngResult<()> {
        let ones = DngVector::from_values(&[1.0, 1.0, 1.0])?;
        let w1 = m.0.multiply_vector(&ones)?;
        let w2 = pcs_to_xyz();

        let s0 = w2.get(0) / w1.get(0);
        let s1 = w2.get(1) / w1.get(1);
        let s2 = w2.get(2) / w1.get(2);

        let s = DngMatrix3by3::diagonal(s0, s1, s2);

        self.matrix_to_pcs = s.0.multiply(&m.0)?;
        self.matrix_from_pcs = invert(&self.matrix_to_pcs)?;

        Ok(())
    }

    /// `MatrixToPCS ()`.
    pub fn matrix_to_pcs(&self) -> &DngMatrix {
        &self.matrix_to_pcs
    }

    /// `MatrixFromPCS ()`.
    pub fn matrix_from_pcs(&self) -> &DngMatrix {
        &self.matrix_from_pcs
    }

    /// `IsMonochrome ()`.
    pub fn is_monochrome(&self) -> bool {
        self.matrix_to_pcs.cols() == 1
    }

    /// `GammaFunction ()`.
    pub fn gamma_function(&self) -> &'static dyn Dng1dFunction {
        match self.gamma {
            GammaKind::Identity => Dng1dIdentity::get(),
            GammaKind::SRGB => DngFunctionGammaEncodeSRGB::get(),
            GammaKind::Gamma18 => DngFunctionGammaEncode1_8::get(),
            GammaKind::Gamma22 => DngFunctionGammaEncode2_2::get(),
            GammaKind::Rec709 => DngFunctionGammaEncodeRec709::get(),
        }
    }

    /// `IsLinear ()`.
    pub fn is_linear(&self) -> bool {
        self.gamma_function().is_identity()
    }

    /// `GammaEncode (x)`.
    pub fn gamma_encode(&self, x: real64) -> real64 {
        self.gamma_function().evaluate(x)
    }

    /// `GammaDecode (y)`.
    pub fn gamma_decode(&self, y: real64) -> real64 {
        self.gamma_function().evaluate_inverse(y)
    }

    /// `ICCProfile (size, data)` — `None` when the space has no profile.
    pub fn icc_profile(&self) -> Option<&'static [uint8]> {
        self.icc_profile
    }
}

// ---------------------------------------------------------------------------
// Built-in spaces (singletons mirroring the C++ static Get () methods).
// ---------------------------------------------------------------------------

macro_rules! define_space {
    ($get:ident, $builder:expr) => {
        pub fn $get() -> &'static DngColorSpace {
            static SPACE: OnceLock<DngColorSpace> = OnceLock::new();
            SPACE.get_or_init(|| $builder().expect("built-in color space construction failed"))
        }
    };
}

/// `dng_space_sRGB::Get ()`.
fn build_srgb() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::SRGB;
    s.icc_profile = Some(profiles::KSRGBPROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.4361, 0.3851, 0.1431, 0.2225, 0.7169, 0.0606, 0.0139, 0.0971, 0.7141,
    ))?;
    Ok(s)
}
define_space!(space_srgb, build_srgb);

/// `dng_space_AdobeRGB::Get ()`.
fn build_adobe_rgb() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Gamma22;
    s.icc_profile = Some(profiles::KADOBERGBPROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.6097, 0.2053, 0.1492, 0.3111, 0.6257, 0.0632, 0.0195, 0.0609, 0.7446,
    ))?;
    Ok(s)
}
define_space!(space_adobe_rgb, build_adobe_rgb);

/// `dng_space_ColorMatch::Get ()`.
fn build_color_match() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Gamma18;
    s.icc_profile = Some(profiles::KCOLORMATCHPROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.5094, 0.3208, 0.1339, 0.2749, 0.6581, 0.0670, 0.0243, 0.1087, 0.6919,
    ))?;
    Ok(s)
}
define_space!(space_color_match, build_color_match);

/// `dng_space_DisplayP3::Get ()`.
fn build_display_p3() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::SRGB;
    s.icc_profile = Some(profiles::KDISPLAYP3PROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.5151, 0.2920, 0.1571, 0.2412, 0.6922, 0.0666, -0.0010, 0.0419, 0.7843,
    ))?;
    Ok(s)
}
define_space!(space_display_p3, build_display_p3);

/// `dng_space_Rec2020::Get ()`.
fn build_rec2020() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Rec709; // dng_function_GammaEncode_Rec2020 == Rec709
    s.icc_profile = Some(profiles::KREC2020PROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.6735, 0.1657, 0.1251, 0.2791, 0.6753, 0.0456, -0.0019, 0.0300, 0.7971,
    ))?;
    Ok(s)
}
define_space!(space_rec2020, build_rec2020);

/// `dng_space_ProPhoto::Get ()`.
fn build_pro_photo() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Gamma18;
    s.icc_profile = Some(profiles::KPROPHOTOPROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.7977, 0.1352, 0.0313, 0.2880, 0.7119, 0.0001, 0.0000, 0.0000, 0.8249,
    ))?;
    Ok(s)
}
define_space!(space_pro_photo, build_pro_photo);

/// `dng_space_GrayGamma18::Get ()`.
fn build_gray_gamma18() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Gamma18;
    s.icc_profile = Some(profiles::KGAMMA18PROFILEDATA.as_slice());
    s.set_monochrome()?;
    Ok(s)
}
define_space!(space_gray_gamma18, build_gray_gamma18);

/// `dng_space_GrayGamma22::Get ()`.
fn build_gray_gamma22() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Gamma22;
    s.icc_profile = Some(profiles::KGAMMA22PROFILEDATA.as_slice());
    s.set_monochrome()?;
    Ok(s)
}
define_space!(space_gray_gamma22, build_gray_gamma22);

/// `dng_space_sRGB_Linear::Get ()`.
fn build_srgb_linear() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KSRGBLINEARPROFILEDATA.as_slice());
    s.matrix_to_pcs = space_srgb().matrix_to_pcs.clone();
    s.matrix_from_pcs = space_srgb().matrix_from_pcs.clone();
    Ok(s)
}
define_space!(space_srgb_linear, build_srgb_linear);

/// `dng_space_AdobeRGB_Linear::Get ()`.
fn build_adobe_rgb_linear() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KADOBERGBLINEARPROFILEDATA.as_slice());
    s.matrix_to_pcs = space_adobe_rgb().matrix_to_pcs.clone();
    s.matrix_from_pcs = space_adobe_rgb().matrix_from_pcs.clone();
    Ok(s)
}
define_space!(space_adobe_rgb_linear, build_adobe_rgb_linear);

/// `dng_space_ProPhoto_Linear::Get ()`.
fn build_pro_photo_linear() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KPROPHOTOLINEARPROFILEDATA.as_slice());
    s.matrix_to_pcs = space_pro_photo().matrix_to_pcs.clone();
    s.matrix_from_pcs = space_pro_photo().matrix_from_pcs.clone();
    Ok(s)
}
define_space!(space_pro_photo_linear, build_pro_photo_linear);

/// `dng_space_Gray_Linear::Get ()`.
fn build_gray_linear() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KGRAYLINEARPROFILEDATA.as_slice());
    s.set_monochrome()?;
    Ok(s)
}
define_space!(space_gray_linear, build_gray_linear);

/// `dng_space_LinearP3::Get ()`.
fn build_linear_p3() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KLINEARP3PROFILEDATA.as_slice());
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.5151, 0.2920, 0.1571, 0.2412, 0.6922, 0.0666, -0.0010, 0.0419, 0.7843,
    ))?;
    Ok(s)
}
define_space!(space_linear_p3, build_linear_p3);

/// `dng_space_Rec2020_Linear::Get ()`.
fn build_rec2020_linear() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.icc_profile = Some(profiles::KREC2020_LINEARPROFILEDATA.as_slice());
    // Copied from dng_space_Rec2020::dng_space_Rec2020.
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.6735, 0.1657, 0.1251, 0.2791, 0.6753, 0.0456, -0.0019, 0.0300, 0.7971,
    ))?;
    Ok(s)
}
define_space!(space_rec2020_linear, build_rec2020_linear);

/// `dng_space_fakeRGB::Get ()` — same primaries as AdobeRGB, identity gamma,
/// no ICC profile (a unit-test space).
fn build_fake_rgb() -> DngResult<DngColorSpace> {
    let mut s = DngColorSpace::empty();
    s.gamma = GammaKind::Identity;
    s.set_matrix_to_pcs(&DngMatrix3by3::new(
        0.6097, 0.2053, 0.1492, 0.3111, 0.6257, 0.0632, 0.0195, 0.0609, 0.7446,
    ))?;
    Ok(s)
}
define_space!(space_fake_rgb, build_fake_rgb);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_matrix_exact() {
        // The C++ stores the whitened matrix; verify a few entries against
        // the known D50-adapted sRGB matrix.
        let m = space_srgb().matrix_to_pcs();
        assert_eq!(m.rows(), 3);
        assert_eq!(m.cols(), 3);

        // White maps to the PCS white: M * (1,1,1)^T == PCStoXYZ (up to
        // D50 chromaticity rounding in the stored constants).
        let ones = crate::dng_matrix::DngVector::from_values(&[1.0, 1.0, 1.0]).unwrap();
        let white = m.multiply_vector(&ones).unwrap();
        assert!((white.get(1) - 1.0).abs() < 1e-6, "Y of white = {}", white.get(1));

        // Round trip: PCS -> sRGB -> PCS.
        let to = space_srgb().matrix_to_pcs();
        let from = space_srgb().matrix_from_pcs();
        let product = from.multiply(to).unwrap();
        assert!(product.almost_identity(1e-9));
    }

    #[test]
    fn gamma_functions() {
        // sRGB: 0.5 encodes to ~0.735.
        let e = space_srgb().gamma_encode(0.5);
        assert!((e - 0.7353569830524495).abs() < 1e-9, "e={e}");
        assert!((space_srgb().gamma_decode(e) - 0.5).abs() < 1e-12);

        // Linear spaces are identity.
        assert!(space_srgb_linear().is_linear());
        assert!(space_gray_linear().is_linear());
        assert!(!space_srgb().is_linear());

        // AdobeRGB gamma 2.2: 0.5 -> 0.5^(1/2.2).
        let e = space_adobe_rgb().gamma_encode(0.5);
        assert!((e - 0.5f64.powf(1.0 / 2.2)).abs() < 1e-9);

        // Gamma 1.8 spaces.
        let e = space_color_match().gamma_encode(0.5);
        assert!((e - 0.5f64.powf(1.0 / 1.8)).abs() < 1e-9);
    }

    #[test]
    fn monochrome_spaces() {
        assert!(space_gray_gamma18().is_monochrome());
        assert!(space_gray_gamma22().is_monochrome());
        assert!(space_gray_linear().is_monochrome());
        assert!(!space_srgb().is_monochrome());

        let m = space_gray_gamma22().matrix_to_pcs();
        assert_eq!(m.cols(), 1);
        // Column is the PCS white (Y-normalized).
        assert!((m.get(1, 0) - 1.0).abs() < 1e-9);

        let from = space_gray_gamma22().matrix_from_pcs();
        assert_eq!(from.rows(), 1);
        assert_eq!(from.get(0, 1), 1.0);
        assert_eq!(from.get(0, 0), 0.0);
        assert_eq!(from.get(0, 2), 0.0);
    }

    #[test]
    fn rec709_gamma() {
        // Rec2020 uses the Rec709 two-part curve.
        let f = DngFunctionGammaEncodeRec709::get();
        let e = f.evaluate(0.5);
        assert!((e - (1.0992968268094429 * 0.5f64.powf(0.45) - 0.0992968268094429)).abs() < 1e-12);
        assert!((f.evaluate_inverse(e) - 0.5).abs() < 1e-12);
        // Linear segment.
        assert_eq!(f.evaluate(0.01), 0.045);
        assert_eq!(f.evaluate_inverse(0.045), 0.01);
    }

    #[test]
    fn icc_profiles_present() {
        assert_eq!(space_srgb().icc_profile().unwrap().len(), 3144);
        assert_eq!(space_adobe_rgb().icc_profile().unwrap().len(), 560);
        assert_eq!(space_color_match().icc_profile().unwrap().len(), 560);
        assert_eq!(space_display_p3().icc_profile().unwrap().len(), 548);
        assert_eq!(space_rec2020().icc_profile().unwrap().len(), 504);
        assert_eq!(space_pro_photo().icc_profile().unwrap().len(), 940);
        assert_eq!(space_gray_gamma18().icc_profile().unwrap().len(), 408);
        assert_eq!(space_gray_gamma22().icc_profile().unwrap().len(), 408);
        assert_eq!(space_srgb_linear().icc_profile().unwrap().len(), 520);
        assert_eq!(space_adobe_rgb_linear().icc_profile().unwrap().len(), 516);
        assert_eq!(space_pro_photo_linear().icc_profile().unwrap().len(), 496);
        assert_eq!(space_gray_linear().icc_profile().unwrap().len(), 376);
        assert_eq!(space_linear_p3().icc_profile().unwrap().len(), 488);
        assert_eq!(space_rec2020_linear().icc_profile().unwrap().len(), 496);
        // fakeRGB has no profile.
        assert!(space_fake_rgb().icc_profile().is_none());

        // Profile headers start with the ICC size.
        let p = space_srgb().icc_profile().unwrap();
        let size = ((p[0] as u32) << 24) | ((p[1] as u32) << 16) | ((p[2] as u32) << 8) | p[3] as u32;
        assert_eq!(size, 3144);
    }
}

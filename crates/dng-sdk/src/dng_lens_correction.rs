// Port of dng_lens_correction.h/.cpp (SDK 1.7.1) — lens warp and vignette
// opcode classes.
//
// This replaces Agent B's TEMPORARY SEAM STUB (backed up at the time of
// replacement); the stream-format contract for the four opcode classes is
// preserved exactly.
//
// Porting notes:
//   * C++ class hierarchy `dng_warp_params` -> rectilinear/fisheye becomes
//     trait `DngWarpParams` implemented by `DngWarpParamsRectilinear` and
//     `DngWarpParamsFisheye` (Rust has no inheritance; same pattern as
//     `DngFilterTaskBase` in dng-core).
//   * `dng_warp_params_radial` is a value struct (kMaxTerms = 15 polynomial
//     with optional reciprocal mode).
//   * `dng_filter_warp` runs serially via `DngAreaTask` (ARCHITECTURE.md
//     section 9) using dng-core's 2D bicubic resample weights; the C++
//     tile-size upper-bound logic (`SrcTileSize`) is kept for parity.
//   * `DoVignetteMask16` / `DoVignette32` are the scalar reference ports
//     (dng_reference.cpp RefVignetteMask16 / RefVignette32) inlined into
//     ProcessArea per the dng-core PORTING_NOTES phase-2 placement note.
//   * `MakeParamsForRender` (subclass hook) returns the stored params
//     exactly like the C++ base implementation.
//   * The C++ `dng_filter_warp::SrcTileSize` bound is advisory in the port:
//     `SrcArea` already returns the exact enclosing rect, so the filter
//     reads that rect directly.
//   * `dng_vignette_radial_function` implements dng-core's Dng1dFunction so
//     the gain LUT is built with the same 1d-table machinery as C++.

use dng_core::{
    abs_real64, bad_format, lerp_real64, max_int32, max_real32, max_real64, min_int32,
    min_real32, min_real64, pin_real32, pin_real64, program_error, real64_to_fixed64,
    round_uint32_f64, Dng1dFunction, DngError, DngHost, DngPoint, DngPointReal64, DngRect,
    DngRectReal64, DngResult, DngStream, DngVector, ImageRef, int32, int64, real32, real64,
    uint32,
};
use dng_core::dng_1d_table::Dng1dTable;
use dng_core::dng_area_task::DngAreaTask;
use dng_core::dng_image::DngImage;
use dng_core::dng_pixel_buffer::DngPixelBuffer;
use dng_core::dng_resample::{
    kResampleSubsampleCount2D, DngResampleBicubic, DngResampleFunction, DngResampleWeights2d,
};
use dng_core::dng_sdk_limits::kMaxColorPlanes;
use dng_core::dng_tag_types::ttFloat;
use dng_core::dng_tag_values::{dngVersion_1_3_0_0, dngVersion_1_6_0_0, pcRowInterleavedAlignSIMD};
use dng_core::dng_utils::{max_distance_point_to_rect, round_int32_f64};

use crate::dng_negative::DngNegative;
use crate::dng_opcodes::{
    dngOpcode_FixVignetteRadial, dngOpcode_WarpFisheye, dngOpcode_WarpRectilinear,
    dngOpcode_WarpRectilinear2, DngFilterOpcode, DngInplaceOpcode, DngOpcode, DngOpcodeBase,
};

// ---------------------------------------------------------------------------
// dng_warp_params (abstract base) — trait.
// ---------------------------------------------------------------------------

/// Port of `dng_warp_params` (abstract) — common warp opcode parameters and
/// routines.
pub trait DngWarpParams: Send + Sync {
    /// `fPlanes` access.
    fn planes(&self) -> uint32;

    /// `fPlanes` mutation.
    fn set_planes(&mut self, planes: uint32);

    /// `fCenter` access.
    fn center(&self) -> DngPointReal64;

    /// `IsNOPAll ()` — is the entire correction a NOP for all planes?
    fn is_nop_all(&self) -> bool {
        self.is_rad_nop_all() && self.is_tan_nop_all()
    }

    /// `IsNOP (plane)`.
    fn is_nop(&self, plane: uint32) -> bool {
        self.is_rad_nop(plane) && self.is_tan_nop(plane)
    }

    /// `IsRadNOPAll ()`.
    fn is_rad_nop_all(&self) -> bool {
        for plane in 0..self.planes() {
            if !self.is_rad_nop(plane) {
                return false;
            }
        }

        true
    }

    /// `IsRadNOP (plane)`.
    fn is_rad_nop(&self, plane: uint32) -> bool;

    /// `IsTanNOPAll ()`.
    fn is_tan_nop_all(&self) -> bool {
        for plane in 0..self.planes() {
            if !self.is_tan_nop(plane) {
                return false;
            }
        }

        true
    }

    /// `IsTanNOP (plane)`.
    fn is_tan_nop(&self, plane: uint32) -> bool;

    /// `IsValid ()`.
    fn is_valid(&self) -> bool {
        if self.planes() < 1 || self.planes() > kMaxColorPlanes {
            return false;
        }

        let center = self.center();

        if center.h < 0.0 || center.h > 1.0 || center.v < 0.0 || center.v > 1.0 {
            return false;
        }

        true
    }

    /// `IsValidForNegative (negative)`.
    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        if !self.is_valid() {
            return false;
        }

        if self.planes() != 1 && self.planes() != negative.color_channels() {
            return false;
        }

        true
    }

    /// `PropagateToAllPlanes (totalPlanes)`.
    fn propagate_to_all_planes(&mut self, total_planes: uint32);

    /// `Evaluate (plane, r)` — the 1D radial warp function; `r` is the
    /// destination (corrected) normalized radius in [0, 1].
    fn evaluate(&self, plane: uint32, r: real64) -> real64;

    /// `EvaluateInverse (plane, r)` — Newton's method inversion of
    /// `Evaluate` (base implementation).
    fn evaluate_inverse(&self, plane: uint32, y: real64) -> real64 {
        const K_MAX_ITERATIONS: uint32 = 30;
        const K_NEAR_ZERO: real64 = 1.0e-10;

        let mut x0 = 0.0;
        let mut y0 = self.evaluate(plane, x0);

        let mut x1 = 1.0;
        let mut y1 = self.evaluate(plane, x1);

        for _iteration in 0..K_MAX_ITERATIONS {
            if abs_real64(y1 - y0) < K_NEAR_ZERO {
                break;
            }

            let x2 = pin_real64(0.0, x1 + (y - y1) * (x1 - x0) / (y1 - y0), 1.0);

            let y2 = self.evaluate(plane, x2);

            x0 = x1;
            y0 = y1;

            x1 = x2;
            y1 = y2;
        }

        x1
    }

    /// `EvaluateRatio (plane, r2)` — the radial warp ratio function;
    /// `EvaluateRatio (r * r)` equals `Evaluate (r) / r`.
    fn evaluate_ratio(&self, plane: uint32, r2: real64) -> real64;

    /// `EvaluateTangential (plane, r2, diff, diff2)` — the 2D tangential
    /// warp; result is the offset in pixels.
    fn evaluate_tangential(
        &self,
        plane: uint32,
        r2: real64,
        diff: &DngPointReal64,
        diff2: &DngPointReal64,
    ) -> DngPointReal64;

    /// `EvaluateTangential2 (plane, diff)`.
    fn evaluate_tangential2(&self, plane: uint32, diff: &DngPointReal64) -> DngPointReal64 {
        let dvdv = diff.v * diff.v;
        let dhdh = diff.h * diff.h;

        let rr = dvdv + dhdh;

        let diff_sqr = DngPointReal64::new(dvdv, dhdh);

        self.evaluate_tangential(plane, rr, diff, &diff_sqr)
    }

    /// `EvaluateTangential3 (plane, r2, diff)`.
    fn evaluate_tangential3(
        &self,
        plane: uint32,
        r2: real64,
        diff: &DngPointReal64,
    ) -> DngPointReal64 {
        let diff_sqr = DngPointReal64::new(diff.v * diff.v, diff.h * diff.h);

        self.evaluate_tangential(plane, r2, diff, &diff_sqr)
    }

    /// `MaxSrcRadiusGap (maxDstGap)`.
    fn max_src_radius_gap(&self, max_dst_gap: real64) -> real64;

    /// `MaxSrcTanGap (minDst, maxDst)`.
    fn max_src_tan_gap(
        &self,
        min_dst: &DngPointReal64,
        max_dst: &DngPointReal64,
    ) -> DngPointReal64;

    /// `SafeMinRatio ()`.
    fn safe_min_ratio(&self) -> real64;

    /// `SafeMaxRatio ()`.
    fn safe_max_ratio(&self) -> real64;
}

// ---------------------------------------------------------------------------
// dng_warp_params_radial.
// ---------------------------------------------------------------------------

/// Port of `dng_warp_params_radial` — the radial polynomial model
/// f(r) = sum_{i=0..N-1} (k_i * r^i) with optional reciprocal mode.
#[derive(Debug, Clone)]
pub struct DngWarpParamsRadial {
    /// `fData [plane][term]` — polynomial coefficients.
    pub data: Vec<Vec<real64>>,

    /// `fValidRange [plane]` — [min_valid_radius, max_valid_radius].
    pub valid_range: Vec<[real64; 2]>,

    /// `fUseReciprocal` — when true the mapping is r_real = r_ideal / f.
    pub use_reciprocal: bool,
}

impl DngWarpParamsRadial {
    /// `kMaxTerms`.
    pub const kMaxTerms: uint32 = 15;

    /// `dng_warp_params_radial ()` — all planes NOP.
    pub fn new() -> Self {
        let mut result = DngWarpParamsRadial {
            data: vec![vec![0.0; Self::kMaxTerms as usize]; kMaxColorPlanes as usize],
            valid_range: vec![[0.0, 1.0]; kMaxColorPlanes as usize],
            use_reciprocal: false,
        };

        result.set_nop_all();

        result
    }

    /// `IsValid (plane)`.
    pub fn is_valid(&self, plane: uint32) -> bool {
        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        let min_valid_radius = self.valid_range[plane as usize][0];
        let max_valid_radius = self.valid_range[plane as usize][1];

        if !(0.0 <= min_valid_radius
            && min_valid_radius < max_valid_radius
            && max_valid_radius <= 1.0)
        {
            return false;
        }

        true
    }

    /// `IsNOP (plane)`.
    pub fn is_nop(&self, plane: uint32) -> bool {
        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        let data = &self.data[plane as usize];

        if data[0] != 1.0 {
            return false;
        }

        for i in 1..Self::kMaxTerms as usize {
            if data[i] != 0.0 {
                return false;
            }
        }

        true
    }

    /// `SetNOP ()` — all planes.
    pub fn set_nop_all(&mut self) {
        for i in 0..kMaxColorPlanes {
            self.set_nop(i);
        }
    }

    /// `SetNOP (plane)`.
    pub fn set_nop(&mut self, plane: uint32) {
        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        let data = &mut self.data[plane as usize];

        data[0] = 1.0;

        for i in 1..Self::kMaxTerms as usize {
            data[i] = 0.0;
        }

        self.valid_range[plane as usize][0] = 0.0;
        self.valid_range[plane as usize][1] = 1.0;
    }

    /// `SetWarpRectilinear_1_3 (plane, params)` — DNG 1.3 opcode layout:
    /// params are [offset, r^2, r^4, r^6].
    pub fn set_warp_rectilinear_1_3(&mut self, plane: uint32, params: &DngVector) {
        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        let dst = &mut self.data[plane as usize];

        dst[0] = params.get(0); // offset
        dst[1] = 0.0;
        dst[2] = params.get(1); // r^2
        dst[3] = 0.0;
        dst[4] = params.get(2); // r^4
        dst[5] = 0.0;
        dst[6] = params.get(3); // r^6

        // Also clear the higher-order terms.

        for i in 7..Self::kMaxTerms as usize {
            dst[i] = 0.0;
        }

        // Don't use the reciprocal model.

        self.use_reciprocal = false;

        // Set the valid range to wide-open.

        self.valid_range[plane as usize][0] = 0.0;
        self.valid_range[plane as usize][1] = 1.0;
    }

    /// `CompatibleWithWarpRectilinear_1_3 (plane)`.
    pub fn compatible_with_warp_rectilinear_1_3(&self, plane: uint32) -> bool {
        // DNG 1.3 model doesn't support reciprocal mode.

        if self.use_reciprocal {
            return false;
        }

        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        // Check valid range.

        if self.valid_range[plane as usize][0] != 0.0
            || self.valid_range[plane as usize][1] != 1.0
        {
            return false;
        }

        // Check low-order terms.

        let data = &self.data[plane as usize];

        if data[1] != 0.0 || data[3] != 0.0 || data[5] != 0.0 {
            return false;
        }

        // Also check higher-order terms.

        for i in 7..Self::kMaxTerms as usize {
            if data[i] != 0.0 {
                return false;
            }
        }

        true
    }

    /// `Evaluate (plane, r)` — ideal (dst) to real (src) radial warp.
    pub fn evaluate(&self, plane: uint32, r: real64) -> real64 {
        r * self.evaluate_ratio(plane, r * r)
    }

    /// `EvaluateRatio (plane, r2)` — `r2` is the square of the radius.
    pub fn evaluate_ratio(&self, plane: uint32, mut r2: real64) -> real64 {
        debug_assert!((plane as usize) < kMaxColorPlanes as usize, "Bad plane");

        let min_valid_radius = self.valid_range[plane as usize][0];
        let max_valid_radius = self.valid_range[plane as usize][1];

        let min_valid_radius_sqr = min_valid_radius * min_valid_radius;
        let max_valid_radius_sqr = max_valid_radius * max_valid_radius;

        let data = &self.data[plane as usize];

        // Limit r2 to valid range.

        r2 = pin_real64(min_valid_radius_sqr, r2, max_valid_radius_sqr);

        let r = r2.sqrt();

        let mut poly = data[(Self::kMaxTerms - 1) as usize];

        for i in (0..(Self::kMaxTerms - 1) as i32).rev() {
            poly = (poly * r) + data[i as usize];
        }

        if self.use_reciprocal {
            1.0 / poly
        } else {
            poly
        }
    }
}

impl Default for DngWarpParamsRadial {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// dng_warp_params_rectilinear.
// ---------------------------------------------------------------------------

/// Port of `dng_warp_params_rectilinear` — pinhole perspective camera model
/// with radial and tangential (decentering) correction.
#[derive(Debug, Clone)]
pub struct DngWarpParamsRectilinear {
    /// `fPlanes` (base class member).
    pub planes_count: uint32,

    /// `fCenter` (base class member).
    pub center_point: DngPointReal64,

    /// `fRadParams`.
    pub rad_params: DngWarpParamsRadial,

    /// `fTanParams [plane]` — 2-entry tangential vectors.
    pub tan_params: Vec<DngVector>,
}

impl DngWarpParamsRectilinear {
    /// `dng_warp_params_rectilinear ()` — empty (invalid).
    pub fn new() -> Self {
        DngWarpParamsRectilinear {
            planes_count: 1,
            center_point: DngPointReal64::new(0.5, 0.5),
            rad_params: DngWarpParamsRadial::new(),
            tan_params: vec![DngVector::with_count(2).unwrap(); kMaxColorPlanes as usize],
        }
    }

    /// `dng_warp_params_rectilinear (planes, radParams, tanParams, center)`.
    pub fn with_params(
        planes: uint32,
        rad_params: &DngWarpParamsRadial,
        tan_params: &[DngVector],
        center: &DngPointReal64,
    ) -> Self {
        let mut result = DngWarpParamsRectilinear {
            planes_count: planes,
            center_point: *center,
            rad_params: rad_params.clone(),
            tan_params: vec![DngVector::with_count(2).unwrap(); kMaxColorPlanes as usize],
        };

        for i in 0..planes as usize {
            result.tan_params[i] = tan_params[i].clone();
        }

        result
    }
}

impl Default for DngWarpParamsRectilinear {
    fn default() -> Self {
        Self::new()
    }
}

impl DngWarpParams for DngWarpParamsRectilinear {
    fn planes(&self) -> uint32 {
        self.planes_count
    }

    fn set_planes(&mut self, planes: uint32) {
        self.planes_count = planes;
    }

    fn center(&self) -> DngPointReal64 {
        self.center_point
    }

    fn is_rad_nop(&self, plane: uint32) -> bool {
        debug_assert!(plane < self.planes_count, "plane out of range.");

        self.rad_params.is_nop(plane)
    }

    fn is_tan_nop(&self, plane: uint32) -> bool {
        debug_assert!(plane < self.planes_count, "plane out of range.");

        let t = &self.tan_params[plane as usize];

        t.get(0) == 0.0 && t.get(1) == 0.0
    }

    fn is_valid(&self) -> bool {
        for plane in 0..self.planes_count {
            if !self.rad_params.is_valid(plane) {
                return false;
            }

            if self.tan_params[plane as usize].count() < 2 {
                return false;
            }
        }

        // Base-class validity.

        if self.planes_count < 1 || self.planes_count > kMaxColorPlanes {
            return false;
        }

        if self.center_point.h < 0.0
            || self.center_point.h > 1.0
            || self.center_point.v < 0.0
            || self.center_point.v > 1.0
        {
            return false;
        }

        true
    }

    fn propagate_to_all_planes(&mut self, total_planes: uint32) {
        for plane in self.planes_count..total_planes {
            self.rad_params.data[plane as usize] = self.rad_params.data[0].clone();

            self.rad_params.valid_range[plane as usize] = self.rad_params.valid_range[0];

            self.tan_params[plane as usize] = self.tan_params[0].clone();
        }

        self.planes_count = total_planes;
    }

    fn evaluate(&self, plane: uint32, x: real64) -> real64 {
        self.rad_params.evaluate(plane, x)
    }

    fn evaluate_ratio(&self, plane: uint32, r2: real64) -> real64 {
        self.rad_params.evaluate_ratio(plane, r2)
    }

    fn evaluate_tangential(
        &self,
        plane: uint32,
        r2: real64,
        diff: &DngPointReal64,
        diff2: &DngPointReal64,
    ) -> DngPointReal64 {
        let kt0 = self.tan_params[plane as usize].get(0);
        let kt1 = self.tan_params[plane as usize].get(1);

        let dh = diff.h;
        let dv = diff.v;

        let dhdh = diff2.h;
        let dvdv = diff2.v;

        // C++ returns dng_point_real64 (v, h).

        DngPointReal64::new(
            kt0 * (r2 + 2.0 * dvdv) + (2.0 * kt1 * dh * dv), // v
            kt1 * (r2 + 2.0 * dhdh) + (2.0 * kt0 * dh * dv), // h
        )
    }

    fn max_src_radius_gap(&self, max_dst_gap: real64) -> real64 {
        let mut max_src_gap = 0.0;

        for plane in 0..self.planes_count {
            // Find minimum and maximum ratio by discretizing over r2.

            const K_SAMPLES: int32 = 65000;

            let norm = 1.0 / (K_SAMPLES - 1) as real64;

            let mut min_ratio = 1.0;
            let mut max_ratio = 1.0;

            for i in 1..K_SAMPLES {
                let r2 = i as real64 * norm;

                let s = self.evaluate_ratio(plane, r2);

                min_ratio = min_real64(s, min_ratio);
                max_ratio = max_real64(s, max_ratio);
            }

            let max_gap_scale_factor = max_ratio / min_ratio;

            let src_gap = max_gap_scale_factor * max_dst_gap;

            max_src_gap = max_real64(max_src_gap, src_gap);
        }

        max_src_gap
    }

    fn max_src_tan_gap(
        &self,
        min_dst: &DngPointReal64,
        max_dst: &DngPointReal64,
    ) -> DngPointReal64 {
        let v = [min_dst.v, max_dst.v, 0.0];
        let h = [min_dst.h, max_dst.h, 0.0];

        let mut max_gap = DngPointReal64::new(0.0, 0.0);

        for plane in 0..self.planes_count {
            let mut h_min = f64::MAX;
            let mut h_max = f64::MIN;

            let mut v_min = f64::MAX;
            let mut v_max = f64::MIN;

            for i in 0..3 {
                for j in 0..3 {
                    let dst_diff = DngPointReal64::new(v[i], h[j]);

                    let src_diff = self.evaluate_tangential2(plane, &dst_diff);

                    h_min = min_real64(h_min, src_diff.h);
                    h_max = max_real64(h_max, src_diff.h);

                    v_min = min_real64(v_min, src_diff.v);
                    v_max = max_real64(v_max, src_diff.v);
                }
            }

            let h_gap = h_max - h_min;
            let v_gap = v_max - v_min;

            max_gap.h = max_real64(max_gap.h, h_gap);
            max_gap.v = max_real64(max_gap.v, v_gap);
        }

        max_gap
    }

    fn safe_min_ratio(&self) -> real64 {
        0.5
    }

    fn safe_max_ratio(&self) -> real64 {
        2.0
    }
}

// ---------------------------------------------------------------------------
// dng_warp_params_fisheye.
// ---------------------------------------------------------------------------

/// Port of `dng_warp_params_fisheye` — fisheye camera model (radial only).
#[derive(Debug, Clone)]
pub struct DngWarpParamsFisheye {
    /// `fPlanes` (base class member).
    pub planes_count: uint32,

    /// `fCenter` (base class member).
    pub center_point: DngPointReal64,

    /// `fRadParams [plane]` — 4-entry radial vectors.
    pub rad_params: Vec<DngVector>,
}

impl DngWarpParamsFisheye {
    /// `dng_warp_params_fisheye ()` — empty (invalid).
    pub fn new() -> Self {
        DngWarpParamsFisheye {
            planes_count: 1,
            center_point: DngPointReal64::new(0.5, 0.5),
            rad_params: vec![DngVector::with_count(4).unwrap(); kMaxColorPlanes as usize],
        }
    }

    /// `dng_warp_params_fisheye (planes, radParams, center)`.
    pub fn with_params(
        planes: uint32,
        rad_params: &[DngVector],
        center: &DngPointReal64,
    ) -> Self {
        let mut result = DngWarpParamsFisheye {
            planes_count: planes,
            center_point: *center,
            rad_params: vec![DngVector::with_count(4).unwrap(); kMaxColorPlanes as usize],
        };

        for i in 0..planes as usize {
            result.rad_params[i] = rad_params[i].clone();
        }

        result
    }
}

impl Default for DngWarpParamsFisheye {
    fn default() -> Self {
        Self::new()
    }
}

impl DngWarpParams for DngWarpParamsFisheye {
    fn planes(&self) -> uint32 {
        self.planes_count
    }

    fn set_planes(&mut self, planes: uint32) {
        self.planes_count = planes;
    }

    fn center(&self) -> DngPointReal64 {
        self.center_point
    }

    fn is_rad_nop(&self, _plane: uint32) -> bool {
        false
    }

    fn is_tan_nop(&self, _plane: uint32) -> bool {
        true
    }

    fn is_valid(&self) -> bool {
        for plane in 0..self.planes_count {
            if self.rad_params[plane as usize].count() != 4 {
                return false;
            }
        }

        // Base-class validity.

        if self.planes_count < 1 || self.planes_count > kMaxColorPlanes {
            return false;
        }

        if self.center_point.h < 0.0
            || self.center_point.h > 1.0
            || self.center_point.v < 0.0
            || self.center_point.v > 1.0
        {
            return false;
        }

        true
    }

    fn propagate_to_all_planes(&mut self, total_planes: uint32) {
        for plane in self.planes_count..total_planes {
            self.rad_params[plane as usize] = self.rad_params[0].clone();
        }

        self.planes_count = total_planes;
    }

    fn evaluate(&self, plane: uint32, r: real64) -> real64 {
        let t = r.atan();

        let k = &self.rad_params[plane as usize];

        let t2 = t * t;

        t * (k.get(0) + t2 * (k.get(1) + t2 * (k.get(2) + t2 * k.get(3))))
    }

    fn evaluate_ratio(&self, plane: uint32, r_sqr: real64) -> real64 {
        let eps = 1.0e-12;

        if r_sqr < eps {
            // r is very close to zero.

            return 1.0;
        }

        let r = r_sqr.sqrt();

        self.evaluate(plane, r) / r
    }

    fn evaluate_tangential(
        &self,
        _plane: uint32,
        _r2: real64,
        _diff: &DngPointReal64,
        _diff2: &DngPointReal64,
    ) -> DngPointReal64 {
        // This fisheye model does not support tangential warping.
        // C++ ThrowProgramError; the port panics (dng_assert semantics).

        panic!("dng_warp_params_fisheye::EvaluateTangential called");
    }

    fn max_src_radius_gap(&self, max_dst_gap: real64) -> real64 {
        //
        // Let f (r) be the radius warp function. Consider the function
        //
        //   gap (r) = f (r + maxDstGap) - f (r)
        //
        // We wish to maximize gap (r) over the domain [0, 1 - maxDstGap].

        let mut max_src_gap = 0.0;

        debug_assert!(max_dst_gap > 0.0, "maxDstGap must be positive.");

        let k_max_value = 1.0 - max_dst_gap;

        const K_STEPS: uint32 = 128;

        let k_norm = k_max_value / (K_STEPS - 1) as real64;

        for plane in 0..self.planes_count {
            for i in 0..K_STEPS {
                let tt = i as real64 * k_norm;

                let gap = self.evaluate(plane, tt + max_dst_gap) - self.evaluate(plane, tt);

                max_src_gap = max_real64(max_src_gap, gap);
            }
        }

        max_src_gap
    }

    fn max_src_tan_gap(
        &self,
        _min_dst: &DngPointReal64,
        _max_dst: &DngPointReal64,
    ) -> DngPointReal64 {
        // This fisheye model does not support tangential distortion.

        DngPointReal64::new(0.0, 0.0)
    }

    fn safe_min_ratio(&self) -> real64 {
        0.2
    }

    fn safe_max_ratio(&self) -> real64 {
        5.0
    }
}

// ---------------------------------------------------------------------------
// dng_filter_warp.
// ---------------------------------------------------------------------------

/// Port of `dng_filter_warp` — the (serial) warp filter task.
struct DngFilterWarp {
    src_image: ImageRef,
    dst_image: ImageRef,

    params: Box<dyn DngWarpParams>,

    center: DngPointReal64,

    weights: DngResampleWeights2d,

    norm_radius: real64,
    inv_norm_radius: real64,

    is_rad_nop: bool,
    is_tan_nop: bool,

    pixel_scale_v: real64,
    pixel_scale_v_inv: real64,
}

impl DngFilterWarp {
    fn new(
        src_image: ImageRef,
        dst_image: ImageRef,
        negative: &DngNegative,
        params: Box<dyn DngWarpParams>,
    ) -> DngResult<Self> {
        let pixel_scale_v = 1.0 / negative.pixel_aspect_ratio();
        let pixel_scale_v_inv = 1.0 / pixel_scale_v;

        let is_rad_nop = params.is_rad_nop_all();
        let is_tan_nop = params.is_tan_nop_all();

        // At least one set of params must do something interesting.

        if is_rad_nop && is_tan_nop {
            return Err(program_error());
        }

        // Make sure the warp params are valid for this negative.

        if !params.is_valid_for_negative(negative) {
            return Err(bad_format());
        }

        // Compute center.

        let bounds = src_image.read().unwrap().bounds();

        let center = DngPointReal64::new(
            lerp_real64(bounds.l as real64, bounds.r as real64, params.center().h),
            lerp_real64(bounds.t as real64, bounds.b as real64, params.center().v),
        );

        // Compute max pixel radius and derive various normalized radius
        // values.  Note that when computing the max pixel radius, we must
        // take into account the pixel aspect ratio.

        let norm_radius = {
            let mut square_bounds = bounds;

            square_bounds.b = square_bounds.t
                + round_int32_f64(pixel_scale_v * square_bounds.h() as real64);

            let square_center = DngPointReal64::new(
                lerp_real64(
                    square_bounds.t as real64,
                    square_bounds.b as real64,
                    params.center().v,
                ),
                lerp_real64(
                    square_bounds.l as real64,
                    square_bounds.r as real64,
                    params.center().h,
                ),
            );

            max_distance_point_to_rect(square_center, DngRectReal64::from_rect(square_bounds))
        };

        let mut filter = DngFilterWarp {
            src_image,
            dst_image,
            params,
            center,
            weights: DngResampleWeights2d::new(),
            norm_radius,
            inv_norm_radius: 1.0 / norm_radius,
            is_rad_nop,
            is_tan_nop,
            pixel_scale_v,
            pixel_scale_v_inv,
        };

        // Propagate warp params to other planes.

        let dst_planes = filter.dst_image.read().unwrap().planes();

        filter.params.propagate_to_all_planes(dst_planes);

        Ok(filter)
    }

    /// `Initialize (host)` — make resample weights.
    fn initialize(&mut self) -> DngResult<()> {
        let kernel: &dyn DngResampleFunction = DngResampleBicubic::get();

        self.weights.initialize(kernel)
    }

    /// `GetSrcPixelPosition (dst, plane)`.
    fn get_src_pixel_position(&self, dst: &DngPointReal64, plane: uint32) -> DngPointReal64 {
        let diff = DngPointReal64::new(dst.v - self.center.v, dst.h - self.center.h);

        let diff_norm = DngPointReal64::new(
            diff.v * self.inv_norm_radius,
            diff.h * self.inv_norm_radius,
        );

        let diff_norm_scaled = DngPointReal64::new(diff_norm.v * self.pixel_scale_v, diff_norm.h);

        let diff_norm_sqr = DngPointReal64::new(
            diff_norm_scaled.v * diff_norm_scaled.v,
            diff_norm_scaled.h * diff_norm_scaled.h,
        );

        let rr = min_real64(diff_norm_sqr.v + diff_norm_sqr.h, 1.0);

        let d_src;

        if self.is_tan_nop {
            // Radial only.

            let ratio = self.params.evaluate_ratio(plane, rr);

            d_src = DngPointReal64::new(diff.v * ratio, diff.h * ratio);
        } else if self.is_rad_nop {
            // Tangential only.

            let tan = self
                .params
                .evaluate_tangential(plane, rr, &diff_norm_scaled, &diff_norm_sqr);

            d_src = DngPointReal64::new(
                diff.v + (self.norm_radius * tan.v * self.pixel_scale_v_inv),
                diff.h + (self.norm_radius * tan.h),
            );
        } else {
            // Radial and tangential.

            let ratio = self.params.evaluate_ratio(plane, rr);

            let tan = self
                .params
                .evaluate_tangential(plane, rr, &diff_norm_scaled, &diff_norm_sqr);

            d_src = DngPointReal64::new(
                self.norm_radius * (diff_norm.v * ratio + tan.v * self.pixel_scale_v_inv),
                self.norm_radius * (diff_norm.h * ratio + tan.h),
            );
        }

        DngPointReal64::new(self.center.v + d_src.v, self.center.h + d_src.h)
    }

    /// `SrcArea (dstArea)` — walk the boundary of dstArea, map each pixel to
    /// the uncorrected src position, and return the containing rect.
    fn src_area(&self, dst_area: DngRect) -> DngRect {
        let mut x_min = int32::MAX;
        let mut x_max = int32::MIN;
        let mut y_min = int32::MAX;
        let mut y_max = int32::MIN;

        let dst_planes = self.dst_image.read().unwrap().planes();

        for plane in 0..dst_planes {
            // Top and bottom edges.

            for c in dst_area.l..dst_area.r {
                // Top edge.

                {
                    let dst = DngPointReal64::new(dst_area.t as real64, c as real64);

                    let src = self.get_src_pixel_position(&dst, plane);

                    let y = round_int32_f64(src.v.floor());

                    y_min = min_int32(y_min, y);
                }

                // Bottom edge.

                {
                    let dst = DngPointReal64::new((dst_area.b - 1) as real64, c as real64);

                    let src = self.get_src_pixel_position(&dst, plane);

                    let y = round_int32_f64(src.v.ceil());

                    y_max = max_int32(y_max, y);
                }
            }

            // Left and right edges.

            for r in dst_area.t..dst_area.b {
                // Left edge.

                {
                    let dst = DngPointReal64::new(r as real64, dst_area.l as real64);

                    let src = self.get_src_pixel_position(&dst, plane);

                    let x = round_int32_f64(src.h.floor());

                    x_min = min_int32(x_min, x);
                }

                // Right edge.

                {
                    let dst = DngPointReal64::new(r as real64, (dst_area.r - 1) as real64);

                    let src = self.get_src_pixel_position(&dst, plane);

                    let x = round_int32_f64(src.h.ceil());

                    x_max = max_int32(x_max, x);
                }
            }
        }

        // Pad each side by filter radius.

        let pad = self.weights.radius() as int32;

        let x_min = x_min.saturating_sub(pad);
        let y_min = y_min.saturating_sub(pad);
        let x_max = x_max.saturating_add(pad);
        let y_max = y_max.saturating_add(pad);

        let x_max = x_max.saturating_add(1);
        let y_max = y_max.saturating_add(1);

        let src_area = DngRect::from_corners(y_min, x_min, y_max, x_max);

        // Limit to src image area.

        src_area.and_(self.src_image.read().unwrap().bounds())
    }
}

impl DngAreaTask for DngFilterWarp {
    fn name(&self) -> &str {
        "dng_filter_warp"
    }

    fn repeating_tile1(&self) -> DngRect {
        self.src_image.read().unwrap().repeating_tile()
    }

    fn repeating_tile2(&self) -> DngRect {
        self.dst_image.read().unwrap().repeating_tile()
    }

    fn process(
        &mut self,
        _thread_index: uint32,
        dst_area: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn dng_core::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        // Source area covering all warped pixels of dst_area.

        let src_area = self.src_area(dst_area);

        // Prepare resample constants.

        let w_count = self.weights.width() as usize;

        let src_offset = self.weights.offset();

        let num_subsamples = kResampleSubsampleCount2D as real64;

        let src_image_area = DngRectReal64::from_rect(self.src_image.read().unwrap().bounds());

        let h_min = src_area.l;
        let h_max = src_area.r - w_count as int32 - 1;

        let v_min = src_area.t;
        let v_max = src_area.b - w_count as int32 - 1;

        if h_max < h_min || v_max < v_min {
            return Err(bad_format()); // "Empty source area in dng_filter_warp."
        }

        // Source tile buffer (float).

        let src_planes = self.src_image.read().unwrap().planes();
        let src_pixel_type = ttFloat;

        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            0,
            src_planes,
            src_pixel_type,
            pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(src_area, src_planes, src_pixel_type)?,
        )?;

        {
            let src = self.src_image.read().unwrap();
            src.get(
                &mut src_buffer,
                dng_core::dng_image::EdgeOption::EdgeRepeat,
                1,
                1,
            )?;
        }

        // Destination tile buffer (float).

        let dst_planes = self.dst_image.read().unwrap().planes();

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            dst_area,
            0,
            dst_planes,
            ttFloat,
            pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(dst_area, dst_planes, ttFloat)?,
        )?;

        // Warp each plane.

        let src_row_step = src_buffer.row_step as usize;
        let dst_row_step = dst_buffer.row_step as usize;

        let src_f32 = src_buffer.as_f32_slice().unwrap().to_vec();
        let dst_len = dst_buffer.as_f32_slice().unwrap().len();
        let mut dst_f32 = vec![0.0f32; dst_len];

        for plane in 0..dst_planes as usize {
            let d_base0 =
                dst_buffer.pixel_element_index_public(dst_area.t, dst_area.l, plane as uint32)?;

            for dst_row in dst_area.t..dst_area.b {
                let d_row_base = d_base0 + ((dst_row - dst_area.t) as usize) * dst_row_step;

                for (dst_col_i, dst_col) in (dst_area.l..dst_area.r).enumerate() {
                    // Get destination (corrected) pixel position.

                    let d_pos = DngPointReal64::new(dst_row as real64, dst_col as real64);

                    // Warp to source (uncorrected) pixel position.

                    let mut s_pos = self.get_src_pixel_position(&d_pos, plane as uint32);

                    // Limit to source image area.

                    s_pos.h = max_real64(s_pos.h, src_image_area.l);
                    s_pos.h = min_real64(s_pos.h, src_image_area.r - 1.0);
                    s_pos.v = max_real64(s_pos.v, src_image_area.t);
                    s_pos.v = min_real64(s_pos.v, src_image_area.b - 1.0);

                    // Decompose into integer and fractional parts.
                    // C++ uses ConvertDoubleToInt32 (a truncating cast), so
                    // the fractional subsample index must truncate, not
                    // round: 31.97 must stay 31, not become 32.

                    let mut s_int = DngPoint::new(
                        round_int32_f64(s_pos.v.floor()),
                        round_int32_f64(s_pos.h.floor()),
                    );

                    let mut s_fct = DngPoint::new(
                        ((s_pos.v - s_int.v as real64) * num_subsamples) as int32,
                        ((s_pos.h - s_int.h as real64) * num_subsamples) as int32,
                    );

                    // Add resample offset.

                    s_int.v += src_offset;
                    s_int.h += src_offset;

                    // Clip.

                    if s_int.h < h_min {
                        s_int.h = h_min;
                        s_fct.h = 0;
                    } else if s_int.h > h_max {
                        s_int.h = h_max;
                        s_fct.h = 0;
                    }

                    if s_int.v < v_min {
                        s_int.v = v_min;
                        s_fct.v = 0;
                    } else if s_int.v > v_max {
                        s_int.v = v_max;
                        s_fct.v = 0;
                    }

                    // Perform 2D resample.

                    let w = self.weights.weights32_at(s_fct.v as uint32, s_fct.h as uint32)?;

                    let s_base =
                        src_buffer.pixel_element_index_public(s_int.v, s_int.h, plane as uint32)?;

                    let mut total: real32 = 0.0;

                    for i in 0..w_count {
                        for j in 0..w_count {
                            total += w[i * w_count + j]
                                * src_f32[s_base + i * src_row_step + j];
                        }
                    }

                    // Store final pixel value.

                    let d_index = d_row_base + dst_col_i;

                    if d_index < dst_f32.len() {
                        dst_f32[d_index] = pin_real32(0.0, total, 1.0);
                    }
                }
            }
        }

        // Write back.

        dst_buffer
            .as_f32_slice_mut()
            .unwrap()
            .copy_from_slice(&dst_f32);

        let dst = self.dst_image.read().unwrap();
        dst.put(&dst_buffer)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_BaseWarpRectilinear.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_BaseWarpRectilinear` — abstract base for
/// WarpRectilinear / WarpRectilinear2.
pub struct DngOpcodeBaseWarpRectilinear {
    base: DngOpcodeBase,

    /// `fWarpParams`.
    pub warp_params: DngWarpParamsRectilinear,
}

impl DngOpcodeBaseWarpRectilinear {
    /// Constructor with explicit params (C++ protected ctor 1).
    pub fn new_with_params(
        opcode_tag: uint32,
        min_version: uint32,
        params: DngWarpParamsRectilinear,
        flags: uint32,
    ) -> DngResult<Self> {
        if !params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeBaseWarpRectilinear {
            base: DngOpcodeBase::new(opcode_tag, min_version, flags),
            warp_params: params,
        })
    }

    /// Constructor reading the opcode header from the stream (protected
    /// ctor 2; parameter reading is completed by the concrete subclass).
    pub fn new_from_stream(opcode_tag: uint32, stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(opcode_tag, stream)?;

        let _ = stream;

        Ok(DngOpcodeBaseWarpRectilinear {
            base,
            warp_params: DngWarpParamsRectilinear::new(),
        })
    }

    /// `HasDistort ()`.
    pub fn has_distort(&self) -> bool {
        for plane in 0..self.warp_params.planes_count {
            if self.warp_params.is_nop(plane) {
                return false;
            }
        }

        true
    }

    /// `HasLateralCA ()`.
    pub fn has_lateral_ca(&self) -> bool {
        if self.warp_params.planes_count <= 1 {
            return false;
        }

        for plane in 0..self.warp_params.planes_count {
            if !self.warp_params.is_nop(plane) {
                return true;
            }
        }

        false
    }

    /// `Apply (host, negative, image)` — warp the image into a new image.
    pub fn apply_warp(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        // Allocate destination image.

        let (bounds, planes, pixel_type) = {
            let img = image.read().unwrap();
            (img.bounds(), img.planes(), img.pixel_type())
        };

        let dst = dng_core::dng_simple_image::DngSimpleImage::new(bounds, planes, pixel_type)?;

        let dst_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(dst));

        // Warp the image.

        let mut filter = DngFilterWarp::new(
            image.clone(),
            dst_ref.clone(),
            negative,
            Box::new(self.warp_params.clone()),
        )?;

        filter.initialize()?;

        host.perform_area_task(&mut filter, bounds)?;

        // Return the new image.

        *image = dst_ref;

        Ok(())
    }
}

impl DngOpcode for DngOpcodeBaseWarpRectilinear {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `IsNOP ()`.
    fn is_nop(&self) -> bool {
        self.warp_params.is_nop_all()
    }

    /// `IsValidForNegative (negative)`.
    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        self.warp_params.is_valid_for_negative(negative)
    }

    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_warp(host, negative, image)
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_WarpRectilinear.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_WarpRectilinear` (DNG 1.3).
pub struct DngOpcodeWarpRectilinear {
    inner: DngOpcodeBaseWarpRectilinear,
}

impl DngOpcodeWarpRectilinear {
    /// `ParamBytes (planes)`.
    pub fn param_bytes(planes: uint32) -> uint32 {
        (1 * 4) +          // Number of planes.
        (6 * 8 * planes) + // Warp coefficients.
        (2 * 8)            // Optical center.
    }

    /// `dng_opcode_WarpRectilinear (params, flags)`.
    pub fn new_with_params(params: DngWarpParamsRectilinear, flags: uint32) -> DngResult<Self> {
        Ok(DngOpcodeWarpRectilinear {
            inner: DngOpcodeBaseWarpRectilinear::new_with_params(
                dngOpcode_WarpRectilinear,
                dngVersion_1_3_0_0,
                params,
                flags,
            )?,
        })
    }

    /// `dng_opcode_WarpRectilinear (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let mut inner =
            DngOpcodeBaseWarpRectilinear::new_from_stream(dngOpcode_WarpRectilinear, stream)?;

        // Grab the size in bytes.

        let bytes = stream.get_uint32()?;

        // Grab the number of planes to warp.

        inner.warp_params.planes_count = stream.get_uint32()?;

        // Verify number of planes.

        if inner.warp_params.planes_count == 0 || inner.warp_params.planes_count > kMaxColorPlanes
        {
            return Err(bad_format());
        }

        // Verify the size.

        if bytes != Self::param_bytes(inner.warp_params.planes_count) {
            return Err(bad_format());
        }

        // Read warp parameters for each plane.

        for plane in 0..inner.warp_params.planes_count as usize {
            inner.warp_params.rad_params.data[plane][0] = stream.get_real64()?; // offset
            inner.warp_params.rad_params.data[plane][2] = stream.get_real64()?; // r^2
            inner.warp_params.rad_params.data[plane][4] = stream.get_real64()?; // r^4
            inner.warp_params.rad_params.data[plane][6] = stream.get_real64()?; // r^6

            // Tan params.

            let mut tan = inner.warp_params.tan_params[plane].clone();
            tan.set(0, stream.get_real64()?);
            tan.set(1, stream.get_real64()?);
            inner.warp_params.tan_params[plane] = tan;
        }

        // Read the image center.

        inner.warp_params.center_point.h = stream.get_real64()?;
        inner.warp_params.center_point.v = stream.get_real64()?;

        if !inner.warp_params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeWarpRectilinear { inner })
    }

    /// `PutData (stream)`.
    pub fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let bytes = Self::param_bytes(self.inner.warp_params.planes_count);

        stream.put_uint32(bytes)?;

        stream.put_uint32(self.inner.warp_params.planes_count)?;

        for plane in 0..self.inner.warp_params.planes_count as usize {
            debug_assert!(
                self.inner
                    .warp_params
                    .rad_params
                    .compatible_with_warp_rectilinear_1_3(plane as uint32),
                "Incompatible WarpRectilinear DNG 1.3 params"
            );

            stream.put_real64(self.inner.warp_params.rad_params.data[plane][0])?; // offset
            stream.put_real64(self.inner.warp_params.rad_params.data[plane][2])?; // r^2
            stream.put_real64(self.inner.warp_params.rad_params.data[plane][4])?; // r^4
            stream.put_real64(self.inner.warp_params.rad_params.data[plane][6])?; // r^6

            stream.put_real64(self.inner.warp_params.tan_params[plane].get(0))?;
            stream.put_real64(self.inner.warp_params.tan_params[plane].get(1))?;
        }

        stream.put_real64(self.inner.warp_params.center_point.h)?;
        stream.put_real64(self.inner.warp_params.center_point.v)?;

        Ok(())
    }
}

impl DngOpcode for DngOpcodeWarpRectilinear {
    fn base(&self) -> &DngOpcodeBase {
        &self.inner.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.inner.base
    }

    fn is_nop(&self) -> bool {
        self.inner.is_nop()
    }

    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        self.inner.is_valid_for_negative(negative)
    }

    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        self.put_data(stream)
    }

    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.inner.apply(host, negative, image)
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_WarpRectilinear2.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_WarpRectilinear2` (DNG 1.6).
///
/// Format:
///
/// ```text
/// uint32 planes
/// for each plane:
///     real64 kr0..kr14 (15 radial terms)
///     real64 kt0, kt1
///     real64 min_valid_radius
///     real64 max_valid_radius
/// real64 cx
/// real64 cy
/// uint32 useReciprocal
/// ```
pub struct DngOpcodeWarpRectilinear2 {
    inner: DngOpcodeBaseWarpRectilinear,
}

impl DngOpcodeWarpRectilinear2 {
    /// `ParamBytes (planes)`.
    pub fn param_bytes(planes: uint32) -> uint32 {
        let num_radial_terms = DngWarpParamsRadial::kMaxTerms;
        let num_valid_range_terms = 2;
        let num_tangential_terms = 2;

        let warp_terms = num_radial_terms + num_valid_range_terms + num_tangential_terms;

        (1 * 4) +                     // Number of planes
        (warp_terms * 8 * planes) +   // Warp coefficients
        (2 * 8) +                     // Optical center
        (1 * 4)                       // Reciprocal
    }

    /// `dng_opcode_WarpRectilinear2 (params, flags)`.
    pub fn new_with_params(params: DngWarpParamsRectilinear, flags: uint32) -> DngResult<Self> {
        Ok(DngOpcodeWarpRectilinear2 {
            inner: DngOpcodeBaseWarpRectilinear::new_with_params(
                dngOpcode_WarpRectilinear2,
                dngVersion_1_6_0_0,
                params,
                flags,
            )?,
        })
    }

    /// `dng_opcode_WarpRectilinear2 (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let mut inner =
            DngOpcodeBaseWarpRectilinear::new_from_stream(dngOpcode_WarpRectilinear2, stream)?;

        // Grab the size in bytes.

        let bytes = stream.get_uint32()?;

        // Grab the number of planes to warp.

        inner.warp_params.planes_count = stream.get_uint32()?;

        // Verify number of planes.

        if inner.warp_params.planes_count == 0 || inner.warp_params.planes_count > kMaxColorPlanes
        {
            return Err(bad_format());
        }

        // Verify the size.

        if bytes != Self::param_bytes(inner.warp_params.planes_count) {
            return Err(bad_format());
        }

        // Read warp parameters for each plane.

        for plane in 0..inner.warp_params.planes_count as usize {
            // Radial terms.

            for i in 0..DngWarpParamsRadial::kMaxTerms as usize {
                inner.warp_params.rad_params.data[plane][i] = stream.get_real64()?;
            }

            // Tangential terms.

            let mut tan = inner.warp_params.tan_params[plane].clone();
            tan.set(0, stream.get_real64()?);
            tan.set(1, stream.get_real64()?);
            inner.warp_params.tan_params[plane] = tan;

            // Min & max valid radius.

            inner.warp_params.rad_params.valid_range[plane][0] = stream.get_real64()?;
            inner.warp_params.rad_params.valid_range[plane][1] = stream.get_real64()?;
        }

        // Read the image center.

        inner.warp_params.center_point.h = stream.get_real64()?;
        inner.warp_params.center_point.v = stream.get_real64()?;

        // Read the reciprocal radial flag.

        inner.warp_params.rad_params.use_reciprocal = stream.get_uint32()? != 0;

        if !inner.warp_params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeWarpRectilinear2 { inner })
    }

    /// `PutData (stream)`.
    pub fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        // Write the variable tag size (byte count).

        let bytes = Self::param_bytes(self.inner.warp_params.planes_count);

        stream.put_uint32(bytes)?;

        // Write # of planes.

        stream.put_uint32(self.inner.warp_params.planes_count)?;

        // Write coefficient data for each plane.

        for plane in 0..self.inner.warp_params.planes_count as usize {
            // Radial.

            for i in 0..DngWarpParamsRadial::kMaxTerms as usize {
                stream.put_real64(self.inner.warp_params.rad_params.data[plane][i])?;
            }

            // Tangential.

            stream.put_real64(self.inner.warp_params.tan_params[plane].get(0))?;
            stream.put_real64(self.inner.warp_params.tan_params[plane].get(1))?;

            // Min & max valid radius.

            stream.put_real64(self.inner.warp_params.rad_params.valid_range[plane][0])?;
            stream.put_real64(self.inner.warp_params.rad_params.valid_range[plane][1])?;
        }

        // Optical center.

        stream.put_real64(self.inner.warp_params.center_point.h)?;
        stream.put_real64(self.inner.warp_params.center_point.v)?;

        // Reciprocal radial model.

        stream.put_uint32(if self.inner.warp_params.rad_params.use_reciprocal {
            1
        } else {
            0
        })?;

        Ok(())
    }
}

impl DngOpcode for DngOpcodeWarpRectilinear2 {
    fn base(&self) -> &DngOpcodeBase {
        &self.inner.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.inner.base
    }

    fn is_nop(&self) -> bool {
        self.inner.is_nop()
    }

    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        self.inner.is_valid_for_negative(negative)
    }

    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        self.put_data(stream)
    }

    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.inner.apply(host, negative, image)
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_WarpFisheye.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_WarpFisheye`.
pub struct DngOpcodeWarpFisheye {
    base: DngOpcodeBase,

    /// `fWarpParams`.
    pub warp_params: DngWarpParamsFisheye,
}

impl DngOpcodeWarpFisheye {
    /// `ParamBytes (planes)`.
    pub fn param_bytes(planes: uint32) -> uint32 {
        (1 * 4) +         // Number of planes.
        (4 * 8 * planes) + // Warp coefficients.
        (2 * 8)            // Optical center.
    }

    /// `dng_opcode_WarpFisheye (params, flags)`.
    pub fn new_with_params(params: DngWarpParamsFisheye, flags: uint32) -> DngResult<Self> {
        if !params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeWarpFisheye {
            base: DngOpcodeBase::new(dngOpcode_WarpFisheye, dngVersion_1_3_0_0, flags),
            warp_params: params,
        })
    }

    /// `dng_opcode_WarpFisheye (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_WarpFisheye, stream)?;

        let mut warp_params = DngWarpParamsFisheye::new();

        // Grab the size in bytes.

        let bytes = stream.get_uint32()?;

        // Grab the number of planes to warp.

        warp_params.planes_count = stream.get_uint32()?;

        // Verify number of planes.

        if warp_params.planes_count == 0 || warp_params.planes_count > kMaxColorPlanes {
            return Err(bad_format());
        }

        // Verify the size.

        if bytes != Self::param_bytes(warp_params.planes_count) {
            return Err(bad_format());
        }

        // Read warp parameters for each plane.

        for plane in 0..warp_params.planes_count as usize {
            let mut rad = warp_params.rad_params[plane].clone();

            rad.set(0, stream.get_real64()?);
            rad.set(1, stream.get_real64()?);
            rad.set(2, stream.get_real64()?);
            rad.set(3, stream.get_real64()?);

            warp_params.rad_params[plane] = rad;
        }

        // Read the image center.

        warp_params.center_point.h = stream.get_real64()?;
        warp_params.center_point.v = stream.get_real64()?;

        if !warp_params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeWarpFisheye {
            base,
            warp_params,
        })
    }

    /// `PutData (stream)`.
    pub fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let bytes = Self::param_bytes(self.warp_params.planes_count);

        stream.put_uint32(bytes)?;

        // Write the number of planes.

        stream.put_uint32(self.warp_params.planes_count)?;

        // Write the warp coefficients.

        for plane in 0..self.warp_params.planes_count as usize {
            stream.put_real64(self.warp_params.rad_params[plane].get(0))?;
            stream.put_real64(self.warp_params.rad_params[plane].get(1))?;
            stream.put_real64(self.warp_params.rad_params[plane].get(2))?;
            stream.put_real64(self.warp_params.rad_params[plane].get(3))?;
        }

        // Write the optical center.

        stream.put_real64(self.warp_params.center_point.h)?;
        stream.put_real64(self.warp_params.center_point.v)?;

        Ok(())
    }

    /// `Apply (host, negative, image)` — warp the image into a new image.
    pub fn apply_warp(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        // Allocate destination image.

        let (bounds, planes, pixel_type) = {
            let img = image.read().unwrap();
            (img.bounds(), img.planes(), img.pixel_type())
        };

        let dst = dng_core::dng_simple_image::DngSimpleImage::new(bounds, planes, pixel_type)?;

        let dst_ref: ImageRef = std::sync::Arc::new(std::sync::RwLock::new(dst));

        // Warp the image.

        let mut filter = DngFilterWarp::new(
            image.clone(),
            dst_ref.clone(),
            negative,
            Box::new(self.warp_params.clone()),
        )?;

        filter.initialize()?;

        host.perform_area_task(&mut filter, bounds)?;

        // Return the new image.

        *image = dst_ref;

        Ok(())
    }
}

impl DngOpcode for DngOpcodeWarpFisheye {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `IsNOP ()`.
    fn is_nop(&self) -> bool {
        self.warp_params.is_nop_all()
    }

    /// `IsValidForNegative (negative)`.
    fn is_valid_for_negative(&self, negative: &DngNegative) -> bool {
        self.warp_params.is_valid_for_negative(negative)
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        self.put_data(stream)
    }

    /// `Apply (host, negative, image)`.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_warp(host, negative, image)
    }
}

// ---------------------------------------------------------------------------
// dng_vignette_radial_params.
// ---------------------------------------------------------------------------

/// Port of `dng_vignette_radial_params` — radially-symmetric vignette
/// (peripheral illumination falloff) correction parameters.
#[derive(Debug, Clone)]
pub struct DngVignetteRadialParams {
    /// `fParams` — gain polynomial
    /// g = 1 + k0*r^2 + k1*r^4 + k2*r^6 + k3*r^8 + k4*r^10.
    pub params: Vec<real64>,

    /// `fCenter`.
    pub center: DngPointReal64,
}

impl DngVignetteRadialParams {
    /// `kNumTerms`.
    pub const kNumTerms: uint32 = 5;

    /// `dng_vignette_radial_params ()` — default center (0.5, 0.5), zero
    /// params.
    pub fn new() -> Self {
        DngVignetteRadialParams {
            params: vec![0.0; Self::kNumTerms as usize],
            center: DngPointReal64::new(0.5, 0.5),
        }
    }

    /// `dng_vignette_radial_params (params, center)`.
    pub fn with_params(params: Vec<real64>, center: DngPointReal64) -> Self {
        DngVignetteRadialParams { params, center }
    }

    /// `IsNOP ()`.
    pub fn is_nop(&self) -> bool {
        for value in &self.params {
            if *value != 0.0 {
                return false;
            }
        }

        true
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        if self.params.len() != Self::kNumTerms as usize {
            return false;
        }

        if self.center.h < 0.0
            || self.center.h > 1.0
            || self.center.v < 0.0
            || self.center.v > 1.0
        {
            return false;
        }

        true
    }
}

impl Default for DngVignetteRadialParams {
    fn default() -> Self {
        Self::new()
    }
}

/// Port of `dng_vignette_radial_function` — the gain 1d function; `x`
/// represents r^2 in [0, 1].
#[derive(Debug, Clone)]
struct DngVignetteRadialFunction {
    params: DngVignetteRadialParams,
}

impl Dng1dFunction for DngVignetteRadialFunction {
    fn evaluate(&self, x: real64) -> real64 {
        debug_assert!(
            self.params.params.len() == DngVignetteRadialParams::kNumTerms as usize,
            "Bad number of vignette opcode coefficients."
        );

        let mut sum = 0.0;

        for value in self.params.params.iter().rev() {
            sum = x * (value + sum);
        }

        sum += 1.0;

        sum
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_FixVignetteRadial.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_FixVignetteRadial` — radially-symmetric lens vignette
/// correction (in-place opcode).
pub struct DngOpcodeFixVignetteRadial {
    base: DngOpcodeBase,

    /// `fParams`.
    pub params: DngVignetteRadialParams,

    /// `fImagePlanes` (set by Prepare).
    image_planes: uint32,

    /// `fSrcOriginH` / `fSrcOriginV` (fixed-point).
    src_origin_h: int64,
    src_origin_v: int64,

    /// `fSrcStepH` / `fSrcStepV` (fixed-point).
    src_step_h: int64,
    src_step_v: int64,

    /// `fTableInputBits`.
    table_input_bits: uint32,

    /// `fTableOutputBits`.
    table_output_bits: uint32,

    /// `fGainTable` — 16-bit gain LUT.
    gain_table: Option<Vec<u16>>,
}

impl DngOpcodeFixVignetteRadial {
    /// `ParamBytes ()`.
    pub fn param_bytes() -> uint32 {
        let n = DngVignetteRadialParams::kNumTerms;

        (n as usize * 8 + 2 * 8) as uint32
    }

    /// `dng_opcode_FixVignetteRadial (params, flags)`.
    pub fn new_with_params(params: DngVignetteRadialParams, flags: uint32) -> DngResult<Self> {
        if !params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeFixVignetteRadial {
            base: DngOpcodeBase::new(dngOpcode_FixVignetteRadial, dngVersion_1_3_0_0, flags),
            params,
            image_planes: 1,
            src_origin_h: 0,
            src_origin_v: 0,
            src_step_h: 0,
            src_step_v: 0,
            table_input_bits: 0,
            table_output_bits: 0,
            gain_table: None,
        })
    }

    /// `dng_opcode_FixVignetteRadial (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_FixVignetteRadial, stream)?;

        // Grab the size in bytes.

        let bytes = stream.get_uint32()?;

        // Verify the size.

        if bytes != Self::param_bytes() {
            return Err(bad_format());
        }

        // Read vignette coefficients.

        let mut coefficients = Vec::with_capacity(DngVignetteRadialParams::kNumTerms as usize);

        for _ in 0..DngVignetteRadialParams::kNumTerms {
            coefficients.push(stream.get_real64()?);
        }

        // Read the image center.

        let center = DngPointReal64::new(stream.get_real64()?, stream.get_real64()?);

        let params = DngVignetteRadialParams::with_params(coefficients, center);

        if !params.is_valid() {
            return Err(bad_format());
        }

        Ok(DngOpcodeFixVignetteRadial {
            base,
            params,
            image_planes: 1,
            src_origin_h: 0,
            src_origin_v: 0,
            src_step_h: 0,
            src_step_v: 0,
            table_input_bits: 0,
            table_output_bits: 0,
            gain_table: None,
        })
    }

    /// `PutData (stream)`.
    pub fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let bytes = Self::param_bytes();

        stream.put_uint32(bytes)?;

        debug_assert!(
            self.params.params.len() == DngVignetteRadialParams::kNumTerms as usize,
            "Bad number of vignette opcode coefficients."
        );

        for value in &self.params.params {
            stream.put_real64(*value)?;
        }

        stream.put_real64(self.params.center.h)?;
        stream.put_real64(self.params.center.v)?;

        Ok(())
    }

    /// `MakeParamsForRender (negative)` — hook for subclasses; returns the
    /// stored params.
    pub fn make_params_for_render(&self, _negative: &DngNegative) -> DngVignetteRadialParams {
        self.params.clone()
    }

    /// `Prepare (negative, threadCount, tileSize, imageBounds, imagePlanes,
    /// bufferPixelType, allocator)` — builds the gain table and fixed-point
    /// resample constants (mask scratch is per-tile in the port).
    fn prepare(
        &mut self,
        negative: &DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        image_bounds: DngRect,
        image_planes: uint32,
        buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        // This opcode is restricted to 32-bit images.

        if buffer_pixel_type != ttFloat {
            return Err(bad_format());
        }

        // Sanity check number of planes.

        debug_assert!(
            image_planes >= 1 && image_planes <= kMaxColorPlanes,
            "Bad number of planes."
        );

        if image_planes < 1 || image_planes > kMaxColorPlanes {
            return Err(program_error());
        }

        self.image_planes = image_planes;

        // Get the vignette radial params.

        let params = self.make_params_for_render(negative);

        // Grab the destination image area.

        let bounds = DngRectReal64::from_rect(image_bounds);

        // Determine the optical center and maximum radius in pixel
        // coordinates.

        let center_pixel = DngPointReal64::new(
            lerp_real64(bounds.t, bounds.b, params.center.v),
            lerp_real64(bounds.l, bounds.r, params.center.h),
        );

        let pixel_scale_v = 1.0 / negative.pixel_aspect_ratio();

        let max_radius = (max_real64(
            abs_real64(center_pixel.v - bounds.t),
            abs_real64(center_pixel.v - bounds.b),
        ) * pixel_scale_v)
            .hypot(max_real64(
                abs_real64(center_pixel.h - bounds.l),
                abs_real64(center_pixel.h - bounds.r),
            ));

        // Find origin and scale.

        let pixel_scale_h = 1.0;

        self.src_origin_h = real64_to_fixed64(-center_pixel.h * pixel_scale_h / max_radius);
        self.src_origin_v = real64_to_fixed64(-center_pixel.v * pixel_scale_v / max_radius);

        self.src_step_h = real64_to_fixed64(pixel_scale_h / max_radius);
        self.src_step_v = real64_to_fixed64(pixel_scale_v / max_radius);

        // Adjust for pixel centers.

        self.src_origin_h += self.src_step_h >> 1;
        self.src_origin_v += self.src_step_v >> 1;

        if self.gain_table.is_none() {
            // Set the vignette correction curve.

            let curve = DngVignetteRadialFunction {
                params: params.clone(),
            };

            // Evaluate 32-bit vignette correction table.

            let mut table32 = Dng1dTable::default_table()?;

            table32.initialize(&curve, false)?;

            // Find maximum scale factor.

            let max_scale = max_real32(table32.interpolate(0.0)?, table32.interpolate(1.0)?) as real64;

            // Find table input bits.

            self.table_input_bits = 16;

            // Find table output bits.

            self.table_output_bits = 15;

            while ((1u32 << self.table_output_bits) as real64) * max_scale > 65535.0 {
                self.table_output_bits -= 1;
            }

            // Allocate 16-bit scale table.

            let table_entries = (1u32 << self.table_input_bits) + 1;

            let mut table16 = vec![0u16; table_entries as usize];

            // Interpolate 32-bit table into 16-bit table.

            let scale0 = 1.0f32 / ((1u32 << self.table_input_bits) as f32);
            let scale1 = (1u32 << self.table_output_bits) as f32;

            for (index, entry) in table16.iter_mut().enumerate() {
                let x = index as f32 * scale0;

                let y = table32.interpolate(x)? * scale1;

                *entry = round_uint32_f64(y as real64) as u16;
            }

            self.gain_table = Some(table16);
        }

        Ok(())
    }

    /// `ProcessArea (negative, threadIndex, buffer, dstArea, imageBounds)` —
    /// computes the vignette mask for the tile and applies it in place.
    fn process_area_impl(
        &mut self,
        negative: &DngNegative,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
    ) -> DngResult<()> {
        let gain_table = match &self.gain_table {
            Some(table) => table.clone(),
            None => return Err(program_error()), // Prepare () not called.
        };

        // Setup mask pixel buffer.

        let mask_row_step = dst_area.w() as usize * self.image_planes as usize;

        let mut mask_data = vec![0u16; mask_row_step * dst_area.h() as usize];

        // Compute mask (scalar port of RefVignetteMask16).

        {
            let t_bits = self.table_input_bits;

            let t_shift = 32 - t_bits;
            let t_round = 1i64 << (t_shift - 1);
            let t_limit = 1i64 << t_bits;

            let mut offset_v = self.src_origin_v + self.src_step_v * dst_area.t as int64;
            let offset_h_base = self.src_origin_h + self.src_step_h * dst_area.l as int64;

            for row in 0..dst_area.h() as usize {
                let mut base_delta = (offset_v + 32768) >> 16;

                base_delta = base_delta * base_delta + t_round;

                let mut delta_h = offset_h_base + 32768;

                for col in 0..dst_area.w() as usize {
                    let temp = delta_h >> 16;

                    let delta = base_delta + (temp * temp);

                    let index = min_real64(delta as real64, t_limit as real64) as usize;

                    let mask_index = row * mask_row_step + col;

                    if index < gain_table.len() {
                        mask_data[mask_index] = gain_table[index];
                    }

                    delta_h += self.src_step_h;
                }

                offset_v += self.src_step_v;
            }
        }

        // Apply mask (scalar port of RefVignette32).

        {
            let black_level = if self.base.stage >= 2 {
                negative.stage3_black_level()
            } else {
                0
            };

            let s_row_step = buffer.row_step as usize;
            let s_plane_step = buffer.plane_step as usize;
            let m_row_step = mask_row_step;

            let m_bits = self.table_output_bits;

            let rows = dst_area.h() as usize;
            let cols = dst_area.w() as usize;

            let planes = self.image_planes as usize;

            // Stage 1: black-level rescale (when blackLevel != 0).

            let mut black_scale1: real32 = 1.0;
            let mut black_scale2: real32 = 1.0;
            let mut black_offset1: real32 = 0.0;
            let mut black_offset2: real32 = 0.0;

            if black_level != 0 {
                black_offset2 = (black_level as real32) / 65535.0;
                black_scale2 = 1.0 - black_offset2;
                black_scale1 = 1.0 / black_scale2;
                black_offset1 = 1.0 - black_scale1;

                // Rescale the buffer before applying the mask.

                for plane in 0..planes {
                    let base = buffer
                        .pixel_element_index_public(dst_area.t, dst_area.l, plane as uint32)?;

                    let dst = buffer.as_f32_slice_mut().unwrap();

                    for row in 0..rows {
                        for col in 0..cols {
                            let index = base + row * s_row_step + col;

                            if index < dst.len() {
                                dst[index] = dst[index] * black_scale1 + black_offset1;
                            }
                        }
                    }
                }
            }

            let _ = (black_scale2, black_offset2);

            let k_norm: real32 = 1.0 / ((1u32 << m_bits) as real32);

            for plane in 0..planes {
                let s_base =
                    buffer.pixel_element_index_public(dst_area.t, dst_area.l, plane as uint32)?;

                let dst = buffer.as_f32_slice_mut().unwrap();

                for row in 0..rows {
                    for col in 0..cols {
                        let s_index = s_base + row * s_row_step + col;
                        let m_index = row * m_row_step + col;

                        if s_index < dst.len() && m_index < mask_data.len() {
                            let s = dst[s_index];

                            let m = mask_data[m_index];

                            let scale = m as real32 * k_norm;

                            let s = min_real32(s * scale, 1.0);

                            dst[s_index] = s;
                        }
                    }
                }
            }

            let _ = s_plane_step;
        }

        Ok(())
    }
}

impl DngOpcode for DngOpcodeFixVignetteRadial {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `IsNOP ()`.
    fn is_nop(&self) -> bool {
        self.params.is_nop()
    }

    /// `IsValidForNegative (negative)`.
    fn is_valid_for_negative(&self, _negative: &DngNegative) -> bool {
        self.params.is_valid()
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        self.put_data(stream)
    }

    /// `BufferPixelType (imagePixelType)` — restricted to float.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        // dng_inplace_opcode::Apply — the base-class driver handles tile
        // iteration (dng_opcodes::apply_inplace_opcode).  The port invokes
        // the same driver through the DngInplaceOpcode impl below; this
        // override exists for direct trait-object use.

        let _ = host;

        crate::dng_opcodes::apply_inplace_opcode(self, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeFixVignetteRadial {
    /// `BufferPixelType (imagePixelType)` — restricted to float.
    fn buffer_pixel_type(&mut self, _image_pixel_type: uint32) -> DngResult<uint32> {
        Ok(ttFloat)
    }

    /// `Prepare (negative, threadCount, tileSize, imageBounds, imagePlanes,
    /// bufferPixelType, allocator)`.
    fn prepare(
        &mut self,
        negative: &mut DngNegative,
        thread_count: uint32,
        tile_size: DngPoint,
        image_bounds: DngRect,
        image_planes: uint32,
        buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        DngOpcodeFixVignetteRadial::prepare(
            self,
            negative,
            thread_count,
            tile_size,
            image_bounds,
            image_planes,
            buffer_pixel_type,
        )
    }

    /// `ProcessArea (negative, threadIndex, buffer, dstArea, imageBounds)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        self.process_area_impl(negative, buffer, dst_area)
    }
}

// ---------------------------------------------------------------------------
// Compatibility aliases for Agent B's registry call sites.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_nop_defaults() {
        let radial = DngWarpParamsRadial::new();

        for plane in 0..kMaxColorPlanes {
            assert!(radial.is_nop(plane));
            assert!(radial.is_valid(plane));
        }

        assert!(radial.compatible_with_warp_rectilinear_1_3(0));
    }

    #[test]
    fn radial_evaluate_identity_is_one() {
        let mut radial = DngWarpParamsRadial::new();

        // Set r^2 coefficient (DNG 1.3 style).

        let params = DngVector::from_values(&[1.0, 0.0, 0.0, 0.0]).unwrap();

        radial.set_warp_rectilinear_1_3(0, &params);

        // f(r) = 1 + 0*r^2 -> ratio 1 everywhere.

        assert!((radial.evaluate_ratio(0, 0.0) - 1.0).abs() < 1e-12);
        assert!((radial.evaluate_ratio(0, 0.5) - 1.0).abs() < 1e-12);
        assert!((radial.evaluate_ratio(0, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn radial_evaluate_barrel() {
        let mut radial = DngWarpParamsRadial::new();

        let params = DngVector::from_values(&[1.0, -0.2, 0.0, 0.0]).unwrap();

        radial.set_warp_rectilinear_1_3(0, &params);

        // f(r) = 1 - 0.2 * r^2; at r=1 ratio = 0.8.

        assert!((radial.evaluate_ratio(0, 1.0) - 0.8).abs() < 1e-12);
        assert!((radial.evaluate(0, 1.0) - 0.8).abs() < 1e-12);
    }

    #[test]
    fn rectilinear_tangential_matches_cpp_formula() {
        let rect = DngWarpParamsRectilinear::new();

        // Default tan params are zero: tangential output is (0, 0).

        let diff = DngPointReal64::new(10.0, 20.0);
        let diff2 = DngPointReal64::new(diff.v * diff.v, diff.h * diff.h);

        let result = rect.evaluate_tangential(0, diff.h * diff.h + diff.v * diff.v, &diff, &diff2);

        assert_eq!(result.v, 0.0);
        assert_eq!(result.h, 0.0);

        // With kt0 = 1, kt1 = 0: v = 1 * (r2 + 2*dvdv), h = 2 * kt0 * dh*dv.

        let mut rect = DngWarpParamsRectilinear::new();

        let tan = DngVector::from_values(&[1.0, 0.0]).unwrap();

        rect.tan_params[0] = tan;

        let result = rect.evaluate_tangential(0, diff.h * diff.h + diff.v * diff.v, &diff, &diff2);

        let r2 = diff.h * diff.h + diff.v * diff.v;

        assert!((result.v - (r2 + 2.0 * (diff.v * diff.v))).abs() < 1e-9);
        assert!((result.h - (2.0 * diff.h * diff.v)).abs() < 1e-9);
    }

    #[test]
    fn fisheye_evaluate_matches_cpp_formula() {
        let mut fisheye = DngWarpParamsFisheye::new();

        let rad = DngVector::from_values(&[1.0, 0.0, 0.0, 0.0]).unwrap();

        fisheye.rad_params[0] = rad;

        // w(r) = atan (r) * 1.

        for r in [0.1f64, 0.5f64, 1.0f64] {
            let t = r.atan();

            assert!((fisheye.evaluate(0, r) - t).abs() < 1e-12);
        }
    }

    #[test]
    fn fisheye_tangential_panics() {
        let fisheye = DngWarpParamsFisheye::new();

        let diff = DngPointReal64::new(1.0, 1.0);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = fisheye.evaluate_tangential(0, 0.5, &diff, &diff);
        }));

        assert!(result.is_err());
    }

    #[test]
    fn warp_rectilinear_stream_round_trip() -> DngResult<()> {
        let mut host = DngHost::new();
        let mut negative = DngNegative::new(&mut host).unwrap();

        negative.set_color_channels(3);

        let mut params = DngWarpParamsRectilinear::new();

        params.planes_count = 1;

        let radial_params = DngVector::from_values(&[1.0, -0.1, 0.01, 0.0]).unwrap();

        params.rad_params.set_warp_rectilinear_1_3(0, &radial_params);

        let tan = DngVector::from_values(&[0.001, 0.002]).unwrap();

        params.tan_params[0] = tan;

        let opcode = DngOpcodeWarpRectilinear::new_with_params(params, 0).unwrap();

        // Serialize.

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        stream.put_uint32(opcode.base().opcode_id)?;
        stream.put_uint32(opcode.base().min_version)?;
        stream.put_uint32(opcode.base().flags)?;

        opcode.put_data(&mut stream).unwrap();

        // Deserialize: skip only the opcode ID; from_stream reads
        // minVersion + flags itself (C++ ctor semantics).

        stream.set_read_position(0).unwrap();

        let _header_opcode = stream.get_uint32().unwrap();

        let parsed = DngOpcodeWarpRectilinear::from_stream(&mut stream).unwrap();

        assert!(!parsed.is_nop());
        assert!(parsed.is_valid_for_negative(&negative));

        // Payload byte count matches C++ layout.

        assert_eq!(
            DngOpcodeWarpRectilinear::param_bytes(1),
            4 + 6 * 8 + 2 * 8
        );

        Ok(())
    }

    #[test]
    fn warp_rectilinear2_stream_round_trip() -> DngResult<()> {
        let mut params = DngWarpParamsRectilinear::new();

        params.planes_count = 1;

        // Set term 0 = 1 so the radial model is a NOP (identity).

        params.rad_params.data[0][0] = 1.0;

        params.rad_params.valid_range[0][0] = 0.0;
        params.rad_params.valid_range[0][1] = 1.0;

        let opcode = DngOpcodeWarpRectilinear2::new_with_params(params, 0).unwrap();

        assert!(opcode.is_nop());

        // Serialize (ID + minVersion + flags + PutData, per the C++
        // opcode-list entry layout).

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        stream.put_uint32(opcode.base().opcode_id)?;
        stream.put_uint32(opcode.base().min_version)?;
        stream.put_uint32(opcode.base().flags)?;

        opcode.put_data(&mut stream).unwrap();

        // Deserialize: skip only the opcode ID.

        stream.set_read_position(0).unwrap();

        let _header_opcode = stream.get_uint32()?;

        let parsed = DngOpcodeWarpRectilinear2::from_stream(&mut stream)?;

        assert!(parsed.is_nop());
        assert_eq!(parsed.base().opcode_id, dngOpcode_WarpRectilinear2);

        Ok(())
    }

    #[test]
    fn warp_fisheye_stream_round_trip() {
        let mut params = DngWarpParamsFisheye::new();

        params.planes_count = 1;

        let rad = DngVector::from_values(&[1.0, 0.0, 0.0, 0.0]).unwrap();

        params.rad_params[0] = rad;

        let opcode = DngOpcodeWarpFisheye::new_with_params(params, 0).unwrap();

        // Serialize the payload (after the opcode header the C++ ctor
        // consumes).

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        stream.put_uint32(opcode.base().opcode_id).unwrap();
        stream.put_uint32(opcode.base().min_version).unwrap();
        stream.put_uint32(opcode.base().flags).unwrap();

        opcode.put_data(&mut stream).unwrap();

        stream.set_read_position(0).unwrap();

        // Skip only the opcode ID; from_stream reads minVersion + flags.
        let _ = stream.get_uint32().unwrap();

        let parsed = DngOpcodeWarpFisheye::from_stream(&mut stream).unwrap();

        assert!(!parsed.is_nop());

        assert_eq!(DngOpcodeWarpFisheye::param_bytes(1), 4 + 4 * 8 + 2 * 8);
    }

    #[test]
    fn vignette_params_and_gain_table() -> DngResult<()> {
        let mut host = DngHost::new();
        let mut negative = DngNegative::new(&mut host).unwrap();

        // All-zero params: NOP.

        let nop_params = DngVignetteRadialParams::new();

        assert!(nop_params.is_nop());
        assert!(nop_params.is_valid());

        let opcode = DngOpcodeFixVignetteRadial::new_with_params(nop_params, 0).unwrap();

        assert!(opcode.is_nop());

        // Non-zero params: gain function evaluation matches the C++ Horner
        // layout.

        let params = DngVignetteRadialParams::with_params(
            vec![0.1, -0.02, 0.003, 0.0, 0.0],
            DngPointReal64::new(0.5, 0.5),
        );

        assert!(!params.is_nop());
        assert!(params.is_valid());

        let curve = DngVignetteRadialFunction {
            params: params.clone(),
        };

        // g(r^2 = 0) = 1.

        assert!((curve.evaluate(0.0) - 1.0).abs() < 1e-12);

        // g(x) = 1 + k0*x + k1*x^2 + ... (Horner over reversed terms).

        let x = 0.25;

        let expected = 1.0 + 0.1 * x + (-0.02) * x * x + 0.003 * x * x * x;

        assert!((curve.evaluate(x) - expected).abs() < 1e-12);

        // Prepare builds the gain table.

        let mut opcode = DngOpcodeFixVignetteRadial::new_with_params(params, 0).unwrap();

        let bounds = DngRect::from_corners(0, 0, 16, 16);

        opcode
            .prepare(&mut negative, 1, DngPoint::new(16, 16), bounds, 1, ttFloat)
            .unwrap();

        assert!(opcode.gain_table.is_some());

        assert_eq!(
            opcode.gain_table.as_ref().unwrap().len(),
            (1u32 << opcode.table_input_bits) as usize + 1
        );

        Ok(())
    }

    #[test]
    fn vignette_stream_round_trip() {
        let params = DngVignetteRadialParams::with_params(
            vec![0.1, -0.02, 0.003, 0.0, 0.0],
            DngPointReal64::new(0.5, 0.5),
        );

        let opcode = DngOpcodeFixVignetteRadial::new_with_params(params, 0).unwrap();

        let mut stream = dng_core::dng_memory_stream::DngMemoryStream::new().stream();

        // Wire layout: [opcodeID][minVersion][flags] + PutData payload.
        stream.put_uint32(opcode.base().opcode_id).unwrap();
        stream.put_uint32(opcode.base().min_version).unwrap();
        stream.put_uint32(opcode.base().flags).unwrap();

        opcode.put_data(&mut stream).unwrap();

        stream.set_read_position(0).unwrap();

        // Skip only the opcode ID; from_stream reads minVersion + flags.
        let _ = stream.get_uint32().unwrap();

        let parsed = DngOpcodeFixVignetteRadial::from_stream(&mut stream).unwrap();

        assert!(!parsed.is_nop());

        assert_eq!(DngOpcodeFixVignetteRadial::param_bytes(), 5 * 8 + 2 * 8);
    }

    #[test]
    fn warp_params_newton_inverse_round_trip() {
        let mut radial = DngWarpParamsRadial::new();

        let params = DngVector::from_values(&[1.0, -0.2, 0.0, 0.0]).unwrap();

        radial.set_warp_rectilinear_1_3(0, &params);

        let rect = DngWarpParamsRectilinear::new();

        // Use rectilinear delegate: evaluate/evaluate_inverse pair.

        let mut rect = DngWarpParamsRectilinear::new();

        rect.rad_params = radial;

        for r in [0.0, 0.25, 0.5, 1.0] {
            let y = rect.evaluate(0, r);

            let x = rect.evaluate_inverse(0, y);

            assert!((x - r).abs() < 1e-6, "r={} y={} x={}", r, y, x);
        }
    }
}

// Port of dng_tone_curve.h/.cpp — a monotone piecewise tone curve in [0,1].

use crate::dng_core::{DngPointReal64, DngResult};

/// Port of `dng_tone_curve`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngToneCurve {
    pub coord: Vec<DngPointReal64>,
}

impl DngToneCurve {
    pub fn new() -> Self {
        let mut tc = DngToneCurve::default();
        tc.set_null();
        tc
    }

    /// `SetNull ()` — the identity curve.
    pub fn set_null(&mut self) {
        self.coord.clear();
        self.coord.push(DngPointReal64::new(0.0, 0.0));
        self.coord.push(DngPointReal64::new(1.0, 1.0));
    }

    /// `IsNull ()`.
    pub fn is_null(&self) -> bool {
        self.coord.len() == 2
            && self.coord[0].h == 0.0
            && self.coord[0].v == 0.0
            && self.coord[1].h == 1.0
            && self.coord[1].v == 1.0
    }

    /// `SetInvalid ()`.
    pub fn set_invalid(&mut self) {
        self.coord.clear();
    }

    /// `IsValid ()` — monotone increasing h, both coords in [0,1].
    pub fn is_valid(&self) -> bool {
        if self.coord.len() < 2 {
            return false;
        }

        for j in 0..self.coord.len() {
            if !(0.0..=1.0).contains(&self.coord[j].h)
                || !(0.0..=1.0).contains(&self.coord[j].v)
            {
                return false;
            }

            if j > 0 && self.coord[j].h <= self.coord[j - 1].h {
                return false;
            }
        }

        true
    }

    /// `Solve (solver)` — feed the coordinates to a spline solver.
    pub fn solve(
        &self,
        solver: &mut crate::dng_core::dng_1d_function::DngPiecewiseLinear,
    ) -> DngResult<()> {
        // PORT-NOTE(dng_tone_curve.cpp:91): the C++ feeds a dng_spline_solver;
        // the port uses the piecewise-linear 1D function for the default
        // path (dng_color_spec constructs its own spline solver where the
        // C++ does).
        solver.reset();

        for point in &self.coord {
            solver.add(point.h, point.v);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_curve_is_identity() {
        let tc = DngToneCurve::new();
        assert!(tc.is_null());
        assert_eq!(tc.coord.len(), 2);
        assert_eq!(tc.coord[0].h, 0.0);
        assert_eq!(tc.coord[1].v, 1.0);
    }

    #[test]
    fn validity_checks() {
        let mut tc = DngToneCurve::new();
        assert!(tc.is_valid());

        // Non-monotone h.
        tc.coord.push(DngPointReal64::new(0.5, 0.5));
        assert!(!tc.is_valid());

        // Out-of-range v.
        let mut tc2 = DngToneCurve::new();
        tc2.coord[0].v = -0.1;
        assert!(!tc2.is_valid());

        // Too few points.
        let mut tc3 = DngToneCurve::new();
        tc3.coord.clear();
        assert!(!tc3.is_valid());
    }

    #[test]
    fn solve_feeds_spline() {
        let mut tc = DngToneCurve::new();
        tc.coord.push(DngPointReal64::new(0.5, 0.25));

        // Keep h sorted.
        tc.coord.sort_by(|a, b| a.h.partial_cmp(&b.h).unwrap());
        let mut solver = crate::dng_core::dng_1d_function::DngPiecewiseLinear::new();
        tc.solve(&mut solver).unwrap();
        assert!(solver.is_valid());
    }
}

// Port of dng_spline.h/.cpp — cubic spline solver (C0/C1/C2 continuous,
// zero second derivative at endpoints) with optional linear slope extension.

use crate::dng_1d_function::Dng1dFunction;
use crate::dng_errors::{DngError, DngResult};
use crate::dng_types::real64;

/// `EvaluateSplineSegment` (shared with dng_color_space gamma functions).
#[allow(clippy::too_many_arguments)]
#[inline]
pub fn evaluate_spline_segment(
    x: real64,
    x0: real64,
    y0: real64,
    s0: real64,
    x1: real64,
    y1: real64,
    s1: real64,
) -> real64 {
    let a = x1 - x0;
    let b = (x - x0) / a;
    let c = (x1 - x) / a;

    ((y0 * (2.0 - c + b) + (s0 * a * b)) * (c * c))
        + ((y1 * (2.0 - b + c) - (s1 * a * c)) * (b * b))
}

/// `EvaluateSlopeSplineSegment`.
#[allow(clippy::too_many_arguments)]
fn evaluate_slope_spline_segment(
    x: real64,
    x0: real64,
    y0: real64,
    s0: real64,
    x1: real64,
    y1: real64,
    s1: real64,
) -> real64 {
    let a = x1 - x0;
    let a2 = a * a;
    let a3 = a2 * a;

    let c0 = ((s1 * x0 * x0)
        + (2.0 * s0 * x0 * x1)
        + (2.0 * s1 * x0 * x1)
        + (s0 * x1 * x1)
        - (4.0 * x1 * y0)
        - (4.0 * x0 * y1))
        / a2;

    let c1 = ((2.0 * x0 * x1 * y0) + (4.0 * x1 * x1 * y0) - (4.0 * x0 * x0 * y1)
        - (2.0 * x0 * x1 * y1))
        / a3;

    let quad_term = 3.0 * x * x * (((s0 + s1) / a2) + (2.0 * (y0 - y1) / a3));

    let linear_term = 2.0 * x
        * (((-((s0 * x0)) - (2.0 * s1 * x0) - (2.0 * s0 * x1) - (s1 * x1) + (2.0 * y0)
            + (2.0 * y1))
            / a2)
            + ((-(x0 * y0) - (5.0 * x1 * y0) + (5.0 * x0 * y1) + (x1 * y1)) / a3));

    linear_term + quad_term + c0 + c1
}

/// Port of `dng_spline_solver`.
#[derive(Debug, Clone, Default)]
pub struct DngSplineSolver {
    pub x: Vec<real64>,
    pub y: Vec<real64>,
    s: Vec<real64>,

    /// Linear slope extension before the first curve endpoint.
    use_slope_extension_lo: bool,
    /// Linear slope extension after the last curve endpoint.
    use_slope_extension_hi: bool,
}

impl DngSplineSolver {
    pub fn new() -> Self {
        DngSplineSolver::default()
    }

    /// `Reset ()`.
    pub fn reset(&mut self) {
        self.x.clear();
        self.y.clear();
        self.s.clear();
    }

    /// `Add (x, y)`.
    pub fn add(&mut self, x: real64, y: real64) {
        self.x.push(x);
        self.y.push(y);
    }

    /// `Solve ()`.
    pub fn solve(&mut self) -> DngResult<()> {
        // This code computes the unique curve such that:
        //   It is C0, C1, and C2 continuous
        //   The second derivative is zero at the end points

        let count = self.x.len() as i32;

        if count < 2 {
            return Err(DngError::Unsupported("Too few points".to_string()));
        }

        let start = 0i32;
        let end = count;

        let mut a = self.x[(start + 1) as usize] - self.x[start as usize];
        let mut b = (self.y[(start + 1) as usize] - self.y[start as usize]) / a;

        self.s.resize(count as usize, 0.0);

        self.s[start as usize] = b;

        // Slopes here are a weighted average of the slopes to each of the
        // adjacent control points.
        for j in (start + 2)..end {
            let c = self.x[j as usize] - self.x[(j - 1) as usize];
            let d = (self.y[j as usize] - self.y[(j - 1) as usize]) / c;

            self.s[(j - 1) as usize] = (b * c + d * a) / (a + c);

            a = c;
            b = d;
        }

        self.s[(end - 1) as usize] = 2.0 * b - self.s[(end - 2) as usize];
        self.s[start as usize] = 2.0 * self.s[start as usize] - self.s[(start + 1) as usize];

        if (end - start) > 2 {
            let mut e = vec![0.0; count as usize];
            let mut f = vec![0.0; count as usize];
            let mut g = vec![0.0; count as usize];

            f[start as usize] = 0.5;
            e[(end - 1) as usize] = 0.5;
            g[start as usize] = 0.75 * (self.s[start as usize] + self.s[(start + 1) as usize]);
            g[(end - 1) as usize] =
                0.75 * (self.s[(end - 2) as usize] + self.s[(end - 1) as usize]);

            for j in (start + 1)..(end - 1) {
                let a = (self.x[(j + 1) as usize] - self.x[(j - 1) as usize]) * 2.0;

                e[j as usize] = (self.x[(j + 1) as usize] - self.x[j as usize]) / a;
                f[j as usize] = (self.x[j as usize] - self.x[(j - 1) as usize]) / a;
                g[j as usize] = 1.5 * self.s[j as usize];
            }

            for j in (start + 1)..end {
                let a = 1.0 - f[(j - 1) as usize] * e[j as usize];

                if j != end - 1 {
                    f[j as usize] /= a;
                }

                g[j as usize] = (g[j as usize] - g[(j - 1) as usize] * e[j as usize]) / a;
            }

            let mut j = end - 2;
            while j >= start {
                g[j as usize] = g[j as usize] - f[j as usize] * g[(j + 1) as usize];
                j -= 1;
            }

            for j in start..end {
                self.s[j as usize] = g[j as usize];
            }
        }

        Ok(())
    }

    /// `EvaluateSlope (x)`.
    pub fn evaluate_slope(&self, x: real64) -> real64 {
        let count = self.x.len() as i32;

        let mut lower: i32 = 1;
        let mut upper: i32 = count - 1;

        let mut x = x;

        // Check for off each end of point list.
        if x <= self.x[0] {
            x = self.x[0];
            lower = 1;
        } else if x >= self.x[(count - 1) as usize] {
            x = self.x[(count - 1) as usize];
            lower = upper;
        } else {
            // Binary search for the index.
            while upper > lower {
                let mid = (lower + upper) >> 1;

                if x == self.x[mid as usize] {
                    return self.y[mid as usize];
                }

                if x > self.x[mid as usize] {
                    lower = mid + 1;
                } else {
                    upper = mid;
                }
            }
        }

        let j = lower as usize;

        evaluate_slope_spline_segment(
            x,
            self.x[j - 1],
            self.y[j - 1],
            self.s[j - 1],
            self.x[j],
            self.y[j],
            self.s[j],
        )
    }

    /// `UseSlopeExtensionLo ()`.
    pub fn use_slope_extension_lo(&self) -> bool {
        self.use_slope_extension_lo
    }

    /// `UseSlopeExtensionHi ()`.
    pub fn use_slope_extension_hi(&self) -> bool {
        self.use_slope_extension_hi
    }

    /// `SetUseSlopeExtension (flag)`.
    pub fn set_use_slope_extension(&mut self, flag: bool) {
        self.use_slope_extension_lo = flag;
        self.use_slope_extension_hi = flag;
    }

    /// `SetUseSlopeExtensionLo (flag)`.
    pub fn set_use_slope_extension_lo(&mut self, flag: bool) {
        self.use_slope_extension_lo = flag;
    }

    /// `SetUseSlopeExtensionHi (flag)`.
    pub fn set_use_slope_extension_hi(&mut self, flag: bool) {
        self.use_slope_extension_hi = flag;
    }
}

impl Dng1dFunction for DngSplineSolver {
    fn is_identity(&self) -> bool {
        if self.x.len() != 2 {
            return false;
        }
        if self.x[0] != 0.0
            || self.x[1] != 1.0
            || self.y[0] != 0.0
            || self.y[1] != 1.0
        {
            return false;
        }
        true
    }

    fn evaluate(&self, x: real64) -> real64 {
        let count = self.x.len();

        // Check for off each end of point list.
        if x <= self.x[0] {
            return if self.use_slope_extension_lo {
                self.y[0] + self.evaluate_slope(self.x[0]) * (x - self.x[0])
            } else {
                self.y[0]
            };
        }

        if x >= self.x[count - 1] {
            return if self.use_slope_extension_hi {
                self.y[count - 1]
                    + self.evaluate_slope(self.x[count - 1]) * (x - self.x[count - 1])
            } else {
                self.y[count - 1]
            };
        }

        // Binary search for the index.
        let mut lower: usize = 1;
        let mut upper: usize = count - 1;

        while upper > lower {
            let mid = (lower + upper) >> 1;

            if x == self.x[mid] {
                return self.y[mid];
            }

            if x > self.x[mid] {
                lower = mid + 1;
            } else {
                upper = mid;
            }
        }

        let j = lower;

        evaluate_spline_segment(
            x,
            self.x[j - 1],
            self.y[j - 1],
            self.s[j - 1],
            self.x[j],
            self.y[j],
            self.s[j],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple_spline() {
        let mut solver = DngSplineSolver::new();
        solver.add(0.0, 0.0);
        solver.add(1.0, 1.0);
        solver.solve().unwrap();

        assert!(solver.is_identity());
        assert!((solver.evaluate(0.5) - 0.5).abs() < 1e-9);
        assert_eq!(solver.evaluate(-1.0), 0.0);
        assert_eq!(solver.evaluate(2.0), 1.0);
    }

    #[test]
    fn three_point_monotone() {
        let mut solver = DngSplineSolver::new();
        solver.add(0.0, 0.0);
        solver.add(0.5, 0.4);
        solver.add(1.0, 1.0);
        solver.solve().unwrap();

        assert!(!solver.is_identity());

        // Endpoints exact; interior close to control polygon.
        assert_eq!(solver.evaluate(0.0), 0.0);
        assert_eq!(solver.evaluate(0.5), 0.4);
        assert_eq!(solver.evaluate(1.0), 1.0);
        assert!(solver.evaluate(0.25) > 0.0 && solver.evaluate(0.25) < 0.4);
        assert!(solver.evaluate(0.75) > 0.4 && solver.evaluate(0.75) < 1.0);

        // Monotone.
        let mut prev = solver.evaluate(0.0);
        for i in 1..=100 {
            let x = i as f64 / 100.0;
            let v = solver.evaluate(x);
            assert!(v >= prev - 1e-9);
            prev = v;
        }
    }

    #[test]
    fn slope_extension() {
        let mut solver = DngSplineSolver::new();
        solver.add(0.0, 0.0);
        solver.add(1.0, 1.0);
        solver.set_use_slope_extension(true);
        solver.solve().unwrap();

        // With slope extension, below/above continue the end slope.
        let below = solver.evaluate(-1.0);
        let above = solver.evaluate(2.0);
        assert!((below - -1.0).abs() < 1e-6);
        assert!((above - 2.0).abs() < 1e-6);
    }

    #[test]
    fn evaluate_slope_matches_segments() {
        let mut solver = DngSplineSolver::new();
        solver.add(0.0, 0.0);
        solver.add(0.5, 0.25);
        solver.add(1.0, 1.0);
        solver.solve().unwrap();

        // At control points, slope of y=x*x on [0,0.5] is ~1 at 0.
        let slope0 = solver.evaluate_slope(0.0);
        assert!((slope0 - 1.0).abs() < 1e-6, "slope0={slope0}");
    }

    #[test]
    fn too_few_points_errors() {
        let mut solver = DngSplineSolver::new();
        solver.add(0.0, 0.0);
        assert!(solver.solve().is_err());
    }
}

// Port of dng_1d_function.h/.cpp — 1D function abstraction, identity,
// concatenation, inverse and piecewise-linear functions.

use crate::dng_types::real64;
use crate::dng_utils::{abs_real64, pin_real64_01};

/// Port of `dng_1d_function`.  The domain is always 0.0..1.0.
pub trait Dng1dFunction: Send + Sync {
    /// `IsIdentity ()` — Evaluate(x) == x for all x.
    fn is_identity(&self) -> bool {
        false
    }

    /// `Evaluate (x)`.
    fn evaluate(&self, x: real64) -> real64;

    /// `EvaluateInverse (y)` — default: Newton/secant iteration.
    fn evaluate_inverse(&self, y: real64) -> real64 {
        const kMaxIterations: u32 = 30;
        const kNearZero: real64 = 1.0e-10;

        let mut x0 = 0.0;
        let mut y0 = self.evaluate(x0);

        let mut x1 = 1.0;
        let mut y1 = self.evaluate(x1);

        for _ in 0..kMaxIterations {
            if abs_real64(y1 - y0) < kNearZero {
                break;
            }

            let x2 = pin_real64_01(x1 + (y - y1) * (x1 - x0) / (y1 - y0));
            let y2 = self.evaluate(x2);

            x0 = x1;
            y0 = y1;

            x1 = x2;
            y1 = y2;
        }

        x1
    }
}

/// Port of `dng_1d_identity`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Dng1dIdentity;

impl Dng1dFunction for Dng1dIdentity {
    fn is_identity(&self) -> bool {
        true
    }

    fn evaluate(&self, x: real64) -> real64 {
        x
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        y
    }
}

impl Dng1dIdentity {
    /// `dng_1d_identity::Get ()`.
    pub fn get() -> &'static dyn Dng1dFunction {
        static INSTANCE: std::sync::OnceLock<Dng1dIdentity> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(|| Dng1dIdentity)
    }
}

/// Port of `dng_1d_concatenate` — y = f2(f1(x)).
pub struct Dng1dConcatenate {
    function1: Box<dyn Dng1dFunction>,
    function2: Box<dyn Dng1dFunction>,
}

impl Dng1dConcatenate {
    pub fn new(function1: Box<dyn Dng1dFunction>, function2: Box<dyn Dng1dFunction>) -> Self {
        Dng1dConcatenate {
            function1,
            function2,
        }
    }
}

impl Dng1dFunction for Dng1dConcatenate {
    fn is_identity(&self) -> bool {
        self.function1.is_identity() && self.function2.is_identity()
    }

    fn evaluate(&self, x: real64) -> real64 {
        let y = pin_real64_01(self.function1.evaluate(x));
        self.function2.evaluate(y)
    }

    fn evaluate_inverse(&self, x: real64) -> real64 {
        let y = self.function2.evaluate_inverse(x);
        self.function1.evaluate_inverse(y)
    }
}

/// Port of `dng_1d_inverse`.
pub struct Dng1dInverse {
    function: Box<dyn Dng1dFunction>,
}

impl Dng1dInverse {
    pub fn new(function: Box<dyn Dng1dFunction>) -> Self {
        Dng1dInverse { function }
    }
}

impl Dng1dFunction for Dng1dInverse {
    fn is_identity(&self) -> bool {
        self.function.is_identity()
    }

    fn evaluate(&self, x: real64) -> real64 {
        self.function.evaluate_inverse(x)
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        self.function.evaluate(y)
    }
}

/// Port of `dng_piecewise_linear`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngPiecewiseLinear {
    pub x: Vec<real64>,
    pub y: Vec<real64>,
}

impl DngPiecewiseLinear {
    pub fn new() -> Self {
        DngPiecewiseLinear::default()
    }

    /// `Reset ()`.
    pub fn reset(&mut self) {
        self.x.clear();
        self.y.clear();
    }

    /// `Add (x, y)`.
    pub fn add(&mut self, x: real64, y: real64) {
        self.x.push(x);
        self.y.push(y);
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.x.len() >= 2 && self.x.len() == self.y.len()
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `IsIdentity ()`.
    pub fn is_identity_fn(&self) -> bool {
        self.x.len() == 2
            && self.x.len() == self.y.len()
            && self.x[0] == 0.0
            && self.y[0] == 0.0
            && self.x[1] == 1.0
            && self.y[1] == 1.0
    }

    /// `PutFingerprintData (stream)` — piecewise data for fingerprints.
    pub fn put_fingerprint_data(&self, stream: &mut crate::dng_stream::DngStream) -> crate::dng_errors::DngResult<()> {
        use crate::dng_types::uint32;

        stream.put_uint32(self.x.len() as uint32)?;

        for x in &self.x {
            stream.put_real64(*x)?;
        }

        stream.put_uint32(self.y.len() as uint32)?;

        for y in &self.y {
            stream.put_real64(*y)?;
        }

        Ok(())
    }
}

impl Dng1dFunction for DngPiecewiseLinear {
    fn is_identity(&self) -> bool {
        self.is_identity_fn()
    }

    fn evaluate(&self, x: real64) -> real64 {
        debug_assert!(self.x.len() >= 2, "Too few points.");
        debug_assert!(self.x.len() == self.y.len(), "Input/output vector size mismatch.");

        if x <= self.x[0] {
            return self.y[0];
        } else if x >= *self.x.last().unwrap() {
            return *self.y.last().unwrap();
        }

        let mut lower: i32 = 1;
        let mut upper: i32 = self.x.len() as i32 - 1;

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

        let index0 = (lower - 1) as usize;
        let index1 = lower as usize;

        let x0 = self.x[index0];
        let x1 = self.x[index1];

        let y0 = self.y[index0];
        let y1 = self.y[index1];

        if x0 == x1 {
            return 0.5 * (y0 + y1);
        }

        let t = (x - x0) / (x1 - x0);

        y0 + t * (y1 - y0)
    }

    fn evaluate_inverse(&self, y: real64) -> real64 {
        debug_assert!(self.x.len() >= 2, "Too few points.");
        debug_assert!(self.x.len() == self.y.len(), "Input/output vector size mismatch.");

        let mut lower: i32 = 1;
        let mut upper: i32 = self.y.len() as i32 - 1;

        while upper > lower {
            let mid = (lower + upper) >> 1;

            if y == self.y[mid as usize] {
                return self.x[mid as usize];
            }

            if y > self.y[mid as usize] {
                lower = mid + 1;
            } else {
                upper = mid;
            }
        }

        let index0 = (lower - 1) as usize;
        let index1 = lower as usize;

        let x0 = self.x[index0];
        let x1 = self.x[index1];

        let y0 = self.y[index0];
        let y1 = self.y[index1];

        if y0 == y1 {
            return 0.5 * (x0 + x1);
        }

        let t = (y - y0) / (y1 - y0);

        x0 + t * (x1 - x0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity() {
        let f = Dng1dIdentity;
        assert!(f.is_identity());
        assert_eq!(f.evaluate(0.25), 0.25);
        assert_eq!(f.evaluate_inverse(0.75), 0.75);
    }

    #[test]
    fn default_newton_inverse() {
        struct Square;
        impl Dng1dFunction for Square {
            fn evaluate(&self, x: real64) -> real64 {
                x * x
            }
        }

        let f = Square;
        let inv = f.evaluate_inverse(0.25);
        assert!((inv - 0.5).abs() < 1e-4);
    }

    #[test]
    fn piecewise_linear() {
        let mut f = DngPiecewiseLinear::new();
        f.add(0.0, 0.0);
        f.add(0.5, 1.0);
        f.add(1.0, 0.0);

        assert!(f.is_valid());
        assert!(!f.is_identity_fn());

        assert_eq!(f.evaluate(0.0), 0.0);
        assert_eq!(f.evaluate(1.0), 0.0);
        assert_eq!(f.evaluate(0.25), 0.5);
        assert_eq!(f.evaluate(0.75), 0.5);
        // Outside domain clamps to endpoints.
        assert_eq!(f.evaluate(-1.0), 0.0);
        assert_eq!(f.evaluate(2.0), 0.0);

        assert!((f.evaluate_inverse(0.5) - 0.25).abs() < 1e-12);
    }

    #[test]
    fn concatenate_and_inverse() {
        // Concatenating identity with itself is identity.
        let cat = Dng1dConcatenate::new(Box::new(Dng1dIdentity), Box::new(Dng1dIdentity));
        assert!(cat.is_identity());
        assert_eq!(cat.evaluate(0.42), 0.42);

        // Inverse of a piecewise linear ramp.
        let mut ramp = DngPiecewiseLinear::new();
        ramp.add(0.0, 0.0);
        ramp.add(1.0, 2.0);

        let inv = Dng1dInverse::new(Box::new(ramp));
        assert!((inv.evaluate(1.0) - 0.5).abs() < 1e-6);
        assert!((inv.evaluate_inverse(0.5) - 1.0).abs() < 1e-6);
    }
}

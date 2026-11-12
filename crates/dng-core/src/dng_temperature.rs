// Port of dng_temperature.h/.cpp — CCT temperature/tint <-> xy chromaticity
// conversion using the Wyszecki & Stiles black-body locus table.

use crate::dng_types::real64;
use crate::dng_xy_coord::DngXyCoord;

/// Scale factor between distances in uv space to a "tint" parameter.
const kTintScale: real64 = -3000.0;

#[derive(Debug, Clone, Copy)]
struct RuvT {
    r: real64,
    u: real64,
    v: real64,
    t: real64,
}

/// Table from Wyszecki & Stiles, "Color Science", second edition, page 228.
#[rustfmt::skip]
static kTempTable: [RuvT; 31] = [
    RuvT { r:   0.0, u: 0.18006, v: 0.26352, t:  -0.24341 },
    RuvT { r:  10.0, u: 0.18066, v: 0.26589, t:  -0.25479 },
    RuvT { r:  20.0, u: 0.18133, v: 0.26846, t:  -0.26876 },
    RuvT { r:  30.0, u: 0.18208, v: 0.27119, t:  -0.28539 },
    RuvT { r:  40.0, u: 0.18293, v: 0.27407, t:  -0.30470 },
    RuvT { r:  50.0, u: 0.18388, v: 0.27709, t:  -0.32675 },
    RuvT { r:  60.0, u: 0.18494, v: 0.28021, t:  -0.35156 },
    RuvT { r:  70.0, u: 0.18611, v: 0.28342, t:  -0.37915 },
    RuvT { r:  80.0, u: 0.18740, v: 0.28668, t:  -0.40955 },
    RuvT { r:  90.0, u: 0.18880, v: 0.28997, t:  -0.44278 },
    RuvT { r: 100.0, u: 0.19032, v: 0.29326, t:  -0.47888 },
    RuvT { r: 125.0, u: 0.19462, v: 0.30141, t:  -0.58204 },
    RuvT { r: 150.0, u: 0.19962, v: 0.30921, t:  -0.70471 },
    RuvT { r: 175.0, u: 0.20525, v: 0.31647, t:  -0.84901 },
    RuvT { r: 200.0, u: 0.21142, v: 0.32312, t:  -1.0182  },
    RuvT { r: 225.0, u: 0.21807, v: 0.32909, t:  -1.2168  },
    RuvT { r: 250.0, u: 0.22511, v: 0.33439, t:  -1.4512  },
    RuvT { r: 275.0, u: 0.23247, v: 0.33904, t:  -1.7298  },
    RuvT { r: 300.0, u: 0.24010, v: 0.34308, t:  -2.0637  },
    RuvT { r: 325.0, u: 0.24702, v: 0.34655, t:  -2.4681  },
    RuvT { r: 350.0, u: 0.25591, v: 0.34951, t:  -2.9641  },
    RuvT { r: 375.0, u: 0.26400, v: 0.35200, t:  -3.5814  },
    RuvT { r: 400.0, u: 0.27218, v: 0.35407, t:  -4.3633  },
    RuvT { r: 425.0, u: 0.28039, v: 0.35577, t:  -5.3762  },
    RuvT { r: 450.0, u: 0.28863, v: 0.35714, t:  -6.7262  },
    RuvT { r: 475.0, u: 0.29685, v: 0.35823, t:  -8.5955  },
    RuvT { r: 500.0, u: 0.30505, v: 0.35907, t: -11.324   },
    RuvT { r: 525.0, u: 0.31320, v: 0.35968, t: -15.628   },
    RuvT { r: 550.0, u: 0.32129, v: 0.36011, t: -23.325   },
    RuvT { r: 575.0, u: 0.32931, v: 0.36038, t: -40.770   },
    RuvT { r: 600.0, u: 0.33724, v: 0.36051, t: -116.45   },
];

/// Port of `dng_temperature`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DngTemperature {
    temperature: real64,
    tint: real64,
}

impl DngTemperature {
    pub fn new() -> Self {
        DngTemperature::default()
    }

    /// `dng_temperature (temperature, tint)`.
    pub fn from_temperature_tint(temperature: real64, tint: real64) -> Self {
        DngTemperature {
            temperature,
            tint,
        }
    }

    /// `dng_temperature (xy)`.
    pub fn from_xy(xy: DngXyCoord) -> Self {
        let mut t = DngTemperature::default();
        t.set_xy_coord(&xy);
        t
    }

    /// `SetTemperature (temperature)`.
    pub fn set_temperature(&mut self, temperature: real64) {
        self.temperature = temperature;
    }

    /// `Temperature ()`.
    pub fn temperature(&self) -> real64 {
        self.temperature
    }

    /// `SetTint (tint)`.
    pub fn set_tint(&mut self, tint: real64) {
        self.tint = tint;
    }

    /// `Tint ()`.
    pub fn tint(&self) -> real64 {
        self.tint
    }

    /// `Set_xy_coord (xy)`.
    pub fn set_xy_coord(&mut self, xy: &DngXyCoord) {
        // Convert to uv space.
        let u = 2.0 * xy.x / (1.5 - xy.x + 6.0 * xy.y);
        let v = 3.0 * xy.y / (1.5 - xy.x + 6.0 * xy.y);

        // Search for line pair coordinate is between.
        let mut last_dt: real64 = 0.0;
        let mut last_dv: real64 = 0.0;
        let mut last_du: real64 = 0.0;

        for index in 1..=30usize {
            // Convert slope to delta-u and delta-v, with length 1.
            let mut du = 1.0;
            let mut dv = kTempTable[index].t;

            let mut len = (1.0 + dv * dv).sqrt();

            du /= len;
            dv /= len;

            // Find delta from black body point to test coordinate.
            let mut uu = u - kTempTable[index].u;
            let mut vv = v - kTempTable[index].v;

            // Find distance above or below line.
            let mut dt = -uu * dv + vv * du;

            // If below line, we have found line pair.
            if dt <= 0.0 || index == 30 {
                // Find fractional weight of two lines.
                if dt > 0.0 {
                    dt = 0.0;
                }

                dt = -dt;

                let f = if index == 1 { 0.0 } else { dt / (last_dt + dt) };

                // Interpolate the temperature.
                self.temperature = 1.0e6
                    / (kTempTable[index - 1].r * f + kTempTable[index].r * (1.0 - f));

                // Find delta from black body point to test coordinate.
                uu = u - (kTempTable[index - 1].u * f + kTempTable[index].u * (1.0 - f));
                vv = v - (kTempTable[index - 1].v * f + kTempTable[index].v * (1.0 - f));

                // Interpolate vectors along slope.
                du = du * (1.0 - f) + last_du * f;
                dv = dv * (1.0 - f) + last_dv * f;

                len = (du * du + dv * dv).sqrt();

                du /= len;
                dv /= len;

                // Find distance along slope.
                self.tint = (uu * du + vv * dv) * kTintScale;

                break;
            }

            // Try next line pair.
            last_dt = dt;
            last_du = du;
            last_dv = dv;
        }
    }

    /// `Get_xy_coord ()`.
    pub fn get_xy_coord(&self) -> DngXyCoord {
        let mut result = DngXyCoord::default();

        // Find inverse temperature to use as index.
        let r = 1.0e6 / self.temperature;

        // Convert tint to offset in uv space.
        let offset = self.tint * (1.0 / kTintScale);

        // Search for line pair containing coordinate.
        for index in 0..=29usize {
            if r < kTempTable[index + 1].r || index == 29 {
                // Find relative weight of first line.
                let f = (kTempTable[index + 1].r - r)
                    / (kTempTable[index + 1].r - kTempTable[index].r);

                // Interpolate the black body coordinates.
                let mut u = kTempTable[index].u * f + kTempTable[index + 1].u * (1.0 - f);
                let mut v = kTempTable[index].v * f + kTempTable[index + 1].v * (1.0 - f);

                // Find vectors along slope for each line.
                let mut uu1 = 1.0;
                let mut vv1 = kTempTable[index].t;

                let mut uu2 = 1.0;
                let mut vv2 = kTempTable[index + 1].t;

                let len1 = (1.0 + vv1 * vv1).sqrt();
                let len2 = (1.0 + vv2 * vv2).sqrt();

                uu1 /= len1;
                vv1 /= len1;

                uu2 /= len2;
                vv2 /= len2;

                // Find vector from black body point.
                let mut uu3 = uu1 * f + uu2 * (1.0 - f);
                let mut vv3 = vv1 * f + vv2 * (1.0 - f);

                let len3 = (uu3 * uu3 + vv3 * vv3).sqrt();

                uu3 /= len3;
                vv3 /= len3;

                // Adjust coordinate along this vector.
                u += uu3 * offset;
                v += vv3 * offset;

                // Convert to xy coordinates.
                result.x = 1.5 * u / (u - 4.0 * v + 2.0);
                result.y = v / (u - 4.0 * v + 2.0);

                break;
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_temperature() {
        // D65 (6504 K) round-trips approximately.
        let xy = crate::dng_xy_coord::d65_xy_coord();
        let t = DngTemperature::from_xy(xy);
        assert!((t.temperature() - 6504.0).abs() < 60.0, "temp={}", t.temperature());
        assert!(t.tint().abs() < 12.0, "tint={}", t.tint());

        let back = t.get_xy_coord();
        assert!((back.x - xy.x).abs() < 0.002, "x={} vs {}", back.x, xy.x);
        assert!((back.y - xy.y).abs() < 0.002, "y={} vs {}", back.y, xy.y);
    }

    #[test]
    fn roundtrip_std_a() {
        // StdA illuminant is 2856 K by definition.
        let xy = crate::dng_xy_coord::std_a_xy_coord();
        let t = DngTemperature::from_xy(xy);
        assert!((t.temperature() - 2856.0).abs() < 30.0, "temp={}", t.temperature());

        // Zero tint should map back to the source xy.
        let mut exact = DngTemperature::from_temperature_tint(t.temperature(), 0.0);
        exact.set_tint(0.0);
        let back = exact.get_xy_coord();
        assert!((back.x - xy.x).abs() < 0.005);
    }

    #[test]
    fn tint_moves_perpendicular() {
        let base = DngTemperature::from_temperature_tint(5000.0, 0.0);
        let plus = DngTemperature::from_temperature_tint(5000.0, 10.0);
        let minus = DngTemperature::from_temperature_tint(5000.0, -10.0);

        let b = base.get_xy_coord();
        let p = plus.get_xy_coord();
        let m = minus.get_xy_coord();

        // Tint moves the chromaticity off the black-body locus; both signs
        // move it (the exact direction depends on the locus slope).
        assert!(p != b);
        assert!(m != b);
        assert!(p != m);
    }
}

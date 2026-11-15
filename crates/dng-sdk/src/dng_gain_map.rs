// Port of dng_gain_map.h/.cpp — spatial gain maps and gain table maps.
//
// The buffer is a typed `Vec<f32>` (replacing `dng_memory_block` real32
// views); the interpolator is a 1:1 port of the cpp-local
// `dng_gain_map_interpolator` class.

use crate::dng_core::dng_exceptions::program_error;
use crate::dng_core::dng_fingerprint::DngFingerprint;
use crate::dng_core::{
    compute_buffer_size, convert_double_to_int32, DngPoint, DngPointReal64, DngRect, DngResult,
    DngStream, PaddingType, ttFloat,
};

/// Port of `dng_gain_map`.
#[derive(Debug, Clone)]
pub struct DngGainMap {
    points: DngPoint,
    spacing: DngPointReal64,
    origin: DngPointReal64,
    planes: u32,
    row_step: u32,
    buffer: Vec<f32>,
}

impl DngGainMap {
    /// `dng_gain_map (allocator, points, spacing, origin, planes)`.
    pub fn new(
        points: DngPoint,
        spacing: DngPointReal64,
        origin: DngPointReal64,
        planes: u32,
    ) -> DngResult<Self> {
        let row_step = crate::dng_core::dng_safe_arithmetic::safe_uint32_mult2(planes, points.h as u32)?;

        let bytes = compute_buffer_size(ttFloat, points, planes, PaddingType::padSIMDBytes)?;
        let floats = (bytes / 4) as usize;

        Ok(DngGainMap {
            points,
            spacing,
            origin,
            planes,
            row_step,
            buffer: vec![0.0; floats],
        })
    }

    /// `Points ()`.
    pub fn points(&self) -> DngPoint {
        self.points
    }

    /// `Spacing ()`.
    pub fn spacing(&self) -> DngPointReal64 {
        self.spacing
    }

    /// `Origin ()`.
    pub fn origin(&self) -> DngPointReal64 {
        self.origin
    }

    /// `Planes ()`.
    pub fn planes(&self) -> u32 {
        self.planes
    }

    /// `Entry (rowIndex, colIndex, plane)`.
    #[inline]
    pub fn entry(&self, row_index: u32, col_index: u32, plane: u32) -> f32 {
        self.buffer[(row_index * self.row_step + col_index * self.planes + plane) as usize]
    }

    /// Mutable `Entry`.
    #[inline]
    pub fn entry_mut(&mut self, row_index: u32, col_index: u32, plane: u32) -> &mut f32 {
        let index = (row_index * self.row_step + col_index * self.planes + plane) as usize;
        &mut self.buffer[index]
    }

    /// `Interpolate (row, col, plane, bounds)`.
    pub fn interpolate(&self, row: i32, col: i32, plane: u32, bounds: DngRect) -> DngResult<f32> {
        let mut interp = GainMapInterpolator::new(self, bounds, row, col, plane)?;
        Ok(interp.interpolate())
    }

    /// `PutStreamSize ()`.
    pub fn put_stream_size(&self) -> u32 {
        44 + self.points.v as u32 * self.points.h as u32 * self.planes * 4
    }

    /// `PutStream (stream)`.
    pub fn put_stream(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(self.points.v as u32)?;
        stream.put_uint32(self.points.h as u32)?;

        stream.put_real64(self.spacing.v)?;
        stream.put_real64(self.spacing.h)?;

        stream.put_real64(self.origin.v)?;
        stream.put_real64(self.origin.h)?;

        stream.put_uint32(self.planes)?;

        for row in 0..self.points.v as u32 {
            for col in 0..self.points.h as u32 {
                for plane in 0..self.planes {
                    stream.put_real32(self.entry(row, col, plane))?;
                }
            }
        }

        Ok(())
    }

    /// `GetStream (host, stream)`.
    pub fn get_stream(stream: &mut DngStream) -> DngResult<Self> {
        let mut map_points = DngPoint::new(0, 0);
        map_points.v = stream.get_uint32()? as i32;
        map_points.h = stream.get_uint32()? as i32;

        let mut map_spacing = DngPointReal64::new(0.0, 0.0);
        map_spacing.v = stream.get_real64()?;
        map_spacing.h = stream.get_real64()?;

        let mut map_origin = DngPointReal64::new(0.0, 0.0);
        map_origin.v = stream.get_real64()?;
        map_origin.h = stream.get_real64()?;

        let map_planes = stream.get_uint32()?;

        if map_points.v == 1 {
            map_spacing.v = 1.0;
            map_origin.v = 0.0;
        }

        if map_points.h == 1 {
            map_spacing.h = 1.0;
            map_origin.h = 0.0;
        }

        if map_points.v < 1 || map_points.h < 1 || map_planes < 1 {
            return Err(program_error()); // "Bad gain map parameters"
        }

        if map_points.v > 1024 || map_points.h > 1024 || map_planes > 16 {
            return Err(program_error()); // Sanity limits (C++ validate block).
        }

        let mut map = DngGainMap::new(map_points, map_spacing, map_origin, map_planes)?;

        for row in 0..map_points.v as u32 {
            for col in 0..map_points.h as u32 {
                for plane in 0..map_planes {
                    let value = stream.get_real32()?;
                    *map.entry_mut(row, col, plane) = value;
                }
            }
        }

        Ok(map)
    }
}

/// Port of the cpp-local `dng_gain_map_interpolator`.
pub struct GainMapInterpolator<'a> {
    map: &'a DngGainMap,
    scale: DngPointReal64,
    offset: DngPointReal64,
    column: i32,
    plane: u32,
    row_index1: u32,
    row_index2: u32,
    row_fract: f32,
    reset_column: i32,
    value_base: f32,
    value_step: f32,
    value_index: f32,
}

impl<'a> GainMapInterpolator<'a> {
    /// `dng_gain_map_interpolator (map, mapBounds, row, column, plane)`.
    pub fn new(
        map: &'a DngGainMap,
        map_bounds: DngRect,
        row: i32,
        column: i32,
        plane: u32,
    ) -> DngResult<Self> {
        let scale = DngPointReal64::new(1.0 / map_bounds.h() as f64, 1.0 / map_bounds.w() as f64);

        let offset = DngPointReal64::new(0.5 - map_bounds.t as f64, 0.5 - map_bounds.l as f64);

        let mut interp = GainMapInterpolator {
            map,
            scale,
            offset,
            column,
            plane,
            row_index1: 0,
            row_index2: 0,
            row_fract: 0.0,
            reset_column: 0,
            value_base: 0.0,
            value_step: 0.0,
            value_index: 0.0,
        };

        let row_index_f = (interp.scale.v * (row as f64 + interp.offset.v) - map.origin().v)
            / map.spacing().v;

        if row_index_f <= 0.0 {
            interp.row_index1 = 0;
            interp.row_index2 = 0;
            interp.row_fract = 0.0;
        } else {
            if map.points().v < 1 {
                return Err(program_error()); // "Empty gain map"
            }

            let last_row = map.points().v - 1;

            if row_index_f >= last_row as f64 {
                interp.row_index1 = last_row as u32;
                interp.row_index2 = interp.row_index1;
                interp.row_fract = 0.0;
            } else {
                let row_index = row_index_f as u32;

                interp.row_index1 = row_index;
                interp.row_index2 = row_index + 1;
                interp.row_fract = (row_index_f - row_index as f64) as f32;
            }
        }

        interp.reset_column()?;

        Ok(interp)
    }

    /// `InterpolateEntry (colIndex)`.
    fn interpolate_entry(&self, col_index: u32) -> f32 {
        self.map.entry(self.row_index1, col_index, self.plane) * (1.0 - self.row_fract)
            + self.map.entry(self.row_index2, col_index, self.plane) * self.row_fract
    }

    /// `ResetColumn ()`.
    fn reset_column(&mut self) -> DngResult<()> {
        let col_index_f = (self.scale.h * (self.column as f64 + self.offset.h)
            - self.map.origin().h)
            / self.map.spacing().h;

        if col_index_f <= 0.0 {
            self.value_base = self.interpolate_entry(0);
            self.value_step = 0.0;
            self.reset_column = ((self.map.origin().h / self.scale.h - self.offset.h).ceil())
                as i32;
        } else {
            if self.map.points().h < 1 {
                return Err(program_error()); // "Empty gain map"
            }

            let last_col = self.map.points().h - 1;

            if col_index_f >= last_col as f64 {
                self.value_base = self.interpolate_entry(last_col as u32);
                self.value_step = 0.0;
                self.reset_column = 0x7FFFFFFF;
            } else {
                let col_index = col_index_f as u32;

                let base = self.interpolate_entry(col_index) as f64;
                let delta = self.interpolate_entry(col_index + 1) as f64 - base;

                self.value_base =
                    (base + delta * (col_index_f - col_index as f64)) as f32;

                self.value_step = ((delta * self.scale.h) / self.map.spacing().h) as f32;

                self.reset_column = convert_double_to_int32(
                    (((col_index as f64 + 1.0) * self.map.spacing().h + self.map.origin().h)
                        / self.scale.h
                        - self.offset.h)
                        .ceil(),
                )?;
            }
        }

        self.value_index = 0.0;

        Ok(())
    }

    /// `Interpolate ()`.
    pub fn interpolate(&self) -> f32 {
        self.value_base + self.value_step * self.value_index
    }

    /// `Increment ()`.
    pub fn increment(&mut self) -> DngResult<()> {
        self.column += 1;
        if self.column >= self.reset_column {
            self.reset_column()?;
        } else {
            self.value_index += 1.0;
        }
        Ok(())
    }
}

/// Port of `dng_gain_table_map` — the ProfileGainTableMap structure.
#[derive(Debug, Clone)]
pub struct DngGainTableMap {
    points: DngPoint,
    spacing: DngPointReal64,
    origin: DngPointReal64,
    num_table_points: u32,
    row_step: u32,
    col_step: u32,
    num_samples: u32,
    sample_bytes: u32,
    map_input_weights: [f32; 5],
    data_type: u32,
    gamma: f32,
    gain_min: f32,
    gain_max: f32,
    buffer: Vec<f32>,
    fingerprint: std::cell::Cell<DngFingerprint>,
}

impl DngGainTableMap {
    /// `dng_gain_table_map (...)`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        points: DngPoint,
        spacing: DngPointReal64,
        origin: DngPointReal64,
        num_table_points: u32,
        weights: &[f32; 5],
        data_type: u32,
        gamma: f32,
        gain_min: f32,
        gain_max: f32,
    ) -> DngResult<Self> {
        let row_step = num_table_points;
        let col_step = 1;
        let num_samples = points.v as u32 * points.h as u32 * row_step * col_step;
        let sample_bytes = num_samples * 2; // uint16 samples on the wire

        Ok(DngGainTableMap {
            points,
            spacing,
            origin,
            num_table_points,
            row_step,
            col_step,
            num_samples,
            sample_bytes,
            map_input_weights: *weights,
            data_type,
            gamma,
            gain_min,
            gain_max,
            buffer: vec![0.0; num_samples as usize],
            fingerprint: std::cell::Cell::new(DngFingerprint::new()),
        })
    }

    pub fn points(&self) -> DngPoint {
        self.points
    }
    pub fn spacing(&self) -> DngPointReal64 {
        self.spacing
    }
    pub fn origin(&self) -> DngPointReal64 {
        self.origin
    }
    pub fn num_table_points(&self) -> u32 {
        self.num_table_points
    }
    pub fn num_samples(&self) -> u32 {
        self.num_samples
    }
    pub fn sample_bytes(&self) -> u32 {
        self.sample_bytes
    }
    pub fn map_input_weights(&self) -> &[f32; 5] {
        &self.map_input_weights
    }
    pub fn data_type(&self) -> u32 {
        self.data_type
    }
    pub fn gamma(&self) -> f32 {
        self.gamma
    }
    pub fn gain_min(&self) -> f32 {
        self.gain_min
    }
    pub fn gain_max(&self) -> f32 {
        self.gain_max
    }

    /// `Entry (rowIndex, colIndex, tableIndex)`.
    pub fn entry(&self, row_index: u32, col_index: u32, table_index: u32) -> f32 {
        self.buffer[(row_index * self.row_step + col_index * self.col_step + table_index) as usize]
    }

    pub fn entry_mut(&mut self, row_index: u32, col_index: u32, table_index: u32) -> &mut f32 {
        let index =
            (row_index * self.row_step + col_index * self.col_step + table_index) as usize;
        &mut self.buffer[index]
    }

    /// `SetFingerprint / GetFingerprint` — cached digest of the wire format.
    pub fn set_fingerprint(&self, fp: DngFingerprint) {
        self.fingerprint.set(fp);
    }

    pub fn get_fingerprint(&self) -> DngFingerprint {
        self.fingerprint.get()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_access_and_stream_roundtrip() {
        let mut map = DngGainMap::new(
            DngPoint::new(2, 3),
            DngPointReal64::new(0.5, 0.25),
            DngPointReal64::new(0.0, 0.0),
            2,
        )
        .unwrap();

        assert_eq!(map.points(), DngPoint::new(2, 3));
        assert_eq!(map.planes(), 2);

        *map.entry_mut(0, 0, 0) = 1.5;
        *map.entry_mut(1, 2, 1) = 2.5;

        assert_eq!(map.entry(0, 0, 0), 1.5);
        assert_eq!(map.entry(1, 2, 1), 2.5);

        // Stream round trip.
        let mut stream = crate::dng_core::dng_memory_stream::DngMemoryStream::new().stream();
        map.put_stream(&mut stream).unwrap();
        stream.set_read_position(0).unwrap();

        let back = DngGainMap::get_stream(&mut stream).unwrap();
        assert_eq!(back.entry(0, 0, 0), 1.5);
        assert_eq!(back.entry(1, 2, 1), 2.5);
        assert_eq!(back.put_stream_size(), map.put_stream_size());
    }

    #[test]
    fn interpolate_flat_map_is_constant() {
        // Uniform gain map: interpolation must return the same value
        // everywhere.
        let mut map = DngGainMap::new(
            DngPoint::new(2, 2),
            DngPointReal64::new(0.5, 0.5),
            DngPointReal64::new(0.0, 0.0),
            1,
        )
        .unwrap();

        for row in 0..2 {
            for col in 0..2 {
                *map.entry_mut(row, col, 0) = 2.0;
            }
        }

        let bounds = DngRect::from_hw(100, 100);

        for (row, col) in [(0i32, 0i32), (50, 50), (99, 99), (10, 90)] {
            let v = map.interpolate(row, col, 0, bounds).unwrap();
            assert!((v - 2.0).abs() < 1e-6, "({row},{col}) = {v}");
        }
    }

    #[test]
    fn interpolate_ramps() {
        // Horizontal ramp: entry (r, c) = c as f32.  Spacing 0.25 means each
        // map cell covers 25% of the width.
        let mut map = DngGainMap::new(
            DngPoint::new(1, 2),
            DngPointReal64::new(1.0, 0.5),
            DngPointReal64::new(0.0, 0.0),
            1,
        )
        .unwrap();

        *map.entry_mut(0, 0, 0) = 0.0;
        *map.entry_mut(0, 1, 0) = 1.0;

        let bounds = DngRect::from_hw(100, 100);

        // Column 0 -> 0.0, column 99 -> near 1.0 (plane 0).
        let left = map.interpolate(0, 0, 0, bounds).unwrap();
        let right = map.interpolate(0, 99, 0, bounds).unwrap();

        assert!(left < 0.1, "left={left}");
        assert!(right > 0.9, "right={right}");

        // Monotone across a scanline (via Increment).
        let mut interp = GainMapInterpolator::new(&map, bounds, 0, 0, 0).unwrap();
        let mut prev = interp.interpolate();
        for _ in 0..99 {
            interp.increment().unwrap();
            let v = interp.interpolate();
            assert!(v >= prev - 1e-6);
            prev = v;
        }
    }

    #[test]
    fn gain_table_map_basics() {
        let map = DngGainTableMap::new(
            DngPoint::new(2, 2),
            DngPointReal64::new(0.5, 0.5),
            DngPointReal64::new(0.0, 0.0),
            4,
            &[1.0, 0.5, 0.25, 0.125, 0.0625],
            2,
            1.0,
            1.0,
            4.0,
        )
        .unwrap();

        assert_eq!(map.num_table_points(), 4);
        assert_eq!(map.num_samples(), 2 * 2 * 4);
        assert_eq!(map.sample_bytes(), 2 * 2 * 4 * 2);
        assert_eq!(map.map_input_weights()[1], 0.5);
    }
}

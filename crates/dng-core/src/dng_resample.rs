// Port of dng_resample.h/.cpp — resampling functions, sub-pixel
// coordinates, 1D/2D weight tables and the ResampleImage filter task.

use crate::dng_area_task::{DngAreaTask, DngFilterTaskBase};
use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::{bad_format, memory_full};
use crate::dng_image::DngImage;
use crate::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use crate::dng_point::DngPoint;
use crate::dng_rect::DngRect;
use crate::dng_types::{int16, int32, real32, real64, uint16, uint32};
use crate::dng_utils::{max_real64, min_real64, round_int32_f64};
use crate::dng_safe_arithmetic::{
    round_up_uint32_to_multiple, safe_int32_add, safe_int32_sub, safe_uint32_mult2,
    convert_uint32_to_int32,
};

/// Port of `dng_resample_function` (abstract).
pub trait DngResampleFunction: Send + Sync {
    /// `Extent ()`.
    fn extent(&self) -> real64;

    /// `Evaluate (x)`.
    fn evaluate(&self, x: real64) -> real64;
}

/// Port of `dng_resample_bicubic`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DngResampleBicubic;

impl DngResampleFunction for DngResampleBicubic {
    fn extent(&self) -> real64 {
        2.0
    }

    fn evaluate(&self, x: real64) -> real64 {
        const A: real64 = -0.75;

        let x = crate::dng_utils::abs_real64(x);

        if x >= 2.0 {
            0.0
        } else if x >= 1.0 {
            ((A * x - 5.0 * A) * x + 8.0 * A) * x - 4.0 * A
        } else {
            ((A + 2.0) * x - (A + 3.0)) * x * x + 1.0
        }
    }
}

impl DngResampleBicubic {
    /// `dng_resample_bicubic::Get ()`.
    pub fn get() -> &'static dyn DngResampleFunction {
        static INSTANCE: std::sync::OnceLock<DngResampleBicubic> = std::sync::OnceLock::new();
        INSTANCE.get_or_init(DngResampleBicubic::default)
    }
}

// ------------------------------------------------------------------ coords

pub const kResampleSubsampleBits: uint32 = 7;
pub const kResampleSubsampleCount: uint32 = 1 << kResampleSubsampleBits;
pub const kResampleSubsampleMask: uint32 = kResampleSubsampleCount - 1;

/// Port of `dng_resample_coords`.
#[derive(Debug, Clone, Default)]
pub struct DngResampleCoords {
    origin: int32,
    coords: Vec<int32>,
}

impl DngResampleCoords {
    pub fn new() -> Self {
        DngResampleCoords::default()
    }

    /// `Initialize (srcOrigin, dstOrigin, srcCount, dstCount)`.
    pub fn initialize(
        &mut self,
        src_origin: int32,
        dst_origin: int32,
        src_count: uint32,
        dst_count: uint32,
    ) -> DngResult<()> {
        self.origin = dst_origin;

        let dst_entries = round_up_uint32_to_multiple(dst_count, 8)?;

        let mut coords = vec![0i32; dst_entries as usize];

        let inv_scale = src_count as real64 / dst_count as real64;

        for j in 0..dst_count as usize {
            let x = j as real64 + 0.5;
            let y = x * inv_scale - 0.5 + src_origin as real64;

            coords[j] = round_int32_f64(y * kResampleSubsampleCount as real64);
        }

        // Pad out table by replicating last entry.
        if dst_count > 0 {
            let last = coords[dst_count as usize - 1];
            for k in dst_count as usize..dst_entries as usize {
                coords[k] = last;
            }
        }

        self.coords = coords;

        Ok(())
    }

    /// `Coords (index)` — slice starting at `index`.
    pub fn coords(&self, index: int32) -> &[int32] {
        let offset = (index - self.origin) as usize;
        &self.coords[offset..]
    }

    /// `Pixel (index)`.
    pub fn pixel(&self, index: int32) -> int32 {
        self.coords(index)[0] >> kResampleSubsampleBits
    }
}

// ------------------------------------------------------------------ weights

/// Port of `dng_resample_weights`.
#[derive(Debug, Clone, Default)]
pub struct DngResampleWeights {
    radius: uint32,
    weight_step: uint32,
    weights32: Vec<real32>,
    weights16: Vec<int16>,
}

impl DngResampleWeights {
    pub fn new() -> Self {
        DngResampleWeights::default()
    }

    /// `Initialize (scale, kernel)`.
    pub fn initialize(&mut self, scale: real64, kernel: &dyn DngResampleFunction) -> DngResult<()> {
        // We only adjust the kernel size for scale factors less than 1.0.
        let scale = min_real64(scale, 1.0);

        // Find radius of this kernel.
        self.radius = (kernel.extent() / scale + 0.9999) as uint32;

        // Width is twice the radius.
        let width = self.radius * 2;

        // Round to each set of weights to a multiple of 8 entries.
        self.weight_step = round_up_uint32_to_multiple(width, 8)?;

        let buffer_len = self.weight_step as usize * kResampleSubsampleCount as usize;

        self.weights32 = vec![0.0; buffer_len];
        self.weights16 = vec![0; buffer_len];

        // Compute kernel for each subsample value.
        for sample in 0..kResampleSubsampleCount {
            let fract = sample as real64 * (1.0 / kResampleSubsampleCount as real64);

            let w32_base = self.weight_step as usize * sample as usize;

            // Evaluate kernel function for 32 bit weights.
            {
                let mut t32: real64 = 0.0;

                for j in 0..width as usize {
                    let k = j as int32 - self.radius as int32 + 1;
                    let x = (k as real64 - fract) * scale;

                    self.weights32[w32_base + j] = kernel.evaluate(x) as real32;
                    t32 += self.weights32[w32_base + j] as real64;
                }

                // Scale 32 bit weights so total of weights is 1.0.
                let s32 = (1.0 / t32) as real32;

                for j in 0..width as usize {
                    self.weights32[w32_base + j] *= s32;
                }
            }

            // Round off 32 bit weights to 16 bit weights.
            {
                let w16_base = w32_base;

                let mut t16: int32 = 0;

                for j in 0..width as usize {
                    self.weights16[w16_base + j] =
                        round_int32_f64(self.weights32[w32_base + j] as real64 * 16384.0) as int16;
                    t16 += self.weights16[w16_base + j] as int32;
                }

                // Adjust center entry for any round off error so total is
                // exactly 16384.
                let center = self.radius as usize - if fract >= 0.5 { 0 } else { 1 };
                self.weights16[w16_base + center] += (16384 - t16) as int16;
            }
        }

        Ok(())
    }

    /// `Radius ()`.
    pub fn radius(&self) -> uint32 {
        self.radius
    }

    /// `Width ()`.
    pub fn width(&self) -> uint32 {
        self.radius * 2
    }

    /// `Offset ()`.
    pub fn offset(&self) -> int32 {
        1 - self.radius as int32
    }

    /// `Step ()`.
    pub fn step(&self) -> uint32 {
        self.weight_step
    }

    /// `Weights32 (fract)`.
    pub fn weights32(&self, fract: uint32) -> DngResult<&[real32]> {
        if fract >= kResampleSubsampleCount {
            return Err(bad_format());
        }
        let base = self.weight_step as usize * fract as usize;
        Ok(&self.weights32[base..])
    }

    /// `Weights32BufferLogicalSize ()`.
    pub fn weights32_buffer_logical_size(&self) -> uint32 {
        (self.weights32.len() * std::mem::size_of::<real32>()) as uint32
    }

    /// `Weights16 (fract)`.
    pub fn weights16(&self, fract: uint32) -> DngResult<&[int16]> {
        if fract >= kResampleSubsampleCount {
            return Err(bad_format());
        }
        let base = self.weight_step as usize * fract as usize;
        Ok(&self.weights16[base..])
    }

    /// `Weights16BufferLogicalSize ()`.
    pub fn weights16_buffer_logical_size(&self) -> uint32 {
        (self.weights16.len() * std::mem::size_of::<int16>()) as uint32
    }
}

// ------------------------------------------------------------------ 2d weights

pub const kResampleSubsampleBits2D: uint32 = 5;
pub const kResampleSubsampleCount2D: uint32 = 1 << kResampleSubsampleBits2D;
pub const kResampleSubsampleMask2D: uint32 = kResampleSubsampleCount2D - 1;

/// Port of `dng_resample_weights_2d`.
#[derive(Debug, Clone, Default)]
pub struct DngResampleWeights2d {
    radius: uint32,
    row_step: uint32,
    col_step: uint32,
    weights32: Vec<real32>,
    weights16: Vec<int16>,
}

impl DngResampleWeights2d {
    pub fn new() -> Self {
        DngResampleWeights2d::default()
    }

    /// `Initialize (kernel)`.
    pub fn initialize(&mut self, kernel: &dyn DngResampleFunction) -> DngResult<()> {
        // Find radius of this kernel.  Unlike the 1d case, never scaled.
        self.radius = (kernel.extent() + 0.9999) as uint32;

        let width = safe_uint32_mult2(self.radius, 2)?;
        let width_sqr = safe_uint32_mult2(width, width)?;
        let step = round_up_uint32_to_multiple(width_sqr, 8)?;
        self.row_step = safe_uint32_mult2(step, kResampleSubsampleCount2D)?;
        self.col_step = step;

        let buffer_len = self.row_step as usize
            * kResampleSubsampleCount2D as usize
            * kResampleSubsampleCount2D as usize;

        self.weights32 = vec![0.0; buffer_len];
        self.weights16 = vec![0; buffer_len];

        let weights32_count = self.row_step as usize / kResampleSubsampleCount2D as usize
            * kResampleSubsampleCount2D as usize;

        let _ = weights32_count;

        for y in 0..kResampleSubsampleCount2D {
            let y_fract = y as real64 * (1.0 / kResampleSubsampleCount2D as real64);

            for x in 0..kResampleSubsampleCount2D {
                let x_fract = x as real64 * (1.0 / kResampleSubsampleCount2D as real64);

                let w32_base = self.weights32_offset(y, x);

                // Evaluate kernel function for 32 bit weights.
                {
                    let mut t32: real64 = 0.0;

                    let mut index = 0usize;

                    for i in 0..width {
                        let y_int = i as int32 - self.radius as int32 + 1;
                        let y_pos = y_int as real64 - y_fract;

                        for j in 0..width {
                            let x_int = j as int32 - self.radius as int32 + 1;
                            let x_pos = x_int as real64 - x_fract;

                            // Separable.
                            self.weights32[w32_base + index] =
                                (kernel.evaluate(x_pos) * kernel.evaluate(y_pos)) as real32;

                            t32 += self.weights32[w32_base + index] as real64;
                            index += 1;
                        }
                    }

                    // Scale 32 bit weights so total of weights is 1.0.
                    let s32 = (1.0 / t32) as real32;

                    for i in 0..width_sqr as usize {
                        self.weights32[w32_base + i] *= s32;
                    }
                }

                // Round off 32 bit weights to 16 bit weights.
                {
                    let w16_base = w32_base;

                    let mut t16: int32 = 0;

                    for j in 0..width_sqr as usize {
                        self.weights16[w16_base + j] = round_int32_f64(
                            self.weights32[w32_base + j] as real64 * 16384.0,
                        ) as int16;
                        t16 += self.weights16[w16_base + j] as int32;
                    }

                    // Adjust one of the center entries so total is 16384.
                    let x_offset = self.radius as usize - if x_fract >= 0.5 { 0 } else { 1 };
                    let y_offset = self.radius as usize - if y_fract >= 0.5 { 0 } else { 1 };
                    let center_offset = width as usize * y_offset + x_offset;

                    self.weights16[w16_base + center_offset] += (16384 - t16) as int16;
                }
            }
        }

        Ok(())
    }

    fn weights32_offset(&self, y: uint32, x: uint32) -> usize {
        (self.row_step as usize * y as usize) + (self.col_step as usize * x as usize)
    }

    /// `Weights32 (point)` — returns slice at (y, x) subsample.
    pub fn weights32_at(&self, y: uint32, x: uint32) -> DngResult<&[real32]> {
        if y >= kResampleSubsampleCount2D || x >= kResampleSubsampleCount2D {
            return Err(bad_format());
        }
        let base = self.weights32_offset(y, x);
        Ok(&self.weights32[base..])
    }

    /// `Weights16 (point)`.
    pub fn weights16_at(&self, y: uint32, x: uint32) -> DngResult<&[int16]> {
        if y >= kResampleSubsampleCount2D || x >= kResampleSubsampleCount2D {
            return Err(bad_format());
        }
        let base = self.weights32_offset(y, x);
        Ok(&self.weights16[base..])
    }

    /// `Radius ()`.
    pub fn radius(&self) -> uint32 {
        self.radius
    }

    /// `Width ()`.
    pub fn width(&self) -> uint32 {
        self.radius * 2
    }

    /// `Offset ()`.
    pub fn offset(&self) -> int32 {
        1 - self.radius as int32
    }

    /// `RowStep ()`.
    pub fn row_step(&self) -> uint32 {
        self.row_step
    }

    /// `ColStep ()`.
    pub fn col_step(&self) -> uint32 {
        self.col_step
    }

    /// `Weights32BufferLogicalSize ()`.
    pub fn weights32_buffer_logical_size(&self) -> uint32 {
        (self.weights32.len() * std::mem::size_of::<real32>()) as uint32
    }

    /// `Weights16BufferLogicalSize ()`.
    pub fn weights16_buffer_logical_size(&self) -> uint32 {
        (self.weights16.len() * std::mem::size_of::<int16>()) as uint32
    }
}

// ------------------------------------------------------------------ task

/// Port of `dng_resample_task` (private in C++).
pub struct DngResampleTask {
    filter: DngFilterTaskBase,
    src_bounds: DngRect,
    dst_bounds: DngRect,
    row_scale: real64,
    col_scale: real64,
    row_coords: DngResampleCoords,
    col_coords: DngResampleCoords,
    weights_v: DngResampleWeights,
    weights_h: DngResampleWeights,
    src_tile_size: DngPoint,
}

impl DngResampleTask {
    pub fn new(
        src_image: crate::dng_image::ImageRef,
        dst_image: crate::dng_image::ImageRef,
        src_bounds: DngRect,
        dst_bounds: DngRect,
    ) -> DngResult<Self> {
        let mut filter = DngFilterTaskBase::new("dng_resample_task", src_image.clone(), dst_image.clone());

        let (src_pixel_size, src_pixel_range, dst_pixel_size, dst_pixel_range) = {
            let src = src_image.read().unwrap();
            let dst = dst_image.read().unwrap();
            (
                src.pixel_size(),
                src.pixel_range(),
                dst.pixel_size(),
                dst.pixel_range(),
            )
        };

        if src_pixel_size <= 2 && dst_pixel_size <= 2 && src_pixel_range == dst_pixel_range {
            filter.src_pixel_type = crate::dng_tag_types::ttShort;
            filter.dst_pixel_type = crate::dng_tag_types::ttShort;
        } else {
            filter.src_pixel_type = crate::dng_tag_types::ttFloat;
            filter.dst_pixel_type = crate::dng_tag_types::ttFloat;
        }

        filter.unit_cell_override = DngPoint::new(8, 8);

        let row_scale = if src_bounds.h() != 0 {
            dst_bounds.h() as real64 / src_bounds.h() as real64
        } else {
            0.0
        };

        let col_scale = if src_bounds.w() != 0 {
            dst_bounds.w() as real64 / src_bounds.w() as real64
        } else {
            0.0
        };

        Ok(DngResampleTask {
            filter,
            src_bounds,
            dst_bounds,
            row_scale,
            col_scale,
            row_coords: DngResampleCoords::new(),
            col_coords: DngResampleCoords::new(),
            weights_v: DngResampleWeights::new(),
            weights_h: DngResampleWeights::new(),
            src_tile_size: DngPoint::new(0, 0),
        })
    }

    /// `SrcArea (dstArea)`.
    pub fn src_area(&self, dst_area: DngRect) -> DngResult<DngRect> {
        let offset_v = self.weights_v.offset();
        let offset_h = self.weights_h.offset();

        let width_v = convert_uint32_to_int32(self.weights_v.width())?;
        let width_h = convert_uint32_to_int32(self.weights_h.width())?;

        let mut src_area = DngRect::new();

        src_area.t = safe_int32_add(self.row_coords.pixel(dst_area.t), offset_v)?;
        src_area.l = safe_int32_add(self.col_coords.pixel(dst_area.l), offset_h)?;

        src_area.b = safe_int32_add(
            safe_int32_add(self.row_coords.pixel(safe_int32_sub(dst_area.b, 1)?), offset_v)?,
            width_v,
        )?;

        src_area.r = safe_int32_add(
            safe_int32_add(self.col_coords.pixel(safe_int32_sub(dst_area.r, 1)?), offset_h)?,
            width_h,
        )?;

        Ok(src_area)
    }
}

impl DngResampleTask {
    pub fn src_area_for_debug(&self, dst_area: DngRect) -> DngResult<DngRect> {
        self.src_area(dst_area)
    }

    pub fn src_image_handle(&self) -> std::sync::Arc<std::sync::RwLock<dyn crate::dng_image::DngImage>> {
        self.filter.src_image.clone()
    }
}

impl DngAreaTask for DngResampleTask {
    fn name(&self) -> &str {
        "dng_resample_task"
    }

    fn unit_cell(&self) -> DngPoint {
        self.filter.unit_cell_override
    }

    fn start(
        &mut self,
        _thread_count: uint32,
        _dst_area: DngRect,
        tile_size: DngPoint,
        _sniffer: Option<&std::sync::Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        // Compute sub-pixel resolution coordinates in the source image for
        // each row and column of the destination area.
        self.row_coords.initialize(
            self.src_bounds.t,
            self.dst_bounds.t,
            self.src_bounds.h(),
            self.dst_bounds.h(),
        )?;

        self.col_coords.initialize(
            self.src_bounds.l,
            self.dst_bounds.l,
            self.src_bounds.w(),
            self.dst_bounds.w(),
        )?;

        // Compute resampling kernels.
        self.weights_v.initialize(self.row_scale, DngResampleBicubic::get())?;
        self.weights_h.initialize(self.col_scale, DngResampleBicubic::get())?;

        // Find upper bound on source tile.
        self.src_tile_size.v = round_int32_f64(tile_size.v as real64 / self.row_scale)
            + self.weights_v.width() as int32
            + 2;
        self.src_tile_size.h = round_int32_f64(tile_size.h as real64 / self.col_scale)
            + self.weights_h.width() as int32
            + 2;

        self.filter.start_filter(_dst_area, tile_size)?;

        Ok(())
    }

    fn process(
        &mut self,
        _thread_index: uint32,
        area: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        let src_area = self.src_area(area)?;

        let src_cols = src_area.w();
        let dst_cols = area.w();

        let width_v = self.weights_v.width();
        let width_h = self.weights_h.width();

        let offset_v = self.weights_v.offset();
        let offset_h = self.weights_h.offset();

        let step_h = self.weights_h.step();

        let src_pixel_type = self.filter.src_pixel_type;

        // Read the padded source area.
        let mut src_buffer = DngPixelBuffer::with_planar_config(
            src_area,
            0,
            self.filter.src_planes,
            src_pixel_type,
            crate::dng_tag_values::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(src_area, self.filter.src_planes, src_pixel_type)?,
        )?;

        {
            let src = self.filter.src_image.read().unwrap();
            src.get(
                &mut src_buffer,
                crate::dng_image::EdgeOption::EdgeRepeat,
                1,
                1,
            )?;
        }

        let mut dst_buffer = DngPixelBuffer::with_planar_config(
            area,
            0,
            self.filter.dst_planes,
            self.filter.dst_pixel_type,
            crate::dng_tag_values::pcRowInterleavedAlignSIMD,
            DngPixelBuffer::allocate_data_simd(area, self.filter.dst_planes, self.filter.dst_pixel_type)?,
        )?;

        let dst_pixel_range = {
            let dst = self.filter.dst_image.read().unwrap();
            dst.pixel_range()
        };

        if src_pixel_type == crate::dng_tag_types::ttFloat {
            let weights_h = self.weights_h.weights32(0)?.to_vec();

            // Temp downsampled row buffer.
            let temp_len = src_cols as usize + (-offset_h) as usize;
            let mut temp = vec![0.0f32; temp_len.max(64)];

            for dst_row in area.t..area.b {
                let row_coord = self.row_coords.coords(0)[dst_row as usize];

                let row_fract = (row_coord as uint32) & kResampleSubsampleMask;

                let weights_v = self.weights_v.weights32(row_fract)?.to_vec();

                let src_row = (row_coord >> kResampleSubsampleBits) + offset_v;

                for plane in 0..dst_buffer.planes as usize {
                    // Down the rows into temp.  The C++ reads (srcRow + k,
                    // srcArea.l + col) relative to the buffer area.
                    let row_base =
                        (src_row as isize - src_area.t as isize) * src_buffer.row_step as isize;
                    let col_base = src_area.l as isize - src_buffer.area.l as isize;
                    for col in 0..src_cols as usize {
                        let mut total: real32 = 0.0;
                        for k in 0..width_v as usize {
                            let index =
                                (row_base + (k as isize) * src_buffer.row_step as isize
                                    + col_base
                                    + col as isize) as usize;
                            let v = src_buffer.data.get_f32(index);
                            total += weights_v[k] * v;
                        }
                        temp[col] = total;
                    }

                    // Across into the destination.  ttPtr = tPtr +
                    // (offsetH - srcArea.l); reads ttPtr[sPixel + k].
                    let dst_col_base = area.l as isize - dst_buffer.area.l as isize;
                    for j in 0..dst_cols as usize {
                        let coord = self.col_coords.coords(area.l)[j];

                        let s_fract = (coord as uint32) & kResampleSubsampleMask;
                        let s_pixel = coord >> kResampleSubsampleBits;

                        let w = &weights_h[s_fract as usize * step_h as usize..];

                        let mut total: real32 = 0.0;
                        for k in 0..width_h as usize {
                            let idx = (offset_h - src_area.l) as isize + (s_pixel as isize) + (k as isize);
                            let v = temp[idx as usize];
                            total += w[k] * v;
                        }

                        let di = (dst_row as isize - dst_buffer.area.t as isize)
                            * dst_buffer.row_step as isize
                            + dst_col_base
                            + j as isize;
                        dst_buffer
                            .data
                            .set_f32(di as usize, crate::dng_utils::pin_real32_01(total));
                    }

                    let _ = plane;
                }
            }
        } else {
            let weights_h = self.weights_h.weights16(0)?.to_vec();

            let temp_len = src_cols as usize + (-offset_h) as usize;
            let mut temp = vec![0u16; temp_len.max(64)];

            let pixel_range = dst_pixel_range;

            for dst_row in area.t..area.b {
                let row_coord = self.row_coords.coords(0)[dst_row as usize];

                let row_fract = (row_coord as uint32) & kResampleSubsampleMask;

                let weights_v = self.weights_v.weights16(row_fract)?.to_vec();

                let src_row = (row_coord >> kResampleSubsampleBits) + offset_v;

                for plane in 0..dst_buffer.planes as usize {
                    // Down the rows into temp (16-bit fixed point).
                    let row_base =
                        (src_row as isize - src_area.t as isize) * src_buffer.row_step as isize;
                    let col_base = src_area.l as isize - src_buffer.area.l as isize;
                    for col in 0..src_cols as usize {
                        let mut total: int32 = 8192;
                        for k in 0..width_v as usize {
                            let index =
                                (row_base + (k as isize) * src_buffer.row_step as isize
                                    + col_base
                                    + col as isize) as usize;
                            let v = src_buffer.data.get_u16(index) as int32;
                            total += weights_v[k] as int32 * v;
                        }
                        temp[col] = crate::dng_utils::pin_int32(0, total >> 14, pixel_range as int32)
                            as uint16;
                    }

                    // Across into the destination (16-bit fixed point).
                    let dst_col_base = area.l as isize - dst_buffer.area.l as isize;
                    for j in 0..dst_cols as usize {
                        let coord = self.col_coords.coords(area.l)[j];

                        let s_fract = (coord as uint32) & kResampleSubsampleMask;
                        let s_pixel = coord >> kResampleSubsampleBits;

                        let w = &weights_h[s_fract as usize * step_h as usize..];

                        let mut total: int32 = 0;
                        for k in 0..width_h as usize {
                            let idx = (offset_h - src_area.l) as isize + (s_pixel as isize) + (k as isize);
                            let v = temp[idx as usize] as int32;
                            total += w[k] as int32 * v;
                        }

                        let di = (dst_row as isize - dst_buffer.area.t as isize)
                            * dst_buffer.row_step as isize
                            + dst_col_base
                            + j as isize;
                        dst_buffer.data.set_u16(
                            di as usize,
                            crate::dng_utils::pin_int32(0, (total + 8192) >> 14, pixel_range as int32)
                                as uint16,
                        );
                    }

                    let _ = plane;
                }
            }
        }

        // Write back.
        let dst = self.filter.dst_image.read().unwrap();
        dst.put(&dst_buffer)?;

        Ok(())
    }
}

/// `ResampleImage (host, srcImage, dstImage, srcBounds, dstBounds, kernel)`.
pub fn resample_image(
    _host: &mut crate::dng_host::DngHost,
    src_image: crate::dng_image::ImageRef,
    dst_image: crate::dng_image::ImageRef,
    src_bounds: DngRect,
    dst_bounds: DngRect,
) -> DngResult<()> {
    // PORT-NOTE(dng_resample.cpp:828): the C++ passes the kernel explicitly;
    // all SDK call sites use dng_resample_bicubic::Get (), so the serial
    // port fixes the bicubic kernel (matching every SDK path).
    let mut task = DngResampleTask::new(src_image, dst_image, src_bounds, dst_bounds)?;

    crate::dng_area_task::process_on_single_thread(&mut task, dst_bounds, None, None)
}

#[allow(unused)]
fn unused_max(x: real64, y: real64) -> real64 {
    max_real64(x, y)
}

#[allow(unused)]
fn unused_mem() -> DngError {
    memory_full()
}

#[allow(unused)]
fn unused_pixel_data(p: &PixelData) -> usize {
    p.element_len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_simple_image::DngSimpleImage;
    use crate::dng_tag_types::ttShort;
    use std::sync::{Arc, RwLock};

    fn make_ref(img: DngSimpleImage) -> crate::dng_image::ImageRef {
        Arc::new(RwLock::new(img))
    }

    #[test]
    fn bicubic_kernel() {
        let k = DngResampleBicubic;
        assert_eq!(k.extent(), 2.0);
        assert!((k.evaluate(0.0) - 1.0).abs() < 1e-12);
        assert_eq!(k.evaluate(2.0), 0.0);
        assert!((k.evaluate(0.5) - 0.59375).abs() < 1e-12); // ((A+2)*x - (A+3))*x*x + 1 at x=0.5
        assert_eq!(k.evaluate(-1.0), k.evaluate(1.0));
    }

    #[test]
    fn coords_mapping() {
        let mut coords = DngResampleCoords::new();
        // Identity 1:1 mapping.
        coords.initialize(0, 0, 8, 8).unwrap();
        for j in 0..8 {
            let c = coords.coords(0)[j];
            // y = j+0.5 - 0.5 = j -> coord = j * 128
            assert_eq!(c, j as int32 * kResampleSubsampleCount as int32);
            assert_eq!(coords.pixel(j as int32), j as int32);
        }

        // 2x upscale: dst 16 from src 8.
        coords.initialize(0, 0, 8, 16).unwrap();
        let c0 = coords.pixel(0);
        assert!(c0 >= -1 && c0 <= 0);
    }

    #[test]
    fn weights_sum_to_one() {
        let mut w = DngResampleWeights::new();
        w.initialize(1.0, DngResampleBicubic::get()).unwrap();

        assert_eq!(w.radius(), 2);
        assert_eq!(w.width(), 4);
        assert_eq!(w.offset(), -1);
        assert_eq!(w.step(), 8);

        for fract in 0..kResampleSubsampleCount {
            let w32 = w.weights32(fract).unwrap();
            // Sum of the first `width` entries ~ 1.0.
            let sum: f32 = w32[..4].iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "fract={fract} sum={sum}");

            let w16 = w.weights16(fract).unwrap();
            let sum16: i32 = w16[..4].iter().map(|&v| v as i32).sum();
            assert_eq!(sum16, 16384, "fract={fract}");
        }
    }

    #[test]
    fn resample_upscale_preserves_flat_image() {
        let src = DngSimpleImage::new(DngRect::from_corners(0, 0, 8, 8), 1, ttShort).unwrap();
        src.set_constant_u16(1000, src.bounds()).unwrap();

        let dst = DngSimpleImage::new(DngRect::from_corners(0, 0, 16, 16), 1, ttShort).unwrap();

        let src_ref = make_ref(src);
        let dst_ref = make_ref(dst);

        let mut host = crate::dng_host::DngHost::new();
        resample_image(
            &mut host,
            src_ref.clone(),
            dst_ref.clone(),
            src_ref.read().unwrap().bounds(),
            dst_ref.read().unwrap().bounds(),
        )
        .unwrap();

        let d = dst_ref.read().unwrap();
        let mut buffer = DngPixelBuffer::with_planar_config(
            d.bounds(),
            0,
            1,
            ttShort,
            crate::dng_tag_values::pcInterleaved,
            DngPixelBuffer::allocate_data(d.bounds(), 1, ttShort).unwrap(),
        )
        .unwrap();
        d.do_get(&mut buffer).unwrap();

        // Flat image stays flat (within rounding).
        for row in 0..16 {
            for col in 0..16 {
                let v = buffer.const_pixel_u16(row, col, 0).unwrap();
                assert!((v as i32 - 1000).abs() <= 1, "row={row} col={col} v={v}");
            }
        }
    }

    #[test]
    fn weights2d_sum_to_one() {
        let mut w = DngResampleWeights2d::new();
        w.initialize(DngResampleBicubic::get()).unwrap();

        assert_eq!(w.radius(), 2);
        assert_eq!(w.width(), 4);

        for y in 0..kResampleSubsampleCount2D {
            for x in 0..kResampleSubsampleCount2D {
                let w32 = w.weights32_at(y, x).unwrap();
                let sum: f32 = w32[..16].iter().sum();
                assert!((sum - 1.0).abs() < 1e-3, "y={y} x={x} sum={sum}");

                let w16 = w.weights16_at(y, x).unwrap();
                let sum16: i32 = w16[..16].iter().map(|&v| v as i32).sum();
                assert_eq!(sum16, 16384, "y={y} x={x}");
            }
        }
    }
}

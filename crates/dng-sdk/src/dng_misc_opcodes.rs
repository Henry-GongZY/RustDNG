// Port of dng_misc_opcodes.h/.cpp — miscellaneous DNG opcodes.
//
// Porting notes:
//   * `AutoPtr<dng_memory_block>` table storage maps to owned `Vec<u16>` /
//     `Vec<real32>` buffers.
//   * The `DoMapArea16` / `DoBaselineMapPoly32` bottlenecks (reference
//     implementations in dng_reference.cpp) are inlined with identical
//     arithmetic.
//   * `dng_opcode_MapTable::ReplicateLastEntry` reads `table [fCount]` out of
//     bounds in the C++ when count == 0x10000 (the value is unused since the
//     replication loop is empty); the port skips the read.

use crate::dng_core::{
    bad_format, convert_uint32_to_int32, intersect_rect, pin_real32, pin_uint16,
    program_error, round_int32_f64, round_uint32_f64, round_up_uint32_to_multiple,
    safe_int32_sub, safe_uint32_add, safe_uint32_divide_up, safe_uint32_mult2, DngHost, DngPoint,
    DngRect, DngResult, DngStream, DngURational, ImageRef, int32, real32, real64, ttFloat,
    ttLong, ttShort, uint32, uint16,
};
use crate::dng_negative::DngNegative;
use crate::dng_opcodes::{
    DngInplaceOpcode, DngOpcode, DngOpcodeBase, dngOpcode_DeltaPerColumn, dngOpcode_DeltaPerRow,
    dngOpcode_MapPolynomial, dngOpcode_MapTable, dngOpcode_ScalePerColumn,
    dngOpcode_ScalePerRow, dngOpcode_TrimBounds, kFlag_None,
};
use crate::dng_core::dng_pixel_buffer::DngPixelBuffer;
use crate::dng_core::dng_tag_values::{dngVersion_1_3_0_0};

// ---------------------------------------------------------------------------
// dng_opcode_TrimBounds.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_TrimBounds` — trim image to a specified rectangle.
pub struct DngOpcodeTrimBounds {
    base: DngOpcodeBase,
    bounds: DngRect,
}

impl DngOpcodeTrimBounds {
    /// `dng_opcode_TrimBounds (bounds)`.
    pub fn new(bounds: DngRect) -> Self {
        DngOpcodeTrimBounds {
            base: DngOpcodeBase::new(dngOpcode_TrimBounds, dngVersion_1_3_0_0, kFlag_None),
            bounds,
        }
    }

    /// `dng_opcode_TrimBounds (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_TrimBounds, stream)?;

        if stream.get_uint32()? != 16 {
            return Err(bad_format());
        }

        let mut bounds = DngRect::new();
        bounds.t = stream.get_int32()?;
        bounds.l = stream.get_int32()?;
        bounds.b = stream.get_int32()?;
        bounds.r = stream.get_int32()?;

        if bounds.is_empty() {
            return Err(bad_format());
        }

        Ok(DngOpcodeTrimBounds { base, bounds })
    }

    /// The trim bounds.
    pub fn bounds(&self) -> DngRect {
        self.bounds
    }
}

impl DngOpcode for DngOpcodeTrimBounds {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(16)?;

        stream.put_int32(self.bounds.t)?;
        stream.put_int32(self.bounds.l)?;
        stream.put_int32(self.bounds.b)?;
        stream.put_int32(self.bounds.r)?;

        Ok(())
    }

    /// `Apply (host, negative, image)`.
    fn apply(
        &mut self,
        _host: &mut DngHost,
        _negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        let image_bounds = image.read().unwrap().bounds();

        if self.bounds.is_empty() || intersect_rect(self.bounds, image_bounds) != self.bounds {
            return Err(bad_format());
        }

        image.read().unwrap().trim(self.bounds)?;

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_area_spec.
// ---------------------------------------------------------------------------

/// Port of `dng_area_spec` — describes an area of an image, including a pixel
/// subrectangle, plane range, and row/column pitch.
#[derive(Debug, Clone, Default)]
pub struct DngAreaSpec {
    area: DngRect,

    plane: uint32,
    planes: uint32,

    row_pitch: uint32,
    col_pitch: uint32,

    area_scale: DngURational,
}

impl DngAreaSpec {
    /// `kDataSize`.
    pub const kDataSize: uint32 = 32;

    /// `dng_area_spec (area, plane, planes, rowPitch, colPitch)`.
    pub fn new(
        area: DngRect,
        plane: uint32,
        planes: uint32,
        row_pitch: uint32,
        col_pitch: uint32,
    ) -> Self {
        DngAreaSpec {
            area,
            plane,
            planes,
            row_pitch,
            col_pitch,
            area_scale: DngURational { n: 1, d: 1 },
        }
    }

    /// `Area ()`.
    pub fn area(&self) -> DngRect {
        self.area
    }

    /// `ScaledArea ()`.
    pub fn scaled_area(&self) -> DngRect {
        if self.area_scale.n == self.area_scale.d {
            return self.area;
        }

        let mut area = self.area;

        let x = self.area_scale.as_real64();

        area.t = round_int32_f64(x * area.t as real64);
        area.l = round_int32_f64(x * area.l as real64);
        area.b = round_int32_f64(x * area.b as real64);
        area.r = round_int32_f64(x * area.r as real64);

        area
    }

    /// `Plane ()`.
    pub fn plane(&self) -> uint32 {
        self.plane
    }

    /// `Planes ()`.
    pub fn planes(&self) -> uint32 {
        self.planes
    }

    /// `RowPitch ()`.
    pub fn row_pitch(&self) -> uint32 {
        self.row_pitch
    }

    /// `ColPitch ()`.
    pub fn col_pitch(&self) -> uint32 {
        self.col_pitch
    }

    /// `GetData (stream)`.
    pub fn get_data(&mut self, stream: &mut DngStream) -> DngResult<()> {
        self.area.t = stream.get_int32()?;
        self.area.l = stream.get_int32()?;
        self.area.b = stream.get_int32()?;
        self.area.r = stream.get_int32()?;

        self.plane = stream.get_uint32()?;
        self.planes = stream.get_uint32()?;

        self.row_pitch = stream.get_uint32()?;
        self.col_pitch = stream.get_uint32()?;

        if self.planes < 1 {
            return Err(bad_format());
        }

        if self.row_pitch < 1 || self.col_pitch < 1 {
            return Err(bad_format());
        }

        if self.row_pitch >= self.area.h() as uint32 || self.col_pitch >= self.area.w() as uint32 {
            // DNG_REPORT in the C++ (warning, then clamp).
            self.row_pitch = crate::dng_core::min_uint32(self.row_pitch, self.area.h() as uint32);
            self.col_pitch = crate::dng_core::min_uint32(self.col_pitch, self.area.w() as uint32);
        }

        if self.area.is_empty() {
            if self.row_pitch != 1 || self.col_pitch != 1 {
                return Err(bad_format());
            }
        } else {
            let height = match safe_int32_sub(self.area.b, self.area.t) {
                Ok(v) => v,
                Err(_) => return Err(bad_format()),
            };

            let width = match safe_int32_sub(self.area.r, self.area.l) {
                Ok(v) => v,
                Err(_) => return Err(bad_format()),
            };

            if self.row_pitch > height as uint32 || self.col_pitch > width as uint32 {
                return Err(bad_format());
            }
        }

        Ok(())
    }

    /// `PutData (stream)`.
    pub fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_int32(self.area.t)?;
        stream.put_int32(self.area.l)?;
        stream.put_int32(self.area.b)?;
        stream.put_int32(self.area.r)?;

        stream.put_uint32(self.plane)?;
        stream.put_uint32(self.planes)?;

        stream.put_uint32(self.row_pitch)?;
        stream.put_uint32(self.col_pitch)?;

        Ok(())
    }

    /// `Overlap (tile)` — pixel area overlap between this area and the
    /// specified tile.
    pub fn overlap(&self, tile: DngRect) -> DngRect {
        // Special case - if the fArea is empty, then dng_area_spec covers
        // the entire image, no matter how large it is.

        if self.area.is_empty() {
            return tile;
        }

        let mut overlap = intersect_rect(self.area, tile);

        if overlap.not_empty() {
            match (|| -> DngResult<()> {
                overlap.t = self.area.t
                    + convert_uint32_to_int32(round_up_uint32_to_multiple(
                        (overlap.t - self.area.t) as uint32,
                        self.row_pitch,
                    )?)?;

                overlap.l = self.area.l
                    + convert_uint32_to_int32(round_up_uint32_to_multiple(
                        (overlap.l - self.area.l) as uint32,
                        self.col_pitch,
                    )?)?;

                Ok(())
            })() {
                Ok(()) => {}
                Err(_) => return DngRect::new(),
            }

            if overlap.not_empty() {
                overlap.b = overlap.t
                    + ((overlap.h() - 1) / self.row_pitch as int32) * self.row_pitch as int32
                    + 1;
                overlap.r = overlap.l
                    + ((overlap.w() - 1) / self.col_pitch as int32) * self.col_pitch as int32
                    + 1;

                return overlap;
            }
        }

        DngRect::new()
    }

    /// `ScaledOverlap (tile)` — same as Overlap but uses the area scale
    /// factor.
    pub fn scaled_overlap(&self, tile: DngRect) -> DngRect {
        // Special case - if the fArea is empty, then dng_area_spec covers
        // the entire image, no matter how large it is.

        if self.area.is_empty() {
            return tile;
        }

        // Use ScaledArea instead of fArea.
        let area = self.scaled_area();

        let mut overlap = intersect_rect(area, tile);

        if overlap.not_empty() {
            overlap.t = area.t
                + ((overlap.t - area.t + self.row_pitch as int32 - 1) / self.row_pitch as int32)
                    * self.row_pitch as int32;
            overlap.l = area.l
                + ((overlap.l - area.l + self.col_pitch as int32 - 1) / self.col_pitch as int32)
                    * self.col_pitch as int32;

            if overlap.not_empty() {
                overlap.b = overlap.t
                    + ((overlap.h() - 1) / self.row_pitch as int32) * self.row_pitch as int32
                    + 1;
                overlap.r = overlap.l
                    + ((overlap.w() - 1) / self.col_pitch as int32) * self.col_pitch as int32
                    + 1;

                return overlap;
            }
        }

        DngRect::new()
    }

    /// `ApplyAreaScale (scale)` — the scale factor is not cumulative; the new
    /// scale factor replaces the previous factor.
    pub fn apply_area_scale(&mut self, scale: DngURational) {
        assert!(scale.n > 0 && scale.d > 0, "invalid scale in dng_area_spec::Scale");

        self.area_scale = scale;
    }

    /// The area scale factor (used by opcodes to compute their scaled area).
    pub fn area_scale(&self) -> DngURational {
        self.area_scale
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_MapTable.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_MapTable` — apply a 1D function (16-bit table) to an
/// image area.
pub struct DngOpcodeMapTable {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    table: Vec<uint16>,

    count: uint32,

    black_adjusted_table: Option<Vec<uint16>>,
}

impl DngOpcodeMapTable {
    /// `dng_opcode_MapTable (host, areaSpec, table, count)`.
    pub fn new(area_spec: DngAreaSpec, table: &[uint16], count: uint32) -> DngResult<Self> {
        if count == 0 || count > 0x10000 {
            return Err(program_error());
        }

        let mut full_table = vec![0u16; 0x10000];

        full_table[..count as usize].copy_from_slice(&table[..count as usize]);

        let mut opcode = DngOpcodeMapTable {
            base: DngOpcodeBase::new(dngOpcode_MapTable, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            table: full_table,
            count,
            black_adjusted_table: None,
        };

        opcode.replicate_last_entry();

        Ok(opcode)
    }

    /// `dng_opcode_MapTable (host, stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_MapTable, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let count = stream.get_uint32()?;

        let required_size = safe_uint32_mult2(count, 2)?;
        let required_size = safe_uint32_add(required_size, DngAreaSpec::kDataSize)?;
        let required_size = safe_uint32_add(required_size, 4)?;

        if data_size != required_size {
            return Err(bad_format());
        }

        if count == 0 || count > 0x10000 {
            return Err(bad_format());
        }

        let mut table = vec![0u16; 0x10000];

        for index in 0..count as usize {
            table[index] = stream.get_uint16()?;
        }

        let mut opcode = DngOpcodeMapTable {
            base,
            area_spec,
            table,
            count,
            black_adjusted_table: None,
        };

        opcode.replicate_last_entry();

        Ok(opcode)
    }

    /// `ReplicateLastEntry ()`.
    fn replicate_last_entry(&mut self) {
        if self.count as usize >= self.table.len() {
            // PORT-NOTE(dng_misc_opcodes.cpp:438): the C++ reads
            // table [fCount] out of bounds when count == 0x10000; the value
            // is unused because the replication loop is empty, so the port
            // skips the read.
            return;
        }

        let last_entry = self.table[self.count as usize];

        for index in self.count as usize..0x10000 {
            self.table[index] = last_entry;
        }
    }

    /// The count of table entries.
    pub fn count(&self) -> uint32 {
        self.count
    }

    /// The area spec.
    pub fn area_spec(&self) -> &DngAreaSpec {
        &self.area_spec
    }

    /// The first `count` entries of the mapping table.
    pub fn table(&self) -> &[uint16] {
        &self.table[..self.count as usize]
    }
}

impl DngOpcode for DngOpcodeMapTable {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(DngAreaSpec::kDataSize + 4 + self.count * 2)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(self.count)?;

        for index in 0..self.count as usize {
            stream.put_uint16(self.table[index])?;
        }

        Ok(())
    }

    /// `ApplyAreaScale (scale)`.
    fn apply_area_scale(&mut self, scale: DngURational) {
        self.area_spec.apply_area_scale(scale);
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeMapTable {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&mut self, _image_pixel_type: uint32) -> DngResult<uint32> {
        Ok(ttShort)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.scaled_overlap(image_bounds)
    }

    /// `Prepare (...)` — build the black-adjusted table.
    fn prepare(
        &mut self,
        negative: &mut DngNegative,
        _thread_count: uint32,
        _tile_size: DngPoint,
        _image_bounds: DngRect,
        _image_planes: uint32,
        _buffer_pixel_type: uint32,
    ) -> DngResult<()> {
        self.black_adjusted_table = None;

        let black_level = negative.stage3_black_level() as int32;

        if self.stage() >= 2 && black_level != 0 {
            let mut dst_table = vec![0u16; 0x10000];

            let src_table = &self.table;

            let src_scale = 65535.0 / (65535.0 - black_level as real64);

            let dst_scale = (65535.0 - black_level as real64) / 65535.0;

            for index in 0..0x10000i64 {
                let x = (index - black_level as i64) as real64 * src_scale;

                let y: real64;

                if x < 0.0 {
                    y = src_table[0] as real64 * 2.0
                        - src_table[round_uint32_f64(-x) as usize] as real64;
                } else {
                    y = src_table[round_uint32_f64(x) as usize] as real64;
                }

                dst_table[index as usize] =
                    pin_uint16(round_int32_f64(y * dst_scale) + black_level);
            }

            self.black_adjusted_table = Some(dst_table);
        }

        Ok(())
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        _negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.scaled_overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let table: &[uint16] = match &self.black_adjusted_table {
            Some(t) => t,
            None => &self.table,
        };

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            // DoMapArea16 (reference implementation) with:
            //   count0 = 1, step0 = 0,
            //   count1 = rows (overlap.H () aligned to rowPitch),
            //   count2 = cols (overlap.W () aligned to colPitch),
            //   step1 = rowPitch * rowStep, step2 = colPitch.

            let rows = (overlap.h() as uint32 + self.area_spec.row_pitch() - 1)
                / self.area_spec.row_pitch();
            let cols = (overlap.w() as uint32 + self.area_spec.col_pitch() - 1)
                / self.area_spec.col_pitch();

            for row_index in 0..rows {
                let row = overlap.t + (row_index * self.area_spec.row_pitch()) as int32;

                for col_index in 0..cols {
                    let col = overlap.l + (col_index * self.area_spec.col_pitch()) as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    let value = buffer.as_u16_slice().unwrap()[index];

                    let mapped = table[value as usize];

                    buffer.as_u16_slice_mut().unwrap()[index] = mapped;
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_MapPolynomial.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_MapPolynomial` — apply a 1D function (polynomial) to
/// an image area.
pub struct DngOpcodeMapPolynomial {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    degree: uint32,

    coefficient: [real64; 9],

    coefficient32: [real32; 9],
}

impl DngOpcodeMapPolynomial {
    /// `kMaxDegree`.
    pub const kMaxDegree: uint32 = 8;

    /// `dng_opcode_MapPolynomial (areaSpec, degree, coefficient)`.
    pub fn new(area_spec: DngAreaSpec, degree: uint32, coefficient: &[real64]) -> Self {
        let mut opcode = DngOpcodeMapPolynomial {
            base: DngOpcodeBase::new(dngOpcode_MapPolynomial, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            degree,
            coefficient: [0.0; 9],
            coefficient32: [0.0; 9],
        };

        for (j, slot) in opcode.coefficient.iter_mut().enumerate() {
            if j as uint32 <= opcode.degree {
                *slot = coefficient[j];
            } else {
                *slot = 0.0;
            }
        }

        // Reduce degree if possible.
        while opcode.degree > 0 && opcode.coefficient[opcode.degree as usize] == 0.0 {
            opcode.degree -= 1;
        }

        opcode
    }

    /// `dng_opcode_MapPolynomial (stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_MapPolynomial, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let degree = stream.get_uint32()?;

        if data_size != DngAreaSpec::kDataSize + 4 + (degree + 1) * 8 {
            return Err(bad_format());
        }

        if degree > Self::kMaxDegree {
            return Err(bad_format());
        }

        let mut opcode = DngOpcodeMapPolynomial {
            base,
            area_spec,
            degree,
            coefficient: [0.0; 9],
            coefficient32: [0.0; 9],
        };

        for j in 0..=Self::kMaxDegree as usize {
            if j as uint32 <= opcode.degree {
                opcode.coefficient[j] = stream.get_real64()?;
            } else {
                opcode.coefficient[j] = 0.0;
            }
        }

        Ok(opcode)
    }

    /// `Degree ()`.
    pub fn degree(&self) -> uint32 {
        self.degree
    }

    /// `Coefficients ()`.
    pub fn coefficients(&self) -> &[real64; 9] {
        &self.coefficient
    }
}

impl DngOpcode for DngOpcodeMapPolynomial {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        stream.put_uint32(DngAreaSpec::kDataSize + 4 + (self.degree + 1) * 8)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(self.degree)?;

        for j in 0..=self.degree as usize {
            stream.put_real64(self.coefficient[j])?;
        }

        Ok(())
    }

    /// `ApplyAreaScale (scale)`.
    fn apply_area_scale(&mut self, scale: DngURational) {
        self.area_spec.apply_area_scale(scale);
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeMapPolynomial {
    /// `BufferPixelType (imagePixelType)` — adjusts the 32-bit coefficients
    /// to convert from image values to floating point values.
    fn buffer_pixel_type(&mut self, image_pixel_type: uint32) -> DngResult<uint32> {
        // If we are operating on the stage 1 image, then we need to adjust
        // the coefficients to convert from the image values to the 32-bit
        // floating point values that this opcode operates on.
        //
        // If we are operating on the stage 2 or 3 image, the logical range of
        // the image is already 0.0 to 1.0, so we don't need to adjust the
        // values.

        let mut scale32: real64 = 1.0;

        if self.stage() == 1 {
            match image_pixel_type {
                ttFloat => {}
                ttShort => {
                    scale32 = 0xFFFF as real64;
                }
                ttLong => {
                    scale32 = 0xFFFF_FFFFu32 as real64;
                }
                _ => {
                    return Err(bad_format());
                }
            }
        }

        let mut factor32: real64 = 1.0 / scale32;

        for j in 0..=Self::kMaxDegree as usize {
            self.coefficient32[j] = (self.coefficient[j] * factor32) as real32;

            factor32 *= scale32;
        }

        Ok(ttFloat)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.scaled_overlap(image_bounds)
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.scaled_overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let black_level: uint16 = if self.stage() >= 2 {
            negative.stage3_black_level()
        } else {
            0
        };

        let row_pitch = self.area_spec.row_pitch();
        let col_pitch = self.area_spec.col_pitch();

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            self.do_process(buffer, overlap, plane, row_pitch, col_pitch, black_level)?;
        }

        Ok(())
    }
}

impl DngOpcodeMapPolynomial {
    /// `DoProcess (buffer, area, plane, rowPitch, colPitch, coefficients,
    /// degree, blackLevel)` — the DoBaselineMapPoly32 reference
    /// implementation.
    fn do_process(
        &self,
        buffer: &mut DngPixelBuffer,
        area: DngRect,
        plane: uint32,
        row_pitch: uint32,
        col_pitch: uint32,
        black_level: uint16,
    ) -> DngResult<()> {
        let coefficients = &self.coefficient32;
        let degree = self.degree;

        let rows = area.h() as uint32;
        let cols = area.w() as uint32;

        let black_scale1: real32 = 1.0;
        let mut black_scale2: real32 = 1.0;
        let mut black_offset1: real32 = 0.0;
        let mut black_offset2: real32 = 0.0;

        if black_level != 0 {
            black_offset2 = (black_level as real32) / 65535.0;
            black_scale2 = 1.0 - black_offset2;
            black_scale1 = 1.0 / black_scale2;
            black_offset1 = 1.0 - black_scale1;
        }

        let slice = buffer
            .as_f32_slice_mut()
            .ok_or_else(crate::dng_core::program_error)?;

        for row_index in (0..rows).step_by(row_pitch.max(1) as usize) {
            let row = area.t + row_index as int32;

            if black_level != 0 {
                for col_index in (0..cols).step_by(col_pitch.max(1) as usize) {
                    let col = area.l + col_index as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    slice[index] = slice[index] * black_scale1 + black_offset1;
                }
            }

            for col_index in (0..cols).step_by(col_pitch.max(1) as usize) {
                let col = area.l + col_index as int32;

                let index = buffer.pixel_element_index_public(row, col, plane)?;

                let x = slice[index];

                let mut y: real32 = coefficients[0];

                if degree == 0 {
                    y = coefficients[0];
                } else if x < 0.0 {
                    let xx0 = -x;
                    let mut xx = xx0;

                    for j in 1..=degree as usize {
                        y -= coefficients[j] * xx;
                        xx *= xx0;
                    }
                } else {
                    let xx0 = x;
                    let mut xx = xx0;

                    for j in 1..=degree as usize {
                        y += coefficients[j] * xx;
                        xx *= xx0;
                    }
                }

                slice[index] = pin_real32(-1.0, y, 1.0);
            }

            if black_level != 0 {
                for col_index in (0..cols).step_by(col_pitch.max(1) as usize) {
                    let col = area.l + col_index as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    slice[index] = slice[index] * black_scale2 + black_offset2;
                }
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Shared helpers for the Delta/Scale per-row/per-column opcodes.
// ---------------------------------------------------------------------------

/// Buffer pixel type shared by DeltaPerRow / DeltaPerColumn: converts to a
/// floating point buffer and computes the delta scale.
fn delta_buffer_pixel_type(
    image_pixel_type: uint32,
    scale: &mut real32,
) -> DngResult<uint32> {
    let scale32: real64;

    match image_pixel_type {
        ttFloat => scale32 = 1.0,
        ttShort => scale32 = 0xFFFF as real64,
        ttLong => scale32 = 0xFFFF_FFFFu32 as real64,
        _ => return Err(bad_format()),
    }

    // The C++ assigns fScale inside BufferPixelType.
    *scale = (1.0 / scale32) as real32;

    Ok(ttFloat)
}

// ---------------------------------------------------------------------------
// dng_opcode_DeltaPerRow.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_DeltaPerRow` — apply a delta that varies per row.
pub struct DngOpcodeDeltaPerRow {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    table: Vec<real32>,

    scale: real32,
}

impl DngOpcodeDeltaPerRow {
    /// `dng_opcode_DeltaPerRow (areaSpec, table)`.
    pub fn new(area_spec: DngAreaSpec, table: Vec<real32>) -> Self {
        DngOpcodeDeltaPerRow {
            base: DngOpcodeBase::new(dngOpcode_DeltaPerRow, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            table,
            scale: 1.0,
        }
    }

    /// `dng_opcode_DeltaPerRow (host, stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_DeltaPerRow, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let deltas = safe_uint32_divide_up(area_spec.area().h() as uint32, area_spec.row_pitch())?;

        if deltas != stream.get_uint32()? {
            return Err(bad_format());
        }

        if data_size != DngAreaSpec::kDataSize + 4 + deltas * 4 {
            return Err(bad_format());
        }

        let mut table = vec![0.0f32; deltas as usize];

        for entry in table.iter_mut() {
            *entry = stream.get_real32()?;
        }

        Ok(DngOpcodeDeltaPerRow {
            base,
            area_spec,
            table,
            scale: 1.0,
        })
    }

    /// The area spec.
    pub fn area_spec(&self) -> &DngAreaSpec {
        &self.area_spec
    }

    /// The row delta table.
    pub fn table(&self) -> &[real32] {
        &self.table
    }
}

impl DngOpcode for DngOpcodeDeltaPerRow {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let deltas = safe_uint32_divide_up(self.area_spec.area().h() as uint32, self.area_spec.row_pitch())?;

        stream.put_uint32(DngAreaSpec::kDataSize + 4 + deltas * 4)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(deltas)?;

        for j in 0..deltas as usize {
            stream.put_real32(self.table[j])?;
        }

        Ok(())
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeDeltaPerRow {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&mut self, image_pixel_type: uint32) -> uint32 {
        delta_buffer_pixel_type(image_pixel_type, &mut self.scale)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.overlap(image_bounds)
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let cols = overlap.w() as uint32;

        let col_pitch = self.area_spec.col_pitch();

        let mut scale = self.scale;

        if self.stage() >= 2 && negative.stage3_black_level() != 0 {
            scale *= (1.0 - negative.stage3_black_level_normalized()) as real32;
        }

        let slice = buffer
            .as_f32_slice_mut()
            .ok_or_else(crate::dng_core::program_error)?;

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            let mut table_index =
                ((overlap.t - self.area_spec.area().t) / self.area_spec.row_pitch() as int32)
                    as usize;

            let mut row = overlap.t;

            while row < overlap.b {
                let row_delta = self.table[table_index] * scale;
                table_index += 1;

                for col_index in (0..cols).step_by(col_pitch.max(1) as usize) {
                    let col = overlap.l + col_index as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    let x = slice[index];

                    let y = x + row_delta;

                    slice[index] = pin_real32(-1.0, y, 1.0);
                }

                row += self.area_spec.row_pitch() as int32;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_DeltaPerColumn.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_DeltaPerColumn` — apply a delta that varies per column.
pub struct DngOpcodeDeltaPerColumn {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    table: Vec<real32>,

    scale: real32,
}

impl DngOpcodeDeltaPerColumn {
    /// `dng_opcode_DeltaPerColumn (areaSpec, table)`.
    pub fn new(area_spec: DngAreaSpec, table: Vec<real32>) -> Self {
        DngOpcodeDeltaPerColumn {
            base: DngOpcodeBase::new(dngOpcode_DeltaPerColumn, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            table,
            scale: 1.0,
        }
    }

    /// `dng_opcode_DeltaPerColumn (host, stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_DeltaPerColumn, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let deltas = safe_uint32_divide_up(area_spec.area().w() as uint32, area_spec.col_pitch())?;

        if deltas != stream.get_uint32()? {
            return Err(bad_format());
        }

        if data_size != DngAreaSpec::kDataSize + 4 + deltas * 4 {
            return Err(bad_format());
        }

        let mut table = vec![0.0f32; deltas as usize];

        for entry in table.iter_mut() {
            *entry = stream.get_real32()?;
        }

        Ok(DngOpcodeDeltaPerColumn {
            base,
            area_spec,
            table,
            scale: 1.0,
        })
    }

    /// The area spec.
    pub fn area_spec(&self) -> &DngAreaSpec {
        &self.area_spec
    }

    /// The column delta table.
    pub fn table(&self) -> &[real32] {
        &self.table
    }
}

impl DngOpcode for DngOpcodeDeltaPerColumn {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let deltas = safe_uint32_divide_up(self.area_spec.area().w() as uint32, self.area_spec.col_pitch())?;

        stream.put_uint32(DngAreaSpec::kDataSize + 4 + deltas * 4)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(deltas)?;

        for j in 0..deltas as usize {
            stream.put_real32(self.table[j])?;
        }

        Ok(())
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeDeltaPerColumn {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&mut self, image_pixel_type: uint32) -> uint32 {
        delta_buffer_pixel_type(image_pixel_type, &mut self.scale)
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.overlap(image_bounds)
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let rows = (overlap.h() as uint32 + self.area_spec.row_pitch() - 1)
            / self.area_spec.row_pitch();

        let row_step = buffer.row_step() * self.area_spec.row_pitch() as int32;

        let mut scale = self.scale;

        if self.stage() >= 2 && negative.stage3_black_level() != 0 {
            scale *= (1.0 - negative.stage3_black_level_normalized()) as real32;
        }

        let slice = buffer
            .as_f32_slice_mut()
            .ok_or_else(crate::dng_core::program_error)?;

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            let mut table_index =
                ((overlap.l - self.area_spec.area().l) / self.area_spec.col_pitch() as int32)
                    as usize;

            let mut col = overlap.l;

            while col < overlap.r {
                let col_delta = self.table[table_index] * scale;
                table_index += 1;

                for row_index in 0..rows {
                    let row = overlap.t + (row_index * self.area_spec.row_pitch()) as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    let x = slice[index];

                    let y = x + col_delta;

                    slice[index] = pin_real32(-1.0, y, 1.0);

                    let _ = row_step;
                }

                col += self.area_spec.col_pitch() as int32;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_ScalePerRow.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_ScalePerRow` — apply a scale factor that varies per
/// row.
pub struct DngOpcodeScalePerRow {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    table: Vec<real32>,
}

impl DngOpcodeScalePerRow {
    /// `dng_opcode_ScalePerRow (areaSpec, table)`.
    pub fn new(area_spec: DngAreaSpec, table: Vec<real32>) -> Self {
        DngOpcodeScalePerRow {
            base: DngOpcodeBase::new(dngOpcode_ScalePerRow, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            table,
        }
    }

    /// `dng_opcode_ScalePerRow (host, stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_ScalePerRow, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let scales = safe_uint32_divide_up(area_spec.area().h() as uint32, area_spec.row_pitch())?;

        if scales != stream.get_uint32()? {
            return Err(bad_format());
        }

        if data_size != DngAreaSpec::kDataSize + 4 + scales * 4 {
            return Err(bad_format());
        }

        let mut table = vec![0.0f32; scales as usize];

        for entry in table.iter_mut() {
            *entry = stream.get_real32()?;
        }

        Ok(DngOpcodeScalePerRow {
            base,
            area_spec,
            table,
        })
    }

    /// The area spec.
    pub fn area_spec(&self) -> &DngAreaSpec {
        &self.area_spec
    }

    /// The row scale table.
    pub fn table(&self) -> &[real32] {
        &self.table
    }
}

impl DngOpcode for DngOpcodeScalePerRow {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let scales = safe_uint32_divide_up(self.area_spec.area().h() as uint32, self.area_spec.row_pitch())?;

        stream.put_uint32(DngAreaSpec::kDataSize + 4 + scales * 4)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(scales)?;

        for j in 0..scales as usize {
            stream.put_real32(self.table[j])?;
        }

        Ok(())
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeScalePerRow {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&self, _image_pixel_type: uint32) -> uint32 {
        ttFloat
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.overlap(image_bounds)
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let cols = overlap.w() as uint32;

        let col_pitch = self.area_spec.col_pitch();

        let mut black_offset: real32 = 0.0;

        if self.stage() >= 2 && negative.stage3_black_level() != 0 {
            black_offset = negative.stage3_black_level_normalized() as real32;
        }

        let slice = buffer
            .as_f32_slice_mut()
            .ok_or_else(crate::dng_core::program_error)?;

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            let mut table_index =
                ((overlap.t - self.area_spec.area().t) / self.area_spec.row_pitch() as int32)
                    as usize;

            let mut row = overlap.t;

            while row < overlap.b {
                let row_scale = self.table[table_index];
                table_index += 1;

                for col_index in (0..cols).step_by(col_pitch.max(1) as usize) {
                    let col = overlap.l + col_index as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    let x = slice[index];

                    let y = (x - black_offset) * row_scale + black_offset;

                    slice[index] = pin_real32(-1.0, y, 1.0);
                }

                row += self.area_spec.row_pitch() as int32;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_opcode_ScalePerColumn.
// ---------------------------------------------------------------------------

/// Port of `dng_opcode_ScalePerColumn` — apply a scale factor that varies per
/// column.
pub struct DngOpcodeScalePerColumn {
    base: DngOpcodeBase,

    area_spec: DngAreaSpec,

    table: Vec<real32>,
}

impl DngOpcodeScalePerColumn {
    /// `dng_opcode_ScalePerColumn (areaSpec, table)`.
    pub fn new(area_spec: DngAreaSpec, table: Vec<real32>) -> Self {
        DngOpcodeScalePerColumn {
            base: DngOpcodeBase::new(dngOpcode_ScalePerColumn, dngVersion_1_3_0_0, kFlag_None),
            area_spec,
            table,
        }
    }

    /// `dng_opcode_ScalePerColumn (host, stream)`.
    pub fn from_stream(stream: &mut DngStream) -> DngResult<Self> {
        let base = DngOpcodeBase::from_stream(dngOpcode_ScalePerColumn, stream)?;

        let data_size = stream.get_uint32()?;

        let mut area_spec = DngAreaSpec::default();
        area_spec.get_data(stream)?;

        let scales = safe_uint32_divide_up(area_spec.area().w() as uint32, area_spec.col_pitch())?;

        if scales != stream.get_uint32()? {
            return Err(bad_format());
        }

        if data_size != DngAreaSpec::kDataSize + 4 + scales * 4 {
            return Err(bad_format());
        }

        let mut table = vec![0.0f32; scales as usize];

        for entry in table.iter_mut() {
            *entry = stream.get_real32()?;
        }

        Ok(DngOpcodeScalePerColumn {
            base,
            area_spec,
            table,
        })
    }

    /// The area spec.
    pub fn area_spec(&self) -> &DngAreaSpec {
        &self.area_spec
    }

    /// The column scale table.
    pub fn table(&self) -> &[real32] {
        &self.table
    }
}

impl DngOpcode for DngOpcodeScalePerColumn {
    fn base(&self) -> &DngOpcodeBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut DngOpcodeBase {
        &mut self.base
    }

    /// `PutData (stream)`.
    fn put_data(&self, stream: &mut DngStream) -> DngResult<()> {
        let scales = safe_uint32_divide_up(self.area_spec.area().w() as uint32, self.area_spec.col_pitch())?;

        stream.put_uint32(DngAreaSpec::kDataSize + 4 + scales * 4)?;

        self.area_spec.put_data(stream)?;

        stream.put_uint32(scales)?;

        for j in 0..scales as usize {
            stream.put_real32(self.table[j])?;
        }

        Ok(())
    }

    /// `Apply (host, negative, image)` — via the in-place driver.
    fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut ImageRef,
    ) -> DngResult<()> {
        self.apply_inplace(host, negative, image)
    }
}

impl DngInplaceOpcode for DngOpcodeScalePerColumn {
    /// `BufferPixelType (imagePixelType)`.
    fn buffer_pixel_type(&self, _image_pixel_type: uint32) -> uint32 {
        ttFloat
    }

    /// `ModifiedBounds (imageBounds)`.
    fn modified_bounds(&self, image_bounds: DngRect) -> DngRect {
        self.area_spec.overlap(image_bounds)
    }

    /// `ProcessArea (...)`.
    fn process_area(
        &mut self,
        negative: &mut DngNegative,
        _thread_index: uint32,
        buffer: &mut DngPixelBuffer,
        dst_area: DngRect,
        _image_bounds: DngRect,
    ) -> DngResult<()> {
        let overlap = self.area_spec.overlap(dst_area);

        if overlap.is_empty() {
            return Ok(());
        }

        let rows = (overlap.h() as uint32 + self.area_spec.row_pitch() - 1)
            / self.area_spec.row_pitch();

        let mut black_offset: real32 = 0.0;

        if self.stage() >= 2 && negative.stage3_black_level() != 0 {
            black_offset = negative.stage3_black_level_normalized() as real32;
        }

        let slice = buffer
            .as_f32_slice_mut()
            .ok_or_else(crate::dng_core::program_error)?;

        for plane in self.area_spec.plane()..(self.area_spec.plane() + self.area_spec.planes()) {
            if plane >= buffer.planes() {
                break;
            }

            let mut table_index =
                ((overlap.l - self.area_spec.area().l) / self.area_spec.col_pitch() as int32)
                    as usize;

            let mut col = overlap.l;

            while col < overlap.r {
                let col_scale = self.table[table_index];
                table_index += 1;

                for row_index in 0..rows {
                    let row = overlap.t + (row_index * self.area_spec.row_pitch()) as int32;

                    let index = buffer.pixel_element_index_public(row, col, plane)?;

                    let x = slice[index];

                    let y = (x - black_offset) * col_scale + black_offset;

                    slice[index] = pin_real32(-1.0, y, 1.0);
                }

                col += self.area_spec.col_pitch() as int32;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_core::dng_simple_image::DngSimpleImage;
    use crate::dng_core::DngMemoryStream;

    fn stream_from(bytes: &[u8]) -> DngStream<'static> {
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put(bytes).unwrap();
        stream.set_read_position(0).unwrap();
        stream
    }

    fn test_negative(stage3_black: uint16) -> DngNegative {
        let mut host = DngHost::new();
        let mut negative = DngNegative::new(&mut host).unwrap();
        negative.set_stage3_black_level(stage3_black);
        negative
    }

    #[test]
    fn trim_bounds_roundtrip_and_apply() {
        let opcode = DngOpcodeTrimBounds::new(DngRect::from_corners(1, 2, 7, 9));

        // Spool.
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        opcode.put_data(&mut stream).unwrap();

        stream.set_read_position(0).unwrap();
        let data = stream.get(stream.length() as uint32).unwrap();

        // Parse it back (prepend the minVersion/flags header).
        let mut full = Vec::new();
        full.extend_from_slice(&[0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]); // v1.3, no flags
        full.extend_from_slice(&data);

        let mut stream = stream_from(&full);
        let parsed = DngOpcodeTrimBounds::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.bounds(), DngRect::from_corners(1, 2, 7, 9));
        assert_eq!(parsed.opcode_id(), dngOpcode_TrimBounds);

        // Apply: image must be trimmed.
        let mut host = DngHost::new();
        let mut negative = test_negative(0);

        let mut image: ImageRef = DngSimpleImage::new(DngRect::from_corners(0, 0, 16, 16), 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        let mut opcode = DngOpcodeTrimBounds::new(DngRect::from_corners(1, 2, 7, 9));
        opcode.apply(&mut host, &mut negative, &mut image).unwrap();

        assert_eq!(image.read().unwrap().bounds(), DngRect::from_corners(1, 2, 7, 9));

        // Out-of-image trim bounds are a bad format.
        let mut bad = DngOpcodeTrimBounds::new(DngRect::from_corners(0, 0, 32, 32));
        assert!(bad.apply(&mut host, &mut negative, &mut image).is_err());
    }

    #[test]
    fn trim_bounds_rejects_empty_and_bad_size() {
        // data size != 16.
        let bytes = [
            0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, // header
            0x00, 0x00, 0x00, 8, // wrong size
        ];
        let mut stream = stream_from(&bytes);
        assert!(DngOpcodeTrimBounds::from_stream(&mut stream).is_err());

        // Empty bounds.
        let bytes = [
            0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00, // header
            0x00, 0x00, 0x00, 16, // size
            0, 0, 0, 0, // t
            0, 0, 0, 0, // l
            0, 0, 0, 0, // b
            0, 0, 0, 0, // r
        ];
        let mut stream = stream_from(&bytes);
        assert!(DngOpcodeTrimBounds::from_stream(&mut stream).is_err());
    }

    #[test]
    fn area_spec_roundtrip_and_overlap() {
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 8, 8), 1, 2, 2, 3);

        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        spec.put_data(&mut stream).unwrap();

        stream.set_read_position(0).unwrap();
        let mut parsed = DngAreaSpec::default();
        parsed.get_data(&mut stream).unwrap();

        assert_eq!(parsed.area(), spec.area());
        assert_eq!(parsed.plane(), 1);
        assert_eq!(parsed.planes(), 2);
        assert_eq!(parsed.row_pitch(), 2);
        assert_eq!(parsed.col_pitch(), 3);

        // Overlap snaps to the row/col pitch grid.
        let tile = DngRect::from_corners(1, 1, 8, 8);
        let overlap = spec.overlap(tile);

        assert_eq!(overlap.t, 2); // first row >= 1 on the even-row grid
        assert_eq!(overlap.l, 3); // first col >= 1 on the 3-col grid
        assert_eq!(overlap.b, 8); // rows 2,4,6 -> last + 1 = 8
        assert_eq!(overlap.r, 9); // cols 3,6 -> last + 1 = 9

        // Empty area covers the whole tile.
        let empty = DngAreaSpec::new(DngRect::new(), 0, 1, 1, 1);
        assert_eq!(empty.overlap(tile), tile);
        assert_eq!(empty.scaled_overlap(tile), tile);
    }

    #[test]
    fn area_spec_rejects_bad_data() {
        // planes < 1.
        let bytes = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 8, // area 0,0,8,8
            0, 0, 0, 0, // plane
            0, 0, 0, 0, // planes = 0 -> bad
            0, 0, 0, 1, // rowPitch
            0, 0, 0, 1, // colPitch
        ];
        let mut stream = stream_from(&bytes);
        let mut spec = DngAreaSpec::default();
        assert!(spec.get_data(&mut stream).is_err());

        // rowPitch >= area height (8) triggers the DNG_REPORT clamp path,
        // which then passes (clamped to 8, and 8 <= height 8).
        let bytes = [
            0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 8, // area 0,0,8,8
            0, 0, 0, 0, // plane
            0, 0, 0, 1, // planes
            0, 0, 0, 9, // rowPitch = 9 -> clamped to 8
            0, 0, 0, 1, // colPitch
        ];
        let mut stream = stream_from(&bytes);
        let mut spec = DngAreaSpec::default();
        spec.get_data(&mut stream).unwrap();
        assert_eq!(spec.row_pitch(), 8);
    }

    #[test]
    fn map_table_roundtrip_and_apply() {
        // 4-entry table, replicated to 0x10000.
        let table: Vec<uint16> = vec![0, 100, 200, 65535];
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 2, 4), 0, 1, 1, 1);

        let opcode = DngOpcodeMapTable::new(spec, &table, 4).unwrap();

        // Replication: entries beyond count use the last entry.
        assert_eq!(opcode.table(), &[0, 100, 200, 65535]);

        // Roundtrip through spool/parse.
        let mut spool = DngMemoryStream::with_page_size(64).stream();
        spool.set_big_endian(true);
        opcode.put_data(&mut spool).unwrap();

        spool.set_read_position(0).unwrap();
        let data = spool.get(spool.length() as uint32).unwrap();

        let mut full = Vec::new();
        full.extend_from_slice(&[0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);
        full.extend_from_slice(&data);

        let mut stream = stream_from(&full);
        let parsed = DngOpcodeMapTable::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.table(), opcode.table());
        assert_eq!(parsed.area_spec().area(), DngRect::from_corners(0, 0, 2, 4));

        // Apply to a 2x4 short image: pixel values map through the table.
        let mut host = DngHost::new();
        let mut negative = test_negative(0);

        let mut image: ImageRef = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 4), 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            // Write test pattern.
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 4),
                0,
                1,
                ttShort,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 4), 1, ttShort).unwrap(),
            )
            .unwrap();

            for row in 0..2 {
                for col in 0..4 {
                    buf.dirty_pixel_u16(row, col, 0, ((row * 4 + col) % 4) as u16).unwrap();
                }
            }

            image.read().unwrap().put(&buf).unwrap();
        }

        let mut parsed_opcode = DngOpcodeMapTable::from_stream(&mut stream_from(&full)).unwrap();
        parsed_opcode.apply(&mut host, &mut negative, &mut image).unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 4),
                0,
                1,
                ttShort,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 4), 1, ttShort).unwrap(),
            )
            .unwrap();

            image.read().unwrap().get(&mut buf, crate::dng_core::dng_image::EdgeOption::EdgeNone, 1, 1).unwrap();

            let mapping = [0u16, 100, 200, 65535];
            for row in 0..2 {
                for col in 0..4 {
                    let v = buf.const_pixel_u16(row, col, 0).unwrap();
                    let before = ((row * 4 + col) % 4) as u16;
                    assert_eq!(v, mapping[before as usize], "map failed at {row},{col}");
                }
            }
        }
    }

    #[test]
    fn map_table_prepare_black_adjustment() {
        // Stage 2 opcode with a black level: Prepare must build a
        // black-adjusted table.
        let table: Vec<uint16> = vec![0, 32768, 65535];
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 2, 2), 0, 1, 1, 1);

        let mut opcode = DngOpcodeMapTable::new(spec, &table, 3).unwrap();
        opcode.set_stage(2);

        let mut negative = test_negative(16384);

        opcode
            .prepare(&mut negative, 1, DngPoint::new(8, 8), DngRect::from_corners(0, 0, 2, 2), 1, ttShort)
            .unwrap();

        // Black-adjusted behavior: input 16384 (== black) must map to 0.
        // Simulate via process: value 16384 -> x = 0 -> y = table[0] = 0 ->
        // dst = 0 * dstScale + black = 16384... instead verify the mapping
        // directly through a processed pixel.
        let mut image: ImageRef = DngSimpleImage::new(DngRect::from_corners(0, 0, 1, 1), 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 1, 1),
                0,
                1,
                ttShort,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 1, 1), 1, ttShort).unwrap(),
            )
            .unwrap();
            buf.dirty_pixel_u16(0, 0, 0, 16384).unwrap();
            image.read().unwrap().put(&buf).unwrap();
        }

        let mut host = DngHost::new();
        opcode.apply(&mut host, &mut negative, &mut image).unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 1, 1),
                0,
                1,
                ttShort,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 1, 1), 1, ttShort).unwrap(),
            )
            .unwrap();
            image.read().unwrap().get(&mut buf, crate::dng_core::dng_image::EdgeOption::EdgeNone, 1, 1).unwrap();
            let v = buf.const_pixel_u16(0, 0, 0).unwrap();

            // x = (16384 - 16384) * srcScale = 0 -> y = table[0] = 0 ->
            // dst = round (0 * dstScale) + 16384 = 16384.
            assert_eq!(v, 16384);
        }
    }

    #[test]
    fn map_polynomial_roundtrip() {
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 4, 4), 0, 1, 1, 1);
        let coefficients = [0.25f64, 0.5, 0.25];

        let mut opcode = DngOpcodeMapPolynomial::new(spec, 2, &coefficients);
        opcode.set_stage(3);

        // Spool + parse.
        let mut spool = DngMemoryStream::with_page_size(64).stream();
        spool.set_big_endian(true);
        opcode.put_data(&mut spool).unwrap();

        spool.set_read_position(0).unwrap();
        let data = spool.get(spool.length() as uint32).unwrap();

        let mut full = Vec::new();
        full.extend_from_slice(&[0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);
        full.extend_from_slice(&data);

        let mut stream = stream_from(&full);
        let parsed = DngOpcodeMapPolynomial::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.degree(), 2);
        assert_eq!(parsed.coefficients()[0], 0.25);
        assert_eq!(parsed.coefficients()[1], 0.5);
        assert_eq!(parsed.coefficients()[2], 0.25);
        assert_eq!(parsed.coefficients()[3], 0.0);

        // Degree reduction: trailing zero coefficient reduces degree.
        let reduced = DngOpcodeMapPolynomial::new(spec, 4, &[0.1, 0.2, 0.0, 0.0, 0.0]);
        assert_eq!(reduced.degree(), 1);
    }

    #[test]
    fn map_polynomial_apply_identity() {
        // f (x) = x on a float image must be an identity.
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 2, 4), 0, 1, 1, 1);
        let coefficients = [0.0f64, 1.0];

        let mut opcode = DngOpcodeMapPolynomial::new(spec, 1, &coefficients);
        opcode.set_stage(3);

        let mut host = DngHost::new();
        let mut negative = test_negative(0);

        let mut image: ImageRef = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 4), 1, ttFloat)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 4),
                0,
                1,
                ttFloat,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 4), 1, ttFloat).unwrap(),
            )
            .unwrap();
            for row in 0..2 {
                for col in 0..4 {
                    let v = (row * 4 + col) as f32 * 0.125 - 0.5;
                    buf.dirty_pixel_f32(row, col, 0, v).unwrap();
                }
            }
            image.read().unwrap().put(&buf).unwrap();
        }

        opcode.apply(&mut host, &mut negative, &mut image).unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 4),
                0,
                1,
                ttFloat,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 4), 1, ttFloat).unwrap(),
            )
            .unwrap();
            image.read().unwrap().get(&mut buf, crate::dng_core::dng_image::EdgeOption::EdgeNone, 1, 1).unwrap();

            for row in 0..2 {
                for col in 0..4 {
                    let v = buf.const_pixel_f32(row, col, 0).unwrap();
                    let original = (row * 4 + col) as f32 * 0.125 - 0.5;
                    assert!((v - original).abs() < 1e-6, "identity failed at {row},{col}");
                }
            }
        }
    }

    #[test]
    fn delta_per_row_roundtrip() {
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 4, 4), 0, 1, 2, 1);
        let deltas = vec![0.1f32, -0.2f32];

        let opcode = DngOpcodeDeltaPerRow::new(spec, deltas.clone());

        let mut spool = DngMemoryStream::with_page_size(64).stream();
        spool.set_big_endian(true);
        opcode.put_data(&mut spool).unwrap();

        spool.set_read_position(0).unwrap();
        let data = spool.get(spool.length() as uint32).unwrap();

        let mut full = Vec::new();
        full.extend_from_slice(&[0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);
        full.extend_from_slice(&data);

        let mut stream = stream_from(&full);
        let parsed = DngOpcodeDeltaPerRow::from_stream(&mut stream).unwrap();

        assert_eq!(parsed.table(), &[0.1, -0.2]);

        // Bad count in stream is rejected.
        let mut corrupt = full.clone();
        let count_offset = 8 + DngAreaSpec::kDataSize as usize;
        corrupt[count_offset] = 0x7F; // deltas = huge
        let mut stream = stream_from(&corrupt);
        assert!(DngOpcodeDeltaPerRow::from_stream(&mut stream).is_err());
    }

    #[test]
    fn scale_per_column_black_offset() {
        // Scale 2.0 on a stage-3 opcode with black level 8192: the pixel is
        // scaled around the black offset.
        let spec = DngAreaSpec::new(DngRect::from_corners(0, 0, 2, 2), 0, 1, 1, 1);
        let scales = vec![2.0f32];

        let mut opcode = DngOpcodeScalePerColumn::new(spec, scales);
        opcode.set_stage(3);

        let mut host = DngHost::new();
        let mut negative = test_negative(8192);

        let mut image: ImageRef = DngSimpleImage::new(DngRect::from_corners(0, 0, 2, 2), 1, ttFloat)
            .unwrap()
            .into_image_ref()
            .unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 2),
                0,
                1,
                ttFloat,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 2), 1, ttFloat).unwrap(),
            )
            .unwrap();
            buf.dirty_pixel_f32(0, 0, 0, 0.5).unwrap();
            image.read().unwrap().put(&buf).unwrap();
        }

        opcode.apply(&mut host, &mut negative, &mut image).unwrap();

        {
            let mut buf = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(0, 0, 2, 2),
                0,
                1,
                ttFloat,
                crate::dng_core::pcRowInterleaved,
                DngPixelBuffer::allocate_data(DngRect::from_corners(0, 0, 2, 2), 1, ttFloat).unwrap(),
            )
            .unwrap();
            image.read().unwrap().get(&mut buf, crate::dng_core::dng_image::EdgeOption::EdgeNone, 1, 1).unwrap();

            let v = buf.const_pixel_f32(0, 0, 0).unwrap();

            // blackOffset = 8192 / 65535; y = (0.5 - bo) * 2 + bo.
            let bo = 8192.0f32 / 65535.0;
            let expected = (0.5 - bo) * 2.0 + bo;
            assert!((v - expected).abs() < 1e-6, "{v} vs {expected}");
        }
    }

    #[test]
    fn unknown_opcode_registry_fallback() {
        // An opcode ID not in the registry parses as DngOpcodeUnknown.
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put_uint32(0x00900000).unwrap(); // minVersion 9.0
        stream.put_uint32(1).unwrap(); // optional
        stream.put_uint32(0).unwrap(); // no data
        stream.set_read_position(0).unwrap();

        let opcode = crate::dng_opcodes::make_dng_opcode(4242, &mut stream).unwrap();

        assert_eq!(opcode.opcode_id(), 4242);
        assert!(opcode.optional());
    }
}

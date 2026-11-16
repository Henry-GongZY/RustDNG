// Port of dng_pixel_buffer.h/.cpp — a buffer of pixel data with "pixel
// geometry" metadata (area, planes, steps, pixel type) plus the type
// conversion engine (`copy_area`).
//
// Ownership design (binding, see DESIGN.md): the C++ buffer holds a raw
// `void *fData` view over caller memory.  The Rust port OWNS its pixels in
// a typed storage enum (`PixelData`), with a base element offset so the
// Flip*/Rotate/Trim "move fData, negate step" tricks still work without
// raw pointers.  All C++ call patterns (`Get`/`Put` with temp buffer views)
// are re-expressed as owned buffers; iteration-order-only optimizations
// (`OptimizeOrder`) are omitted (PORT-NOTE below — identical results).
//
// PORT-NOTE(dng_pixel_buffer.cpp:38): `OptimizeOrder` only permutes/reverses
// loop order and collapses contiguous loops for cache/SIMD efficiency; every
// pixel receives the same value either way, so the port iterates rows, then
// columns, then planes directly.
//
// PORT-NOTE: `ConvertPixel`/`GetPixel`/`SetPixel` methods named in the
// phase-1 task do not exist in SDK 1.7.1 — all cross-type conversion runs
// through `CopyArea` (backed by dng_reference.cpp scalars), including the
// ttSShort sign-bit bias ("black at 0x8000") and the white-level clamp via
// `Pin_Overrange` in the R32 -> integer paths.

use crate::dng_exceptions::{not_yet_implemented, program_error};
use crate::dng_errors::DngResult;
use crate::dng_rect::DngRect;
use crate::dng_tag_types::*;
use crate::dng_tag_values::{pcInterleaved, pcPlanar, pcRowInterleaved, pcRowInterleavedAlignSIMD};
use crate::dng_types::{int32, real32, real64, uint32, uint8};
use crate::dng_utils::{max_int32, min_int32, pin_overrange, round_up_for_pixel_size};
use crate::dng_safe_arithmetic::convert_uint32_to_int32;
use half::f16;

/// Typed pixel storage (keeps element alignment valid without `unsafe`).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum PixelData {
    #[default]
    None,
    U8(Vec<uint8>),
    I8(Vec<i8>),
    U16(Vec<u16>),
    I16(Vec<i16>),
    U32(Vec<u32>),
    I32(Vec<i32>),
    U64(Vec<u64>),
    I64(Vec<i64>),
    F16(Vec<f16>),
    F32(Vec<real32>),
    F64(Vec<real64>),
}

impl PixelData {
    pub fn element_len(&self) -> usize {
        match self {
            PixelData::U8(v) => v.len(),
            PixelData::I8(v) => v.len(),
            PixelData::U16(v) => v.len(),
            PixelData::I16(v) => v.len(),
            PixelData::U32(v) => v.len(),
            PixelData::I32(v) => v.len(),
            PixelData::U64(v) => v.len(),
            PixelData::I64(v) => v.len(),
            PixelData::F16(v) => v.len(),
            PixelData::F32(v) => v.len(),
            PixelData::F64(v) => v.len(),
            PixelData::None => 0,
        }
    }

    pub fn as_u8_slice(&self) -> Option<&[uint8]> {
        match self {
            PixelData::U8(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u8_slice_mut(&mut self) -> Option<&mut [uint8]> {
        match self {
            PixelData::U8(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u16_slice(&self) -> Option<&[u16]> {
        match self {
            PixelData::U16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u16_slice_mut(&mut self) -> Option<&mut [u16]> {
        match self {
            PixelData::U16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_s16_slice(&self) -> Option<&[i16]> {
        match self {
            PixelData::I16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_s16_slice_mut(&mut self) -> Option<&mut [i16]> {
        match self {
            PixelData::I16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_u32_slice(&self) -> Option<&[u32]> {
        match self {
            PixelData::U32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f16_slice(&self) -> Option<&[f16]> {
        match self {
            PixelData::F16(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f32_slice(&self) -> Option<&[real32]> {
        match self {
            PixelData::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f32_slice_mut(&mut self) -> Option<&mut [real32]> {
        match self {
            PixelData::F32(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_f64_slice(&self) -> Option<&[real64]> {
        match self {
            PixelData::F64(v) => Some(v),
            _ => None,
        }
    }

    /// Mutable `as_u32_slice`.
    pub fn as_u32_slice_mut(&mut self) -> Option<&mut [u32]> {
        match self {
            PixelData::U32(v) => Some(v),
            _ => None,
        }
    }
}

/// Typed element read/write dispatch.  Values are reinterpreted across
/// element widths exactly like the C++ pointer casts, which is what the
/// bottleneck conversions rely on.
impl PixelData {
    pub fn get_u16(&self, i: usize) -> u16 {
        match self {
            PixelData::U16(v) => v[i],
            PixelData::U8(v) => v[i] as u16,
            PixelData::I16(v) => v[i] as u16,
            PixelData::U32(v) => v[i] as u16,
            PixelData::I32(v) => v[i] as u16,
            PixelData::F16(v) => v[i].to_bits(),
            PixelData::F32(v) => v[i].to_bits() as u16,
            PixelData::F64(v) => v[i].to_bits() as u16,
            _ => panic!("PixelData: bad element width for get_u16"),
        }
    }

    pub fn set_u16(&mut self, i: usize, x: u16) {
        match self {
            PixelData::U16(v) => v[i] = x,
            PixelData::U8(v) => v[i] = x as u8,
            PixelData::I16(v) => v[i] = x as i16,
            PixelData::U32(v) => v[i] = x as u32,
            PixelData::I32(v) => v[i] = x as i32,
            PixelData::F16(v) => v[i] = f16::from_bits(x),
            _ => panic!("PixelData: bad element width for set_u16"),
        }
    }

    pub fn get_i16(&self, i: usize) -> i16 {
        match self {
            PixelData::I16(v) => v[i],
            _ => self.get_u16(i) as i16,
        }
    }

    pub fn set_i16(&mut self, i: usize, x: i16) {
        match self {
            PixelData::I16(v) => v[i] = x,
            PixelData::U16(v) => v[i] = x as u16,
            _ => panic!("PixelData: bad element width for set_i16"),
        }
    }

    pub fn get_u32(&self, i: usize) -> u32 {
        match self {
            PixelData::U32(v) => v[i],
            PixelData::U8(v) => v[i] as u32,
            PixelData::U16(v) => v[i] as u32,
            PixelData::I32(v) => v[i] as u32,
            PixelData::F32(v) => v[i].to_bits(),
            _ => panic!("PixelData: bad element width for get_u32"),
        }
    }

    pub fn set_u32(&mut self, i: usize, x: u32) {
        match self {
            PixelData::U32(v) => v[i] = x,
            PixelData::U8(v) => v[i] = x as u8,
            PixelData::U16(v) => v[i] = x as u16,
            PixelData::I32(v) => v[i] = x as i32,
            // Same-width raw copy (4-byte path) must accept float storage,
            // symmetric with get_u32.
            PixelData::F32(v) => v[i] = real32::from_bits(x),
            _ => panic!("PixelData: bad element width for set_u32"),
        }
    }

    pub fn get_f32(&self, i: usize) -> real32 {
        match self {
            PixelData::F32(v) => v[i],
            _ => real32::from_bits(self.get_u32(i)),
        }
    }

    pub fn set_f32(&mut self, i: usize, x: real32) {
        match self {
            PixelData::F32(v) => v[i] = x,
            PixelData::U32(v) => v[i] = x.to_bits(),
            _ => panic!("PixelData: bad element width for set_f32"),
        }
    }

    pub fn get_u8(&self, i: usize) -> uint8 {
        match self {
            PixelData::U8(v) => v[i],
            PixelData::I8(v) => v[i] as u8,
            _ => self.get_u32(i) as uint8,
        }
    }

    pub fn set_u8(&mut self, i: usize, x: uint8) {
        match self {
            PixelData::U8(v) => v[i] = x,
            PixelData::I8(v) => v[i] = x as i8,
            _ => panic!("PixelData: bad element width for set_u8"),
        }
    }
}

/// Port of `dng_pixel_buffer`.
#[derive(Debug, Clone, Default)]


pub struct DngPixelBuffer {
    /// Area this buffer holds.
    pub area: DngRect,

    /// Range of planes this buffer holds.
    pub plane: uint32,
    pub planes: uint32,

    /// Steps between pixels (in pixels, not bytes).
    pub row_step: int32,
    pub col_step: int32,
    pub plane_step: int32,

    /// Basic pixel type (TIFF tag type code).
    pub pixel_type: uint32,

    /// Size of pixel type in bytes.
    pub pixel_size: uint32,

    /// Base element offset into `data` (emulates C++ `fData` pointer moves).
    base: isize,

    /// Buffer's data.
    pub data: PixelData,

    /// Do we have write-access to this data?
    pub dirty: bool,
}

impl DngPixelBuffer {
    /// Empty buffer (C++ default ctor).
    pub fn new() -> Self {
        DngPixelBuffer {
            area: DngRect::new(),
            plane: 0,
            planes: 1,
            row_step: 1,
            col_step: 1,
            plane_step: 1,
            pixel_type: ttUndefined,
            pixel_size: 0,
            base: 0,
            data: PixelData::None,
            dirty: true,
        }
    }

    /// `dng_pixel_buffer (area, plane, planes, pixelType, planarConfiguration, data)`.
    pub fn with_planar_config(
        area: DngRect,
        plane: uint32,
        planes: uint32,
        pixel_type: uint32,
        planar_configuration: uint32,
        data: PixelData,
    ) -> DngResult<Self> {
        use crate::dng_errors::DngError;
        let overflow_message = "Arithmetic overflow in pixel buffer setup";
        let overflow = || DngError::Unsupported(overflow_message.to_string());

        let pixel_size = tag_type_size(pixel_type);

        let (row_step, col_step, plane_step) = match planar_configuration {
            x if x == pcInterleaved => {
                let col_step = convert_uint32_to_int32(planes).map_err(|_| overflow())?;
                let row_step = crate::dng_safe_arithmetic::safe_uint32_mult2(area.w(), planes)
                    .map_err(|_| overflow())? as i32;
                (row_step, col_step, 1)
            }
            x if x == pcPlanar => {
                let row_step = convert_uint32_to_int32(area.w()).map_err(|_| overflow())?;
                let plane_step = crate::dng_safe_arithmetic::safe_uint32_mult2(area.h(), area.w())
                    .map_err(|_| overflow())? as i32;
                (row_step, 1, plane_step)
            }
            x if x == pcRowInterleaved || x == pcRowInterleavedAlignSIMD => {
                let plane_step_uint32 = if planar_configuration == pcRowInterleaved {
                    area.w()
                } else {
                    round_up_for_pixel_size(area.w(), pixel_size).map_err(|_| overflow())?
                };
                let plane_step =
                    convert_uint32_to_int32(plane_step_uint32).map_err(|_| overflow())?;
                let row_step =
                    crate::dng_safe_arithmetic::safe_uint32_mult2(plane_step_uint32, planes)
                        .map_err(|_| overflow())? as i32;
                (row_step, 1, plane_step)
            }
            _ => return Err(program_error()), // "Invalid value for 'planarConfiguration'"
        };

        Ok(DngPixelBuffer {
            area,
            plane,
            planes,
            row_step,
            col_step,
            plane_step,
            pixel_type,
            pixel_size,
            base: 0,
            data,
            dirty: true,
        })
    }

    /// Interleaved-layout constructor (the common case).
    pub fn new_interleaved(
        area: DngRect,
        plane: uint32,
        planes: uint32,
        pixel_type: uint32,
        data: PixelData,
    ) -> DngResult<Self> {
        Self::with_planar_config(area, plane, planes, pixel_type, pcInterleaved, data)
    }

    /// Allocate for a `pcRowInterleavedAlignSIMD` buffer (row step padded to
    /// the SIMD multiple, matching `ComputeBufferSize (..., padSIMDBytes)`).
    pub fn allocate_data_simd(
        area: DngRect,
        planes: uint32,
        pixel_type: uint32,
    ) -> DngResult<PixelData> {
        let bytes = crate::dng_utils::compute_buffer_size(
            pixel_type,
            area.size(),
            planes,
            crate::dng_utils::PaddingType::padSIMDBytes,
        )?;
        let pixels = (bytes / tag_type_size(pixel_type).max(1)) as usize;
        Ok(match pixel_type {
            ttByte | ttAscii | ttUndefined => PixelData::U8(vec![0; bytes as usize]),
            ttSByte => PixelData::I8(vec![0; bytes as usize]),
            ttShort | ttUnicode => PixelData::U16(vec![0; pixels]),
            ttSShort => PixelData::I16(vec![0; pixels]),
            ttLong | ttIFD => PixelData::U32(vec![0; pixels]),
            ttSLong => PixelData::I32(vec![0; pixels]),
            ttLong8 => PixelData::U64(vec![0; pixels]),
            ttSLong8 => PixelData::I64(vec![0; pixels]),
            ttHalfFloat => PixelData::F16(vec![f16::ZERO; pixels]),
            ttFloat => PixelData::F32(vec![0.0; pixels]),
            ttDouble => PixelData::F64(vec![0.0; pixels]),
            _ => return Err(not_yet_implemented()),
        })
    }

    /// Allocate a zero-filled data vector sized for the buffer geometry.
    pub fn allocate_data(area: DngRect, planes: uint32, pixel_type: uint32) -> DngResult<PixelData> {
        let bytes = crate::dng_utils::compute_buffer_size(
            pixel_type,
            area.size(),
            planes,
            crate::dng_utils::PaddingType::padNone,
        )?;
        let pixels = (bytes / tag_type_size(pixel_type).max(1)) as usize;
        Ok(match pixel_type {
            ttByte | ttAscii | ttUndefined => PixelData::U8(vec![0; bytes as usize]),
            ttSByte => PixelData::I8(vec![0; bytes as usize]),
            ttShort | ttUnicode => PixelData::U16(vec![0; pixels]),
            ttSShort => PixelData::I16(vec![0; pixels]),
            ttLong | ttIFD => PixelData::U32(vec![0; pixels]),
            ttSLong => PixelData::I32(vec![0; pixels]),
            ttLong8 => PixelData::U64(vec![0; pixels]),
            ttSLong8 => PixelData::I64(vec![0; pixels]),
            ttHalfFloat => PixelData::F16(vec![f16::ZERO; pixels]),
            ttFloat => PixelData::F32(vec![0.0; pixels]),
            ttDouble => PixelData::F64(vec![0.0; pixels]),
            _ => return Err(not_yet_implemented()),
        })
    }

    /// `InternalPixel` — element index of a pixel.
    fn pixel_element_index(&self, row: int32, col: int32, plane: uint32) -> DngResult<usize> {
        let offset_pixels = self.row_step as i64 * (row as i64 - self.area.t as i64)
            + self.col_step as i64 * (col as i64 - self.area.l as i64)
            + self.plane_step as i64 * (plane as i64 - self.plane as i64);

        let index = self.base + offset_pixels as isize;

        if index < 0 || index >= self.data.element_len() as isize {
            return Err(program_error()); // "Out-of-range pixel access"
        }

        Ok(index as usize)
    }

    /// Public form of `pixel_element_index` for direct-storage fast paths
    /// (DngSimpleImage rotate/trim; extension documented in DESIGN.md).
    // -------------------------------------------------- typed data views

    /// `as_u8_slice` on the owned pixel data (None when the pixel type is
    /// not ttByte/ttSByte).
    pub fn as_u8_slice(&self) -> Option<&[uint8]> {
        self.data.as_u8_slice()
    }

    /// Mutable `as_u8_slice`.
    pub fn as_u8_slice_mut(&mut self) -> Option<&mut [uint8]> {
        self.data.as_u8_slice_mut()
    }

    /// `as_u16_slice` on the owned pixel data (ttShort/ttSShort).
    pub fn as_u16_slice(&self) -> Option<&[u16]> {
        self.data.as_u16_slice()
    }

    /// Mutable `as_u16_slice`.
    pub fn as_u16_slice_mut(&mut self) -> Option<&mut [u16]> {
        self.data.as_u16_slice_mut()
    }

    /// `as_u32_slice` on the owned pixel data (ttLong family).
    pub fn as_u32_slice(&self) -> Option<&[u32]> {
        self.data.as_u32_slice()
    }

    /// Mutable `as_u32_slice`.
    pub fn as_u32_slice_mut(&mut self) -> Option<&mut [u32]> {
        self.data.as_u32_slice_mut()
    }

    /// `as_f32_slice` on the owned pixel data (ttFloat).
    pub fn as_f32_slice(&self) -> Option<&[real32]> {
        self.data.as_f32_slice()
    }

    /// Mutable `as_f32_slice`.
    pub fn as_f32_slice_mut(&mut self) -> Option<&mut [real32]> {
        self.data.as_f32_slice_mut()
    }

    pub fn pixel_element_index_public(
        &self,
        row: int32,
        col: int32,
        plane: uint32,
    ) -> DngResult<usize> {
        self.pixel_element_index(row, col, plane)
    }

    /// `PixelRange ()`.
    pub fn pixel_range(&self) -> uint32 {
        match self.pixel_type {
            ttByte | ttSByte => 0x0FF,
            ttShort | ttSShort => 0x0FFFF,
            ttLong | ttSLong => 0xFFFFFFFF,
            _ => 0,
        }
    }

    /// `Area ()`.
    pub fn area(&self) -> &DngRect {
        &self.area
    }

    /// `Planes ()`.
    pub fn planes(&self) -> uint32 {
        self.planes
    }

    /// `RowStep ()`.
    pub fn row_step(&self) -> int32 {
        self.row_step
    }

    /// `PlaneStep ()`.
    pub fn plane_step(&self) -> int32 {
        self.plane_step
    }

    // -------------------------------------------------- typed pixel access

    pub fn const_pixel_u16(&self, row: int32, col: int32, plane: uint32) -> DngResult<u16> {
        debug_assert!(self.pixel_type == ttShort, "Pixel type access mismatch");
        Ok(self.data.get_u16(self.pixel_element_index(row, col, plane)?))
    }

    pub fn dirty_pixel_u16(&mut self, row: int32, col: int32, plane: uint32, value: u16) -> DngResult<()> {
        debug_assert!(self.dirty, "Dirty access to const pixel buffer");
        let i = self.pixel_element_index(row, col, plane)?;
        self.data.set_u16(i, value);
        Ok(())
    }

    pub fn const_pixel_u8(&self, row: int32, col: int32, plane: uint32) -> DngResult<uint8> {
        debug_assert!(self.pixel_type == ttByte, "Pixel type access mismatch");
        Ok(self.data.get_u8(self.pixel_element_index(row, col, plane)?))
    }

    pub fn dirty_pixel_u8(&mut self, row: int32, col: int32, plane: uint32, value: uint8) -> DngResult<()> {
        let i = self.pixel_element_index(row, col, plane)?;
        self.data.set_u8(i, value);
        Ok(())
    }

    pub fn const_pixel_u32(&self, row: int32, col: int32, plane: uint32) -> DngResult<u32> {
        debug_assert!(self.pixel_type == ttLong, "Pixel type access mismatch");
        Ok(self.data.get_u32(self.pixel_element_index(row, col, plane)?))
    }

    pub fn dirty_pixel_u32(&mut self, row: int32, col: int32, plane: uint32, value: u32) -> DngResult<()> {
        let i = self.pixel_element_index(row, col, plane)?;
        self.data.set_u32(i, value);
        Ok(())
    }

    pub fn const_pixel_f32(&self, row: int32, col: int32, plane: uint32) -> DngResult<real32> {
        debug_assert!(self.pixel_type == ttFloat, "Pixel type access mismatch");
        Ok(self.data.get_f32(self.pixel_element_index(row, col, plane)?))
    }

    pub fn dirty_pixel_f32(&mut self, row: int32, col: int32, plane: uint32, value: real32) -> DngResult<()> {
        let i = self.pixel_element_index(row, col, plane)?;
        self.data.set_f32(i, value);
        Ok(())
    }

    pub fn const_pixel_i16(&self, row: int32, col: int32, plane: uint32) -> DngResult<i16> {
        debug_assert!(self.pixel_type == ttSShort, "Pixel type access mismatch");
        Ok(self.data.get_i16(self.pixel_element_index(row, col, plane)?))
    }

    pub fn dirty_pixel_i16(&mut self, row: int32, col: int32, plane: uint32, value: i16) -> DngResult<()> {
        let i = self.pixel_element_index(row, col, plane)?;
        self.data.set_i16(i, value);
        Ok(())
    }

    // -------------------------------------------------------- set constant

    /// `SetConstant (area, plane, planes, value)`.
    pub fn set_constant(
        &mut self,
        area: DngRect,
        plane: uint32,
        planes: uint32,
        value: uint32,
    ) -> DngResult<()> {
        let rows = area.h();
        let cols = area.w();

        match self.pixel_size {
            1 => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let i = self.pixel_element_index(
                                area.t + row as int32,
                                area.l + col as int32,
                                (plane as i64 + p) as uint32,
                            )?;
                            self.data.set_u8(i, value as uint8);
                        }
                    }
                }
            }
            2 => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let i = self.pixel_element_index(
                                area.t + row as int32,
                                area.l + col as int32,
                                (plane as i64 + p) as uint32,
                            )?;
                            self.data.set_u16(i, value as u16);
                        }
                    }
                }
            }
            4 => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let i = self.pixel_element_index(
                                area.t + row as int32,
                                area.l + col as int32,
                                (plane as i64 + p) as uint32,
                            )?;
                            self.data.set_u32(i, value);
                        }
                    }
                }
            }
            _ => return Err(not_yet_implemented()),
        }

        Ok(())
    }

    /// `SetConstant_uint8`.
    pub fn set_constant_u8(&mut self, area: DngRect, plane: uint32, planes: uint32, value: uint8) -> DngResult<()> {
        debug_assert!(self.pixel_type == ttByte, "Mismatched pixel type");
        self.set_constant(area, plane, planes, value as uint32)
    }

    /// `SetConstant_uint16`.
    pub fn set_constant_u16(&mut self, area: DngRect, plane: uint32, planes: uint32, value: u16) -> DngResult<()> {
        debug_assert!(self.pixel_type == ttShort, "Mismatched pixel type");
        self.set_constant(area, plane, planes, value as uint32)
    }

    /// `SetConstant_int16`.
    pub fn set_constant_i16(&mut self, area: DngRect, plane: uint32, planes: uint32, value: i16) -> DngResult<()> {
        debug_assert!(self.pixel_type == ttSShort, "Mismatched pixel type");
        self.set_constant(area, plane, planes, value as u16 as uint32)
    }

    /// `SetConstant_uint32`.
    pub fn set_constant_u32(&mut self, area: DngRect, plane: uint32, planes: uint32, value: uint32) -> DngResult<()> {
        debug_assert!(self.pixel_type == ttLong, "Mismatched pixel type");
        self.set_constant(area, plane, planes, value)
    }

    /// `SetConstant_real32`.
    pub fn set_constant_f32(&mut self, area: DngRect, plane: uint32, planes: uint32, value: real32) -> DngResult<()> {
        debug_assert!(self.pixel_type == ttFloat, "Mismatched pixel type");
        self.set_constant(area, plane, planes, value.to_bits())
    }

    /// `SetZero (area, plane, planes)`.
    pub fn set_zero(&mut self, area: DngRect, plane: uint32, planes: uint32) -> DngResult<()> {
        let value: uint32 = match self.pixel_type {
            ttByte | ttShort | ttLong | ttFloat => 0,
            ttSShort => 0x8000,
            _ => return Err(not_yet_implemented()),
        };

        self.set_constant(area, plane, planes, value)
    }

    // ------------------------------------------------------------ copy area

    /// `CopyArea (src, area, srcPlane, dstPlane, planes)`.
    pub fn copy_area_shifted(
        &mut self,
        src: &DngPixelBuffer,
        src_area: DngRect,
        dst_area: DngRect,
        src_plane: uint32,
        dst_plane: uint32,
        planes: uint32,
    ) -> DngResult<()> {
        let rows = dst_area.h();
        let cols = dst_area.w();

        let not_yet = || not_yet_implemented();

        macro_rules! for_each_pixel {
            ($row_var:ident, $col_var:ident, $plane_var:ident, $body:block) => {
                for $row_var in 0..rows as i64 {
                    for $col_var in 0..cols as i64 {
                        for $plane_var in 0..planes as i64 {
                            $body
                        }
                    }
                }
            };
        }

        if self.pixel_type == src.pixel_type {
            // Same pixel type: plain copy (any of the supported sizes).
            match self.pixel_size {
                1 => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u8(di, src.data.get_u8(si));
                    });
                }
                2 => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u16(di, src.data.get_u16(si));
                    });
                }
                4 => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u32(di, src.data.get_u32(si));
                    });
                }
                _ => return Err(not_yet()),
            }
        } else if src.pixel_type == ttByte {
            match self.pixel_type {
                ttShort => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u16(di, src.data.get_u8(si) as u16);
                    });
                }
                ttSShort => {
                    // 8-bit to signed 16: toggle the sign bit (bias by 0x8000).
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_u8(si) as u16;
                        self.data.set_i16(di, (x ^ 0x8000) as i16);
                    });
                }
                ttLong => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u32(di, src.data.get_u8(si) as u32);
                    });
                }
                ttFloat => {
                    let scale = 1.0f32 / src.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_f32(di, scale * src.data.get_u8(si) as real32);
                    });
                }
                _ => return Err(not_yet()),
            }
        } else if src.pixel_type == ttShort {
            match self.pixel_type {
                ttByte => {
                    // Truncate to the low byte (little-endian host behavior
                    // of the C++ `+ (qDNGBigEndian ? 1 : 0)` pointer offset).
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_u16(si);
                        self.data.set_u8(di, (x & 0xFF) as uint8);
                    });
                }
                ttSShort => {
                    // 16-bit to signed 16: toggle the sign bit.
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_u16(si);
                        self.data.set_i16(di, (x ^ 0x8000) as i16);
                    });
                }
                ttLong => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_u32(di, src.data.get_u16(si) as u32);
                    });
                }
                ttFloat => {
                    let scale = 1.0f32 / src.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        self.data.set_f32(di, scale * src.data.get_u16(si) as real32);
                    });
                }
                _ => return Err(not_yet()),
            }
        } else if src.pixel_type == ttSShort {
            match self.pixel_type {
                ttByte => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_i16(si);
                        self.data.set_u8(di, (x as u16 & 0xFF) as uint8);
                    });
                }
                ttShort => {
                    // Moving between signed and unsigned 16-bit just toggles
                    // the sign bit (same "backwards" bottleneck in C++).
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_i16(si);
                        self.data.set_u16(di, (x as u16) ^ 0x8000);
                    });
                }
                ttFloat => {
                    let scale = 1.0f32 / src.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_i16(si) as i64 + 32768;
                        self.data.set_f32(di, scale * x as real32);
                    });
                }
                _ => return Err(not_yet()),
            }
        } else if src.pixel_type == ttLong {
            match self.pixel_type {
                ttByte => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_u32(si);
                        self.data.set_u8(di, (x & 0xFF) as uint8);
                    });
                }
                ttShort => {
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_u32(si);
                        self.data.set_u16(di, (x & 0xFFFF) as u16);
                    });
                }
                _ => return Err(not_yet()),
            }
        } else if src.pixel_type == ttFloat {
            match self.pixel_type {
                ttByte => {
                    let scale = self.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_f32(si);
                        self.data.set_u8(di, (pin_overrange(x) * scale + 0.5) as uint8);
                    });
                }
                ttShort => {
                    let scale = self.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_f32(si);
                        self.data.set_u16(di, (pin_overrange(x) * scale + 0.5) as u16);
                    });
                }
                ttSShort => {
                    let scale = self.pixel_range() as real32;
                    for_each_pixel!(r, c, p, {
                        let si = src.pixel_element_index(src_area.t + r as int32, src_area.l + c as int32, (src_plane as i64 + p) as uint32)?;
                        let di = self.pixel_element_index(dst_area.t + r as int32, dst_area.l + c as int32, (dst_plane as i64 + p) as uint32)?;
                        let x = src.data.get_f32(si);
                        let xi = (pin_overrange(x) * scale + 0.5) as int32;
                        self.data.set_i16(di, ((xi as u16) ^ 0x8000) as i16);
                    });
                }
                _ => return Err(not_yet()),
            }
        } else {
            return Err(not_yet());
        }

        Ok(())
    }

    /// `CopyArea (src, area, plane, planes)` — same-area variant.
    pub fn copy_area(
        &mut self,
        src: &DngPixelBuffer,
        area: DngRect,
        src_plane: uint32,
        dst_plane: uint32,
        planes: uint32,
    ) -> DngResult<()> {
        self.copy_area_shifted(src, area, area, src_plane, dst_plane, planes)
    }

    /// `CopyArea (src, area, plane, planes)`.
    pub fn copy_area_same_plane(
        &mut self,
        src: &DngPixelBuffer,
        area: DngRect,
        plane: uint32,
        planes: uint32,
    ) -> DngResult<()> {
        self.copy_area(src, area, plane, plane, planes)
    }

    // ------------------------------------------------------------ repeating

    /// `RepeatPhase (srcArea, dstArea)` — static.
    pub fn repeat_phase(src_area: DngRect, dst_area: DngRect) -> crate::dng_point::DngPoint {
        use crate::dng_point::DngPoint;

        let repeat_v = src_area.h() as int32;
        let repeat_h = src_area.w() as int32;

        if repeat_v == 0 || repeat_h == 0 {
            // "Bad srcArea in RepeatPhase"
            return DngPoint::new(0, 0);
        }

        let phase_v = if src_area.t >= dst_area.t {
            (repeat_v - ((src_area.t - dst_area.t) % repeat_v)) % repeat_v
        } else {
            (dst_area.t - src_area.t) % repeat_v
        };

        let phase_h = if src_area.l >= dst_area.l {
            (repeat_h - ((src_area.l - dst_area.l) % repeat_h)) % repeat_h
        } else {
            (dst_area.l - src_area.l) % repeat_h
        };

        DngPoint::new(phase_v, phase_h)
    }

    /// `RepeatArea (srcArea, dstArea)`.
    pub fn repeat_area(&mut self, src_area: DngRect, dst_area: DngRect) -> DngResult<()> {
        let repeat = src_area.size();
        let phase = Self::repeat_phase(src_area, dst_area);

        let rows = dst_area.h();
        let cols = dst_area.w();

        let mut src_row = src_area.t + phase.v;
        let mut phase_v = phase.v;

        for drow in 0..rows as i64 {
            let mut src_col = src_area.l + phase.h;
            let mut col_phase = phase.h as uint32;

            for dcol in 0..cols as i64 {
                for p in 0..self.planes as i64 {
                    let si = self.pixel_element_index(
                        src_row,
                        src_col,
                        (self.plane as i64 + p) as uint32,
                    )?;
                    let di = self.pixel_element_index(
                        dst_area.t + drow as int32,
                        dst_area.l + dcol as int32,
                        (self.plane as i64 + p) as uint32,
                    )?;

                    match self.pixel_size {
                        1 => {
                            let v = self.data.get_u8(si);
                            self.data.set_u8(di, v);
                        }
                        2 => {
                            let v = self.data.get_u16(si);
                            self.data.set_u16(di, v);
                        }
                        4 => {
                            let v = self.data.get_u32(si);
                            self.data.set_u32(di, v);
                        }
                        _ => return Err(not_yet_implemented()),
                    }
                }

                col_phase += 1;
                if col_phase as int32 == repeat.h {
                    col_phase = 0;
                    src_col -= repeat.h - 1;
                } else {
                    src_col += 1;
                }
            }

            phase_v += 1;
            if phase_v == repeat.v {
                phase_v = 0;
                src_row -= repeat.v - 1;
            } else {
                src_row += 1;
            }
        }

        Ok(())
    }

    /// `RepeatSubArea (subArea, repeatV, repeatH)`.
    pub fn repeat_sub_area(&mut self, sub_area: DngRect, repeat_v: uint32, repeat_h: uint32) -> DngResult<()> {
        let area = self.area;

        if area.t < sub_area.t {
            let src = DngRect::from_corners(sub_area.t, area.l, sub_area.t + repeat_v as int32, area.r);
            let dst = DngRect::from_corners(area.t, area.l, sub_area.t, area.r);
            self.repeat_area(src, dst)?;
        }

        if area.b > sub_area.b {
            let src = DngRect::from_corners(sub_area.b - repeat_v as int32, area.l, sub_area.b, area.r);
            let dst = DngRect::from_corners(sub_area.b, area.l, area.b, area.r);
            self.repeat_area(src, dst)?;
        }

        if area.l < sub_area.l {
            let src = DngRect::from_corners(area.t, sub_area.l, area.b, sub_area.l + repeat_h as int32);
            let dst = DngRect::from_corners(area.t, area.l, area.b, sub_area.l);
            self.repeat_area(src, dst)?;
        }

        if area.r > sub_area.r {
            let src = DngRect::from_corners(area.t, sub_area.r - repeat_h as int32, area.b, sub_area.r);
            let dst = DngRect::from_corners(area.t, sub_area.r, area.b, area.r);
            self.repeat_area(src, dst)?;
        }

        Ok(())
    }

    /// `ShiftRight (shift)` — ttShort buffers only.
    pub fn shift_right(&mut self, shift: uint32) -> DngResult<()> {
        if self.pixel_type != ttShort {
            return Err(not_yet_implemented());
        }

        let rows = self.area.h();
        let cols = self.area.w();

        for row in 0..rows as i64 {
            for col in 0..cols as i64 {
                for p in 0..self.planes as i64 {
                    let i = self.pixel_element_index(
                        self.area.t + row as int32,
                        self.area.l + col as int32,
                        (self.plane as i64 + p) as uint32,
                    )?;
                    let x = self.data.get_u16(i);
                    self.data.set_u16(i, x >> shift);
                }
            }
        }

        Ok(())
    }

    /// Offset in elements of a pixel relative to the current base.
    fn base_offset_of(&self, row: int32, col: int32, plane: uint32) -> isize {
        (self.row_step as i64 * (row as i64 - self.area.t as i64)
            + self.col_step as i64 * (col as i64 - self.area.l as i64)
            + self.plane_step as i64 * (plane as i64 - self.plane as i64)) as isize
    }

    /// `FlipH ()` — re-anchor the base at old pixel (t, r-1) and negate the
    /// column step (exactly the C++ pointer/step manipulation).
    pub fn flip_h(&mut self) -> DngResult<()> {
        self.base = self.pixel_element_index(self.area.t, self.area.r - 1, self.plane)? as isize;
        self.col_step = -self.col_step;
        Ok(())
    }

    /// `FlipV ()` — re-anchor at old pixel (b-1, l), negate row step.
    pub fn flip_v(&mut self) -> DngResult<()> {
        self.base = self.pixel_element_index(self.area.b - 1, self.area.l, self.plane)? as isize;
        self.row_step = -self.row_step;
        Ok(())
    }

    /// `FlipZ ()` — re-anchor at old pixel (t, l, planes-1), negate plane step.
    pub fn flip_z(&mut self) -> DngResult<()> {
        self.base = self.pixel_element_index(self.area.t, self.area.l, self.planes - 1)? as isize;
        self.plane_step = -self.plane_step;
        Ok(())
    }

    // -------------------------------------------------------------- compare

    /// `EqualArea (src, area, plane, planes)`.
    pub fn equal_area(
        &self,
        src: &DngPixelBuffer,
        area: DngRect,
        plane: uint32,
        planes: uint32,
    ) -> DngResult<bool> {
        let rows = area.h();
        let cols = area.w();

        if self.pixel_type != src.pixel_type {
            return Ok(false);
        }

        for row in 0..rows as i64 {
            for col in 0..cols as i64 {
                for p in 0..planes as i64 {
                    let di = self.pixel_element_index(
                        area.t + row as int32,
                        area.l + col as int32,
                        (plane as i64 + p) as uint32,
                    )?;
                    let si = src.pixel_element_index(
                        area.t + row as int32,
                        area.l + col as int32,
                        (plane as i64 + p) as uint32,
                    )?;

                    let equal = match self.pixel_size {
                        1 => self.data.get_u8(di) == src.data.get_u8(si),
                        2 => self.data.get_u16(di) == src.data.get_u16(si),
                        4 => self.data.get_u32(di) == src.data.get_u32(si),
                        _ => return Err(not_yet_implemented()),
                    };

                    if !equal {
                        return Ok(false);
                    }
                }
            }
        }

        Ok(true)
    }

    /// `MaximumDifference (rhs, area, plane, planes)`.
    pub fn maximum_difference(
        &self,
        rhs: &DngPixelBuffer,
        area: DngRect,
        plane: uint32,
        planes: uint32,
    ) -> DngResult<real64> {
        use crate::dng_exceptions::program_error;
        if self.pixel_type != rhs.pixel_type {
            return Err(program_error()); // "attempt to difference pixel buffers of different formats."
        }

        let rows = area.h();
        let cols = area.w();

        let mut result: real64 = 0.0;

        macro_rules! diff_loop {
            ($get_a:ident, $get_b:ident) => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let a = self.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let b = rhs.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let diff = (self.data.$get_a(a) as real64 - rhs.data.$get_b(b) as real64).abs();
                            if diff > result {
                                result = diff;
                            }
                        }
                    }
                }
            };
        }

        match self.pixel_type {
            ttByte => diff_loop!(get_u8, get_u8),
            ttShort => diff_loop!(get_u16, get_u16),
            ttLong => diff_loop!(get_u32, get_u32),
            ttSByte | ttSShort => diff_loop!(get_i16, get_i16),
            ttSLong => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let a = self.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let b = rhs.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let diff = (self.data.get_u32(a) as i32 as real64
                                - rhs.data.get_u32(b) as i32 as real64)
                                .abs();
                            if diff > result {
                                result = diff;
                            }
                        }
                    }
                }
            }
            ttFloat => diff_loop!(get_f32, get_f32),
            ttDouble => {
                for row in 0..rows as i64 {
                    for col in 0..cols as i64 {
                        for p in 0..planes as i64 {
                            let a = self.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let b = rhs.pixel_element_index(area.t + row as int32, area.l + col as int32, (plane as i64 + p) as uint32)?;
                            let diff = (self.data.get_f32(a) as real64 - rhs.data.get_f32(b) as real64).abs();
                            if diff > result {
                                result = diff;
                            }
                        }
                    }
                }
            }
            _ => return Err(not_yet_implemented()),
        }

        Ok(result)
    }
}

#[allow(unused)]
fn unused_min_max(a: int32, b: int32) -> int32 {
    min_int32(a, max_int32(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(t: i32, l: i32, b: i32, r: i32) -> DngRect {
        DngRect::from_corners(t, l, b, r)
    }

    #[test]
    fn steps_for_planar_configs() {
        let area = rect(0, 0, 4, 6);

        let b = DngPixelBuffer::with_planar_config(
            area, 0, 3, ttShort, pcInterleaved, PixelData::U16(vec![0; 6 * 4 * 3]),
        )
        .unwrap();
        assert_eq!(b.col_step, 3);
        assert_eq!(b.row_step, 18);
        assert_eq!(b.plane_step, 1);

        let b = DngPixelBuffer::with_planar_config(
            area, 0, 3, ttShort, pcPlanar, PixelData::U16(vec![0; 6 * 4 * 3]),
        )
        .unwrap();
        assert_eq!(b.col_step, 1);
        assert_eq!(b.row_step, 6);
        assert_eq!(b.plane_step, 24);

        let b = DngPixelBuffer::with_planar_config(
            area, 0, 3, ttShort, pcRowInterleaved, PixelData::U16(vec![0; 6 * 4 * 3]),
        )
        .unwrap();
        assert_eq!(b.plane_step, 6);
        assert_eq!(b.row_step, 18);

        // Invalid config.
        assert!(DngPixelBuffer::with_planar_config(area, 0, 3, ttShort, 99, PixelData::U16(vec![0; 72])).is_err());
    }

    #[test]
    fn pixel_range() {
        let area = rect(0, 0, 2, 2);
        let b = DngPixelBuffer::new_interleaved(area, 0, 1, ttByte, PixelData::U8(vec![0; 4])).unwrap();
        assert_eq!(b.pixel_range(), 0xFF);
        let b = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 4])).unwrap();
        assert_eq!(b.pixel_range(), 0xFFFF);
        let b = DngPixelBuffer::new_interleaved(area, 0, 1, ttLong, PixelData::U32(vec![0; 4])).unwrap();
        assert_eq!(b.pixel_range(), 0xFFFFFFFF);
        let b = DngPixelBuffer::new_interleaved(area, 0, 1, ttFloat, PixelData::F32(vec![0.0; 4])).unwrap();
        assert_eq!(b.pixel_range(), 0);
    }

    #[test]
    fn set_constant_and_access() {
        let area = rect(2, 3, 6, 9);
        let mut b = DngPixelBuffer::new_interleaved(area, 0, 2, ttShort, PixelData::U16(vec![0; 4 * 6 * 2])).unwrap();

        b.set_constant(rect(3, 4, 5, 7), 0, 2, 1234).unwrap();

        assert_eq!(b.const_pixel_u16(3, 4, 0).unwrap(), 1234);
        assert_eq!(b.const_pixel_u16(4, 6, 1).unwrap(), 1234);
        // Outside the set area but inside buffer: still zero.
        assert_eq!(b.const_pixel_u16(5, 8, 0).unwrap(), 0);

        // Out-of-range access errors.
        assert!(b.const_pixel_u16(2, 3, 0).is_ok());
        assert!(b.const_pixel_u16(6, 3, 0).is_err());
    }

    #[test]
    fn set_zero_sshort_bias() {
        let area = rect(0, 0, 2, 2);
        let mut b = DngPixelBuffer::new_interleaved(area, 0, 1, ttSShort, PixelData::I16(vec![0; 4])).unwrap();
        b.set_zero(area, 0, 1).unwrap();
        assert_eq!(b.const_pixel_i16(0, 0, 0).unwrap(), -32768); // 0x8000
    }

    #[test]
    fn copy_area_u16_to_f32() {
        let area = rect(0, 0, 2, 3);

        let mut src = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 6])).unwrap();
        src.dirty_pixel_u16(0, 0, 0, 0xFFFF).unwrap();
        src.dirty_pixel_u16(0, 1, 0, 0x8000).unwrap();
        src.dirty_pixel_u16(0, 2, 0, 0).unwrap();

        let mut dst = DngPixelBuffer::new_interleaved(area, 0, 1, ttFloat, PixelData::F32(vec![0.0; 6])).unwrap();
        dst.copy_area(&src, area, 0, 0, 1).unwrap();

        assert!((dst.const_pixel_f32(0, 0, 0).unwrap() - 1.0).abs() < 1e-6);
        assert!((dst.const_pixel_f32(0, 1, 0).unwrap() - (0x8000 as f32 / 65535.0)).abs() < 1e-6);
        assert_eq!(dst.const_pixel_f32(0, 2, 0).unwrap(), 0.0);
    }

    #[test]
    fn copy_area_f32_to_u16_white_clamp() {
        let area = rect(0, 0, 1, 5);

        let mut src = DngPixelBuffer::new_interleaved(area, 0, 1, ttFloat, PixelData::F32(vec![0.0; 5])).unwrap();
        src.dirty_pixel_f32(0, 0, 0, 0.0).unwrap();
        src.dirty_pixel_f32(0, 1, 0, 1.0).unwrap();
        src.dirty_pixel_f32(0, 2, 0, 0.5).unwrap();
        src.dirty_pixel_f32(0, 3, 0, 2.0).unwrap(); // overrange -> clamps to max
        src.dirty_pixel_f32(0, 4, 0, -1.0).unwrap(); // -> clamps to 0

        let mut dst = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 5])).unwrap();
        dst.copy_area(&src, area, 0, 0, 1).unwrap();

        assert_eq!(dst.const_pixel_u16(0, 0, 0).unwrap(), 0);
        assert_eq!(dst.const_pixel_u16(0, 1, 0).unwrap(), 65535);
        assert!((dst.const_pixel_u16(0, 2, 0).unwrap() as i32 - 32768).abs() <= 1);
        assert_eq!(dst.const_pixel_u16(0, 3, 0).unwrap(), 65535);
        assert_eq!(dst.const_pixel_u16(0, 4, 0).unwrap(), 0);
    }

    #[test]
    fn copy_area_short_sshort_sign_bit() {
        let area = rect(0, 0, 2, 2);

        let mut src = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 4])).unwrap();
        src.dirty_pixel_u16(0, 0, 0, 0).unwrap();
        src.dirty_pixel_u16(0, 1, 0, 0xFFFF).unwrap();

        let mut dst = DngPixelBuffer::new_interleaved(area, 0, 1, ttSShort, PixelData::I16(vec![0; 4])).unwrap();
        dst.copy_area(&src, area, 0, 0, 1).unwrap();

        // Sign bit toggled: 0 -> 0x8000 (i16::MIN), 0xFFFF -> 0x7FFF (max).
        assert_eq!(dst.const_pixel_i16(0, 0, 0).unwrap(), i16::MIN);
        assert_eq!(dst.const_pixel_i16(0, 1, 0).unwrap(), i16::MAX);

        // And back.
        let mut back = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 4])).unwrap();
        back.copy_area(&dst, area, 0, 0, 1).unwrap();
        assert_eq!(back.const_pixel_u16(0, 0, 0).unwrap(), 0);
        assert_eq!(back.const_pixel_u16(0, 1, 0).unwrap(), 0xFFFF);
    }

    #[test]
    fn copy_area_truncation_semantics() {
        // Short -> Byte keeps the LOW byte (no scaling) per C++.
        let area = rect(0, 0, 1, 2);
        let mut src = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0; 2])).unwrap();
        src.dirty_pixel_u16(0, 0, 0, 0x12FF).unwrap();

        let mut dst = DngPixelBuffer::new_interleaved(area, 0, 1, ttByte, PixelData::U8(vec![0; 2])).unwrap();
        dst.copy_area(&src, area, 0, 0, 1).unwrap();
        assert_eq!(dst.const_pixel_u8(0, 0, 0).unwrap(), 0xFF);
    }

    #[test]
    fn repeat_area_tiling() {
        // 2x2 source pattern repeated into 4x4.
        let buf_area = rect(0, 0, 4, 4);
        let mut buf = DngPixelBuffer::new_interleaved(buf_area, 0, 1, ttByte, PixelData::U8(vec![0; 16])).unwrap();

        // Fill a 2x2 region at (1,1) with a recognizable pattern.
        let src_area = rect(1, 1, 3, 3);
        buf.set_constant(src_area, 0, 1, 7).unwrap();
        buf.dirty_pixel_u8(1, 2, 0, 9).unwrap();

        let dst_area = rect(0, 0, 4, 4);
        buf.repeat_area(src_area, dst_area).unwrap();

        // Pattern tiles with the phase computed by RepeatPhase: the first
        // dst pixel reads src(2,2).  Row 0 is all 7; odd rows alternate
        // 9,7,9,7 (src(1,2)=9, src(1,1)=7).
        for row in 0..4 {
            for col in 0..4 {
                let v = buf.const_pixel_u8(row, col, 0).unwrap();
                let expect = if row % 2 == 1 && col % 2 == 0 { 9 } else { 7 };
                assert_eq!(v, expect, "row={row} col={col}");
            }
        }
    }

    #[test]
    fn repeat_phase_calc() {
        use crate::dng_point::DngPoint;
        // src at (0,0) size 2x2, dst at (0,0): phase 0,0.
        let p = DngPixelBuffer::repeat_phase(rect(0, 0, 2, 2), rect(0, 0, 4, 4));
        assert_eq!(p, DngPoint::new(0, 0));

        // dst shifted by 1: phase (1,1).
        let p = DngPixelBuffer::repeat_phase(rect(0, 0, 2, 2), rect(1, 1, 5, 5));
        assert_eq!(p, DngPoint::new(1, 1));
    }

    #[test]
    fn shift_right_16() {
        let area = rect(0, 0, 2, 2);
        let mut b = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![0xFFFF; 4])).unwrap();
        b.shift_right(4).unwrap();
        assert_eq!(b.const_pixel_u16(0, 0, 0).unwrap(), 0x0FFF);

        // Unsupported type.
        let mut f = DngPixelBuffer::new_interleaved(area, 0, 1, ttFloat, PixelData::F32(vec![0.0; 4])).unwrap();
        assert!(f.shift_right(1).is_err());
    }

    #[test]
    fn flips() {
        let area = rect(0, 0, 2, 3);
        let mut b = DngPixelBuffer::new_interleaved(area, 0, 1, ttByte, PixelData::U8(vec![0; 6])).unwrap();
        for col in 0..3 {
            b.dirty_pixel_u8(0, col, 0, col as u8 + 1).unwrap();
        }

        b.flip_h().unwrap();
        assert_eq!(b.col_step, -1);
        // Reading row 0 now goes right-to-left: pixel at col 2 is old col 0.
        assert_eq!(b.const_pixel_u8(0, 2, 0).unwrap(), 1);
        assert_eq!(b.const_pixel_u8(0, 0, 0).unwrap(), 3);

        let mut v = DngPixelBuffer::new_interleaved(area, 0, 1, ttByte, PixelData::U8(vec![0; 6])).unwrap();
        v.dirty_pixel_u8(0, 0, 0, 11).unwrap();
        v.dirty_pixel_u8(1, 0, 0, 22).unwrap();
        v.flip_v().unwrap();
        assert_eq!(v.row_step, -3); // row_step == -(area width)
        assert_eq!(v.const_pixel_u8(1, 0, 0).unwrap(), 11);
        assert_eq!(v.const_pixel_u8(0, 0, 0).unwrap(), 22);
    }

    #[test]
    fn equal_and_max_difference() {
        let area = rect(0, 0, 2, 2);
        let a = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![100; 4])).unwrap();
        let mut b = DngPixelBuffer::new_interleaved(area, 0, 1, ttShort, PixelData::U16(vec![100; 4])).unwrap();
        assert!(a.equal_area(&b, area, 0, 1).unwrap());
        assert_eq!(a.maximum_difference(&b, area, 0, 1).unwrap(), 0.0);

        b.dirty_pixel_u16(1, 1, 0, 190).unwrap();
        assert!(!a.equal_area(&b, area, 0, 1).unwrap());
        assert_eq!(a.maximum_difference(&b, area, 0, 1).unwrap(), 90.0);

        // Different types -> not equal.
        let c = DngPixelBuffer::new_interleaved(area, 0, 1, ttByte, PixelData::U8(vec![100; 4])).unwrap();
        assert!(!a.equal_area(&c, area, 0, 1).unwrap());
    }

    #[test]
    fn repeat_sub_area() {
        // Buffer 0..4, sub area 1..3 with repeat 1: edges replicate.
        let buf_area = rect(0, 0, 4, 4);
        let mut b = DngPixelBuffer::new_interleaved(buf_area, 0, 1, ttByte, PixelData::U8(vec![0; 16])).unwrap();
        b.set_constant(rect(1, 1, 3, 3), 0, 1, 5).unwrap();
        b.dirty_pixel_u8(1, 1, 0, 1).unwrap();
        b.dirty_pixel_u8(1, 2, 0, 2).unwrap();
        b.dirty_pixel_u8(2, 1, 0, 3).unwrap();
        b.dirty_pixel_u8(2, 2, 0, 4).unwrap();

        b.repeat_sub_area(rect(1, 1, 3, 3), 1, 1).unwrap();

        // After top/bottom row repeats and left/right column repeats:
        // rows 0-1 = [1,1,2,2], rows 2-3 = [3,3,4,4].
        assert_eq!(b.const_pixel_u8(0, 0, 0).unwrap(), 1);
        assert_eq!(b.const_pixel_u8(0, 1, 0).unwrap(), 1);
        assert_eq!(b.const_pixel_u8(0, 2, 0).unwrap(), 2);
        assert_eq!(b.const_pixel_u8(3, 2, 0).unwrap(), 4);
        assert_eq!(b.const_pixel_u8(2, 3, 0).unwrap(), 4);
    }
}

/// Index snapshot for map-opcode loops: plain values so the loops can compute
/// pixel indices while holding a mutable slice borrow.
#[derive(Debug, Clone, Copy)]
pub struct PixelIndexer {
    base: usize,
    row_step: i64,
    col_step: i64,
    plane_step: i64,
    area_t: int32,
    area_l: int32,
    buffer_plane: uint32,
}

impl PixelIndexer {
    /// Capture the index state of `buffer` (plain values, no borrow held).
    pub fn snapshot(buffer: &DngPixelBuffer) -> Self {
        PixelIndexer {
            base: buffer.base as usize,
            row_step: buffer.row_step as i64,
            col_step: buffer.col_step as i64,
            plane_step: buffer.plane_step as i64,
            area_t: buffer.area.t,
            area_l: buffer.area.l,
            buffer_plane: buffer.plane,
        }
    }

    /// `pixel_element_index (row, col, plane)` on the captured state.
    pub fn index(&self, row: int32, col: int32, plane: uint32) -> usize {
        let offset = self.row_step * (row as i64 - self.area_t as i64)
            + self.col_step * (col as i64 - self.area_l as i64)
            + self.plane_step * (plane as i64 - self.buffer_plane as i64);

        (self.base as i64 + offset) as usize
    }
}

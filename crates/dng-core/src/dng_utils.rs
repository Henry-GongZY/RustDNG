// Port of dng_utils.h/.cpp — general purpose utilities: min/max/pin helpers,
// rounding, byte swapping, half/FP24 float conversions, HSV conversions,
// timers, dither noise, histogram/limit-float-depth image helpers and the
// DNG 1.7 image sequence/stats metadata helpers.
//
// SIMD/AVX paths of the C++ are scalar-only here (see dng_bottlenecks.rs and
// PORTING_NOTES.md); results are bit-identical because the reference scalar
// math is used.

use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::{bad_format, memory_full, overflow_error, program_error};

/// Marker type used by modules that mirror C++ `uint64` out-params.
pub enum Pin_uint64_unused {}
use crate::dng_point::{distance_squared, DngPointReal64};
use crate::dng_area_task::DngAreaTask;
use crate::dng_rect::{DngRect, DngRectReal64};
use crate::dng_types::{int16, int32, int64, real32, real64, uint16, uint32, uint64, uint8};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::OnceLock;

// ---------------------------------------------------------------- int32

/// `Abs_int32` — branchless wrap-around abs (reference behavior preserved).
#[inline]
pub fn abs_int32(x: int32) -> uint32 {
    // Reference version: (uint32) (x < 0 ? -x : x) — including INT_MIN.
    (x as int64).unsigned_abs() as uint32
}

#[inline]
pub fn min_int32(x: int32, y: int32) -> int32 {
    if x <= y { x } else { y }
}

#[inline]
pub fn max_int32(x: int32, y: int32) -> int32 {
    if x >= y { x } else { y }
}

#[inline]
pub fn pin_int32(min: int32, x: int32, max: int32) -> int32 {
    max_int32(min, min_int32(x, max))
}

#[inline]
pub fn pin_int32_between(a: int32, x: int32, b: int32) -> int32 {
    let (min, max) = if a < b { (a, b) } else { (b, a) };
    pin_int32(min, x, max)
}

// ---------------------------------------------------------------- int16/uint16

#[inline]
pub fn min_uint16(x: uint16, y: uint16) -> uint16 {
    if x <= y { x } else { y }
}

#[inline]
pub fn max_uint16(x: uint16, y: uint16) -> uint16 {
    if x >= y { x } else { y }
}

#[inline]
pub fn pin_int16(x: int32) -> int16 {
    pin_int32(-32768, x, 32767) as int16
}

// ---------------------------------------------------------------- uint32

#[inline]
pub fn min_uint32(x: uint32, y: uint32) -> uint32 {
    if x <= y { x } else { y }
}

#[inline]
pub fn min_uint32_3(x: uint32, y: uint32, z: uint32) -> uint32 {
    min_uint32(x, min_uint32(y, z))
}

#[inline]
pub fn max_uint32(x: uint32, y: uint32) -> uint32 {
    if x >= y { x } else { y }
}

#[inline]
pub fn max_uint32_3(x: uint32, y: uint32, z: uint32) -> uint32 {
    max_uint32(x, max_uint32(y, z))
}

#[inline]
pub fn pin_uint32(min: uint32, x: uint32, max: uint32) -> uint32 {
    max_uint32(min, min_uint32(x, max))
}

/// `Pin_uint16 (int32)` — single-branch version preserved exactly.
#[inline]
pub fn pin_uint16(x: int32) -> uint16 {
    let mut x = x;
    if (x & !65535) != 0 {
        x = (!x) >> 31;
    }
    x as uint16
}

// ---------------------------------------------------------------- round up/down

#[inline]
pub fn round_up_2(x: uint32) -> uint32 {
    (x.wrapping_add(1)) & !1u32
}

#[inline]
pub fn round_up_4(x: uint32) -> uint32 {
    (x.wrapping_add(3)) & !3u32
}

#[inline]
pub fn round_up_8(x: uint32) -> uint32 {
    (x.wrapping_add(7)) & !7u32
}

#[inline]
pub fn round_up_16(x: uint32) -> uint32 {
    (x.wrapping_add(15)) & !15u32
}

#[inline]
pub fn round_up_32(x: uint32) -> uint32 {
    (x.wrapping_add(31)) & !31u32
}

#[inline]
pub fn round_up_simd(x: uint32) -> uint32 {
    if crate::dng_flags::qDNGAVXSupport {
        round_up_32(x)
    } else {
        round_up_16(x)
    }
}

#[inline]
pub fn round_up_4096(x: uint32) -> uint32 {
    (x.wrapping_add(4095)) & !4095u32
}

#[inline]
pub fn round_down_2(x: uint32) -> uint32 {
    x & !1u32
}

#[inline]
pub fn round_down_4(x: uint32) -> uint32 {
    x & !3u32
}

#[inline]
pub fn round_down_8(x: uint32) -> uint32 {
    x & !7u32
}

#[inline]
pub fn round_down_16(x: uint32) -> uint32 {
    x & !15u32
}

#[inline]
pub fn round_down_32(x: uint32) -> uint32 {
    x & !31u32
}

#[inline]
pub fn round_down_simd(x: uint32) -> uint32 {
    if crate::dng_flags::qDNGAVXSupport {
        round_down_32(x)
    } else {
        round_down_16(x)
    }
}

/// `RoundUpForPixelSize (x, pixelSize, &result)` — bool form.
pub fn try_round_up_for_pixel_size(x: uint32, pixel_size: uint32) -> Option<uint32> {
    const TARGET_MULTIPLE_AVX: uint32 = 32;
    const TARGET_MULTIPLE: uint32 = 16;
    let k_target_multiple = if crate::dng_flags::qDNGAVXSupport {
        TARGET_MULTIPLE_AVX
    } else {
        TARGET_MULTIPLE
    };

    let multiple = match pixel_size {
        1 | 2 | 4 | 8 => k_target_multiple / pixel_size,
        _ => k_target_multiple,
    };

    crate::dng_safe_arithmetic::try_round_up_uint32_to_multiple(x, multiple)
}

/// `RoundUpForPixelSize (x, pixelSize)` — throwing form.
pub fn round_up_for_pixel_size(x: uint32, pixel_size: uint32) -> DngResult<uint32> {
    try_round_up_for_pixel_size(x, pixel_size).ok_or_else(|| overflow_error())
}

/// `RoundUpForPixelSizeAsInt32`.
pub fn round_up_for_pixel_size_as_int32(x: uint32, pixel_size: uint32) -> DngResult<int32> {
    let result = round_up_for_pixel_size(x, pixel_size)?;
    crate::dng_safe_arithmetic::convert_uint32_to_int32(result)
}

/// Type of padding to be performed by `ComputeBufferSize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaddingType {
    /// Don't perform any padding.
    padNone,
    /// Pad each scanline to an integer multiple of SIMD vector width bytes.
    padSIMDBytes,
}

/// `ComputeBufferSize` — bytes required for a tile.
pub fn compute_buffer_size(
    pixel_type: uint32,
    tile_size: crate::dng_point::DngPoint,
    num_planes: uint32,
    padding_type: PaddingType,
) -> DngResult<uint32> {
    if tile_size.h < 0 || tile_size.v < 0 {
        return Err(memory_full()); // "Negative tile size"
    }

    let tile_size_h = tile_size.h as uint32;
    let tile_size_v = tile_size.v as uint32;
    let pixel_size = crate::dng_tag_types::tag_type_size(pixel_type);

    let mut padded_width = tile_size_h;
    if padding_type == PaddingType::padSIMDBytes {
        padded_width = round_up_for_pixel_size(padded_width, pixel_size)?;
    }

    let mut buffer_size = padded_width;
    buffer_size = crate::dng_safe_arithmetic::safe_uint32_mult2(buffer_size, tile_size_v)?;
    buffer_size = crate::dng_safe_arithmetic::safe_uint32_mult2(buffer_size, pixel_size)?;
    buffer_size = crate::dng_safe_arithmetic::safe_uint32_mult2(buffer_size, num_planes)?;
    Ok(buffer_size)
}

// ---------------------------------------------------------------- int64/uint64

#[inline]
pub fn abs_int64(x: int64) -> uint64 {
    x.unsigned_abs()
}

#[inline]
pub fn min_int64(x: int64, y: int64) -> int64 {
    if x <= y { x } else { y }
}

#[inline]
pub fn max_int64(x: int64, y: int64) -> int64 {
    if x >= y { x } else { y }
}

#[inline]
pub fn pin_int64(min: int64, x: int64, max: int64) -> int64 {
    max_int64(min, min_int64(x, max))
}

#[inline]
pub fn min_uint64(x: uint64, y: uint64) -> uint64 {
    if x <= y { x } else { y }
}

#[inline]
pub fn max_uint64(x: uint64, y: uint64) -> uint64 {
    if x >= y { x } else { y }
}

#[inline]
pub fn pin_uint64(min: uint64, x: uint64, max: uint64) -> uint64 {
    max_uint64(min, min_uint64(x, max))
}

// ---------------------------------------------------------------- real32

#[inline]
pub fn abs_real32(x: real32) -> real32 {
    if x < 0.0 { -x } else { x }
}

#[inline]
pub fn min_real32(x: real32, y: real32) -> real32 {
    if x < y { x } else { y }
}

#[inline]
pub fn max_real32(x: real32, y: real32) -> real32 {
    if x > y { x } else { y }
}

#[inline]
pub fn pin_real32(min: real32, x: real32, max: real32) -> real32 {
    max_real32(min, min_real32(x, max))
}

#[inline]
pub fn pin_real32_01(x: real32) -> real32 {
    pin_real32(0.0, x, 1.0)
}

#[inline]
pub fn pin_real32_overrange(min: real32, x: real32, max: real32) -> real32 {
    // Normal numbers in (min,max). No change.
    if x > min && x < max {
        x
    }
    // Map large numbers (including positive infinity) to max.
    else if x > min {
        max
    }
    // Map everything else (including negative infinity and all NaNs) to min.
    else {
        min
    }
}

#[inline]
pub fn pin_overrange(x: real32) -> real32 {
    // Normal in-range numbers, except for plus and minus zero.
    if x > 0.0 && x <= 1.0 {
        x
    }
    // Large numbers, including positive infinity.
    else if x > 0.5 {
        1.0
    }
    // Plus and minus zero, negative numbers, negative infinity, and all NaNs.
    else {
        0.0
    }
}

#[inline]
pub fn lerp_real32(a: real32, b: real32, t: real32) -> real32 {
    a + t * (b - a)
}

// ---------------------------------------------------------------- real64

#[inline]
pub fn abs_real64(x: real64) -> real64 {
    if x < 0.0 { -x } else { x }
}

#[inline]
pub fn min_real64(x: real64, y: real64) -> real64 {
    if x < y { x } else { y }
}

#[inline]
pub fn max_real64(x: real64, y: real64) -> real64 {
    if x > y { x } else { y }
}

#[inline]
pub fn pin_real64(min: real64, x: real64, max: real64) -> real64 {
    max_real64(min, min_real64(x, max))
}

#[inline]
pub fn pin_real64_01(x: real64) -> real64 {
    pin_real64(0.0, x, 1.0)
}

#[inline]
pub fn pin_real64_overrange(min: real64, x: real64, max: real64) -> real64 {
    if x > min && x < max {
        x
    } else if x > min {
        max
    } else {
        min
    }
}

#[inline]
pub fn lerp_real64(a: real64, b: real64, t: real64) -> real64 {
    a + t * (b - a)
}

// ---------------------------------------------------------------- conversions

/// `Floor_uint8 (real32)` — (uint8) Max_real32 (0, x).
#[inline]
pub fn floor_uint8_f32(x: real32) -> uint8 {
    max_real32(0.0, x) as uint8
}

#[inline]
pub fn floor_uint8_f64(x: real64) -> uint8 {
    max_real64(0.0, x) as uint8
}

#[inline]
pub fn round_uint8_f32(x: real32) -> uint8 {
    floor_uint8_f32(x + 0.5)
}

#[inline]
pub fn round_uint8_f64(x: real64) -> uint8 {
    floor_uint8_f64(x + 0.5)
}

/// `Round_int32 (real32)`.
#[inline]
pub fn round_int32_f32(x: real32) -> int32 {
    (if x > 0.0 { x + 0.5 } else { x - 0.5 }) as int32
}

/// `Round_int32 (real64)` — overflow checked (C++ throws program error).
pub fn round_int32_f64(x: real64) -> int32 {
    let temp = if x > 0.0 { x + 0.5 } else { x - 0.5 };
    // NaNs fail this test and take the else branch, as in C++.
    if temp > (int32::MIN as f64) - 1.0 && temp < (int32::MAX as f64) + 1.0 {
        temp as int32
    } else {
        panic!("Overflow in Round_int32");
    }
}

/// `Floor_uint32 (real64)`.
pub fn floor_uint32_f64(x: real64) -> uint32 {
    let temp = max_real64(0.0, x);
    if temp < (uint32::MAX as f64) + 1.0 {
        temp as uint32
    } else {
        panic!("Overflow in Floor_uint32");
    }
}

#[inline]
pub fn floor_uint32_f32(x: real32) -> uint32 {
    floor_uint32_f64(x as real64)
}

#[inline]
pub fn round_uint32_f32(x: real32) -> uint32 {
    floor_uint32_f32(x + 0.5)
}

#[inline]
pub fn round_uint32_f64(x: real64) -> uint32 {
    floor_uint32_f64(x + 0.5)
}

#[inline]
pub fn round_int64_f64(x: real64) -> int64 {
    // C++: (int64) (x >= 0.0 ? x + 0.5 : x - 0.5)
    let t = if x >= 0.0 { x + 0.5 } else { x - 0.5 };
    t as int64
}

// ---------------------------------------------------------------- fixed64

pub const kFixed64_One: int64 = 1i64 << 32;
pub const kFixed64_Half: int64 = 1i64 << 31;

#[inline]
pub fn real64_to_fixed64(x: real64) -> int64 {
    round_int64_f64(x * (kFixed64_One as real64))
}

#[inline]
pub fn fixed64_to_real64(x: int64) -> real64 {
    x as real64 * (1.0 / kFixed64_One as real64)
}

// ---------------------------------------------------------------- misc

/// `ForceUppercase`.
#[inline]
pub fn force_uppercase(c: u8) -> u8 {
    if c.is_ascii_lowercase() {
        c - (b'a' - b'A')
    } else {
        c
    }
}

#[inline]
pub fn swap_bytes16(x: uint16) -> uint16 {
    x.swap_bytes()
}

#[inline]
pub fn swap_bytes32(x: uint32) -> uint32 {
    x.swap_bytes()
}

#[inline]
pub fn is_aligned16(p: usize) -> bool {
    p & 1 == 0
}

#[inline]
pub fn is_aligned32(p: usize) -> bool {
    p & 3 == 0
}

#[inline]
pub fn is_aligned64(p: usize) -> bool {
    p & 7 == 0
}

#[inline]
pub fn is_aligned128(p: usize) -> bool {
    p & 15 == 0
}

// ---------------------------------------------------------------- HSV

/// `DNG_RGBtoHSV` — RGB (0..1) to HSV (h: 0..6, s/v: 0..1).
/// INPUT REQUIREMENTS: all input RGB values are expected to be >= 0.
#[allow(clippy::too_many_arguments)]
pub fn dng_rgb_to_hsv(r: real32, g: real32, b: real32) -> (real32, real32, real32) {
    let v = max_real32(r, max_real32(g, b));
    let gap = v - min_real32(r, min_real32(g, b));

    if gap > 0.0 {
        let h = if r == v {
            let h = (g - b) / gap;
            if h < 0.0 { h + 6.0 } else { h }
        } else if g == v {
            2.0 + (b - r) / gap
        } else {
            4.0 + (r - g) / gap
        };
        let s = gap / v;
        (h, s, v)
    } else {
        (0.0, 0.0, v)
    }
}

/// `DNG_PinnedNonnegativeRGBtoHSV`.
pub fn dng_pinned_nonnegative_rgb_to_hsv(
    r: real32,
    g: real32,
    b: real32,
) -> (real32, real32, real32) {
    let r = max_real32(r, 0.0);
    let g = max_real32(g, 0.0);
    let b = max_real32(b, 0.0);
    dng_rgb_to_hsv(r, g, b)
}

/// `DNG_HSVtoRGB` — HSV (h: 0..6) to RGB (0..1).
pub fn dng_hsv_to_rgb(h: real32, s: real32, v: real32) -> (real32, real32, real32) {
    if s > 0.0 {
        if !h.is_finite() {
            panic!("Unexpected NaN or Inf");
        }

        let mut h = h % 6.0;
        if h < 0.0 {
            h += 6.0;
        }

        let i = h as int32;
        let f = h - i as real32;

        let p = v * (1.0 - s);
        let q = v * (1.0 - s * f);
        let t = v * (1.0 - s * (1.0 - f));

        // 'i' is normally {0..5}; i = 6 can occur (see C++ comment) and is
        // treated like i = 0.
        match i {
            0 | 6 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            5 => (v, p, q),
            _ => (v, t, p), // unreachable for i in 0..=6
        }
    } else {
        (v, v, v)
    }
}

// ---------------------------------------------------------------- timers

/// `TickTimeInSeconds` — high resolution monotonic timer.
pub fn tick_time_in_seconds() -> real64 {
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs_f64()
}

/// `TickCountInSeconds` — lower resolution but more stable (same impl here).
pub fn tick_count_in_seconds() -> real64 {
    tick_time_in_seconds()
}

static sTimerLevel: AtomicI32 = AtomicI32::new(0);

/// `DNGIncrementTimerLevel`.
pub fn dng_increment_timer_level() {
    if !crate::dng_globals::get_imagecore() {
        sTimerLevel.fetch_add(1, Ordering::Relaxed);
    }
}

/// `DNGDecrementTimerLevel`.
pub fn dng_decrement_timer_level() -> int32 {
    if crate::dng_globals::get_imagecore() {
        0
    } else {
        sTimerLevel.fetch_sub(1, Ordering::Relaxed) - 1
    }
}

/// Port of `dng_timer` — prints the elapsed time on Drop when
/// `gDNGShowTimers` is set.
pub struct DngTimer {
    message: String,
    start_time: real64,
}

impl DngTimer {
    pub fn new(message: &str) -> Self {
        dng_increment_timer_level();
        DngTimer {
            message: message.to_string(),
            start_time: tick_time_in_seconds(),
        }
    }
}

impl Drop for DngTimer {
    fn drop(&mut self) {
        let level = pin_int32(0, dng_decrement_timer_level(), 10) as usize;

        if !crate::dng_globals::get_dng_show_timers() {
            return;
        }

        let total_time = tick_time_in_seconds() - self.start_time;
        eprintln!(
            "{}{}: {:.3} sec",
            " ".repeat(level * 2),
            self.message,
            total_time
        );
    }
}

// ---------------------------------------------------------------- distances

/// `MaxSquaredDistancePointToRect`.
pub fn max_squared_distance_point_to_rect(point: DngPointReal64, rect: DngRectReal64) -> real64 {
    let mut dist_sqr = distance_squared(point, rect.tl());
    dist_sqr = max_real64(dist_sqr, distance_squared(point, rect.bl()));
    dist_sqr = max_real64(dist_sqr, distance_squared(point, rect.br()));
    dist_sqr = max_real64(dist_sqr, distance_squared(point, rect.tr()));
    dist_sqr
}

/// `MaxDistancePointToRect`.
pub fn max_distance_point_to_rect(point: DngPointReal64, rect: DngRectReal64) -> real64 {
    max_squared_distance_point_to_rect(point, rect).sqrt()
}

// ---------------------------------------------------------------- half/FP24

/// `DNG_HalfToFloat` — exact C++ bit-level conversion.
/// Note: NaN inputs map to 0.0 and infinities to max values, exactly as C++.
#[inline]
pub fn dng_half_to_float(half_value: uint16) -> uint32 {
    let mut sign = ((half_value >> 15) & 0x0001) as int32;
    let mut exponent = ((half_value >> 10) & 0x001f) as int32;
    let mut mantissa = (half_value & 0x03ff) as int32;

    if exponent == 0 {
        if mantissa == 0 {
            // Plus or minus zero.
            return (sign << 31) as uint32;
        } else {
            // Denormalized number -- renormalize it.
            while (mantissa & 0x00000400) == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            exponent += 1;
            mantissa &= !0x00000400;
        }
    } else if exponent == 31 {
        if mantissa == 0 {
            // Positive or negative infinity -> maximum (16 bit) values.
            return ((sign << 31) | ((0x1e + 127 - 15) << 23) | (0x3ff << 13)) as uint32;
        } else {
            // NaN -- just set to zero.
            return 0;
        }
    }

    // Normalized number.
    exponent += 127 - 15;
    mantissa <<= 13;

    // Assemble sign, exponent and mantissa.
    ((sign << 31) | (exponent << 23) | mantissa) as uint32
}

/// `DNG_FloatToHalf` — exact C++ bit-level conversion.
#[inline]
pub fn dng_float_to_half(i: uint32) -> uint16 {
    let sign = ((i >> 16) & 0x00008000) as int32;
    let mut exponent = (((i >> 23) & 0x000000ff) as int32) - (127 - 15);
    let mut mantissa = (i & 0x007fffff) as int32;

    if exponent <= 0 {
        if exponent < -10 {
            // Zero or underflow to zero.
            return sign as uint16;
        }

        // E is between -10 and 0.  Convert to a denormalized half.
        mantissa = (mantissa | 0x00800000) >> (1 - exponent);

        // Round to nearest, round "0.5" up.
        if (mantissa & 0x00001000) != 0 {
            mantissa += 0x00002000;
        }

        // Assemble the half from sign, exponent (zero) and mantissa.
        return (sign | (mantissa >> 13)) as uint16;
    } else if exponent == 0xff - (127 - 15) {
        if mantissa == 0 {
            // Infinity.
            return (sign | 0x7c00) as uint16;
        } else {
            // NaN preserving sign and top significand bits.
            return (sign | 0x7c00 | (mantissa >> 13)) as uint16;
        }
    }

    // Normalized float: round to nearest, round "0.5" up.
    if (mantissa & 0x00001000) != 0 {
        mantissa += 0x00002000;
        if (mantissa & 0x00800000) != 0 {
            mantissa = 0; // overflow in significand,
            exponent += 1; // adjust exponent
        }
    }

    // Handle exponent overflow.
    if exponent > 30 {
        return (sign | 0x7c00) as uint16; // infinity with the same sign.
    }

    // Assemble the half from sign, exponent and mantissa.
    (sign | (exponent << 10) | (mantissa >> 13)) as uint16
}

/// `DNG_FP24ToFloat` — 3-byte FP24 to float bits.
#[inline]
pub fn dng_fp24_to_float(input: &[uint8]) -> uint32 {
    let mut sign = ((input[0] >> 7) & 0x01) as int32;
    let mut exponent = (input[0] & 0x7F) as int32;
    let mut mantissa = ((input[1] as int32) << 8) | input[2] as int32;

    if exponent == 0 {
        if mantissa == 0 {
            // Plus or minus zero.
            return (sign << 31) as uint32;
        } else {
            // Denormalized number -- renormalize it.
            while (mantissa & 0x00010000) == 0 {
                mantissa <<= 1;
                exponent -= 1;
            }
            exponent += 1;
            mantissa &= !0x00010000;
        }
    } else if exponent == 127 {
        if mantissa == 0 {
            // Positive or negative infinity -> maximum (24 bit) values.
            return ((sign << 31) | ((0x7e + 128 - 64) << 23) | (0xffff << 7)) as uint32;
        } else {
            // NaN -- just set to zero.
            return 0;
        }
    }

    // Normalized number.
    exponent += 128 - 64;
    mantissa <<= 7;

    ((sign << 31) | (exponent << 23) | mantissa) as uint32
}

/// `DNG_FloatToFP24` — float bits to 3-byte FP24.
#[inline]
pub fn dng_float_to_fp24(input: uint32) -> [uint8; 3] {
    let mut exponent = (((input >> 23) & 0xFF) as int32) - 128;
    let mut mantissa = (input & 0x007FFFFF) as int32;

    if exponent == 127 {
        // infinity or NaN
        // Will the NaN alias to infinity?
        if mantissa != 0x007FFFFF && ((mantissa >> 7) == 0xFFFF) {
            mantissa &= 0x003FFFFF; // knock out msb to make it a NaN
        }
    } else if exponent > 63 {
        // overflow, map to infinity
        exponent = 63;
        mantissa = 0x007FFFFF;
    } else if exponent <= -64 {
        if exponent >= -79 {
            // encode as denorm
            mantissa = (mantissa | 0x00800000) >> (-63 - exponent);
        } else {
            // underflow to zero
            mantissa = 0;
        }
        exponent = -64;
    }

    [
        (((input >> 24) & 0x80) as uint8) | ((exponent + 64) as uint8),
        ((mantissa >> 15) & 0x00FF) as uint8,
        ((mantissa >> 7) & 0x00FF) as uint8,
    ]
}

// ---------------------------------------------------------------- muluh/mulsh

/// `MULUH` — high order 32 bits of an unsigned 32x32 multiply.
#[inline]
pub fn muluh(x: uint32, y: uint32) -> uint32 {
    (((x as uint64) * (y as uint64)) >> 32) as uint32
}

/// `MULSH` — high order 32 bits of a signed 32x32 multiply.
#[inline]
pub fn mulsh(x: int32, y: int32) -> int32 {
    (((x as int64) * (y as int64)) >> 32) as int32
}

// ---------------------------------------------------------------- random

/// `DNG_Random` — Park/Miller "minimal standard" 30 bit generator
/// (identical to Apple's), portable.  x' = (x * 16807) mod 2^31-1.
#[inline]
pub fn dng_random(mut seed: uint32) -> uint32 {
    // high = seed / 127773
    let temp = muluh(0x069C16BD, seed);
    let high = (temp.wrapping_add((seed.wrapping_sub(temp)) >> 1)) >> 16;

    // low = seed % 127773
    let low = seed.wrapping_sub(high.wrapping_mul(127773));

    // seed = (seed * 16807) % 2147483647
    seed = 16807i32
        .wrapping_mul(low as i32)
        .wrapping_sub(2836i32.wrapping_mul(high as i32)) as uint32;

    if (seed & 0x80000000) != 0 {
        seed = seed.wrapping_add(2147483647);
    }

    seed
}

// ---------------------------------------------------------------- dither

/// Port of `dng_dither` — a lazily-initialized shared noise buffer.
pub struct DngDither {
    noise_buffer: Vec<uint16>,
}

impl DngDither {
    pub const kRNGBits: uint32 = 7;
    pub const kRNGSize: uint32 = 1 << Self::kRNGBits;
    pub const kRNGMask: uint32 = Self::kRNGSize - 1;
    pub const kRNGSize2D: uint32 = Self::kRNGSize * Self::kRNGSize;

    fn new() -> Self {
        let k_seed: uint32 = 1;
        let mut buffer = vec![0u16; Self::kRNGSize2D as usize];
        let mut seed = k_seed;

        for item in buffer.iter_mut() {
            // 16-to-8-bit dither values are limited to 255..=65535 so that
            // exact 8-bit values round trip (see C++ comment).
            let mut value: uint16;
            loop {
                seed = dng_random(seed);
                value = seed as uint16;
                if value >= 255 {
                    break;
                }
            }
            *item = value;
        }

        DngDither { noise_buffer: buffer }
    }

    /// `DngDither::Get ()`.
    pub fn get() -> &'static DngDither {
        static DITHER: OnceLock<DngDither> = OnceLock::new();
        DITHER.get_or_init(DngDither::new)
    }

    /// `NoiseBuffer16 ()`.
    pub fn noise_buffer16(&self) -> &[uint16] {
        &self.noise_buffer
    }
}

// ---------------------------------------------------------------- smoothstep

/// `SmoothStep` — x assumed in [0,1]; result in [0,1].
#[inline]
pub fn smooth_step(x: real64) -> real64 {
    x * x * (3.0 - 2.0 * x)
}

/// `EncodeOverrange`.
#[inline]
pub fn encode_overrange(x: real32) -> real32 {
    let x = max_real32(x, 0.0);
    x * (256.0 + x) / (256.0 * (1.0 + x))
}

/// `DecodeOverrange`.
#[inline]
pub fn decode_overrange(x: real32) -> real32 {
    let x = max_real32(x, 0.0);
    16.0 * ((8.0 * x) - 8.0 + (64.0 * x * x - 127.0 * x + 64.0).sqrt())
}

// ---------------------------------------------------------------- line

/// Port of `dng_line_real32`.
#[derive(Debug, Clone, Copy)]
pub struct DngLineReal32 {
    pub x: real32,
    pub y: real32,
    pub slope: real32,
}

impl DngLineReal32 {
    pub fn new(x0: real32, y0: real32, x1: real32, y1: real32) -> Self {
        DngLineReal32 {
            x: x0,
            y: y0,
            slope: if x0 == x1 { 0.0 } else { (y0 - y1) / (x0 - x1) },
        }
    }

    /// `Evaluate (x)`.
    #[inline]
    pub fn evaluate(&self, x: real32) -> real32 {
        self.y + self.slope * (x - self.x)
    }
}

// ---------------------------------------------------------------- compression

/// `MinBackwardVersionForCompression`.
pub fn min_backward_version_for_compression(compression: uint32) -> uint32 {
    use crate::dng_tag_values::{ccJXL, ccLossyJPEG, dngVersion_1_1_0_0, dngVersion_1_4_0_0, dngVersion_1_7_0_0};

    if compression == ccLossyJPEG {
        return dngVersion_1_4_0_0;
    }
    if compression == ccJXL {
        return dngVersion_1_7_0_0;
    }
    dngVersion_1_1_0_0
}

// ---------------------------------------------------------------- histogram

/// `HistogramArea` — histogram of a ttShort image area (serial port of the
/// C++ loop; the tile iterator step is identical).
pub fn histogram_area(
    _host: &mut crate::dng_host::DngHost,
    image: &dyn crate::dng_image::DngImage,
    area: DngRect,
    hist: &mut [uint32],
    max_value: uint32,
    plane: uint32,
) -> DngResult<()> {
    debug_assert!(image.pixel_type() == crate::dng_tag_types::ttShort, "Unsupported pixel type");

    let need = (max_value as usize + 1) * std::mem::size_of::<uint32>();
    if hist.len() * std::mem::size_of::<uint32>() < need {
        return Err(program_error());
    }
    for v in hist.iter_mut() {
        *v = 0;
    }

    let mut tile = DngRect::new();
    let mut iter = crate::dng_tile_iterator::DngTileIterator::new_image(image, area);

    while iter.get_one_tile(&mut tile)? {
        let buffer = crate::dng_image::DngConstTileBuffer::new(image, tile)?;

        let mut row = tile.t;
        let mut col = tile.l;

        let row_step = buffer.row_step();
        let col_step = buffer.col_step();

        for _r in 0..tile.h() {
            let mut c = col;
            for _cc in 0..tile.w() {
                let x = buffer.const_pixel_u16(row, c, plane)? as uint32;
                if max_value == 0x0FFFF {
                    hist[x as usize] += 1;
                } else if x <= max_value {
                    hist[x as usize] += 1;
                }
                c += col_step;
            }
            row += row_step;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------- limit float depth

/// `LimitFloatBitDepth` (scalar path — the only path in the Rust port).
/// Copies srcImage to dstImage applying `scale` and optionally quantizing
/// through half (16-bit) or FP24 (24-bit) precision.
pub fn limit_float_bit_depth(
    _host: &mut crate::dng_host::DngHost,
    src_image: crate::dng_image::ImageRef,
    dst_image: crate::dng_image::ImageRef,
    bit_depth: uint32,
    scale: real32,
) -> DngResult<()> {
    {
        let src = src_image.read().unwrap();
        let dst = dst_image.read().unwrap();
        debug_assert!(
            src.pixel_type() == crate::dng_tag_types::ttFloat,
            "Floating point image expected"
        );
        debug_assert!(
            dst.pixel_type() == crate::dng_tag_types::ttFloat,
            "Floating point image expected"
        );
    }

    let mut task = DngLimitFloatDepthTask {
        src_image,
        dst_image,
        bit_depth,
        scale,
    };

    // Serial execution of the area task over the destination bounds
    // (see dng_area_task.rs for the threading deviation).
    let bounds = task.dst_image.read().unwrap().bounds();
    crate::dng_area_task::process_on_single_thread(&mut task, bounds, None, None)
}

/// Port of `dng_limit_float_depth_task` (private in C++).
pub struct DngLimitFloatDepthTask {
    src_image: crate::dng_image::ImageRef,
    dst_image: crate::dng_image::ImageRef,
    bit_depth: uint32,
    scale: real32,
}

impl crate::dng_area_task::DngAreaTask for DngLimitFloatDepthTask {
    fn name(&self) -> &'static str {
        "dng_limit_float_depth_task"
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
        tile: DngRect,
        _sniffer: Option<&std::sync::Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngResult<()> {
        let src = self.src_image.read().unwrap();
        let dst = self.dst_image.read().unwrap();

        let planes = dst.planes();

        // Read the tile from src (edge none; tile is inside bounds).
        let mut src_buffer = crate::dng_pixel_buffer::DngPixelBuffer::with_planar_config(
            tile,
            0,
            planes,
            src.pixel_type(),
            crate::dng_tag_values::pcInterleaved,
            crate::dng_pixel_buffer::DngPixelBuffer::allocate_data(tile, planes, src.pixel_type())?,
        )?;
        src.do_get(&mut src_buffer)?;

        // Scale and quantize into a destination buffer.
        let mut dst_buffer = crate::dng_pixel_buffer::DngPixelBuffer::with_planar_config(
            tile,
            0,
            planes,
            dst.pixel_type(),
            crate::dng_tag_values::pcInterleaved,
            crate::dng_pixel_buffer::DngPixelBuffer::allocate_data(tile, planes, dst.pixel_type())?,
        )?;

        let limit16 = self.bit_depth == 16;
        let limit24 = self.bit_depth == 24;
        let scale = self.scale;

        for row in tile.t..tile.b {
            for col in tile.l..tile.r {
                for plane in 0..planes {
                    let mut x = src_buffer.const_pixel_f32(row, col, plane)?;

                    x *= scale;

                    let mut bits = x.to_bits();

                    if limit16 {
                        let y = dng_float_to_half(bits);
                        bits = dng_half_to_float(y);
                    } else if limit24 {
                        let temp = dng_float_to_fp24(bits);
                        bits = dng_fp24_to_float(&temp);
                    }

                    dst_buffer.dirty_pixel_f32(row, col, plane, f32::from_bits(bits))?;
                }
            }
        }

        dst.do_put(&dst_buffer)?;

        Ok(())
    }
}

// ---------------------------------------------------------------- sequence info

/// Port of `dng_image_sequence_info` — implements the ImageSequenceInfo tag
/// introduced in DNG 1.7.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngImageSequenceInfo {
    pub sequence_id: crate::dng_string::DngString,
    pub sequence_type: crate::dng_string::DngString,
    pub frame_info: crate::dng_string::DngString,
    pub index: uint32,
    pub count: uint32,
    /// 2 == unknown.
    pub is_final: uint8,
}

impl DngImageSequenceInfo {
    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.sequence_id.length() >= 8 && self.sequence_type.length() >= 1
    }

    /// `TagCount ()`.
    pub fn tag_count(&self) -> DngResult<uint32> {
        let mut sum = crate::dng_safe_arithmetic::safe_uint32_add(
            self.sequence_id.length(),
            self.sequence_type.length(),
        )?;
        sum = crate::dng_safe_arithmetic::safe_uint32_add(self.frame_info.length(), sum)?;
        sum = crate::dng_safe_arithmetic::safe_uint32_add(
            sum,
            1 + 1 + 1 + 4 + 4 + 1, // null terms + index + count + final
        )?;
        Ok(sum)
    }

    /// Binary parse of the MakeTag layout (used by dng_shared::ParseTag for
    /// tcImageSequenceInfo inline data).
    pub fn parse_sequence_info_stream(
        &mut self,
        stream: &mut crate::dng_stream::DngStream,
        tag_count: uint32,
    ) -> DngResult<()> {
        // Layout: sequence ID (NUL-term) + type (NUL-term) + frame info
        // (NUL-term) + index + count + isFinal.  The exact string lengths are
        // derived from the total tag size: the C++ parser reads the same
        // layout; trailing strings shorter than 8 chars invalidate the info.
        if tag_count < 4 + 4 + 4 + 1 + 1 + 1 + 4 + 4 + 1 {
            return Err(bad_format());
        }

        // Read the three NUL-terminated strings.
        let mut read_cstr = |stream: &mut crate::dng_stream::DngStream| -> DngResult<Vec<uint8>> {
            let mut out = Vec::new();
            loop {
                let b = stream.get_uint8()?;
                if b == 0 {
                    break;
                }
                out.push(b);
            }
            Ok(out)
        };

        let id = read_cstr(stream)?;
        let ty = read_cstr(stream)?;
        let frame = read_cstr(stream)?;

        self.index = stream.get_uint32()?;
        self.count = stream.get_uint32()?;
        self.is_final = stream.get_uint8()?;

        self.sequence_id.set(&id);
        self.sequence_type.set(&ty);
        self.frame_info.set(&frame);

        Ok(())
    }

    /// `MakeTag (allocator)` — serializes to a `TagOwnedDataPtr` with
    /// `tcImageSequenceInfo` / `ttUndefined`.
    pub fn make_tag(&self) -> DngResult<crate::dng_tiff_directory::TagOwnedDataPtr> {
        use crate::dng_stream::DngStream;
        use crate::dng_tags::tcImageSequenceInfo;
        use crate::dng_tag_types::ttUndefined;

        let mut stream = crate::dng_memory_stream::DngMemoryStream::new().stream();

        // Big endian for this tag.
        let saved = stream.big_endian();
        stream.set_big_endian(true);

        if self.sequence_id.not_empty() {
            stream.put_bytes(self.sequence_id.get())?;
        }
        stream.put_zeros(1)?;
        if self.sequence_type.not_empty() {
            stream.put_bytes(self.sequence_type.get())?;
        }
        stream.put_zeros(1)?;
        if self.frame_info.not_empty() {
            stream.put_bytes(self.frame_info.get())?;
        }
        stream.put_zeros(1)?;

        stream.put_uint32(self.index)?;
        stream.put_uint32(self.count)?;
        stream.put_uint8(self.is_final)?;

        stream.set_big_endian(saved);
        stream.set_read_position(0)?;

        let block = stream.as_memory_block()?;
        Ok(crate::dng_tiff_directory::TagOwnedDataPtr::new(
            tcImageSequenceInfo as uint16,
            ttUndefined as uint16,
            block.logical_size(),
            std::sync::Arc::new(block.buffer().to_vec()),
        ))
    }
}

// ---------------------------------------------------------------- image stats

/// Port of `dng_image_stats` — implements the ImageStats tag introduced in
/// DNG 1.7.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngImageStats {
    /// Empty vectors mean invalid/missing.
    pub weighted_average: Vec<real32>,
    pub weighted_samples: Vec<WeightedSample>,
    pub weights: Vec<real32>,
    pub color_average: Vec<real32>,
    pub color_samples: Vec<ColorSample>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct WeightedSample {
    pub frac: real32,
    pub value: real32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ColorSample {
    pub frac: real32,
    pub values: Vec<real32>,
}

impl DngImageStats {
    pub const kTag_WeightedAverage: uint32 = 1;
    pub const kTag_WeightedSamples: uint32 = 2;
    pub const kTag_Weights: uint32 = 3;
    pub const kTag_ColorAverage: uint32 = 4;
    pub const kTag_ColorSamples: uint32 = 5;

    pub const kMaxSamples: usize = 1024;

    /// `IsValidForPlaneCount (planeCount)`.
    pub fn is_valid_for_plane_count(&self, plane_count: uint32) -> bool {
        debug_assert!(plane_count > 0, "Invalid plane count");
        let plane_count = plane_count as usize;

        if self.weighted_average.len() >= 2 {
            return false;
        }

        if !self.weights.is_empty() && self.weights.len() != plane_count {
            return false;
        }

        if !self.color_average.is_empty() && self.color_average.len() != plane_count {
            return false;
        }

        // Check weighted samples.
        if !self.weighted_samples.is_empty() {
            if self.weighted_samples.len() > Self::kMaxSamples {
                return false;
            }
            let mut prev = self.weighted_samples[0].frac;
            for s in &self.weighted_samples[1..] {
                let current = s.frac;
                if current <= prev {
                    return false;
                }
                prev = current;
            }
        }

        // Check color samples.
        if !self.color_samples.is_empty() {
            if self.color_samples.len() > Self::kMaxSamples {
                return false;
            }
            let mut prev = self.color_samples[0].frac;
            for sample in &self.color_samples[1..] {
                if sample.frac <= prev {
                    return false;
                }
                prev = sample.frac;
                if sample.values.len() != plane_count {
                    return false;
                }
            }
        }

        true
    }

    /// `TagCount ()`.
    pub fn tag_count(&self) -> uint32 {
        let mut count = 0;
        count += (!self.weighted_average.is_empty()) as uint32;
        count += (!self.weighted_samples.is_empty()) as uint32;
        count += (!self.weights.is_empty()) as uint32;
        count += (!self.color_average.is_empty()) as uint32;
        count += (!self.color_samples.is_empty()) as uint32;
        count
    }

    /// `MakeTag (allocator)` — serializes to a `TagOwnedDataPtr` with
    /// `tcImageStats` / `ttUndefined` (big endian).
    pub fn make_tag(&self) -> DngResult<crate::dng_tiff_directory::TagOwnedDataPtr> {
        use crate::dng_stream::DngStream;
        use crate::dng_tags::tcImageStats;
        use crate::dng_tag_types::ttUndefined;

        let mut stream = crate::dng_memory_stream::DngMemoryStream::new().stream();
        let saved = stream.big_endian();
        stream.set_big_endian(true);

        stream.put_uint32(self.tag_count())?;

        put_real32_vec(&mut stream, Self::kTag_WeightedAverage, &self.weighted_average)?;
        put_weighted_samples(&mut stream, &self.weighted_samples)?;
        put_real32_vec(&mut stream, Self::kTag_Weights, &self.weights)?;
        put_real32_vec(&mut stream, Self::kTag_ColorAverage, &self.color_average)?;
        put_color_samples(&mut stream, &self.color_samples)?;

        stream.set_big_endian(saved);
        stream.set_read_position(0)?;

        let block = stream.as_memory_block()?;
        Ok(crate::dng_tiff_directory::TagOwnedDataPtr::new(
            tcImageStats as uint16,
            ttUndefined as uint16,
            block.logical_size(),
            std::sync::Arc::new(block.buffer().to_vec()),
        ))
    }

    /// `Parse (stream)` — replaces the current contents.
    pub fn parse(&mut self, stream: &mut crate::dng_stream::DngStream) -> DngResult<()> {
        use crate::dng_sdk_limits::kMaxColorPlanes;

        // Tag data is big-endian byte order.
        let saved = stream.big_endian();
        stream.set_big_endian(true);
        let result = self.parse_inner(stream, kMaxColorPlanes);
        stream.set_big_endian(saved);
        result
    }

    fn parse_inner(
        &mut self,
        stream: &mut crate::dng_stream::DngStream,
        k_max_color_planes: uint32,
    ) -> DngResult<()> {
        let count = stream.get_uint32()?;

        // Only 5 possible tags, no tag may be repeated.
        if count > 5 {
            return Err(bad_format()); // "too many tags in dng_image_stats"
        }

        for _ in 0..count {
            let child_tag_code = stream.get_uint32()?;
            let length = stream.get_uint32()?;

            if length == 0 {
                return Err(bad_format()); // "child tag byte length must be > 0"
            }
            if (length & 3) != 0 {
                return Err(bad_format()); // "child tag byte length expected to be multiple of 4"
            }

            let k_max_bytes: uint32 =
                4 + (Self::kMaxSamples as uint32) * 4 * (k_max_color_planes + 1);
            if length > k_max_bytes {
                return Err(bad_format()); // "child tag byte length too large"
            }

            match child_tag_code {
                Self::kTag_WeightedAverage | Self::kTag_Weights | Self::kTag_ColorAverage => {
                    let num_floats = length >> 2;
                    let data = match child_tag_code {
                        Self::kTag_WeightedAverage => &mut self.weighted_average,
                        Self::kTag_Weights => &mut self.weights,
                        _ => &mut self.color_average,
                    };
                    data.clear();
                    for _ in 0..num_floats {
                        data.push(stream.get_real32()?);
                    }
                }

                Self::kTag_WeightedSamples => {
                    let sample_count = stream.get_uint32()?;
                    if sample_count == 0 {
                        return Err(bad_format()); // "too few samples for weighted samples"
                    }
                    if sample_count as usize > Self::kMaxSamples {
                        return Err(bad_format()); // "too many samples for weighted samples"
                    }
                    if 4 + 8 * sample_count != length {
                        return Err(bad_format()); // "mismatch byte length for weighted samples"
                    }
                    self.weighted_samples.clear();
                    for _ in 0..sample_count {
                        let frac = stream.get_real32()?;
                        let value = stream.get_real32()?;
                        self.weighted_samples.push(WeightedSample { frac, value });
                    }
                }

                Self::kTag_ColorSamples => {
                    let sample_count = stream.get_uint32()?;
                    if sample_count == 0 {
                        return Err(bad_format()); // "too few samples for color samples"
                    }
                    if sample_count as usize > Self::kMaxSamples {
                        return Err(bad_format()); // "too many samples for color samples"
                    }
                    // Infer plane count.
                    let planes = ((length - 4) / sample_count / 4).wrapping_sub(1);
                    if planes == 0 {
                        return Err(bad_format()); // "unexpected 0 plane count for color samples"
                    }
                    if planes > k_max_color_planes {
                        return Err(bad_format()); // "too large plane count for color samples"
                    }
                    if 4 + sample_count * 4 * (planes + 1) != length {
                        return Err(bad_format()); // "mismatched plane count for color samples"
                    }
                    self.color_samples.clear();
                    for _ in 0..sample_count {
                        let frac = stream.get_real32()?;
                        let mut values = Vec::with_capacity(planes as usize);
                        for _ in 0..planes {
                            values.push(stream.get_real32()?);
                        }
                        self.color_samples.push(ColorSample { frac, values });
                    }
                }

                _ => {
                    return Err(bad_format()); // "unsupported child tag code"
                }
            }
        }

        Ok(())
    }
}

fn put_real32_vec(
    stream: &mut crate::dng_stream::DngStream,
    tag_code: uint32,
    values: &[real32],
) -> DngResult<()> {
    if !values.is_empty() {
        debug_assert!(values.len() <= 16, "values vector too large");
        stream.put_uint32(tag_code)?;
        stream.put_uint32(4 * values.len() as uint32)?;
        for x in values {
            stream.put_real32(*x)?;
        }
    }
    Ok(())
}

fn put_weighted_samples(
    stream: &mut crate::dng_stream::DngStream,
    samples: &[WeightedSample],
) -> DngResult<()> {
    if !samples.is_empty() {
        debug_assert!(
            samples.len() <= DngImageStats::kMaxSamples,
            "samples vector too large"
        );
        stream.put_uint32(DngImageStats::kTag_WeightedSamples)?;
        stream.put_uint32((4 + 8 * samples.len()) as uint32)?;
        stream.put_uint32(samples.len() as uint32)?;
        for sample in samples {
            stream.put_real32(sample.frac)?;
            stream.put_real32(sample.value)?;
        }
    }
    Ok(())
}

fn put_color_samples(
    stream: &mut crate::dng_stream::DngStream,
    samples: &[ColorSample],
) -> DngResult<()> {
    if !samples.is_empty() {
        debug_assert!(
            samples.len() <= DngImageStats::kMaxSamples,
            "samples vector too large"
        );
        stream.put_uint32(DngImageStats::kTag_ColorSamples)?;

        let mut bytes: uint32 = 4;
        for sample in samples {
            bytes = crate::dng_safe_arithmetic::safe_uint32_add(
                bytes,
                (4 + 4 * sample.values.len()) as uint32,
            )?;
        }
        stream.put_uint32(bytes)?;

        stream.put_uint32(samples.len() as uint32)?;
        for sample in samples {
            stream.put_real32(sample.frac)?;
            for x in &sample.values {
                stream.put_real32(*x)?;
            }
        }
    }
    Ok(())
}

/// C++ `DoZeroBytes` — zero a byte range (kept for port fidelity of callers).
pub fn do_zero_bytes(dest: &mut [uint8]) {
    dest.fill(0);
}

/// C++ helper used by bottleneck-style loops; returns an error so call sites
/// can propagate (maps `ThrowBadFormat`-style checks).
pub fn require(condition: bool, message: &str) -> DngResult<()> {
    if !condition {
        return Err(DngError::Unsupported(message.to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_tag_types::{ttByte, ttFloat, ttShort, ttUndefined};

    #[test]
    fn pin_helpers() {
        assert_eq!(pin_int32(-5, 10, 5), 5);
        assert_eq!(pin_int32_between(5, 100, -5), 5);
        assert_eq!(pin_uint16(-3), 0);
        assert_eq!(pin_uint16(70000), 65535);
        assert_eq!(pin_uint16(1000), 1000);
        assert_eq!(pin_real32_01(1.5), 1.0);
        assert_eq!(pin_real32_01(-0.1), 0.0);
    }

    #[test]
    fn rounding_helpers() {
        assert_eq!(round_up_2(5), 6);
        assert_eq!(round_up_2(4), 4);
        assert_eq!(round_up_16(17), 32);
        assert_eq!(round_up_4096(1), 4096);
        assert_eq!(round_down_8(15), 8);
        assert_eq!(round_int32_f64(2.5), 3);
        assert_eq!(round_int32_f64(-2.5), -3);
        assert_eq!(round_int32_f32(-0.4), 0);
        assert_eq!(floor_uint32_f64(3.99), 3);
        assert_eq!(round_uint32_f64(3.5), 4);
        assert_eq!(round_up_for_pixel_size(10, 2).unwrap(), 16);
        assert_eq!(round_up_for_pixel_size(10, 1).unwrap(), 16);
        assert_eq!(round_up_for_pixel_size(10, 4).unwrap(), 12);
    }

    #[test]
    fn compute_buffer_size_cases() {
        let tile = crate::dng_point::DngPoint::new(4, 5);
        // ttByte (size 1), no padding: 4*5*1*3 = 60
        assert_eq!(
            compute_buffer_size(ttByte, tile, 3, PaddingType::padNone).unwrap(),
            60
        );
        // ttShort (size 2): 4*5*2*1 = 40
        assert_eq!(
            compute_buffer_size(ttShort, tile, 1, PaddingType::padNone).unwrap(),
            40
        );
        // ttFloat with SIMD padding: width 5 -> 8, 8*4*4*2 = 256
        assert_eq!(
            compute_buffer_size(ttFloat, tile, 2, PaddingType::padSIMDBytes).unwrap(),
            256
        );
        assert!(compute_buffer_size(ttShort, crate::dng_point::DngPoint::new(-1, 5), 1, PaddingType::padNone).is_err());
    }

    #[test]
    fn half_conversions() {
        // Round trip normal values.
        for bits in [0x0000u32, 0x3F800000, 0xBF800000, 0x42480000, 0x00800000] {
            let f = f32::from_bits(bits);
            let h = dng_float_to_half(bits);
            let back = dng_half_to_float(h);
            if f.is_nan() {
                assert_eq!(back, 0);
            } else if f.is_infinite() {
                // Infinity maps to max finite value per C++.
                assert_eq!(f32::from_bits(back), f32::from_bits(0x7F7FFFFF).max(f32::from_bits(back)));
            } else {
                let round = f32::from_bits(back);
                assert!((round - f).abs() <= f.abs() * 1e-3 + 1e-7, "{f} -> {round}");
            }
        }
        // Known values.
        assert_eq!(dng_float_to_half(0x3F800000), 0x3C00); // 1.0
        assert_eq!(dng_half_to_float(0x3C00), 0x3F800000); // 1.0
        assert_eq!(dng_half_to_float(0x0000), 0); // +0
        assert_eq!(dng_half_to_float(0x8000), 0x80000000); // -0
        assert_eq!(dng_half_to_float(0x7C00), 0x7F800000 | 0x3FF << 13); // +inf -> max
        assert_eq!(dng_half_to_float(0x7E00), 0); // NaN -> 0
    }

    #[test]
    fn fp24_conversions() {
        let bits = 0x3F800000u32; // 1.0
        let fp24 = dng_float_to_fp24(bits);
        let back = dng_fp24_to_float(&fp24);
        assert_eq!(f32::from_bits(back), 1.0);

        let bits = 0x00000000u32;
        assert_eq!(dng_fp24_to_float(&dng_float_to_fp24(bits)), 0);
    }

    #[test]
    fn random_sequence() {
        // Park-Miller: seed 1 -> 16807 -> 282475249 -> ...
        let s1 = dng_random(1);
        assert_eq!(s1, 16807);
        let s2 = dng_random(s1);
        assert_eq!(s2, 282475249);
        assert!(s2 & 0x80000000 == 0);
    }

    #[test]
    fn dither_table() {
        let d = DngDither::get();
        assert_eq!(d.noise_buffer16().len(), DngDither::kRNGSize2D as usize);
        // All values are in 255..=65535 by construction.
        assert!(d.noise_buffer16().iter().all(|&v| v >= 255));
    }

    #[test]
    fn hsv_roundtrip() {
        let (h, s, v) = dng_rgb_to_hsv(1.0, 0.5, 0.25);
        let (r, g, b) = dng_hsv_to_rgb(h, s, v);
        assert!((r - 1.0).abs() < 1e-6);
        assert!((g - 0.5).abs() < 1e-6);
        assert!((b - 0.25).abs() < 1e-6);
    }

    #[test]
    fn overrange() {
        for x in [0.0f32, 0.25, 0.5, 1.0, 2.0] {
            let e = encode_overrange(x);
            let d = decode_overrange(e);
            assert!((d - x).abs() < 1e-4, "x={x} e={e} d={d}");
        }
    }

    #[test]
    fn image_stats_roundtrip() {
        let mut stats = DngImageStats::default();
        stats.weights = vec![0.5, 0.25, 0.25];
        stats.weighted_average = vec![0.75];
        stats.weighted_samples = vec![
            WeightedSample { frac: 0.25, value: 1.0 },
            WeightedSample { frac: 0.5, value: 2.0 },
        ];
        stats.color_samples = vec![ColorSample {
            frac: 0.3,
            values: vec![1.0, 2.0, 3.0],
        }];

        assert!(stats.is_valid_for_plane_count(3));

        let mut stream = crate::dng_memory_stream::DngMemoryStream::new().stream();
        {
            let tag = stats.make_tag().unwrap();
            stream.put_bytes(&tag.data()).unwrap();
            stream.set_read_position(0).unwrap();
        }

        let mut parsed = DngImageStats::default();
        parsed.parse(&mut stream).unwrap();
        assert_eq!(parsed, stats);
    }

    #[test]
    fn sequence_info_roundtrip() {
        let mut info = DngImageSequenceInfo::default();
        info.sequence_id.set_utf8("0123456789ABCDEF".as_bytes());
        info.sequence_type.set_utf8("B".as_bytes());
        info.frame_info.set_utf8("frame1".as_bytes());
        info.index = 3;
        info.count = 9;
        info.is_final = 1;

        assert!(info.is_valid());

        let tag = info.make_tag().unwrap();
        let bytes = tag.data();

        // tag layout: seq_id + NUL + type + NUL + frame + NUL + 4 + 4 + 1
        let expected_len = 16 + 1 + 1 + 1 + 6 + 1 + 4 + 4 + 1;
        assert_eq!(bytes.len() as uint32, expected_len);
        assert_eq!(info.tag_count().unwrap(), expected_len);

        // Check big-endian encoding of index at the right offset.
        let idx_off = 16 + 1 + 1 + 1 + 6 + 1;
        assert_eq!(&bytes[idx_off..idx_off + 4], &[0, 0, 0, 3]);
    }
}

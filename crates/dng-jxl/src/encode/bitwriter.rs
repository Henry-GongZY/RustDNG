//! LSB-first bit writer matching `jxl-bitstream`'s `Bitstream` reader
//! (little-endian bit packing: bit 0 of byte 0 is the first bit of the
//! stream).

use crate::JxlError;

pub struct BitWriter {
    buf: Vec<u8>,
    /// Bits used in the last byte (0..8).
    bit_pos: usize,
    /// Partial byte being assembled (bits 0..bit_pos valid).
    current: u8,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BitWriter {
    pub fn new() -> Self {
        BitWriter {
            buf: Vec::new(),
            bit_pos: 0,
            current: 0,
        }
    }

    /// Write `n` bits of `value` (value's low `n` bits, LSB first).
    pub fn write_bits(&mut self, value: u64, n: usize) {
        debug_assert!(n <= 64);
        let mut remaining = n;
        let mut v = value;
        while remaining > 0 {
            let space = 8 - self.bit_pos;
            let take = space.min(remaining);
            self.current |= ((v & ((1u64 << take) - 1)) as u8) << self.bit_pos;
            self.bit_pos += take;
            v >>= take;
            remaining -= take;
            if self.bit_pos == 8 {
                self.buf.push(self.current);
                self.current = 0;
                self.bit_pos = 0;
            }
        }
    }

    /// Write a U32-dispatched value: 2-bit selector then payload.
    pub fn write_u32(&mut self, selector: u8, offset: u32, payload_bits: usize, value: u32) {
        self.write_bits(selector as u64, 2);
        debug_assert!(value >= offset);
        self.write_bits((value - offset) as u64, payload_bits);
    }

    /// Zero-pad to a byte boundary.
    pub fn zero_pad_to_byte(&mut self) {
        if self.bit_pos > 0 {
            self.buf.push(self.current);
            self.current = 0;
            self.bit_pos = 0;
        }
    }

    /// Number of whole bytes written so far (pads first).
    pub fn byte_len(&self) -> usize {
        self.buf.len() + usize::from(self.bit_pos > 0)
    }

    /// Current bit position.
    pub fn bit_len(&self) -> usize {
        self.buf.len() * 8 + self.bit_pos
    }

    /// Finish and return the bytes.
    pub fn finish(mut self) -> Vec<u8> {
        self.zero_pad_to_byte();
        self.buf
    }
}

/// Zigzag encode an i32 to u32 (matches `jxl_bitstream::unpack_signed`
/// inverse).
pub fn zigzag(v: i32) -> u32 {
    ((v << 1) ^ (v >> 31)) as u32
}

/// Floor(log2(x)) for x >= 1.
pub fn floor_log2(x: u32) -> u32 {
    debug_assert!(x >= 1);
    31 - x.leading_zeros()
}

/// `add_log2_ceil` from jxl-coding: log2 floor + 1 (for x >= 1), 0 for x == 0.
pub fn add_log2_ceil(x: u32) -> u32 {
    if x == 0 {
        0
    } else {
        floor_log2(x) + 1
    }
}

/// Write a U32 value in the jxl `U32(a, b, c, d)` selector encoding with the
/// given (offset, bits) tuples.
pub type U32Spec = (u32, usize);

pub fn write_u32_spec(w: &mut BitWriter, specs: [U32Spec; 4], value: u32) -> Result<(), JxlError> {
    for (sel, &(offset, bits)) in specs.iter().enumerate() {
        let max = offset + ((1u32 << bits) - 1);
        if value >= offset && value <= max {
            w.write_u32(sel as u8, offset, bits, value);
            return Ok(());
        }
    }
    Err(JxlError::Encode(format!("value {value} out of range for U32 spec")))
}

//! Entropy coding: prefix-code (Brotli-style) histogram writer and
//! hybrid-uint token encoding, matching `jxl-coding`'s decoder.

use super::bitwriter::{add_log2_ceil, BitWriter};

/// Integer config mirroring `jxl_coding::IntegerConfig`:
/// `split = 1 << split_exponent`, msb/lsb bits carried in the token.
#[derive(Debug, Clone, Copy)]
pub struct IntegerConfig {
    pub split_exponent: u32,
    pub msb_in_token: u32,
    pub lsb_in_token: u32,
}

impl IntegerConfig {
    /// Simple config: no msb/lsb in token. Values below `1 << split_exponent`
    /// are tokens directly; larger values use `n = split_exponent +
    /// (token - split)` extra bits, value = `(1 << n) | rest`.
    pub const fn simple(split_exponent: u32) -> Self {
        IntegerConfig {
            split_exponent,
            msb_in_token: 0,
            lsb_in_token: 0,
        }
    }

    /// Alphabet size needed to encode values up to `max_value` (inclusive).
    pub fn alphabet_size(&self, max_value: u32) -> u32 {
        let split = 1u32 << self.split_exponent;
        if max_value < split {
            return split.max(1);
        }
        let max_n = 31 - max_value.leading_zeros(); // floor_log2
        split + (max_n - self.split_exponent) + 1
    }
}

/// Encode `value` with the given config: returns the token plus extra bits.
pub struct TokenEncoding {
    pub token: u32,
    /// Extra bits to write (LSB first) after the token symbol.
    pub rest: u64,
    pub rest_bits: usize,
}

pub fn encode_uint(config: &IntegerConfig, value: u32) -> TokenEncoding {
    let split = 1u32 << config.split_exponent;
    if value < split {
        return TokenEncoding {
            token: value,
            rest: 0,
            rest_bits: 0,
        };
    }
    let n = 31 - value.leading_zeros(); // floor_log2(value)
    let rest = (value ^ (1 << n)) as u64; // value - (1 << n)
    let token = split + n - config.split_exponent;
    TokenEncoding {
        token,
        rest,
        rest_bits: n as usize,
    }
}

/// Write the prefix-code histogram for one cluster with a uniform code over
/// `alphabet_size` symbols (alphabet_size must be a power of two <= 64 for
/// this simplified writer; code length = log2(alphabet_size) <= 6, which is
/// always representable in the Brotli code-length alphabet).
///
/// Format (see `jxl_coding::prefix::Histogram::parse`):
/// - hskip = 0 (2 bits)
/// - code-length-code lengths for the 18 code-length symbols, written in
///   CODE_LENGTH_ORDER with the fixed 0/4/3/8 scheme; exactly one symbol
///   (the uniform code length `len`) gets base 3 ("length 2"), all others 0.
///   A single nonzero code-length symbol yields a single-symbol code-length
///   histogram, so the following `alphabet_size` code lengths are read with
///   zero bits each and all equal `len`.
pub fn write_uniform_prefix_histogram(w: &mut BitWriter, alphabet_size: u32) {
    debug_assert!(alphabet_size.is_power_of_two());
    debug_assert!(alphabet_size >= 2 && alphabet_size <= 64);
    let len = alphabet_size.trailing_zeros(); // log2
    debug_assert!(len >= 1 && len <= 6);

    // hskip = 0.
    w.write_bits(0, 2);

    // CODE_LENGTH_ORDER = [1,2,3,4,0,5,17,6,16,7,8,9,10,11,12,13,14,15].
    const CODE_LENGTH_ORDER: [u32; 18] = [
        1, 2, 3, 4, 0, 5, 17, 6, 16, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    ];
    for &sym in CODE_LENGTH_ORDER.iter() {
        if sym == len {
            // base = 3 -> code length 2 for this symbol.
            w.write_bits(2, 2); // read_u32(0,4,3,8): selector 2 -> 3
        } else {
            // base = 0 -> code length 0.
            w.write_bits(0, 2);
        }
    }
    // The code-length histogram is now a single-symbol code (the decoder
    // takes the nonzero_count == 1 path), so each of the alphabet_size
    // code lengths reads as `len` with zero bits consumed. No further bits
    // are written here.
    //
    // With every symbol at code length `len` the Kraft sum is exactly full:
    // alphabet_size * 2^-len = 1, so the decoder's length-reading loop also
    // stops after alphabet_size symbols. Nothing more to write.
}

/// Write a full "decoder config" (no LZ77, prefix code, given clusters =>
/// single cluster) for `num_dist` distributions sharing one uniform
/// histogram of `alphabet_size` symbols with the given integer config.
pub fn write_entropy_config(
    w: &mut BitWriter,
    num_dist: u32,
    alphabet_size: u32,
    config: &IntegerConfig,
) {
    // Lz77::parse: disabled.
    w.write_bits(0, 1);

    // read_clusters(num_dist):
    if num_dist == 1 {
        // No bits at all.
    } else {
        // simple dist branch: 1 bool, nbits = 0 (2 bits), num_dist × 0 bits.
        w.write_bits(1, 1); // simple
        w.write_bits(0, 2); // nbits = 0 -> every cluster reads 0 bits (value 0)
        // num_dist clusters, each 0 bits (all cluster 0). No writes.
    }

    // use_prefix_code = true.
    w.write_bits(1, 1);
    // (log_alphabet_size for prefix codes is fixed at 15 — no bits.)

    // IntegerConfig per cluster (num_clusters == 1).
    let split_exponent = config.split_exponent;
    debug_assert!(split_exponent <= 15);
    w.write_bits(split_exponent as u64, add_log2_ceil(15) as usize); // 4 bits
    if split_exponent != 15 {
        let msb_bits = add_log2_ceil(split_exponent) as usize;
        w.write_bits(config.msb_in_token as u64, msb_bits);
        let lsb_bits = add_log2_ceil(split_exponent - config.msb_in_token) as usize;
        w.write_bits(config.lsb_in_token as u64, lsb_bits);
    }

    // counts[0]: the alphabet size of the histogram.
    if alphabet_size == 1 {
        w.write_bits(0, 1); // count = 1 -> single-symbol histogram
    } else {
        w.write_bits(1, 1);
        // count = 1 + (1 << n) + r, r < (1 << n).
        let reduced = alphabet_size - 1;
        let n = 31 - reduced.leading_zeros(); // floor_log2(reduced)
        debug_assert!(n <= 15);
        let r = reduced - (1 << n);
        w.write_bits(n as u64, 4);
        w.write_bits(r as u64, n as usize);
    }

    // Histogram::parse(count): uniform-code writer above.
    write_uniform_prefix_histogram(w, alphabet_size);
}

/// Write a token (uniform prefix code) and its extra bits.
///
/// Brotli prefix codes are bit-reversed: the decoder's lookup table stores
/// the symbol for code value `reverse_bits(s)` at index `reverse_bits(s)`,
/// so writing symbol `s` means emitting `reverse_bits(s, len)` as `len`
/// LSB-first bits.
pub fn write_token(w: &mut BitWriter, alphabet_size: u32, enc: &TokenEncoding) {
    debug_assert!(enc.token < alphabet_size);
    let len = alphabet_size.trailing_zeros() as usize;
    let code = reverse_bits(enc.token, len as u32);
    w.write_bits(code as u64, len);
    if enc.rest_bits > 0 {
        w.write_bits(enc.rest, enc.rest_bits);
    }
}

/// Reverse the low `n` bits of `v`.
pub fn reverse_bits(v: u32, n: u32) -> u32 {
    let mut out = 0u32;
    for i in 0..n {
        out |= ((v >> i) & 1) << (n - 1 - i);
    }
    out
}

//! Modular sub-bitstream writer: MA tree, entropy config, and residual
//! streams with the clamped-gradient predictor (mirrors
//! `jxl-modular`'s `decode_simple_grad` exactly).

use super::bitwriter::{zigzag, BitWriter};
use super::entropy::{encode_uint, write_entropy_config, write_token, IntegerConfig};

/// Predictor index in the `jxl-modular::Predictor` enum: Gradient = 5.
const PREDICTOR_GRADIENT: u32 = 5;

/// Residual integer config: split_exponent 4 (split = 16).
///
/// Values 0..15 are tokens directly; larger values use
/// `n = floor_log2(v)` extra bits with token `16 + n - 4`.
pub const RESIDUAL_CONFIG: IntegerConfig = IntegerConfig::simple(4);

/// Token alphabet used by the MA tree stream: tokens written are 0 (leaf
/// marker), 5 (Gradient predictor), 0, 0, 0 — max 5, so 8 symbols suffice.
const TREE_ALPHABET: u32 = 8;

/// Clamped-gradient prediction: `(n + w - nw).clamp(min(n, w), max(n, w))`
/// — matches `Sample::grad_clamped` for i32.
fn grad_clamped(n: i32, w: i32, nw: i32) -> i32 {
    let grad = n.wrapping_add(w).wrapping_sub(nw);
    grad.clamp(n.min(w), n.max(w))
}

/// Prediction for sample (y, x) of a single row-major channel.
fn predict(data: &[i32], width: usize, y: usize, x: usize) -> i32 {
    if y == 0 {
        // First row: first pixel predicts 0, rest predict the left pixel.
        if x == 0 {
            0
        } else {
            data[x - 1]
        }
    } else if x == 0 {
        // First column: predict the pixel above.
        data[(y - 1) * width]
    } else {
        let n = data[(y - 1) * width + x];
        let nw = data[(y - 1) * width + x - 1];
        let wv = data[y * width + x - 1];
        grad_clamped(n, wv, nw)
    }
}

/// Scan all channels for the largest zigzag-encoded residual.
pub fn scan_max_residual(channels: &[&[i32]], dims: &[(u32, u32)]) -> u32 {
    let mut max_u = 1u32;
    for (data, &(cw, ch)) in channels.iter().zip(dims.iter()) {
        let width = cw as usize;
        for y in 0..ch as usize {
            for x in 0..width {
                let pred = predict(data, width, y, x);
                let residual = data[y * width + x].wrapping_sub(pred);
                max_u = max_u.max(zigzag(residual));
            }
        }
    }
    max_u
}

/// Pick the uniform-token alphabet size for the residual stream: power of
/// two in [8, 64] covering the largest token.
fn residual_alphabet(max_residual: u32) -> u32 {
    let needed = RESIDUAL_CONFIG.alphabet_size(max_residual);
    needed.next_power_of_two().max(8).min(64)
}

/// Write the MA config portion of GlobalModular:
/// `use_global_ma = 1`, tree entropy config, tree tokens, residual entropy
/// config. Returns the residual alphabet size (caller reuses it for every
/// pass-group stream).
pub fn write_ma_config(w: &mut BitWriter, max_residual: u32) -> u32 {
    // GlobalModular: ma_config present.
    w.write_bits(1, 1);

    // MaConfig::parse part 1: tree decoder (6 contexts), all tokens land in
    // cluster 0 (simple dist with nbits=0).
    write_entropy_config(w, 6, TREE_ALPHABET, &IntegerConfig::simple(1));

    // Tree tokens: single leaf (property 0), Gradient predictor, offset 0,
    // mul_log 0, mul_bits 0.
    let cfg = IntegerConfig::simple(1);
    write_token(w, TREE_ALPHABET, &encode_uint(&cfg, 0)); // leaf marker (ctx 1)
    write_token(w, TREE_ALPHABET, &encode_uint(&cfg, PREDICTOR_GRADIENT)); // ctx 2
    write_token(w, TREE_ALPHABET, &encode_uint(&cfg, 0)); // offset (ctx 3)
    write_token(w, TREE_ALPHABET, &encode_uint(&cfg, 0)); // mul_log (ctx 4)
    write_token(w, TREE_ALPHABET, &encode_uint(&cfg, 0)); // mul_bits (ctx 5)

    // MaConfig::parse part 2: residual decoder (1 context).
    let alphabet = residual_alphabet(max_residual);
    write_entropy_config(w, 1, alphabet, &RESIDUAL_CONFIG);
    alphabet
}

/// Write the ModularHeader with `use_global_tree = true`, default WP
/// params, no transforms.
pub fn write_modular_header(w: &mut BitWriter) {
    // use_global_tree = true (the MA config written at GlobalModular level).
    w.write_bits(1, 1);
    // wp_params: default_wp = true.
    w.write_bits(1, 1);
    // nb_transforms = U32(0, 1, 2 + u(4), 18 + u(8)) -> selector 0.
    w.write_bits(0, 2);
    // transforms: none.
}

/// Write residual tokens for the given channels (row-major, sequential),
/// using the shared residual alphabet.
pub fn write_residuals(w: &mut BitWriter, alphabet: u32, channels: &[&[i32]], dims: &[(u32, u32)]) {
    for (data, &(cw, ch)) in channels.iter().zip(dims.iter()) {
        let width = cw as usize;
        for y in 0..ch as usize {
            for x in 0..width {
                let pred = predict(data, width, y, x);
                let residual = data[y * width + x].wrapping_sub(pred);
                let enc = encode_uint(&RESIDUAL_CONFIG, zigzag(residual));
                write_token(w, alphabet, &enc);
            }
        }
    }
}

/// Convenience: scan + write a complete modular sub-stream with a fresh
/// entropy config (used for the single-group "everything global" case).
pub fn write_full_stream(w: &mut BitWriter, channels: &[&[i32]], dims: &[(u32, u32)]) {
    let max_residual = scan_max_residual(channels, dims);
    let alphabet = write_ma_config(w, max_residual);
    write_modular_header(w);
    write_residuals(w, alphabet, channels, dims);
}

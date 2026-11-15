//! Port of `dng_lossless_jpeg.h` / `dng_lossless_jpeg.cpp` — public entry
//! points for encoding and decoding lossless JPEG (TIFF compression code
//! 34892, `ccLossyJPEG` in `dng_tag_values.h`).
//!
//! The C++ header declares `dng_spooler` (output sink) and the templated
//! `DecodeLosslessJPEG` / `EncodeLosslessJPEG` entry points; `dng_lossless_jpeg.cpp`
//! instantiates the `Scalar` SIMD specialization. This module provides the
//! trait + free functions; the implementation lives in
//! [`crate::dng_lossless_jpeg_shared`].

use dng_core::dng_types::{int32, uint32, uint16, uint64};
use dng_core::dng_stream::DngStream;
use dng_core::DngResult;

use crate::dng_lossless_jpeg_shared::{LosslessDecoder, LosslessEncoder};

/// Port of `dng_spooler` — output sink for decoded 16-bit samples.
///
/// The C++ `Spool (const void *data, uint32 count)` receives raw bytes of
/// `uint16` samples; the Rust port receives the sample slice directly.
pub trait DngSpooler {
    /// `Spool (data, count)` — `count` is implied by the slice length.
    fn spool(&mut self, data: &[uint16]);
}

/// Port of `DecodeLosslessJPEG` — decode a lossless JPEG stream, spooling
/// decoded rows of 16-bit samples.
///
/// C++ returns void; the Rust port returns the parsed
/// `(imageWidth, imageHeight, imageChannels)` for caller convenience
/// (PORT-NOTE — additive; the C++ reads these via `StartRead` out-params
/// inside the template).
///
/// * `minDecodedSize` / `maxDecodedSize` — accepted decoded byte size range
///   (`width * height * channels * 2`), checked against the SOF header.
/// * `bug16` — decode with the "16-bit bug" (`fLosslessJPEGBug16` IFD flag).
/// * `endOfData` — stream position the codestream must not run past.
pub fn decode_lossless_jpeg<'s, 'a, S: DngSpooler + ?Sized>(
    stream: &'s mut DngStream<'a>,
    spooler: &'s mut S,
    min_decoded_size: uint32,
    max_decoded_size: uint32,
    bug16: bool,
    end_of_data: uint64,
) -> DngResult<(uint32, uint32, uint32)> {
    let mut decoder = LosslessDecoder::new(stream, spooler, bug16);

    let (image_width, image_height, image_channels) = decoder.start_read()?;

    let decoded_size = image_width
        * image_height
        * image_channels
        * (std::mem::size_of::<uint16>() as uint32);

    if decoded_size < min_decoded_size || decoded_size > max_decoded_size {
        return Err(dng_core::dng_exceptions::bad_format());
    }

    let stream_pos = decoder.stream_position();

    let hasselblad = crate::dng_lossless_jpeg_shared::qSupportHasselblad_3FR
        && decoder.is_hasselblad_3fr();

    decoder.finish_read()?;

    if stream_pos > end_of_data {
        let mut throw_bad_format = true;

        // Per Hasselblad's request:
        // If we have a Hassy file with exactly four extra bytes,
        // let it through; the file is likely still valid.

        if hasselblad && stream_pos - end_of_data == 4 {
            throw_bad_format = false;
        }

        if throw_bad_format {
            return Err(dng_core::dng_exceptions::bad_format());
        }
    }

    Ok((image_width, image_height, image_channels))
}

/// Port of `EncodeLosslessJPEG` — encode 16-bit samples as lossless JPEG.
///
/// `src_data` is addressed like the C++ pointer walk:
/// `src_data[(row * src_row_step + col * src_col_step + channel) as usize]`.
pub fn encode_lossless_jpeg<'s, 'a>(
    src_data: &[uint16],
    src_rows: uint32,
    src_cols: uint32,
    src_channels: uint32,
    src_bit_depth: uint32,
    src_row_step: int32,
    src_col_step: int32,
    stream: &'s mut DngStream<'a>,
) -> DngResult<()> {
    let mut encoder = LosslessEncoder::new(
        src_data,
        src_rows,
        src_cols,
        src_channels,
        src_bit_depth,
        src_row_step,
        src_col_step,
        stream,
    );

    encoder.encode()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_tags::{M_APP0, M_DHT, M_EOI, M_RST0, M_SOF3, M_SOI, M_SOS};
    use dng_core::dng_types::{int32, uint8};

    /// Collects spooled samples (test sink).
    struct VecSpooler {
        data: Vec<uint16>,
    }

    impl VecSpooler {
        fn new() -> Self {
            VecSpooler { data: Vec::new() }
        }

        fn into_inner(self) -> Vec<uint16> {
            self.data
        }
    }

    impl DngSpooler for VecSpooler {
        fn spool(&mut self, data: &[uint16]) {
            self.data.extend_from_slice(data);
        }
    }

    /// MSB-first JPEG entropy bit writer with 0xFF stuffing (matches the
    /// decoder's `FillBitBuffer` conventions).
    struct JpegBitWriter {
        bytes: Vec<uint8>,
        current: uint8,
        bits: u32,
    }

    impl JpegBitWriter {
        fn new() -> Self {
            JpegBitWriter {
                bytes: Vec::new(),
                current: 0,
                bits: 0,
            }
        }

        fn write_bits(&mut self, value: u32, n: u32) {
            for i in (0..n).rev() {
                let bit = (value >> i) & 1;
                self.current |= (bit as uint8) << (7 - self.bits);
                self.bits += 1;
                if self.bits == 8 {
                    self.push_byte(self.current);
                    self.current = 0;
                    self.bits = 0;
                }
            }
        }

        fn push_byte(&mut self, b: uint8) {
            self.bytes.push(b);
            if b == 0xFF {
                self.bytes.push(0x00);
            }
        }

        /// Write the canonical code for the symbol with index `idx` in a
        /// table of `count` one-symbol-per-length entries: symbol 0 → '0',
        /// symbol 1 → '10', symbol 2 → '110', … (code = 2^(idx+1) - 2,
        /// length idx+1).
        fn write_symbol(&mut self, idx: u32, count: u32) {
            let _ = count;
            let code = (1u32 << (idx + 1)) - 2;
            self.write_bits(code, idx + 1);
        }

        fn finish(mut self) -> Vec<uint8> {
            if self.bits > 0 {
                // Pad with 1-bits (matching the C++ encoder's FlushBits).
                while self.bits < 8 {
                    self.current |= 1 << (7 - self.bits);
                    self.bits += 1;
                }
                self.push_byte(self.current);
            }
            self.bytes
        }
    }

    /// Marker helpers.
    fn marker(code: u32) -> Vec<uint8> {
        vec![0xFF, code as uint8]
    }

    fn seg2(code: u32, payload: &[uint8]) -> Vec<uint8> {
        let mut out = marker(code);
        let len = (payload.len() + 2) as u16;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(payload);
        out
    }

    /// DHT segment with `symbols.len()` symbols at code lengths
    /// 1..=symbols.len() (one symbol per length): symbol at index i gets
    /// length i+1, code 2^(i+1) - 2. Symbols must be sorted ascending to
    /// match the decoder's huffval ordering (sorted by code length; equal
    /// lengths keep symbol order).
    fn dht_seg_multi(index: uint8, symbols: &[uint8]) -> Vec<uint8> {
        assert!(!symbols.is_empty() && symbols.len() <= 16);
        let mut payload = vec![index];
        let mut bits_row = [0u8; 16];
        for k in 0..symbols.len() {
            bits_row[k] = 1;
        }
        payload.extend_from_slice(&bits_row);
        payload.extend_from_slice(symbols);
        seg2(M_DHT, &payload)
    }

    /// Build a minimal lossless JPEG stream:
    /// SOI, SOF3, DHT (2-symbol table), SOS (PSV), entropy bytes, EOI.
    ///
    /// Huffman table: symbol `s0` with 1-bit code `0`, symbol `s1` with
    /// 2-bit code `10`.
    fn build_stream(
        precision: u32,
        width: u32,
        height: u32,
        psv: u32,
        symbols: &[uint8],
        entropy: &[u8],
    ) -> Vec<uint8> {
        let mut out = Vec::new();

        out.extend(marker(M_SOI));

        // SOF3: P, H, W, Ns=1, (id, 0x11, 0)
        let sof_payload = [
            precision as uint8,
            (height >> 8) as uint8,
            height as uint8,
            (width >> 8) as uint8,
            width as uint8,
            1,
            0,
            0x11,
            0,
        ];
        out.extend(seg2(M_SOF3, &sof_payload));

        // DHT: index 0 with the given symbol list.
        out.extend(dht_seg_multi(0, symbols));

        // SOS: Ns=1, (Cs=0, Td=0<<4), PSV, Se=0, Pt=0
        let sos_payload = [1, 0, 0, psv as uint8, 0, 0];
        out.extend(seg2(M_SOS, &sos_payload));

        out.extend_from_slice(entropy);

        out.extend(marker(M_EOI));

        out
    }


    /// Decoder-side value of a coded difference (Figure F.12):
    /// `d` from get_bits(s), then `HuffExtend`.
    fn extend(d: i32, s: u32) -> i32 {
        if d < (0x08000 >> (16 - s as i32)) {
            d + ((-1i32 << s) + 1)
        } else {
            d
        }
    }

    // -- encoder round-trip tests ------------------------------------------

    fn roundtrip(
        src: &[uint16],
        rows: u32,
        cols: u32,
        channels: u32,
        bit_depth: u32,
    ) -> Vec<uint16> {
        // Interleaved layout, matching dng_image_writer's pcInterleaved
        // pixel buffer: row_step = cols * channels, col_step = channels.
        let row_step = (cols * channels) as int32;
        let col_step = channels as int32;

        let mut enc_mem = dng_core::dng_memory_stream::DngMemoryStream::new();
        let mut enc_stream = DngStream::new(Box::new(enc_mem));
        encode_lossless_jpeg(
            src,
            rows,
            cols,
            channels,
            bit_depth,
            row_step,
            col_step,
            &mut enc_stream,
        )
        .unwrap_or_else(|e| panic!("encode failed: {e:?}"));

        let encoded = enc_stream.as_memory_block().unwrap().buffer().to_vec();

        // Decoder side: expected decoded size = rows*cols*channels*2.
        let expected_bytes = (rows * cols * channels * 2) as uint32;

        let mut dec_stream = DngStream::from_static(&encoded, 0);
        let mut spooler = VecSpooler::new();

        let dims = decode_lossless_jpeg(
            &mut dec_stream,
            &mut spooler,
            expected_bytes,
            expected_bytes,
            false,
            encoded.len() as uint64,
        )
        .expect("decode");

        assert_eq!(
            (dims.0, dims.1, dims.2),
            (cols, rows, channels),
            "decoded dimensions mismatch"
        );

        spooler.into_inner()
    }

    #[test]
    fn roundtrip_bit_depths() {
        // Bit depths 2, 4, 8, 12, 16 — synthetic gradients + noise.
        for &bits in &[2u32, 4, 8, 12, 16] {
            let max_val: u32 = (1u32 << bits) - 1;
            let (rows, cols, channels) = (5u32, 17u32, 3u32);

            let mut src = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    for ch in 0..channels {
                        let v = ((r * 7 + c * 13 + ch * 29) as u32 * max_val)
                            / (rows * cols * channels * max_val.max(1) / max_val.max(1))
                            % (max_val + 1);
                        src.push(v as uint16);
                    }
                }
            }

            let out = roundtrip(&src, rows, cols, channels, bits);
            assert_eq!(out, src, "bit depth {bits} round-trip mismatch");
        }
    }

    #[test]
    fn roundtrip_channel_counts() {
        for &channels in &[1u32, 2, 3, 4] {
            let (rows, cols) = (7u32, 9u32);
            let mut src = Vec::new();
            for r in 0..rows {
                for c in 0..cols {
                    for ch in 0..channels {
                        let v = ((r * 31 + c * 17 + ch * 5) % 251) as uint16;
                        src.push(v);
                    }
                }
            }
            let out = roundtrip(&src, rows, cols, channels, 8);
            assert_eq!(out, src, "channels {channels} round-trip mismatch");
        }
    }

    #[test]
    fn roundtrip_flat_constant_and_extremes() {
        // Constant image.
        let src: Vec<uint16> = vec![777; 4 * 6 * 3];
        let out = roundtrip(&src, 4, 6, 3, 12);
        assert_eq!(out, src);

        // Extremes at 8 bits: 0 and 255 alternating (max residuals).
        let mut src = Vec::new();
        for i in 0..(6 * 5 * 2) {
            src.push(if i % 2 == 0 { 0 } else { 255 });
        }
        let out = roundtrip(&src, 6, 5, 2, 8);
        assert_eq!(out, src);

        // Full-scale step at 16 bits (residual up to +/-65535 → s=16 case
        // handled via bug16=false path: d = -32768 for s==16).
        let mut src = Vec::new();
        for i in 0..(4 * 4 * 1) {
            src.push(if i % 2 == 0 { 0 } else { 65535 });
        }
        let out = roundtrip(&src, 4, 4, 1, 16);
        assert_eq!(out, src);
    }

    #[test]
    fn roundtrip_single_pixel() {
        let src = [42u16];
        let out = roundtrip(&src, 1, 1, 1, 8);
        assert_eq!(out, vec![42u16]);
    }

    #[test]
    fn roundtrip_non_contiguous_stride() {
        // Strided source: 2 rows x 3 cols x 2 channels with padding columns.
        let rows = 2u32;
        let cols = 3u32;
        let channels = 2u32;
        let row_step = (cols * channels + 4) as int32; // extra padding
        let col_step = channels as int32;

        let len = (rows as usize) * (row_step as usize);
        let mut src = vec![0u16; len];
        for r in 0..rows as usize {
            for c in 0..cols as usize {
                for ch in 0..channels as usize {
                    let idx = r * row_step as usize + c * col_step as usize + ch;
                    src[idx] = ((r * 11 + c * 7 + ch * 3) % 200) as uint16;
                }
            }
        }

        let mut enc_mem = dng_core::dng_memory_stream::DngMemoryStream::new();
        let mut enc_stream = DngStream::new(Box::new(enc_mem));
        encode_lossless_jpeg(&src, rows, cols, channels, 8, row_step, col_step, &mut enc_stream)
            .expect("encode");
        let encoded = enc_stream.as_memory_block().unwrap().buffer().to_vec();

        let mut dec_stream = DngStream::from_static(&encoded, 0);
        let mut spooler = VecSpooler::new();
        decode_lossless_jpeg(
            &mut dec_stream,
            &mut spooler,
            (rows * cols * channels * 2) as uint32,
            (rows * cols * channels * 2) as uint32,
            false,
            encoded.len() as uint64,
        )
        .expect("decode");

        // Verify the decoded interleaved data against the source layout.
        let out = spooler.into_inner();
        let mut expected = Vec::new();
        for r in 0..rows as usize {
            for c in 0..cols as usize {
                for ch in 0..channels as usize {
                    expected.push(src[r * row_step as usize + c * col_step as usize + ch]);
                }
            }
        }
        assert_eq!(out, expected);
    }

    // -- hand-built stream tests --------------------------------------------

    /// Hand-parse test: 2x2 image, precision 8, PSV 1 (left), pixels coded
    /// manually.
    #[test]
    fn hand_built_stream_psv1() {
        // Pixel plan (2x2, precision 8):
        //   row 0: [100, 120]  row 1: [90, 110]
        // Predictors: first pixel of first row = 1 << 7 = 128.
        //   (0,0): d = -28  → s = 5
        //   (0,1): d = +20  → s = 5
        //   (1,0): d = -10  → s = 4
        //   (1,1): d = +20  → s = 5
        // Symbols: [4, 5]: 4 → '0', 5 → '10' + 5 value bits.
        let symbols = [4u8, 5];
        // JPEG F.1.2.1: negative V is emitted as the low s bits of (V - 1).
        let mag = |d: i32| -> u32 {
            if d >= 0 {
                d as u32
            } else {
                (d as u32).wrapping_sub(1) & ((1 << bits_of(d)) - 1)
            }
        };

        let mut bw = JpegBitWriter::new();
        bw.write_symbol(1, 2); // s=5
        bw.write_bits(mag(-28), 5);
        bw.write_symbol(1, 2); // s=5
        bw.write_bits(mag(20), 5);
        bw.write_symbol(0, 2); // s=4
        bw.write_bits(mag(-10), 4);
        bw.write_symbol(1, 2); // s=5
        bw.write_bits(mag(20), 5);
        let entropy = bw.finish();

        let stream = build_stream(8, 2, 2, 1, &symbols, &entropy);

        let mut stream = DngStream::from_static(&stream, 0);
        let mut spooler = VecSpooler::new();

        let dims = decode_lossless_jpeg(
            &mut stream,
            &mut spooler,
            2 * 2 * 1 * 2,
            2 * 2 * 1 * 2,
            false,
            1 << 40,
        )
        .expect("hand-built decode");
        assert_eq!((dims.0, dims.1, dims.2), (2, 2, 1));

        let out = spooler.into_inner();
        assert_eq!(out, vec![100u16, 120, 90, 110]);
    }

    /// Predictor coverage: PSV 2..7 via hand-built streams. Pixels are
    /// chosen per PSV so the predicted value differs; the decoder must
    /// reconstruct them exactly.
    #[test]
    fn hand_built_stream_predictors_2_to_7() {
        // Image 3x2 (w x h), precision 8:
        //   row 0: [10, 200, 100]
        //   row 1: [30, X, Y]
        // Row 1 col 0 uses the upper predictor; cols 1.. use the PSV.

        let row0 = [10i32, 200, 100];
        let r1c0 = 30i32;

        for psv in 2u32..=7 {
            let predict = |left: i32, upper: i32, diag: i32| -> i32 {
                match psv {
                    2 => upper,
                    3 => diag,
                    4 => left + upper - diag,
                    5 => left + ((upper - diag) >> 1),
                    6 => upper + ((left - diag) >> 1),
                    7 => (left + upper) >> 1,
                    _ => unreachable!(),
                }
            };

            let pred1 = predict(r1c0, row0[1], row0[0]);
            let x = pred1 + 5;
            let pred2 = predict(x, row0[2], row0[1]);
            let y = pred2 + 3;
            if !(0..=255).contains(&x) || !(0..=255).contains(&y) {
                continue;
            }

            // Diffs and their s values.
            let diffs = [
                10 - 128, // (0,0)
                190,      // (0,1)
                -100,     // (0,2)
                20,       // (1,0) — upper predictor
                x - pred1,
                y - pred2,
            ];
            let s_of = |d: i32| -> u32 {
                if d == 0 {
                    0
                } else {
                    bits_of(d)
                }
            };

            // Collect the unique symbols, sorted ascending.
            let mut syms: Vec<u8> = diffs
                .iter()
                .map(|&d| s_of(d) as u8)
                .collect();
            syms.sort();
            syms.dedup();

            let idx = |sv: u32| -> u32 {
                syms.iter().position(|&x| x as u32 == sv).unwrap() as u32
            };
            let count = syms.len() as u32;

            // JPEG F.1.2.1: negative V is emitted as the low s bits of
            // (V - 1).
            let mag = |d: i32| -> u32 {
                if d >= 0 {
                    d as u32
                } else {
                    (d as u32).wrapping_sub(1) & ((1 << bits_of(d)) - 1)
                }
            };

            let mut bw = JpegBitWriter::new();
            for &d in &diffs {
                let sv = s_of(d);
                bw.write_symbol(idx(sv), count);
                if sv != 0 && sv != 16 {
                    bw.write_bits(mag(d), sv);
                }
            }
            let entropy = bw.finish();

            let stream = build_stream(8, 3, 2, psv, &syms, &entropy);
            let mut stream = DngStream::from_static(&stream, 0);
            let mut spooler = VecSpooler::new();

            decode_lossless_jpeg(
                &mut stream,
                &mut spooler,
                3 * 2 * 1 * 2,
                3 * 2 * 1 * 2,
                false,
                1 << 40,
            )
            .unwrap_or_else(|e| panic!("hand-built decode PSV {psv}: {e:?}"));

            let out = spooler.into_inner();
            assert_eq!(
                out,
                vec![
                    10u16,
                    200,
                    100,
                    r1c0 as uint16,
                    x as uint16,
                    y as uint16
                ],
                "PSV {psv} mismatch"
            );
        }
    }

    fn bits_of(d: i32) -> u32 {
        if d == 0 {
            return 0;
        }
        let m = d.unsigned_abs();
        32 - m.leading_zeros()
    }

    fn mag_bits_of(d: i32) -> u32 {
        if d >= 0 {
            d as u32
        } else {
            (d.unsigned_abs().wrapping_sub(1)) & ((1u32 << bits_of(d)) - 1)
        }
    }

    /// bug16 flag handling: s=16 coded symbol.
    #[test]
    fn hand_built_bug16() {
        // Precision 8, 1x1 image. Code s=16 for the first pixel.
        // bug16=false: d = -32768 → pixel = (-32768 + 128) as u16.
        let mut bw = JpegBitWriter::new();
        bw.write_bits(0b0, 1); // symbol 16 (s0=16): 1-bit code '0'
        let entropy = bw.finish();

        let stream = build_stream(8, 1, 1, 1, &[16, 1], &entropy);
        let mut stream = DngStream::from_static(&stream, 0);
        let mut spooler = VecSpooler::new();

        decode_lossless_jpeg(&mut stream, &mut spooler, 2, 2, false, 1 << 40).expect("decode bug16=false");

        let expected = (-32768i32 + 128) as u16 as uint16;
        assert_eq!(spooler.into_inner(), vec![expected]);

        // bug16=true: 16 magnitude bits are read and extended.
        // s=16 → 16 bits read; choose mag = 0x1234 →
        // HuffExtend: 0x1234 < 0x8000 → +((-1<<16)+1) = 0x1234 - 65535.
        let mut bw = JpegBitWriter::new();
        bw.write_bits(0b0, 1);
        bw.write_bits(0x1234, 16);
        let entropy = bw.finish();

        let stream = build_stream(8, 1, 1, 1, &[16, 1], &entropy);
        let mut stream = DngStream::from_static(&stream, 0);
        let mut spooler = VecSpooler::new();

        decode_lossless_jpeg(&mut stream, &mut spooler, 2, 2, true, 1 << 40).expect("decode bug16=true");

        let d = extend(0x1234, 16);
        let expected = (d + 128) as u16 as uint16;
        assert_eq!(spooler.into_inner(), vec![expected]);
    }

    /// Restart interval handling: 4-wide image, DRI=4 → one restart marker
    /// before row 1 (restartInRows = 4/4 = 1).
    #[test]
    fn hand_built_restart_interval() {
        // Image 4x2 (w=4, h=2), precision 8, PSV 1.
        // Row 0: [50, 60, 70, 80] — first pixel diffs from predictor 128.
        // RST0 marker.
        // Row 1 (restart row): [20, 30, 40, 50] — predictor resets to 128.
        let row0 = [50i32, 60, 70, 80];
        let row1 = [20i32, 30, 40, 50];

        // Diffs: row0: -78 (s7), 10 (s4) x3; row1: -108 (s7), 10 (s4) x3.
        let symbols = [4u8, 7];
        let idx = |sv: u32| -> u32 {
            symbols.iter().position(|&x| x as u32 == sv).unwrap() as u32
        };
        // JPEG F.1.2.1: negative V is emitted as the low s bits of (V - 1).
        let mag = |d: i32| -> u32 {
            if d >= 0 {
                d as u32
            } else {
                (d as u32).wrapping_sub(1) & ((1 << bits_of(d)) - 1)
            }
        };

        let encode_row = |bw: &mut JpegBitWriter, row: &[i32]| {
            let mut pred = 128i32; // restart rows re-seed the predictor
            for (i, &v) in row.iter().enumerate() {
                let p = if i == 0 { pred } else { row[i - 1] };
                let d = v - p;
                let sv = bits_of(d);
                bw.write_symbol(idx(sv), 2);
                bw.write_bits(mag(d), sv);
            }
            let _ = &mut pred;
        };

        let mut bw = JpegBitWriter::new();
        encode_row(&mut bw, &row0);
        let entropy0 = bw.finish();

        let mut bw = JpegBitWriter::new();
        encode_row(&mut bw, &row1);
        let entropy1 = bw.finish();

        // Assemble stream with DRI and RST0 between rows.
        let mut out = Vec::new();
        out.extend(marker(M_SOI));

        let sof_payload = [
            8u8,
            0,
            2, // height 2
            0,
            4, // width 4
            1,
            0,
            0x11,
            0,
        ];
        out.extend(seg2(M_SOF3, &sof_payload));

        out.extend(dht_seg_multi(0, &symbols));

        // DRI: length 4, interval 4.
        out.extend(marker(0xDD));
        out.extend_from_slice(&[0, 4, 0, 4]);

        let sos_payload = [1, 0, 0, 1, 0, 0];
        out.extend(seg2(M_SOS, &sos_payload));

        out.extend_from_slice(&entropy0);
        out.extend(marker(M_RST0));
        out.extend_from_slice(&entropy1);
        out.extend(marker(M_EOI));

        let mut stream = DngStream::from_static(&out, 0);
        let mut spooler = VecSpooler::new();

        decode_lossless_jpeg(
            &mut stream,
            &mut spooler,
            4 * 2 * 1 * 2,
            4 * 2 * 1 * 2,
            false,
            1 << 40,
        )
        .expect("restart decode");

        let out = spooler.into_inner();
        assert_eq!(
            out,
            row0.iter()
                .chain(row1.iter())
                .map(|&v| v as uint16)
                .collect::<Vec<_>>()
        );
    }

    /// Canon sRAW-like stream (3 components, hSamp=2/1/1, precision 15,
    /// Ss=1, even width): verifies the h2v1 decoder branch.
    #[test]
    fn hand_built_canon_sraw_h2v1() {
        // Image 4x1 (w=4 h=1), precision 15, 3 components, comp0 hSamp=2.
        // Decoder branch: h2v1, Canon (not sony_sraw2 since precision != 16).
        // Predictors row 0: p0 = 1 << 14, p1 = p2 = 1 << 14.
        //
        // col 0: lum d → p0 += d → [0] = p0; lum d → p0 += d → [1] = p0;
        //        ch1 d → p1 += d → [0],[1] = p1; ch2 d → p2 += d → [0],[1] = p2.
        // col 2: same again.

        let p0_vals = [16384i32, 16390, 16400, 16410];
        let p1_final = 16500i32;
        let p2_final = 16520i32;

        // Luminance diffs: 6 (s=3), 10 (s=4), 10, 10.
        // Chroma diffs: p1: 116 (s=7); p2: 136 (s=8); second col: 0 (s=0).
        let symbols0 = [3u8, 4];
        let symbols1 = [0u8, 7];
        let symbols2 = [0u8, 8];

        let idx = |symbols: &[u8], sv: u32| -> u32 {
            symbols.iter().position(|&x| x as u32 == sv).unwrap() as u32
        };
        // JPEG F.1.2.1: negative V is emitted as the low s bits of (V - 1).
        let mag = |d: i32| -> u32 {
            if d >= 0 {
                d as u32
            } else {
                (d as u32).wrapping_sub(1) & ((1 << bits_of(d)) - 1)
            }
        };

        let mut bw = JpegBitWriter::new();

        // The h2v1 decoder reads, per 2-column group:
        //   lum(col), lum(col+1), chroma1(col,col+1), chroma2(col,col+1).
        // Group 1 (cols 0-1): lum diffs 6 (s=3), 10 (s=4); chroma 116 (s=7),
        // 136 (s=8).  Group 2 (cols 2-3): lum diffs 10, 10 (s=4); chroma 0, 0.
        for (lum_diffs, ch1_diff, ch2_diff) in [
            ([6i32, 10], Some(116i32), Some(136i32)),
            ([10, 10], Some(0), Some(0)),
        ] {
            for d in lum_diffs {
                let sv = bits_of(d);
                bw.write_symbol(idx(&symbols0, sv), 2);
                bw.write_bits(mag(d), sv);
            }
            for (symbols, diff) in [(&symbols1, ch1_diff), (&symbols2, ch2_diff)] {
                let d = diff.unwrap();
                let sv = bits_of(d);
                bw.write_symbol(idx(symbols, sv), 2);
                if sv != 0 {
                    bw.write_bits(mag(d), sv);
                }
            }
        }

        let entropy = bw.finish();

        // Build the stream manually (3 components).
        let mut out = Vec::new();
        out.extend(marker(M_SOI));

        let sof_payload = [
            15u8,
            0,
            1, // height 1
            0,
            4, // width 4 (even)
            3, // 3 components
            0,
            0x21, // comp0: hSamp=2, vSamp=1
            0,
            1,
            0x11, // comp1
            0,
            2,
            0x11, // comp2
            0,
        ];
        out.extend(seg2(M_SOF3, &sof_payload));

        out.extend(dht_seg_multi(0, &symbols0));
        out.extend(dht_seg_multi(1, &symbols1));
        out.extend(dht_seg_multi(2, &symbols2));

        // SOS: Ns=3, (Cs=0, Td=0), (Cs=1, Td=1<<4), (Cs=2, Td=2<<4), Ss=1.
        let sos_payload = [3u8, 0, 0x00, 1, 0x10, 2, 0x20, 1, 0, 0];
        out.extend(seg2(M_SOS, &sos_payload));

        out.extend_from_slice(&entropy);
        out.extend(marker(M_EOI));

        let mut stream = DngStream::from_static(&out, 0);
        let mut spooler = VecSpooler::new();

        let dims = decode_lossless_jpeg(
            &mut stream,
            &mut spooler,
            4 * 1 * 3 * 2,
            4 * 1 * 3 * 2,
            false,
            1 << 40,
        )
        .expect("sRAW h2v1 decode");
        assert_eq!((dims.0, dims.1, dims.2), (4, 1, 3));

        let out = spooler.into_inner();
        // Luma values: 16384 + 6, +10, +10, +10 → 16390, 16400, 16410, 16420;
        // chroma constant at 16500 / 16520.
        assert_eq!(
            out,
            vec![
                16390u16, 16500, 16520, 16400, 16500, 16520, 16410, 16500, 16520, 16420, 16500,
                16520
            ]
        );
    }

    /// Hasselblad 3FR-style stream (Ss=8, even width): the decoder pairs
    /// columns and seeds both predictors at 32768.
    ///
    /// PORT-NOTE on test data: the C++ 3FR bit-buffer fill reads 4 bytes at
    /// a time and packs them last-read-first (`getBuffer = c3c2c1c0`), so
    /// the effective bit consumption order reverses each 4-byte group. The
    /// entropy below is a single repeated byte pattern (one symbol, one
    /// 3-bit value, uniform bytes) so the reversal is a no-op, and it is
    /// long enough that the preemptive 32-bit fills never reach the EOI
    /// marker.
    #[test]
    fn hand_built_hasselblad_3fr() {
        // Image 16x1 (w=16 h=1), precision 16, 1 component, PSV=8.
        // 8 col-pairs; every read decodes diff +4 (symbol 3, value 100b).
        let symbols = [3u8];

        // Bit layout per col pair (the C++ reads BOTH symbols before either
        // value): sym0, sym1, val0, val1 → '0' '0' '100' '100' = 0x24.
        let mut bw = JpegBitWriter::new();
        for _ in 0..8 {
            bw.write_symbol(0, 1); // symbol 3 (p0)
            bw.write_symbol(0, 1); // symbol 3 (p1)
            bw.write_bits(4, 3); // val0
            bw.write_bits(4, 3); // val1
        }
        let entropy = bw.finish();
        assert_eq!(entropy.len(), 8);
        assert!(entropy.iter().all(|&b| b == 0x24), "reversal-neutral entropy required");

        let mut out = Vec::new();
        out.extend(marker(M_SOI));

        let sof_payload = [16u8, 0, 1, 0, 16, 1, 0, 0x11, 0];
        out.extend(seg2(M_SOF3, &sof_payload));
        out.extend(dht_seg_multi(0, &symbols));

        // SOS with PSV = 8 (Hasselblad).
        let sos_payload = [1u8, 0, 0, 8, 0, 0];
        out.extend(seg2(M_SOS, &sos_payload));
        out.extend_from_slice(&entropy);
        out.extend(marker(M_EOI));

        let mut stream = DngStream::from_static(&out, 0);
        let mut spooler = VecSpooler::new();

        let dims = decode_lossless_jpeg(
            &mut stream,
            &mut spooler,
            16 * 1 * 1 * 2,
            16 * 1 * 1 * 2,
            false,
            1 << 40,
        )
        .expect("Hasselblad decode");
        assert_eq!((dims.0, dims.1, dims.2), (16, 1, 1));

        // p0 and p1 each accumulate +4 per col pair: 32772, 32776, … 32800.
        let out = spooler.into_inner();
        let mut expected = Vec::new();
        for k in 1..=8u32 {
            let v = (32768 + 4 * k) as uint16;
            expected.push(v);
            expected.push(v);
        }
        assert_eq!(out, expected);
    }

    /// Bad format handling.
    #[test]
    fn bad_format_errors() {
        // Not a JPEG stream.
        let mut stream = DngStream::from_static(b"nope", 0);
        let mut spooler = VecSpooler::new();
        assert!(decode_lossless_jpeg(&mut stream, &mut spooler, 0, 1 << 30, false, 1 << 40).is_err());

        // Empty stream.
        let mut stream = DngStream::from_static(&[], 0);
        let mut spooler = VecSpooler::new();
        assert!(decode_lossless_jpeg(&mut stream, &mut spooler, 0, 1 << 30, false, 1 << 40).is_err());

        // Truncated entropy (dimensions say 4 pixels but data ends early) —
        // the decoder tolerates EOF by stuffing zeros; only dimension check
        // failures are hard errors. Build a stream whose decoded size check
        // fails: min > max impossible sizes.
        let stream_bytes = build_stream(8, 2, 2, 1, &[0, 1], &[0]);
        let mut stream = DngStream::from_static(&stream_bytes, 0);
        let mut spooler = VecSpooler::new();
        // decoded size = 2*2*1*2 = 8; ask for min 100 → BadFormat.
        assert!(decode_lossless_jpeg(&mut stream, &mut spooler, 100, 200, false, 1 << 40).is_err());

        // Overshoot of endOfData.
        let mut stream = DngStream::from_static(&stream_bytes, 0);
        let mut spooler = VecSpooler::new();
        assert!(decode_lossless_jpeg(&mut stream, &mut spooler, 8, 8, false, 2).is_err());
    }

    /// Encoder emits markers we can parse back: SOI/SOF3/DHT/SOS/EOI present.
    #[test]
    fn encoder_stream_structure() {
        let src = vec![1u16, 2, 3, 4];
        let mut enc_mem = dng_core::dng_memory_stream::DngMemoryStream::new();
        let mut enc_stream = DngStream::new(Box::new(enc_mem));
        encode_lossless_jpeg(&src, 1, 4, 1, 8, 4, 1, &mut enc_stream).expect("encode");
        let encoded = enc_stream.as_memory_block().unwrap().buffer().to_vec();

        // SOI.
        assert_eq!(&encoded[0..2], &[0xFF, M_SOI as uint8]);
        // SOF3.
        assert_eq!(&encoded[2..4], &[0xFF, M_SOF3 as uint8]);
        // EOI at the end.
        assert_eq!(&encoded[encoded.len() - 2..], &[0xFF, M_EOI as uint8]);
        // Contains DHT and SOS.
        let window = encoded.windows(2);
        assert!(window.clone().any(|w| w == [0xFF, M_DHT as uint8]));
        assert!(window.clone().any(|w| w == [0xFF, M_SOS as uint8]));
        let _ = M_APP0; // referenced to silence unused-import in some configs
    }
}

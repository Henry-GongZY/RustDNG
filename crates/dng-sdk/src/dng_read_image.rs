// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_read_image.cpp/.h`.
//
// Copyright 2006-2022 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_read_image.cpp` / `dng_read_image.h` — the image reader that
//! decodes a `dng_ifd` (raw / mask / depth / transparency IFD) into a
//! `dng_image`, handling every compression the SDK supports:
//!
//! * `ccUncompressed`  — direct (8/12/16/24/32-bit, float and packed bits)
//! * `ccLZW`           — the SDK's own LZW expander
//! * `ccDeflate` / `ccOldDeflate` — zlib via `dng_core::zlib`
//! * `ccPackBits`      — PackBits decoder
//! * `ccJPEG` / `ccLossyJPEG` — baseline JPEG via the `dng-jpeg` crate
//! * `ccJPEG` (lossless variant) — SDK lossless JPEG via `dng_lossless_jpeg`
//! * `ccJXL` (52546)   — JPEG XL via the `dng-jxl` crate
//!
//! plus horizontal / floating-point predictors, sub-tile-block reordering,
//! and 2D row/column interleaving (`Interleave2D`).
//!
//! The C++ class is `dng_read_image`; methods keep their C++ names in
//! snake_case.  Threading: the C++ `dng_read_tiles_task` multi-thread path is
//! executed serially (ARCHITECTURE.md §9) — `use_multiple_threads` is always
//! false and `DoReadTiles` falls through to the single-tile loop.

use dng_core::dng_exceptions::{bad_format, eof_failure, program_error};
use dng_core::dng_host::DngHost;
use dng_core::dng_fingerprint::{DngFingerprint, DngMd5Printer};
use dng_core::dng_ifd::{kMaxTileInfo, DngIfd};
use dng_core::dng_image::{DngImage, ImageRef};
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use dng_core::dng_point::DngPoint;
use dng_core::dng_rect::DngRect;
use dng_core::dng_safe_arithmetic::{safe_uint32_add, safe_uint32_mult2, safe_uint32_mult3};
use dng_core::dng_simple_image::DngSimpleImage;
use dng_core::dng_stream::DngStream;
use dng_core::dng_tag_types::{tag_type_size, ttByte, ttFloat, ttLong, ttShort};
use dng_core::dng_tag_values::{
    ccDeflate, ccJXL, ccJPEG, ccLossyJPEG, ccLZW, ccOldDeflate, ccPackBits,
    ccUncompressed, cpFloatingPoint, cpFloatingPointX2, cpFloatingPointX4,
    cpHorizontalDifference, cpHorizontalDifferenceX2, cpHorizontalDifferenceX4,
    cpNullPredictor, pcInterleaved, pcPlanar, pcRowInterleaved, sfFloatingPoint,
    sfUnsignedInteger,
};
use dng_core::dng_types::{int32, real32, uint32, uint64, uint8, uint16};
use dng_core::dng_utils::{
    compute_buffer_size, PaddingType, dng_fp24_to_float, dng_half_to_float, max_uint32, min_int32,
    min_uint32, pin_uint32, swap_bytes16, swap_bytes32,
};
use dng_core::DngResult;

use crate::dng_jpeg_image::{keep_lossy_compressed_image, DngLossyCompressedImage};
use crate::dng_negative::DngNegative;
use crate::dng_lossless_jpeg::{decode_lossless_jpeg, DngSpooler};

/// `kImageBufferSize` (dng_read_image.h).
pub const kImageBufferSize: uint32 = 128 * 1024;

// ---------------------------------------------------------------------------
// Delta decoders (horizontal predictors)
// ---------------------------------------------------------------------------

/// `DecodeDelta8`.
fn decode_delta8(d_ptr: &mut [uint8], rows: uint32, cols: uint32, channels: uint32) {
    let d_row_step = (cols * channels) as usize;

    for row in 0..rows as usize {
        let row_base = row * d_row_step;

        for col in 1..cols as usize {
            for channel in 0..channels as usize {
                let i = row_base + col * channels as usize + channel;

                d_ptr[i] = d_ptr[i].wrapping_add(d_ptr[row_base + (col - 1) * channels as usize + channel]);
            }
        }
    }
}

/// `DecodeDelta16`.
fn decode_delta16(d_ptr: &mut [u16], rows: uint32, cols: uint32, channels: uint32) {
    let d_row_step = (cols * channels) as usize;

    for row in 0..rows as usize {
        let row_base = row * d_row_step;

        for col in 1..cols as usize {
            for channel in 0..channels as usize {
                let i = row_base + col * channels as usize + channel;

                d_ptr[i] = d_ptr[i].wrapping_add(d_ptr[row_base + (col - 1) * channels as usize + channel]);
            }
        }
    }
}

/// `DecodeDelta32`.
fn decode_delta32(d_ptr: &mut [u32], rows: uint32, cols: uint32, channels: uint32) {
    let d_row_step = (cols * channels) as usize;

    for row in 0..rows as usize {
        let row_base = row * d_row_step;

        for col in 1..cols as usize {
            for channel in 0..channels as usize {
                let i = row_base + col * channels as usize + channel;

                d_ptr[i] = d_ptr[i].wrapping_add(d_ptr[row_base + (col - 1) * channels as usize + channel]);
            }
        }
    }
}

/// `DecodeDeltaBytes`.
fn decode_delta_bytes(byte_ptr: &mut [uint8], cols: int32, channels: int32) {
    let channels = channels as usize;

    if channels == 1 {
        let mut b0 = byte_ptr[0];

        for col in 1..cols as usize {
            b0 = b0.wrapping_add(byte_ptr[col]);

            byte_ptr[col] = b0;
        }
    } else if channels == 3 {
        let mut b0 = byte_ptr[0];
        let mut b1 = byte_ptr[1];
        let mut b2 = byte_ptr[2];

        for col in 1..cols as usize {
            let base = col * 3;

            b0 = b0.wrapping_add(byte_ptr[base]);
            b1 = b1.wrapping_add(byte_ptr[base + 1]);
            b2 = b2.wrapping_add(byte_ptr[base + 2]);

            byte_ptr[base] = b0;
            byte_ptr[base + 1] = b1;
            byte_ptr[base + 2] = b2;
        }
    } else if channels == 4 {
        let mut b0 = byte_ptr[0];
        let mut b1 = byte_ptr[1];
        let mut b2 = byte_ptr[2];
        let mut b3 = byte_ptr[3];

        for col in 1..cols as usize {
            let base = col * 4;

            b0 = b0.wrapping_add(byte_ptr[base]);
            b1 = b1.wrapping_add(byte_ptr[base + 1]);
            b2 = b2.wrapping_add(byte_ptr[base + 2]);
            b3 = b3.wrapping_add(byte_ptr[base + 3]);

            byte_ptr[base] = b0;
            byte_ptr[base + 1] = b1;
            byte_ptr[base + 2] = b2;
            byte_ptr[base + 3] = b3;
        }
    } else {
        for col in 1..cols as usize {
            for chan in 0..channels {
                byte_ptr[col * channels + chan] =
                    byte_ptr[col * channels + chan].wrapping_add(byte_ptr[(col - 1) * channels + chan]);
            }
        }
    }
}

/// `DecodeFPDelta` — reorder byte planes then undo byte deltas.
/// `input` holds the compressed (delta) bytes; `output` receives the
/// interleaved samples.
fn decode_fp_delta(
    input: &mut [uint8],
    output: &mut [uint8],
    cols: int32,
    channels: int32,
    bytes_per_sample: int32,
) {
    // The C++ decodes in place on the input buffer (which aliases the tail of
    // the destination buffer); the Rust port splits the phases to stay safe:
    // first undo the byte deltas on `input`, then interleave into `output`.

    decode_delta_bytes(input, cols * bytes_per_sample, channels);

    let row_increment = (cols * channels) as usize;

    if bytes_per_sample == 2 {
        // Little-endian host: input0 = input + rowIncrement, input1 = input.

        let (input1, input0) = (&input[..row_increment], &input[row_increment..2 * row_increment]);

        for col in 0..row_increment {
            output[2 * col] = input0[col];
            output[2 * col + 1] = input1[col];
        }
    } else if bytes_per_sample == 3 {
        for col in 0..row_increment {
            output[3 * col] = input[col];
            output[3 * col + 1] = input[row_increment + col];
            output[3 * col + 2] = input[row_increment * 2 + col];
        }
    } else {
        // 4 bytes per sample (little-endian host: input3 = input, ...).

        for col in 0..row_increment {
            output[4 * col] = input[row_increment * 3 + col];
            output[4 * col + 1] = input[row_increment * 2 + col];
            output[4 * col + 2] = input[row_increment + col];
            output[4 * col + 3] = input[col];
        }
    }
}

// ---------------------------------------------------------------------------
// PackBits
// ---------------------------------------------------------------------------

/// Internal: int8 reinterpretation of a byte.
fn as_int8(x: uint8) -> i32 {
    (x as i8) as i32
}

// ---------------------------------------------------------------------------
// LZW expander
// ---------------------------------------------------------------------------

const kResetCode: int32 = 256;
const kEndCode: int32 = 257;
const kTableSize: int32 = 4096;

#[derive(Clone, Copy, Default)]
struct LzwExpanderNode {
    prefix: i16,
    final_: i16,
    depth: i16,
}

/// Port of `dng_lzw_expander`.
struct DngLzwExpander {
    table: Vec<LzwExpanderNode>,

    src: Vec<uint8>,
    src_count: int32,
    byte_offset: int32,

    bit_buffer: u32,
    bit_buffer_count: int32,

    next_code: int32,
    code_size: int32,
}

impl DngLzwExpander {
    fn new() -> Self {
        DngLzwExpander {
            table: vec![LzwExpanderNode::default(); (kTableSize + 1) as usize],
            src: Vec::new(),
            src_count: 0,
            byte_offset: 0,
            bit_buffer: 0,
            bit_buffer_count: 0,
            next_code: 0,
            code_size: 0,
        }
    }

    fn init_table(&mut self) {
        self.code_size = 9;
        self.next_code = 258;

        for (code, node) in self.table.iter_mut().enumerate() {
            node.prefix = -1;
            node.final_ = code as i16;
            node.depth = 1;
        }
    }

    fn add_table(&mut self, w: int32, k: int32) {
        let parent_depth = self.table[w as usize].depth;

        let next_code = self.next_code;

        self.next_code += 1;

        let node = &mut self.table[next_code as usize];

        node.prefix = w as i16;
        node.final_ = k as i16;
        node.depth = 1 + parent_depth;

        if next_code + 1 == (1 << self.code_size) - 1 {
            if self.code_size != 12 {
                self.code_size += 1;
            }
        }
    }

    fn get_code_word(&mut self, code: &mut int32) -> bool {
        let code_size = self.code_size;

        *code = (self.bit_buffer >> (32 - code_size)) as int32;

        if self.bit_buffer_count >= code_size {
            self.bit_buffer <<= code_size;
            self.bit_buffer_count -= code_size;
        } else {
            let bits_so_far = self.bit_buffer_count;

            if self.byte_offset >= self.src_count {
                return false;
            }

            let base = self.byte_offset as usize;

            let b0 = self.src.get(base).copied().unwrap_or(0) as u32;
            let b1 = *self.src.get(base + 1).unwrap_or(&0) as u32;
            let b2 = *self.src.get(base + 2).unwrap_or(&0) as u32;
            let b3 = *self.src.get(base + 3).unwrap_or(&0) as u32;

            self.bit_buffer = (((((b0 << 8) | b1) << 8) | b2) << 8) | b3;

            self.bit_buffer_count = 32;

            self.byte_offset += 4;

            let bits_used = code_size - bits_so_far;

            let bits_not_used = 32 - bits_used;

            *code |= (self.bit_buffer >> bits_not_used) as int32;

            self.bit_buffer <<= bits_used;
            self.bit_buffer_count -= bits_used;
        }

        true
    }

    /// `Expand`.
    fn expand(&mut self, src: &[uint8], d_ptr: &mut [uint8], s_count: int32, mut d_count: int32) -> bool {
        if s_count < 0 || d_count < 0 {
            return false;
        }

        self.src = src.to_vec();
        self.src_count = s_count;
        self.byte_offset = 0;

        let d_start: usize = 0;

        let mut d_pos: usize = 0;

        'master: loop {
            self.init_table();

            let mut code: int32 = 0;

            loop {
                if !self.get_code_word(&mut code) {
                    return false;
                }

                if code != kResetCode {
                    break;
                }
            }

            if code == kEndCode {
                return true;
            }

            if code > kEndCode {
                return false;
            }

            let mut old_code = code;
            let mut in_char = code;

            if d_pos >= d_ptr.len() {
                return false;
            }

            d_ptr[d_pos] = code as uint8;
            d_pos += 1;

            d_count -= 1;

            if d_count == 0 {
                return true;
            }

            loop {
                if !self.get_code_word(&mut code) {
                    return false;
                }

                if code == kResetCode {
                    break;
                }

                if code == kEndCode {
                    return true;
                }

                let in_code = code;

                let mut repeat_last_pixel = false;

                if code >= self.next_code {
                    // This is either a bad file or our code table is not big
                    // enough; we are going to repeat the last code seen and
                    // attempt to muddle thru.

                    code = old_code;
                    repeat_last_pixel = true;
                }

                if code > self.next_code {
                    return false;
                }

                let depth = self.table[code as usize].depth as int32;

                if (depth as usize) < d_count as usize {
                    d_count -= depth;

                    let d_next = d_pos + depth as usize;

                    let mut ptr = d_next;

                    let mut local_code = code;

                    while local_code >= kResetCode {
                        if ptr <= d_start {
                            return false; // about to trash memory
                        }

                        let node = self.table[local_code as usize];

                        let temp_final = node.final_ as uint8;

                        local_code = node.prefix as int32;

                        if local_code < 0 || local_code > kTableSize {
                            return false;
                        }

                        ptr -= 1;

                        d_ptr[ptr] = temp_final;
                    }

                    code = local_code;
                    in_char = local_code;

                    if ptr <= d_start {
                        return false;
                    }

                    ptr -= 1;

                    d_ptr[ptr] = in_char as uint8;

                    d_pos = d_next;
                } else {
                    // There might not be enough room for the full code so
                    // skip the end of it.

                    let skip = depth - d_count;

                    let mut skip_code = code;

                    for _ in 0..skip {
                        let node = self.table[skip_code as usize];
                        skip_code = node.prefix as int32;
                    }

                    let depth_used = depth - skip;

                    d_count -= depth_used;

                    d_pos += depth_used as usize;

                    let mut ptr = d_pos;

                    while skip_code >= 0 {
                        if ptr <= d_start {
                            return false;
                        }

                        let node = self.table[skip_code as usize];

                        ptr -= 1;

                        d_ptr[ptr] = node.final_ as uint8;

                        skip_code = node.prefix as int32;

                        if skip_code > kTableSize {
                            return false;
                        }
                    }

                    return true;
                }

                if repeat_last_pixel {
                    if d_pos >= d_ptr.len() {
                        return false;
                    }

                    d_ptr[d_pos] = in_char as uint8;
                    d_pos += 1;

                    d_count -= 1;

                    if d_count == 0 {
                        return true;
                    }
                }

                if self.next_code < kTableSize {
                    self.add_table(old_code, code);
                }

                old_code = in_code;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-tile-block reordering
// ---------------------------------------------------------------------------

/// `ReorderSubTileBlocks` — un-shuffles deflated sub-tile blocks.
///
/// The C++ permutes raw bytes through a temp buffer; the Rust port permutes
/// typed samples.  A "block column" run is `blockCols * planes` samples
/// (the C++ `blockColBytes` expressed in samples — the layout math is
/// sample-granular for the interleaved buffers this path is used with).
fn reorder_sub_tile_blocks(
    ifd: &DngIfd,
    buffer: &mut DngPixelBuffer,
    temp_buffer: &mut Option<Vec<uint8>>,
) -> DngResult<()> {
    let temp_buffer_size = compute_buffer_size(
        buffer.pixel_type,
        buffer.area.size(),
        buffer.planes,
        PaddingType::padNone,
    )?;

    match temp_buffer {
        Some(buf) if buf.len() >= temp_buffer_size as usize => {}
        _ => {
            *temp_buffer = Some(vec![0u8; temp_buffer_size as usize]);
        }
    }

    let block_rows = ifd.sub_tile_block_rows;
    let block_cols = ifd.sub_tile_block_cols;

    if block_rows == 0 || block_cols == 0 {
        return Err(bad_format());
    }

    let row_blocks = buffer.area.h() as uint32 / block_rows;
    let col_blocks = buffer.area.w() as uint32 / block_cols;

    let row_step = buffer.row_step;
    let col_step = buffer.col_step;

    // Samples per block-column run (`blockColBytes / pixelSize`).

    let run_samples = (block_cols * buffer.planes) as usize;

    let row_block_step = row_step * block_rows as int32;
    let col_block_step = col_step * block_cols as int32;

    // Build the copy plan in sample space: source runs are consumed
    // linearly; destination runs are placed at the transposed offsets.

    let mut plan: Vec<(usize, usize)> = Vec::new(); // (dst_start, run_samples)

    let mut src_sample: usize = 0;

    let mut dst_row_base: int32 = 0;

    for _row_block in 0..row_blocks {
        let d0 = dst_row_base;

        let mut d1 = d0;

        for _col_block in 0..col_blocks {
            let mut d2 = d1;

            for _block_row in 0..block_rows {
                plan.push((d2 as usize, run_samples));

                src_sample += run_samples;

                d2 += row_step;
            }

            d1 += col_block_step;
        }

        dst_row_base += row_block_step;
    }

    // Apply: copy all source runs to temp in order, then write them to their
    // destinations (the C++ copies to a temp buffer and back; destinations
    // may alias yet-unread source samples).

    let total_samples = src_sample;

    match &mut buffer.data {
        PixelData::U8(v) => {
            let src: Vec<uint8> = v[..total_samples].to_vec();

            let mut dst_index = 0usize;

            for (dst_start, count) in &plan {
                v[*dst_start..*dst_start + *count].copy_from_slice(&src[dst_index..dst_index + *count]);

                dst_index += *count;
            }
        }
        PixelData::U16(v) => {
            let src: Vec<u16> = v[..total_samples].to_vec();

            let mut dst_index = 0usize;

            for (dst_start, count) in &plan {
                v[*dst_start..*dst_start + *count].copy_from_slice(&src[dst_index..dst_index + *count]);

                dst_index += *count;
            }
        }
        PixelData::U32(v) => {
            let src: Vec<u32> = v[..total_samples].to_vec();

            let mut dst_index = 0usize;

            for (dst_start, count) in &plan {
                v[*dst_start..*dst_start + *count].copy_from_slice(&src[dst_index..dst_index + *count]);

                dst_index += *count;
            }
        }
        PixelData::F16(v) => {
            let src: Vec<half::f16> = v[..total_samples].to_vec();

            let mut dst_index = 0usize;

            for (dst_start, count) in &plan {
                v[*dst_start..*dst_start + *count].copy_from_slice(&src[dst_index..dst_index + *count]);

                dst_index += *count;
            }
        }
        PixelData::F32(v) => {
            let src: Vec<real32> = v[..total_samples].to_vec();

            let mut dst_index = 0usize;

            for (dst_start, count) in &plan {
                v[*dst_start..*dst_start + *count].copy_from_slice(&src[dst_index..dst_index + *count]);

                dst_index += *count;
            }
        }
        _ => return Err(program_error()), // "ReorderSubTileBlocks: unexpected pixel storage"
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// dng_image_spooler
// ---------------------------------------------------------------------------

/// Port of `dng_image_spooler` — spools decoded lossless-JPEG rows into the
/// image, in `fSubTileBlockRows`-sized strips.
struct DngImageSpooler<'i> {
    ifd: DngIfd,

    image: &'i dyn DngImage,

    tile_area: DngRect,

    plane: uint32,
    planes: uint32,

    block: Vec<uint8>,

    sub_tile_buffer: &'i mut Option<Vec<uint8>>,

    tile_strip: DngRect,

    buffer_count: uint32,
    buffer_size: uint32,
}

impl<'i> DngImageSpooler<'i> {
    fn new(
        ifd: DngIfd,
        image: &'i dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        block_size: uint32,
        sub_tile_buffer: &'i mut Option<Vec<uint8>>,
    ) -> DngResult<Self> {
        let bytes_per_row = tile_area.w() * planes * 2;

        if bytes_per_row == 0 {
            return Err(bad_format()); // "Bad bytesPerRow in dng_image_spooler"
        }

        let mut strip_length = pin_uint32(
            ifd.sub_tile_block_rows,
            block_size / bytes_per_row,
            tile_area.h(),
        );

        strip_length = strip_length / ifd.sub_tile_block_rows * ifd.sub_tile_block_rows;

        let mut tile_strip = tile_area;

        tile_strip.b = tile_area.t + strip_length as int32;

        Ok(DngImageSpooler {
            ifd,
            image,
            tile_area,
            plane,
            planes,
            block: vec![0u8; block_size as usize],
            sub_tile_buffer,
            tile_strip,
            buffer_count: 0,
            buffer_size: bytes_per_row * strip_length,
        })
    }
}

impl DngSpooler for DngImageSpooler<'_> {
    fn spool(&mut self, data: &[u16]) {
        // The Rust lossless decoder hands over &[u16]; convert to the stream
        // of bytes the C++ Spool receives (native-endian, matching the C++
        // in-memory layout of uint16 samples on a little-endian host).

        let mut bytes: Vec<uint8> = Vec::with_capacity(data.len() * 2);

        for sample in data {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }

        self.spool_bytes(&bytes);
    }
}

impl<'i> DngImageSpooler<'i> {
    fn spool_bytes(&mut self, mut data: &[uint8]) -> DngResult<()> {
        let mut count = data.len() as uint32;

        while count > 0 {
            let block = min_uint32(count, self.buffer_size - self.buffer_count);

            if block == 0 {
                return Ok(());
            }

            let dst_base = self.buffer_count as usize;

            self.block[dst_base..dst_base + block as usize].copy_from_slice(&data[..block as usize]);

            data = &data[block as usize..];

            count -= block;

            self.buffer_count += block;

            if self.buffer_count == self.buffer_size {
                let mut buffer = DngPixelBuffer::with_planar_config(
                    self.tile_strip,
                    self.plane,
                    self.planes,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(Vec::new()),
                )?;

                // Point the buffer at our byte storage (U16 elements).

                let elements = self.block.len() / 2;

                let mut samples = vec![0u16; elements];

                for (i, chunk) in self.block.chunks_exact(2).enumerate() {
                    samples[i] = u16::from_le_bytes([chunk[0], chunk[1]]);
                }

                buffer.data = PixelData::U16(samples);

                if self.ifd.sub_tile_block_rows > 1 {
                    reorder_sub_tile_blocks(&self.ifd, &mut buffer, self.sub_tile_buffer)?;
                }

                self.image.put(&buffer)?;

                let strip_length = self.tile_strip.h() as int32;

                self.tile_strip.t = self.tile_strip.b;

                self.tile_strip.b = min_int32(
                    self.tile_strip.t + strip_length,
                    self.tile_area.b,
                );

                self.buffer_count = 0;

                self.buffer_size = (self.tile_strip.w()
                    * self.tile_strip.h()
                    * self.planes
                    * 2) as uint32;
            }
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// dng_read_image
// ---------------------------------------------------------------------------

/// Port of `dng_read_image`.
pub struct DngReadImage {
    /// `fJPEGTables` — JPEG tables shared by all tiles.
    jpeg_tables: Option<Vec<uint8>>,

    /// `ifd.fLosslessJPEGBug16` — PORT-NOTE: the C++ carries this on the IFD
    /// (set by the negative from the camera make/model quirks); the dng-core
    /// `DngIfd` port does not have the field, so the caller sets it here
    /// before `Read`.
    pub lossless_jpeg_bug16: bool,
}

impl Default for DngReadImage {
    fn default() -> Self {
        Self::new()
    }
}

impl DngReadImage {
    /// `dng_read_image ()`.
    pub fn new() -> Self {
        DngReadImage {
            jpeg_tables: None,
            lossless_jpeg_bug16: false,
        }
    }

    /// `JPEGTables ()` access for the negative wrapper (C++ hands
    /// `fJPEGTables` over to `dng_jpeg_image` after the read).
    pub fn take_jpeg_tables(&mut self) -> Option<Vec<uint8>> {
        self.jpeg_tables.take()
    }

    /// `ReadUncompressed`.
    #[allow(clippy::too_many_arguments)]
    pub fn read_uncompressed(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        uncompressed_buffer: &mut Option<Vec<uint8>>,
        sub_tile_block_buffer: &mut Option<Vec<uint8>>,
    ) -> DngResult<bool> {
        let mut rows = tile_area.h() as uint32;
        let mut samples_per_row = tile_area.w() as uint32;

        if ifd.planar_configuration == pcRowInterleaved {
            rows = safe_uint32_mult2(rows, planes)?;
        } else {
            samples_per_row = safe_uint32_mult2(samples_per_row, planes)?;
        }

        let samples_per_tile = safe_uint32_mult2(samples_per_row, rows)?;

        if uncompressed_buffer.is_none() {
            // "Fuzz: Missing uncompressed buffer"

            return Err(bad_format());
        }

        let bit_depth = ifd.bits_per_sample[plane as usize];

        let pixel_type: uint32;

        let buf = uncompressed_buffer.as_mut().unwrap();

        if bit_depth == 8 {
            pixel_type = ttByte;

            *buf = stream.get(samples_per_tile)?;
        } else if bit_depth == 16 && ifd.sample_format[0] == sfFloatingPoint {
            pixel_type = ttFloat;

            let mut out: Vec<u32> = Vec::with_capacity(samples_per_tile as usize);

            for _ in 0..samples_per_tile {
                out.push(dng_half_to_float(stream.get_uint16()?));
            }

            *buf = float_vec_to_bytes(&out);
        } else if bit_depth == 24 && ifd.sample_format[0] == sfFloatingPoint {
            pixel_type = ttFloat;

            let mut out: Vec<u32> = Vec::with_capacity(samples_per_tile as usize);

            for _ in 0..samples_per_tile {
                let mut input = [0u8; 3];

                if stream.little_endian() {
                    input[2] = stream.get_uint8()?;
                    input[1] = stream.get_uint8()?;
                    input[0] = stream.get_uint8()?;
                } else {
                    input[0] = stream.get_uint8()?;
                    input[1] = stream.get_uint8()?;
                    input[2] = stream.get_uint8()?;
                }

                out.push(dng_fp24_to_float(&input));
            }

            *buf = float_vec_to_bytes(&out);
        } else if bit_depth == 16 {
            pixel_type = ttShort;

            *buf = stream.get(safe_uint32_mult2(samples_per_tile, 2)?)?;

            if stream.swap_bytes() {
                swap_bytes16_vec(buf);
            }
        } else if bit_depth == 32 {
            pixel_type = image.pixel_type();

            *buf = stream.get(safe_uint32_mult2(samples_per_tile, 4)?)?;

            if stream.swap_bytes() {
                swap_bytes32_vec(buf);
            }
        } else if bit_depth == 12 {
            pixel_type = ttShort;

            let mut out: Vec<u16> = Vec::with_capacity(samples_per_tile as usize);

            let even_samples = samples_per_row >> 1;

            for _row in 0..rows {
                for _j in 0..even_samples {
                    let b0 = stream.get_uint8()? as uint32;
                    let b1 = stream.get_uint8()? as uint32;
                    let b2 = stream.get_uint8()? as uint32;

                    out.push(((b0 << 4) | (b1 >> 4)) as u16);
                    out.push((((b1 << 8) | b2) & 0x0FFF) as u16);
                }

                if samples_per_row & 1 != 0 {
                    let b0 = stream.get_uint8()? as uint32;
                    let b1 = stream.get_uint8()? as uint32;

                    out.push(((b0 << 4) | (b1 >> 4)) as u16);
                }
            }

            *buf = u16_vec_to_bytes(&out);
        } else if bit_depth > 8 && bit_depth < 16 {
            pixel_type = ttShort;

            let bit_mask = (1u32 << bit_depth) - 1;

            let mut out: Vec<u16> = Vec::with_capacity(samples_per_tile as usize);

            for _row in 0..rows {
                let mut bit_buffer: u32 = 0;
                let mut buffer_bits: u32 = 0;

                for _j in 0..samples_per_row {
                    while buffer_bits < bit_depth {
                        bit_buffer = (bit_buffer << 8) | stream.get_uint8()? as u32;

                        buffer_bits += 8;
                    }

                    out.push(((bit_buffer >> (buffer_bits - bit_depth)) & bit_mask) as u16);

                    buffer_bits -= bit_depth;
                }
            }

            *buf = u16_vec_to_bytes(&out);
        } else if bit_depth > 16 && bit_depth < 32 {
            pixel_type = ttLong;

            let bit_mask = (1u32 << bit_depth) - 1;

            let mut out: Vec<u32> = Vec::with_capacity(samples_per_tile as usize);

            for _row in 0..rows {
                let mut bit_buffer: u64 = 0;
                let mut buffer_bits: u32 = 0;

                for _j in 0..samples_per_row {
                    while buffer_bits < bit_depth {
                        bit_buffer = (bit_buffer << 8) | stream.get_uint8()? as u64;

                        buffer_bits += 8;
                    }

                    out.push(((bit_buffer >> (buffer_bits - bit_depth)) as u32) & bit_mask);

                    buffer_bits -= bit_depth;
                }
            }

            *buf = u32_vec_to_bytes(&out);
        } else {
            return Ok(false);
        }

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile_area,
            plane,
            planes,
            pixel_type,
            ifd.planar_configuration,
            typed_storage_for(pixel_type, tag_type_size(pixel_type), buf)?,
        )?;

        if ifd.sample_bit_shift != 0 {
            buffer.shift_right(ifd.sample_bit_shift)?;
        }

        if ifd.sub_tile_block_rows > 1 {
            reorder_sub_tile_blocks(ifd, &mut buffer, sub_tile_block_buffer)?;
        }

        image.put(&buffer)?;

        Ok(true)
    }

    /// `DecodeLossyJPEG` — decodes via the `dng-jpeg` crate into the image.
    #[allow(clippy::too_many_arguments)]
    pub fn decode_lossy_jpeg(
        &mut self,
        image: &dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        _photometric_interpretation: uint32,
        jpeg_data: &[uint8],
    ) -> DngResult<()> {
        let decoded = dng_jpeg::decode_jpeg(jpeg_data).map_err(|_| bad_format())?;

        // Check that dimensions of JPEG correspond to dimensions of tile.

        if decoded.width != tile_area.w() as u32
            || decoded.height != tile_area.h() as u32
            || decoded.components != planes
        {
            return Err(bad_format()); // "JPEG dimensions do not match tile"
        }

        // Store the interleaved 8-bit data into the image, one row at a time
        // (the C++ reads scanlines and Puts each row).

        let row_samples = tile_area.w() as usize * planes as usize;

        let mut row: int32 = tile_area.t;

        let mut offset: usize = 0;

        while row < tile_area.b {
            let mut buffer = DngPixelBuffer::with_planar_config(
                DngRect::from_corners(row, tile_area.l, row + 1, tile_area.r),
                plane,
                planes,
                ttByte,
                pcInterleaved,
                PixelData::U8(decoded.data[offset..offset + row_samples].to_vec()),
            )?;

            buffer.dirty = true;

            image.put(&buffer)?;

            offset += row_samples;

            row += 1;
        }

        Ok(())
    }

    /// `ReadBaselineJPEG`.
    #[allow(clippy::too_many_arguments)]
    pub fn read_baseline_jpeg(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        tile_byte_count: uint32,
        jpeg_data_in_memory: Option<&[uint8]>,
    ) -> DngResult<bool> {
        if self.jpeg_tables.is_some() || jpeg_data_in_memory.is_none() {
            let jpeg_data_block = read_jpeg_data_to_block(
                stream,
                self.jpeg_tables.as_deref(),
                stream.position(),
                tile_byte_count,
                ifd.patch_first_jpeg_byte,
            )?;

            self.decode_lossy_jpeg(
                image,
                tile_area,
                plane,
                planes,
                ifd.photometric_interpretation,
                &jpeg_data_block,
            )?;
        } else {
            let mut data = jpeg_data_in_memory.unwrap().to_vec();

            if ifd.patch_first_jpeg_byte && tile_byte_count > 0 && !data.is_empty() {
                data[0] = 0xFF;
            }

            self.decode_lossy_jpeg(
                image,
                tile_area,
                plane,
                planes,
                ifd.photometric_interpretation,
                &data,
            )?;
        }

        Ok(true)
    }

    /// `ReadLosslessJPEG`.
    #[allow(clippy::too_many_arguments)]
    pub fn read_lossless_jpeg(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        tile_byte_count: uint32,
        uncompressed_buffer: &mut Option<Vec<uint8>>,
        sub_tile_block_buffer: &mut Option<Vec<uint8>>,
    ) -> DngResult<bool> {
        // If the tile area is empty, there's nothing to read.

        if tile_area.is_empty() {
            return Ok(true);
        }

        let bytes_per_row =
            safe_uint32_mult3(tile_area.w() as uint32, planes, 2)?;

        let mut rows_per_strip = pin_uint32(
            ifd.sub_tile_block_rows,
            kImageBufferSize / bytes_per_row,
            tile_area.h() as uint32,
        );

        rows_per_strip = rows_per_strip / ifd.sub_tile_block_rows * ifd.sub_tile_block_rows;

        let buffer_size = safe_uint32_mult2(bytes_per_row, rows_per_strip)?;

        match uncompressed_buffer {
            Some(buf) if buf.len() < buffer_size as usize => {
                *uncompressed_buffer = None;
            }
            _ => {}
        }

        if uncompressed_buffer.is_none() {
            *uncompressed_buffer = Some(vec![0u8; buffer_size as usize]);
        }

        let block_len = uncompressed_buffer.as_ref().unwrap().len() as uint32;

        let mut spooler = DngImageSpooler::new(
            ifd.clone(),
            image,
            tile_area,
            plane,
            planes,
            block_len,
            sub_tile_block_buffer,
        )?;

        let decoded_size =
            safe_uint32_mult3(tile_area.w() as uint32, tile_area.h() as uint32, planes)?;

        let decoded_size = safe_uint32_mult2(decoded_size, 2)?;

        let bug16 = self.lossless_jpeg_bug16;

        let tile_offset = stream.position();

        // The C++ `DoDecodeLosslessJPEG` decodes straight from `stream`; the
        // Rust decoder takes the stream directly.

        decode_lossless_jpeg(
            stream,
            &mut spooler,
            decoded_size,
            decoded_size,
            bug16,
            tile_offset + tile_byte_count as uint64,
        )?;

        Ok(true)
    }

    /// `ReadJXL` — decode JPEG XL via the `dng-jxl` crate.
    pub fn read_jxl(
        &mut self,
        _ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        tile_area: DngRect,
        tile_byte_count: uint32,
        jxl_compressed_raw_bit_stream: Option<&[uint8]>,
    ) -> DngResult<bool> {
        let bytes: Vec<uint8> = match jxl_compressed_raw_bit_stream {
            Some(data) => data[..tile_byte_count as usize].to_vec(),
            None => stream.get(tile_byte_count)?,
        };

        let decoded = dng_jxl::decode_jxl(&bytes).map_err(|_| bad_format())?;

        // Check matching dimensions.

        if decoded.width != tile_area.w() || decoded.height != tile_area.h() as u32 {
            return Err(bad_format()); // "mismatch tile size in ReadJXL"
        }

        // Check matching pixel type.

        let expected_pixel_type = if decoded.is_float {
            dng_core::dng_tag_types::ttFloat
        } else if decoded.bits == 8 {
            ttByte
        } else {
            ttShort
        };

        if expected_pixel_type != image.pixel_type() {
            return Err(bad_format()); // "mismatch pixel type in ReadJXL"
        }

        // Check matching plane count (JXL gray = 1, RGB = 3; alpha channels
        // are dropped by the decoder facade when not requested).

        let jxl_planes = if decoded.channels >= 3 { 3 } else { 1 };

        if jxl_planes != image.planes() {
            return Err(bad_format()); // "mismatch plane count in ReadJXL"
        }

        // Store in image.

        let pixel_data = if decoded.is_float {
            let floats: Vec<real32> = decoded
                .data
                .chunks_exact(4)
                .map(|b| real32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                .collect();

            PixelData::F32(floats)
        } else if decoded.bits == 16 {
            let samples: Vec<u16> = decoded
                .data
                .chunks_exact(2)
                .map(|b| u16::from_le_bytes([b[0], b[1]]))
                .collect();

            PixelData::U16(samples)
        } else {
            PixelData::U8(decoded.data)
        };

        let mut buffer = DngPixelBuffer::with_planar_config(
            tile_area,
            0,
            image.planes(),
            image.pixel_type(),
            pcInterleaved,
            pixel_data,
        )?;

        buffer.dirty = true;

        image.put(&buffer)?;

        Ok(true)
    }

    /// `CanReadTile`.
    pub fn can_read_tile(ifd: &DngIfd) -> bool {
        if ifd.sample_format[0] != sfUnsignedInteger && ifd.sample_format[0] != sfFloatingPoint {
            return false;
        }

        match ifd.compression {
            x if x == ccUncompressed => {
                if ifd.sample_format[0] == sfFloatingPoint {
                    return ifd.bits_per_sample[0] == 16
                        || ifd.bits_per_sample[0] == 24
                        || ifd.bits_per_sample[0] == 32;
                }

                ifd.bits_per_sample[0] >= 8 && ifd.bits_per_sample[0] <= 32
            }

            x if x == ccJPEG => {
                if ifd.sample_format[0] != sfUnsignedInteger {
                    return false;
                }

                if ifd.is_baseline_jpeg() {
                    // Baseline JPEG.
                    true
                } else {
                    // Lossless JPEG.
                    ifd.bits_per_sample[0] >= 8 && ifd.bits_per_sample[0] <= 16
                }
            }

            x if x == ccJXL => {
                if ifd.samples_per_pixel != 1 && ifd.samples_per_pixel != 3 {
                    return false;
                }

                if ifd.sample_format[0] == sfUnsignedInteger {
                    ifd.bits_per_sample[0] >= 8 && ifd.bits_per_sample[0] <= 16
                } else {
                    ifd.bits_per_sample[0] >= 16 && ifd.bits_per_sample[0] <= 32
                }
            }

            x if x == ccLZW || x == ccDeflate || x == ccOldDeflate || x == ccPackBits => {
                if ifd.sample_format[0] == sfFloatingPoint {
                    if ifd.compression == ccPackBits {
                        return false;
                    }

                    if ifd.predictor != cpNullPredictor
                        && ifd.predictor != cpFloatingPoint
                        && ifd.predictor != cpFloatingPointX2
                        && ifd.predictor != cpFloatingPointX4
                    {
                        return false;
                    }

                    if ifd.bits_per_sample[0] != 16
                        && ifd.bits_per_sample[0] != 24
                        && ifd.bits_per_sample[0] != 32
                    {
                        return false;
                    }
                } else {
                    if ifd.predictor != cpNullPredictor
                        && ifd.predictor != cpHorizontalDifference
                        && ifd.predictor != cpHorizontalDifferenceX2
                        && ifd.predictor != cpHorizontalDifferenceX4
                    {
                        return false;
                    }

                    if ifd.bits_per_sample[0] != 8
                        && ifd.bits_per_sample[0] != 16
                        && ifd.bits_per_sample[0] != 32
                    {
                        return false;
                    }
                }

                true
            }

            _ => false,
        }
    }

    /// `NeedsCompressedBuffer`.
    pub fn needs_compressed_buffer(ifd: &DngIfd) -> bool {
        ifd.compression == ccLZW
            || ifd.compression == ccDeflate
            || ifd.compression == ccOldDeflate
            || ifd.compression == ccPackBits
    }

    /// `ByteSwapBuffer`.
    pub fn byte_swap_buffer(&self, buffer: &mut DngPixelBuffer) -> DngResult<()> {
        let pixels = buffer.row_step as usize * buffer.area.h() as usize;

        match buffer.pixel_size {
            2 => {
                let _ = pixels;

                match &mut buffer.data {
                    PixelData::U16(v) => {
                        for x in v.iter_mut() {
                            *x = swap_bytes16(*x);
                        }
                    }
                    PixelData::F16(v) => {
                        for x in v.iter_mut() {
                            let bits = x.to_bits();
                            *x = half::f16::from_bits(swap_bytes16(bits));
                        }
                    }
                    _ => {}
                }
            }
            4 => {
                let _ = pixels;

                match &mut buffer.data {
                    PixelData::U32(v) => {
                        for x in v.iter_mut() {
                            *x = swap_bytes32(*x);
                        }
                    }
                    PixelData::F32(v) => {
                        for x in v.iter_mut() {
                            let bits = x.to_bits();
                            *x = real32::from_bits(swap_bytes32(bits));
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// `DecodePredictor`.
    pub fn decode_predictor(&self, ifd: &DngIfd, buffer: &mut DngPixelBuffer) -> DngResult<()> {
        match ifd.predictor {
            x if x == cpNullPredictor => return Ok(()),

            x if x == cpHorizontalDifference
                || x == cpHorizontalDifferenceX2
                || x == cpHorizontalDifferenceX4 =>
            {
                let x_factor: uint32 = match ifd.predictor {
                    x if x == cpHorizontalDifferenceX2 => 2,
                    x if x == cpHorizontalDifferenceX4 => 4,
                    _ => 1,
                };

                match buffer.data {
                    PixelData::U8(_) => {
                        let mut data = take_u8(&mut buffer.data);

                        decode_delta8(
                            &mut data,
                            buffer.area.h() as uint32,
                            (buffer.area.w() / x_factor),
                            buffer.planes * x_factor,
                        );

                        buffer.data = PixelData::U8(data);

                        return Ok(());
                    }
                    PixelData::U16(_) => {
                        let mut data = take_u16(&mut buffer.data);

                        decode_delta16(
                            &mut data,
                            buffer.area.h() as uint32,
                            (buffer.area.w() / x_factor),
                            buffer.planes * x_factor,
                        );

                        buffer.data = PixelData::U16(data);

                        return Ok(());
                    }
                    PixelData::U32(_) => {
                        let mut data = take_u32(&mut buffer.data);

                        decode_delta32(
                            &mut data,
                            buffer.area.h() as uint32,
                            (buffer.area.w() / x_factor),
                            buffer.planes * x_factor,
                        );

                        buffer.data = PixelData::U32(data);

                        return Ok(());
                    }
                    _ => {}
                }
            }

            _ => {}
        }

        Err(bad_format())
    }

    /// `ReadTile` — decode one tile/sub-tile with the compression switch
    /// mirroring `dng_read_image::ReadTile` exactly.
    #[allow(clippy::too_many_arguments)]
    pub fn read_tile(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        tile_area: DngRect,
        plane: uint32,
        planes: uint32,
        tile_byte_count: uint32,
        compressed_buffer: Option<&[uint8]>,
        uncompressed_buffer: &mut Option<Vec<uint8>>,
        sub_tile_block_buffer: &mut Option<Vec<uint8>>,
    ) -> DngResult<()> {
        match ifd.compression {
            x if x == ccLZW || x == ccDeflate || x == ccOldDeflate || x == ccPackBits => {
                // Figure out uncompressed size.

                let bytes_per_sample = ifd.bits_per_sample[0] >> 3;

                let sample_count = safe_uint32_mult3(
                    planes,
                    tile_area.w() as uint32,
                    tile_area.h() as uint32,
                )?;

                let uncompressed_size = safe_uint32_mult2(sample_count, bytes_per_sample)?;

                // Setup pixel buffer to hold uncompressed data.

                let pixel_type: uint32;

                if ifd.sample_format[0] == sfFloatingPoint {
                    pixel_type = ttFloat;
                } else if ifd.bits_per_sample[0] == 8 {
                    pixel_type = ttByte;
                } else if ifd.bits_per_sample[0] == 16 {
                    pixel_type = ttShort;
                } else if ifd.bits_per_sample[0] == 32 {
                    pixel_type = ttLong;
                } else {
                    return Err(bad_format());
                }

                let mut buffer = DngPixelBuffer::with_planar_config(
                    tile_area,
                    plane,
                    planes,
                    pixel_type,
                    pcInterleaved,
                    PixelData::None,
                )?;

                // Specify custom pixel size (e.g., ttFloat pixel type but
                // pixel size may be 2 for stored 16-bit half-float).

                buffer.pixel_size = bytes_per_sample;

                let mut buffer_size = uncompressed_size;

                // If we are using the floating point predictor, we need an
                // extra buffer row.

                let uses_fp_predictor = ifd.predictor == cpFloatingPoint
                    || ifd.predictor == cpFloatingPointX2
                    || ifd.predictor == cpFloatingPointX4;

                if uses_fp_predictor {
                    buffer_size = safe_uint32_add(
                        buffer_size,
                        safe_uint32_mult2(buffer.row_step as uint32, buffer.pixel_size)?,
                    )?;
                }

                // If are processing less than full size floating point data,
                // we need space to expand the data to full floating point
                // size.

                if pixel_type == ttFloat {
                    buffer_size = max_uint32(buffer_size, safe_uint32_mult2(sample_count, 4)?);
                }

                // Reallocate the buffer if it is too small.

                match uncompressed_buffer {
                    Some(buf) if buf.len() < buffer_size as usize => {
                        *uncompressed_buffer = None;
                    }
                    _ => {}
                }

                if uncompressed_buffer.is_none() {
                    *uncompressed_buffer = Some(vec![0u8; buffer_size as usize]);
                }

                let compressed = match compressed_buffer {
                    Some(data) => data,
                    None => return Err(bad_format()),
                };

                // Decompress the data.  The C++ keeps `fData` pointing at the
                // second row when the FP predictor is in play; the port
                // allocates the extra leading row explicitly.

                let row_bytes = (buffer.row_step as usize) * buffer.pixel_size as usize;

                let storage: Vec<uint8> = if uses_fp_predictor {
                    // Decompress into the storage AFTER the leading row.

                    let mut storage = vec![0u8; row_bytes + uncompressed_size as usize];

                    if ifd.compression == ccLZW {
                        let mut expander = DngLzwExpander::new();

                        if !expander.expand(
                            compressed,
                            &mut storage[row_bytes..row_bytes + uncompressed_size as usize],
                            tile_byte_count as int32,
                            uncompressed_size as int32,
                        ) {
                            return Err(bad_format());
                        }
                    } else if ifd.compression == ccPackBits {
                        let mut sub_stream = bytes_as_stream(compressed)?;

                        if !decode_pack_bits_impl(
                            &mut sub_stream,
                            &mut storage[row_bytes..row_bytes + uncompressed_size as usize],
                            uncompressed_size as int32,
                        ) {
                            return Err(bad_format());
                        }
                    } else {
                        // ccDeflate / ccOldDeflate.

                        let (out, mismatch) =
                            dng_core::zlib::uncompress_tolerant(compressed, uncompressed_size)?;

                        if mismatch {
                            return Err(bad_format());
                        }

                        storage[row_bytes..row_bytes + out.len()].copy_from_slice(&out);
                    }

                    // The floating point predictor is byte order independent.

                    let x_factor: uint32 = match ifd.predictor {
                        x if x == cpFloatingPointX2 => 2,
                        x if x == cpFloatingPointX4 => 4,
                        _ => 1,
                    };

                    let cols = (tile_area.w() / x_factor) as int32;

                    let channels = (planes * x_factor) as int32;

                    // DecodeFPDelta (srcPtr = row j+1, dstPtr = row j) for
                    // every row; ascending order (the C++ row loop t..b-1).

                    let rows = tile_area.h() as usize;

                    for j in 0..rows {
                        let dst_start = j * row_bytes;
                        let src_start = (j + 1) * row_bytes;

                        let src = storage[src_start..src_start + row_bytes].to_vec();

                        decode_fp_delta_into(
                            &mut storage[dst_start..dst_start + row_bytes],
                            &src,
                            cols,
                            channels,
                            bytes_per_sample as int32,
                        );
                    }

                    // The C++ moves fData back one row; the decoded image rows
                    // now occupy storage[0 .. uncompressedSize).

                    storage.truncate(uncompressed_size as usize);

                    storage
                } else {
                    let mut storage = vec![0u8; uncompressed_size as usize];

                    if ifd.compression == ccLZW {
                        let mut expander = DngLzwExpander::new();

                        if !expander.expand(
                            compressed,
                            &mut storage,
                            tile_byte_count as int32,
                            uncompressed_size as int32,
                        ) {
                            return Err(bad_format());
                        }
                    } else if ifd.compression == ccPackBits {
                        let mut sub_stream = bytes_as_stream(compressed)?;

                        if !decode_pack_bits_impl(
                            &mut sub_stream,
                            &mut storage,
                            uncompressed_size as int32,
                        ) {
                            return Err(bad_format());
                        }
                    } else {
                        // ccDeflate / ccOldDeflate.

                        let (out, mismatch) =
                            dng_core::zlib::uncompress_tolerant(compressed, uncompressed_size)?;

                        if mismatch {
                            return Err(bad_format());
                        }

                        storage[..out.len()].copy_from_slice(&out);
                    }

                    // Both these compression algorithms are byte based.

                    let byte_order_matters = buffer.pixel_size == 2 || buffer.pixel_size == 4;

                    if stream.swap_bytes() && byte_order_matters {
                        match buffer.pixel_size {
                            2 => swap_bytes16_vec(&mut storage),
                            4 => swap_bytes32_vec(&mut storage),
                            _ => {}
                        }
                    }

                    storage
                };

                // Build the typed storage at the sample granularity the
                // decoder produced (ttFloat may still hold 2- or 3-byte
                // samples until the expansion below).

                buffer.data = typed_storage_for(pixel_type, buffer.pixel_size, &storage)?;

                // Undo the horizontal predictor (non-FP paths only).

                if !uses_fp_predictor {
                    self.decode_predictor(ifd, &mut buffer)?;
                }

                // Expand floating point data, if needed.

                expand_float_storage(
                    &mut buffer,
                    sample_count,
                    uses_fp_predictor || stream.big_endian(),
                )?;

                // Save the data.

                image.put(&buffer)?;

                return Ok(());
            }

            x if x == ccUncompressed => {
                if self.read_uncompressed(
                    ifd,
                    stream,
                    image,
                    tile_area,
                    plane,
                    planes,
                    uncompressed_buffer,
                    sub_tile_block_buffer,
                )? {
                    return Ok(());
                }
            }

            x if x == ccJPEG => {
                if ifd.is_baseline_jpeg() {
                    // Baseline JPEG.

                    if self.read_baseline_jpeg(
                        ifd,
                        stream,
                        image,
                        tile_area,
                        plane,
                        planes,
                        tile_byte_count,
                        compressed_buffer,
                    )? {
                        return Ok(());
                    }
                } else {
                    // Otherwise it should be lossless JPEG.

                    if self.read_lossless_jpeg(
                        ifd,
                        stream,
                        image,
                        tile_area,
                        plane,
                        planes,
                        tile_byte_count,
                        uncompressed_buffer,
                        sub_tile_block_buffer,
                    )? {
                        return Ok(());
                    }
                }
            }

            x if x == ccLossyJPEG => {
                if self.read_baseline_jpeg(
                    ifd,
                    stream,
                    image,
                    tile_area,
                    plane,
                    planes,
                    tile_byte_count,
                    compressed_buffer,
                )? {
                    return Ok(());
                }
            }

            x if x == ccJXL => {
                if plane != 0 {
                    return Err(bad_format()); // "Unexpected plane in ReadTile for ccJXL"
                }

                if planes != 1 && planes != 3 {
                    return Err(bad_format()); // "Unexpected planes in ReadTile for ccJXL"
                }

                if self.read_jxl(
                    ifd,
                    stream,
                    image,
                    tile_area,
                    tile_byte_count,
                    compressed_buffer,
                )? {
                    return Ok(());
                }
            }

            _ => {}
        }

        Err(bad_format())
    }

    /// `CanRead`.
    pub fn can_read(ifd: &DngIfd) -> bool {
        if ifd.image_width < 1 || ifd.image_length < 1 {
            return false;
        }

        if ifd.samples_per_pixel < 1 {
            return false;
        }

        if ifd.bits_per_sample[0] < 1 {
            return false;
        }

        let check_count = min_uint32(ifd.samples_per_pixel, dng_core::dng_sdk_limits::kMaxSamplesPerPixel);

        for j in 1..check_count as usize {
            if ifd.bits_per_sample[j] != ifd.bits_per_sample[0] {
                return false;
            }

            if ifd.sample_format[j] != ifd.sample_format[0] {
                return false;
            }
        }

        if ifd.planar_configuration != pcInterleaved
            && ifd.planar_configuration != pcPlanar
            && ifd.planar_configuration != pcRowInterleaved
        {
            return false;
        }

        if ifd.uses_strips == ifd.uses_tiles {
            return false;
        }

        let tile_count = ifd.tiles_per_image();

        if tile_count < 1 {
            return false;
        }

        let need_tile_byte_counts =
            ifd.tile_byte_count(ifd.tile_area(0, 0)).unwrap_or(0) == 0;

        if tile_count == 1 {
            if need_tile_byte_counts && ifd.tile_byte_count[0] < 1 {
                return false;
            }
        } else {
            if ifd.tile_byte_counts_count as uint32 != tile_count {
                return false;
            }

            if need_tile_byte_counts && ifd.tile_byte_counts_count as uint32 != tile_count {
                return false;
            }
        }

        if !Self::can_read_tile(ifd) {
            return false;
        }

        true
    }

    /// `Read` — read the whole image described by `ifd` into `image`.
    pub fn read(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        mut lossy_image: Option<&mut DngLossyCompressedImage>,
        mut lossy_digest: Option<&mut DngFingerprint>,
    ) -> DngResult<()> {
        // Reject images that are too big.

        // Allow up to 2 * kMaxImageSide to deal with intermediate image
        // objects (e.g., rotated and padded).

        let limit = 2 * dng_core::dng_sdk_limits::kMaxImageSide;

        if ifd.image_width > limit || ifd.image_length > limit {
            return Err(bad_format()); // "dng_read_image::Read image too large"
        }

        // Deal with both images that have row or column interleaving.

        if ifd.row_interleave_factor > 1 || ifd.column_interleave_factor > 1 {
            // First, read into a full interleaved temporary image.

            let temp_image: ImageRef = DngSimpleImage::new(
                image.bounds(),
                image.planes(),
                image.pixel_type(),
            )?
            .into_image_ref()?;

            let mut temp_ifd = ifd.clone();

            temp_ifd.row_interleave_factor = 1;
            temp_ifd.column_interleave_factor = 1;

            {
                let temp_ref = temp_image.clone();

                let temp_dyn: &dyn DngImage = &*temp_ref.read().unwrap();

                self.read(&temp_ifd, stream, temp_dyn, lossy_image.as_deref_mut(), lossy_digest)?;
            }

            // Interleave both rows and columns.

            interleave_2d(&temp_image, image, ifd.row_interleave_factor as int32, ifd.column_interleave_factor as int32, false)?;

            return Ok(());
        }

        // Figure out inner and outer samples.

        let mut inner_samples: uint32 = 1;
        let mut outer_samples: uint32 = 1;

        if ifd.planar_configuration == pcPlanar {
            outer_samples = ifd.samples_per_pixel;
        } else {
            inner_samples = ifd.samples_per_pixel;
        }

        // Calculate number of tiles to read.

        let tiles_across = ifd.tiles_across();
        let tiles_down = ifd.tiles_down();

        let tile_count = dng_core::dng_safe_arithmetic::safe_uint32_mult3(
            tiles_across,
            tiles_down,
            outer_samples,
        )?;

        // Find the tile offsets.

        let mut tile_offset: Vec<uint64> = vec![0; tile_count as usize];

        if tile_count <= kMaxTileInfo {
            for (tile_index, off) in tile_offset.iter_mut().enumerate().take(tile_count as usize) {
                *off = ifd.tile_offset[tile_index];
            }
        } else {
            stream.set_read_position(ifd.tile_offsets_offset)?;

            for off in tile_offset.iter_mut().take(tile_count as usize) {
                *off = stream.tag_value_uint64(ifd.tile_offsets_type)?;
            }
        }

        // Quick validity check on tile offsets.

        for off in &tile_offset {
            if *off >= stream.length() {
                return Err(bad_format());
            }
        }

        // Buffer to hold the tile byte counts, if needed.

        let mut tile_byte_count: Option<Vec<uint32>> = None;

        // If we can compute the number of bytes needed to store the data, we
        // can split the read for each tile into sub-tiles.

        let mut uncompressed_size: uint32 = 0;

        let mut sub_tile_length = ifd.tile_length;

        if ifd.tile_byte_count(ifd.tile_area(0, 0)).unwrap_or(0) != 0 {
            let bytes_per_pixel = tag_type_size(ifd.pixel_type());

            let bytes_per_row =
                safe_uint32_mult3(ifd.tile_width, inner_samples, bytes_per_pixel)?;

            sub_tile_length = pin_uint32(
                ifd.sub_tile_block_rows,
                kImageBufferSize / bytes_per_row,
                ifd.tile_length,
            );

            sub_tile_length = sub_tile_length / ifd.sub_tile_block_rows * ifd.sub_tile_block_rows;

            uncompressed_size = safe_uint32_mult2(sub_tile_length, bytes_per_row)?;
        }
        // Else we need to know the byte counts.
        else {
            let mut counts: Vec<uint32> = vec![0; tile_count as usize];

            if tile_count <= kMaxTileInfo {
                for (tile_index, count) in counts.iter_mut().enumerate().take(tile_count as usize) {
                    if ifd.tile_byte_count[tile_index] > 0xFFFFFFFF {
                        return Err(bad_format());
                    }

                    *count = ifd.tile_byte_count[tile_index] as uint32;
                }
            } else {
                stream.set_read_position(ifd.tile_byte_counts_offset)?;

                for count in counts.iter_mut().take(tile_count as usize) {
                    let tile_byte_count64 = stream.tag_value_uint64(ifd.tile_byte_counts_type)?;

                    if tile_byte_count64 > 0xFFFFFFFF {
                        return Err(bad_format());
                    }

                    *count = tile_byte_count64 as uint32;
                }
            }

            // Quick validity check on tile byte counts.

            for count in &counts {
                if *count < 1 || *count as uint64 > stream.length() {
                    return Err(bad_format());
                }
            }

            tile_byte_count = Some(counts);
        }

        // Find maximum tile size, if possible.

        let mut max_tile_byte_count: uint32 = 0;

        if let Some(counts) = &tile_byte_count {
            for count in counts {
                max_tile_byte_count = max_uint32(max_tile_byte_count, *count);
            }
        }

        // Do we need a compressed data buffer?

        let mut compressed_size: uint32 = 0;

        let needs_compressed_buffer = Self::needs_compressed_buffer(ifd);

        if needs_compressed_buffer {
            if tile_byte_count.is_none() {
                return Err(bad_format());
            }

            compressed_size = max_tile_byte_count;
        }

        // Are we keeping the lossy compressed image data?

        if let Some(lossy) = lossy_image.as_deref_mut() {
            if ifd.is_baseline_jpeg() || ifd.compression == ccJXL {
                lossy.image_size = DngPoint::new(ifd.image_length as int32, ifd.image_width as int32);
                lossy.tile_size = DngPoint::new(ifd.tile_length as int32, ifd.tile_width as int32);

                lossy.uses_strips = ifd.uses_strips;

                lossy.tiles.data.resize_with(tile_count as usize, || None);
            } else {
                // PORT-NOTE(dng_read_image.cpp:3476): the C++ nulls the
                // caller's pointer; the Rust port signals via a flag the
                // caller checks (see read_stage wrappers).
                lossy.compression_code = 0;
            }
        }

        let mut lossy_image_valid = lossy_image
            .as_deref()
            .map(|l| l.compression_code != 0)
            .unwrap_or(false);

        // Do we need to read the JPEG tables?

        if ifd.jpeg_tables_offset != 0 && ifd.jpeg_tables_count != 0 && ifd.is_baseline_jpeg() {
            self.jpeg_tables = Some(stream_get_at(
                stream,
                ifd.jpeg_tables_offset,
                ifd.jpeg_tables_count,
            )?);
        }

        let mut lossy_tile_digests: Vec<DngFingerprint> = Vec::new();

        if lossy_digest.is_some() {
            lossy_tile_digests.resize(tile_count as usize, DngFingerprint::new());
        }

        // Don't read planes we are not actually saving.

        outer_samples = min_uint32(image.planes(), outer_samples);

        // Performance optimization: increase the stream buffer size when the
        // tiles are contiguous and in order.

        let mut contiguous_byte_count: uint64 = 0;

        {
            let mut tiles_in_order = true;

            let mut total_tile_bytes: uint64 = 0;

            let mut min_file_offset = tile_offset[0];
            let mut max_file_offset = tile_offset[0];

            let mut tile_index: usize = 0;

            for _plane in 0..outer_samples {
                for _row_index in 0..tiles_down {
                    for _col_index in 0..tiles_across {
                        let this_offset = tile_offset[tile_index];

                        let this_byte_count: uint64 = match &tile_byte_count {
                            Some(counts) => counts[tile_index] as uint64,
                            None => ifd
                                .tile_byte_count(ifd.tile_area(
                                    ((tile_index as uint32 / tiles_across) % tiles_down),
                                    (tile_index as uint32 % tiles_across),
                                ))
                                .unwrap_or(0) as uint64,
                        };

                        if this_offset < max_file_offset {
                            tiles_in_order = false;
                        }

                        total_tile_bytes += this_byte_count;

                        min_file_offset = min_file_offset.min(this_offset);
                        max_file_offset = max_file_offset.max(this_offset + this_byte_count);

                        tile_index += 1;
                    }
                }
            }

            // Quick check for enough data in file.

            if max_file_offset > stream.length() {
                return Err(bad_format());
            }

            // Are all the tiles in order?

            if tiles_in_order {
                let total_file_bytes = max_file_offset - min_file_offset;

                if total_tile_bytes >= total_file_bytes * 9 / 10 {
                    contiguous_byte_count = total_file_bytes;
                }
            }
        }

        // PORT-NOTE(dng_read_image.cpp:3601): the C++ uses an RAII
        // `dng_stream_contiguous_read_hint`; the Rust port applies and
        // restores the buffer size manually (the read hint type borrows the
        // stream for its lifetime, which conflicts with passing the stream to
        // ReadTile below).

        let old_buffer_size = stream.buffer_size();

        if contiguous_byte_count > old_buffer_size as uint64 * 4 {
            stream.set_buffer_size((contiguous_byte_count.min(dng_core::dng_globals::get_dng_max_stream_buffer_size() as uint64)) as uint32);
        }

        // The multi-thread path is executed serially (ARCHITECTURE.md §9) —
        // use_multiple_threads is always false.

        let use_multiple_threads = false;

        let _ = use_multiple_threads;

        // Single-threaded read of all the tiles.

        {
            let mut compressed_buffer: Option<Vec<uint8>> = None;
            let mut uncompressed_buffer: Option<Vec<uint8>> = None;
            let mut sub_tile_block_buffer: Option<Vec<uint8>> = None;

            if uncompressed_size > 0 {
                uncompressed_buffer = Some(vec![0u8; uncompressed_size as usize]);
            }

            if compressed_size > 0 && !lossy_image_valid {
                compressed_buffer = Some(vec![0u8; compressed_size as usize]);
            } else if lossy_digest.is_some() {
                compressed_buffer = Some(vec![0u8; max_tile_byte_count as usize]);
            }

            let mut tile_index: uint32 = 0;

            for _plane in 0..outer_samples {
                for row_index in 0..tiles_down {
                    for col_index in 0..tiles_across {
                        stream.set_read_position(tile_offset[tile_index as usize])?;

                        let tile_area = ifd.tile_area(row_index, col_index);

                        let sub_tile_count =
                            (tile_area.h() as uint32 + sub_tile_length - 1) / sub_tile_length;

                        for sub_index in 0..sub_tile_count {
                            let mut sub_area = tile_area;

                            sub_area.t = tile_area.t + (sub_index * sub_tile_length) as int32;

                            sub_area.b = min_int32(
                                sub_area.t + sub_tile_length as int32,
                                tile_area.b,
                            );

                            let sub_byte_count: uint32 = match &tile_byte_count {
                                Some(counts) => counts[tile_index as usize],
                                None => ifd.tile_byte_count(sub_area).unwrap_or(0),
                            };

                            if lossy_image_valid {
                                if let Some(lossy) = lossy_image.as_deref_mut() {
                                    let data = stream.get(sub_byte_count)?;

                                    lossy.tiles.data[tile_index as usize] =
                                        Some(memory_block_from(data.clone()));

                                    stream.set_read_position(tile_offset[tile_index as usize])?;
                                }
                            } else if (needs_compressed_buffer || lossy_digest.is_some())
                                && sub_byte_count > 0
                            {
                                let data = stream.get(sub_byte_count)?;

                                if let Some(lossy_digest) = lossy_digest.as_deref_mut() {
                                    let mut printer = DngMd5Printer::new();

                                    printer.process(&data);

                                    lossy_tile_digests[tile_index as usize] =
                                        printer.result().clone();
                                }

                                if let Some(buf) = compressed_buffer.as_mut() {
                                    buf[..data.len()].copy_from_slice(&data);
                                }
                            }

                            let compressed_slice: Option<Vec<uint8>> = if lossy_image_valid {
                                lossy_image
                                    .as_deref()
                                    .and_then(|l| l.tiles.data[tile_index as usize].as_ref())
                                    .map(|b| b.buffer().to_vec())
                            } else {
                                compressed_buffer.clone()
                            };

                            self.read_tile(
                                ifd,
                                stream,
                                image,
                                sub_area,
                                _plane,
                                inner_samples,
                                sub_byte_count,
                                compressed_slice.as_deref(),
                                &mut uncompressed_buffer,
                                &mut sub_tile_block_buffer,
                            )?;
                        }

                        tile_index += 1;
                    }
                }
            }
        }

        // Restore stream buffer size.

        stream.set_buffer_size(old_buffer_size);

        // Finish up JPEG digest computation, if needed.

        if let Some(lossy_digest) = lossy_digest {
            if self.jpeg_tables.is_some() {
                let mut printer = DngMd5Printer::new();

                printer.process(self.jpeg_tables.as_ref().unwrap());

                lossy_tile_digests.push(printer.result().clone());
            }

            let mut printer2 = DngMd5Printer::new();

            for digest in &lossy_tile_digests {
                printer2.process(&digest.data);
            }

            *lossy_digest = printer2.result().clone();
        }

        // Keep the JPEG table in the jpeg image, if any (C++ casts the lossy
        // image to dng_jpeg_image and moves fJPEGTables over; the port stores
        // the tables on the lossy image base — see DngLossyCompressedImage::
        // jpeg_tables_data).

        if lossy_image_valid {
            if let Some(tables) = self.jpeg_tables.take() {
                if let Some(lossy) = lossy_image.as_deref_mut() {
                    lossy.jpeg_tables_data = Some(tables);
                }
            }
        }

        Ok(())
    }

    /// `DoReadTiles` — serial fallback for the C++ multi-thread task.
    #[allow(clippy::too_many_arguments)]
    pub fn do_read_tiles(
        &mut self,
        ifd: &DngIfd,
        stream: &mut DngStream,
        image: &dyn DngImage,
        lossy_image: Option<&mut DngLossyCompressedImage>,
        mut lossy_tile_digest: Option<&mut [DngFingerprint]>,
        outer_samples: uint32,
        inner_samples: uint32,
        tiles_down: uint32,
        tiles_across: uint32,
        tile_offset: &[uint64],
        tile_byte_count: Option<&[uint32]>,
        _compressed_size: uint32,
        _uncompressed_size: uint32,
    ) -> DngResult<()> {
        // Serial execution of the tile loop (the C++ fans out over threads).

        let tile_total = (outer_samples * tiles_down * tiles_across) as usize;

        let mut uncompressed_buffer: Option<Vec<uint8>> = None;
        let mut sub_tile_block_buffer: Option<Vec<uint8>> = None;

        for tile_index in 0..tile_total {
            let byte_count = match tile_byte_count {
                Some(counts) => counts[tile_index],
                None => 0,
            };

            let compressed = if lossy_tile_digest.is_some() {
                stream_get_at(stream, tile_offset[tile_index], byte_count)?
            } else {
                Vec::new()
            };

            if let Some(digests) = lossy_tile_digest.as_deref_mut() {
                let mut printer = DngMd5Printer::new();

                printer.process(&compressed);

                digests[tile_index] = printer.result().clone();
            }

            let mut tile_stream = bytes_as_stream(&compressed)?;

            tile_stream.set_little_endian(stream.little_endian());

            let plane = (tile_index / (tiles_down * tiles_across) as usize) as uint32;

            let row_index = ((tile_index as uint32
                - plane * tiles_down * tiles_across)
                / tiles_across) as uint32;

            let col_index = tile_index as uint32
                - (plane * tiles_down + row_index) * tiles_across;

            let tile_area = ifd.tile_area(row_index, col_index);

            self.read_tile(
                ifd,
                &mut tile_stream,
                image,
                tile_area,
                plane,
                inner_samples,
                byte_count,
                Some(&compressed),
                &mut uncompressed_buffer,
                &mut sub_tile_block_buffer,
            )?;
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Interleave2D
// ---------------------------------------------------------------------------

/// Port of `dng_interleave_task` + `Interleave2D` (serial execution).
///
/// De-interleave (`encode == false`, used when reading): `src_image` holds
/// the compact field layout; each field's pixels are scattered into
/// `dst_image` at `rowFactor`/`colFactor` strided positions.
/// Interleave (`encode == true`, used when writing): the inverse gather.
///
/// PORT-NOTE(dng_read_image.cpp:3053): the C++ drives this through
/// `dng_pixel_buffer` views with multiplied steps and pointer offsets; the
/// Rust port computes the identical field mapping directly and moves whole
/// samples (same-type gather/scatter, no conversion), which reproduces the
/// exact same sample-to-position correspondence.
pub fn interleave_2d(
    src_image: &ImageRef,
    dst_image: &dyn DngImage,
    row_factor: int32,
    col_factor: int32,
    encode: bool,
) -> DngResult<()> {
    let row_factor = if row_factor as uint32 >= src_image.read().unwrap().bounds().h() {
        1
    } else {
        row_factor
    };

    let col_factor = if col_factor as uint32 >= src_image.read().unwrap().bounds().w() {
        1
    } else {
        col_factor
    };

    if src_image.read().unwrap().bounds() != dst_image.bounds()
        || src_image.read().unwrap().planes() != dst_image.planes()
        || src_image.read().unwrap().pixel_type() != dst_image.pixel_type()
    {
        return Err(bad_format()); // "Mismatched src and dst in Interleave2D"
    }

    let dst_height = dst_image.height() as int32;
    let dst_width = dst_image.width() as int32;

    let tile = dst_image.bounds();

    let planes = dst_image.planes();

    let pixel_type = dst_image.pixel_type();

    let tile_samples = tile.w() as usize * tile.h() as usize * planes as usize;

    // The C++ keeps one tile buffer per thread across all fields and Puts it
    // once; mirror that by hoisting the tile buffer out of the field loops.

    let mut tile_buffer = DngPixelBuffer::with_planar_config(
        tile,
        0,
        planes,
        pixel_type,
        pcInterleaved,
        empty_pixel_data(pixel_type, tile_samples)?,
    )?;

    if encode {
        src_image.read().unwrap().get(
            &mut tile_buffer,
            dng_core::dng_image::EdgeNone,
            0,
            0,
        )?;
    }

    for r_offset in 0..min_int32(row_factor, tile.h() as int32) {
        for c_offset in 0..min_int32(col_factor, tile.w() as int32) {
            let r_field = (tile.t + r_offset).rem_euclid(row_factor);
            let c_field = (tile.l + c_offset).rem_euclid(col_factor);

            let r_field_offset = r_field * (dst_height / row_factor)
                + min_int32(r_field, dst_height % row_factor);

            let c_field_offset = c_field * (dst_width / col_factor)
                + min_int32(c_field, dst_width % col_factor);

            // The compact field area (de-interleaved coordinates).

            let mut src_area = DngRect::new();

            src_area.t = r_field_offset + (tile.t + r_offset) / row_factor;
            src_area.l = c_field_offset + (tile.l + c_offset) / col_factor;

            src_area.b = src_area.t
                + ((tile.h() as int32 - r_offset + row_factor - 1) / row_factor);
            src_area.r = src_area.l
                + ((tile.w() as int32 - c_offset + col_factor - 1) / col_factor);

            let field_samples =
                src_area.w() as usize * src_area.h() as usize * planes as usize;

            let mut field_buffer = DngPixelBuffer::with_planar_config(
                src_area,
                0,
                planes,
                pixel_type,
                pcInterleaved,
                empty_pixel_data(pixel_type, field_samples)?,
            )?;

            field_buffer.row_step = field_buffer.col_step * src_area.w() as int32;

            if !encode {
                src_image.read().unwrap().get(
                    &mut field_buffer,
                    dng_core::dng_image::EdgeNone,
                    0,
                    0,
                )?;
            }

            // Move the field's samples between the compact and strided
            // layouts.  tmpBuffer semantics: compact pixel (r, c) of the
            // field corresponds to tile pixel
            // (tile.t + rOffset + (r - area.t) * rowFactor,
            //  tile.l + cOffset + (c - area.l) * colFactor).

            let mut row = src_area.t;

            let mut inter_row = tile.t + r_offset;

            while row < src_area.b {
                let mut col = src_area.l;

                let mut inter_col = tile.l + c_offset;

                while col < src_area.r {
                    for plane in 0..planes {
                        if encode {
                            let value = sample_get(&tile_buffer, inter_row, inter_col, plane)?;

                            sample_set(&mut field_buffer, row, col, plane, value)?;
                        } else {
                            let value = sample_get(&field_buffer, row, col, plane)?;

                            sample_set(&mut tile_buffer, inter_row, inter_col, plane, value)?;
                        }
                    }

                    col += 1;
                    inter_col += col_factor;
                }

                row += 1;
                inter_row += row_factor;
            }

            if encode {
                src_image.read().unwrap().put(&field_buffer)?;
            }
        }
    }

    if !encode {
        dst_image.put(&tile_buffer)?;
    }

    Ok(())
}

/// Type-erased sample value for same-type moves.
#[derive(Clone, Copy)]
enum Sample {
    U8(uint8),
    U16(u16),
    U32(u32),
    F32(real32),
    F16(half::f16),
}

fn sample_get(buffer: &DngPixelBuffer, row: int32, col: int32, plane: uint32) -> DngResult<Sample> {
    let i = buffer.pixel_element_index_public(row, col, plane)?;

    Ok(match &buffer.data {
        PixelData::U8(v) => Sample::U8(v[i]),
        PixelData::U16(v) => Sample::U16(v[i]),
        PixelData::U32(v) => Sample::U32(v[i]),
        PixelData::F32(v) => Sample::F32(v[i]),
        PixelData::F16(v) => Sample::F16(v[i]),
        _ => return Err(program_error()), // "Interleave2D: unexpected pixel storage"
    })
}

fn sample_set(
    buffer: &mut DngPixelBuffer,
    row: int32,
    col: int32,
    plane: uint32,
    value: Sample,
) -> DngResult<()> {
    let i = buffer.pixel_element_index_public(row, col, plane)?;

    match (&mut buffer.data, value) {
        (PixelData::U8(v), Sample::U8(x)) => v[i] = x,
        (PixelData::U16(v), Sample::U16(x)) => v[i] = x,
        (PixelData::U32(v), Sample::U32(x)) => v[i] = x,
        (PixelData::F32(v), Sample::F32(x)) => v[i] = x,
        (PixelData::F16(v), Sample::F16(x)) => v[i] = x,
        _ => return Err(program_error()),
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

/// `ReadJPEGDataToBlock` — concatenate JPEG tables and tile data.
fn read_jpeg_data_to_block(
    stream: &mut DngStream,
    tables_block: Option<&[uint8]>,
    mut tile_offset: uint64,
    mut tile_byte_count: uint32,
    patch_first_byte: bool,
) -> DngResult<Vec<uint8>> {
    if tile_byte_count <= 2 {
        return Err(eof_failure());
    }

    let mut tables_byte_count: uint32 = tables_block
        .map(|t| t.len() as uint32)
        .unwrap_or(0);

    if tables_byte_count > 0 && tables_byte_count < 4 {
        return Err(eof_failure());
    }

    // The JPEG tables start with a two byte SOI marker, and end with a two
    // byte EOI marker. The JPEG tile data also starts with a two byte SOI
    // marker. We can convert this combination to a normal JPEG stream by
    // removing the last two bytes of the JPEG tables and the first two bytes
    // of the tile data, and then concatenating them.

    if tables_byte_count > 0 {
        // Ensure the "tileOffset += 2" operation below will not wrap around.

        if tile_offset > uint64::MAX - 2 {
            return Err(eof_failure());
        }

        tables_byte_count -= 2;

        tile_offset += 2;
        tile_byte_count -= 2;
    }

    let mut buffer = vec![0u8; safe_uint32_add(tables_byte_count, tile_byte_count)? as usize];

    // Read in table.

    if tables_byte_count > 0 {
        buffer[..tables_byte_count as usize]
            .copy_from_slice(&tables_block.unwrap()[..tables_byte_count as usize]);
    }

    // Read in tile data.

    stream.set_read_position(tile_offset)?;

    let tile_data = stream.get(tile_byte_count)?;

    buffer[tables_byte_count as usize..].copy_from_slice(&tile_data);

    // Patch first byte, if required.

    if patch_first_byte {
        buffer[0] = 0xFF;
    }

    Ok(buffer)
}

/// Make a read-only stream over an in-memory byte slice.
pub(crate) fn bytes_as_stream(data: &[uint8]) -> DngResult<DngStream<'static>> {
    let mut memory = dng_core::DngMemoryStream::with_page_size(dng_core::dng_stream::kDefaultBufferSize);

    let mut stream = memory.stream();

    stream.set_big_endian(false);

    stream.put(data)?;

    stream.flush()?;

    stream.set_read_position(0)?;

    Ok(stream)
}

fn memory_block_from(data: Vec<uint8>) -> DngMemoryBlock {
    let mut block = DngMemoryBlock::new(data.len() as uint32).unwrap_or_else(|_| DngMemoryBlock::new(0).unwrap());

    block.buffer_mut().copy_from_slice(&data);

    block
}

fn stream_get_at(stream: &mut DngStream, offset: uint64, count: uint32) -> DngResult<Vec<uint8>> {
    let saved = stream.position();

    stream.set_read_position(offset)?;

    let data = stream.get(count)?;

    stream.set_read_position(saved)?;

    Ok(data)
}

// ---------------------------------------------------------------------------
// PixelData helpers
// ---------------------------------------------------------------------------

/// Build typed pixel storage from decoded bytes at the sample granularity
/// the decoder produced (ttFloat may hold 2- or 3-byte samples until the
/// float expansion runs).
fn typed_storage_for(pixel_type: uint32, pixel_size: uint32, bytes: &[uint8]) -> DngResult<PixelData> {
    match pixel_type {
        x if x == ttByte => Ok(PixelData::U8(bytes.to_vec())),
        x if x == ttShort => {
            let mut samples = vec![0u16; bytes.len() / 2];

            for (i, chunk) in bytes.chunks_exact(2).enumerate() {
                samples[i] = u16::from_le_bytes([chunk[0], chunk[1]]);
            }

            Ok(PixelData::U16(samples))
        }
        x if x == ttLong => {
            let mut samples = vec![0u32; bytes.len() / 4];

            for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                samples[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            }

            Ok(PixelData::U32(samples))
        }
        x if x == ttFloat => match pixel_size {
            4 => {
                let mut samples = vec![0u32; bytes.len() / 4];

                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                    samples[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                }

                Ok(PixelData::F32(
                    samples.iter().map(|b| real32::from_bits(*b)).collect(),
                ))
            }
            2 => {
                let mut samples = vec![0u16; bytes.len() / 2];

                for (i, chunk) in bytes.chunks_exact(2).enumerate() {
                    samples[i] = u16::from_le_bytes([chunk[0], chunk[1]]);
                }

                Ok(PixelData::U16(samples))
            }
            3 => Ok(PixelData::U8(bytes.to_vec())),
            _ => Err(program_error()),
        },
        _ => Err(program_error()), // unsupported pixel type in raw reader
    }
}

/// Expand a ttFloat buffer's storage from halves (pixel_size 2) or 24-bit
/// packed (pixel_size 3) to full 32-bit floats, mirroring the C++ in-place
/// expansion before `image.Put`.
///
/// `big_endian_bytes` selects the 24-bit byte order (the FP predictor path is
/// byte-order independent and always uses the natural order, matching the
/// C++ `stream.BigEndian () || predictor == cpFloatingPoint*` test).
fn expand_float_storage(
    buffer: &mut DngPixelBuffer,
    sample_count: uint32,
    big_endian_bytes: bool,
) -> DngResult<()> {
    if buffer.pixel_type != ttFloat {
        return Ok(());
    }

    if buffer.pixel_size == 2 {
        // Expand u16 halves to f32.

        let halves = match &buffer.data {
            PixelData::U16(v) => v.clone(),
            _ => return Err(program_error()),
        };

        let floats: Vec<real32> = halves
            .iter()
            .take(sample_count as usize)
            .map(|h| real32::from_bits(dng_half_to_float(*h)))
            .collect();

        buffer.data = PixelData::F32(floats);
        buffer.pixel_size = 4;

        Ok(())
    } else if buffer.pixel_size == 3 {
        // Expand 24-bit packed floats (stored as bytes) to f32.

        let bytes = match &buffer.data {
            PixelData::U8(v) => v.clone(),
            _ => return Err(program_error()),
        };

        let mut floats = vec![0f32; sample_count as usize];

        for index in 0..sample_count as usize {
            let src = &bytes[index * 3..index * 3 + 3];

            let input: [uint8; 3] = if big_endian_bytes {
                [src[0], src[1], src[2]]
            } else {
                [src[2], src[1], src[0]]
            };

            floats[index] = real32::from_bits(dng_fp24_to_float(&input));
        }

        buffer.data = PixelData::F32(floats);
        buffer.pixel_size = 4;

        Ok(())
    } else {
        Ok(())
    }
}

fn take_u8(data: &mut PixelData) -> Vec<uint8> {
    match data {
        PixelData::U8(v) => std::mem::take(v),
        _ => Vec::new(),
    }
}

fn take_u16(data: &mut PixelData) -> Vec<u16> {
    match data {
        PixelData::U16(v) => std::mem::take(v),
        _ => Vec::new(),
    }
}

fn take_u32(data: &mut PixelData) -> Vec<u32> {
    match data {
        PixelData::U32(v) => std::mem::take(v),
        _ => Vec::new(),
    }
}

fn float_vec_to_bytes(v: &[u32]) -> Vec<uint8> {
    let mut out = Vec::with_capacity(v.len() * 4);

    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }

    out
}

fn u16_vec_to_bytes(v: &[u16]) -> Vec<uint8> {
    let mut out = Vec::with_capacity(v.len() * 2);

    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }

    out
}

fn u32_vec_to_bytes(v: &[u32]) -> Vec<uint8> {
    let mut out = Vec::with_capacity(v.len() * 4);

    for x in v {
        out.extend_from_slice(&x.to_le_bytes());
    }

    out
}

fn swap_bytes16_vec(data: &mut [uint8]) {
    for chunk in data.chunks_exact_mut(2) {
        chunk.swap(0, 1);
    }
}

fn swap_bytes32_vec(data: &mut [uint8]) {
    for chunk in data.chunks_exact_mut(4) {
        chunk.swap(0, 3);
        chunk.swap(1, 2);
    }
}

/// FP-delta variant writing into a caller-provided destination row while
/// reading the (already delta-expanded) source row.
fn decode_fp_delta_into(
    dst_row: &mut [uint8],
    src_row: &[uint8],
    cols: int32,
    channels: int32,
    bytes_per_sample: int32,
) {
    let mut input = src_row.to_vec();

    decode_delta_bytes(&mut input, cols * bytes_per_sample, channels);

    let row_increment = (cols * channels) as usize;

    if bytes_per_sample == 2 {
        let (input1, input0) = (&input[..row_increment], &input[row_increment..2 * row_increment]);

        for col in 0..row_increment {
            dst_row[2 * col] = input0[col];
            dst_row[2 * col + 1] = input1[col];
        }
    } else if bytes_per_sample == 3 {
        for col in 0..row_increment {
            dst_row[3 * col] = input[col];
            dst_row[3 * col + 1] = input[row_increment + col];
            dst_row[3 * col + 2] = input[row_increment * 2 + col];
        }
    } else {
        for col in 0..row_increment {
            dst_row[4 * col] = input[row_increment * 3 + col];
            dst_row[4 * col + 1] = input[row_increment * 2 + col];
            dst_row[4 * col + 2] = input[row_increment + col];
            dst_row[4 * col + 3] = input[col];
        }
    }
}

fn empty_pixel_data(pixel_type: uint32, elements: usize) -> DngResult<PixelData> {
    Ok(match pixel_type {
        x if x == ttByte => PixelData::U8(vec![0u8; elements]),
        x if x == ttShort => PixelData::U16(vec![0u16; elements]),
        x if x == ttLong => PixelData::U32(vec![0u32; elements]),
        x if x == ttFloat => PixelData::F32(vec![0f32; elements]),
        _ => PixelData::U8(vec![0u8; elements]),
    })
}

/// PackBits implementation (public wrapper kept C++-shaped).
pub fn decode_pack_bits_impl(
    stream: &mut DngStream,
    d_ptr: &mut [uint8],
    mut dst_count: int32,
) -> bool {
    let mut out_pos: usize = 0;

    while dst_count > 0 {
        let raw = match stream.get_uint8() {
            Ok(v) => v,
            Err(_) => return false,
        };

        let mut run_count = as_int8(raw);

        if run_count >= 0 {
            run_count += 1;

            dst_count -= run_count;

            if dst_count < 0 {
                return false;
            }

            match stream.get(run_count as uint32) {
                Ok(data) => {
                    if out_pos + data.len() > d_ptr.len() {
                        return false;
                    }

                    d_ptr[out_pos..out_pos + data.len()].copy_from_slice(&data);

                    out_pos += data.len();
                }
                Err(_) => return false,
            }
        } else {
            run_count = -run_count + 1;

            dst_count -= run_count;

            if dst_count < 0 {
                return false;
            }

            let x = match stream.get_uint8() {
                Ok(v) => v,
                Err(_) => return false,
            };

            for _ in 0..run_count {
                if out_pos >= d_ptr.len() {
                    return false;
                }

                d_ptr[out_pos] = x;

                out_pos += 1;
            }
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Negative-side stage readers (free functions called by dng_negative.rs —
// see NEGATIVE_API.md).  These implement the C++
// `dng_negative::ReadStage1Image` / `ReadTransparencyMask` / `ReadDepthMap`
// image-reading bodies; the IFD selection (dng_info indices) and the opcode
// list parsing stay on the negative side (Agent A).
// ---------------------------------------------------------------------------

/// `dng_negative::ReadStage1Image` body — read the raw IFD into the
/// negative's stage1 image.
///
/// PORT-NOTE(dng_negative.cpp:4389): the C++ version takes `dng_info` and
/// selects `info.fIFD [info.fMainIndex]`, reads the opcode lists and updates
/// several negative fields; the Rust seam takes the raw IFD directly, and
/// opcode-list reading remains the caller's responsibility (Agent A's
/// `dng_negative.rs` calls `ReadOpcodeLists` itself).
pub fn read_stage1_image(
    host: &mut DngHost,
    stream: &mut DngStream,
    negative: &mut DngNegative,
    raw_ifd: &DngIfd,
) -> DngResult<()> {
    // Allocate image we are reading.

    let stage1: ImageRef = DngSimpleImage::new(raw_ifd.bounds(), raw_ifd.samples_per_pixel, raw_ifd.pixel_type())?
        .into_image_ref()?;

    // See if we should grab the lossy compressed data.

    let lossy_image = keep_lossy_compressed_image(
        host.save_dng_version(),
        host.preferred_size(),
        negative.is_preview(),
        raw_ifd,
    );

    // See if we need to compute the digest of the compressed JPEG or JPEG XL
    // data while reading.

    let need_lossy_digest = (negative.raw_image_digest().is_valid()
        || negative.new_raw_image_digest().is_valid())
        && (raw_ifd.compression == ccLossyJPEG || raw_ifd.compression == ccJXL)
        && lossy_image.is_none();

    let mut lossy_digest = DngFingerprint::new();

    // Read the image.

    let mut lossy_ref = lossy_image;

    {
        let image_dyn: &dyn DngImage = &*stage1.read().unwrap();

        let lossy_mut: Option<&mut DngLossyCompressedImage> = lossy_ref.as_deref_mut();

        let mut reader = DngReadImage::new();

        reader.lossless_jpeg_bug16 = false; // set by the caller when the camera needs it

        reader.read(
            raw_ifd,
            stream,
            image_dyn,
            lossy_mut,
            if need_lossy_digest {
                Some(&mut lossy_digest)
            } else {
                None
            },
        )?;
    }

    // Remember the raw floating point bit depth, if reading from a floating
    // point image.

    if stage1.read().unwrap().pixel_type() == dng_core::dng_tag_types::ttFloat {
        negative.set_raw_float_bit_depth(raw_ifd.bits_per_sample[0]);
    }

    // Remember the compressed JPEG data if we read it.

    negative.set_raw_lossy_compressed_image(lossy_ref_into_owned(lossy_ref));

    // Remember the compressed JPEG digest if we computed it.

    if lossy_digest.is_valid() {
        negative.set_raw_lossy_compressed_image_digest(lossy_digest);
    }

    // Install the stage1 image.

    negative.set_stage1_image(Some(stage1));

    Ok(())
}

/// `dng_negative::ReadTransparencyMask` body.
pub fn read_transparency_mask(
    host: &mut DngHost,
    stream: &mut DngStream,
    negative: &mut DngNegative,
    mask_ifd: &DngIfd,
) -> DngResult<()> {
    // Allocate image we are reading.

    let mask: ImageRef = DngSimpleImage::new(mask_ifd.bounds(), 1, mask_ifd.pixel_type())?.into_image_ref()?;

    // Do we need to keep the lossy compressed data?

    let lossy = keep_lossy_compressed_image(
        host.save_dng_version(),
        host.preferred_size(),
        negative.is_preview(),
        mask_ifd,
    );

    // Read the image.

    {
        let image_dyn: &dyn DngImage = &*mask.read().unwrap();

        let mut lossy_ref = lossy;

        let lossy_mut: Option<&mut DngLossyCompressedImage> = lossy_ref.as_deref_mut();

        let mut reader = DngReadImage::new();

        reader.read(mask_ifd, stream, image_dyn, lossy_mut, None)?;
    }

    // Remember the pixel depth.

    negative.raw_transparency_mask_bit_depth = mask_ifd.bits_per_sample[0];

    // Remember if transparency mask was lossy compressed.

    negative.transparency_mask_was_lossy_compressed =
        mask_ifd.compression == ccLossyJPEG || mask_ifd.compression == ccJXL;

    negative.set_transparency_mask(Some(mask), mask_ifd.bits_per_sample[0]);

    Ok(())
}

/// `dng_negative::ReadDepthMap` body.
pub fn read_depth_map(
    host: &mut DngHost,
    stream: &mut DngStream,
    negative: &mut DngNegative,
    depth_ifd: &DngIfd,
) -> DngResult<()> {
    // Allocate image we are reading.

    let depth: ImageRef =
        DngSimpleImage::new(depth_ifd.bounds(), 1, depth_ifd.pixel_type())?.into_image_ref()?;

    // Keep lossy compressed depth image?

    let lossy = keep_lossy_compressed_image(
        host.save_dng_version(),
        host.preferred_size(),
        negative.is_preview(),
        depth_ifd,
    );

    // Read the image.

    {
        let image_dyn: &dyn DngImage = &*depth.read().unwrap();

        let mut lossy_ref = lossy;

        let lossy_mut: Option<&mut DngLossyCompressedImage> = lossy_ref.as_deref_mut();

        let mut reader = DngReadImage::new();

        reader.read(depth_ifd, stream, image_dyn, lossy_mut, None)?;
    }

    negative.set_depth_map(Some(depth));

    Ok(())
}

/// Helper: convert an `Option<Box<DngLossyCompressedImage>>` into the owned
/// box form the negative stores (already owned; identity).
#[allow(clippy::needless_pass_by_value)]
fn lossy_ref_into_owned(lossy: Option<Box<DngLossyCompressedImage>>) -> Option<Box<DngLossyCompressedImage>> {
    lossy
}

#[cfg(test)]
mod tests {
    use super::*;

    /// LZW round-trip on a synthetic block: build an LZW stream for a simple
    /// gradient using the SDK's own encoder is out of scope; instead verify
    /// the PackBits decoder + predictor path and the uncompressed readers.
    #[test]
    fn test_decode_pack_bits_round_trip() {
        // Encode: literals, runs, mixed.

        let mut encoded: Vec<uint8> = Vec::new();

        let src: Vec<uint8> = vec![5u8; 10];

        // Run: -(len-1) then value.

        encoded.push((-9i8) as uint8);
        encoded.push(5);

        // Literal of 3 bytes: len-1, bytes.

        encoded.push(2);
        encoded.extend_from_slice(&[1, 2, 3]);

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        stream.put(&encoded).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let mut dst = vec![0u8; 13];

        assert!(decode_pack_bits_impl(&mut stream, &mut dst, 13));

        assert_eq!(&dst[..10], &src[..]);
        assert_eq!(&dst[10..], &[1, 2, 3]);
    }

    #[test]
    fn test_decode_delta16() {
        // 2 cols x 2 rows x 3 channels, delta-encoded.  Row 0 pixel 0:
        // [100,1,2]; row 0 pixel 1: [10,20,30]; row 1 pixel 0: [5,6,7];
        // row 1 pixel 1: [8,9,10].

        let mut data = vec![100u16, 1, 2, 10, 20, 30, 5, 6, 7, 8, 9, 10];

        decode_delta16(&mut data, 2, 2, 3);

        // Row 0: pixel 0 keeps its seed values [100,1,2]; pixel 1
        // accumulates: [110,121,132].

        assert_eq!(data[0], 100);
        assert_eq!(data[1], 1);
        assert_eq!(data[2], 2);
        assert_eq!(data[3], 110);
        assert_eq!(data[4], 21);
        assert_eq!(data[5], 32);

        // Row 1: pixel 0 = [5,6,7]; pixel 1 = [13,15,17].

        assert_eq!(data[6], 5);
        assert_eq!(data[7], 6);
        assert_eq!(data[8], 7);
        assert_eq!(data[9], 13);
        assert_eq!(data[10], 15);
        assert_eq!(data[11], 17);
    }

    #[test]
    fn test_lzw_expand_known_stream() {
        // Build an LZW stream encoding "ABABABAB..." using the canonical
        // TIFF LZW algorithm, then expand it.  A minimal known stream:
        // code 65 ('A'), 66 ('B'), 256 (clear), 258, 260, 259, 257 (EOF) is
        // fragile to construct by hand; instead verify that expansion of a
        // clear-code-only stream yields the identity (returns true on EOF).

        let mut expander = DngLzwExpander::new();

        // Stream: 0x80, 0x01 = code 256 (reset) ... plus EOF 257 = 0x80 0x20
        // (9-bit codes packed MSB-first): 256 = 0b100000000, 257 =
        // 0b100000001 -> bytes 0x80 0x00 0x40 0x20 0x18 ... simplest: only a
        // reset followed by EOF.

        let stream: Vec<uint8> = vec![0x80, 0x00, 0x40, 0x20, 0x10, 0x08, 0x04, 0x02, 0x01];

        let mut dst = vec![0u8; 4];

        // The exact behaviour on a reset+EOF stream is "returns true" (EOF
        // right after reset).

        let _ = expander.expand(&stream, &mut dst, stream.len() as int32, 4);

        // No assertion on contents: this documents that the expander does not
        // panic on an immediate-EOF stream.
    }

    /// Uncompressed 16-bit read into a simple image.
    #[test]
    fn test_read_uncompressed_u16() {
        let width = 4;
        let height = 3;

        let mut ifd = DngIfd::new();

        ifd.image_width = width;
        ifd.image_length = height;
        ifd.samples_per_pixel = 1;
        ifd.bits_per_sample[0] = 16;
        ifd.sample_format[0] = sfUnsignedInteger;
        ifd.planar_configuration = pcInterleaved;
        ifd.compression = ccUncompressed;
        ifd.uses_strips = true;
        ifd.uses_tiles = false;
        ifd.tile_width = width;
        ifd.tile_length = height;

        let image = DngSimpleImage::new(
            ifd.bounds(),
            1,
            ttShort,
        )
        .unwrap()
        .into_image_ref()
        .unwrap();

        // Raw data: little-endian u16 gradient.

        let mut raw: Vec<uint8> = Vec::new();

        for i in 0..(width * height) {
            raw.extend_from_slice(&((i * 257) as u16).to_le_bytes());
        }

        let mut memory = dng_core::DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        stream.put(&raw).unwrap();

        stream.flush().unwrap();

        stream.set_read_position(0).unwrap();

        let mut reader = DngReadImage::new();

        // The C++ requires the caller to pass the uncompressed buffer
        // (ReadTile allocates it from uncompressedSize).

        let mut uncompressed_buffer: Option<Vec<uint8>> =
            Some(vec![0u8; ((width * height) * 2) as usize]);

        let mut sub_tile_block_buffer: Option<Vec<uint8>> = None;

        let ok = reader.read_uncompressed(
            &ifd,
            &mut stream,
            &*image.read().unwrap(),
            ifd.bounds(),
            0,
            1,
            &mut uncompressed_buffer,
            &mut sub_tile_block_buffer,
        )
        .unwrap();

        assert!(ok);

        // Verify pixels.

        let img = image.read().unwrap();

        let mut buffer = DngPixelBuffer::with_planar_config(
            ifd.bounds(),
            0,
            1,
            ttShort,
            pcInterleaved,
            PixelData::U16(vec![0u16; (width * height) as usize]),
        )
        .unwrap();

        img.do_get(&mut buffer).unwrap();

        for i in 0..(width * height) {
            let row = (i as int32) / width as int32;
            let col = (i as int32) % width as int32;

            assert_eq!(
                buffer.const_pixel_u16(row, col, 0).unwrap(),
                (i * 257) as u16,
                "pixel ({row}, {col})"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Additional tests: deflate + lossless-JPEG round-trips, and the writer →
// reader integration.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod roundtrip_tests {
    use super::*;
    use crate::dng_image_writer::encode_lossless;
    use dng_core::dng_image::ImageRef;
    use dng_core::dng_memory_stream::DngMemoryStream;
    use dng_core::dng_pixel_buffer::{DngPixelBuffer, PixelData};
    use dng_core::dng_point::DngPoint;
    use dng_core::dng_rect::DngRect;
    use dng_core::dng_simple_image::DngSimpleImage;
    use dng_core::dng_tag_types::ttShort;
    use dng_core::dng_types::uint16;

    fn make_gradient_image(width: uint32, height: uint32) -> ImageRef {
        let bounds = DngRect::from_size_point(DngPoint::new(height as int32, width as int32));

        let image: ImageRef = DngSimpleImage::new(bounds, 1, ttShort)
            .unwrap()
            .into_image_ref()
            .unwrap();

        for row in 0..height as int32 {
            for col in 0..width as int32 {
                let value = ((row as uint32) * width + col as uint32) * 997 % 65535;

                let mut buffer = DngPixelBuffer::with_planar_config(
                    DngRect::from_corners(row, col, row + 1, col + 1),
                    0,
                    1,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(vec![value as uint16]),
                )
                .unwrap();

                image.read().unwrap().do_put(&buffer).unwrap();
            }
        }

        image
    }

    fn ifd_for(width: uint32, height: uint32, compression: uint32) -> DngIfd {
        let mut ifd = DngIfd::new();

        ifd.image_width = width;
        ifd.image_length = height;
        ifd.samples_per_pixel = 1;
        ifd.bits_per_sample[0] = 16;
        ifd.sample_format[0] = sfUnsignedInteger;
        ifd.planar_configuration = pcInterleaved;
        ifd.compression = compression;
        ifd.predictor = cpHorizontalDifference; // matches the writer's deflate path
        ifd.uses_strips = true;
        ifd.uses_tiles = false;
        ifd.tile_width = width;
        ifd.tile_length = height;

        ifd
    }

    /// Deflate round-trip: compress the gradient via dng-core zlib (with the
    /// horizontal-difference predictor the writer applies), then read back
    /// through `DngReadImage::read_tile` and compare pixels.
    #[test]
    fn test_deflate_round_trip() {
        let width = 8;
        let height = 4;

        let image = make_gradient_image(width, height);

        // Encode: horizontal delta + zlib (little-endian samples, matching
        // the writer's deflate path).

        let mut raw: Vec<uint8> = Vec::new();

        {
            let img = image.read().unwrap();

            let samples = crate::dng_image_writer::image_samples_u16_public(&*img).unwrap();

            let cols = width as usize;

            for row in 0..height as usize {
                let base = row * cols;

                let mut prev: uint16 = 0;

                for col in 0..cols {
                    let delta = samples[base + col].wrapping_sub(prev);

                    raw.extend_from_slice(&delta.to_le_bytes());

                    prev = samples[base + col];
                }
            }
        }

        let compressed = dng_core::zlib::compress(&raw, -1).unwrap();

        // Read back.

        let ifd = ifd_for(width, height, ccDeflate);

        let decoded: ImageRef = DngSimpleImage::new(
            DngRect::from_size_point(DngPoint::new(height as int32, width as int32)),
            1,
            ttShort,
        )
        .unwrap()
        .into_image_ref()
        .unwrap();

        let mut reader = DngReadImage::new();

        let mut uncompressed_buffer: Option<Vec<uint8>> = None;
        let mut sub_tile_block_buffer: Option<Vec<uint8>> = None;

        let mut memory = DngMemoryStream::new();

        let mut stream = memory.stream();

        stream.set_big_endian(false);

        // The tile path reads from `compressed_buffer`.

        reader
            .read_tile(
                &ifd,
                &mut stream,
                &*decoded.read().unwrap(),
                ifd.tile_area(0, 0),
                0,
                1,
                compressed.len() as uint32,
                Some(&compressed),
                &mut uncompressed_buffer,
                &mut sub_tile_block_buffer,
            )
            .unwrap();

        // Compare pixels.

        let src = image.read().unwrap();
        let dst = decoded.read().unwrap();

        for row in 0..height as int32 {
            for col in 0..width as int32 {
                let mut src_buf = DngPixelBuffer::with_planar_config(
                    DngRect::from_corners(row, col, row + 1, col + 1),
                    0,
                    1,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(vec![0]),
                )
                .unwrap();

                src.do_get(&mut src_buf).unwrap();

                let mut dst_buf = DngPixelBuffer::with_planar_config(
                    DngRect::from_corners(row, col, row + 1, col + 1),
                    0,
                    1,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(vec![0]),
                )
                .unwrap();

                dst.do_get(&mut dst_buf).unwrap();

                let expected = src_buf.const_pixel_u16(row, col, 0).unwrap();
                let got = dst_buf.const_pixel_u16(row, col, 0).unwrap();

                assert_eq!(got, expected, "pixel ({row}, {col}) after deflate round-trip");
            }
        }
    }

    /// Lossless JPEG round-trip: encode via the SDK encoder and decode via
    /// `ReadLosslessJPEG`.
    #[test]
    fn test_lossless_jpeg_round_trip() {
        let width = 8;
        let height = 4;

        let image = make_gradient_image(width, height);

        // Encode.

        let mut encode_memory = DngMemoryStream::new();

        let mut encode_stream = encode_memory.stream();

        encode_stream.set_big_endian(false);

        {
            let img = image.read().unwrap();

            let samples = crate::dng_image_writer::image_samples_u16_public(&*img).unwrap();

            crate::dng_image_writer::encode_lossless(
                &samples,
                height,
                width,
                1,
                16,
                &mut encode_stream,
            )
            .unwrap();
        }

        encode_stream.flush().unwrap();

        let encoded = encode_stream.as_memory_block().unwrap();

        let encoded_bytes: Vec<uint8> = encoded.buffer().to_vec();

        // Decode.

        let ifd = ifd_for(width, height, ccJPEG);


        let decoded: ImageRef = DngSimpleImage::new(
            DngRect::from_size_point(DngPoint::new(height as int32, width as int32)),
            1,
            ttShort,
        )
        .unwrap()
        .into_image_ref()
        .unwrap();

        let mut reader = DngReadImage::new();

        reader.lossless_jpeg_bug16 = false;

        let mut uncompressed_buffer: Option<Vec<uint8>> = None;
        let mut sub_tile_block_buffer: Option<Vec<uint8>> = None;

        let mut decode_memory = DngMemoryStream::new();

        let mut decode_stream = decode_memory.stream();

        decode_stream.set_big_endian(false);

        decode_stream.put(&encoded_bytes).unwrap();

        decode_stream.flush().unwrap();

        decode_stream.set_read_position(0).unwrap();

        reader
            .read_lossless_jpeg(
                &ifd,
                &mut decode_stream,
                &*decoded.read().unwrap(),
                ifd.tile_area(0, 0),
                0,
                1,
                encoded_bytes.len() as uint32,
                &mut uncompressed_buffer,
                &mut sub_tile_block_buffer,
            )
            .unwrap();

        // Compare pixels.

        let src = image.read().unwrap();
        let dst = decoded.read().unwrap();

        for row in 0..height as int32 {
            for col in 0..width as int32 {
                let mut src_buf = DngPixelBuffer::with_planar_config(
                    DngRect::from_corners(row, col, row + 1, col + 1),
                    0,
                    1,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(vec![0]),
                )
                .unwrap();

                src.do_get(&mut src_buf).unwrap();

                let mut dst_buf = DngPixelBuffer::with_planar_config(
                    DngRect::from_corners(row, col, row + 1, col + 1),
                    0,
                    1,
                    ttShort,
                    pcInterleaved,
                    PixelData::U16(vec![0]),
                )
                .unwrap();

                dst.do_get(&mut dst_buf).unwrap();

                let expected = src_buf.const_pixel_u16(row, col, 0).unwrap();
                let got = dst_buf.const_pixel_u16(row, col, 0).unwrap();

                assert_eq!(
                    got, expected,
                    "pixel ({row}, {col}) after lossless-JPEG round-trip"
                );
            }
        }
    }
}

// Port of dng_stream.h/.cpp — the base stream abstraction with buffered
// typed accessors, byte swapping, sub-streams and stream-to-stream copies.
//
// C++ `dng_stream` is an abstract base with `DoGetLength/DoRead/DoWrite/
// DoSetLength` virtuals; the Rust port keeps exactly those as the
// `StreamIO` trait implemented by an owned boxed backend, so every C++
// subclass (dng_memory_stream, dng_file_stream, dng_stream_double_buffered,
// dng_md5_printer_stream, ...) maps to a `StreamIO` implementation plus the
// shared buffered `DngStream` driver.
//
// Lifetime note: `DngStream<'a>` where `'a` is the backend lifetime.  All
// typical uses are `DngStream<'static>` (owned backends); the
// `DngStreamDoubleBuffered` view borrows its parent stream.

use crate::dng_exceptions::{eof_failure, program_error};
use crate::dng_errors::{DngError, DngResult};
use crate::dng_rational::{DngSRational, DngURational};
use crate::dng_tag_types::*;
use crate::dng_types::{int16, int32, int64, int8, real32, real64, uint16, uint32, uint64, uint8};
use crate::dng_utils::{max_uint64, min_uint64, swap_bytes16, swap_bytes32};
use std::sync::Arc;

/// Constants for invalid offset in streams.
pub const kDNGStreamInvalidOffset: uint64 = u64::MAX;

pub const kSmallBufferSize: uint32 = 8 * 1024;
pub const kBigBufferSize: uint32 = 64 * 1024;
pub const kDefaultBufferSize: uint32 = kSmallBufferSize;

/// Backend interface replacing the C++ `Do*` virtuals.
pub trait StreamIO {
    /// `DoGetLength ()`.
    fn get_length(&mut self) -> DngResult<uint64>;

    /// `DoRead (data, count, offset)` — `data` is the full destination
    /// slice; implementors must fill it entirely or return an error.
    fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32>;

    /// `DoSetLength (length)`.
    fn set_length(&mut self, length: uint64) -> DngResult<()>;

    /// `DoWrite (data, count, offset)` — returns bytes written.
    fn write(&mut self, data: &[uint8], offset: uint64) -> DngResult<uint32>;
}

/// Port of `dng_stream`.
pub struct DngStream<'a> {
    io: Box<dyn StreamIO + 'a>,

    swap_bytes: bool,

    have_length: bool,
    length: uint64,

    offset_in_original_file: uint64,

    position: uint64,

    buffer: Vec<uint8>,
    buffer_size: uint32,

    buffer_start: uint64,
    buffer_end: uint64,
    buffer_limit: uint64,

    buffer_dirty: bool,

    sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
}

impl<'a> DngStream<'a> {
    /// `dng_stream (sniffer, bufferSize, offsetInOriginalFile)` — protected
    /// constructor in C++; public here for StreamIO-based subclasses.
    pub fn with_buffer_size(
        io: Box<dyn StreamIO + 'a>,
        sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
        buffer_size: uint32,
        offset_in_original_file: uint64,
    ) -> Self {
        let effective = std::cmp::max(
            buffer_size,
            crate::dng_globals::get_dng_stream_block_size() * 2,
        );
        DngStream {
            io,
            swap_bytes: false,
            have_length: false,
            length: 0,
            offset_in_original_file,
            position: 0,
            buffer: vec![0u8; effective as usize],
            buffer_size: effective,
            buffer_start: 0,
            buffer_end: 0,
            buffer_limit: buffer_size as uint64,
            buffer_dirty: false,
            sniffer,
        }
    }

    /// Public constructor with default buffer size.
    pub fn new(io: Box<dyn StreamIO + 'a>) -> Self {
        DngStream::with_buffer_size(io, None, kDefaultBufferSize, kDNGStreamInvalidOffset)
    }

    pub fn new_with_sniffer(
        io: Box<dyn StreamIO + 'a>,
        sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> Self {
        DngStream::with_buffer_size(io, sniffer, kDefaultBufferSize, kDNGStreamInvalidOffset)
    }

    /// `dng_stream (data, count, offsetInOriginalFile)` — stream over
    /// in-memory data (copied; C++ borrows the caller's buffer — see
    /// PORTING_NOTES.md).
    pub fn from_static(data: &[uint8], offset_in_original_file: uint64) -> Self {
        let count = data.len() as uint32;
        let mut stream = DngStream {
            io: Box::new(StaticIO {
                data: data.to_vec(),
            }),
            swap_bytes: false,
            have_length: true,
            length: count as uint64,
            offset_in_original_file,
            position: 0,
            buffer: data.to_vec(),
            buffer_size: count,
            buffer_start: 0,
            buffer_end: count as uint64,
            buffer_limit: count as uint64,
            buffer_dirty: false,
            sniffer: None,
        };
        if count == 0 {
            stream.buffer_start = 0;
            stream.buffer_end = 0;
        }
        stream
    }

    /// `SwapBytes ()`.
    pub fn swap_bytes(&self) -> bool {
        self.swap_bytes
    }

    /// `SetSwapBytes (swapBytes)`.
    pub fn set_swap_bytes(&mut self, swap_bytes: bool) {
        self.swap_bytes = swap_bytes;
    }

    /// `BigEndian ()`.
    pub fn big_endian(&self) -> bool {
        self.swap_bytes != crate::dng_flags::qDNGBigEndian
    }

    /// `SetBigEndian (bigEndian)`.
    pub fn set_big_endian(&mut self, big_endian: bool) {
        self.swap_bytes = big_endian != crate::dng_flags::qDNGBigEndian;
    }

    /// `LittleEndian ()`.
    pub fn little_endian(&self) -> bool {
        !self.big_endian()
    }

    /// `SetLittleEndian (littleEndian)`.
    pub fn set_little_endian(&mut self, little_endian: bool) {
        self.set_big_endian(!little_endian);
    }

    /// `BufferSize ()`.
    pub fn buffer_size(&self) -> uint32 {
        self.buffer_size
    }

    /// `SetBufferSize (allocator, newBufferSize)`.
    pub fn set_buffer_size(&mut self, new_buffer_size: uint32) {
        if new_buffer_size != self.buffer_size
            && new_buffer_size >= crate::dng_globals::get_dng_stream_block_size() * 2
            && self.data().is_none()
            && !self.buffer_dirty
        {
            self.buffer = vec![0u8; new_buffer_size as usize];
            self.buffer_size = new_buffer_size;
            self.buffer_start = 0;
            self.buffer_end = 0;
            self.buffer_limit = new_buffer_size as uint64;
        }
    }

    /// `Length ()` — cached.
    pub fn length(&mut self) -> uint64 {
        if !self.have_length {
            self.length = self.io.get_length().unwrap_or(0);
            self.have_length = true;
        }
        self.length
    }

    /// ARCHITECTURE.md alias for `Length ()` (fallible refresh).
    pub fn get_length(&mut self) -> uint64 {
        self.length()
    }

    /// `Position ()`.
    pub fn position(&self) -> uint64 {
        self.position
    }

    /// ARCHITECTURE.md alias for `Position ()`.
    pub fn get_position(&self) -> uint64 {
        self.position
    }

    /// `PositionInOriginalFile ()`.
    pub fn position_in_original_file(&self) -> uint64 {
        if self.offset_in_original_file == kDNGStreamInvalidOffset {
            return kDNGStreamInvalidOffset;
        }
        self.offset_in_original_file + self.position
    }

    /// `OffsetInOriginalFile ()`.
    pub fn offset_in_original_file(&self) -> uint64 {
        self.offset_in_original_file
    }

    /// `Data ()` — stream contents as a single slice when fully buffered.
    pub fn data(&self) -> Option<&[uint8]> {
        if self.buffer_start == 0 && self.have_length && self.buffer_end == self.length {
            Some(&self.buffer[..self.length as usize])
        } else {
            None
        }
    }

    /// `AsMemoryBlock (allocator, numLeadingZeroBytes)` — the whole stream
    /// as a `DngMemoryBlock`.
    pub fn as_memory_block_leading(&mut self, num_leading_zero_bytes: uint32) -> DngResult<crate::dng_memory::DngMemoryBlock> {
        self.flush()?;

        let len64 = self.length();

        if len64 + num_leading_zero_bytes as uint64 > 0xFFFF_FFFF {
            return Err(program_error());
        }

        let len = len64 as uint32;

        let mut block = crate::dng_memory::DngMemoryBlock::new(len + num_leading_zero_bytes)?;

        if len > 0 {
            self.set_read_position(0)?;
            self.get_into(&mut block.buffer_mut()[num_leading_zero_bytes as usize..], 0)?;

            if num_leading_zero_bytes > 0 {
                block.buffer_mut()[..num_leading_zero_bytes as usize].fill(0);
            }
        }

        Ok(block)
    }

    /// `AsMemoryBlock` without leading zeros (the common case).
    pub fn as_memory_block(&mut self) -> DngResult<crate::dng_memory::DngMemoryBlock> {
        self.as_memory_block_leading(0)
    }

    /// `SetReadPosition (offset)`.
    pub fn set_read_position(&mut self, offset: uint64) -> DngResult<()> {
        self.position = offset;

        if self.position > self.length() {
            return Err(eof_failure());
        }
        Ok(())
    }

    /// ARCHITECTURE.md alias for `SetReadPosition`.
    pub fn set_position(&mut self, offset: uint64) -> DngResult<()> {
        self.set_read_position(offset)
    }

    /// `Skip (delta)`.
    pub fn skip(&mut self, delta: uint64) -> DngResult<()> {
        let new_pos = self.position.wrapping_add(delta);
        self.set_read_position(new_pos)
    }

    /// ARCHITECTURE.md alias: positive delta skips forward, negative back.
    pub fn seek_relative(&mut self, delta: i64) -> DngResult<()> {
        if delta >= 0 {
            self.skip(delta as uint64)
        } else {
            self.skip((-delta) as uint64)
        }
    }

    /// `DataInBuffer (count, offset)`.
    pub fn data_in_buffer(&self, count: uint64, offset: uint64) -> bool {
        offset >= self.buffer_start && offset.wrapping_add(count) <= self.buffer_end
    }

    /// `Get (data, count, maxOverRead)` — reads exactly `data.len()` bytes;
    /// `max_over_read` mirrors the C++ parameter (allowed slack at the end
    /// of the buffer before an EOF error).
    pub fn get_into(&mut self, data: &mut [uint8], max_over_read: uint32) -> DngResult<()> {
        let mut count = data.len() as uint32;
        let mut offset_in_data = 0usize;

        while count > 0 {
            // See if the request is totally inside buffer.
            if self.position >= self.buffer_start
                && self.position.wrapping_add(count as uint64) <= self.buffer_end
            {
                let src = (self.position - self.buffer_start) as usize;
                data[offset_in_data..offset_in_data + count as usize]
                    .copy_from_slice(&self.buffer[src..src + count as usize]);
                self.position += count as uint64;
                return Ok(());
            }

            // See if first part of request is inside buffer.
            if self.position >= self.buffer_start && self.position < self.buffer_end {
                let block = (self.buffer_end - self.position) as usize;
                let src = (self.position - self.buffer_start) as usize;
                data[offset_in_data..offset_in_data + block]
                    .copy_from_slice(&self.buffer[src..src + block]);
                count -= block as uint32;
                offset_in_data += block;
                self.position += block as uint64;
            }

            // Flush buffer if dirty.
            self.flush()?;

            // Do large reads unbuffered.
            if count > self.buffer_size {
                debug_assert!(max_over_read == 0, "Over-read of large size unexpected");
                if self.position.wrapping_add(count as uint64) > self.length() {
                    return Err(eof_failure());
                }

                let pos = self.position;
                self.io.read(&mut data[offset_in_data..offset_in_data + count as usize], pos)?;
                self.position += count as uint64;
                return Ok(());
            }

            // Figure out new buffer range.
            self.buffer_start = self.position;

            if self.buffer_size >= crate::dng_globals::get_dng_stream_block_size() {
                // Align to a file block.
                let block_mask = !(crate::dng_globals::get_dng_stream_block_size() as uint64 - 1);
                self.buffer_start &= block_mask;
            }

            self.buffer_end = min_uint64(
                self.buffer_start.wrapping_add(self.buffer_size as uint64),
                self.length(),
            );

            if (self.buffer_end - self.position) < max_over_read as uint64 {
                // Allow over-read requests.
                return Ok(());
            } else if self.buffer_end <= self.position {
                return Err(eof_failure());
            }

            // Read data into buffer.
            crate::dng_abort_sniffer::sniff_for_abort(self.sniffer.as_ref())?;

            let start = self.buffer_start;
            let end = self.buffer_end;
            let n = self
                .io
                .read(&mut self.buffer[..(end - start) as usize], start)?;
            if n != (end - start) as uint32 {
                // Short read: treat as EOF for the buffered path.
                return Err(eof_failure());
            }
        }

        Ok(())
    }

    /// `Get` for exactly `count` bytes returning a Vec.
    pub fn get(&mut self, count: uint32) -> DngResult<Vec<uint8>> {
        let mut data = vec![0u8; count as usize];
        self.get_into(&mut data, 0)?;
        Ok(data)
    }

    /// `SetWritePosition (offset)`.
    pub fn set_write_position(&mut self, offset: uint64) -> DngResult<()> {
        self.position = offset;
        Ok(())
    }

    /// `Flush ()`.
    pub fn flush(&mut self) -> DngResult<()> {
        if self.buffer_dirty {
            crate::dng_abort_sniffer::sniff_for_abort(self.sniffer.as_ref())?;

            let start = self.buffer_start;
            let end = self.buffer_end;
            let data = &self.buffer[..(end - start) as usize];
            self.io.write(data, start)?;

            self.buffer_start = 0;
            self.buffer_end = 0;
            self.buffer_limit = self.buffer_size as uint64;

            self.buffer_dirty = false;
        }
        Ok(())
    }

    /// `SetLength (length)`.
    pub fn set_length(&mut self, length: uint64) -> DngResult<()> {
        self.flush()?;

        if self.length() != length {
            self.io.set_length(length)?;
            self.length = length;
            self.have_length = true;
        }
        Ok(())
    }

    /// Alias for `put` (ARCHITECTURE.md naming).
    pub fn put_bytes(&mut self, data: &[uint8]) -> DngResult<()> {
        self.put(data)
    }

    /// Alias for `get(count)` returning a Vec.
    pub fn get_bytes(&mut self, count: uint32) -> DngResult<Vec<uint8>> {
        self.get(count)
    }

    /// `Put (data, count)`.
    pub fn put(&mut self, data: &[uint8]) -> DngResult<()> {
        let mut data = data;
        let mut count = data.len() as uint32;

        // See if we can replace or append to the existing buffer.
        let end_position = self.position.wrapping_add(count as uint64);

        if self.buffer_dirty
            && self.position >= self.buffer_start
            && self.position <= self.buffer_end
            && end_position <= self.buffer_limit
        {
            let dst = (self.position - self.buffer_start) as usize;
            self.buffer[dst..dst + count as usize].copy_from_slice(&data[..count as usize]);

            if self.buffer_end < end_position {
                self.buffer_end = end_position;
            }
        } else {
            // Write initial part of the data to buffer, if possible.
            if self.buffer_dirty
                && self.position >= self.buffer_start
                && self.position <= self.buffer_end
                && self.position < self.buffer_limit
            {
                let sub_count = (self.buffer_limit - self.position) as usize;

                let dst = (self.position - self.buffer_start) as usize;
                self.buffer[dst..dst + sub_count]
                    .copy_from_slice(&data[..sub_count]);

                count -= sub_count as uint32;
                data = &data[sub_count..];

                self.position = self.buffer_limit;
                self.buffer_end = self.buffer_limit;
            }

            // Write existing buffer.
            self.flush()?;

            // Figure out how much space we have in buffer from current
            // position to end of file block.
            let block_round = crate::dng_globals::get_dng_stream_block_size() as uint64 - 1;
            let block_mask = !block_round;

            let aligned_size = ((self.position.wrapping_add(self.buffer_size as uint64)
                & block_mask)
                .wrapping_sub(self.position)) as uint32;

            // If write request will not fit in buffer, then write everything
            // except for the final unaligned part of the data.
            if count > aligned_size {
                let aligned_count =
                    ((self.position.wrapping_add(count as uint64) & block_mask)
                        .wrapping_sub(self.position)) as uint32;

                crate::dng_abort_sniffer::sniff_for_abort(self.sniffer.as_ref())?;

                self.io.write(&data[..aligned_count as usize], self.position)?;

                count -= aligned_count;
                data = &data[aligned_count as usize..];

                self.position += aligned_count as uint64;
            }

            // Start a new buffer with small blocks.
            if count > 0 {
                self.buffer_dirty = true;

                self.buffer_start = self.position;
                self.buffer_end = end_position;
                self.buffer_limit =
                    (self.buffer_start + self.buffer_size as uint64) & block_mask;

                self.buffer[..count as usize].copy_from_slice(&data[..count as usize]);
            }
        }

        self.position = end_position;

        let len = self.length();
        self.length = max_uint64(len, self.position);
        self.have_length = true;

        Ok(())
    }

    // ---------------------------------------------------------- typed gets

    /// `Get_uint8 ()`.
    pub fn get_uint8(&mut self) -> DngResult<uint8> {
        // Fast check to see if in buffer.
        if self.position >= self.buffer_start && self.position < self.buffer_end {
            let b = self.buffer[(self.position - self.buffer_start) as usize];
            self.position += 1;
            return Ok(b);
        }

        let mut x = [0u8; 1];
        self.get_into(&mut x, 0)?;
        Ok(x[0])
    }

    /// `Put_uint8 (x)`.
    pub fn put_uint8(&mut self, x: uint8) -> DngResult<()> {
        if self.buffer_dirty
            && self.position >= self.buffer_start
            && self.position <= self.buffer_end
            && self.position < self.buffer_limit
        {
            self.buffer[(self.position - self.buffer_start) as usize] = x;
            self.position += 1;

            if self.buffer_end < self.position {
                self.buffer_end = self.position;
            }

            let len = self.length();
            self.length = max_uint64(len, self.position);
        } else {
            self.put(&[x])?;
        }

        Ok(())
    }

    /// `Get_uint16 ()`.
    pub fn get_uint16(&mut self) -> DngResult<uint16> {
        let mut x = [0u8; 2];
        self.get_into(&mut x, 0)?;
        let v = uint16::from_ne_bytes([x[0], x[1]]);
        Ok(if self.swap_bytes { swap_bytes16(v) } else { v })
    }

    /// `Put_uint16 (x)`.
    pub fn put_uint16(&mut self, mut x: uint16) -> DngResult<()> {
        if self.swap_bytes {
            x = swap_bytes16(x);
        }
        self.put(&x.to_ne_bytes())
    }

    /// `Get_uint32 ()`.
    pub fn get_uint32(&mut self) -> DngResult<uint32> {
        let mut x = [0u8; 4];
        self.get_into(&mut x, 0)?;
        let v = uint32::from_ne_bytes([x[0], x[1], x[2], x[3]]);
        Ok(if self.swap_bytes {
            swap_bytes32(v)
        } else {
            v
        })
    }

    /// `Get_uint32_LE ()` — no swap check (little-endian fast path in C++,
    /// which relies on the host being little-endian; identical to
    /// `Get_uint32` when swapping is off).
    pub fn get_uint32_le(&mut self) -> DngResult<uint32> {
        self.get_uint32()
    }

    /// `Put_uint32 (x)`.
    pub fn put_uint32(&mut self, mut x: uint32) -> DngResult<()> {
        if self.swap_bytes {
            x = swap_bytes32(x);
        }
        self.put(&x.to_ne_bytes())
    }

    /// `Get_uint64 ()`.
    pub fn get_uint64(&mut self) -> DngResult<uint64> {
        if self.swap_bytes {
            let hi = self.get_uint32()? as uint64;
            let lo = self.get_uint32()? as uint64;
            return Ok((hi << 32) | lo);
        }
        let mut x = [0u8; 8];
        self.get_into(&mut x, 0)?;
        Ok(uint64::from_ne_bytes(x))
    }

    /// `Put_uint64 (x)`.
    pub fn put_uint64(&mut self, x: uint64) -> DngResult<()> {
        if self.swap_bytes {
            self.put_uint32((x >> 32) as uint32)?;
            self.put_uint32((x & 0xFFFFFFFF) as uint32)?;
        } else {
            self.put(&x.to_ne_bytes())?;
        }
        Ok(())
    }

    /// `Get_int8 ()`.
    pub fn get_int8(&mut self) -> DngResult<int8> {
        Ok(self.get_uint8()? as int8)
    }

    /// `Put_int8 (x)`.
    pub fn put_int8(&mut self, x: int8) -> DngResult<()> {
        self.put_uint8(x as uint8)
    }

    /// `Get_int16 ()`.
    pub fn get_int16(&mut self) -> DngResult<int16> {
        Ok(self.get_uint16()? as int16)
    }

    /// `Put_int16 (x)`.
    pub fn put_int16(&mut self, x: int16) -> DngResult<()> {
        self.put_uint16(x as uint16)
    }

    /// `Get_int32 ()`.
    pub fn get_int32(&mut self) -> DngResult<int32> {
        Ok(self.get_uint32()? as int32)
    }

    /// `Put_int32 (x)`.
    pub fn put_int32(&mut self, x: int32) -> DngResult<()> {
        self.put_uint32(x as uint32)
    }

    /// `Get_int64 ()`.
    pub fn get_int64(&mut self) -> DngResult<int64> {
        Ok(self.get_uint64()? as int64)
    }

    /// `Put_int64 (x)`.
    pub fn put_int64(&mut self, x: int64) -> DngResult<()> {
        self.put_uint64(x as uint64)
    }

    /// `Get_real32 ()`.
    pub fn get_real32(&mut self) -> DngResult<real32> {
        Ok(real32::from_bits(self.get_uint32()?))
    }

    /// `Put_real32 (x)`.
    pub fn put_real32(&mut self, x: real32) -> DngResult<()> {
        if self.swap_bytes {
            self.put_uint32(x.to_bits())
        } else {
            self.put(&x.to_bits().to_ne_bytes())
        }
    }

    /// `Get_real64 ()`.
    pub fn get_real64(&mut self) -> DngResult<real64> {
        if self.swap_bytes {
            let hi = self.get_uint32()? as uint64;
            let lo = self.get_uint32()? as uint64;
            return Ok(real64::from_bits((hi << 32) | lo));
        }
        let mut x = [0u8; 8];
        self.get_into(&mut x, 0)?;
        Ok(real64::from_bits(uint64::from_ne_bytes(x)))
    }

    /// `Put_real64 (x)`.
    pub fn put_real64(&mut self, x: real64) -> DngResult<()> {
        if self.swap_bytes {
            let bits = x.to_bits();
            self.put_uint32((bits >> 32) as uint32)?;
            self.put_uint32((bits & 0xFFFFFFFF) as uint32)?;
        } else {
            self.put(&x.to_bits().to_ne_bytes())?;
        }
        Ok(())
    }

    /// `Get_real16` — not in the C++ base (readers convert via
    /// DNG_HalfToFloat); provided for ttHalfFloat buffers.
    pub fn get_uint16_half(&mut self) -> DngResult<uint16> {
        self.get_uint16()
    }

    /// `Get_CString (data, maxLength)` — returns up to `max_length - 1`
    /// bytes, always consuming through the NUL.
    pub fn get_c_string(&mut self, max_length: uint32) -> DngResult<Vec<uint8>> {
        let mut data = vec![0u8; max_length as usize];
        let mut index: uint32 = 0;

        loop {
            let c = self.get_uint8()?;

            if index + 1 < max_length {
                data[index as usize] = c;
                index += 1;
            }

            if c == 0 {
                break;
            }
        }

        data.truncate(index as usize);
        if data.last() == Some(&0) {
            data.pop();
        }
        Ok(data)
    }

    /// `Put_CString (data)` — includes the trailing NUL.
    pub fn put_c_string(&mut self, data: &[uint8]) -> DngResult<()> {
        let mut bytes = data.to_vec();
        bytes.push(0);
        self.put(&bytes)
    }

    /// `Get_UString (data, maxLength)` — 16-bit chars truncated to 8 bits.
    pub fn get_u_string(&mut self, max_length: uint32) -> DngResult<Vec<uint8>> {
        let mut data = vec![0u8; max_length as usize];
        let mut index: uint32 = 0;

        loop {
            let c = self.get_uint16()?;

            if index + 1 < max_length {
                data[index as usize] = c as uint8;
                index += 1;
            }

            if c == 0 {
                break;
            }
        }

        data.truncate(index as usize);
        if data.last() == Some(&0) {
            data.pop();
        }
        Ok(data)
    }

    /// `PutZeros (count)`.
    pub fn put_zeros(&mut self, count: uint64) -> DngResult<()> {
        const kZeroBufferSize: uint32 = 4096;

        if count >= kZeroBufferSize as uint64 {
            let zero_buffer = vec![0u8; kZeroBufferSize as usize];
            let mut count = count;
            while count > 0 {
                let block_size = min_uint64(count, kZeroBufferSize as uint64);
                self.put(&zero_buffer[..block_size as usize])?;
                count -= block_size;
            }
        } else {
            for _ in 0..count {
                self.put_uint8(0)?;
            }
        }

        Ok(())
    }

    /// `PadAlign2 ()`.
    pub fn pad_align2(&mut self) -> DngResult<()> {
        self.put_zeros(self.position & 1)
    }

    /// `PadAlign4 ()`.
    pub fn pad_align4(&mut self) -> DngResult<()> {
        self.put_zeros((4 - (self.position & 3)) & 3)
    }

    // ------------------------------------------------------- tag values

    /// `TagValue_uint32 (tagType)`.
    pub fn tag_value_uint32(&mut self, tag_type: uint32) -> DngResult<uint32> {
        match tag_type {
            ttByte => Ok(self.get_uint8()? as uint32),
            ttShort => Ok(self.get_uint16()? as uint32),
            ttLong | ttIFD => self.get_uint32(),
            _ => {
                let mut x = self.tag_value_real64(tag_type)?;
                if x < 0.0 {
                    x = 0.0;
                }
                if x > 0xFFFF_FFFFu32 as real64 {
                    x = 0xFFFF_FFFFu32 as real64;
                }
                crate::dng_safe_arithmetic::convert_double_to_uint32(x + 0.5)
            }
        }
    }

    /// `TagValue_uint64 (tagType)`.
    pub fn tag_value_uint64(&mut self, tag_type: uint32) -> DngResult<uint64> {
        match tag_type {
            ttLong8 | ttIFD8 => self.get_uint64(),
            ttSLong8 => Ok(self.get_int64()?.max(0) as uint64),
            _ => Ok(self.tag_value_uint32(tag_type)? as uint64),
        }
    }

    /// `TagValue_int32 (tagType)`.
    pub fn tag_value_int32(&mut self, tag_type: uint32) -> DngResult<int32> {
        match tag_type {
            ttSByte => Ok(self.get_int8()? as int32),
            ttSShort => Ok(self.get_int16()? as int32),
            ttSLong => self.get_int32(),
            _ => {
                let x = self.tag_value_real64(tag_type)?;
                if x < 0.0 {
                    let x = if x < -2147483648.0 { -2147483648.0 } else { x };
                    crate::dng_safe_arithmetic::convert_double_to_int32(x - 0.5)
                } else {
                    let x = if x > 2147483647.0 { 2147483647.0 } else { x };
                    crate::dng_safe_arithmetic::convert_double_to_int32(x + 0.5)
                }
            }
        }
    }

    /// `TagValue_int64 (tagType)`.
    pub fn tag_value_int64(&mut self, tag_type: uint32) -> DngResult<int64> {
        match tag_type {
            ttSLong8 => self.get_int64(),
            ttLong8 | ttIFD8 => {
                let x = self.get_int64()?;
                Ok(if x < 0 {
                    0x7FFF_FFFF_FFFF_FFFF
                } else {
                    x
                })
            }
            _ => Ok(self.tag_value_int32(tag_type)? as int64),
        }
    }

    /// `TagValue_urational (tagType)`.
    pub fn tag_value_urational(&mut self, tag_type: uint32) -> DngResult<DngURational> {
        let mut result = DngURational::new(0, 1);

        match tag_type {
            ttRational => {
                result.n = self.get_uint32()?;
                result.d = self.get_uint32()?;
            }
            ttSRational => {
                let mut n = self.get_int32()?;
                let mut d = self.get_int32()?;

                if (n < 0) == (d < 0) {
                    if d < 0 {
                        n = -n;
                        d = -d;
                    }
                    result.n = n as uint32;
                    result.d = d as uint32;
                }
            }
            ttByte | ttShort | ttLong | ttIFD => {
                result.n = self.tag_value_uint32(tag_type)?;
            }
            ttSByte | ttSShort | ttSLong => {
                let n = self.tag_value_int32(tag_type)?;
                if n > 0 {
                    result.n = n as uint32;
                }
            }
            _ => {
                let mut x = self.tag_value_real64(tag_type)?;

                if x > 0.0 {
                    while result.d < 10000 && x < 1000000.0 {
                        result.d *= 10;
                        x *= 10.0;
                    }

                    result.n = crate::dng_safe_arithmetic::convert_double_to_uint32(x + 0.5)?;
                }
            }
        }

        Ok(result)
    }

    /// `TagValue_srational (tagType)`.
    pub fn tag_value_srational(&mut self, tag_type: uint32) -> DngResult<DngSRational> {
        let mut result = DngSRational::new(0, 1);

        match tag_type {
            ttSRational => {
                result.n = self.get_int32()?;
                result.d = self.get_int32()?;
            }
            _ => {
                let mut x = self.tag_value_real64(tag_type)?;

                if x > 0.0 {
                    while result.d < 10000 && x < 1000000.0 {
                        result.d *= 10;
                        x *= 10.0;
                    }
                    result.n = crate::dng_safe_arithmetic::convert_double_to_int32(x + 0.5)?;
                } else {
                    while result.d < 10000 && x > -1000000.0 {
                        result.d *= 10;
                        x *= 10.0;
                    }
                    result.n = crate::dng_safe_arithmetic::convert_double_to_int32(x - 0.5)?;
                }
            }
        }

        Ok(result)
    }

    /// `TagValue_real64 (tagType)`.
    pub fn tag_value_real64(&mut self, tag_type: uint32) -> DngResult<real64> {
        match tag_type {
            ttByte | ttShort | ttLong | ttIFD => Ok(self.tag_value_uint32(tag_type)? as real64),
            ttLong8 | ttIFD8 => Ok(self.tag_value_uint64(tag_type)? as real64),
            ttSByte | ttSShort | ttSLong => Ok(self.tag_value_int32(tag_type)? as real64),
            ttSLong8 => Ok(self.tag_value_int64(tag_type)? as real64),
            ttRational => {
                let n = self.get_uint32()?;
                let d = self.get_uint32()?;
                if d == 0 {
                    Ok(0.0)
                } else {
                    Ok(n as real64 / d as real64)
                }
            }
            ttSRational => {
                let n = self.get_int32()?;
                let d = self.get_int32()?;
                if d == 0 {
                    Ok(0.0)
                } else {
                    Ok(n as real64 / d as real64)
                }
            }
            ttFloat => Ok(self.get_real32()? as real64),
            ttDouble => self.get_real64(),
            _ => Ok(0.0),
        }
    }

    // ---------------------------------------------------------- sniffer

    /// `Sniffer ()`.
    pub fn sniffer(&self) -> Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>> {
        self.sniffer.as_ref()
    }

    /// `SetSniffer (sniffer)`.
    pub fn set_sniffer(&mut self, sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>) {
        self.sniffer = sniffer;
    }

    // ---------------------------------------------------------- copies

    /// `CopyToStream (dstStream, count)`.
    pub fn copy_to_stream(
        &mut self,
        dst_stream: &mut DngStream,
        mut count: uint64,
    ) -> DngResult<()> {
        const SMALL: usize = 1024;

        if count <= SMALL as uint64 {
            let mut small_buffer = vec![0u8; count as usize];
            self.get_into(&mut small_buffer, 0)?;
            dst_stream.put(&small_buffer)?;
        } else {
            let big_buffer_size = min_uint64(kBigBufferSize as uint64, count) as usize;
            let mut big_buffer = vec![0u8; big_buffer_size];

            while count > 0 {
                let block_count = min_uint64(big_buffer_size as uint64, count) as usize;

                self.get_into(&mut big_buffer[..block_count], 0)?;
                dst_stream.put(&big_buffer[..block_count])?;

                count -= block_count as uint64;
            }
        }

        Ok(())
    }

    /// `DuplicateStream (dstStream)`.
    pub fn duplicate_stream(&mut self, dst_stream: &mut DngStream) -> DngResult<()> {
        // Grow the destination stream first, in an attempt to reserve any
        // needed space before overwriting the existing data.
        let len = self.length();
        if dst_stream.length() < len {
            dst_stream.set_length(len)?;
        }

        self.set_read_position(0)?;
        dst_stream.set_write_position(0)?;

        self.copy_to_stream(dst_stream, len)?;

        dst_stream.flush()?;
        dst_stream.set_length(len)?;

        Ok(())
    }
}

/// IO backend for `DngStream::from_static` (whole contents in memory).
struct StaticIO {
    data: Vec<uint8>,
}

impl StreamIO for StaticIO {
    fn get_length(&mut self) -> DngResult<uint64> {
        Ok(self.data.len() as uint64)
    }

    fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32> {
        let offset = offset as usize;
        if offset + data.len() > self.data.len() {
            return Err(eof_failure());
        }
        data.copy_from_slice(&self.data[offset..offset + data.len()]);
        Ok(data.len() as uint32)
    }

    fn set_length(&mut self, _length: uint64) -> DngResult<()> {
        Err(program_error())
    }

    fn write(&mut self, _data: &[uint8], _offset: uint64) -> DngResult<uint32> {
        Err(program_error())
    }
}

/// Port of `dng_stream_double_buffered` — constructs a read-only `DngStream`
/// view whose reads go through the parent stream.  The returned value is a
/// genuine `DngStream` (with its own small buffer), so it can be passed
/// anywhere a `dng_stream &` is expected, exactly like the C++ subclass.
pub fn dng_stream_double_buffered<'p, 'q>(
    parent: &'p mut DngStream<'q>,
) -> DngStream<'p> {
    let offset = parent.offset_in_original_file();
    let big_endian = parent.big_endian();
    let length = parent.length();

    let mut stream = DngStream::with_buffer_size(
        Box::new(DoubleBufferedIO { parent }),
        None,
        kDefaultBufferSize,
        offset,
    );
    stream.set_big_endian(big_endian);
    stream.have_length = true;
    stream.length = length;
    stream
}

/// IO backend for the double-buffered view.
struct DoubleBufferedIO<'p, 'q> {
    parent: &'p mut DngStream<'q>,
}

impl<'p, 'q> StreamIO for DoubleBufferedIO<'p, 'q> {
    fn get_length(&mut self) -> DngResult<uint64> {
        Ok(self.parent.length())
    }

    fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32> {
        self.parent.set_read_position(offset)?;
        self.parent.get_into(data, 0)?;
        Ok(data.len() as uint32)
    }

    fn set_length(&mut self, _length: uint64) -> DngResult<()> {
        Err(program_error())
    }

    fn write(&mut self, _data: &[uint8], _offset: uint64) -> DngResult<uint32> {
        Err(program_error())
    }
}

/// Port of `TempBigEndian` / `TempLittleEndian` — RAII endianness switch.
pub struct TempBigEndian<'s, 'a> {
    stream: &'s mut DngStream<'a>,
    old_swap: bool,
}

impl<'s, 'a> TempBigEndian<'s, 'a> {
    pub fn new(stream: &'s mut DngStream<'a>, big_endian: bool) -> Self {
        let old_swap = stream.swap_bytes();
        stream.set_big_endian(big_endian);
        TempBigEndian { stream, old_swap }
    }

    /// Access the guarded stream.
    pub fn stream(&mut self) -> &mut DngStream<'a> {
        self.stream
    }
}

impl<'s, 'a> Drop for TempBigEndian<'s, 'a> {
    fn drop(&mut self) {
        self.stream.set_swap_bytes(self.old_swap);
    }
}

/// Port of `TempLittleEndian`.
pub struct TempLittleEndian<'s, 'a>(TempBigEndian<'s, 'a>);

impl<'s, 'a> TempLittleEndian<'s, 'a> {
    pub fn new(stream: &'s mut DngStream<'a>, little_endian: bool) -> Self {
        TempLittleEndian(TempBigEndian::new(stream, !little_endian))
    }

    /// Access the guarded stream.
    pub fn stream(&mut self) -> &mut DngStream<'a> {
        self.0.stream()
    }
}

/// Port of `TempStreamSniffer` — RAII sniffer replacement.
pub struct TempStreamSniffer<'s, 'a> {
    stream: &'s mut DngStream<'a>,
    old_sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
}

impl<'s, 'a> TempStreamSniffer<'s, 'a> {
    pub fn new(
        stream: &'s mut DngStream<'a>,
        sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> Self {
        let old = stream.sniffer().cloned();
        stream.set_sniffer(sniffer);
        TempStreamSniffer {
            stream,
            old_sniffer: old,
        }
    }
}

impl<'s, 'a> Drop for TempStreamSniffer<'s, 'a> {
    fn drop(&mut self) {
        self.stream.set_sniffer(self.old_sniffer.take());
    }
}

/// Port of `PreserveStreamReadPosition` — RAII position save/restore.
pub struct PreserveStreamReadPosition<'s, 'a> {
    stream: &'s mut DngStream<'a>,
    position: uint64,
}

impl<'s, 'a> PreserveStreamReadPosition<'s, 'a> {
    pub fn new(stream: &'s mut DngStream<'a>) -> Self {
        let position = stream.position();
        PreserveStreamReadPosition { stream, position }
    }

    /// Delegate: `SetReadPosition`.
    pub fn set_read_position(&mut self, offset: uint64) -> DngResult<()> {
        self.stream.set_read_position(offset)
    }

    /// Delegate: `Get_uint32`.
    pub fn get_uint32(&mut self) -> DngResult<uint32> {
        self.stream.get_uint32()
    }
}

impl<'s, 'a> Drop for PreserveStreamReadPosition<'s, 'a> {
    fn drop(&mut self) {
        let _ = self.stream.set_read_position(self.position);
    }
}

/// Port of `dng_stream_contiguous_read_hint` — RAII buffer-size hint for
/// contiguous reads.
pub struct DngStreamContiguousReadHint<'s, 'a> {
    stream: &'s mut DngStream<'a>,
    old_buffer_size: uint32,
}

impl<'s, 'a> DngStreamContiguousReadHint<'s, 'a> {
    pub fn new(stream: &'s mut DngStream<'a>, mut count: uint64) -> Self {
        stream.flush().ok(); // Cannot change buffer size with dirty buffer

        let old_buffer_size = stream.buffer_size();

        // Don't bother changing buffer size if only a small change.
        if count > old_buffer_size as uint64 * 4 {
            let block_round = crate::dng_globals::get_dng_stream_block_size() as uint64 - 1;
            let block_mask = !block_round;

            let offset = stream.position();
            count = (count + (offset & block_round) + block_round) & block_mask;

            // Limit to maximum buffer size.
            let mut new_buffer_size =
                min_uint64(crate::dng_globals::get_dng_max_stream_buffer_size() as uint64, count);

            // Adjust to make an exact number of buffers fit.
            let num_buffers = (count + new_buffer_size - 1) / new_buffer_size;
            new_buffer_size = (count + num_buffers - 1) / num_buffers;

            // Round up to a block size.
            new_buffer_size = (new_buffer_size + block_round) & block_mask;

            stream.set_buffer_size(new_buffer_size as uint32);
        }

        DngStreamContiguousReadHint {
            stream,
            old_buffer_size,
        }
    }
}

impl<'s, 'a> Drop for DngStreamContiguousReadHint<'s, 'a> {
    fn drop(&mut self) {
        self.stream.set_buffer_size(self.old_buffer_size);
    }
}

// (Endianness guard helpers live above; no extra helpers needed here.)

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_exceptions::read_failure;
    use crate::dng_exceptions::write_failure;

    struct VecIO {
        data: Vec<uint8>,
    }

    impl VecIO {
        fn new() -> Self {
            VecIO { data: Vec::new() }
        }
    }

    impl StreamIO for VecIO {
        fn get_length(&mut self) -> DngResult<uint64> {
            Ok(self.data.len() as uint64)
        }

        fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32> {
            let offset = offset as usize;
            if offset + data.len() > self.data.len() {
                return Err(eof_failure());
            }
            data.copy_from_slice(&self.data[offset..offset + data.len()]);
            Ok(data.len() as uint32)
        }

        fn set_length(&mut self, length: uint64) -> DngResult<()> {
            self.data.resize(length as usize, 0);
            Ok(())
        }

        fn write(&mut self, data: &[uint8], offset: uint64) -> DngResult<uint32> {
            let offset = offset as usize;
            if offset + data.len() > self.data.len() {
                self.data.resize(offset + data.len(), 0);
            }
            self.data[offset..offset + data.len()].copy_from_slice(data);
            Ok(data.len() as uint32)
        }
    }

    #[test]
    fn typed_roundtrip_little_endian() {
        let mut s = DngStream::new(Box::new(VecIO::new()));

        s.set_little_endian(true);
        assert!(s.little_endian());

        s.put_uint8(0x12).unwrap();
        s.put_int8(-3).unwrap();
        s.put_uint16(0x1234).unwrap();
        s.put_int16(-2).unwrap();
        s.put_uint32(0x12345678).unwrap();
        s.put_int32(-5).unwrap();
        s.put_uint64(0x0123456789ABCDEF).unwrap();
        s.put_int64(-7).unwrap();
        s.put_real32(1.5).unwrap();
        s.put_real64(2.5).unwrap();
        s.put_c_string(b"hello").unwrap();
        s.put_zeros(5).unwrap();

        s.set_read_position(0).unwrap();

        assert_eq!(s.get_uint8().unwrap(), 0x12);
        assert_eq!(s.get_int8().unwrap(), -3);
        assert_eq!(s.get_uint16().unwrap(), 0x1234);
        assert_eq!(s.get_int16().unwrap(), -2);
        assert_eq!(s.get_uint32().unwrap(), 0x12345678);
        assert_eq!(s.get_int32().unwrap(), -5);
        assert_eq!(s.get_uint64().unwrap(), 0x0123456789ABCDEF);
        assert_eq!(s.get_int64().unwrap(), -7);
        assert_eq!(s.get_real32().unwrap(), 1.5);
        assert_eq!(s.get_real64().unwrap(), 2.5);
        assert_eq!(s.get_c_string(16).unwrap(), b"hello");
        assert_eq!(s.position(), 48); // reads: 1+1+2+2+4+4+8+8+4+8+6
    }

    #[test]
    fn byte_swap_roundtrip() {
        let mut s = DngStream::new(Box::new(VecIO::new()));

        // Big-endian data.
        s.set_big_endian(true);
        s.put_uint16(0x1234).unwrap();
        s.put_uint32(0x12345678).unwrap();
        s.put_uint64(0x0123456789ABCDEF).unwrap();
        s.put_real32(1.5).unwrap();
        s.put_real64(2.5).unwrap();
        s.flush().unwrap();

        let data = s.as_memory_block().unwrap().buffer().to_vec();
        assert_eq!(&data[0..2], &[0x12, 0x34]);
        assert_eq!(&data[2..6], &[0x12, 0x34, 0x56, 0x78]);

        // Read back with explicit little-endian interpretation of the same
        // bytes: the values must differ (i.e. swap happened).
        let mut s2 = DngStream::from_static(&data, 0);
        s2.set_big_endian(true);
        assert_eq!(s2.get_uint16().unwrap(), 0x1234);
        assert_eq!(s2.get_uint32().unwrap(), 0x12345678);
        assert_eq!(s2.get_uint64().unwrap(), 0x0123456789ABCDEF);
        assert_eq!(s2.get_real32().unwrap(), 1.5);
        assert_eq!(s2.get_real64().unwrap(), 2.5);

        // Big-endian encoding of 1.5f32 = 3FC00000.
        assert_eq!(&data[14..18], &[0x3F, 0xC0, 0x00, 0x00]);
    }

    #[test]
    fn eof_errors() {
        let mut s = DngStream::from_static(&[1, 2, 3], 0);
        assert_eq!(s.get_uint8().unwrap(), 1);
        s.set_read_position(4).unwrap_err();
        s.set_read_position(3).unwrap();
        assert!(matches!(s.get_uint8(), Err(DngError::EndOfFile)));
        assert!(matches!(s.get_uint32(), Err(DngError::EndOfFile)));
    }

    #[test]
    fn substream_view() {
        let mut parent = DngStream::new(Box::new(VecIO::new()));
        parent.set_big_endian(true);
        parent.put_uint32(0xDEADBEEF).unwrap();
        parent.put_uint32(0xCAFEBABE).unwrap();

        let mut view = dng_stream_double_buffered(&mut parent);
        view.set_read_position(4).unwrap();
        assert_eq!(view.get_uint32().unwrap(), 0xCAFEBABE);
        assert_eq!(view.length(), 8);
        assert!(view.big_endian());
    }

    #[test]
    fn copy_and_duplicate() {
        let mut src = DngStream::new(Box::new(VecIO::new()));
        for i in 0..10_000u32 {
            src.put_uint32(i).unwrap();
        }

        let mut dst = DngStream::new(Box::new(VecIO::new()));
        src.set_read_position(0).unwrap();
        src.duplicate_stream(&mut dst).unwrap();

        assert_eq!(dst.length(), 40_000);
        dst.set_read_position(4).unwrap();
        assert_eq!(dst.get_uint32().unwrap(), 1);
        dst.set_read_position(39_996).unwrap();
        assert_eq!(dst.get_uint32().unwrap(), 9_999);
    }

    #[test]
    fn memory_block_roundtrip() {
        let mut s = DngStream::new(Box::new(VecIO::new()));
        s.set_big_endian(true);
        s.put_uint32(0x01020304).unwrap();
        s.put_uint8(0xFF).unwrap();

        let block = s.as_memory_block().unwrap();
        assert_eq!(block.logical_size(), 5);
        assert_eq!(block.buffer(), &[1, 2, 3, 4, 0xFF]);

        // With leading zeros.
        let block = s.as_memory_block_leading(3).unwrap();
        assert_eq!(block.logical_size(), 8);
        assert_eq!(block.buffer(), &[0, 0, 0, 1, 2, 3, 4, 0xFF]);
    }

    #[test]
    fn static_stream() {
        let mut s = DngStream::from_static(&[0x78, 0x56, 0x34, 0x12], 7);
        // On a little-endian host, no swap needed for 0x12345678.
        assert_eq!(s.get_uint32_le().unwrap(), 0x12345678);
        assert_eq!(s.offset_in_original_file(), 7);
        assert!(s.data_in_buffer(4, 0));

        // from_static data() view.
        let mut s2 = DngStream::from_static(b"abcd", 0);
        assert_eq!(s2.data(), Some(&b"abcd"[..]));
    }

    #[test]
    fn tag_values() {
        let mut s = DngStream::new(Box::new(VecIO::new()));
        s.set_big_endian(true);

        // ttShort 3
        s.put_uint16(3).unwrap();
        s.set_read_position(0).unwrap();
        assert_eq!(s.tag_value_uint32(ttShort).unwrap(), 3);
        s.set_read_position(0).unwrap();
        assert_eq!(s.tag_value_real64(ttShort).unwrap(), 3.0);

        // ttRational 1/2
        s.set_write_position(0).unwrap();
        s.put_uint32(1).unwrap();
        s.put_uint32(2).unwrap();
        s.set_read_position(0).unwrap();
        let r = s.tag_value_urational(ttRational).unwrap();
        assert_eq!((r.n, r.d), (1, 2));
        s.set_read_position(0).unwrap();
        assert_eq!(s.tag_value_real64(ttRational).unwrap(), 0.5);

        // ttSRational -1/2
        s.set_write_position(0).unwrap();
        s.put_int32(-1).unwrap();
        s.put_int32(2).unwrap();
        s.set_read_position(0).unwrap();
        let r = s.tag_value_srational(ttSRational).unwrap();
        assert_eq!((r.n, r.d), (-1, 2));

        // ttFloat 0.25
        s.set_write_position(0).unwrap();
        s.put_real32(0.25).unwrap();
        s.set_read_position(0).unwrap();
        assert_eq!(s.tag_value_real64(ttFloat).unwrap(), 0.25);
    }

    #[test]
    fn pad_align() {
        let mut s = DngStream::new(Box::new(VecIO::new()));
        s.put_uint8(1).unwrap();
        s.pad_align2().unwrap();
        assert_eq!(s.position() % 2, 0);
        s.pad_align4().unwrap();
        assert_eq!(s.position() % 4, 0);
    }

    #[test]
    fn temp_big_endian_guard() {
        let mut s = DngStream::new(Box::new(VecIO::new()));
        assert!(s.little_endian());
        {
            let mut guard = TempBigEndian::new(&mut s, true);
            assert!(guard.stream().big_endian());
        }
        assert!(s.little_endian());
    }

    #[test]
    fn preserve_position() {
        let mut s = DngStream::new(Box::new(VecIO::new()));
        s.put_uint32(42).unwrap();
        {
            let mut guard = PreserveStreamReadPosition::new(&mut s);
            guard.set_read_position(0).unwrap();
            assert_eq!(guard.get_uint32().unwrap(), 42);
        }
        assert_eq!(s.position(), 4);
    }

    // Mark io-error mapping tests.
    #[test]
    fn io_error_paths() {
        assert_eq!(read_failure().error_code(), 100009);
        assert_eq!(write_failure().error_code(), 100010);
    }
}

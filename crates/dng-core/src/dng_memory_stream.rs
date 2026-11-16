// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_memory_stream.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_memory_stream.h/.cpp — a growable in-memory stream.
//
// The C++ keeps a list of fixed-size pages allocated through the (unported)
// allocator abstraction.  The Rust port keeps a `Vec<Vec<u8>>` page list
// with the same page-size semantics (reads/writes never copy the whole
// stream), honoring `length_limit` exactly.  See DESIGN.md.
//
// PORT-NOTE(dng_memory_stream.cpp:196): the C++ `CopyToStream` override
// copies large ranges page-by-page to skip the double buffer.  The Rust port
// relies on the base `DngStream::copy_to_stream` buffered copy (identical
// results, one extra memcpy per block).

use crate::dng_errors::DngResult;
use crate::dng_exceptions::eof_failure;
use crate::dng_stream::{kDefaultBufferSize, DngStream, StreamIO, kDNGStreamInvalidOffset};
use crate::dng_types::{uint32, uint64, uint8};
use crate::dng_utils::max_uint64;
use std::sync::Arc;

/// Port of `dng_memory_stream`.
pub struct DngMemoryStream {
    page_size: uint32,
    pages: Vec<Vec<uint8>>,
    memory_stream_length: uint64,
    length_limit: uint64,
}

impl DngMemoryStream {
    /// `dng_memory_stream (allocator, sniffer, pageSize)`.
    pub fn with_page_size(page_size: uint32) -> Self {
        DngMemoryStream {
            page_size: page_size.max(1),
            pages: Vec::new(),
            memory_stream_length: 0,
            length_limit: 0,
        }
    }

    /// Default page size constructor.
    pub fn new() -> Self {
        // C++ default pageSize = gDNGStreamBlockSize * 8.
        Self::with_page_size(crate::dng_globals::get_dng_stream_block_size() * 8)
    }

    /// `SetLengthLimit (maxLength)` — 0 means no limit.
    pub fn set_length_limit(&mut self, max_length: uint64) {
        self.length_limit = max_length;
    }

    /// Wrap as a buffered `DngStream` driver (consumes this stream).
    pub fn stream(self) -> DngStream<'static> {
        DngStream::with_buffer_size(
            Box::new(self),
            None,
            kDefaultBufferSize,
            kDNGStreamInvalidOffset,
        )
    }

    /// Wrap with a sniffer attached to the driver.
    pub fn into_stream_with_sniffer(
        self,
        sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,
    ) -> DngStream<'static> {
        DngStream::with_buffer_size(
            Box::new(self),
            sniffer,
            kDefaultBufferSize,
            kDNGStreamInvalidOffset,
        )
    }
}

impl Default for DngMemoryStream {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamIO for DngMemoryStream {
    fn get_length(&mut self) -> DngResult<uint64> {
        Ok(self.memory_stream_length)
    }

    fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32> {
        if offset + data.len() as uint64 > self.memory_stream_length {
            return Err(eof_failure());
        }

        let mut offset = offset;
        let mut remaining = data.len();

        while remaining > 0 {
            let page_index = (offset / self.page_size as uint64) as usize;
            let page_offset = (offset % self.page_size as uint64) as usize;

            let block_count = std::cmp::min(self.page_size as usize - page_offset, remaining);

            let dst_index = data.len() - remaining;
            data[dst_index..dst_index + block_count]
                .copy_from_slice(&self.pages[page_index][page_offset..page_offset + block_count]);

            offset += block_count as uint64;
            remaining -= block_count;
        }

        Ok(data.len() as uint32)
    }

    fn set_length(&mut self, length: uint64) -> DngResult<()> {
        if self.length_limit != 0 && length > self.length_limit {
            // C++ throws a *silent* end-of-file error here.
            return Err(eof_failure()); // "dng_memory_stream::fLengthLimit"
        }

        let needed_pages = length.div_ceil(self.page_size as uint64) as usize;

        while self.pages.len() < needed_pages {
            self.pages.push(vec![0u8; self.page_size as usize]);
        }

        self.memory_stream_length = length;

        Ok(())
    }

    fn write(&mut self, data: &[uint8], offset: uint64) -> DngResult<uint32> {
        let new_length = max_uint64(self.memory_stream_length, offset + data.len() as uint64);
        self.set_length(new_length)?;

        let mut offset = offset;
        let mut src = data;

        while !src.is_empty() {
            let page_index = (offset / self.page_size as uint64) as usize;
            let page_offset = (offset % self.page_size as uint64) as usize;

            let block_count = std::cmp::min(self.page_size as usize - page_offset, src.len());

            let (chunk, rest) = src.split_at(block_count);
            self.pages[page_index][page_offset..page_offset + block_count].copy_from_slice(chunk);
            src = rest;

            offset += block_count as uint64;
        }

        Ok(data.len() as uint32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_stream_roundtrip() {
        let mut s = DngMemoryStream::new().stream();

        s.set_little_endian(true);
        for i in 0..10_000u32 {
            s.put_uint32(i).unwrap();
        }
        assert_eq!(s.length(), 40_000);

        s.set_read_position(0).unwrap();
        for i in 0..10_000u32 {
            assert_eq!(s.get_uint32().unwrap(), i);
        }

        // Seek to arbitrary spots.
        s.set_read_position(40_000 - 4).unwrap();
        assert_eq!(s.get_uint32().unwrap(), 9_999);
    }

    #[test]
    fn memory_stream_length_limit() {
        let mut ms = DngMemoryStream::new();
        ms.set_length_limit(16);
        let mut s = ms.stream();
        s.put_zeros(16).unwrap();
        // Small writes are buffered; force the underlying DoSetLength via a
        // flush (matching the C++ where the limit trips in DoSetLength).
        s.flush().unwrap();
        assert!(s.put_zeros(1).is_err() || {
            // The 1-byte write may stay in the buffer; flush to trip.
            s.flush().is_err()
        });
    }

    #[test]
    fn memory_stream_sparse_write() {
        let mut s = DngMemoryStream::new().stream();
        s.set_write_position(100).unwrap();
        s.put(&[7, 8, 9]).unwrap();
        assert_eq!(s.length(), 103);
        s.set_read_position(100).unwrap();
        assert_eq!(s.get(3).unwrap(), vec![7, 8, 9]);
        // Leading bytes are zero-filled.
        s.set_read_position(0).unwrap();
        assert_eq!(s.get(100).unwrap(), vec![0u8; 100]);
    }

    #[test]
    fn memory_stream_page_boundaries() {
        // Force tiny pages to exercise page-spanning reads/writes.
        let mut s = DngMemoryStream::with_page_size(7).stream();
        let data: Vec<u8> = (0..100u8).collect();
        s.put(&data).unwrap();
        s.set_read_position(0).unwrap();
        let out = s.get(100).unwrap();
        assert_eq!(out, data);
        assert_eq!(s.length(), 100);
    }
}

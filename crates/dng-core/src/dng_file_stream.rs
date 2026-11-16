// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_file_stream.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_file_stream.h/.cpp — stream over a file via std::fs.
//
// The C++ offers several constructors (path, FILE*, fd, wide path); the Rust
// port provides the path constructor (`new` for read, `new_output` for
// write) and an explicit `std::fs::File` constructor.  All I/O honors the
// exact error mapping of the C++ (dng_error_open_file / read_file /
// write_file).

use crate::dng_errors::{DngError, DngResult};
use crate::dng_exceptions::{open_file_error, read_failure, write_failure};
use crate::dng_stream::{kDefaultBufferSize, DngStream, StreamIO, kDNGStreamInvalidOffset};
use crate::dng_types::{uint32, uint64, uint8};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};

/// File-backed `StreamIO`.
pub struct FileStreamIO {
    file: File,
    output: bool,
}

impl FileStreamIO {
    /// `dng_file_stream (filename, output = false)`.
    pub fn open(filename: &std::path::Path, output: bool) -> DngResult<Self> {
        let file = if output {
            File::create(filename)
        } else {
            File::open(filename)
        }
        .map_err(|_| open_file_error())?;

        Ok(FileStreamIO { file, output })
    }
}

impl StreamIO for FileStreamIO {
    fn get_length(&mut self) -> DngResult<uint64> {
        let len = self
            .file
            .seek(SeekFrom::End(0))
            .map_err(|_| read_failure())?;
        Ok(len)
    }

    fn read(&mut self, data: &mut [uint8], offset: uint64) -> DngResult<uint32> {
        if self.file.seek(SeekFrom::Start(offset)).is_err() {
            return Err(read_failure());
        }

        let mut total = 0usize;
        while total < data.len() {
            match self.file.read(&mut data[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(read_failure()),
            }
        }

        if total != data.len() {
            return Err(eof_error());
        }

        Ok(total as uint32)
    }

    fn set_length(&mut self, length: uint64) -> DngResult<()> {
        self.file
            .set_len(length)
            .map_err(|_| write_failure())
    }

    fn write(&mut self, data: &[uint8], offset: uint64) -> DngResult<uint32> {
        if self.file.seek(SeekFrom::Start(offset)).is_err() {
            return Err(write_failure());
        }

        let mut total = 0usize;
        while total < data.len() {
            match self.file.write(&data[total..]) {
                Ok(0) => break,
                Ok(n) => total += n,
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(write_failure()),
            }
        }

        if total != data.len() {
            return Err(write_failure());
        }

        Ok(total as uint32)
    }
}

fn eof_error() -> DngError {
    crate::dng_exceptions::eof_failure()
}

/// Port of `dng_file_stream`.
pub struct DngFileStream;

impl DngFileStream {
    /// Open for reading: `dng_file_stream (filename)`.
    pub fn open_read(filename: &std::path::Path) -> DngResult<DngStream<'static>> {
        let io = FileStreamIO::open(filename, false)?;
        Ok(DngStream::with_buffer_size(
            Box::new(io),
            None,
            kDefaultBufferSize,
            kDNGStreamInvalidOffset,
        ))
    }

    /// Open for writing: `dng_file_stream (filename, output = true)`.
    pub fn open_write(filename: &std::path::Path) -> DngResult<DngStream<'static>> {
        let io = FileStreamIO::open(filename, true)?;
        Ok(DngStream::with_buffer_size(
            Box::new(io),
            None,
            kDefaultBufferSize,
            kDNGStreamInvalidOffset,
        ))
    }

    /// Wrap an already-open `std::fs::File`
    /// (`dng_file_stream (file)` C++ constructor).
    pub fn from_file(file: File, output: bool) -> DngResult<DngStream<'static>> {
        if file.metadata().is_err() {
            return Err(open_file_error()); // "Unable to open FILE *"
        }
        Ok(DngStream::with_buffer_size(
            Box::new(FileStreamIO { file, output }),
            None,
            kDefaultBufferSize,
            kDNGStreamInvalidOffset,
        ))
    }
}

#[allow(unused)]
fn unused_type_padding(_: uint32, _: uint64, _: uint8) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stream_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("dng_core_test_{}.tmp", std::process::id()));

        {
            let mut s = DngFileStream::open_write(&path).unwrap();
            s.set_little_endian(true);
            for i in 0..5000u32 {
                s.put_uint32(i).unwrap();
            }
            s.flush().unwrap();
        }

        {
            let mut s = DngFileStream::open_read(&path).unwrap();
            assert_eq!(s.length(), 20_000);
            s.set_read_position(4).unwrap();
            assert_eq!(s.get_uint32().unwrap(), 1);
            s.set_read_position(19_996).unwrap();
            assert_eq!(s.get_uint32().unwrap(), 4_999);
        }

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_missing_file_errors() {
        let err = match DngFileStream::open_read(std::path::Path::new("/nonexistent/dng_test/xyz"))
        {
            Err(e) => e,
            Ok(_) => panic!("expected open failure"),
        };
        assert_eq!(err.error_code(), 100008); // dng_error_open_file
    }
}

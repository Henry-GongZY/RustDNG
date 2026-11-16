// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_memory.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_memory.h/.cpp — memory allocation support.
//
// Design (binding per ARCHITECTURE.md §8): the C++ allocator abstraction is
// NOT ported.  `dng_memory_data` maps to plain `Vec<u8>` and
// `dng_ref_counted_block` to `Arc<Vec<u8>>` at the call sites.  The one
// class that survives is `dng_memory_block` (logical size + padded buffer),
// implemented over `Vec<u8>`.  Because Rust `&[u8]` cannot be reinterpreted
// as `&[u16]` without `unsafe`, the typed `Buffer_uint16()`-style accessors
// are replaced by safe element accessors that read/write native-endian
// values (`get_uint16(i)` / `set_uint16(i, v)`) plus `to_*_vec()` copies.
// See DESIGN.md.

use crate::dng_errors::DngResult;
use crate::dng_safe_arithmetic::safe_uint32_add;
use crate::dng_types::{int16, int32, int64, real32, real64, uint16, uint32};

/// Port of `dng_memory_block`: a logical-size byte buffer with physical
/// padding (the C++ pads by 64 bytes to allow safe SIMD overread; harmless
/// and kept for size parity).
#[derive(Debug, Default, Clone)]
pub struct DngMemoryBlock {
    logical_size: uint32,
    buffer: Vec<u8>,
}

impl DngMemoryBlock {
    /// `dng_malloc_block (logicalSize)` / `Allocate (size)`.
    pub fn new(logical_size: uint32) -> DngResult<Self> {
        // PhysicalSize = logical + 64 (non-AVX path in the C++).
        let physical = safe_uint32_add(logical_size, 64)?;
        Ok(DngMemoryBlock {
            logical_size,
            buffer: vec![0u8; physical as usize],
        })
    }

    /// `LogicalSize ()`.
    pub fn logical_size(&self) -> uint32 {
        self.logical_size
    }

    /// `Buffer ()` — logical portion only.
    pub fn buffer(&self) -> &[u8] {
        &self.buffer[..self.logical_size as usize]
    }

    /// Mutable logical portion.
    pub fn buffer_mut(&mut self) -> &mut [u8] {
        let n = self.logical_size as usize;
        &mut self.buffer[..n]
    }

    /// Full padded buffer (physical size), matching C++ overread allowance.
    pub fn buffer_physical(&self) -> &[u8] {
        &self.buffer
    }

    pub fn buffer_physical_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    // ------------------------------------------------ typed element access
    // Native endian, mirroring the C++ reinterpret-cast Buffer_* accessors
    // on a little/big-endian host respectively.

    pub fn get_uint16(&self, index: usize) -> uint16 {
        let b = &self.buffer[index * 2..index * 2 + 2];
        if crate::dng_flags::qDNGLittleEndian {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        }
    }

    pub fn set_uint16(&mut self, index: usize, v: uint16) {
        let bytes = if crate::dng_flags::qDNGLittleEndian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        self.buffer[index * 2..index * 2 + 2].copy_from_slice(&bytes);
    }

    pub fn get_int16(&self, index: usize) -> int16 {
        self.get_uint16(index) as int16
    }

    pub fn set_int16(&mut self, index: usize, v: int16) {
        self.set_uint16(index, v as uint16);
    }

    pub fn get_uint32(&self, index: usize) -> uint32 {
        let b = &self.buffer[index * 4..index * 4 + 4];
        if crate::dng_flags::qDNGLittleEndian {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        }
    }

    pub fn set_uint32(&mut self, index: usize, v: uint32) {
        let bytes = if crate::dng_flags::qDNGLittleEndian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        self.buffer[index * 4..index * 4 + 4].copy_from_slice(&bytes);
    }

    pub fn get_int32(&self, index: usize) -> int32 {
        self.get_uint32(index) as int32
    }

    pub fn set_int32(&mut self, index: usize, v: int32) {
        self.set_uint32(index, v as uint32);
    }

    pub fn get_int64(&self, index: usize) -> int64 {
        let mut b = [0u8; 8];
        b.copy_from_slice(&self.buffer[index * 8..index * 8 + 8]);
        if crate::dng_flags::qDNGLittleEndian {
            i64::from_le_bytes(b)
        } else {
            i64::from_be_bytes(b)
        }
    }

    pub fn set_int64(&mut self, index: usize, v: int64) {
        let bytes = if crate::dng_flags::qDNGLittleEndian {
            v.to_le_bytes()
        } else {
            v.to_be_bytes()
        };
        self.buffer[index * 8..index * 8 + 8].copy_from_slice(&bytes);
    }

    pub fn get_real32(&self, index: usize) -> real32 {
        f32::from_bits(self.get_uint32(index))
    }

    pub fn set_real32(&mut self, index: usize, v: real32) {
        self.set_uint32(index, v.to_bits());
    }

    // ------------------------------------------------ copies

    pub fn to_uint16_vec(&self) -> Vec<uint16> {
        let n = self.logical_size as usize / 2;
        (0..n).map(|i| self.get_uint16(i)).collect()
    }

    pub fn to_uint32_vec(&self) -> Vec<uint32> {
        let n = self.logical_size as usize / 4;
        (0..n).map(|i| self.get_uint32(i)).collect()
    }

    pub fn to_real32_vec(&self) -> Vec<real32> {
        let n = self.logical_size as usize / 4;
        (0..n).map(|i| self.get_real32(i)).collect()
    }
}

/// Port of `dng_memory_data` shell.  The ARCHITECTURE contract says call
/// sites should use `Vec<u8>` directly; this thin wrapper exists only for
/// code that wants the RAII allocate/clear shape of the C++ class.
pub type DngMemoryData = Vec<u8>;

/// Creates a zero-filled buffer of `size` bytes
/// (C++ `dng_memory_data (uint32 size)`; allocation failure aborts in Rust).
pub fn allocate_memory_data(size: uint32) -> DngResult<Vec<u8>> {
    Ok(vec![0u8; size as usize])
}

/// `dng_ref_counted_block` equivalent: shared, immutable-after-share bytes.
pub type DngRefCountedBlock = std::sync::Arc<Vec<u8>>;

/// `ComputeUpdateID`-style helper used by several call sites to allocate a
/// `DngMemoryBlock` through the (removed) allocator abstraction.  Kept as a
/// free function so phase-2 code ports mechanically.
pub fn allocate_block(logical_size: uint32) -> DngResult<DngMemoryBlock> {
    DngMemoryBlock::new(logical_size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_roundtrip() {
        let mut b = DngMemoryBlock::new(16).unwrap();
        assert_eq!(b.logical_size(), 16);
        assert_eq!(b.buffer().len(), 16);
        b.set_uint16(3, 0x1234);
        assert_eq!(b.get_uint16(3), 0x1234);
        b.set_real32(0, 1.5);
        assert_eq!(b.get_real32(0), 1.5);
        assert_eq!(b.buffer_physical().len(), 16 + 64);
    }
}

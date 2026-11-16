// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_globals.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_globals.h/.cpp — global variables controlling SDK behavior.
//
// C++ mutable globals become atomics (the SDK mutates them from arbitrary
// code, including dng_validate).  Accessors are provided; the statics remain
// public with their C++ names for greppability.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// When validation is turned on, enables verbose output about DNG tags.
pub static gVerbose: AtomicBool = AtomicBool::new(false);

/// Limits the number of lines of text dumped for each tag (verbose mode).
pub static gDumpLineLimit: AtomicU32 = AtomicU32::new(100);

/// Print out results from dng_timers.
pub static gDNGShowTimers: AtomicBool = AtomicBool::new(false);

/// MWG says don't use fake time zones in XMP, but some old software requires
/// them to work correctly.
pub static gDNGUseFakeTimeZonesInXMP: AtomicBool = AtomicBool::new(false);

/// Stream block size.  A size the OS likes for file system efficient
/// read/write alignment.  (C++ default: 4096.)
pub static gDNGStreamBlockSize: AtomicU32 = AtomicU32::new(4096);

/// Maximum stream buffer size to use on large reads and writes.
/// (C++ default: 1024 * 1024.)
pub static gDNGMaxStreamBufferSize: AtomicU32 = AtomicU32::new(1024 * 1024);

/// Are we running as part of the imagecore library?
pub static gImagecore: AtomicBool = AtomicBool::new(false);

/// Print out timing info for area tasks?
pub static gPrintTimings: AtomicBool = AtomicBool::new(false);

/// Print assert messages?  (C++ default: true.)
pub static gPrintAsserts: AtomicBool = AtomicBool::new(true);

/// Break into debugger on asserts?
pub static gBreakOnAsserts: AtomicBool = AtomicBool::new(false);

// ---------------------------------------------------------------- accessors

pub fn get_verbose() -> bool {
    gVerbose.load(Ordering::Relaxed)
}
pub fn set_verbose(v: bool) {
    gVerbose.store(v, Ordering::Relaxed);
}

pub fn get_dump_line_limit() -> uint32 {
    gDumpLineLimit.load(Ordering::Relaxed)
}
pub fn set_dump_line_limit(v: uint32) {
    gDumpLineLimit.store(v, Ordering::Relaxed);
}

pub fn get_dng_show_timers() -> bool {
    gDNGShowTimers.load(Ordering::Relaxed)
}
pub fn set_dng_show_timers(v: bool) {
    gDNGShowTimers.store(v, Ordering::Relaxed);
}

pub fn get_dng_use_fake_time_zones_in_xmp() -> bool {
    gDNGUseFakeTimeZonesInXMP.load(Ordering::Relaxed)
}
pub fn set_dng_use_fake_time_zones_in_xmp(v: bool) {
    gDNGUseFakeTimeZonesInXMP.store(v, Ordering::Relaxed);
}

pub fn get_dng_stream_block_size() -> uint32 {
    gDNGStreamBlockSize.load(Ordering::Relaxed)
}
pub fn set_dng_stream_block_size(v: uint32) {
    gDNGStreamBlockSize.store(v, Ordering::Relaxed);
}

pub fn get_dng_max_stream_buffer_size() -> uint32 {
    gDNGMaxStreamBufferSize.load(Ordering::Relaxed)
}
pub fn set_dng_max_stream_buffer_size(v: uint32) {
    gDNGMaxStreamBufferSize.store(v, Ordering::Relaxed);
}

pub fn get_imagecore() -> bool {
    gImagecore.load(Ordering::Relaxed)
}
pub fn set_imagecore(v: bool) {
    gImagecore.store(v, Ordering::Relaxed);
}

pub fn get_print_timings() -> bool {
    gPrintTimings.load(Ordering::Relaxed)
}
pub fn set_print_timings(v: bool) {
    gPrintTimings.store(v, Ordering::Relaxed);
}

pub fn get_print_asserts() -> bool {
    gPrintAsserts.load(Ordering::Relaxed)
}
pub fn set_print_asserts(v: bool) {
    gPrintAsserts.store(v, Ordering::Relaxed);
}

pub fn get_break_on_asserts() -> bool {
    gBreakOnAsserts.load(Ordering::Relaxed)
}
pub fn set_break_on_asserts(v: bool) {
    gBreakOnAsserts.store(v, Ordering::Relaxed);
}

use crate::dng_types::{uint32, uint8};

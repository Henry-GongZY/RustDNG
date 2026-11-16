// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_flags.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_flags.h — compile-time feature flags.
//
// The C++ SDK resolves most of these via the preprocessor.  In Rust they are
// `pub const bool` values evaluated for this build target.  Only the flags
// that affect behavior of ported code are mirrored here; pure platform
// detection boilerplate (RawEnvironment, faux platform builds, ...) has no
// Rust equivalent and is omitted (see PORTING_NOTES.md).

// ---------------------------------------------------------------- platforms

pub const qMacOS: bool = cfg!(target_os = "macos");
pub const qiPhone: bool = false;
pub const qiPhoneSimulator: bool = false;
pub const qAndroid: bool = false;
pub const qWinOS: bool = cfg!(target_os = "windows");
pub const qWinRT: bool = false;
pub const qLinux: bool = cfg!(target_os = "linux");
pub const qWeb: bool = false;

pub const qIsFauxPlatformBuild: bool = false;
pub const qIsFauxWebPlatformBuild: bool = false;
pub const qIsFauxLinuxPlatformBuild: bool = false;

pub const qMacOSNonFaux: bool = qMacOS && !qIsFauxPlatformBuild;

// ---------------------------------------------------------------- cpu arch

pub const qARM: bool = cfg!(target_arch = "arm") || cfg!(target_arch = "aarch64");
pub const qARM64: bool = cfg!(target_arch = "aarch64");
pub const qX86_64: bool = cfg!(target_arch = "x86_64");

// ---------------------------------------------------------------- debug

/// 1 if debug code is compiled in (assertions and other debug checks).
pub const qDNGDebug: bool = cfg!(debug_assertions);

pub const qDNGIntelCompiler: bool = false;

/// Byte order of the target platform.
pub const qDNGBigEndian: bool = cfg!(target_endian = "big");
pub const qDNGLittleEndian: bool = !qDNGBigEndian;

pub const qDNG64Bit: bool = cfg!(target_pointer_width = "64");

// ---------------------------------------------------------------- features

pub const qDNGThreadSafe: bool = true;

/// 1 if dng_validate command line tool is being built.
pub const qDNGValidateTarget: bool = false;

/// 1 if DNG validation code is enabled.
pub const qDNGValidate: bool = qDNGValidateTarget;

/// 1 if dng_show_message should use fprintf to stderr.
pub const qDNGPrintMessages: bool = qDNGValidate;

/// Experimental features (work in progress for Lightroom and Camera Raw).
pub const qDNGExperimental: bool = true;

pub const qDNGXMPFiles: bool = true;

pub const qDNGXMPDocOps: bool = !qDNGValidateTarget;

/// 1 to use open-source libjpeg for lossy jpeg processing (replaced by the
/// pure-Rust `dng-jpeg` crate; the flag is kept for parity and is false here
/// because dng-core itself has no JPEG support).
pub const qDNGUseLibJPEG: bool = false;

pub const qDNGAVXSupport: bool = false;

pub const qDNGSupportVC5: bool = true;

pub const qDNGUsingSanitizer: bool = false;

// Big image support:
//  true  -> maximum linear image dimensions is 300000 pixels,
//           maximum total number of pixels is 10 gigapixels.
//  false -> maximum linear image dimensions is 65000 pixels,
//           maximum total number of pixels is 512 megapixels.
pub const qDNGBigImage: bool = qDNGExperimental;

pub const qDNGUseXMP: bool = true;

pub const qDNGUseCustomIntegralTypes: bool = false;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endian_flags_are_consistent() {
        assert_eq!(qDNGBigEndian, !qDNGLittleEndian);
    }
}

# dng-core porting notes (deviations from C++ SDK 1.7.1)

Retroactive list for phase 1 (agent A).  Rules: deviations are marked
`// PORT-NOTE(<file>:<line>): reason` at the site and listed here.

## Global

1. **No unsafe.**  All pointer/`reinterpret_cast` idioms are re-expressed
   (typed `PixelData` enum, base-offset pixel indexing, byte-wise tagged
   access in `DngMemoryBlock`).
2. **Allocator abstraction not ported** (dng_memory.h `dng_memory_allocator`,
   `dng_std_allocator`): ARCHITECTURE.md §8 maps `dng_memory_data` →
   `Vec<u8>` and `dng_ref_counted_block` → `Arc<Vec<u8>>`.
   `dng_memory_block` survives as `DngMemoryBlock` over `Vec<u8>` with the
   C++ +64-byte physical padding.
3. **Threading removed** (dng_mutex, dng_pthread not ported): area tasks run
   serially; `kMaxMPThreads` and `PerformAreaTaskThreads` kept for parity.
4. **qDNGValidate-only code omitted** (`Lookup*`/`Dump*` in
   dng_parse_utils.h, `Dump` methods): `qDNGValidate` is 0 in this build.

## Per-file

- **dng_errors.h**: `DngError` mirrors the 1.7.1 codes; ARCHITECTURE.md
  contract names used for variants (`ReadFailure`, `WriteFailure`,
  `ArithmeticOverflow`).  Compatibility variants `NotThorough`, `NotSupported`,
  `Syntax`, `AlreadyExists`, `MissingSubImages`, `Unsupported(String)` have no
  numeric C++ counterpart (`error_code()` returns `dng_error_none`).
- **dng_exceptions.h**: throw helpers become error constructors
  (`read_failure()` etc.).  `throw_dng_error` folds message/sub-message into
  `Unsupported(String)`; the `silent` flag is advisory.
- **dng_string.cpp:786** `Set_JIS_X208_1990`: JIS conversion tables are
  OS-provided in C++ (Mac TextEncoding / Win CP 20932); the port uses the
  C++ non-Mac/non-Win fallback (ASCII extraction via Set_SystemEncoding).
- **dng_string.cpp:208** SystemEncoding: OS API (Mac `TextEncoding`, Win
  `GetACP`) replaced by the Windows-1252 codec (deterministic, matches
  Western-locale `GetACP`); `Get_SystemEncoding` returns the bytes instead of
  NUL-terminating an out-buffer.
- **dng_string.cpp:2151** `Compare`: OS collation (UCCompareTextDefault /
  CompareStringW) replaced by the C++ "other platform" pure-Unicode fallback
  (two passes; second pass case-sensitive), so `"ABC"` vs `"abc"` = -1.
- **dng_string.cpp Get_UTF16**: returns the code units without the trailing
  NUL the C++ out-buffer carried.
- **dng_date_time.cpp:772/838/884** `CurrentDateTimeAndZone`,
  `DecodeUnixTime`, `LocalTimeZone`: OS local-time APIs replaced by UTC
  (civil-from-days).  `DecodeUnixTime` matches the C++ gmtime path exactly;
  local-time paths return a zero offset (documented in the module; callers
  needing true local time set the zone explicitly).
- **dng_date_time.h**: Julian-day helpers do not exist in SDK 1.7.1
  (removed in earlier releases) — nothing to port.
- **dng_stream.cpp:51** `dng_stream (data, count)`: C++ borrows the caller's
  buffer; the port copies into the static IO backend.
- **dng_stream.cpp:355** `Get_uint32_LE` overread: the 3-byte overread slack
  is a performance hack; the port performs an exact 4-byte read (same value).
- **dng_memory_stream.cpp:196** `CopyToStream` page-direct fast path omitted
  (buffered base-class copy; identical results).
- **dng_pixel_buffer.cpp:38** `OptimizeOrder`: loop-order/SIMD optimization
  omitted; every pixel receives the same value (row→col→plane iteration).
- **dng_pixel_buffer.h**: `ConvertPixel/GetPixel/SetPixel` methods named in
  the phase-1 task do not exist in SDK 1.7.1 — cross-type conversion runs
  through `CopyArea` (dng_reference.cpp scalars); "BlackIsOne" is the
  ttSShort 0x8000 sign-bit bias.
- **dng_pixel_buffer**: out-of-range pixel access returns an error (C++ is
  unchecked / has a #if-0 bounds check).
- **dng_image.cpp:1080/1113** `DoCopyArea`/`EqualArea` pointer-identity
  shortcuts: not expressible with `?Sized` generics; the copy/compare is
  performed unconditionally (same final content).
- **dng_image.cpp** tile buffers (`dng_const_tile_buffer`,
  `dng_dirty_tile_buffer`) borrow image memory in C++; the port copies tile
  data into owned buffers (`DngDirtyTileBuffer::flush` writes back
  explicitly).  `DngSimpleImage` exposes direct storage accessors for hot
  paths.
- **dng_image.h `edge_wrap_vertical`/`edge_wrap_all`**: `Get` returns
  `NotYetImplemented` for these exactly like the C++.
- **dng_area_task.cpp**: multi-threaded dispatch removed (serial);
  `dng_range_parallel_task::Do` runs the range once on "thread 0";
  `dng_set_minimum_priority` records the request (OS priority APIs not
  portable).
- **dng_reference.cpp**: IS the scalar reference implementation of the
  bottleneck suite; fully ported for the pixel-buffer/resample/equal-area
  set.  Render-side bottlenecks that need phase-2 parameter types
  (`DoBaselineHueSatMap`, `DoBaselineProfileGainTableMap`, vignette family,
  `DoMapArea16`, `DoBaselineMapPoly32`, `DoBaseline*Color`, RGB-to-RGB
  tables, lossless-JPEG hooks) land in phase 2 with their types.
- **dng_bottlenecks.h** `Do*LosslessJPEG` hooks: phase 2 (dng_lossless_jpeg).
- **dng_fingerprint**: MD5 verified against RFC 1321 vectors; digests are
  byte-identical to the C++.
- **dng_rational.cpp `Set_real64`**: the C++ fall-through after the zero
  guard (final value (0, dd), not (0, 1)) is preserved.
  `ReduceByFactor(0)` is UB in C++; the port returns unchanged.
- **dng_shared.cpp / dng_ifd.cpp**: `dng_camera_profile_info::ParseExtended`
  stops after the magic/version header in phase 1 (extended profile payloads
  — hue-sat deltas, look tables, gain table maps — are consumed by phase-2
  `dng_camera_profile`); offsets/counts are captured.
- **dng_shared.cpp tcImageSequenceInfo**: parsed via the MakeTag layout
  (inline strings + index/count/final).
- **dng_resample.cpp:828** `ResampleImage`: the C++ takes the kernel as a
  parameter; all SDK call sites use `dng_resample_bicubic::Get()`, so the
  port fixes the bicubic kernel.
- **dng_temperature/dng_xy_coord**: ICC-style white points quantize through
  `dng_urational` (32768² denominator) exactly as `SetWhiteXY` does.
- **dng_color_space**: ICC profile blobs extracted verbatim from
  dng_color_space.cpp into dng_color_space_profiles.rs.
- **dng_xmp / dng_exif**: not in phase 1 (phase 2 files in dng-sdk).
- **dng_sdk_limits.h**: `kMaxImageSide` uses the qDNGBigImage=true path
  (300000); `kMaxMPThreads` = 128 (qDNG64Bit) but tasks run serially.
- **dng_host**: phase-2 factory virtuals (`Make_dng_exif`, `Make_dng_xmp`,
  `Make_dng_negative`, `Make_dng_opcode`, `ApplyOpcodeList`,
  `ResampleImage`, JXL encode settings) are implemented in the dng-sdk crate
  which owns those types.
- **dng_1d_table::UnsafeInterpolate**: C++ reads out of bounds for out-of-range
  x; the port clamps to the end entries (callers never rely on the UB).
- **dng_memory_stream length limit**: throws at `DoSetLength` time exactly
  like C++ (small buffered writes trip on flush).

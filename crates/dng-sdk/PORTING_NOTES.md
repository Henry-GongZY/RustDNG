
## Agent B2 (batch 2) porting notes — 2026-11-15

### dng_bad_pixels.rs (finished)
* `FixSingleColumn` ported 1:1 (green split estimator, 7 directions, min/grad
  limit `(minGrad * 3) >> 1`, pin to local G range). `FixSingleRow` reuses the
  column body through a logical-coordinate swap instead of the C++ shallow
  buffer copy with transposed steps (same writes, no clone) — dng_bad_pixels.cpp:1523.
* `DngPixelBuffer` typed storage: the byte-pointer arithmetic of the C++
  (`p0[3 * cs]`) maps to `const_pixel_u16`/`dirty_pixel_u16` with absolute
  coordinates; out-of-range access returns `DngError` where C++ relied on the
  padded SrcArea (defensive `.unwrap_or` only at the neighbor read of the
  constant-fix opcode, where C++ would read within the padded area).

### dng_read_image.rs
* Serial execution everywhere (ARCHITECTURE.md §9): `useMultipleThreads` is
  always false; `DoReadTiles` keeps the C++ signature for API parity.
* The FP-predictor path allocates the C++ "extra leading row" explicitly and
  undoes the byte-plane deltas row by row (dng_read_image.cpp:2407), then
  expands 2-byte halves / 3-byte FP24 samples to f32 storage before `Put`.
* `ReorderSubTileBlocks` permutes typed samples (the C++ permutes raw bytes
  through a temp buffer); the sample-granular plan is layout-identical for
  the interleaved buffers this path is used with (dng_read_image.cpp:808).
* `ifd.fLosslessJPEGBug16` has no dng-core counterpart; moved to
  `DngReadImage.lossless_jpeg_bug16` (caller sets it before `Read`).
* Lossy JPEG decode via `dng-jpeg` (zune-jpeg) replaces libjpeg; JXL via
  `dng-jxl` (jxl-oxide) replaces libjxl. The dng-jxl decoder drops alpha to
  keep the SDK's 1/3-plane contract (dng_read_image.cpp:1866 plane check).
* The stream buffer-size hint (dng_read_image.cpp:3601
  `dng_stream_contiguous_read_hint`) is applied/restored manually because the
  RAII guard borrows the stream for its lifetime, conflicting with passing
  the stream to `ReadTile`.

### dng_update_meta.rs
* `dng_file_updater` is split into `UpdaterState` (bigTIFF/DNG flags,
  truncate-on-failure, zero ranges) + the stream reference, so the
  tag/IFD-updater methods take `(state, stream)` — the C++ passes a single
  `dng_file_updater &` but Rust borrow rules require the split (the
  destructor-truncate semantics moved to `abandon()`).
* `UpdateTag` carries payloads as an enum `TagData { Bytes, U16, U32, U64 }`
  replacing the C++ `const void *` + byteSwapWidth reinterpret casts.
* IPTC in-place updates: the IPTC tag stores byte data under ttLong; the
  port pads to a 4-byte multiple before writing (C++ passes
  `IPTCLength () >> 2` longs of the raw bytes).
* `DngImageSequenceInfo` / `DngImageStats` are re-exported from dng-core
  (Agent A's port of dng_utils.h); this module keeps only the updater
  machinery + `CleanUpMetadataForUpdate` + `DNGUpdateMetadata`.

### dng_image_writer.rs
* Core WriteDNG pipeline only (see the module doc's scope note): WriteTIFF
  non-DNG, profile_tag_set/color_tag_set full surface, gain-map sub-IFD,
  masked RGB tables, semantic masks, preview list and BMFF writer are
  deferred with PORT-NOTEs; `WriteDNG`/`WriteDNGWithMetadata`/
  `WriteDNGUncompressed`/`SetEncodeSettingsForIFD`/
  `MinBackwardVersionForCompression` are complete.
* Strip/tile offsets use ttLong8 via `TagBigUints` and are patched in-stream
  after encoding (`patch_offset_in_ifd`) because dng-core's directory writer
  stores tag values immutably (C++ uses mutable `dng_basic_tag_set`).
* The deflate path applies the predictors exactly like the C++ (horizontal
  difference for ints, byte-plane FP delta big-endian for floats) before
  zlib; the lossless-JPEG path goes through the SDK's own encoder
  (`dng_lossless_jpeg::encode_lossless_jpeg`); ccJXL uses the dng-jxl
  modular encoder at quality 100.
* Tag codes tcUniqueName/tcCFARepeatPatternRows/tcCFARepeatPatternCols are
  defined locally (spec values) until they land in dng-core dng_tags.

### dng_render.rs
* The ACR3 default tone-curve tables are extracted verbatim from the C++
  (1025-entry forward + inverse tables, `acr3_tables` submodule).
* The baseline bottleneck math (gDNGSuite reference implementations in
  dng_reference.cpp) is inlined: ABC/ABCD→RGB, HueSatMap (2.5D + 3D with
  encode/decode tables), RGB→Gray/RGB, 1D table, RGBTone (+ overrange
  variants), ProfileGainTableMap trilinear.
* MaskedRGBTables (DNG 1.7) is not applied — needs dng_reference
  RGB-table machinery + profile APIs (documented `NotSupported` note).
* The negative-attached PGTM is passed to the task through a static
  holder (`PGTM_HOLDER`) for the duration of `render()` — a documented
  deviation from the C++ task holding a negative reference.
* The profile tone curve is stored as a leaked `'static DngPiecewiseLinear`
  (the C++ keeps it on the render object; the task's tone-curve slot is a
  'static function reference).

### dng_lens_correction.rs (TEMP stub — Agent A owns the real port)
* Provides the four opcode types referenced by A's registry; `from_stream`
  reads the payload size and skips the payload so opcode-list parsing keeps
  stream positions; `apply`/`process_area` return `NotSupported`.

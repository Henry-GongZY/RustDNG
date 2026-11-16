# dng-sdk INTEGRATION NOTES — Agent B (batch 2)

Appended by Agent B2 (2026-11-15). Covers cross-agent mismatches discovered
while porting `dng_read_image`, `dng_update_meta`, `dng_image_writer`,
`dng_render`, and finishing `dng_bad_pixels`.

## Status snapshot (batch 2)

* **Compiles clean (B-owned)**: `dng_bad_pixels`, `dng_read_image`,
  `dng_update_meta`, `dng_image_writer`, `dng_render`, `dng_lens_correction`
  (TEMP stub), `dng_negative` (TEMP stub — superseded, see below).
* **Blocked on A**: as of this note the crate does not link because
  `dng_negative.rs` / `dng_misc_opcodes.rs` / `dng_camera_profile.rs` /
  `dng_xmp.rs` / `dng_opcodes.rs` / `dng_opcode_list.rs` are mid-edit on A's
  side (transient errors). All B modules are written against the
  NEGATIVE_API.md contract and A's *real* `dng_negative.rs` API once landed.

## Seam history — IMPORTANT for integration

1. Agent B2 first created `src/dng_negative.rs` as a **TEMPORARY SEAM STUB**
   (clearly marked) because batch 2 needed the negative to compile. Agent A
   subsequently replaced it with the full port (A backed up the stub — thanks
   — and the backup comment lives at the top of the real file).
2. Agent B2 likewise created `src/dng_lens_correction.rs` as a TEMP stub
   (Agent A owns the real port per NEGATIVE_API.md). The stub provides
   `DngOpcodeWarpRectilinear / WarpRectilinear2 / WarpFisheye /
   FixVignetteRadial` implementing the `DngOpcode`/`DngFilterOpcode` traits;
   `from_stream` parses+skeeps the payload so opcode-list parsing keeps
   stream positions correct, `apply` returns `NotSupported`. **Agent A
   should replace the file wholesale**; nothing else in B's code depends on
   its internals (only `crate::dng_opcodes` registry references the types).
3. lib.rs: B appended its own module declarations plus — during the window
   when A had not yet declared them — `pub mod dng_opcode_list; pub mod
   dng_opcodes; pub mod dng_misc_opcodes;` (needed by `dng_bad_pixels`).
   **If A appends their own copies, remove one set** (duplicate `pub mod`
   is a compile error). B marked the section accordingly.

## API mismatches found and resolved (B-side adaptations)

* `dng_core::DngHost`, `dng_core::max_uint32` etc. are **not** flat
  re-exported from the dng-core root — A's `dng_opcode_list.rs` /
  `dng_opcodes.rs` import them that way (their transient errors; B's files
  always use the module path, e.g. `dng_core::dng_host::DngHost`).
* `dng_core::dng_utils` now hosts `DngImageSequenceInfo` / `DngImageStats` /
  `WeightedSample` / `ColorSample` (A ported them from dng_utils.h).
  `dng_update_meta` re-exports them under its own name so the
  NEGATIVE_API.md note ("they live in dng_update_meta.rs") keeps working for
  callers; the definitions in dng-core are authoritative.
* `DngTagOwnedDataPtr::new` takes `Arc<Vec<u8>>` (not `Vec<u8>`); the writer
  wraps with `std::sync::Arc::new` via the `owned_tag` helper.
* `DngXmp::serialize(as_packet, target_bytes, pad_bytes, for_jpeg, compact)`
  — 5-arg facade; `DNGUpdateMetadata` maps C++
  `Serialize (true, existingSize)` onto `(true, existing_size, 0, false,
  false)`.
* `DngXmp` is a non-optional facade (`has_meta()` instead of
  `Option<DngXmp>`); `DngMetadata.exif` is a plain `DngExif` field.
* `DngNegative::set_stage1_image / set_stage3_image / set_depth_map /
  set_raw_lossy_compressed_image / set_transparency_mask` take
  `Option<...>` arguments (B's read-stage wrappers adapted).
* `DngNegative::make_color_spec(id, allow_stubbed)` and
  `get_profile_by_id(id, &mut profile, use_default_if_no_match,
  group_selector) -> bool` — B's render adapted.
* `DngIfd` in dng-core has no `fLosslessJPEGBug16` field; the flag moved to
  `DngReadImage.lossless_jpeg_bug16` (set by the negative wrapper before
  `Read`) — PORT-NOTE in dng_read_image.rs.
* Tag codes `tcUniqueName` (50708), `tcCFARepeatPatternRows` (50810),
  `tcCFARepeatPatternCols` (50811) were missing from dng-core `dng_tags`
  when the writer was built; the writer defines them locally with spec
  values. **When A adds them to dng-core, switch the writer imports back**
  (search for "not yet present in dng-core" in dng_image_writer.rs).

## Deferred C++ features (documented, not silently dropped)

All marked with PORT-NOTE + listed in PORTING_NOTES.md:

* `dng_image_writer`: `WriteTIFF` non-DNG path, full `profile_tag_set` /
  `color_tag_set`, gain-map sub-IFD, masked-RGB-table profiles, semantic
  masks, preview-list writing, BMFF (`WriteBMFF`) — the DNG core path is
  complete.
* `dng_render`: DNG 1.7 `MaskedRGBTables` (needs `dng_reference`
  RGB-table machinery + profile APIs), `dng_image::edge_repeat` mask reads
  work but the semantic-mask compositing does not.
* `dng_read_image`: multi-threaded `dng_read_tiles_task` executes serially
  (ARCHITECTURE.md §9); `DoReadTiles` keeps the C++ signature.

## Test coverage added (B)

* `dng_bad_pixels`: bad-pixel list stream round-trip, FixSingleColumn on a
  synthetic defective column, FixSingleRow transpose, constant-opcode
  round-trip.
* `dng_update_meta`: `DngImageStats` make/parse round-trip with the four
  official PGTM child-tag shapes + error paths,
  `DngImageSequenceInfo` round-trip, `DngFileUpdater` in-place string-tag
  update on a synthetic TIFF (header parse → patch → re-parse verify).
* `dng_read_image`: uncompressed 16-bit read, deflate round-trip
  (horizontal predictor + zlib), lossless-JPEG round-trip (SDK encoder →
  SDK decoder), PackBits, delta decoders, LZW smoke.
* `dng_image_writer`: synthetic negative → WriteDNGUncompressed → header +
  IFD0 re-parse (DNGVersion tag bytes), compression-selection semantics.
* `dng_render`: synthetic RGB float stage3 → render → shape/type checks,
  overrange encode/decode inverse property, ACR3 curve endpoints.

## Notes for the integration phase

* The writer emits `TagBigUints` (ttLong8) for strip/tile offsets and then
  patches the two values in-stream (`patch_offset_in_ifd`) — the C++ uses
  mutable `dng_basic_tag_set` entries. If A later lands a mutable tag set,
  the patch step can be dropped.
* `dng_update_meta::DNGUpdateMetadata` triggers the truncate-on-failure
  destructor semantics explicitly (`updater.abandon()` on error paths);
  callers should keep that pattern.

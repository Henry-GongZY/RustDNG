# Phase-2 seam contract: dng-sdk crate (Agent A ↔ Agent B)

All code lands in `/Users/shinano/Opensource/RustDNG/crates/dng-sdk/src/` as modules named after the C++ files. Full method-level enumeration is done **from the C++ headers** by the implementing agent — 1:1 porting, no API drops (rules in /Users/shinano/Opensource/RustDNG/ARCHITECTURE.md). This file fixes only the cross-agent types and call directions.

## Fixed cross-boundary types

```rust
// dng_metadata (dng_metadata.h/cpp — Agent A)
pub struct DngMetadata {
    pub exif: DngExif,                  // dng_exif (A)
    pub iptc: DngIptc,                  // dng_iptc  (A)
    pub xmp: DngXmp,                    // dng_xmp facade (A) — wraps Option<dng_xmp::XmpMeta>
    pub origin: DngMetadataOrigin? ,     // per dng_metadata.h fields: has_origin etc.
    // all remaining dng_metadata.h fields 1:1 (BaseOrientation, DocName, IsMainDocument ...)
}
pub struct DngXmp { pub sdk: Option<dng_xmp::XmpMeta> }   // facade named after dng_xmp.cpp class dng_xmp
```

```rust
// dng_negative (Agent A implements; B consumes). Mirrors dng_negative.h 1:1 (~368 methods):
pub struct DngNegative {
    // every field of dng_negative.h; C++ AutoPtr<T> → Option<Box<T>>; dng_image refs → Option<ImageRef>
    // (ImageRef = Arc<RwLock<dyn DngImage>> from dng_core::dng_image — binding decision 2026-11-12)
    // camera profiles: Vec<DngCameraProfile>; opcode lists: DngOpcodeList
    // big tables (BigTableIndex/Dictionary/GroupIndex) live in dng_big_table.rs (B) — DngNegative holds
    //   the concrete types B defines; A declares the field, B implements the type; agree on names:
    //   pub struct DngBigTableIndex { ... }  (B)
    // metadata: DngMetadata
}
impl DngNegative {
    // 1:1 methods from dng_negative.h in snake_case; constructors:
    pub fn new(host: &mut DngHost) -> DngResult<Self>;      // C++ dng_negative::New + initializer
    // Parse hooks used by dng_info (A calls own methods): parse_*, post_parse, SynchronizeMetadata
    // Stage pipeline: build_stage1/2/3, read_stage1_image / read_stage2_image / read_stage3_image,
    //   need_image? etc. — the read_* methods DELEGATE to dng_read_image (B):
    //   pub fn read_stage1_image(&mut self, host:&mut DngHost, stream:&mut DngStream) -> DngResult<()>
    //     → calls crate-local `dng_read_image::read_stage1_image(host, stream, self, ...)` (B).
    //   To avoid cyclic blocking: B implements dng_read_image as free functions taking
    //   &mut DngNegative; A's methods call them via `super::dng_read_image::...`.
}
```

```rust
// dng_lossy_compressed_image (dng_image.h — B)
pub struct DngLossyCompressedImage { /* fields per dng_negative.h usage: data, dimensions, tile info */ }
```

```rust
// opcode layer (A): DngOpcodeList (dng_opcode_list), DngOpcode trait + all opcode classes
// (dng_opcodes.cpp registry, dng_misc_opcodes.cpp, dng_lens_correction.cpp) with
// `pub trait DngOpcode { fn id(&self)->u32; fn parse/apply... }` mirroring C++ virtuals.
```

## Ownership (files → agent)
**A**: dng_exif, dng_iptc, dng_xmp, dng_xmp_sdk (facade over dng-xmp crate), dng_metadata (in dng_negative.rs or own file — A decides, document), dng_camera_profile, dng_color_spec, dng_hue_sat_map, dng_tone_curve, dng_gain_map, dng_opcode_list, dng_opcodes, dng_misc_opcodes, dng_lens_correction (opcode classes only), dng_info, dng_mosaic_info, dng_linearization_info, dng_negative, dng_noise_profile (in negative or own file per C++ header layout), dng_exif types incl. dng_exif_parser? (A).

**B**: dng_image_writer, dng_read_image, dng_render, dng_preview, dng_jpeg_image, dng_lossless_jpeg (+ dng_lossless_jpeg_shared), dng_jxl (glue over dng-jxl crate), dng_big_table, dng_bad_pixels, dng_update_meta, dng_bmff, dng_lens_correction encode-side helpers if split, plus crates/dng-sdk/src/lib.rs assembly of re-exports.

## Call directions (no cycles at file level)
- `dng_info` (A) parses stream→ifds→negative (A).
- `DngNegative::read_stageN_image` (A methods) → `dng_read_image::*` (B fns) → codec crates (B crates) → negative getters/setters (A).
- `DngImageWriter::write_dng` (B) → negative/metadata getters (A) + opcode writer hooks (A's DngOpcodeList::write) + previews (B).
- `DngRender` (B) → negative (A) + dng-core resample/tables.
- Any signature mismatch discovered by B against A's real code: do NOT edit A's files; record in crates/dng-sdk/INTEGRATION_NOTES.md (B) and continue; main agent resolves.

## Sequencing
1. Agent A first lands: dng_exif, dng_iptc, dng_xmp(+sdk facade), dng_camera_profile, dng_hue_sat_map, dng_tone_curve, dng_gain_map, dng_color_spec, opcode types, then dng_negative (+DngMetadata), dng_mosaic_info, dng_linearization_info, dng_info.
2. Agent B first lands: dng_lossless_jpeg(+shared), dng_jpeg_image, dng_jxl glue, dng_big_table, dng_bad_pixels, dng_update_meta, dng_preview, dng_read_image, dng_image_writer, dng_render.
3. A may start before B's phase-1 crates finish: dng-xmp crate API is already fixed in ARCHITECTURE.md — code the facade against it. Same for dng-jpeg/dng-jxl signatures.
4. Integration (main agent): assemble lib.rs re-exports, run sample-file tests, resolve INTEGRATION_NOTES.md.

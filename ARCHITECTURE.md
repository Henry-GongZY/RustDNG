# RustDNG — Adobe DNG SDK 1.7.1 → Pure Rust Port

## Mission
Rewrite the complete Adobe DNG SDK 1.7.1 (reference source: `vendor/extracted/dng_sdk_1_7_1/dng_sdk/source/`, 73 .cpp + 86 .h ≈ 152k LOC) in **100% pure Rust**. No C/C++ linking, no `-sys` crates, no bindgen. Every external dependency (libjpeg, libjxl, XMP Toolkit, zlib) is replaced by a pure-Rust implementation or pure-Rust crate.

Dependency verdict (verified: workspace dependency tree contains zero `cc`/`cmake`/`*-sys` packages):

| C++ dep | Pure Rust replacement | Where |
|---|---|---|
| libjpeg encode | `jpeg-encoder` 0.6 | crate `dng-jpeg` |
| libjpeg decode | `zune-jpeg` 0.4 | crate `dng-jpeg` |
| zlib (deflate) | `flate2`/`miniz_oxide` | `dng-core::zlib` |
| libjxl decode | `jxl-oxide` 0.11 | crate `dng-jxl` |
| libjxl encode | **own implementation** (modular-mode JXL encoder) | crate `dng-jxl` |
| XMP Toolkit | **own implementation** (XMP data model + RDF/XML engine) | crate `dng-xmp` |
| SDK's own lossless JPEG (cc 34892) | port of `dng_lossless_jpeg_shared.cpp` (SDK source, no dep) | phase 2, `dng-sdk` |

## Workspace layout
```
crates/
  dng-core/      foundation library (Agent A, phase 1)
  dng-jpeg/      JPEG facade (Agent B, phase 1)
  dng-jxl/       JXL decode+encode (Agent B, phase 1)
  dng-xmp/       XMP engine (Agent B, phase 1)
  dng-sdk/       the SDK proper: negative/exif/iptc/xmp-glue/info/read_image/image_writer/render/preview/opcodes/big_table/bad_pixels/lens_correction/update_meta/jpeg_image/lossless_jpeg/mosaic (phase 2: A = metadata+negative, B = writer+render+codecs glue)
  dng-validate/  port of dng_validate.cpp CLI (integration phase)
vendor/extracted/dng_sdk_1_7_1/   reference C++ source (NEVER modify; read-only)
```

## Non-negotiable conventions (both agents MUST follow)

1. **Module ↔ file mapping**: `dng_foo.cpp` → module `dng_foo` (file `src/dng_foo.rs`). Keep all public names greppable against C++.
2. **Type names**: Rust UpperCamelCase of C++ names: `dng_stream` → `DngStream`, `dng_negative` → `DngNegative`. Keep the `Dng` prefix.
3. **Constants keep C++ spelling verbatim**: `tcNewSubFileType`, `ccDeflate`, `ptRGB`, `cvDNGVersion1_4` … declared as `pub const` with `#![allow(non_upper_case_globals)]`. This preserves 1:1 traceability with `dng_tags.h` / `dng_tag_values.h`.
4. **Methods**: snake_case of the C++ name: `ParseTag` → `parse_tag`, `GetImageInfo` → `get_image_info`. Field `fFooBar` → `foo_bar` (drop the `f` prefix).
5. **Errors**: C++ `dng_exception` throws → `Result<T, DngError>`. `DngError` enum mirrors `dng_errors.h` codes (`Unknown, UserCancel, Memory, EndOfFile, ReadFailure, WriteFailure, BadFormat, NotThorough, NotSupported, Syntax, AlreadyExists, ArithmeticOverflow, MissingSubImages, Unsupported(String)`). Helper fns in `dng_exceptions.rs`: `read_failure() -> DngError` etc. `dng_abort_sniffer`'s `ThrowUserCancel` → `Err(DngError::UserCancel)`. Public SDK APIs return `DngResult<T>`; internal helpers may use `Option`/panics where C++ asserts (`dng_assert` → `debug_assert!`/`panic!`).
6. **Rationals**: `DngURational` (u32/u32), `DngSRational` (i32/i32), `DngRational` (i64/i64) with `Set_to_real32`-style methods matching C++ behavior exactly.
7. **Half float**: `half::f16` everywhere `ttFloat16`/`real16` appears.
8. **Memory blocks**: `dng_memory_data` / `dng_ref_counted_block` → plain `Vec<u8>` / `Arc<Vec<u8>>`; do NOT port the allocator abstraction. `dng_pixel_buffer` stores bytes in a `Vec<u8>` and exposes typed accessors (`as_u16_slice`, `as_f32_slice`, …) — keep `Vec` alignment correct via `u16`/`u32`/`f32` typed storage where needed (see Agent A design note below).
9. **Threading**: port `dng_area_task` API but execute serially (single-threaded loop) by default; document deviation. `dng_mutex`/`dng_pthread` → `std::sync` primitives.
10. **Streams**: `DngStream` wraps `Box<dyn StreamIO>`; sub-streams via `DngMemoryStream`/offset-limited wrappers. All typed `Get`/`Put` accessors (get_u8/get_u16/get_u32/get_u64/get_f16/get_f32/get_f64/get_string/get_tag_data … put_*) honoring per-stream endianness (qEndIndicator logic), plus `get_position/set_position/get_length/set_length/seek_relative`.
11. **Porting fidelity**: algorithms, bit-layouts, byte orders, rounding, default values, tag IDs, error paths must match C++ exactly. When a simplification is unavoidable, mark it `// PORT-NOTE(<file>:<line>): reason` and list it in the crate's `PORTING_NOTES.md`. Never silently drop functionality — if something can't be done, keep the API and return `DngError::NotSupported` + PORT-NOTE.
12. **Unsafe**: forbidden except with a written justification in PORTING_NOTES.md (target: zero `unsafe`).
13. **Testing**: every crate must `cargo build` clean and `cargo test` pass. Unit tests near code; port-accuracy tests where C++ has known outputs (e.g. color space primaries, tone curves, string parsing).
14. **Never modify** anything under `vendor/`. Never add dependencies that are not already vetted in ARCHITECTURE.md without checking they are pure Rust (`cargo tree` must not gain `cc`/`cmake`/`*-sys`).
15. Each crate root: `.rs` files mirror .cpp file names; crate-level attributes `#![allow(non_upper_case_globals)]` (and `#![allow(non_snake_case)]` only where needed).

## Boundary contract — dng-core public API (Agent A)
Crate `dng-core`, deps allowed: `half`, `flate2`, `crc32fast` (all pure Rust).

Modules (in port priority order):
- **infra**: `dng_flags` (feature constants as `pub const bool`, e.g. `qDNGXMPDocOps=false`... mirror what's relevant), `dng_types` (type aliases: `int32`, `uint32`, `real64`… kept for fidelity), `dng_errors`, `dng_exceptions`, `dng_memory` (`DngMemoryBlock`), `dng_utils` (bit ops, Clip/Round/LimDiff/Abs_float etc., pin-down exact float behavior), `dng_safe_arithmetic`, `dng_globals`.
- **strings/dates**: `dng_string` (full port: encoding-aware `Set_SystemEncoding`, `Set_UTF8/16`, `Get_UTF8`, numeric getters, `Is_Filename`, `TrimLeadingWhitespace` …), `dng_string_list`, `dng_local_string`, `dng_date_time` (full: parse/encode ISO/EXIF formats, `Encode_XMP`, Julian day, time zone), `dng_rational`.
- **geometry**: `dng_point`, `dng_rect` (operators as methods: `and_`, `or_`, `intersect`, `Not_zero` …).
- **streams**: `dng_stream`, `dng_memory_stream`, `dng_file_stream` (std::fs backed), plus `zlib` module (flate2 `ZlibEncoder/Decoder`, both zlib-wrapped and raw variants — check each C++ call site's windowBits) and `crc32` helper (crc32fast).
- **tags/IFD**: `dng_tag_types` (`dng_tag_type` enum + `TagTypeSize`, `TagTypeValue`), `dng_tag_values` (ALL enums/constants verbatim), `dng_tags` (ALL tag code constants tcXXX + `dng_tag_code`/`dng_tag_signature`), `dng_tiff_directory` (full writer port: `dng_tiff_directory`, `dng_tiff_entry`, `dng_tiff_tag`, subdirectories), `dng_ifd` (full: `dng_ifd`, `dng_ifd_info`?? no — `dng_ifd` + `dng_preview_info` + `dng_camera_profile_info` + `dng_mask_info` + `dng_ifd::ParseTag` covering every tag in dng_ifd.cpp), `dng_parse_utils` (all parse helpers), `dng_shared` (constants + helpers used by writer).
- **images**: `dng_pixel_buffer` (full port incl. `ConvertPixel`/`SetPixel`/copy/offset logic), `dng_image` (trait + `DngImageIterator` helpers + shared-buffer design), `dng_simple_image`, `dng_tile_iterator`, `dng_area_task` (serial), `dng_filter_task`, `dng_resample` (weights, `dng_resample_function`, `dng_resample_bicubic`, `dng_resample_downscale`), `dng_bottlenecks`.
- **math/tables**: `dng_1d_function` (+identity/gamma/curve), `dng_1d_table`, `dng_2d_table`, `dng_spline`, `dng_matrix` (all 3x3/3x4/2x3 ops, inverse, `Round`/`Set_ident`…), `dng_xy_coord` (xy/uv/XYZ, `StdCameraXYZ`, illuminants), `dng_temperature`, `dng_color_space` (ALL built-in spaces with their EXACT C++ coefficient tables — copy values verbatim), `dng_orientation`.
- **misc**: `dng_fingerprint` (md5! port pure-Rust MD5 — must produce identical digests), `dng_host` (struct with `needs_meta_preserved`, `negatives_supported`… keep method parity; sniffer via `Box<dyn AbortSniffer>`), `dng_abort_sniffer` (trait + null sniffer).

Design notes (binding):
- `DngPixelBuffer`: store data as `Vec<u8>`; provide `pixel_type: DngTagType` and typed views: `fn as_u8_slice(&self)->&[u8]`, `as_u16_slice`, `as_s16_slice`, `as_u32_slice`, `as_f16_slice`, `as_f32_slice`, and `*_mut` variants. Row/col/plane steps as `i32` matching C++ (`fRowStep` etc.). `ConvertPixel`/`SetPixel`/`GetPixel` semantics (incl. BlackIsOne inversion, white-level clamp) must match `dng_pixel_buffer.cpp`.
- `dng_image`: `pub trait DngImage` with `pixel_type()`, `planes()`, `width()`, `height()`, `bounds()`, `pixel_size()`, `get_pixel_buffer(&self) -> DngPixelBuffer`, `put_pixel_buffer(&mut self, buf: &DngPixelBuffer) -> DngResult<()>`, `get_pixel(x,y,plane) -> PixelValue` etc. `DngSimpleImage` implements it. Where C++ passes `dng_image_ref` around, Rust uses `&mut dyn DngImage` or `Arc<Mutex<dyn DngImage>>` — Agent A decides and documents; later crates follow A's choice. Note: pixel_type includes ttFloat16/32.
- Keep `dng_ifd` parsing able to consume `DngStream`; `dng_tiff_directory` writer must be usable without `dng_negative` (standalone), emitting correct types/counts/ordering per dng_tiff_directory.cpp.

## Boundary contract — codec crates (Agent B)

### crate `dng-jpeg` (deps: `jpeg-encoder`, `zune-jpeg`)
```rust
pub enum Photometric { Gray /*BlackIsZero*/, RGB, YCbCr }
pub struct JpegImage { pub width: u32, pub height: u32, pub components: u32 /*1 or 3*/, pub bits: u32 /*8*/, pub data: Vec<u8> /*interleaved*/, pub photometric: Photometric }
pub struct JpegEncodeOptions { pub quality: u32 /*0..=100, libjpeg-style IJG mapping*/, pub subsampling: JpegSubsampling /* Gray|S444|S422|S420, default per photometric */, pub progressive: bool, pub optimize_coding: bool, pub x_dpi: u32, pub y_dpi: u32 /*JFIF density*/, pub icc_profile: Option<Vec<u8>> }
pub fn encode_jpeg(img: &JpegImage, opts: &JpegEncodeOptions) -> DngResult<Vec<u8>>;   // DngError re-exported from dng-core? NO — codec crates must NOT depend on dng-core. Use their own small error enum `JxlError`/`XmpError`/`JpegError` with `impl From` at the dng-sdk glue layer.
pub struct JpegDecoded { pub width: u32, pub height: u32, pub components: u32, pub bits: u32, pub data: Vec<u8> /*interleaved, converted: gray or RGB*/, pub icc_profile: Option<Vec<u8>> }
pub fn decode_jpeg(bytes: &[u8]) -> DngResult<JpegDecoded>;
pub fn jpeg_dimensions(bytes: &[u8]) -> DngResult<(u32, u32, u32)>; // w,h,components from SOF
pub fn jpeg_is_progressive(bytes: &[u8]) -> bool;
```
Quality mapping note: `jpeg-encoder` uses quality 0-100 same scale as libjpeg; verify visually/numerically via round-trip tests (encode@q85, decode, PSNR sanity).

### crate `dng-jxl` (deps: `jxl-oxide` for decode; encoder = own code, may use `jxl-bitstream`-style hand-rolled bit IO — must stay pure Rust)
```rust
pub struct JxlImage { pub width: u32, pub height: u32, pub channels: u32 /*1 gray, 3 rgb, 4 rgba*/, pub bits: u32 /*8,16 (int) or 32 (float)*/, pub data: Vec<u8> /*interleaved little-endian*/ }
pub struct JxlImageInfo { pub width: u32, pub height: u32, pub bits_per_sample: u32, pub is_float: bool, pub channels: u32, pub alpha: bool, pub uses_original_profile: bool, pub icc: Option<Vec<u8>>, pub exif: Option<Vec<u8>>, pub xmp: Option<Vec<u8>> }
pub fn jxl_image_info(bytes: &[u8]) -> DngResult<JxlImageInfo>;
pub fn decode_jxl(bytes: &[u8]) -> DngResult<JxlDecodedImage>; // pixels in native bit depth (u8/u16 or f32), icc + boxes if present
pub struct JxlEncodeOptions { pub quality: f32 /*0..100, 100 => lossless; map to modular/VarDCT decision like libjxl distance mapping: q>=100 lossless modular, else approximate via modular quantization or document approach*/, pub effort: u8 /*1..9, default 7; may be ignored with PORT-NOTE*/, pub bits_per_sample: u32, pub is_float: bool, pub channels: u32, pub exif: Option<Vec<u8>>, pub xmp: Option<Vec<u8>>, pub icc: Option<Vec<u8>> }
pub fn encode_jxl(img: &JxlImage, opts: &JxlEncodeOptions) -> DngResult<Vec<u8>>;
```
Encoder requirements:
- Output MUST be decodable by `jxl-oxide` (round-trip test in CI) and structurally valid per ISO 21496-1/JXL spec: signature, SizeHeader, ImageMetadata (bit_depth, color encoding; WantICC + ICC box when icc provided), frame(s) in **modular mode** (no VarDCT — document), MA tree, prefix-coded residuals. Use `jxl-oxide`'s sub-crate sources (`jxl-modular`, `jxl-bitstream`, `jxl-frame`) as the *bitstream ground truth* while implementing.
- MUST support the JXL **BMFF container** form (ftyp `jxl ` + `jxlc`/`jxlp` + `Exif`/`xml `/`colr` boxes) because DNG previews embed Exif/XMP/ICC in container boxes (see dng_jxl.cpp + JXL_ReadMe). Provide container both ways if cheap (`encode_jxl` emits container when exif/xmp/icc present, else bare codestream).
- Decoder must handle: container or bare codestream, integer 8/16 and float32 samples, gray/rgb(+alpha), ICC extraction, Exif/XMP box extraction (jxl-oxide exposes box accessors; if it doesn't, parse BMFF boxes manually).

### crate `dng-xmp` (deps: `quick-xml`; NO other heavy deps)
Pure-Rust XMP engine equivalent to the subset of the Adobe XMP Toolkit used by DNG SDK (survey `dng_xmp_sdk.cpp` + `dng_xmp.cpp` + `dng_exif.cpp` for the exact API surface before designing).
```rust
pub struct XmpMeta { /* node tree */ }
impl XmpMeta {
    pub fn new() -> Self;
    pub fn parse_from_buffer(data: &[u8]) -> DngResult<Self>;  // accepts full packet, x:xmpmeta wrapper, or bare rdf:RDF
    pub fn serialize_to_buffer(&self, opts: &XmpSerializeOptions) -> DngResult<Vec<u8>>; // <?xpacket begin…?>…<?xpacket end?> with padding, newline rules per XMP spec Part 3
    pub fn get_property(&self, schema_ns: &str, prop: &str) -> Option<String>;
    pub fn set_property(&mut self, schema_ns: &str, prop: &str, value: &str);
    pub fn does_property_exist(&self, schema_ns: &str, prop: &str) -> bool;
    pub fn delete_property(&mut self, schema_ns: &str, prop: &str);
    pub fn get_struct_field(&self, schema_ns: &str, prop: &str, field_ns: &str, field: &str) -> Option<String>;
    pub fn set_struct_field(&mut self, schema_ns: &str, prop: &str, field_ns: &str, field: &str, value: &str);
    pub fn get_array_item(&self, schema_ns: &str, prop: &str, index: usize /*1-based*/) -> Option<String>;
    pub fn set_array_item(&mut self, schema_ns: &str, prop: &str, index: usize, value: &str);
    pub fn append_array_item(&mut self, schema_ns: &str, prop: &str, ordered: bool, value: &str); // handles first-item setup like kXMP_PropArrayIsOrdered
    pub fn count_array_items(&self, schema_ns: &str, prop: &str) -> usize;
    pub fn get_localized_text(&self, schema_ns: &str, prop: &str, lang: &str) -> Option<String>;
    pub fn set_localized_text(&mut self, schema_ns: &str, prop: &str, lang: &str, value: &str); // x-default rules per XMP spec
    pub fn set_object_name(&mut self, about: &str); pub fn object_name(&self) -> String;
    pub fn register_namespace(prefix: &str, uri: &str); pub fn namespace_prefix(&self, uri: &str) -> Option<String>;
}
pub struct XmpSerializeOptions { pub read_only_packet: bool, pub use_compact_syntax: bool, pub padding: usize, pub include_padding: bool, pub new_line: String /* "\n" */ }
```
Requirements:
- Path syntax subset: `prop` = `ns:root`, `ns:root[i]` (1-based arrays), `ns:root/ns:field` (structs), combos; qualifiers `?ns:q` if used by SDK (check).
- RDF/XML serialization must be re-parseable by itself AND by any standard XMP reader: correct `rdf:Description` grouping per schema ns, `rdf:parseType="Resource"` for structs, `rdf:Bag/Seq/Alt` for arrays, `xml:lang` for LangAlt, attribute-form where the toolkit uses it (e.g. simple props in compact mode only).
- XML escaping correct (both directions); UTF-8 in/out; support `rdf:value` + `rdf:type`? only if SDK uses it.
- Include the schema namespace constants the SDK uses (dc, xmp, xmpRights, xmpMM, photoshop, tiff, exif, exifEX, aux, crs, Iptc4xmpCore, plus `x` packet ns) as constants mirroring `XMPCore` headers.
- Property model must preserve unknown namespaces/fields on round-trip (do not drop data).
- Tests: build → serialize → parse → node-tree equality; parse a packet produced with attribute-form syntax; lang-alt x-default handling; array ops; struct ops; escaping; incremental ID string handling is SDK-side (not here).

## dng-negative seam (phase 2 contract — both agents code against this)
`crates/dng-sdk` re-exports everything: `pub use dng_core::*; pub use dng_jpeg; …`. In phase 2, Agent A owns `dng_negative.rs`; Agent B owns `dng_image_writer.rs`, `dng_read_image.rs`, `dng_render.rs`, `dng_preview.rs`, `dng_jpeg_image.rs`, `dng_lossless_jpeg.rs`(+shared), `dng_jxl` glue, `dng_big_table.rs`, `dng_bad_pixels.rs`, `dng_update_meta.rs`, `dng_lens_correction.rs`. Agent A also owns: `dng_exif.rs`, `dng_iptc.rs`, `dng_xmp.rs` (glue over `dng_xmp` crate), `dng_xmp_sdk.rs` (facade), `dng_camera_profile.rs`, `dng_color_spec.rs`, `dng_hue_sat_map.rs`, `dng_tone_curve.rs`, `dng_gain_map.rs`, `dng_opcode_list.rs`, `dng_opcodes.rs`, `dng_misc_opcodes.rs`, `dng_info.rs`, `dng_mosaic_info.rs`, `dng_linearization_info.rs`. To avoid blocking, Agent B may code against the `DngNegative` **public API** defined in `crates/dng-sdk/NEGATIVE_API.md` (Agent A must also write it there first, before implementing). Integration/resolution of mismatches happens in the integration phase.

## Testing strategy
- Per-crate unit tests (each agent).
- Integration fixtures: `vendor/extracted/dng_sdk_1_7_1/sample_files/*.dng` (14 files incl. JXL previews, PGTM2 variants, float/integer, mask/region cases) — used in phase 2/integration to verify reading (IFD parse, tag correctness, image decode) and writing.
- Round-trip: synthetic negative → WriteDNG → re-read → compare pixels/metadata bit-exact where C++ would be.
- `dng-validate` CLI port for final acceptance (phase 2 end / integration).

## Status log
- 2026-11-12: SDK 1.7.1 fetched from Adobe official zip; deps verified pure Rust; workspace created; ARCHITECTURE.md v1 written; agents A+B dispatched (phase 1).
- 2026-11-12: Phase 1 DONE. dng-core complete (47 modules, 27.4k LOC, 217 tests green; stream API uses C++-faithful get_uint16/put_uint16 naming; ImageRef = Arc<RwLock<dyn DngImage>> confirmed). dng-jpeg/dng-xmp/dng-jxl complete (60 tests green; hand-written modular JXL encoder round-trips via jxl-oxide; known upstream issue: zune-jpeg misdecodes baseline RGB JPEGs with optimized Huffman tables → encode defaults optimize_coding=false). Both subagents hit subagent quota mid-session and were resumed for phase 2.
- 2026-11-12: Phase 2 dispatched. A: dng-core retro-docs (PORTING_NOTES/DESIGN) + exif/iptc/xmp_sdk/xmp/camera_profile/hue_sat/tone_curve/gain_map/color_spec/opcode*/linearization/mosaic/negative/info. B: finish lossless_jpeg(+shared), jpeg_image, preview, big_table, bad_pixels, update_meta, bmff; batch2 (read_image/image_writer/render) later.

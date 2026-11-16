# RustDNG

**Adobe DNG SDK 1.7.1 rewritten in 100% pure Rust.** No C/C++ linking, no `bindgen`, no `*-sys` crates — every external dependency of the original SDK (libjpeg, libjxl, XMP Toolkit, zlib) is replaced by a pure-Rust crate or a pure-Rust implementation written for this project.

## Status

See [ARCHITECTURE.md](ARCHITECTURE.md) (contract + status log) and each crate's `PORTING_NOTES.md` for the honest, itemized list of what is fully ported, simplified, or stubbed. The goal is API-for-API parity with the C++ SDK (`vendor/extracted/dng_sdk_1_7_1/dng_sdk/source`, 73 .cpp / ~152k LOC, read-only reference).

## Crates

| Crate | Replaces | Contents |
|---|---|---|
| `dng-core` | SDK foundation (`dng_stream`, `dng_ifd`, `dng_tiff_directory`, `dng_pixel_buffer`, images, matrix/color primitives, resample, MD5 fingerprints, zlib glue…) | 47 modules, mirrors `dng_*.cpp` 1:1 |
| `dng-jpeg` | libjpeg | `jpeg-encoder` (enc) + `zune-jpeg` (dec) facade |
| `dng-jxl` | libjxl | decode via `jxl-oxide`; **encoder is hand-written pure Rust** (modular mode, BMFF container with Exif/XMP/ICC boxes) |
| `dng-xmp` | XMP Toolkit | full XMP data model + RDF/XML parser/serializer + packet protocol |
| `dng-sdk` | the SDK proper | negative, EXIF, IPTC, XMP glue, opcodes, big tables, previews, lossless JPEG (cc 34892), read/write pipeline, render |
| `dng-validate` | `dng_validate` CLI | validation / conversion tool |

## Build & test

```sh
cargo build --workspace
cargo test  --workspace
cargo run -p dng-validate -- <file.dng>
```

## Conventions

- `dng_foo.cpp` → module `dng_foo` (file `dng_foo.rs`); C++ class names → `UpperCamelCase` (`dng_negative` → `DngNegative`); `ParseTag` → `parse_tag`; tag constants keep C++ spelling (`tcNewSubFileType`, `ccDeflate`).
- C++ exceptions (`dng_exception`) → `Result<T, DngError>` mirroring `dng_errors.h`.
- Unavoidable deviations are marked `// PORT-NOTE(<file>:<line>)` and itemized in the crate's `PORTING_NOTES.md`. The dependency tree must never gain `cc`/`cmake`/`*-sys` packages.

## License / attribution

- **RustDNG itself** is a Rust translation (derivative work) of the Adobe DNG SDK
  and is distributed under the **Adobe DNG SDK License Agreement**
  ([LICENSE](LICENSE), [LICENSE-ADOBE-DNG-SDK.txt](LICENSE-ADOBE-DNG-SDK.txt)).
  Every ported `.rs` file carries the Adobe copyright notice in its header, as
  required by §2 of the agreement. Use/modify/redistribute/sublicense for any
  purpose is granted by §1; the software is "AS IS" (§3–4); commercial
  distribution triggers the indemnification clause (§5).
- The original C++ SDK (`vendor/`, **not included in this repository**) and the
  DNG Specification PDF remain under the same Adobe agreement — download them
  from Adobe's [DNG page](https://helpx.adobe.com/camera-raw/desktop/dng-and-file-formats/digital-negative.html).
- Pure-Rust dependencies: `half`, `flate2`/`miniz_oxide`, `crc32fast`,
  `quick-xml`, `jpeg-encoder`, `zune-jpeg`, `jxl-oxide` — each under its own
  permissive upstream license (MIT and/or Apache-2.0; see [NOTICE.md](NOTICE.md)).
- **Trademark disclaimer**: RustDNG is not affiliated with or endorsed by Adobe.
  "Adobe" and "DNG" are trademarks of Adobe Systems Incorporated.

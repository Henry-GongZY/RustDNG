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

- Adobe DNG SDK source (read-only reference, `vendor/`): Adobe DNG SDK License Agreement.
- Pure-Rust dependencies: `half`, `flate2`/`miniz_oxide`, `crc32fast`, `quick-xml`, `jpeg-encoder`, `zune-jpeg`, `jxl-oxide` (all pure Rust; MIT/Apache-2.0).
- Everything written in this repo: fresh Rust implementation, MIT.

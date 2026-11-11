# dng-jxl PORTING NOTES

Replaces the DNG SDK's libjxl dependency (`dng_jxl.cpp`, `JXL_ReadMe.txt`).

## Architecture

- **Decode** (`decode.rs`): wraps the pure-Rust `jxl-oxide` 0.11 crate.
  `render_frame(0).image_all_channels()` provides f32 samples which are
  converted to native depth (u8/u16 integer via clamp+scale, f32 passthrough
  — bit-exact for uses_original_profile images).
- **Container** (`container.rs`): hand-written ISO-BMFF box parser/builder
  (`ftyp`, `jxlp`/`jxlc`, `Exif`, `xml `/`XML `, `colr`, `brob` passthrough,
  64-bit largesize, end-of-file boxes). Box order mirrors `dng_jxl.cpp`:
  `ftyp`, Exif, `xml `, `colr`, then the codestream box. Exif boxes carry the
  4-byte zero TIFF-offset prefix (`kNumLeadingZeroBytesForEXIF`).
- **Encoder** (`encode/`): ground-up pure-Rust JPEG XL **modular-mode
  lossless** encoder written against the `jxl-modular` / `jxl-frame` /
  `jxl-image` / `jxl-coding` decoder sources as bitstream ground truth:
  - LSB-first bit packing (matches `jxl_bitstream::Bitstream`)
  - SizeHeader / ImageMetadata / ColourEncoding / BitDepth writers
  - FrameHeader for a regular modular still frame (no crop, no animation,
    Gaborish/EPF disabled)
  - TOC (non-permuted), single-entry for ≤256×256 images, full section split
    (LfGlobal + LfGroups + HfGlobal + PassGroups) for larger images
  - MA tree: single leaf, Gradient predictor, offset 0, multiplier 1
  - Entropy: no LZ77, prefix codes (Brotli-style, bit-reversed canonical
    codes), uniform code over a power-of-two alphabet, hybrid-uint with
    split_exponent 4 (values < 16 are tokens, larger use `floor_log2(v)`
    extra bits)
  - Residuals: zigzag-signed, clamped-gradient prediction identical to
    `decode_simple_grad`
  - i32 modular samples (`modular_16bit_buffers = false`) to avoid the
    narrow-buffer wraparound subtleties
- **Validation**: every `encode_jxl` code path is exercised by round-trip
  tests that decode the output with `jxl-oxide` and require bit-exact
  samples (8/16-bit integer, f32, gray/RGB/RGBA, single- and multi-group).
  A real libjxl-encoded stream (extracted from the DNG SDK sample
  `01_jxl_linear_raw_integer.dng`, a VarDCT/XYB lossy preview) is decoded in
  `real_world_sample`.

## Deviations / simplifications (PORT-NOTEs)

1. **Encoder is modular-mode only.** No VarDCT, no XYB, no patches/splines/
   noise, no animation, no progressive passes, no reference frames. The
   `quality` option maps as follows:
   - `quality >= 100` → bit-exact lossless modular (matches libjxl
     `distance = 0` behavior used by `EncodeJXL` when
     `settings.Distance() <= 0`).
   - `quality < 100` → still modular; samples are uniformly quantized
     (step derived from the SDK's `JXLQualityToDistance` table) before
     lossless encoding. This is an honest *approximation* of lossy behavior,
     not rate-controlled: output size is not guaranteed to shrink with
     quality, and the fidelity/size tradeoff differs from libjxl's VarDCT.
     Full VarDCT is out of scope for this port.
2. **`effort` is ignored** (single encoding strategy), as anticipated by the
   ARCHITECTURE.md contract.
3. **ICC profiles**: the codestream always declares an *enum* color encoding
   (sRGB / gray-sRGB / linear); caller-supplied ICC profiles are carried in
   the BMFF `colr` box (method 1, raw profile) when a container is emitted.
   Writing the JXL "encoded ICC stream" (want_icc + MA-tree-coded profile
   bytes) is not implemented — flagged `Unsupported` if it ever becomes
   necessary. On decode, `icc` prefers the `colr` box and falls back to
   jxl-oxide's embedded ICC (want_icc streams decode fine on the read side).
4. **Integer bit depths > 16** are not supported by the decoder facade
   (`Unsupported`); the DNG SDK's JXL usage is 8/16-bit integer and
   float32/half only (`EncodeJXL` handles ttByte/ttShort/ttHalfFloat/ttFloat;
   half-float sources arrive here as f32).
5. **CMYK** pixel formats decode channel-count-wise but no CMYK→RGB
   conversion is attempted (the DNG SDK never produces CMYK JXL previews).
6. **`jxl_image_info.uses_original_profile`** is reported as
   `!xyb_encoded` (true for all encoder output; may be false for external
   lossy files).
7. **Decoder renders the first keyframe only** (still images — the DNG SDK
   never embeds animated JXL).
8. **`brob`-compressed boxes** are decompressed by jxl-oxide for Exif/XMP;
   the local container parser surfaces raw `brob` contents only as opaque
   boxes (not decompressed).
9. **Orientation**: decoded as stored (`JxlDecoderSetKeepOrientation`
   equivalent) — no auto-rotation, matching the DNG SDK's usage.

## Purity

`cargo tree -p dng-jxl | grep -E "cc |cmake|-sys"` is empty (jxl-oxide and
its subcrates are pure Rust). No `unsafe` in this crate.

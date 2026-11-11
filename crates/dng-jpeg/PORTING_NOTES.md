# dng-jpeg PORTING NOTES

Pure-Rust replacement for the DNG SDK's libjpeg usage (JPEG previews and
compressed-thumbnails encode/decode, see `dng_image_writer.cpp` /
`dng_jpeg_image.cpp` / `JPEG_ReadMe.txt` in the SDK source).

## Architecture

- Encode: `jpeg-encoder` 0.6 (pure Rust). Quality uses the same 0..=100 IJG
  scale libjpeg does; `JpegEncodeOptions::quality` is passed through
  directly.
- Decode: `zune-jpeg` 0.4 (pure Rust). Output is normalized to interleaved
  8-bit Luma or RGB (`jpeg_set_out_colorspace`), matching the facade contract
  ("data interleaved, converted: gray or RGB").
- ICC: APP2 "ICC_PROFILE" segments (jpeg-encoder splits/joins segments per
  the ICC-in-JPEG convention; zune-jpeg reassembles them).

## Deviations / simplifications (PORT-NOTEs)

1. **8-bit only.** The SDK feeds libjpeg 8-bit buffers for JPEG previews
   (`ttByte`), so the facade only accepts `bits == 8`. Anything else returns
   `JpegError::Unsupported`. (libjpeg itself is 8/12-bit but the SDK only
   uses 8.)
2. **CMYK/YCCK not supported.** The DNG SDK never writes CMYK JPEGs;
   `decode_jpeg` returns `JpegError::Unsupported` for 4-component sources.
3. **Max dimensions 65535.** `jpeg-encoder`'s width/height parameters are
   `u16`; larger images return `Unsupported`. (Same limit as baseline
   libjpeg anyway.)
4. **Baseline RGB + optimized Huffman tables**: zune-jpeg 0.4.21 misdecodes
   *baseline* 3-component JPEG streams written with custom (optimized)
   Huffman tables (part of the image comes out wrong). The produced file is
   spec-valid — progressive and grayscale streams with optimized tables
   decode correctly, and external decoders (libjpeg) handle all four
   combinations. Because of this, `JpegEncodeOptions::default()` sets
   `optimize_coding = false` (which is also libjpeg's default), and the test
   suite covers decode of optimized output only for progressive and gray.
   Flagged as an upstream zune-jpeg limitation, not a facade bug.
5. **Truncation leniency**: zune-jpeg 0.4 fills missing MCUs (returns a
   full-size buffer) when only the tail of the entropy-coded data is
   truncated; earlier truncations error out. libjpeg behaves similarly
   (emits a warning and fills). `decode_jpeg` therefore errors on
   header-region truncation and tolerates tail truncation; tests document
   both.
6. **JFIF header always written.** jpeg-encoder always emits a JFIF APP0
   segment; density comes from `x_dpi`/`y_dpi` (unit = inches), matching the
   SDK's `kJFIFDensityUnit = 1` usage. No EXIF/JPEG-com APP segments are
   emitted (the SDK passes EXIF through TIFF tags, not JPEG markers).
7. **Photometric::YCbCr** passes raw YCbCr samples through to the encoder
   (input ColorType::Ycbcr), matching libjpeg's `JCS_YCbCr` in-space option.
   RGB input is converted to YCbCr by the encoder (JFIF convention).
8. **`JpegSubsampling::Gray`** converts 3-channel input to luma
   (BT.601 weights identical to libjpeg's `rgb_gray` fixed-point math) and
   emits a 1-component JPEG. For 1-channel input all subsampling values are
   equivalent.
9. **Default quality 90** (not libjpeg's 75): DNG SDK preview quality
   constants (`kDefaultQuality = 90` in `dng_image_writer.cpp` / preview
   paths) were taken as the more relevant baseline for a DNG-focused facade.

## Purity

`cargo tree -p dng-jpeg | grep -E "cc |cmake|-sys"` is empty. No `unsafe`.

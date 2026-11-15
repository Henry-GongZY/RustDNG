//! Port of `dng_preview.cpp` / `dng_preview.h` — `dng_preview`,
//! `dng_preview_info` (in dng-core's `dng_ifd`), `dng_image_preview`,
//! `dng_jpeg_preview`, `dng_jxl_preview`, `dng_raw_preview`,
//! `dng_mask_preview`, `dng_semantic_mask_preview`, `dng_depth_preview`
//! and `dng_preview_list`.
//!
//! Writer-coupled entry points (`AddTagSet`/`WriteData`/`Compress` need
//! `dng_image_writer`) either take the writer step as a closure or route to
//! `super::dng_image_writer` free functions completed in batch 2 — the
//! signatures here are the fixed seam per NEGATIVE_API.md.

use dng_core::dng_fingerprint::DngFingerprint;
use dng_core::dng_ifd::{DngIfd, DngPreviewInfo};
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_stream::DngStream;
use dng_core::dng_tag_types::{tag_type_size, ttFloat, ttShort};
use dng_core::dng_tag_values::{
    cacheVersionDefault, ccDeflate, ccJPEG, ccJXL, ccLossyJPEG, cpFloatingPoint,
    cpHorizontalDifference, cpNullPredictor, piBlackIsZero, piDepth, piLinearRaw,
    piPhotometricMask, piRGB, piTransparencyMask, piYCbCr, previewColorSpace_MaxEnum,
    sfAltPreviewImage,
    sfDepthMap, sfFloatingPoint, sfPreviewDepthMap, sfPreviewImage, sfPreviewMask,
    sfPreviewSemanticMask, sfSemanticMask, sfUnsignedInteger,
};
use dng_core::dng_types::{int32, real32, real64, uint32, uint64};
use dng_core::DngResult;

/// Test/preview helper mirroring `dng_preview`'s image dependency: the
/// image geometry the IFD info is derived from.
#[derive(Debug, Clone, Copy, Default)]
pub struct PreviewImageInfo {
    pub width: uint32,
    pub height: uint32,
    pub planes: uint32,
    /// TIFF tag type of a sample (ttByte/ttShort/ttFloat).
    pub pixel_type: uint32,
}

/// Which concrete preview class this instance represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewKind {
    #[default]
    /// `dng_preview` / `dng_image_preview`.
    Image,
    /// `dng_jpeg_preview`.
    Jpeg,
    /// `dng_jxl_preview`.
    Jxl,
    /// `dng_raw_preview`.
    Raw,
    /// `dng_mask_preview`.
    Mask,
    /// `dng_semantic_mask_preview`.
    SemanticMask,
    /// `dng_depth_preview`.
    Depth,
}

/// Port of `dng_preview` (base).
///
/// The C++ holds `shared_ptr<const dng_image>` and
/// `shared_ptr<dng_compressed_image_tiles>`; the Rust port holds the
/// compressed tiles directly plus (optionally) the geometry of the
/// uncompressed image, since the writer (batch 2) owns the image encode
/// step.
#[derive(Debug, Default)]
pub struct DngPreview {
    /// `fInfo`.
    pub info: DngPreviewInfo,

    /// `fPreferJXL`.
    pub prefer_jxl: bool,

    /// `fForNegativeCache`.
    pub for_negative_cache: bool,

    /// `fIFD` (mutable in C++; the writer tweaks compression codes here).
    pub ifd: DngIfd,

    /// `fCompressedImage` — set after `Compress` or `SetCompressedData`.
    pub compressed_image: Option<crate::dng_jpeg_image::DngCompressedImageTiles>,

    /// Geometry of the uncompressed source image (C++ `fImage`), if any.
    pub image: Option<PreviewImageInfo>,

    /// Kind tag for the polymorphic C++ hierarchy.
    pub kind: PreviewKind,
}

impl DngPreview {
    /// `dng_preview ()` — protected ctor in C++.
    pub fn new(kind: PreviewKind) -> Self {
        DngPreview {
            info: DngPreviewInfo::new(),
            prefer_jxl: false,
            for_negative_cache: false,
            ifd: DngIfd::new(),
            compressed_image: None,
            image: None,
            kind,
        }
    }

    /// `dng_image_preview ()`.
    pub fn new_image_preview() -> Self {
        DngPreview::new(PreviewKind::Image)
    }

    /// `dng_jpeg_preview ()`.
    pub fn new_jpeg_preview() -> Self {
        DngPreview::new(PreviewKind::Jpeg)
    }

    /// `dng_jxl_preview ()`.
    pub fn new_jxl_preview() -> Self {
        DngPreview::new(PreviewKind::Jxl)
    }

    /// `dng_raw_preview ()`.
    pub fn new_raw_preview() -> Self {
        DngPreview::new(PreviewKind::Raw)
    }

    /// `dng_mask_preview ()`.
    pub fn new_mask_preview() -> Self {
        DngPreview::new(PreviewKind::Mask)
    }

    /// `dng_semantic_mask_preview ()`.
    pub fn new_semantic_mask_preview() -> Self {
        DngPreview::new(PreviewKind::SemanticMask)
    }

    /// `dng_depth_preview ()`.
    pub fn new_depth_preview() -> Self {
        DngPreview::new(PreviewKind::Depth)
    }

    /// `SetImage (host, image)` — records the image geometry and derives
    /// the IFD info.
    pub fn set_image(&mut self, image: PreviewImageInfo) -> DngResult<()> {
        self.image = Some(image);
        self.set_ifd_info(image)
    }

    /// `SetIFDInfo (host, image)` (base).
    pub fn set_ifd_info(&mut self, image: PreviewImageInfo) -> DngResult<()> {
        self.ifd.new_sub_file_type = if self.info.is_primary {
            sfPreviewImage
        } else {
            sfAltPreviewImage
        };

        self.ifd.image_width = image.width;
        self.ifd.image_length = image.height;

        self.ifd.samples_per_pixel = image.planes;

        self.ifd.photometric_interpretation = if self.ifd.samples_per_pixel == 1 {
            piBlackIsZero
        } else {
            piRGB
        };

        let bytes_per = tag_type_size(image.pixel_type);
        self.ifd.bits_per_sample[0] = bytes_per * 8;

        self.ifd.sample_format[0] = if image.pixel_type == ttFloat {
            sfFloatingPoint
        } else {
            sfUnsignedInteger
        };

        for j in 1..self.ifd.samples_per_pixel as usize {
            self.ifd.bits_per_sample[j] = self.ifd.bits_per_sample[0];
            self.ifd.sample_format[j] = self.ifd.sample_format[0];
        }

        self.set_single_strip();

        Ok(())
    }

    /// `fIFD.SetSingleStrip ()` (dng_ifd helper).
    fn set_single_strip(&mut self) {
        self.ifd.uses_tiles = false;
        self.ifd.tile_length = self.ifd.image_length;
        self.ifd.tile_width = self.ifd.image_width;
    }

    /// `FindTileSize (bytesPerTile)`.
    pub fn find_tile_size(&mut self, bytes_per_tile: uint32) -> DngResult<()> {
        self.ifd.find_tile_size(bytes_per_tile, 1, 1)
    }

    /// `ImageWidth ()`.
    pub fn image_width(&self) -> uint32 {
        self.ifd.image_width
    }

    /// `ImageLength ()`.
    pub fn image_length(&self) -> uint32 {
        self.ifd.image_length
    }

    /// `SamplesPerPixel ()`.
    pub fn samples_per_pixel(&self) -> uint32 {
        self.ifd.samples_per_pixel
    }

    /// `BitsPerSample ()`.
    pub fn bits_per_sample(&self) -> uint32 {
        self.ifd.bits_per_sample[0]
    }

    /// `SampleFormat ()`.
    pub fn sample_format(&self) -> uint32 {
        self.ifd.sample_format[0]
    }

    /// `PhotometricInterpretation ()`.
    pub fn photometric_interpretation(&self) -> uint32 {
        self.ifd.photometric_interpretation
    }

    /// `NewSubFileType ()`.
    pub fn new_sub_file_type(&self) -> uint32 {
        self.ifd.new_sub_file_type
    }

    /// `WriteData (host, writer, basic, stream)` — writes either the
    /// compressed tiles or (batch 2) routes to
    /// `dng_image_writer::WriteImage` via the closure.
    pub fn write_data(
        &self,
        stream: &mut DngStream<'static>,
        write_image: &mut dyn FnMut(&mut DngStream<'static>) -> DngResult<()>,
        mut set_tile_offset: impl FnMut(uint32, uint64),
        mut set_tile_byte_count: impl FnMut(uint32, uint64),
    ) -> DngResult<()> {
        if let Some(compressed) = &self.compressed_image {
            compressed.write_data(stream, set_tile_offset, set_tile_byte_count)?;
        } else {
            write_image(stream)?;
        }

        Ok(())
    }

    /// `MaxImageDataByteCount ()`.
    pub fn max_image_data_byte_count(&self) -> uint64 {
        if let Some(compressed) = &self.compressed_image {
            compressed.non_header_size()
        } else {
            // dng_ifd::MaxImageDataByteCount equivalent (samples × area ×
            // bytes, single strip).
            let bytes_per_sample = (self.ifd.bits_per_sample[0] + 7) / 8;
            self.ifd.image_width as uint64
                * self.ifd.image_length as uint64
                * self.ifd.samples_per_pixel as uint64
                * bytes_per_sample as uint64
        }
    }

    /// `Compress (host, writer)` — base behavior: compress via tiles for
    /// JXL / lossy JPEG / deflate. `encode_tiles` is the writer hook.
    pub fn compress(
        &mut self,
        encode_tiles: &mut dyn FnMut(
            &DngIfd,
        ) -> DngResult<crate::dng_jpeg_image::DngCompressedImageTiles>,
    ) -> DngResult<()> {
        if self.ifd.compression == ccJXL
            || self.ifd.compression == ccLossyJPEG
            || self.ifd.compression == ccDeflate
        {
            let compressed = encode_tiles(&self.ifd)?;

            self.compressed_image = Some(compressed);
            self.image = None;
        }

        Ok(())
    }
}

/// Port of `dng_jpeg_preview` methods layered on `DngPreview`
/// (kind == `PreviewKind::Jpeg`).
impl DngPreview {
    /// `dng_jpeg_preview::SetIFDInfo` override.
    pub fn jpeg_set_ifd_info(&mut self, image: PreviewImageInfo) -> DngResult<()> {
        self.set_ifd_info(image)?;

        self.ifd.compression = ccJPEG; // Lossy version

        if image.planes == 1 {
            self.ifd.photometric_interpretation = piBlackIsZero;
        } else {
            self.jpeg_set_ycbcr(1, 1);
        }

        Ok(())
    }

    /// `SetCompressionQuality (quality)`.
    pub fn jpeg_set_compression_quality(&mut self, quality: uint32) {
        self.ifd.compression_quality = quality as int32;
    }

    /// `SetYCbCr (subSampleH, subSampleV)`.
    pub fn jpeg_set_ycbcr(&mut self, sub_sample_h: uint32, sub_sample_v: uint32) {
        self.ifd.photometric_interpretation = piYCbCr;
        self.ifd.ycbcr_sub_sample_h = sub_sample_h;
        self.ifd.ycbcr_sub_sample_v = sub_sample_v;
    }

    /// `SetCompressedData (compressedData)`.
    pub fn jpeg_set_compressed_data(&mut self, compressed_data: DngMemoryBlock) {
        self.image = None;

        let mut compressed_tiles = crate::dng_jpeg_image::DngCompressedImageTiles::new();
        compressed_tiles.data = vec![Some(compressed_data)];

        self.compressed_image = Some(compressed_tiles);
    }

    /// `CompressedData ()` — requires compressed data to be set.
    pub fn jpeg_compressed_data(&self) -> Option<&DngMemoryBlock> {
        match &self.compressed_image {
            Some(tiles) if tiles.data.len() == 1 => tiles.data[0].as_ref(),
            _ => None,
        }
    }

    /// `dng_jpeg_preview::WriteData` override — force lossy JPEG for the
    /// data write, then restore ccJPEG for the IFD. The write itself goes
    /// through [`DngPreview::write_data`].
    pub fn jpeg_write_data(
        &mut self,
        stream: &mut DngStream<'static>,
        write_image: &mut dyn FnMut(&mut DngStream<'static>) -> DngResult<()>,
        set_tile_offset: impl FnMut(uint32, uint64),
        set_tile_byte_count: impl FnMut(uint32, uint64),
    ) -> DngResult<()> {
        // Force the data to be written using lossy JPEG.

        self.ifd.compression = ccLossyJPEG;

        self.write_data(stream, write_image, set_tile_offset, set_tile_byte_count)?;

        // But we still want to use the normal jpeg compression code in the
        // IFD.

        self.ifd.compression = ccJPEG;

        Ok(())
    }

    /// `dng_jpeg_preview::MaxImageDataByteCount` override.
    pub fn jpeg_max_image_data_byte_count(&mut self) -> uint64 {
        self.ifd.compression = ccLossyJPEG;

        let result = self.max_image_data_byte_count();

        self.ifd.compression = ccJPEG;

        result
    }

    /// `dng_jpeg_preview::Compress` override.
    pub fn jpeg_compress(
        &mut self,
        encode_tiles: &mut dyn FnMut(
            &DngIfd,
        ) -> DngResult<crate::dng_jpeg_image::DngCompressedImageTiles>,
    ) -> DngResult<()> {
        self.ifd.compression = ccLossyJPEG;

        self.compress(encode_tiles)?;

        self.ifd.compression = ccJPEG;

        Ok(())
    }

    /// `SpoolAdobeThumbnail (stream)`.
    pub fn spool_adobe_thumbnail(&self, stream: &mut DngStream<'static>) -> DngResult<()> {
        debug_assert!(
            self.ifd.photometric_interpretation == piYCbCr,
            "SpoolAdobeThumbnail: Non-YCbCr"
        );

        let compressed_size = self
            .jpeg_compressed_data()
            .map(|b| b.logical_size())
            .unwrap_or(0);

        const DNG_CHAR4_8BIM: uint32 = ((b'8' as uint32) << 24)
            | ((b'B' as uint32) << 16)
            | ((b'I' as uint32) << 8)
            | (b'M' as uint32);

        stream.put_uint32(DNG_CHAR4_8BIM)?;
        stream.put_uint16(1036)?;
        stream.put_uint16(0)?;

        stream.put_uint32(compressed_size + 28)?;

        let width_bytes = ((self.ifd.image_width * 24 + 31) / 32) * 4;

        stream.put_uint32(1)?;
        stream.put_uint32(self.ifd.image_width)?;
        stream.put_uint32(self.ifd.image_length)?;
        stream.put_uint32(width_bytes)?;
        stream.put_uint32(width_bytes * self.ifd.image_length)?;
        stream.put_uint32(compressed_size)?;
        stream.put_uint16(24)?;
        stream.put_uint16(1)?;

        if let Some(block) = self.jpeg_compressed_data() {
            stream.put(block.buffer())?;
        }

        if compressed_size & 1 != 0 {
            stream.put_uint8(0)?;
        }

        Ok(())
    }
}

/// Port of `dng_jxl_preview::SetIFDInfo`.
///
/// The C++ queries `host.MakeJXLEncodeSettings(use_case_RenderedPreview,
/// image)` — the phase-2 glue passes the resulting distance/effort/decode
/// speed in so the host seam stays behind `DngHost` (Agent A).
pub fn jxl_preview_set_ifd_info(
    preview: &mut DngPreview,
    image: PreviewImageInfo,
    jxl_distance: real32,
    jxl_effort: int32,
    jxl_decode_speed: int32,
) -> DngResult<()> {
    preview.set_ifd_info(image)?;

    // Store a copy of the preview info so that the writer can get
    // information about the color space.

    preview.ifd.preview_info = preview.info.clone();

    preview.ifd.compression = ccJXL;
    preview.ifd.predictor = cpNullPredictor;

    preview.ifd.jxl_distance = jxl_distance;
    preview.ifd.jxl_effort = jxl_effort;
    preview.ifd.jxl_decode_speed = jxl_decode_speed;

    Ok(())
}

/// Extra fields of `dng_raw_preview`.
#[derive(Debug, Default)]
pub struct RawPreviewExtra {
    /// `fOpcodeList2Data`.
    pub opcode_list2_data: Option<DngMemoryBlock>,
    /// `fBlackLevel [kMaxColorPlanes]`.
    pub black_level: [real64; 4],
    /// `fCompressionQuality`.
    pub compression_quality: int32,
}

impl RawPreviewExtra {
    /// Mirrors the C++ ctor (`fCompressionQuality = -1`).
    pub fn new() -> Self {
        RawPreviewExtra {
            opcode_list2_data: None,
            black_level: [0.0; 4],
            compression_quality: -1,
        }
    }
}

/// Extra fields of `dng_mask_preview`.
#[derive(Debug, Default)]
pub struct MaskPreviewExtra {
    pub compression_quality: int32,
}

impl MaskPreviewExtra {
    pub fn new() -> Self {
        MaskPreviewExtra { compression_quality: -1 }
    }
}

/// Extra fields of `dng_semantic_mask_preview`.
#[derive(Debug, Default)]
pub struct SemanticMaskPreviewExtra {

    /// `fName`.
    pub name: dng_core::dng_string::DngString,
    /// `fInstanceID`.
    pub instance_id: dng_core::dng_string::DngString,
    /// `fCompressionQuality`.
    pub compression_quality: int32,
    /// `fOriginalSize`.
    pub original_size: bool,
    /// `fMaskSubArea [4]`.
    pub mask_sub_area: [uint32; 4],
}

/// Extra fields of `dng_depth_preview`.
#[derive(Debug, Default)]
pub struct DepthPreviewExtra {
    pub compression_quality: int32,
    pub full_resolution: bool,
}

impl DepthPreviewExtra {
    pub fn new() -> Self {
        DepthPreviewExtra { compression_quality: -1, full_resolution: false }
    }
}

/// `dng_raw_preview::SetIFDInfo`.
///
/// The JXL-settings portion mirrors the C++ host queries via parameters
/// (`use_case_ProxyImage` defaults supplied by phase-2 glue). The
/// JxlColorEncoding attachment for ttShort images is applied by the glue
/// (see PORTING_NOTES.md).
pub fn raw_preview_set_ifd_info(
    preview: &mut DngPreview,
    extra: &RawPreviewExtra,
    image: PreviewImageInfo,
    prefer_jxl_supported: bool,
    jxl_distance: real32,
    jxl_effort: int32,
    jxl_decode_speed: int32,
) -> DngResult<()> {
    preview.set_ifd_info(image)?;

    preview.ifd.new_sub_file_type = sfPreviewImage;

    preview.ifd.photometric_interpretation = piLinearRaw;

    if image.pixel_type == ttFloat {
        if preview.prefer_jxl && prefer_jxl_supported {
            preview.ifd.compression = ccJXL;
            preview.ifd.predictor = cpNullPredictor;
        } else {
            preview.ifd.compression = ccDeflate;
            preview.ifd.compression_quality = extra.compression_quality;
            preview.ifd.predictor = cpFloatingPoint;
        }

        for j in 0..preview.ifd.samples_per_pixel as usize {
            preview.ifd.bits_per_sample[j] = 16;
        }

        preview.ifd.find_tile_size(512 * 1024, 1, 1)?;
    } else {
        if preview.prefer_jxl && prefer_jxl_supported {
            preview.ifd.compression = ccJXL;
            preview.ifd.predictor = cpNullPredictor;
        } else {
            preview.ifd.compression = ccLossyJPEG;
            preview.ifd.compression_quality = extra.compression_quality;
        }

        preview
            .ifd
            .find_tile_size(512 * 512 * preview.ifd.samples_per_pixel, 1, 1)?;
    }

    if preview.ifd.compression == ccJXL {
        // Use default host for the raw preview compression settings so any
        // customizations targeted for the actual main image are not used
        // (C++ makes a dng_host default here).

        preview.ifd.jxl_distance = jxl_distance;
        preview.ifd.jxl_effort = jxl_effort;
        preview.ifd.jxl_decode_speed = jxl_decode_speed;
    }

    Ok(())
}

/// `dng_mask_preview::SetIFDInfo`.
pub fn mask_preview_set_ifd_info(
    preview: &mut DngPreview,
    extra: &MaskPreviewExtra,
    image: PreviewImageInfo,
    prefer_jxl_supported: bool,
    jxl_distance: real32,
    jxl_effort: int32,
    jxl_decode_speed: int32,
) -> DngResult<()> {
    preview.set_ifd_info(image)?;

    preview.ifd.new_sub_file_type = sfPreviewMask;

    preview.ifd.photometric_interpretation = piTransparencyMask;

    preview
        .ifd
        .find_tile_size(512 * 512 * preview.ifd.samples_per_pixel, 1, 1)?;

    if preview.prefer_jxl && prefer_jxl_supported {
        preview.ifd.compression = ccJXL;
        preview.ifd.predictor = cpNullPredictor;
    } else {
        preview.ifd.compression = ccDeflate;
        preview.ifd.predictor = cpHorizontalDifference;

        preview.ifd.compression_quality = extra.compression_quality;
    }

    if preview.ifd.compression == ccJXL {
        preview.ifd.jxl_distance = jxl_distance;
        preview.ifd.jxl_effort = jxl_effort;
        preview.ifd.jxl_decode_speed = jxl_decode_speed;
    }

    Ok(())
}

/// `dng_semantic_mask_preview::SetIFDInfo`.
pub fn semantic_mask_preview_set_ifd_info(
    preview: &mut DngPreview,
    extra: &SemanticMaskPreviewExtra,
    image: PreviewImageInfo,
    prefer_jxl_supported: bool,
    jxl_distance: real32,
    jxl_effort: int32,
    jxl_decode_speed: int32,
) -> DngResult<()> {
    preview.set_ifd_info(image)?;

    preview.ifd.new_sub_file_type = if extra.original_size {
        sfSemanticMask
    } else {
        sfPreviewSemanticMask
    };

    preview.ifd.photometric_interpretation = piPhotometricMask;

    preview
        .ifd
        .find_tile_size(512 * 512 * preview.ifd.samples_per_pixel, 1, 1)?;

    if preview.prefer_jxl && prefer_jxl_supported {
        preview.ifd.compression = ccJXL;
        preview.ifd.predictor = cpNullPredictor;
    } else {
        preview.ifd.compression = ccDeflate;
        preview.ifd.predictor = cpHorizontalDifference;

        preview.ifd.compression_quality = extra.compression_quality;
    }

    if preview.ifd.compression == ccJXL {
        preview.ifd.jxl_distance = jxl_distance;
        preview.ifd.jxl_effort = jxl_effort;
        preview.ifd.jxl_decode_speed = jxl_decode_speed;
    }

    Ok(())
}

/// `dng_depth_preview::SetIFDInfo`.
pub fn depth_preview_set_ifd_info(
    preview: &mut DngPreview,
    extra: &DepthPreviewExtra,
    image: PreviewImageInfo,
    prefer_jxl_supported: bool,
    jxl_distance: real32,
    jxl_effort: int32,
    jxl_decode_speed: int32,
) -> DngResult<()> {
    preview.set_ifd_info(image)?;

    preview.ifd.new_sub_file_type = if extra.full_resolution {
        sfDepthMap
    } else {
        sfPreviewDepthMap
    };

    preview.ifd.photometric_interpretation = piDepth;

    preview
        .ifd
        .find_tile_size(512 * 512 * preview.ifd.samples_per_pixel, 1, 1)?;

    if preview.prefer_jxl && prefer_jxl_supported {
        preview.ifd.compression = ccJXL;
        preview.ifd.predictor = cpNullPredictor;
    } else {
        preview.ifd.compression = ccDeflate;
        preview.ifd.predictor = cpHorizontalDifference;

        preview.ifd.compression_quality = extra.compression_quality;
    }

    if preview.ifd.compression == ccJXL {
        preview.ifd.jxl_distance = jxl_distance;
        preview.ifd.jxl_effort = jxl_effort;
        preview.ifd.jxl_decode_speed = jxl_decode_speed;
    }

    Ok(())
}

/// Port of `dng_preview_list`.
///
/// The C++ stores `shared_ptr<const dng_preview>`; the Rust port stores the
/// concrete struct by value (the polymorphic behavior the writer needs is
/// carried by `kind` + the extra-field structs held by the phase-2 glue).
#[derive(Debug, Default)]
pub struct DngPreviewList {
    previews: Vec<DngPreview>,
}

impl DngPreviewList {
    pub fn new() -> Self {
        DngPreviewList::default()
    }

    /// `Count ()`.
    pub fn count(&self) -> uint32 {
        self.previews.len() as uint32
    }

    /// `Preview (index)`.
    pub fn preview(&self, index: uint32) -> &DngPreview {
        &self.previews[index as usize]
    }

    /// `Append (preview)` — appends non-null previews (C++ skips null).
    pub fn append(&mut self, preview: Option<DngPreview>) {
        if let Some(preview) = preview {
            self.previews.push(preview);
        }
    }
}

/// `dng_preview_tag_set` — the preview-specific tags added on top of the
/// basic tag set. The writer (batch 2) consumes this description when
/// assembling the TIFF directory (`dng_basic_tag_set` is batch-2 code).
#[derive(Debug, Clone, Default)]
pub struct PreviewTagSetInfo {
    pub has_application_name: bool,
    pub has_application_version: bool,
    pub has_settings_name: bool,
    pub has_settings_digest: bool,
    pub has_color_space: bool,
    pub has_date_time: bool,
    pub has_raw_to_preview_gain: bool,
    pub has_cache_version: bool,
}

/// Build the `dng_preview_tag_set` inclusion flags from a preview
/// (`dng_preview_tag_set` ctor logic, minus the writer coupling).
pub fn preview_tag_set_info(preview: &DngPreview) -> PreviewTagSetInfo {
    let info = &preview.info;

    PreviewTagSetInfo {
        has_application_name: info.application_name.not_empty(),
        has_application_version: info.application_version.not_empty(),
        has_settings_name: info.settings_name.not_empty(),
        has_settings_digest: info.settings_digest.is_valid(),
        has_color_space: info.color_space != previewColorSpace_MaxEnum,
        has_date_time: info.date_time.not_empty(),
        has_raw_to_preview_gain: info.raw_to_preview_gain != 1.0,
        has_cache_version: info.cache_version != 0,
    }
}

/// Default `cacheVersion` used by DngPreviewInfo (exposed for tests).
pub const CACHE_VERSION_DEFAULT: uint32 = cacheVersionDefault;

// JPEG preview YCbCr coefficient constants (dng_jpeg_preview_tag_set).
pub const YCBCR_COEFFICIENTS: [(uint32, uint32); 3] = [(299, 1000), (587, 1000), (114, 1000)];
pub const REFERENCE_BLACK_WHITE: [(uint32, uint32); 6] = [
    (0, 1),
    (255, 1),
    (128, 1),
    (255, 1),
    (128, 1),
    (255, 1),
];

#[cfg(test)]
mod tests {
    use super::*;
    use dng_core::dng_tag_types::ttByte;
    use dng_core::DngMemoryStream;

    fn image(planes: uint32, pixel_type: uint32) -> PreviewImageInfo {
        PreviewImageInfo {
            width: 640,
            height: 480,
            planes,
            pixel_type,
        }
    }

    #[test]
    fn base_set_ifd_info() {
        let mut preview = DngPreview::new_image_preview();

        preview.set_image(image(3, ttByte)).unwrap();

        assert_eq!(preview.image_width(), 640);
        assert_eq!(preview.image_length(), 480);
        assert_eq!(preview.samples_per_pixel(), 3);
        assert_eq!(preview.bits_per_sample(), 8);
        assert_eq!(preview.sample_format(), sfUnsignedInteger);
        assert_eq!(preview.photometric_interpretation(), piRGB);
        assert_eq!(preview.new_sub_file_type(), sfAltPreviewImage); // not primary

        // Primary → sfPreviewImage.
        let mut primary = DngPreview::new_image_preview();
        primary.info.is_primary = true;
        primary.set_image(image(1, ttShort)).unwrap();
        assert_eq!(primary.new_sub_file_type(), sfPreviewImage);
        assert_eq!(primary.photometric_interpretation(), piBlackIsZero);
        assert_eq!(primary.bits_per_sample(), 16);

        // Float → sfFloatingPoint.
        let mut float_preview = DngPreview::new_image_preview();
        float_preview.set_image(image(1, ttFloat)).unwrap();
        assert_eq!(float_preview.sample_format(), sfFloatingPoint);
        assert_eq!(float_preview.bits_per_sample(), 32);
    }

    #[test]
    fn jpeg_preview_ifd_and_ycbcr() {
        let mut preview = DngPreview::new_jpeg_preview();

        preview.jpeg_set_ifd_info(image(3, ttByte)).unwrap();

        assert_eq!(preview.ifd.compression, ccJPEG);
        assert_eq!(preview.ifd.photometric_interpretation, piYCbCr);
        assert_eq!(preview.ifd.ycbcr_sub_sample_h, 1);
        assert_eq!(preview.ifd.ycbcr_sub_sample_v, 1);

        // Single plane → black is zero.
        let mut gray = DngPreview::new_jpeg_preview();
        gray.jpeg_set_ifd_info(image(1, ttByte)).unwrap();
        assert_eq!(gray.ifd.photometric_interpretation, piBlackIsZero);
    }

    #[test]
    fn jpeg_preview_compressed_data_roundtrip() {
        let mut preview = DngPreview::new_jpeg_preview();

        assert!(preview.jpeg_compressed_data().is_none());

        let block = DngMemoryBlock::new(5).unwrap();
        preview.jpeg_set_compressed_data(block);

        let data = preview.jpeg_compressed_data().unwrap();
        assert_eq!(data.logical_size(), 5);
        assert!(preview.image.is_none());

        // MaxImageDataByteCount with compressed data = NonHeaderSize
        // (round_up_2(5) = 6).
        assert_eq!(preview.max_image_data_byte_count(), 6);
    }

    #[test]
    fn spool_adobe_thumbnail() {
        let mut preview = DngPreview::new_jpeg_preview();

        preview
            .set_image(PreviewImageInfo {
                width: 4,
                height: 2,
                planes: 3,
                pixel_type: ttByte,
            })
            .unwrap();
        preview.jpeg_set_ycbcr(1, 1);

        let mut block = DngMemoryBlock::new(3).unwrap();
        block.buffer_mut().copy_from_slice(&[1, 2, 3]);
        preview.jpeg_set_compressed_data(block);

        let mut stream = DngStream::new(Box::new(DngMemoryStream::new()));
        stream.set_big_endian(true);
        preview.spool_adobe_thumbnail(&mut stream).unwrap();

        let data = stream.as_memory_block().unwrap().buffer().to_vec();

        // Header: '8BIM' 1036 0 size(3+28=31)
        assert_eq!(&data[0..4], b"8BIM");
        assert_eq!(u16::from_be_bytes([data[4], data[5]]), 1036);
        assert_eq!(u16::from_be_bytes([data[6], data[7]]), 0);
        assert_eq!(u32::from_be_bytes([data[8], data[9], data[10], data[11]]), 31);
        // 1, width 4, height 2, widthBytes 4, widthBytes*height 8, size 3
        assert_eq!(u32::from_be_bytes([data[12], data[13], data[14], data[15]]), 1);
        assert_eq!(u32::from_be_bytes([data[16], data[17], data[18], data[19]]), 4);
        assert_eq!(u32::from_be_bytes([data[20], data[21], data[22], data[23]]), 2);
        assert_eq!(u32::from_be_bytes([data[24], data[25], data[26], data[27]]), 12);
        assert_eq!(u32::from_be_bytes([data[28], data[29], data[30], data[31]]), 24);
        assert_eq!(u32::from_be_bytes([data[32], data[33], data[34], data[35]]), 3);
        assert_eq!(u16::from_be_bytes([data[36], data[37]]), 24);
        assert_eq!(u16::from_be_bytes([data[38], data[39]]), 1);
        // Data + pad byte (compressedSize 3 is odd).
        assert_eq!(&data[40..43], &[1, 2, 3]);
        assert_eq!(data[43], 0);
        assert_eq!(data.len(), 44);
    }

    #[test]
    fn compress_via_tiles() {
        let mut preview = DngPreview::new_jpeg_preview();
        preview.ifd.compression = ccLossyJPEG;
        preview.ifd.image_width = 2;
        preview.ifd.image_length = 1;
        preview.ifd.tile_width = 1;
        preview.ifd.tile_length = 1;
        preview.ifd.uses_tiles = true;
        preview.ifd.samples_per_pixel = 1;
        preview.ifd.bits_per_sample[0] = 8;

        preview
            .compress(&mut |ifd| {
                let mut tiles = crate::dng_jpeg_image::DngCompressedImageTiles::new();
                let count = ifd.tiles_across() * ifd.tiles_down();
                for i in 0..count {
                    let mut block = DngMemoryBlock::new(1).unwrap();
                    block.buffer_mut()[0] = i as u8;
                    tiles.data.push(Some(block));
                }
                Ok(tiles)
            })
            .unwrap();

        let compressed = preview.compressed_image.as_ref().unwrap();
        assert_eq!(compressed.data.len(), 2); // 2 across x 1 down

        // Compress clears the image reference.
        assert!(preview.image.is_none());

        // MaxImageDataByteCount now = NonHeaderSize
        // = round_up_2(1) + round_up_2(1) = 4.
        assert_eq!(preview.max_image_data_byte_count(), 4);
    }

    #[test]
    fn preview_list() {
        let mut list = DngPreviewList::new();

        assert_eq!(list.count(), 0);

        // Append skips None (C++ null check).
        list.append(None);
        assert_eq!(list.count(), 0);

        list.append(Some(DngPreview::new_jpeg_preview()));
        list.append(Some(DngPreview::new_jxl_preview()));
        assert_eq!(list.count(), 2);
        assert_eq!(list.preview(0).kind, PreviewKind::Jpeg);
        assert_eq!(list.preview(1).kind, PreviewKind::Jxl);
    }

    #[test]
    fn raw_preview_set_ifd() {
        let mut preview = DngPreview::new_raw_preview();
        let extra = RawPreviewExtra::new();

        raw_preview_set_ifd_info(
            &mut preview,
            &extra,
            PreviewImageInfo {
                width: 64,
                height: 64,
                planes: 1,
                pixel_type: ttShort,
            },
            false, // prefer_jxl but not supported → lossy JPEG
            -1.0,
            -1,
            -1,
        )
        .unwrap();

        assert_eq!(preview.ifd.new_sub_file_type, sfPreviewImage);
        assert_eq!(preview.ifd.photometric_interpretation, piLinearRaw);
        assert_eq!(preview.ifd.compression, ccLossyJPEG);
        assert_eq!(preview.ifd.compression_quality, -1);

        // Float path → deflate + fp predictor + 16-bit samples.
        let mut float_preview = DngPreview::new_raw_preview();
        raw_preview_set_ifd_info(
            &mut float_preview,
            &extra,
            PreviewImageInfo {
                width: 64,
                height: 64,
                planes: 1,
                pixel_type: ttFloat,
            },
            false,
            -1.0,
            -1,
            -1,
        )
        .unwrap();
        assert_eq!(float_preview.ifd.compression, ccDeflate);
        assert_eq!(float_preview.ifd.predictor, cpFloatingPoint);
        assert_eq!(float_preview.ifd.bits_per_sample[0], 16);
    }

    #[test]
    fn mask_and_depth_previews() {
        // Mask preview.
        let mut mask = DngPreview::new_mask_preview();
        let mask_extra = MaskPreviewExtra::new();
        mask_preview_set_ifd_info(
            &mut mask,
            &mask_extra,
            image(1, ttByte),
            false,
            -1.0,
            -1,
            -1,
        )
        .unwrap();
        assert_eq!(mask.ifd.new_sub_file_type, sfPreviewMask);
        assert_eq!(mask.ifd.photometric_interpretation, piTransparencyMask);
        assert_eq!(mask.ifd.compression, ccDeflate);
        assert_eq!(mask.ifd.predictor, cpHorizontalDifference);

        // Semantic mask preview.
        let mut sem = DngPreview::new_semantic_mask_preview();
        let sem_extra = SemanticMaskPreviewExtra {
            original_size: true,
            ..Default::default()
        };
        semantic_mask_preview_set_ifd_info(
            &mut sem,
            &sem_extra,
            image(1, ttByte),
            false,
            -1.0,
            -1,
            -1,
        )
        .unwrap();
        assert_eq!(sem.ifd.new_sub_file_type, sfSemanticMask);
        assert_eq!(sem.ifd.photometric_interpretation, piPhotometricMask);

        // Depth preview.
        let mut depth = DngPreview::new_depth_preview();
        let depth_extra = DepthPreviewExtra::new();
        depth_preview_set_ifd_info(
            &mut depth,
            &depth_extra,
            image(1, ttFloat),
            false,
            -1.0,
            -1,
            -1,
        )
        .unwrap();
        assert_eq!(depth.ifd.new_sub_file_type, sfPreviewDepthMap);
        assert_eq!(depth.ifd.photometric_interpretation, piDepth);
    }

    #[test]
    fn jxl_preview_set_ifd() {
        let mut preview = DngPreview::new_jxl_preview();

        jxl_preview_set_ifd_info(&mut preview, image(3, ttByte), 0.5, 5, 0).unwrap();

        assert_eq!(preview.ifd.compression, ccJXL);
        assert_eq!(preview.ifd.predictor, cpNullPredictor);
        assert_eq!(preview.ifd.jxl_distance, 0.5);
        assert_eq!(preview.ifd.jxl_effort, 5);
        assert_eq!(preview.ifd.jxl_decode_speed, 0);
    }

    #[test]
    fn preview_tag_set_flags() {
        let mut preview = DngPreview::new_image_preview();

        let tags = preview_tag_set_info(&preview);
        assert!(!tags.has_application_name);
        assert!(!tags.has_settings_digest);
        assert!(!tags.has_raw_to_preview_gain); // gain == 1.0
        assert!(tags.has_cache_version); // cacheVersionDefault = 256 != 0
        // colorSpace Unknown (0) != MaxEnum → the tag IS added.
        assert!(tags.has_color_space);

        preview.info.raw_to_preview_gain = 2.0;
        preview.info.application_name.set(b"Test");
        let tags = preview_tag_set_info(&preview);
        assert!(tags.has_application_name);
        assert!(tags.has_raw_to_preview_gain);
    }
}

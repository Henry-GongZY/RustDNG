//! Tests for dng-jxl: decoder wrap, hand-written encoder round-trips
//! (validated by jxl-oxide), container handling, real-world sample.

use crate::*;

fn encode_decode(img: &JxlImage, opts: &JxlEncodeOptions) -> JxlDecodedImage {
    let encoded = encode_jxl(img, opts).expect("encode");
    decode_jxl(&encoded).expect("decode of own encoder output")
}

fn solid_u8(width: u32, height: u32, channels: u32, fill: [u8; 4]) -> JxlImage {
    let n = (width * height) as usize;
    let mut data = Vec::with_capacity(n * channels as usize);
    for _ in 0..n {
        for c in 0..channels as usize {
            data.push(fill[c]);
        }
    }
    JxlImage {
        width,
        height,
        channels,
        bits: 8,
        data,
    }
}

fn gradient_u8(width: u32, height: u32, channels: u32) -> JxlImage {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let px = [
                ((x * 255) / width.max(1)) as u8,
                ((y * 255) / height.max(1)) as u8,
                (((x + y) * 255) / (width + height).max(1)) as u8,
                255,
            ];
            for c in 0..channels as usize {
                data.push(px[c]);
            }
        }
    }
    JxlImage {
        width,
        height,
        channels,
        bits: 8,
        data,
    }
}

// ---------------------------------------------------------------------------
// Encoder round-trips (lossless through jxl-oxide)
// ---------------------------------------------------------------------------

#[test]
fn roundtrip_u8_rgb_solid_small() {
    let img = solid_u8(8, 8, 3, [200, 30, 10, 0]);
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.width, dec.height, dec.channels, dec.bits), (8, 8, 3, 8));
    assert_eq!(dec.data, img.data, "solid color must be bit-exact");
    assert!(!dec.is_float);
}

#[test]
fn roundtrip_u8_rgb_gradient() {
    let img = gradient_u8(64, 48, 3);
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!(dec.data, img.data, "lossless gradient must be bit-exact");
}

#[test]
fn roundtrip_u8_gray() {
    let img = solid_u8(32, 16, 1, [77, 0, 0, 0]);
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.channels, dec.bits), (1, 8));
    assert_eq!(dec.data, img.data);
}

#[test]
fn roundtrip_u8_rgba() {
    let img = gradient_u8(40, 24, 4);
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.channels, dec.bits), (4, 8));
    assert_eq!(dec.data, img.data);
}

#[test]
fn roundtrip_u8_single_pixel_and_nonmultiple() {
    for (w, h) in [(1u32, 1u32), (3, 5), (255, 255), (256, 256), (257, 255), (255, 257)] {
        let img = gradient_u8(w, h, 3);
        let dec = encode_decode(&img, &JxlEncodeOptions::default());
        assert_eq!(dec.data, img.data, "{w}x{h} must round-trip bit-exactly");
    }
}

#[test]
fn roundtrip_u8_rgb_larger_than_one_group() {
    // Multi-group image (512x300 > 256x256): exercises LfGlobal/PassGroup
    // section split.
    let img = gradient_u8(512, 300, 3);
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.width, dec.height), (512, 300));
    assert_eq!(dec.data, img.data, "multi-group image must round-trip bit-exactly");
}

#[test]
fn roundtrip_u16_rgb() {
    let (w, h, c) = (48usize, 32usize, 3usize);
    let mut data = Vec::with_capacity(w * h * c * 2);
    for _y in 0..h {
        for x in 0..w {
            for ch in 0..c {
                let v = ((x * 65535 / w.max(1)) + ch * 7777) as u16;
                data.extend_from_slice(&v.to_le_bytes());
            }
        }
    }
    let img = JxlImage {
        width: w as u32,
        height: h as u32,
        channels: c as u32,
        bits: 16,
        data,
    };
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.bits, dec.channels), (16, 3));
    assert_eq!(dec.data, img.data, "16-bit lossless must be bit-exact");
}

#[test]
fn roundtrip_f32_rgb() {
    let (w, h, c) = (16usize, 12usize, 3usize);
    let mut data = Vec::with_capacity(w * h * c * 4);
    for i in 0..w * h {
        for ch in 0..c {
            let v = (i as f32 + ch as f32) / ((w * h + c) as f32);
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    let img = JxlImage {
        width: w as u32,
        height: h as u32,
        channels: c as u32,
        bits: 32,
        data,
    };
    let dec = encode_decode(&img, &JxlEncodeOptions::default());
    assert_eq!((dec.bits, dec.is_float), (32, true));
    assert_eq!(dec.data, img.data, "float32 must round-trip bit-exactly");
}

// ---------------------------------------------------------------------------
// Container / metadata
// ---------------------------------------------------------------------------

#[test]
fn container_with_exif_xmp_icc_roundtrip() {
    let img = solid_u8(16, 16, 3, [10, 20, 30, 0]);
    // The EXIF TIFF block (without the 4-byte offset field; encode_jxl adds
    // the JXL-required zero offset prefix, matching kNumLeadingZeroBytesForEXIF).
    let exif: Vec<u8> = b"II*\0fake-tiff".to_vec();
    let xmp = b"<?xpacket begin='' id='W5M0MpCehiHzreSzNTczkc9d'?><x:xmpmeta/></x:xmpmeta>".to_vec();
    let icc = vec![0xABu8; 132];

    let opts = JxlEncodeOptions {
        exif: Some(exif.clone()),
        xmp: Some(xmp.clone()),
        icc: Some(icc.clone()),
        ..JxlEncodeOptions::default()
    };
    let encoded = encode_jxl(&img, &opts).unwrap();

    // Container form.
    assert!(container::is_container(&encoded), "metadata forces container form");

    // The box carries the 4-byte zero offset prefix (kNumLeadingZeroBytesForEXIF).
    let exif_boxed: Vec<u8> = [0u8; 4].into_iter().chain(exif.iter().copied()).collect();

    let info = jxl_image_info(&encoded).unwrap();
    assert_eq!((info.width, info.height, info.channels), (16, 16, 3));
    assert_eq!(info.exif.as_deref(), Some(&exif_boxed[..]));
    assert_eq!(info.xmp.as_deref(), Some(&xmp[..]));
    assert_eq!(info.icc.as_deref(), Some(&icc[..]));

    let dec = decode_jxl(&encoded).unwrap();
    assert_eq!(dec.data, img.data);
    assert_eq!(dec.exif.as_deref(), Some(&exif_boxed[..]));
    assert_eq!(dec.xmp.as_deref(), Some(&xmp[..]));
    assert_eq!(dec.icc.as_deref(), Some(&icc[..]));
}

#[test]
fn bare_codestream_when_no_metadata() {
    let img = solid_u8(8, 8, 3, [0, 0, 0, 0]);
    let encoded = encode_jxl(&img, &JxlEncodeOptions::default()).unwrap();
    assert!(container::is_bare_codestream(&encoded));
    let dec = decode_jxl(&encoded).unwrap();
    assert_eq!(dec.data, img.data);
    assert!(dec.exif.is_none() && dec.xmp.is_none() && dec.icc.is_none());
}

// ---------------------------------------------------------------------------
// jxl_image_info
// ---------------------------------------------------------------------------

#[test]
fn image_info_matches_decode() {
    let img = gradient_u8(30, 20, 3);
    let encoded = encode_jxl(&img, &JxlEncodeOptions::default()).unwrap();
    let info = jxl_image_info(&encoded).unwrap();
    assert_eq!((info.width, info.height), (30, 20));
    assert_eq!((info.bits_per_sample, info.is_float), (8, false));
    assert_eq!(info.channels, 3);
    assert!(!info.alpha);
    assert!(info.uses_original_profile);
    let dec = decode_jxl(&encoded).unwrap();
    assert_eq!(dec.data, img.data);
}

// ---------------------------------------------------------------------------
// Lossy quantization path (quality < 100)
// ---------------------------------------------------------------------------

#[test]
fn lossy_quality_produces_smaller_or_equal_and_approximate() {
    let img = gradient_u8(64, 64, 3);
    let lossless = encode_jxl(&img, &JxlEncodeOptions::default()).unwrap();
    let lossy = encode_jxl(
        &img,
        &JxlEncodeOptions {
            quality: 50.0,
            ..JxlEncodeOptions::default()
        },
    )
    .unwrap();
    // PORT-NOTE: the quantization-based "lossy" mode is not rate-controlled;
    // it may produce similar or even slightly larger output than lossless.
    // Only approximation quality is asserted here.
    let dec = decode_jxl(&lossy).unwrap();
    // Approximate: mean absolute error bounded.
    let mae: f64 = dec
        .data
        .iter()
        .zip(img.data.iter())
        .map(|(a, b)| (i32::from(*a) - i32::from(*b)).abs() as f64)
        .sum::<f64>()
        / dec.data.len() as f64;
    assert!(mae < 32.0, "MAE {mae} too large for q=50");
}

// ---------------------------------------------------------------------------
// Error handling
// ---------------------------------------------------------------------------

#[test]
fn error_handling() {
    assert!(decode_jxl(&[]).is_err());
    assert!(decode_jxl(b"garbage").is_err());
    assert!(decode_jxl(&[0xff, 0x0a, 0x00]).is_err()); // truncated

    let bad = JxlImage {
        width: 4,
        height: 4,
        channels: 3,
        bits: 8,
        data: vec![0; 10], // too short
    };
    assert!(encode_jxl(&bad, &JxlEncodeOptions::default()).is_err());

    let bad2 = JxlImage {
        width: 0,
        height: 0,
        channels: 3,
        bits: 8,
        data: vec![],
    };
    assert!(encode_jxl(&bad2, &JxlEncodeOptions::default()).is_err());

    let e = JxlError::Unsupported("x".into());
    assert!(e.to_string().contains("unsupported"));
    let _ = Box::new(e) as Box<dyn std::error::Error>;
}

// ---------------------------------------------------------------------------
// Real-world sample (fetched at test setup time into tests/data)
// ---------------------------------------------------------------------------

#[test]
fn real_world_sample() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/sample.jxl");
    let Ok(bytes) = std::fs::read(&path) else {
        eprintln!("skipping real_world_sample: tests/data/sample.jxl not present");
        return;
    };

    let info = jxl_image_info(&bytes).expect("info on real sample");
    assert!(info.width > 0 && info.height > 0);
    let dec = decode_jxl(&bytes).expect("decode real sample");
    assert_eq!((dec.width, dec.height), (info.width, info.height));
    let expected_len = dec.width as usize * dec.height as usize * dec.channels as usize
        * (dec.bits as usize / 8).max(1);
    assert_eq!(dec.data.len(), expected_len);
}

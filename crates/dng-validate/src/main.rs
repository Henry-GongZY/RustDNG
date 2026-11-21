// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1.
//
// Copyright 2005-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! `dng-validate` — pure Rust port of the SDK's `dng_validate.cpp` driver.
//!
//! Validates DNG files by running the full SDK pipeline: header/IFD parse →
//! metadata parse → stage-1 pixel read → stage-2/3 builds → optional render.
//!
//! Usage: dng-validate [options] file1.dng [file2.dng ...]
//! Options:
//!   -v                 verbose metadata dump
//!   --no-stages        stop after metadata validation (no pixel decode)
//!   --render           run the render pipeline and report final image size

use std::path::Path;

use dng_core::dng_file_stream::DngFileStream;
use dng_core::DngHost;
use dng_sdk::dng_info::DngInfo;
use dng_sdk::dng_negative::DngNegative;
use dng_sdk::dng_render::DngRender;

struct Options {
    verbose: bool,
    no_stages: bool,
    render: bool,
    threads: u32,
    /// `-dng <file>` — re-save the validated negative.
    out_dng: Option<String>,
    /// `--ppm <file>` — write the rendered image as binary PPM.
    out_ppm: Option<String>,
    /// `-tif <file>` — write the stage3 image as an uncompressed TIFF.
    out_tif: Option<String>,
    /// `-cs1..-cs6/-csP3/-cs2020` — render color space index.
    color_space: u8,
    /// `-16` / `-32` — render output depth.
    render_16bit: bool,
    render_32bit: bool,
    files: Vec<String>,
}

fn print_usage() -> i32 {
    eprintln!(
        "\ndng-validate (RustDNG), a port of the Adobe DNG SDK 1.7.1 dng_validate\n\
         \n\
         Usage:  dng-validate [options] file1.dng ...\n\
         \n\
         Valid options:\n\
         -v                    Verbose metadata dump\n\
         --no-stages           Validate metadata only (no pixel decode)\n\
         --render              Render the image and report final size\n\
         \n"
    );
    1
}

fn validate_file(options: &Options, path: &str) -> dng_core::DngResult<()> {
    println!("\n===> {} <===", path);

    let mut host = DngHost::new();

    if options.threads > 1 {
        host.set_perform_area_task_threads(options.threads);
        println!("Threading: {} worker threads", options.threads);
    }
    let mut stream = DngFileStream::open_read(Path::new(path))?;

    // info.Parse / info.PostParse / info.IsValidDNG.

    let mut info = DngInfo::new();

    info.parse(&mut host, &mut stream)?;
    info.post_parse(&mut host)?;

    if !info.is_valid_dng() {
        return Err(dng_core::bad_format());
    }

    println!("Valid DNG");

    if options.verbose {
        println!("  magic           = {}", info.magic);
        println!("  main IFD index  = {}", info.main_index);
        println!("  IFD count       = {}", info.ifd_count());
        println!(
            "  camera model    = {}",
            info.shared.unique_camera_model.get_str().unwrap_or("(none)")
        );
        println!("  DNG version     = 0x{:08X}", info.shared.dng_version);
    }

    // negative.Parse / negative.PostParse.

    let mut negative = DngNegative::new(&mut host)?;

    negative
        .parse(&mut host, &mut stream, &mut info)
        .map_err(|err| {
            eprintln!("(failed at negative.parse: {err:?})");
            err
        })?;
    negative
        .post_parse(&mut host, &mut stream, &mut info)
        .map_err(|err| {
            eprintln!("(failed at negative.post_parse: {err:?})");
            err
        })?;

    if options.verbose {
        println!("  color channels  = {}", negative.color_channels());
        println!("  profiles        = {}", negative.profile_count());
    }

    // Pixel pipeline (skippable).

    if !options.no_stages {
        negative
            .read_stage1_image(&mut host, &mut stream, &mut info)
            .map_err(|err| {
                eprintln!("(failed at read_stage1_image: {err:?})");
                err
            })?;

        negative.validate_raw_image_digest(&mut host)?;

        negative
            .build_stage2_image(&mut host)
            .map_err(|err| {
                eprintln!("(failed at build_stage2_image: {err:?})");
                err
            })?;
        negative
            .build_stage3_image(&mut host, -1)
            .map_err(|err| {
                eprintln!("(failed at build_stage3_image: {err:?})");
                err
            })?;

        let describe = |name: &str, image: Option<&dng_core::ImageRef>| {
            match image {
                Some(image) => {
                    let img = image.read().unwrap();
                    println!(
                        "  {}: {}x{}, {} planes, pixel type {}",
                        name,
                        img.width(),
                        img.height(),
                        img.planes(),
                        img.pixel_type()
                    );
                }
                None => println!("  {}: (none)", name),
            };
        };

        describe("stage1", negative.stage1_image());
        describe("stage2", negative.stage2_image());
        describe("stage3", negative.stage3_image());

        if options.render {
            let mut render = DngRender::new(&negative)?;

            let pixel_type = if options.render_32bit {
                dng_core::dng_tag_types::ttFloat
            } else if options.render_16bit || options.render_32bit {
                dng_core::dng_tag_types::ttShort
            } else {
                dng_core::dng_tag_types::ttByte
            };

            render.set_final_pixel_type(pixel_type);

            // Color space (C++ dng_validate -cs1..-cs2020).

            use dng_core::dng_color_space as cs;

            let space = match options.color_space {
                1 => cs::space_srgb(),
                2 => cs::space_adobe_rgb(),
                3 => cs::space_pro_photo(),
                4 => cs::space_adobe_rgb(), // -cs4 ColorMatch: closest built-in
                5 => cs::space_gray_gamma18(),
                6 => cs::space_gray_gamma22(),
                7 => cs::space_display_p3(),
                8 => cs::space_rec2020(),
                _ => cs::space_srgb(),
            };

            render.set_final_space(
                space.matrix_from_pcs().clone(),
                space.gamma_function(),
                space.is_monochrome(),
            );

            let rendered = render.render(&negative)?;

            let out = rendered.read().unwrap();

            println!(
                "  rendered: {}x{}, {} planes, pixel type {}",
                out.width(),
                out.height(),
                out.planes(),
                out.pixel_type()
            );

            // PPM export of the rendered image (8-bit; gray or RGB).

            if let Some(ppm_path) = &options.out_ppm {
                write_ppm(ppm_path, &out)?;
                println!("  wrote {}", ppm_path);
            }
        }
    }

    // -tif <file>: write the stage3 image as an uncompressed TIFF
    // (dng_validate stage dumps use piRGB for >= 3 planes, piBlackIsZero
    // otherwise).

    if let Some(tif_path) = &options.out_tif {
        let stage3 = negative.stage3_image().expect("stage3 image").clone();

        let stage3_img = stage3.read().unwrap();

        let photometric = if stage3_img.planes() >= 3 {
            dng_core::dng_tag_values::piRGB
        } else {
            dng_core::dng_tag_values::piBlackIsZero
        };

        let mut out_stream = DngFileStream::open_write(Path::new(tif_path))?;

        let mut writer = dng_sdk::dng_image_writer::DngImageWriter::new();

        writer.write_tiff_basic(
            &mut out_stream,
            &*stage3_img,
            &dng_sdk::dng_image_writer::TiffWriteOptions {
                photometric,
                compression: dng_core::dng_tag_values::ccUncompressed,
                icc_profile: None,
                software: Some("RustDNG dng-validate"),
                metadata: Some(&negative.metadata),
            },
        )?;

        println!("  wrote {}", tif_path);
    }

    // -dng <file>: re-save the validated negative (exercises the writer on
    // real data).

    if let Some(dng_path) = &options.out_dng {
        let mut out_stream = DngFileStream::open_write(Path::new(dng_path))?;

        let mut writer = dng_sdk::dng_image_writer::DngImageWriter::new();

        writer.write_dng_uncompressed(&mut host, &mut out_stream, &mut negative, false)?;

        println!("  wrote {}", dng_path);
    }

    Ok(())
}

/// Binary PPM (P5 gray / P6 RGB) export for the rendered image.  The C++
/// dng_validate writes TIFF dumps instead; PPM keeps the port dependency
/// free while serving the same verification purpose.
fn write_ppm(path: &str, image: &std::sync::RwLockReadGuard<dyn dng_core::DngImage>) -> dng_core::DngResult<()> {
    use std::io::Write;

    let width = image.width();
    let height = image.height();
    let planes = image.planes();

    let mut buf = dng_core::dng_pixel_buffer::DngPixelBuffer::with_planar_config(
        image.bounds(),
        0,
        planes,
        dng_core::dng_tag_types::ttByte,
        dng_core::dng_tag_values::pcInterleaved,
        dng_core::dng_pixel_buffer::DngPixelBuffer::allocate_data(
            image.bounds(),
            planes,
            dng_core::dng_tag_types::ttByte,
        )?,
    )?;

    image.do_get(&mut buf)?;

    let samples = buf.as_u8_slice().ok_or_else(dng_core::program_error)?;

    let file = std::fs::File::create(path).map_err(|_| dng_core::dng_exceptions::open_file_error())?;

    let mut writer = std::io::BufWriter::new(file);

    if planes == 3 {
        write!(writer, "P6\n{} {}\n255\n", width, height).map_err(|_| dng_core::dng_exceptions::write_failure())?;
        writer.write_all(samples).map_err(|_| dng_core::dng_exceptions::write_failure())?;
    } else {
        write!(writer, "P5\n{} {}\n255\n", width, height).map_err(|_| dng_core::dng_exceptions::write_failure())?;
        writer.write_all(samples).map_err(|_| dng_core::dng_exceptions::write_failure())?;
    }

    Ok(())
}

fn main() {
    let mut options = Options {
        verbose: false,
        no_stages: false,
        render: false,
        threads: 1,
        out_dng: None,
        out_ppm: None,
        out_tif: None,
        color_space: 1, // sRGB
        render_16bit: false,
        render_32bit: false,
        files: Vec::new(),
    };

    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-v" | "--verbose" => options.verbose = true,
            "--no-stages" => options.no_stages = true,
            "--render" => options.render = true,
            "--threads" => {
                let v = args.next().unwrap_or_default();
                options.threads = v.parse().unwrap_or(1);
            }
            "-dng" => {
                let v = args.next().unwrap_or_default();
                if !v.is_empty() {
                    options.out_dng = Some(v);
                }
            }
            "--ppm" => {
                let v = args.next().unwrap_or_default();
                if !v.is_empty() {
                    options.out_ppm = Some(v);
                }
            }
            "-tif" => {
                let v = args.next().unwrap_or_default();
                if !v.is_empty() {
                    options.out_tif = Some(v);
                }
            }
            "-cs1" => options.color_space = 1,
            "-cs2" => options.color_space = 2,
            "-cs3" => options.color_space = 3,
            "-cs4" => options.color_space = 4,
            "-cs5" => options.color_space = 5,
            "-cs6" => options.color_space = 6,
            "-csP3" => options.color_space = 7,
            "-cs2020" => options.color_space = 8,
            "-16" => options.render_16bit = true,
            "-32" => options.render_32bit = true,
            "-h" | "--help" => std::process::exit(print_usage()),
            other => options.files.push(other.to_string()),
        }
    }

    if options.files.is_empty() {
        std::process::exit(print_usage());
    }

    let mut failures = 0;

    for path in &options.files {
        match validate_file(&options, path) {
            Ok(()) => {}
            Err(err) => {
                println!("*** Error validating {path}: {err:?}");
                failures += 1;
            }
        }
    }

    if failures == 0 {
        println!("\nValidation complete");
    } else {
        println!("\nValidation complete with {failures} failure(s)");
        std::process::exit(2);
    }
}

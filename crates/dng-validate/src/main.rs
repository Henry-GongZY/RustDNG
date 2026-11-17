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

            render.set_final_pixel_type(dng_core::dng_tag_types::ttShort);

            let rendered = render.render(&negative)?;

            let out = rendered.read().unwrap();

            println!(
                "  rendered: {}x{}, {} planes, pixel type {}",
                out.width(),
                out.height(),
                out.planes(),
                out.pixel_type()
            );
        }
    }

    Ok(())
}

fn main() {
    let mut options = Options {
        verbose: false,
        no_stages: false,
        render: false,
        threads: 1,
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

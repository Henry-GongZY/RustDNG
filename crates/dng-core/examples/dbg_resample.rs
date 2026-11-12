use dng_core::*;
use dng_core::dng_image::{DngImage, EdgeOption};
use dng_core::dng_area_task::DngAreaTask;
use dng_core::dng_pixel_buffer::{DngPixelBuffer, PixelData};
use std::sync::{Arc, RwLock};

fn main() {
    let src = dng_simple_image::DngSimpleImage::new(dng_rect::DngRect::from_corners(0, 0, 8, 8), 1, dng_tag_types::ttShort).unwrap();
    DngImage::set_constant_u16(&src, 1000, DngImage::bounds(&src)).unwrap();
    let src_ref: dng_image::ImageRef = Arc::new(RwLock::new(src));
    let dst_ref: dng_image::ImageRef = Arc::new(RwLock::new(dng_simple_image::DngSimpleImage::new(dng_rect::DngRect::from_corners(0, 0, 16, 16), 1, dng_tag_types::ttShort).unwrap()));

    let mut task = dng_resample::DngResampleTask::new(
        src_ref.clone(), dst_ref.clone(),
        DngImage::bounds(&*src_ref.read().unwrap()), DngImage::bounds(&*dst_ref.read().unwrap())).unwrap();

    let bounds = DngImage::bounds(&*dst_ref.read().unwrap());
    let tile_size = task.find_tile_size(bounds);
    println!("tile_size: {:?}", (tile_size.v, tile_size.h));
    task.start(1, bounds, tile_size, None).unwrap();
    println!("start ok");
    // Bisect inside process: do the get step manually.
    let src_area = task.src_area_for_debug(bounds).unwrap();
    println!("src_area: {:?}", (src_area.t, src_area.l, src_area.b, src_area.r));

    let mut src_buffer = DngPixelBuffer::with_planar_config(
        src_area, 0, 1, dng_tag_types::ttShort,
        dng_tag_values::pcRowInterleavedAlignSIMD,
        DngPixelBuffer::allocate_data(src_area, 1, dng_tag_types::ttShort).unwrap()).unwrap();
    let r = {
        let src = task.src_image_handle();
        let guard = src.read().unwrap();
        guard.get(&mut src_buffer, EdgeOption::EdgeRepeat, 1, 1)
    };
    println!("get padded: {:?}", r.map_err(|e| format!("{:?}", e)));
    let r = task.process(0, bounds, None);
    println!("process whole: {:?}", r.map_err(|e| format!("{:?}", e)));
}

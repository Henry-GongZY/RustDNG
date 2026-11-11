// Port of dng_filter_task.h/.cpp — the filter-task base lives in
// dng_area_task.rs (`DngFilterTaskBase`, since Rust has no multiple
// inheritance); this module re-exports it under the C++ module name and
// documents the mapping.

pub use crate::dng_area_task::DngFilterTaskBase as DngFilterTask;

/// Helper mirroring `dng_filter_task` construction.
pub fn filter_task(
    name: &str,
    src_image: crate::dng_image::ImageRef,
    dst_image: crate::dng_image::ImageRef,
) -> DngFilterTask {
    DngFilterTask::new(name, src_image, dst_image)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn src_area_default_identity() {
        let src = crate::dng_simple_image::DngSimpleImage::new(
            crate::dng_rect::DngRect::from_corners(0, 0, 4, 4),
            1,
            crate::dng_tag_types::ttByte,
        )
        .unwrap()
        .into_image_ref()
        .unwrap();
        let dst = src.clone();

        let t = filter_task("test", src, dst);
        let area = crate::dng_rect::DngRect::from_corners(1, 1, 3, 3);
        assert_eq!(t.src_area(area), area);
    }
}

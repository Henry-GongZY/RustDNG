use dng_core::*;
use dng_core::dng_ifd::DngIfd;
use dng_core::dng_matrix::DngMatrix;
use dng_core::dng_shared::DngShared;
use dng_core::dng_tag_values::*;

fn main() {
    let mut shared = DngShared::new();
    shared.dng_version = dngVersion_1_4_0_0;
    shared.dng_backward_version = dngVersion_1_1_0_0;
    shared.camera_profile.color_planes = 3;
    shared.camera_profile.color_matrix1 =
        DngMatrix::from_rows(3, 3, &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]).unwrap();
    shared.as_shot_neutral = Some(dng_matrix::DngVector::from_values(&[1.0, 1.0, 1.0]).unwrap());

    let mut ifd = DngIfd::new();
    ifd.uses_new_sub_file_type = true;
    ifd.new_sub_file_type = sfMainImage;
    ifd.image_width = 4;
    ifd.image_length = 4;
    ifd.bits_per_sample[0] = 16;
    ifd.photometric_interpretation = piCFA;
    ifd.samples_per_pixel = 1;
    ifd.cfa_repeat_pattern_rows = 2;
    ifd.cfa_repeat_pattern_cols = 2;
    ifd.cfa_pattern[0][0] = 0;
    ifd.cfa_pattern[0][1] = 1;
    ifd.cfa_pattern[1][0] = 1;
    ifd.cfa_pattern[1][1] = 2;
    ifd.cfa_plane_color = [0, 1, 2, 0];
    ifd.uses_strips = true;
    ifd.tile_width = 4;
    ifd.tile_length = 4;
    ifd.tile_offsets_count = 1;
    ifd.tile_byte_counts_count = 1;
    ifd.post_parse();
    println!("valid: {}", ifd.is_valid_dng(&shared, 0));
}

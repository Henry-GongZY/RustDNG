//! Frame header + TOC writers, matching the `jxl-frame` bundle parsers for
//! a simple modular still frame.

use super::bitwriter::{write_u32_spec, BitWriter};
use crate::JxlError;

/// Write the FrameHeader for a regular modular still frame.
///
/// `alpha` controls the ec_blending_info count (one entry per extra
/// channel).
pub fn write_frame_header(w: &mut BitWriter, alpha: bool) {
    // all_default = false.
    w.write_bits(0, 1);
    // frame_type: Bundle(FrameType) — 2 bits, RegularFrame = 0.
    w.write_bits(0, 2);
    // encoding: 1 bit — Modular = 1.
    w.write_bits(1, 1);
    // flags: Bundle(FrameFlags) = u64 0 (2-bit selector 0).
    w.write_bits(0, 2);
    // do_ycbcr: cond(!all_default && !xyb_encoded) = false.
    w.write_bits(0, 1);
    // encoded_color_channels: computed, no bits.
    // jpeg_upsampling: cond(do_ycbcr && ...) — skipped.
    // upsampling: U32(1, 2, 4, 8) -> selector 0.
    w.write_bits(0, 2);
    // ec_upsampling: one per extra channel, U32(1, 2, 4, 8) selector 0.
    if alpha {
        w.write_bits(0, 2);
    }
    // group_size_shift = u(2): 1 -> group_dim 256.
    w.write_bits(1, 2);
    // x_qm_scale / b_qm_scale: cond(!all_default && xyb && VarDct) — skipped.
    // passes: num_passes = U32(1, 2, 3, 4 + u(3)) -> selector 0; rest
    // skipped (num_passes == 1).
    w.write_bits(0, 2);
    // lf_level: cond(LfFrame) — skipped.
    // have_crop = false.
    w.write_bits(0, 1);
    // x0/y0/width/height: cond(have_crop) — skipped.
    // blending_info: Bundle(BlendingInfo):
    //   mode = Bundle(BlendMode): read_u32(0,1,2,3+U(2)) -> Replace=0 selector 0.
    w.write_bits(0, 2);
    //   alpha_channel / clamp: cond(Blend || MulAdd || Mul) — skipped.
    //   source: cond(!resets_canvas(mode, no-crop canvas)) — resets_canvas
    //   is true for Replace + full canvas -> skipped.
    // ec_blending_info: one per extra channel, same encoding.
    if alpha {
        w.write_bits(0, 2); // mode Replace
    }
    // duration / timecode: cond(animation) — skipped.
    // is_last = true.
    w.write_bits(1, 1);
    // save_as_reference: cond(!all_default && !LfFrame && !is_last) — skipped.
    // resets_canvas: cond(false) — no bits.
    // save_before_ct: cond(!all_default && (ReferenceOnly || (resets_canvas
    // && (!is_last && ...)))) — false -> skipped.
    // name: Bundle(Name): len = U32(0, u(4), 16 + u(5), 48 + u(10)) selector 0.
    w.write_bits(0, 2);
    // restoration_filter: all_default = false;
    w.write_bits(0, 1);
    //   gab: gab_enabled = false.
    w.write_bits(0, 1);
    //   epf: iters = u(2) = 0 (disabled).
    w.write_bits(0, 2);
    //   extensions: u64 0 (2-bit selector 0).
    w.write_bits(0, 2);
    // extensions: u64 0 (2-bit selector 0).
    w.write_bits(0, 2);
}

/// TOC section kinds in original (non-permuted) order for a modular frame
/// with one pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    LfGlobal,
    LfGroup(u32),
    HfGlobal,
    PassGroup(u32),
}

/// Compute the section list for the given image/frame geometry.
pub fn section_list(width: u32, height: u32) -> Vec<SectionKind> {
    let group_dim = 256u32;
    let lf_group_dim = group_dim * 8;
    let num_groups = width.div_ceil(group_dim) * height.div_ceil(group_dim);
    let num_lf_groups = width.div_ceil(lf_group_dim) * height.div_ceil(lf_group_dim);

    if num_groups == 1 {
        // Single TOC entry containing everything.
        return vec![SectionKind::LfGlobal]; // only kind used for accounting
    }

    let mut out = vec![SectionKind::LfGlobal];
    for i in 0..num_lf_groups {
        out.push(SectionKind::LfGroup(i));
    }
    out.push(SectionKind::HfGlobal);
    for g in 0..num_groups {
        out.push(SectionKind::PassGroup(g));
    }
    out
}

/// Write the TOC for the given section byte sizes.
pub fn write_toc(w: &mut BitWriter, sizes: &[u32]) {
    // num_groups == 1 && num_passes == 1 -> 1 entry, else
    // 1 + num_lf_groups + 1 + num_groups entries (single pass).
    debug_assert!(!sizes.is_empty());
    // permutated_toc = false.
    w.write_bits(0, 1);
    w.zero_pad_to_byte();
    for &size in sizes {
        // U32(U(10), 1024 + U(14), 17408 + U(22), 4211712 + U(30)).
        write_u32_spec(
            w,
            [(0, 10), (1024, 14), (17408, 22), (4211712, 30)],
            size,
        )
        .expect("section size out of range");
    }
    w.zero_pad_to_byte();
}

/// Number of TOC entries for the section list.
pub fn toc_entry_count(width: u32, height: u32) -> usize {
    let group_dim = 256u32;
    let lf_group_dim = group_dim * 8;
    let num_groups = width.div_ceil(group_dim) * height.div_ceil(group_dim);
    let num_lf_groups = width.div_ceil(lf_group_dim) * height.div_ceil(lf_group_dim);
    if num_groups == 1 {
        1
    } else {
        (1 + num_lf_groups + 1 + num_groups) as usize
    }
}

/// Compute per-section group geometry.
pub struct GroupGeometry {
    pub num_groups: u32,
    pub groups_per_row: u32,
    pub num_lf_groups: u32,
}

pub fn group_geometry(width: u32, height: u32) -> GroupGeometry {
    let group_dim = 256u32;
    GroupGeometry {
        num_groups: width.div_ceil(group_dim) * height.div_ceil(group_dim),
        groups_per_row: width.div_ceil(group_dim),
        num_lf_groups: width.div_ceil(group_dim * 8) * height.div_ceil(group_dim * 8),
    }
}

pub fn check_writable(width: u32, height: u32) -> Result<(), JxlError> {
    let _ = write_u32_spec; // keep import
    if width == 0 || height == 0 {
        return Err(JxlError::BadGeometry("empty image".into()));
    }
    Ok(())
}

// Port of dng_host.h/.cpp — the SDK host abstraction (allocator removed per
// ARCHITECTURE.md; abort sniffer shared via Arc).
//
// PORT-NOTE(dng_host.h:196): the phase-2 factory methods (Make_dng_exif,
// Make_dng_xmp, Make_dng_shared, Make_dng_ifd, Make_dng_negative,
// Make_dng_image, Make_dng_opcode, Make_dng_rgb_to_rgb_table_data,
// ApplyOpcodeList, ResampleImage) return phase-2 types (dng_exif, dng_xmp,
// dng_negative, dng_opcode, ...) and are added by the dng-sdk crate which
// owns those types.  `fJXLEncodeSettings`/`fJXLDolorSpaceInfo` (JXL encoder
// configuration) are likewise phase-2 (dng_jxl glue); see PORTING_NOTES.md.

use crate::dng_errors::{DngError, DngResult};
use crate::dng_rect::DngRect;
use crate::dng_types::{real64, uint32};
use crate::dng_utils::{min_uint32, pin_int32};
use std::sync::Arc;

/// Port of `dng_host`.
pub struct DngHost {
    sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>,

    needs_meta: bool,
    needs_image: bool,
    for_preview: bool,

    minimum_size: uint32,
    preferred_size: uint32,
    maximum_size: uint32,

    crop_factor: real64,

    save_dng_version: uint32,
    save_linear_dng: bool,

    lossy_mosaic_jxl: bool,
    lossless_jxl: bool,

    keep_original_file: bool,
    ignore_enhanced: bool,

    for_fast_save_to_dng: bool,
    fast_save_to_dng_size: uint32,

    preserve_stage2: bool,
}

impl Default for DngHost {
    fn default() -> Self {
        DngHost::new()
    }
}

impl DngHost {
    /// `dng_host (allocator, sniffer)`.
    pub fn new() -> Self {
        DngHost {
            sniffer: None,
            needs_meta: true,
            needs_image: true,
            for_preview: false,
            minimum_size: 0,
            preferred_size: 0,
            maximum_size: 0,
            crop_factor: 1.0,
            save_dng_version: crate::dng_tag_values::dngVersion_None,
            save_linear_dng: false,
            lossy_mosaic_jxl: false,
            lossless_jxl: false,
            keep_original_file: false,
            ignore_enhanced: false,
            for_fast_save_to_dng: false,
            fast_save_to_dng_size: 0,
            preserve_stage2: false,
        }
    }

    /// `SetSniffer (sniffer)`.
    pub fn set_sniffer(&mut self, sniffer: Option<Arc<dyn crate::dng_abort_sniffer::AbortSniffer>>) {
        self.sniffer = sniffer;
    }

    /// `Sniffer ()`.
    pub fn sniffer(&self) -> Option<&Arc<dyn crate::dng_abort_sniffer::AbortSniffer>> {
        self.sniffer.as_ref()
    }

    /// `SniffForAbort ()`.
    pub fn sniff_for_abort(&self) -> DngResult<()> {
        crate::dng_abort_sniffer::sniff_for_abort(self.sniffer.as_ref())
    }

    /// `SetNeedsMeta`.
    pub fn set_needs_meta(&mut self, needs: bool) {
        self.needs_meta = needs;
    }

    /// `NeedsMeta`.
    pub fn needs_meta(&self) -> bool {
        self.needs_meta
    }

    /// `SetNeedsImage`.
    pub fn set_needs_image(&mut self, needs: bool) {
        self.needs_image = needs;
    }

    /// `NeedsImage`.
    pub fn needs_image(&self) -> bool {
        self.needs_image
    }

    /// `SetForPreview`.
    pub fn set_for_preview(&mut self, preview: bool) {
        self.for_preview = preview;
    }

    /// `ForPreview`.
    pub fn for_preview(&self) -> bool {
        self.for_preview
    }

    pub fn set_minimum_size(&mut self, size: uint32) {
        self.minimum_size = size;
    }

    pub fn minimum_size(&self) -> uint32 {
        self.minimum_size
    }

    pub fn set_preferred_size(&mut self, size: uint32) {
        self.preferred_size = size;
    }

    pub fn preferred_size(&self) -> uint32 {
        self.preferred_size
    }

    pub fn set_maximum_size(&mut self, size: uint32) {
        self.maximum_size = size;
    }

    pub fn maximum_size(&self) -> uint32 {
        self.maximum_size
    }

    /// `SetForFastSaveToDNG (flag, size)`.
    pub fn set_for_fast_save_to_dng(&mut self, flag: bool, size: uint32) {
        self.for_fast_save_to_dng = flag;
        self.fast_save_to_dng_size = size;
    }

    pub fn for_fast_save_to_dng(&self) -> bool {
        self.for_fast_save_to_dng
    }

    pub fn fast_save_to_dng_size(&self) -> uint32 {
        self.fast_save_to_dng_size
    }

    pub fn set_crop_factor(&mut self, crop_factor: real64) {
        self.crop_factor = crop_factor;
    }

    pub fn crop_factor(&self) -> real64 {
        self.crop_factor
    }

    /// `ValidateSizes ()`.
    pub fn validate_sizes(&mut self) {
        // The maximum size limits the other two sizes.
        if self.maximum_size != 0 {
            self.minimum_size = min_uint32(self.minimum_size, self.maximum_size);
            self.preferred_size = min_uint32(self.preferred_size, self.maximum_size);
        }

        // If we have a preferred size, it limits the minimum size.
        if self.preferred_size != 0 {
            self.minimum_size = min_uint32(self.minimum_size, self.preferred_size);
        }
        // Else find default value for preferred size.
        else if self.maximum_size != 0 {
            self.preferred_size = self.maximum_size;
        }

        // If we don't have a minimum size, find default.
        if self.minimum_size == 0 {
            // Common preview-size buckets (see C++ for the rationale).
            let p = self.preferred_size;
            self.minimum_size = if (160..=256).contains(&p) {
                160
            } else if (490..=512).contains(&p) {
                490
            } else if (980..=1024).contains(&p) {
                980
            } else if (1470..=1536).contains(&p) {
                1470
            } else if (1960..=2048).contains(&p) {
                1960
            } else if (2400..=2560).contains(&p) {
                2400
            } else if (2448..=2880).contains(&p) {
                2448
            } else if (2560..=3000).contains(&p) {
                2560
            } else if (3480..=4096).contains(&p) {
                3480
            } else if (3824..=4500).contains(&p) {
                3824
            } else if (4352..=5120).contains(&p) {
                4352
            } else if (6528..=7680).contains(&p) {
                6528
            } else {
                p
            };
        }
    }

    pub fn set_save_dng_version(&mut self, version: uint32) {
        self.save_dng_version = version;
    }

    /// `SaveDNGVersion ()`.
    pub fn save_dng_version(&self) -> uint32 {
        self.save_dng_version
    }

    pub fn set_save_linear_dng(&mut self, linear: bool) {
        self.save_linear_dng = linear;
    }

    /// `SaveLinearDNG (negative)` — the negative-dependent virtual is
    /// phase-2; the flag accessor is provided here.
    pub fn save_linear_dng(&self) -> bool {
        self.save_linear_dng
    }

    pub fn lossy_mosaic_jxl(&self) -> bool {
        self.lossy_mosaic_jxl
    }

    pub fn set_lossy_mosaic_jxl(&mut self, want: bool) -> bool {
        self.lossy_mosaic_jxl = want;
        want
    }

    pub fn lossless_jxl(&self) -> bool {
        self.lossless_jxl
    }

    pub fn set_lossless_jxl(&mut self, want: bool) -> bool {
        self.lossless_jxl = want;
        want
    }

    pub fn set_keep_original_file(&mut self, keep: bool) {
        self.keep_original_file = keep;
    }

    pub fn keep_original_file(&self) -> bool {
        self.keep_original_file
    }

    pub fn ignore_enhanced(&self) -> bool {
        self.ignore_enhanced
    }

    pub fn set_ignore_enhanced(&mut self, state: bool) {
        self.ignore_enhanced = state;
    }

    /// `IsTransientError (code)`.
    pub fn is_transient_error(&self, code: &DngError) -> bool {
        matches!(code, DngError::Memory | DngError::UserCancel)
    }

    /// `WantsPreserveStage2 ()`.
    pub fn wants_preserve_stage2(&self) -> bool {
        self.preserve_stage2
    }

    /// `SetWantsPreserveStage2` (C++: SetWantsPreserveStage2 exists on the
    /// host in 1.7).
    pub fn set_wants_preserve_stage2(&mut self, state: bool) {
        self.preserve_stage2 = state;
    }

    /// `PerformAreaTask (task, area, progress)` — serial execution
    /// (ARCHITECTURE.md §9 deviation).
    pub fn perform_area_task(
        &mut self,
        task: &mut dyn crate::dng_area_task::DngAreaTask,
        area: DngRect,
    ) -> DngResult<()> {
        let sniffer = self.sniffer.clone();
        crate::dng_area_task::perform_area_task(task, area, sniffer.as_ref(), None)
    }

    /// `PerformAreaTaskThreads ()` — always 1 in the serial port.
    pub fn perform_area_task_threads(&self) -> uint32 {
        1
    }
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_cpp() {
        let host = DngHost::new();
        assert!(host.needs_meta());
        assert!(host.needs_image());
        assert!(!host.for_preview());
        assert_eq!(host.minimum_size(), 0);
        assert_eq!(host.preferred_size(), 0);
        assert_eq!(host.maximum_size(), 0);
        assert_eq!(host.crop_factor(), 1.0);
        assert_eq!(host.save_dng_version(), 0);
        assert!(!host.save_linear_dng());
        assert!(!host.keep_original_file());
        assert!(!host.ignore_enhanced());
        assert_eq!(host.perform_area_task_threads(), 1);
    }

    #[test]
    fn validate_sizes_buckets() {
        let mut host = DngHost::new();
        host.set_maximum_size(1024);
        host.validate_sizes();
        assert_eq!(host.preferred_size(), 1024);
        assert_eq!(host.minimum_size(), 980); // bucket 980..=1024

        // 300 is not in a bucket, so min == preferred.
        let mut host2 = DngHost::new();
        host2.set_preferred_size(300);
        host2.validate_sizes();
        assert_eq!(host2.minimum_size(), 300);
    }

    #[test]
    fn transient_errors() {
        let host = DngHost::new();
        assert!(host.is_transient_error(&DngError::Memory));
        assert!(host.is_transient_error(&DngError::UserCancel));
        assert!(!host.is_transient_error(&DngError::BadFormat));
    }

    #[test]
    fn sniffing_through_host() {
        use crate::dng_abort_sniffer::{AbortSniffer, NullAbortSniffer};
        let mut host = DngHost::new();
        assert!(host.sniff_for_abort().is_ok());

        struct Cancel;
        impl AbortSniffer for Cancel {
            fn sniff(&self) -> DngResult<()> {
                Err(crate::dng_exceptions::user_cancel())
            }
        }

        host.set_sniffer(Some(Arc::new(Cancel)));
        assert_eq!(
            host.sniff_for_abort().unwrap_err(),
            DngError::UserCancel
        );

        // Null sniffer never aborts.
        host.set_sniffer(Some(Arc::new(NullAbortSniffer)));
        assert!(host.sniff_for_abort().is_ok());
    }
}

// Port of dng_hue_sat_map.h/.cpp — hue/saturation/value lookup tables.
//
// `dng_ref_counted_block` maps to `Arc<Vec<HSBModify>>` (ARCHITECTURE.md §8);
// the runtime fingerprint counter is an AtomicU64.

use dng_core::{program_error, DngFingerprint, DngResult};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Port of `dng_hue_sat_map::HSBModify`.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct HSBModify {
    pub hue_shift: f32,
    pub sat_scale: f32,
    pub val_scale: f32,
}

static sRuntimeFingerprintCounter: AtomicU64 = AtomicU64::new(0);

/// Port of `dng_hue_sat_map`.
#[derive(Debug, Clone, Default)]
pub struct DngHueSatMap {
    hue_divisions: u32,
    sat_divisions: u32,
    val_divisions: u32,
    hue_step: u32,
    val_step: u32,
    runtime_fingerprint: DngFingerprint,
    deltas: Arc<Vec<HSBModify>>,
}

impl DngHueSatMap {
    pub fn new() -> Self {
        DngHueSatMap::default()
    }

    /// `IsNull ()`.
    pub fn is_null(&self) -> bool {
        !self.is_valid()
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.hue_divisions > 0
            && self.sat_divisions > 1
            && self.val_divisions > 0
            && !self.deltas.is_empty()
    }

    /// `SetInvalid ()`.
    pub fn set_invalid(&mut self) {
        self.hue_divisions = 0;
        self.sat_divisions = 0;
        self.val_divisions = 0;
        self.hue_step = 0;
        self.val_step = 0;
        self.runtime_fingerprint.clear();
        self.deltas = Arc::new(Vec::new());
    }

    /// `GetDivisions (hueDivisions, satDivisions, valDivisions)`.
    pub fn get_divisions(&self) -> (u32, u32, u32) {
        (self.hue_divisions, self.sat_divisions, self.val_divisions)
    }

    /// `SetDivisions (hueDivisions, satDivisions, valDivisions)`.
    pub fn set_divisions(&mut self, hue_divisions: u32, sat_divisions: u32, val_divisions: u32) -> DngResult<()> {
        debug_assert!(hue_divisions >= 1, "Must have at least 1 hue division.");
        debug_assert!(sat_divisions >= 2, "Must have at least 2 sat divisions.");

        let val_divisions = if val_divisions == 0 { 1 } else { val_divisions };

        if hue_divisions == self.hue_divisions
            && sat_divisions == self.sat_divisions
            && val_divisions == self.val_divisions
        {
            return Ok(());
        }

        self.hue_divisions = hue_divisions;
        self.sat_divisions = sat_divisions;
        self.val_divisions = val_divisions;

        self.hue_step = sat_divisions;
        self.val_step = hue_divisions * self.hue_step;

        let size = self.deltas_count()? as usize;

        self.deltas = Arc::new(vec![HSBModify::default(); size]);
        self.runtime_fingerprint.clear();

        Ok(())
    }

    /// `DeltasCount ()`.
    pub fn deltas_count(&self) -> DngResult<u32> {
        let v = dng_core::dng_safe_arithmetic::safe_uint32_mult2(
            self.val_divisions,
            self.hue_divisions,
        )?;
        dng_core::dng_safe_arithmetic::safe_uint32_mult2(v, self.sat_divisions)
    }

    /// `GetDelta (hueDiv, satDiv, valDiv, modify)`.
    pub fn get_delta(&self, hue_div: u32, sat_div: u32, val_div: u32) -> DngResult<HSBModify> {
        if hue_div >= self.hue_divisions
            || sat_div >= self.sat_divisions
            || val_div >= self.val_divisions
            || self.deltas.is_empty()
        {
            return Err(program_error()); // "Bad parameters to GetDelta"
        }

        let offset = (val_div * self.val_step + hue_div * self.hue_step + sat_div) as usize;

        Ok(self.deltas[offset])
    }

    /// `SetDelta (hueDiv, satDiv, valDiv, modify)`.
    pub fn set_delta(
        &mut self,
        hue_div: u32,
        sat_div: u32,
        val_div: u32,
        modify: HSBModify,
    ) -> DngResult<()> {
        self.set_delta_known_writeable(hue_div, sat_div, val_div, modify)
    }

    /// `SetDeltaKnownWriteable`.
    pub fn set_delta_known_writeable(
        &mut self,
        hue_div: u32,
        sat_div: u32,
        val_div: u32,
        modify: HSBModify,
    ) -> DngResult<()> {
        if hue_div >= self.hue_divisions
            || sat_div >= self.sat_divisions
            || val_div >= self.val_divisions
            || self.deltas.is_empty()
        {
            return Err(program_error()); // "Bad parameters to SetDelta"
        }

        let offset = (val_div * self.val_step + hue_div * self.hue_step + sat_div) as usize;

        let deltas: &mut Vec<HSBModify> = Arc::make_mut(&mut self.deltas);
        deltas[offset] = modify;

        // The zero saturation entry is required to have a value scale of 1.0.
        if sat_div == 0 && modify.val_scale != 1.0 {
            deltas[offset].val_scale = 1.0;
        }

        // If setting the first saturation entry, fill the zero-saturation
        // entry by extrapolating the first saturation entry.
        if sat_div == 1 {
            let zero_sat_modify = self.get_delta(hue_div, 0, val_div)?;

            if zero_sat_modify.val_scale != 1.0 {
                let extrapolated = HSBModify {
                    hue_shift: modify.hue_shift,
                    sat_scale: modify.sat_scale,
                    val_scale: 1.0,
                };
                self.set_delta(hue_div, 0, val_div, extrapolated)?;
            }
        }

        Ok(())
    }

    /// `GetConstDeltas ()` — the flat delta table.
    pub fn get_const_deltas(&self) -> &[HSBModify] {
        &self.deltas
    }

    /// `GetDeltas ()` — writable access (CoW).
    pub fn get_deltas(&mut self) -> &mut [HSBModify] {
        Arc::make_mut(&mut self.deltas).as_mut_slice()
    }

    /// `AssignNewUniqueRuntimeFingerprint ()`.
    pub fn assign_new_unique_runtime_fingerprint(&mut self) {
        let counter = sRuntimeFingerprintCounter.fetch_add(1, Ordering::Relaxed);

        let mut fp = DngFingerprint::new();

        // Counter packed big-endian into the first 8 bytes (the C++ packs the
        // atomic counter the same way through the MD5 printer path).
        let bytes = counter.to_be_bytes();
        fp.data[..8].copy_from_slice(&bytes);

        self.runtime_fingerprint = fp;
    }

    /// `SetRuntimeFingerprint (fingerprint)`.
    pub fn set_runtime_fingerprint(&mut self, fingerprint: DngFingerprint) {
        self.runtime_fingerprint = fingerprint;
    }

    /// `RuntimeFingerprint ()`.
    pub fn runtime_fingerprint(&self) -> DngFingerprint {
        self.runtime_fingerprint
    }

    /// `Interpolate (map1, map2, weight1)` — the 3-float lerp variant.
    pub fn interpolate(map1: &DngHueSatMap, map2: &DngHueSatMap, weight1: f64) -> DngResult<DngHueSatMap> {
        if weight1 >= 1.0 {
            if !map1.is_valid() {
                return Err(program_error()); // "map1 is not valid"
            }
            return Ok(map1.clone());
        }

        if weight1 <= 0.0 {
            if !map2.is_valid() {
                return Err(program_error()); // "map2 is not valid"
            }
            return Ok(map2.clone());
        }

        if !map1.is_valid() || !map2.is_valid() {
            return Err(program_error()); // "map1 or map2 is not valid"
        }

        if map1.hue_divisions != map2.hue_divisions
            || map1.sat_divisions != map2.sat_divisions
            || map1.val_divisions != map2.val_divisions
        {
            return Err(program_error()); // "map1 and map2 have different sizes"
        }

        let mut result = DngHueSatMap::new();
        result.set_divisions(map1.hue_divisions, map1.sat_divisions, map1.val_divisions)?;

        let w1 = weight1 as f32;
        let w2 = 1.0 - w1;

        let data1 = map1.get_const_deltas();
        let data2 = map2.get_const_deltas();

        let count = map1.deltas_count()? as usize;

        {
            let data3: &mut Vec<HSBModify> = Arc::make_mut(&mut result.deltas);

            for index in 0..count {
                data3[index].hue_shift = w1 * data1[index].hue_shift + w2 * data2[index].hue_shift;
                data3[index].sat_scale = w1 * data1[index].sat_scale + w2 * data2[index].sat_scale;
                data3[index].val_scale = w1 * data1[index].val_scale + w2 * data2[index].val_scale;
            }
        }

        // Fingerprint from the inputs (MD5 of "Interpolate" + weight + both
        // runtime fingerprints), matching the C++.
        let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();
        printer.process(b"Interpolate");
        printer.process(&weight1.to_le_bytes());
        printer.process(&map1.runtime_fingerprint().data);
        printer.process(&map2.runtime_fingerprint().data);
        result.set_runtime_fingerprint(*printer.result());

        Ok(result)
    }
    /// `Interpolate (map1, map2, map3, weight1, weight2)` — the
    /// triple-illuminant variant (dng_hue_sat_map.cpp).
    pub fn interpolate3(
        map1: &DngHueSatMap,
        map2: &DngHueSatMap,
        map3: &DngHueSatMap,
        weight1: f64,
        weight2: f64,
    ) -> DngResult<DngHueSatMap> {
        if weight1 >= 1.0 {
            if !map1.is_valid() {
                return Err(program_error()); // "map1 is not valid"
            }
            return Ok(map1.clone());
        }

        if weight2 >= 1.0 {
            if !map2.is_valid() {
                return Err(program_error()); // "map2 is not valid"
            }
            return Ok(map2.clone());
        }

        let weight3 = 1.0 - (weight1 + weight2);

        if weight3 >= 1.0 {
            if !map3.is_valid() {
                return Err(program_error()); // "map3 is not valid"
            }
            return Ok(map3.clone());
        }

        // None of the weights can be negative.

        if weight1 < 0.0 || weight2 < 0.0 || weight3 < 0.0 {
            return Err(program_error()); // "Invalid weight"
        }

        // Must all be valid.

        if !map1.is_valid() || !map2.is_valid() || !map3.is_valid() {
            return Err(program_error()); // "map is not valid"
        }

        // Must have the same dimensions.

        if map1.hue_divisions != map2.hue_divisions
            || map1.hue_divisions != map3.hue_divisions
            || map1.sat_divisions != map2.sat_divisions
            || map1.sat_divisions != map3.sat_divisions
            || map1.val_divisions != map2.val_divisions
            || map1.val_divisions != map3.val_divisions
        {
            return Err(program_error()); // "map1, map2, map3 have different sizes"
        }

        let mut result = DngHueSatMap::new();
        result.set_divisions(map1.hue_divisions, map1.sat_divisions, map1.val_divisions)?;

        let w1 = weight1 as f32;
        let w2 = weight2 as f32;
        let w3 = 1.0f32 - (w1 + w2);

        let data1 = map1.get_const_deltas();
        let data2 = map2.get_const_deltas();
        let data3 = map3.get_const_deltas();

        let count = map1.deltas_count()? as usize;

        {
            let data_result: &mut Vec<HSBModify> = Arc::make_mut(&mut result.deltas);

            for index in 0..count {
                data_result[index].hue_shift = w1 * data1[index].hue_shift
                    + w2 * data2[index].hue_shift
                    + w3 * data3[index].hue_shift;
                data_result[index].sat_scale = w1 * data1[index].sat_scale
                    + w2 * data2[index].sat_scale
                    + w3 * data3[index].sat_scale;
                data_result[index].val_scale = w1 * data1[index].val_scale
                    + w2 * data2[index].val_scale
                    + w3 * data3[index].val_scale;
            }
        }

        // Fingerprint from the inputs ("Interpolate3" + weights + all three
        // runtime fingerprints), matching the C++.

        let mut printer = dng_core::dng_fingerprint::DngMd5Printer::new();
        printer.process(b"Interpolate3");
        printer.process(&weight1.to_le_bytes());
        printer.process(&weight2.to_le_bytes());
        printer.process(&map1.runtime_fingerprint().data);
        printer.process(&map2.runtime_fingerprint().data);
        printer.process(&map3.runtime_fingerprint().data);
        result.set_runtime_fingerprint(*printer.result());

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn divisions_and_delta_roundtrip() {
        let mut map = DngHueSatMap::new();
        assert!(map.is_null());

        map.set_divisions(8, 4, 2).unwrap();
        assert!(map.is_valid());
        assert_eq!(map.get_divisions(), (8, 4, 2));
        assert_eq!(map.deltas_count().unwrap(), 2 * 8 * 4);

        let modify = HSBModify {
            hue_shift: 0.5,
            sat_scale: 1.2,
            val_scale: 1.0,
        };
        map.set_delta(3, 2, 1, modify).unwrap();
        assert_eq!(map.get_delta(3, 2, 1).unwrap(), modify);

        // Zero-saturation val_scale is forced to 1.0.
        map.set_delta(3, 0, 1, HSBModify { val_scale: 2.0, ..Default::default() })
            .unwrap();
        assert_eq!(map.get_delta(3, 0, 1).unwrap().val_scale, 1.0);

        // Out-of-range errors.
        assert!(map.get_delta(8, 0, 0).is_err());
        assert!(map.set_delta(0, 4, 0, modify).is_err());
    }

    #[test]
    fn interpolate_endpoints() {
        let mut map = DngHueSatMap::new();
        map.set_divisions(4, 4, 1).unwrap();
        map.set_delta(0, 1, 0, HSBModify { hue_shift: 1.0, sat_scale: 1.0, val_scale: 1.0 })
            .unwrap();
        map.assign_new_unique_runtime_fingerprint();

        // weight 1.0 -> map1.
        let a = DngHueSatMap::interpolate(&map, &map, 1.0).unwrap();
        assert_eq!(a.get_delta(0, 1, 0).unwrap().hue_shift, 1.0);

        // weight 0.0 -> map2.
        let b = DngHueSatMap::interpolate(&map, &map, 0.0).unwrap();
        assert_eq!(b.get_delta(0, 1, 0).unwrap().hue_shift, 1.0);

        // Half-way between identical maps is the same values.
        let m = DngHueSatMap::interpolate(&map, &map, 0.5).unwrap();
        assert!((m.get_delta(0, 1, 0).unwrap().hue_shift - 1.0).abs() < 1e-6);
        // Fingerprint is derived from the inputs (deterministic).
        let m2 = DngHueSatMap::interpolate(&map, &map, 0.5).unwrap();
        assert_eq!(m.runtime_fingerprint(), m2.runtime_fingerprint());
    }

    #[test]
    fn interpolate_requires_same_dims() {
        let mut a = DngHueSatMap::new();
        a.set_divisions(4, 4, 1).unwrap();
        let mut b = DngHueSatMap::new();
        b.set_divisions(6, 4, 1).unwrap();

        assert!(DngHueSatMap::interpolate(&a, &b, 0.5).is_err());
        assert!(DngHueSatMap::interpolate(&a, &DngHueSatMap::new(), 0.5).is_err());
    }

    #[test]
    fn unique_fingerprints() {
        let mut a = DngHueSatMap::new();
        let mut b = DngHueSatMap::new();
        a.assign_new_unique_runtime_fingerprint();
        b.assign_new_unique_runtime_fingerprint();
        assert_ne!(a.runtime_fingerprint(), b.runtime_fingerprint());
    }
}

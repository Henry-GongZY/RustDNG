// Port of dng_opcode_list.h/.cpp — a list of opcodes.
//
// The C++ list owns raw `dng_opcode *` pointers; the port stores
// `Box<dyn DngOpcode>` entries.

use dng_core::{
    bad_format, dngVersion_None, max_uint32, DngHost, DngResult, DngStream, DngURational, uint32,
};
use dng_core::dng_memory::DngMemoryBlock;
use dng_core::dng_memory_stream::DngMemoryStream;
use dng_core::dng_stream::TempBigEndian;
use crate::dng_negative::DngNegative;
use crate::dng_opcodes::{DngOpcode, dngOpcode_Private};

/// Port of `dng_opcode_list`.
#[derive(Default)]
pub struct DngOpcodeList {
    list: Vec<Box<dyn DngOpcode>>,

    always_apply: bool,

    stage: uint32,
}

impl DngOpcodeList {
    /// `dng_opcode_list (stage)` — create an empty opcode list for the
    /// specific image stage (1, 2, or 3).
    pub fn new(stage: uint32) -> Self {
        DngOpcodeList {
            list: Vec::new(),
            always_apply: false,
            stage,
        }
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `AlwaysApply ()`.
    pub fn always_apply(&self) -> bool {
        self.always_apply && self.not_empty()
    }

    /// `SetAlwaysApply ()`.
    pub fn set_always_apply(&mut self) {
        self.always_apply = true;
    }

    /// `Count ()`.
    pub fn count(&self) -> uint32 {
        self.list.len() as uint32
    }

    /// `Entry (index)` — read-only access.
    ///
    /// # Panics
    /// Panics if index is out of range (C++ has unchecked access; the port
    /// bounds-checks via slice indexing).
    pub fn entry(&self, index: uint32) -> &dyn DngOpcode {
        self.list[index as usize].as_ref()
    }

    /// `Entry (index)` — read/write access.
    pub fn entry_mut(&mut self, index: uint32) -> &mut dyn DngOpcode {
        self.list[index as usize].as_mut()
    }

    /// `Clear ()` — remove all opcodes from the list.
    pub fn clear(&mut self) {
        self.list.clear();
        self.always_apply = false;
    }

    /// `Remove (index)` — remove the opcode at a specific index.
    pub fn remove(&mut self, index: uint32) {
        assert!(
            (index as usize) < self.list.len(),
            "Index out of range in dng_opcode_list::Remove"
        );

        self.list.remove(index as usize);

        if self.list.is_empty() {
            self.always_apply = false;
        }
    }

    /// `Swap (otherList)`.
    pub fn swap(&mut self, other_list: &mut DngOpcodeList) {
        std::mem::swap(&mut self.list, &mut other_list.list);
        std::mem::swap(&mut self.always_apply, &mut other_list.always_apply);
        std::mem::swap(&mut self.stage, &mut other_list.stage);
    }

    /// `MinVersion (includeOptional)` — minimum DNG version required to
    /// support all opcodes in this list.
    pub fn min_version(&self, include_optional: bool) -> uint32 {
        let mut result = dngVersion_None;

        for opcode in &self.list {
            if include_optional || !opcode.optional() {
                result = max_uint32(result, opcode.min_version());
            }
        }

        result
    }

    /// `Apply (host, negative, image)`.
    pub fn apply(
        &mut self,
        host: &mut DngHost,
        negative: &mut DngNegative,
        image: &mut dng_core::ImageRef,
    ) -> DngResult<()> {
        let mut prev_opcode_was_optional_warp_rectilinear2 = false;

        let (image_bounds, image_planes) = {
            let img = image.read().unwrap();
            (img.bounds(), img.planes())
        };

        for index in 0..self.count() {
            let opcode = self.list[index as usize].as_mut();

            // Deal with WarpRectilinear2 skip rule.

            if prev_opcode_was_optional_warp_rectilinear2
                && (opcode.opcode_id() == crate::dng_opcodes::dngOpcode_WarpRectilinear
                    || opcode.opcode_id() == crate::dng_opcodes::dngOpcode_WarpFisheye)
            {
                continue;
            }

            if opcode.about_to_apply(host, negative, &image_bounds, image_planes)? {
                opcode.apply(host, negative, image)?;

                prev_opcode_was_optional_warp_rectilinear2 =
                    opcode.optional()
                        && opcode.opcode_id() == crate::dng_opcodes::dngOpcode_WarpRectilinear2;
            }
        }

        Ok(())
    }

    /// `Append (opcode)` — append the specified opcode to this list.
    pub fn append(&mut self, mut opcode: Box<dyn DngOpcode>) {
        if opcode.opcode_id() == dngOpcode_Private {
            self.set_always_apply();
        }

        opcode.set_stage(self.stage);

        self.list.push(opcode);
    }

    /// `Spool (host)` — serialize this opcode list to a block of memory.
    /// Returns `None` for an empty list (C++ returns NULL).
    pub fn spool(&self) -> DngResult<Option<DngMemoryBlock>> {
        if self.is_empty() {
            return Ok(None);
        }

        if self.always_apply() {
            return Err(dng_core::program_error());
        }

        let mut stream = DngMemoryStream::with_page_size(2048).stream();

        stream.set_big_endian(true);

        stream.put_uint32(self.count())?;

        for opcode in &self.list {
            stream.put_uint32(opcode.opcode_id())?;
            stream.put_uint32(opcode.min_version())?;
            stream.put_uint32(opcode.flags())?;

            opcode.put_data(&mut stream)?;
        }

        Ok(Some(stream.as_memory_block()?))
    }

    /// `FingerprintToStream (stream)` — write a fingerprint of this opcode
    /// list to the specified stream.
    pub fn fingerprint_to_stream(&self, stream: &mut DngStream) -> DngResult<()> {
        if self.is_empty() {
            return Ok(());
        }

        stream.put_uint32(self.count())?;

        for opcode in &self.list {
            stream.put_uint32(opcode.opcode_id())?;
            stream.put_uint32(opcode.min_version())?;
            stream.put_uint32(opcode.flags())?;

            if opcode.opcode_id() != dngOpcode_Private {
                opcode.put_data(stream)?;
            }
        }

        Ok(())
    }

    /// `Parse (host, stream, byteCount, streamOffset)` — read an opcode list
    /// from the specified stream, starting at the specified offset.
    pub fn parse(
        &mut self,
        _host: &mut DngHost,
        stream: &mut DngStream,
        byte_count: uint32,
        stream_offset: u64,
    ) -> DngResult<()> {
        self.clear();

        {
            let mut temp_big_endian = TempBigEndian::new(stream, true);

            let stream = temp_big_endian.stream();

            stream.set_read_position(stream_offset)?;

            let count = stream.get_uint32()?;

            for _index in 0..count {
                let opcode_id = stream.get_uint32()?;

                let opcode = crate::dng_opcodes::make_dng_opcode(opcode_id, stream)?;

                self.append(opcode);
            }

            if stream.position() != stream_offset + byte_count as u64 {
                return Err(bad_format()); // "Error parsing opcode list"
            }
        }

        Ok(())
    }

    /// `ApplyAreaScale (scale)` — apply an area scale factor to all opcodes
    /// in the list.
    pub fn apply_area_scale(&mut self, scale: DngURational) {
        if scale.not_valid() {
            return;
        }

        if scale.n == scale.d {
            return;
        }

        assert!(scale.n > 0, "Bad scale factor in dng_opcode_list::Scale");

        for opcode in &mut self.list {
            opcode.apply_area_scale(scale);
        }
    }
}

// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dng_misc_opcodes::DngOpcodeTrimBounds;
    use dng_core::DngMemoryStream;
    use dng_core::DngRect;

    fn sample_list() -> DngOpcodeList {
        let mut list = DngOpcodeList::new(2);

        list.append(Box::new(DngOpcodeTrimBounds::new(DngRect::from_corners(
            1, 1, 9, 9,
        ))));

        list
    }

    #[test]
    fn empty_list_basics() {
        let list = DngOpcodeList::new(1);

        assert!(list.is_empty());
        assert!(!list.not_empty());
        assert!(!list.always_apply());
        assert_eq!(list.count(), 0);
        assert_eq!(list.min_version(true), dngVersion_None);
    }

    #[test]
    fn append_and_entry() {
        let list = sample_list();

        assert!(list.not_empty());
        assert_eq!(list.count(), 1);
        assert_eq!(list.entry(0).opcode_id(), crate::dng_opcodes::dngOpcode_TrimBounds);
        assert_eq!(list.entry(0).stage(), 2);
    }

    #[test]
    fn spool_and_parse_roundtrip() {
        let list = sample_list();

        let block = list.spool().unwrap().expect("non-empty list spools");

        let data = block.buffer().to_vec();

        // Parse it back through a memory stream.
        let mut host = DngHost::new();
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put(&data).unwrap();
        stream.set_read_position(0).unwrap();

        let mut parsed = DngOpcodeList::new(2);
        parsed
            .parse(&mut host, &mut stream, data.len() as uint32, 0)
            .unwrap();

        assert_eq!(parsed.count(), 1);
        assert_eq!(
            parsed.entry(0).opcode_id(),
            crate::dng_opcodes::dngOpcode_TrimBounds
        );

        // Position must exactly match streamOffset + byteCount.
        assert_eq!(stream.position(), data.len() as u64);
    }

    #[test]
    fn parse_length_mismatch_is_bad_format() {
        let mut host = DngHost::new();

        // Two opcodes-worth of data but claim a shorter byte count.
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put_uint32(1).unwrap();
        stream.put_uint32(crate::dng_opcodes::dngOpcode_TrimBounds).unwrap();
        // TrimBounds data: 4 * int32 = 16 bytes.
        for _ in 0..16 {
            stream.put_uint8(0).unwrap();
        }
        stream.set_read_position(0).unwrap();

        let mut parsed = DngOpcodeList::new(1);
        assert!(parsed.parse(&mut host, &mut stream, 10, 0).is_err());
    }

    #[test]
    fn min_version_respects_optional() {
        // Build a list with one opcode whose minVersion is 1.4 and which is
        // optional (flags = 1): read from stream.
        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        stream.put_uint32(0x01040000).unwrap(); // minVersion
        stream.put_uint32(1).unwrap(); // flags: optional
        stream.put_uint32(0).unwrap(); // no data
        stream.set_read_position(0).unwrap();

        let unknown = crate::dng_opcodes::DngOpcodeUnknown::from_stream(9001, &mut stream).unwrap();

        let mut list = DngOpcodeList::new(3);
        list.append(Box::new(unknown));

        // Optional excluded.
        assert_eq!(list.min_version(false), dngVersion_None);
        // Optional included.
        assert_eq!(list.min_version(true), 0x01040000);
    }

    #[test]
    fn fingerprint_skips_private_data() {
        let list = sample_list();

        let mut stream = DngMemoryStream::with_page_size(64).stream();
        stream.set_big_endian(true);
        list.fingerprint_to_stream(&mut stream).unwrap();

        stream.set_read_position(0).unwrap();
        let stream_len = stream.length();

        let data = stream.get(stream_len as uint32).unwrap();

        // count + opcodeID + minVersion + flags + PutData (dataSize + 16-byte
        // TrimBounds area, matching the C++ FingerprintToStream layout).
        assert_eq!(data.len(), 4 + 4 + 4 + 4 + 4 + 16);
    }

    #[test]
    fn swap_and_remove() {
        let mut a = sample_list();
        let mut b = DngOpcodeList::new(3);

        b.set_always_apply();

        a.swap(&mut b);

        assert!(a.always_apply_flag_for_test());
        assert_eq!(b.count(), 1);
        assert_eq!(b.entry(0).stage(), 2);

        b.remove(0);
        assert!(b.is_empty());
        assert!(!b.always_apply_flag_for_test());
    }

    #[test]
    fn apply_area_scale_noop_values() {
        let mut list = sample_list();

        // NotValid (0/0) and 1/1 are no-ops.
        list.apply_area_scale(DngURational { n: 0, d: 0 });
        list.apply_area_scale(DngURational { n: 1, d: 1 });

        // Original bounds (1,1)-(9,9): header is 16 bytes, then dataSize (4),
        // then 4 x int32 (matching the C++ spool layout).
        let block0 = list.spool().unwrap().unwrap();
        assert_eq!(&block0.buffer()[20..36], &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, 9]);

        // 2/1 scales area-spec-based opcodes only. TrimBounds has no
        // ApplyAreaScale override in C++ (base is a no-op), so its bounds
        // are unchanged.
        list.apply_area_scale(DngURational { n: 2, d: 1 });

        let block = list.spool().unwrap().unwrap();
        assert_eq!(&block.buffer()[20..36], &[0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 9, 0, 0, 0, 9]);
    }
}

// Test-only helper accessors.
#[cfg(test)]
impl DngOpcodeList {
    fn always_apply_flag_for_test(&self) -> bool {
        self.always_apply
    }
}

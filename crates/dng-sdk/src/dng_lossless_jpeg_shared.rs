// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_lossless_jpeg_shared.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Port of `dng_lossless_jpeg_shared.cpp` — the shared lossless JPEG
//! decoder and encoder (TIFF compression code 34892 / ccLossyJPEG "lossless
//! JPEG"), including Canon sRAW, Sony sRAW and Hasselblad 3FR support.
//!
//! The C++ file is a template over `SIMDType`; this port implements the
//! `Scalar` path only (SIMD variants are performance bottlenecks, not
//! behavior — see PORTING_NOTES.md).
//!
//! Adapted from the Independent JPEG Group's lossless JPEG code
//! (Thomas G. Lane / Brian C. Smith / Kongji Huang), as in the original.

use dng_core::dng_exceptions::bad_format;
use dng_core::dng_tags::{
    M_APP0, M_DHT, M_DQT, M_DRI, M_EOI, M_ERROR, M_JPG, M_RST0, M_RST7, M_SOF0, M_SOF1, M_SOF10,
    M_SOF11, M_SOF13, M_SOF14, M_SOF15, M_SOF2, M_SOF3, M_SOF5, M_SOF6, M_SOF7, M_SOF9, M_SOI,
    M_SOS, M_TEM,
};
use dng_core::dng_types::{int16, int32, int64, int8, uint16, uint32, uint64, uint8};
use dng_core::dng_stream::DngStream;
use dng_core::{DngError, DngResult};

use crate::dng_lossless_jpeg::DngSpooler;

/// `qSupportCanon_sRAW` (dng_lossless_jpeg_shared.cpp:65).
pub const qSupportCanon_sRAW: bool = true;
/// `qSupportSony_sRAW` (dng_lossless_jpeg_shared.cpp:72).
pub const qSupportSony_sRAW: bool = true;
/// `qSupportHasselblad_3FR` (dng_lossless_jpeg_shared.cpp:79).
pub const qSupportHasselblad_3FR: bool = true;

// ---------------------------------------------------------------------------
// Huffman table
// ---------------------------------------------------------------------------

/// One structure per Huffman coding table; shared between encoding and
/// decoding (`struct HuffmanTable`).
#[derive(Debug, Clone)]
pub struct HuffmanTable {
    /// Contents of a JPEG DHT marker (`bits[17]`).
    pub bits: [uint8; 17],
    /// Contents of a JPEG DHT marker (`huffval[256]`).
    pub huffval: [uint8; 256],

    /// Derived decoding tables.
    pub mincode: [uint16; 17],
    pub maxcode: [int32; 18],
    pub valptr: [int16; 17],
    pub numbits: [int32; 256],
    pub value: [int32; 256],

    /// Derived encoding tables.
    pub ehufco: [uint16; 256],
    pub ehufsi: [int8; 256],
}

impl Default for HuffmanTable {
    fn default() -> Self {
        HuffmanTable {
            bits: [0; 17],
            huffval: [0; 256],
            mincode: [0; 17],
            maxcode: [0; 18],
            valptr: [0; 17],
            numbits: [0; 256],
            value: [0; 256],
            ehufco: [0; 256],
            ehufsi: [0; 256],
        }
    }
}

impl HuffmanTable {
    pub fn new() -> Self {
        HuffmanTable::default()
    }
}

/// Computes the derived fields in the Huffman table structure
/// (`FixHuffTbl`).
pub fn fix_huff_tbl(htbl: &mut HuffmanTable) -> DngResult<()> {
    // Figure C.1: make table of Huffman code length for each symbol
    // (code-length order).

    let mut huffsize = [0u8; 257];

    let mut p: usize = 0;

    for l in 1..=16usize {
        for _ in 0..htbl.bits[l] {
            huffsize[p] = l as u8;
            p += 1;
        }
    }

    huffsize[p] = 0;

    let lastp = p;

    // Figure C.2: generate the codes themselves (code-length order).

    let mut huffcode = [0u16; 257];

    let mut code: uint16 = 0;

    let mut si = huffsize[0] as i32;

    p = 0;

    while huffsize[p] != 0 {
        while (huffsize[p] as i32) == si {
            huffcode[p] = code;
            p += 1;
            code += 1;
        }

        code <<= 1;

        si += 1;
    }

    // Figure C.3: generate encoding tables (code and size indexed by symbol
    // value). Set any codeless symbols to have code length 0; this allows
    // EmitBits to detect any attempt to emit such symbols.
    //
    // PORT-NOTE: the C++ zeroes only ehufsi and leaves ehufco untouched for
    // codeless symbols; ehufsi == 0 makes those entries unreachable, so
    // zeroing ehufco too is observably identical and avoids stale reads in
    // safe Rust.

    htbl.ehufsi = [0; 256];
    htbl.ehufco = [0; 256];

    for p in 0..lastp {
        let sym = htbl.huffval[p] as usize;
        htbl.ehufco[sym] = huffcode[p];
        htbl.ehufsi[sym] = huffsize[p] as int8;
    }

    // Figure F.15: generate decoding tables.

    p = 0;

    for l in 1..=16usize {
        if htbl.bits[l] != 0 {
            htbl.valptr[l] = p as int16;
            htbl.mincode[l] = huffcode[p];

            p += htbl.bits[l] as usize;

            htbl.maxcode[l] = huffcode[p - 1] as int32;
        } else {
            htbl.maxcode[l] = -1;
        }
    }

    // We put in this value to ensure HuffDecode terminates.

    htbl.maxcode[17] = 0xFFFFF;

    // Build the numbits, value lookup tables: gather 8 bits from the bit
    // stream and immediately look up the size and value of the Huffman code.
    // Size zero means more than 8 bits are in the Huffman code (about 3-4%
    // of the time).

    htbl.numbits = [0; 256];

    for p in 0..lastp {
        let size = huffsize[p] as i32;

        if size <= 8 {
            let value = htbl.huffval[p] as int32;

            let code = huffcode[p];

            let ll = (code as i32) << (8 - size);

            let ul = if size < 8 {
                ll | ((1i32 << (8 - size)) - 1) // bitMask[24 + size]
            } else {
                ll
            };

            if ul >= 256 {
                return Err(bad_format());
            }

            for i in ll..=ul {
                htbl.numbits[i as usize] = size;
                htbl.value[i as usize] = value;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Component / stream info structures
// ---------------------------------------------------------------------------

/// Basic information about one component (`struct JpegComponentInfo`).
#[derive(Debug, Clone, Copy, Default)]
pub struct JpegComponentInfo {
    /// Identifier for this component (0..255), read from the SOF marker.
    pub component_id: int16,
    /// Its index in SOF / `comp_info`.
    pub component_index: int16,
    /// Horizontal sampling factor.
    pub h_samp_factor: int16,
    /// Vertical sampling factor.
    pub v_samp_factor: int16,
    /// Huffman table selector (0..3), read from the SOS marker.
    pub dc_tbl_no: int16,
}

/// Decompression information (`struct DecompressInfo`).
//
// PORT-NOTE: the C++ holds raw pointers (`compInfo`, `curCompInfo[4]`,
// `dcHuffTblPtrs[4]`); the Rust port stores component tables as indices into
// decoder-owned storage (`comp_info` Vec and `huff_tables` array), which the
// `cur_comp_info` / `dc_huff_tbl_used` fields mirror.
#[derive(Debug, Clone, Copy)]
pub struct DecompressInfo {
    /// Image width, height, and data precision (bits/sample).
    pub image_width: int32,
    pub image_height: int32,
    pub data_precision: int32,

    /// Number of color components in SOF (`numComponents`).
    pub num_components: int16,

    /// Number of color components in the current scan (`compsInScan`).
    pub comps_in_scan: int16,

    /// Indices into the decoder's component storage for the components in
    /// the scan (C++ `curCompInfo[4]`).
    pub cur_comp_info: [int16; 4],

    /// `MCUmembership[i]` — indexes the i'th component of an MCU into the
    /// `cur_comp_info` array.
    pub mcu_membership: [int16; 10],

    /// Which Huffman tables are defined (C++ `dcHuffTblPtrs[4]` non-null).
    pub dc_huff_tbl_used: [bool; 4],

    /// Prediction selection value (PSV) and point transform parameter (Pt).
    pub ss: int32,
    pub pt: int32,

    /// MCUs per restart interval (0 = no restart) / MCU rows per restart.
    pub restart_interval: int32,
    pub restart_in_rows: int32,

    /// Entropy decoder private state.
    pub restart_rows_to_go: int32,
    pub next_restart_num: int16,
}

impl Default for DecompressInfo {
    fn default() -> Self {
        DecompressInfo {
            image_width: 0,
            image_height: 0,
            data_precision: 0,
            num_components: 0,
            comps_in_scan: 0,
            cur_comp_info: [0; 4],
            mcu_membership: [0; 10],
            dc_huff_tbl_used: [false; 4],
            ss: 0,
            pt: 0,
            restart_interval: 0,
            restart_in_rows: 0,
            restart_rows_to_go: 0,
            next_restart_num: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Decoder
// ---------------------------------------------------------------------------

/// Port of `dng_lossless_decoder<simd>` (Scalar specialization).
pub struct LosslessDecoder<'s, 'a, S: DngSpooler + ?Sized> {
    stream: &'s mut DngStream<'a>,
    spooler: &'s mut S,
    /// Decode data with the "16-bit" bug (`fBug16`).
    bug16: bool,

    /// C++ `huffmanBuffer[4]` — the four possible DHT tables.
    huff_tables: [Option<Box<HuffmanTable>>; 4],

    /// C++ `compInfoBuffer` — components from the SOF marker.
    comp_info: Vec<JpegComponentInfo>,

    info: DecompressInfo,

    /// C++ `mcuROW1` / `mcuROW2`: two rows of MCUs, stored flat as
    /// `row[col * comps_in_scan + comp]` (the C++ pointer table
    /// `mcuROW1[j] = mcuROW1[j-1] + compsInScan` uses the same layout).
    mcu_row1: Vec<uint16>,
    mcu_row2: Vec<uint16>,
    comps_in_scan_stride: usize,

    /// Current bit-extraction buffer / # of unused bits in it.
    get_buffer: uint64,
    bits_left: int32,

    /// `qSupportHasselblad_3FR` decoder flag (`fHasselblad3FR`).
    hasselblad_3fr: bool,
}

impl<'s, 'a, S: DngSpooler + ?Sized> LosslessDecoder<'s, 'a, S> {
    pub fn new(stream: &'s mut DngStream<'a>, spooler: &'s mut S, bug16: bool) -> Self {
        LosslessDecoder {
            stream,
            spooler,
            bug16,
            huff_tables: [None, None, None, None],
            comp_info: Vec::new(),
            info: DecompressInfo::default(),
            mcu_row1: Vec::new(),
            mcu_row2: Vec::new(),
            comps_in_scan_stride: 0,
            get_buffer: 0,
            bits_left: 0,
            hasselblad_3fr: false,
        }
    }

    /// `IsHasselblad3FR ()`.
    pub fn is_hasselblad_3fr(&self) -> bool {
        self.hasselblad_3fr
    }

    /// Stream position accessor (lets callers read `stream.Position ()`
    /// while the decoder still holds the mutable borrow).
    pub fn stream_position(&self) -> uint64 {
        self.stream.position()
    }

    // -- marker stream helpers ----------------------------------------------

    /// `GetJpegChar ()` — EOF/read errors propagate like C++ throws.
    #[inline]
    fn get_jpeg_char(&mut self) -> DngResult<int32> {
        Ok(self.stream.get_uint8()? as int32)
    }

    /// `UnGetJpegChar ()`.
    #[inline]
    fn un_get_jpeg_char(&mut self) -> DngResult<()> {
        let pos = self.stream.position();
        self.stream.set_read_position(pos - 1)
    }

    fn get2bytes(&mut self) -> DngResult<uint16> {
        let a = self.get_jpeg_char()?;
        Ok(((a << 8) + self.get_jpeg_char()?) as uint16)
    }

    /// Skip over an unknown or uninteresting variable-length marker
    /// (`SkipVariable`).
    fn skip_variable(&mut self) -> DngResult<()> {
        let length = (self.get2bytes()? as int32) - 2;
        self.stream.skip(length.max(0) as uint64)
    }

    /// Process a DHT marker (`GetDht`).
    fn get_dht(&mut self) -> DngResult<()> {
        let mut length = (self.get2bytes()? as int32) - 2;

        while length > 0 {
            let index = self.get_jpeg_char()?;

            if !(0..4).contains(&index) {
                return Err(bad_format());
            }

            let index = index as usize;

            if self.huff_tables[index].is_none() {
                self.huff_tables[index] = Some(Box::new(HuffmanTable::new()));
            }

            // PORT-NOTE: read all bytes first, then update the table, to
            // satisfy the borrow checker (the C++ interleaves the reads).

            let mut bits = [0u8; 17];

            let mut count: int32 = 0;

            for i in 1..=16usize {
                bits[i] = self.get_jpeg_char()? as uint8;
                count += bits[i] as int32;
            }

            if count > 256 {
                return Err(bad_format());
            }

            let mut huffval = [0u8; 256];

            for j in 0..count as usize {
                huffval[j] = self.get_jpeg_char()? as uint8;
            }

            let table = self.huff_tables[index].as_mut().unwrap();

            table.bits = bits;
            table.huffval = huffval;

            length -= 1 + 16 + count;
        }

        Ok(())
    }

    /// Process a DRI marker (`GetDri`).
    fn get_dri(&mut self) -> DngResult<()> {
        if self.get2bytes()? != 4 {
            return Err(bad_format());
        }

        self.info.restart_interval = self.get2bytes()? as int32;

        Ok(())
    }

    /// Process an APP0 marker (`GetApp0`).
    fn get_app0(&mut self) -> DngResult<()> {
        self.skip_variable()
    }

    /// Process a SOFn marker (`GetSof`).
    fn get_sof(&mut self, _code: int32) -> DngResult<()> {
        let length = self.get2bytes()? as int32;

        self.info.data_precision = self.get_jpeg_char()?;
        self.info.image_height = self.get2bytes()? as int32;
        self.info.image_width = self.get2bytes()? as int32;
        self.info.num_components = self.get_jpeg_char()? as int16;

        // We don't support files in which the image height is initially
        // specified as 0 and is later redefined by DNL.

        if self.info.image_height <= 0
            || self.info.image_width <= 0
            || self.info.num_components <= 0
        {
            return Err(bad_format());
        }

        // Lossless JPEG specifies data precision to be from 2 to 16
        // bits/sample.

        const MIN_PRECISION_BITS: int32 = 2;
        const MAX_PRECISION_BITS: int32 = 16;

        if self.info.data_precision < MIN_PRECISION_BITS
            || self.info.data_precision > MAX_PRECISION_BITS
        {
            return Err(bad_format());
        }

        // Check length of tag.

        if length != self.info.num_components as int32 * 3 + 8 {
            return Err(bad_format());
        }

        // Read in the per component info.
        //
        // PORT-NOTE: the C++ keeps components in a heap buffer addressed via
        // `info.compInfo` pointers; we keep them in a Vec owned by the
        // decoder, addressed by index.

        let mut comp_info = Vec::with_capacity(self.info.num_components as usize);

        for ci in 0..self.info.num_components as usize {
            let mut compptr = JpegComponentInfo::default();

            compptr.component_index = ci as int16;

            compptr.component_id = self.get_jpeg_char()? as int16;

            let c = self.get_jpeg_char()?;

            compptr.h_samp_factor = ((c >> 4) & 15) as int16;
            compptr.v_samp_factor = (c & 15) as int16;

            let _ = self.get_jpeg_char()?; // skip Tq

            comp_info.push(compptr);
        }

        self.comp_info = comp_info;

        Ok(())
    }

    /// Process a SOS marker (`GetSos`).
    fn get_sos(&mut self) -> DngResult<()> {
        let mut length = self.get2bytes()? as int32;

        // Get the number of image components.

        let n = self.get_jpeg_char()?;
        self.info.comps_in_scan = n as int16;

        // Check length.

        length -= 3;

        if length != n * 2 + 3 || !(1..=4).contains(&n) {
            return Err(bad_format());
        }

        // Find index and Huffman table for each component.

        for i in 0..n as usize {
            let cc = self.get_jpeg_char()?;
            let c = self.get_jpeg_char()?;

            let mut ci = 0usize;
            while ci < self.comp_info.len() {
                if cc == self.comp_info[ci].component_id as int32 {
                    break;
                }
                ci += 1;
            }

            if ci >= self.comp_info.len() {
                return Err(bad_format());
            }

            self.info.cur_comp_info[i] = ci as int16;

            self.comp_info[ci].dc_tbl_no = ((c >> 4) & 15) as int16;
        }

        // Get the PSV, skip Se, and get the point transform parameter.

        self.info.ss = self.get_jpeg_char()?;

        let _ = self.get_jpeg_char()?;

        self.info.pt = self.get_jpeg_char()? & 0x0F;

        Ok(())
    }

    /// Process an SOI marker (`GetSoi`).
    fn get_soi(&mut self) {
        // Reset all parameters that are defined to be reset by SOI.
        self.info.restart_interval = 0;
    }

    /// Find the next JPEG marker (`NextMarker`).
    fn next_marker(&mut self) -> DngResult<int32> {
        loop {
            // skip any non-FF bytes
            loop {
                let v = self.get_jpeg_char()?;
                if v == 0xFF {
                    break;
                }
            }

            // skip any duplicate FFs, since extra FFs are legal
            let mut v;
            loop {
                v = self.get_jpeg_char()?;
                if v != 0xFF {
                    break;
                }
            }

            if v != 0 {
                // repeat if it was a stuffed FF/00
                return Ok(v);
            }
        }
    }

    /// Scan and process JPEG markers that can appear in any order; return
    /// when an SOI, EOI, SOFn, or SOS is found (`ProcessTables`).
    fn process_tables(&mut self) -> DngResult<int32> {
        loop {
            let c = self.next_marker()?;

            match c as uint32 {
                M_SOF0 | M_SOF1 | M_SOF2 | M_SOF3 | M_SOF5 | M_SOF6 | M_SOF7 | M_JPG | M_SOF9
                | M_SOF10 | M_SOF11 | M_SOF13 | M_SOF14 | M_SOF15 | M_SOI | M_EOI | M_SOS => {
                    return Ok(c)
                }

                M_DHT => self.get_dht()?,

                M_DQT => {}

                M_DRI => self.get_dri()?,

                M_APP0 => self.get_app0()?,

                // these are all parameterless
                M_RST0..=M_RST7 | M_TEM => {}

                // must be DNL, DHP, EXP, APPn, JPGn, COM, or RESn
                _ => self.skip_variable()?,
            }
        }
    }

    /// Initialize and read the stream header, everything through the SOF
    /// marker (`ReadFileHeader`).
    fn read_file_header(&mut self) -> DngResult<()> {
        // Demand an SOI marker at the start of the stream --- otherwise it's
        // probably not a JPEG stream at all.

        let c = self.get_jpeg_char()?;
        let c2 = self.get_jpeg_char()?;

        if c != 0xFF || c2 != M_SOI as int32 {
            return Err(bad_format());
        }

        // OK, process SOI

        self.get_soi();

        // Process markers until SOF

        let c = self.process_tables()?;

        match c as uint32 {
            M_SOF0 | M_SOF1 | M_SOF3 => self.get_sof(c)?,

            _ => return Err(bad_format()),
        }

        Ok(())
    }

    /// Read the start of a scan, everything through the SOS marker
    /// (`ReadScanHeader`). Returns 1 if SOS found, 0 if EOI.
    fn read_scan_header(&mut self) -> DngResult<int32> {
        // Process markers until SOS or EOI

        let c = self.process_tables()?;

        match c as uint32 {
            M_SOS => {
                self.get_sos()?;
                Ok(1)
            }

            M_EOI => Ok(0),

            _ => Err(bad_format()),
        }
    }

    /// Initialize the rest of the fields in the decompression structure
    /// (`DecoderStructInit`).
    fn decoder_struct_init(&mut self) -> DngResult<()> {
        let canon_sraw = qSupportCanon_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 1
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 15
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0;

        let canon_sraw2 = qSupportCanon_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 2
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 15
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0
            && (self.info.image_height & 1) == 0;

        let sony_sraw = qSupportSony_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 2
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 16
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0
            && (self.info.image_height & 1) == 0;

        let sony_sraw2 = qSupportSony_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 1
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 16
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0
            && (self.info.image_height & 1) == 0;

        if !canon_sraw && !canon_sraw2 && !sony_sraw && !sony_sraw2 {
            // Check sampling factor validity.

            for comp in &self.comp_info {
                if comp.h_samp_factor != 1 || comp.v_samp_factor != 1 {
                    return Err(bad_format());
                }
            }
        }

        // Prepare array describing MCU composition.

        if !(0..=4).contains(&self.info.comps_in_scan) {
            return Err(bad_format());
        }

        for ci in 0..self.info.comps_in_scan as usize {
            self.info.mcu_membership[ci] = ci as int16;
        }

        // Initialize mcuROW1 and mcuROW2, which buffer two rows of pixels
        // for predictor calculation.
        //
        // PORT-NOTE: C++ allocates `imageWidth` MCU pointers plus backing
        // `imageWidth * compsInScan` uint16 samples; the flat layout here is
        // identical to the C++ pointer walk.

        self.comps_in_scan_stride = self.info.comps_in_scan as usize;

        let row_len = self.info.image_width as usize * self.comps_in_scan_stride;

        self.mcu_row1 = vec![0; row_len];
        self.mcu_row2 = vec![0; row_len];

        Ok(())
    }

    /// Initialize for a Huffman-compressed scan, invoked after reading the
    /// SOS marker (`HuffDecoderInit`).
    fn huff_decoder_init(&mut self) -> DngResult<()> {
        // Initialize bit parser state

        self.get_buffer = 0;
        self.bits_left = 0;

        // Prepare Huffman tables.

        for ci in 0..self.info.comps_in_scan as usize {
            let compptr_idx = self.info.cur_comp_info[ci] as usize;

            // Make sure requested tables are present

            let tbl_no = self.comp_info[compptr_idx].dc_tbl_no;

            if !(0..=3).contains(&tbl_no) {
                return Err(bad_format());
            }

            if self.huff_tables[tbl_no as usize].is_none() {
                return Err(bad_format());
            }

            // Compute derived values for Huffman tables (may be repeated for
            // the same table; not a problem).

            let table = self.huff_tables[tbl_no as usize].as_mut().unwrap();
            fix_huff_tbl(table)?;
        }

        // Initialize restart stuff

        self.info.restart_in_rows = if self.info.image_width != 0 {
            self.info.restart_interval / self.info.image_width
        } else {
            0
        };
        self.info.restart_rows_to_go = self.info.restart_in_rows;
        self.info.next_restart_num = 0;

        Ok(())
    }

    /// Check for a restart marker & resynchronize decoder (`ProcessRestart`).
    fn process_restart(&mut self) -> DngResult<()> {
        // Throw away any unused odd bits in the bit buffer.

        let pos = self.stream.position();
        self.stream
            .set_read_position(pos - (self.bits_left as uint64 / 8))?;

        self.bits_left = 0;
        self.get_buffer = 0;

        // Scan for next JPEG marker

        let c;

        loop {
            // skip any non-FF bytes
            loop {
                let v = self.get_jpeg_char()?;
                if v == 0xFF {
                    break;
                }
            }

            // skip any duplicate FFs
            let mut v;
            loop {
                v = self.get_jpeg_char()?;
                if v != 0xFF {
                    break;
                }
            }

            if v != 0 {
                c = v;
                break;
            }
        }

        // Verify correct restart code.

        if c != (M_RST0 as int32) + self.info.next_restart_num as int32 {
            return Err(bad_format());
        }

        // Update restart state.

        self.info.restart_rows_to_go = self.info.restart_in_rows;
        self.info.next_restart_num = (self.info.next_restart_num + 1) & 7;

        Ok(())
    }

    // -- pixel access ---------------------------------------------------------

    /// Flat row index for `[col][comp]` (matches the C++ MCU pointer walk).
    #[inline]
    fn row_index(&self, col: usize, comp: usize) -> usize {
        col * self.comps_in_scan_stride + comp
    }


    // -- bit buffer -------------------------------------------------------------

    /// Load up the bit buffer with at least `nbits`; process any stuffed
    /// bytes at this time (`FillBitBuffer`).
    fn fill_bit_buffer(&mut self, nbits: int32) -> DngResult<()> {
        let k_min_get_bits: int32 = 32 - 7; // sizeof (uint32) * 8 - 7

        if qSupportHasselblad_3FR && self.hasselblad_3fr {
            while self.bits_left < k_min_get_bits {
                // Read four bytes; tolerate a missing final EOI marker.

                let mut values: [int32; 4] = [0; 4];

                let mut eof_swallowed = false;

                for k in 0..4 {
                    match self.get_jpeg_char() {
                        Ok(v) => values[k] = v,
                        Err(e) => {
                            // Some Hasselblad files use the JPEG end of image
                            // marker; swallow EOF when the marker appeared in
                            // the previously-read bytes. This sequence also
                            // sometimes occurs in the image data, so we wait
                            // until the EOF and then look.

                            let saw_eoi = (values[0] == 0xFF && values[1] == 0xD9)
                                || (values[1] == 0xFF && values[2] == 0xD9);

                            if e != DngError::EndOfFile || !saw_eoi {
                                return Err(e);
                            }

                            eof_swallowed = true;
                            let _ = eof_swallowed;
                            // Fall through with zero-filled tail, matching
                            // the C++ catch-block behavior.
                            break;
                        }
                    }
                }

                self.get_buffer = (self.get_buffer << 8) | (values[3] as uint64);
                self.get_buffer = (self.get_buffer << 8) | (values[2] as uint64);
                self.get_buffer = (self.get_buffer << 8) | (values[1] as uint64);
                self.get_buffer = (self.get_buffer << 8) | (values[0] as uint64);

                self.bits_left += 32;
            }

            return Ok(());
        }

        while self.bits_left < k_min_get_bits {
            let mut c = self.get_jpeg_char()?;

            // If it's 0xFF, check and discard stuffed zero byte

            if c == 0xFF {
                let c2 = self.get_jpeg_char()?;

                if c2 != 0 {
                    // Oops, it's actually a marker indicating end of
                    // compressed data. Put it back for use later.

                    self.un_get_jpeg_char()?;
                    self.un_get_jpeg_char()?;

                    // There should be enough bits still left in the data
                    // segment; if so, just break out of the while loop.

                    if self.bits_left >= nbits {
                        break;
                    }

                    // Uh-oh. Corrupted data: stuff zeroes into the data
                    // stream, since this sometimes occurs when we are on the
                    // last show_bits8 during decoding of the Huffman segment.

                    c = 0;
                }
            }

            self.get_buffer = (self.get_buffer << 8) | (c as uint64);

            self.bits_left += 8;
        }

        Ok(())
    }

    #[inline]
    fn show_bits8(&mut self) -> DngResult<int32> {
        if self.bits_left < 8 {
            self.fill_bit_buffer(8)?;
        }

        Ok(((self.get_buffer >> (self.bits_left - 8)) & 0xff) as int32)
    }

    #[inline]
    fn flush_bits(&mut self, nbits: int32) {
        self.bits_left -= nbits;
    }

    #[inline]
    fn get_bits(&mut self, nbits: int32) -> DngResult<int32> {
        if nbits > 16 {
            return Err(bad_format());
        }

        if self.bits_left < nbits {
            self.fill_bit_buffer(nbits)?;
        }

        self.bits_left -= nbits;

        Ok(((self.get_buffer >> self.bits_left) & (0x0FFFF >> (16 - nbits))) as int32)
    }

    #[inline]
    fn get_bit(&mut self) -> DngResult<int32> {
        if self.bits_left == 0 {
            self.fill_bit_buffer(1)?;
        }

        self.bits_left -= 1;

        Ok(((self.get_buffer >> self.bits_left) & 1) as int32)
    }

    /// Extract the next coded symbol from the input stream (`HuffDecode`,
    /// Figure F.16).
    #[inline]
    fn huff_decode(&mut self, tbl_index: usize) -> DngResult<int32> {
        // If the Huffman code is less than 8 bits, use the fast table lookup.
        // It's more than 8 bits about 3-4% of the time.

        let code = self.show_bits8()?;

        let (numbits, value) = {
            let table = self.huff_tables[tbl_index].as_ref().unwrap();
            (table.numbits[code as usize], table.value[code as usize])
        };

        if numbits != 0 {
            self.flush_bits(numbits);

            return Ok(value);
        }

        // Long code: flush 8 bits and walk bit by bit.

        self.flush_bits(8);

        let mut l = 8usize;

        let mut code = code;

        loop {
            let maxcode = self.huff_tables[tbl_index].as_ref().unwrap().maxcode[l];

            if (code as int32) <= maxcode {
                break;
            }

            code = (code << 1) | self.get_bit()? as int32;
            l += 1;

            if l > 17 {
                break;
            }
        }

        // With garbage input we may reach the sentinel value l = 17.

        if l > 16 {
            Ok(0) // fake a zero as the safest result
        } else {
            let table = self.huff_tables[tbl_index].as_ref().unwrap();
            let idx = table.valptr[l] as int32 + (code as int32 - table.mincode[l] as int32);
            Ok(table.huffval[idx as usize] as int32)
        }
    }

    /// Figure F.12: extend sign bit (`HuffExtend`).
    #[inline]
    fn huff_extend(&self, mut x: int32, s: int32) -> int32 {
        let tt = (-1i32 << s) + 1;

        let tt = if x < (0x08000 >> (16 - s)) { tt } else { 0 };

        x += tt;

        x
    }

    // -- difference decoding variants ---------------------------------------------

    /// Generic difference decode (uses `fBug16` flag):
    /// `if (s == 16 && !fBug16) d = -32768; else { get_bits; HuffExtend; }`.
    #[inline]
    fn decode_diff_generic(&mut self, tbl_index: usize) -> DngResult<int32> {
        let s = self.huff_decode(tbl_index)?;

        let mut d = 0;

        if s != 0 {
            if s == 16 && !self.bug16 {
                d = -32768;
            } else {
                d = self.get_bits(s)?;
                d = self.huff_extend(d, s);
            }
        }

        Ok(d)
    }

    /// sRAW difference decode (ignores `fBug16`):
    /// `if (s == 16) d = -32768; else { get_bits; HuffExtend; }`.
    #[inline]
    fn decode_diff_sraw(&mut self, tbl_index: usize) -> DngResult<int32> {
        let s = self.huff_decode(tbl_index)?;

        let mut d = 0;

        if s != 0 {
            if s == 16 {
                d = -32768;
            } else {
                d = self.get_bits(s)?;
                d = self.huff_extend(d, s);
            }
        }

        Ok(d)
    }

    /// Hasselblad difference decode:
    /// `d = get_bits(s); if (s == 16) d = -32768; else HuffExtend(d, s);`
    /// (order differs from the sRAW variant, as in the C++).
    #[inline]
    fn decode_diff_hasselblad(&mut self, tbl_index: usize) -> DngResult<int32> {
        let s = self.huff_decode(tbl_index)?;

        let mut d = 0;

        if s != 0 {
            d = self.get_bits(s)?;

            if s == 16 {
                d = -32768;
            } else {
                d = self.huff_extend(d, s);
            }
        }

        Ok(d)
    }

    /// Called from `DecodeImage` to write one row (`PmPutRow`).
    fn pm_put_row(&mut self, row: &[uint16], num_comp: int32, num_col: int32) -> DngResult<()> {
        let pixels = (num_col as usize) * (num_comp as usize);

        self.spooler.spool(&row[..pixels]);

        Ok(())
    }

    /// Decode the first raster line of samples at the start of the scan and
    /// at the beginning of each restart interval (`DecodeFirstRow`).
    ///
    /// `dst_row` selects `mcu_row1` (0) or `mcu_row2` (1), mirroring the
    /// C++ pointer argument.
    fn decode_first_row(&mut self, dst_row: usize) -> DngResult<()> {
        let comps_in_scan = self.info.comps_in_scan as usize;

        // Process the first column in the row.

        for cur_comp in 0..comps_in_scan {
            let ci = self.info.mcu_membership[cur_comp] as usize;

            let compptr_idx = self.info.cur_comp_info[ci] as usize;
            let dctbl = self.comp_info[compptr_idx].dc_tbl_no as usize;

            // Section F.2.2.1: decode the difference

            let d = self.decode_diff_generic(dctbl)?;

            // Add the predictor to the difference.

            let pr = self.info.data_precision;
            let pt = self.info.pt;

            let idx = cur_comp; // col 0

            let value = (d + (1 << (pr - pt - 1))) as uint16;

            if dst_row == 0 {
                self.mcu_row1[idx] = value;
            } else {
                self.mcu_row2[idx] = value;
            }
        }

        // Process the rest of the row.

        let num_col = self.info.image_width as usize;

        for col in 1..num_col {
            for cur_comp in 0..comps_in_scan {
                let ci = self.info.mcu_membership[cur_comp] as usize;

                let compptr_idx = self.info.cur_comp_info[ci] as usize;
                let dctbl = self.comp_info[compptr_idx].dc_tbl_no as usize;

                // Section F.2.2.1: decode the difference

                let d = self.decode_diff_generic(dctbl)?;

                // Add the predictor to the difference.

                let idx = col * self.comps_in_scan_stride + cur_comp;
                let prev_idx = (col - 1) * self.comps_in_scan_stride + cur_comp;

                let value = match dst_row {
                    0 => (d + self.mcu_row1[prev_idx] as int32) as uint16,
                    _ => (d + self.mcu_row2[prev_idx] as int32) as uint16,
                };

                if dst_row == 0 {
                    self.mcu_row1[idx] = value;
                } else {
                    self.mcu_row2[idx] = value;
                }
            }
        }

        // Update the restart counter

        if self.info.restart_in_rows != 0 {
            self.info.restart_rows_to_go -= 1;
        }

        Ok(())
    }

    /// Decode the input stream, converting differences to real values
    /// (`DecodeImage`).
    fn decode_image(&mut self) -> DngResult<()> {
        let num_col = self.info.image_width as usize;
        let num_row = self.info.image_height as usize;
        let comps_in_scan = self.info.comps_in_scan as usize;

        // Precompute the decoding table index for each scan component.

        let mut ht = [0usize; 4];

        for cur_comp in 0..comps_in_scan {
            let ci = self.info.mcu_membership[cur_comp] as usize;
            let compptr_idx = self.info.cur_comp_info[ci] as usize;
            ht[cur_comp] = self.comp_info[compptr_idx].dc_tbl_no as usize;
        }

        // Canon sRAW support and Sony sRAW.

        if qSupportCanon_sRAW && qSupportSony_sRAW {
            let h2v1 = self.comp_info[0].h_samp_factor == 2
                && self.comp_info[0].v_samp_factor == 1;

            let h2v2 = self.comp_info[0].h_samp_factor == 2
                && self.comp_info[0].v_samp_factor == 2;

            if h2v1 {
                return self.decode_image_h2v1(ht, num_col, num_row, comps_in_scan);
            }

            if h2v2 {
                return self.decode_image_h2v2(ht, num_col, num_row, comps_in_scan);
            }
        }

        // Hasselblad 3FR support.

        if qSupportHasselblad_3FR && self.info.ss == 8 && (num_col & 1) == 0 {
            return self.decode_image_hasselblad(ht, num_col, num_row, comps_in_scan);
        }

        // Generic path.

        let stride = self.comps_in_scan_stride;

        // Decode the first row of image. Output the row and turn this row
        // into a previous row for later predictor calculation.

        self.decode_first_row(0)?;

        {
            let row = self.mcu_row1.clone();
            self.pm_put_row(&row, comps_in_scan as int32, num_col as int32)?;
        }

        // Process each row.

        for row in 1..num_row {
            // Account for restart interval, process restart marker if needed.

            if self.info.restart_in_rows != 0 {
                if self.info.restart_rows_to_go == 0 {
                    self.process_restart()?;

                    // Reset predictors at restart.

                    self.decode_first_row(1)?;

                    {
                        let row = self.mcu_row2.clone();
                        self.pm_put_row(&row, comps_in_scan as int32, num_col as int32)?;
                    }

                    std::mem::swap(&mut self.mcu_row1, &mut self.mcu_row2);

                    continue;
                }

                self.info.restart_rows_to_go -= 1;
            }

            // The upper neighbors are predictors for the first column.

            for cur_comp in 0..comps_in_scan {
                // Section F.2.2.1: decode the difference

                let d = self.decode_diff_generic(ht[cur_comp])?;

                // First column of row above is predictor for first column.

                let idx = cur_comp; // col 0

                let prev = self.mcu_row1[idx] as int32;

                self.mcu_row2[idx] = (d + prev) as uint16;
            }

            // For the rest of the columns on this row, predictor
            // calculations are based on PSV.

            if comps_in_scan == 2 && self.info.ss == 1 && num_col > 1 {
                // This is the combination used by both the Canon and Kodak
                // raw formats. Unrolling the general case logic results in a
                // significant speed increase.

                let mut prev0 = self.mcu_row2[self.row_index(0, 0)] as int32;
                let mut prev1 = self.mcu_row2[self.row_index(0, 1)] as int32;

                for col in 1..num_col {
                    // C++: decode difference once (HuffDecode + extend) and
                    // add to the running left value.

                    let d = self.decode_diff_generic(ht[0])?;

                    if d != 0 || true {
                        // decode_diff_generic returns the full difference
                        // value d (already extended); when s == 0 it is 0,
                        // so adding it unconditionally matches the C++.
                    }

                    prev0 += d;

                    let d = self.decode_diff_generic(ht[1])?;

                    prev1 += d;

                    let idx0 = self.row_index(col, 0);
                    let idx1 = self.row_index(col, 1);

                    self.mcu_row2[idx0] = prev0 as uint16;
                    self.mcu_row2[idx1] = prev1 as uint16;
                }
            } else {
                for col in 1..num_col {
                    for cur_comp in 0..comps_in_scan {
                        // Section F.2.2.1: decode the difference

                        let d = self.decode_diff_generic(ht[cur_comp])?;

                        // Predict the pixel value.

                        let stride = self.comps_in_scan_stride;

                        let diag = self.mcu_row1[(col - 1) * stride + cur_comp] as int32;
                        let upper = self.mcu_row1[col * stride + cur_comp] as int32;
                        let left = self.mcu_row2[(col - 1) * stride + cur_comp] as int32;

                        let predictor = match self.info.ss {
                            0 => 0,
                            1 => left,
                            2 => upper,
                            3 => diag,
                            4 => left + upper - diag,
                            5 => left + ((upper - diag) >> 1),
                            6 => upper + ((left - diag) >> 1),
                            7 => (left + upper) >> 1,
                            _ => return Err(bad_format()),
                        };

                        // Save the difference.

                        let idx = col * stride + cur_comp;

                        self.mcu_row2[idx] = (d + predictor) as uint16;
                    }
                }
            }

            {
                let row = self.mcu_row2.clone();
                self.pm_put_row(&row, comps_in_scan as int32, num_col as int32)?;
            }

            std::mem::swap(&mut self.mcu_row1, &mut self.mcu_row2);
        }

        Ok(())
    }

    /// C++ `DecodeImage` first sRAW branch: `hSampFactor == 2 &&
    /// vSampFactor == 1` (Canon sRAW / sRAW2 and Sony sRAW2).
    fn decode_image_h2v1(
        &mut self,
        ht: [usize; 4],
        num_col: usize,
        num_row: usize,
        comps_in_scan: usize,
    ) -> DngResult<()> {
        let stride = self.comps_in_scan_stride;

        let sony_sraw2 = qSupportSony_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 1
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 16
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0
            && (self.info.image_height & 1) == 0;

        let mut row_a = vec![0u16; num_col * stride];
        let mut row_b = vec![0u16; num_col * stride];

        let mut cur_is_a = true;

        for row in 0..num_row {
            // Initialize predictors.

            let (mut p0, mut p1, mut p2);

            let prev_row: &Vec<uint16> = if cur_is_a { &row_b } else { &row_a };

            if row == 0 {
                if sony_sraw2 {
                    p0 = 1 << 15;
                } else {
                    // Canon.
                    p0 = 1 << 14;
                }

                p1 = 1 << 14;
                p2 = 1 << 14;
            } else {
                p0 = prev_row[0] as int32;
                p1 = prev_row[1] as int32;
                p2 = prev_row[2] as int32;
            }

            let mut col = 0usize;
            while col < num_col {
                // Read first luminance component.

                {
                    let d = self.decode_diff_sraw(ht[0])?;

                    p0 += d;

                    let cur: &mut Vec<uint16> = if cur_is_a { &mut row_a } else { &mut row_b };
                    cur[col * stride] = p0 as uint16;
                }

                // Read second luminance component.

                {
                    let d = self.decode_diff_sraw(ht[0])?;

                    p0 += d;

                    let cur: &mut Vec<uint16> = if cur_is_a { &mut row_a } else { &mut row_b };
                    cur[(col + 1) * stride] = p0 as uint16;
                }

                // Read first chroma component.

                {
                    let d = self.decode_diff_sraw(ht[1])?;

                    p1 += d;

                    let cur: &mut Vec<uint16> = if cur_is_a { &mut row_a } else { &mut row_b };
                    cur[col * stride + 1] = p1 as uint16;
                    cur[(col + 1) * stride + 1] = p1 as uint16;
                }

                // Read second chroma component.

                {
                    let d = self.decode_diff_sraw(ht[2])?;

                    p2 += d;

                    let cur: &mut Vec<uint16> = if cur_is_a { &mut row_a } else { &mut row_b };
                    cur[col * stride + 2] = p2 as uint16;
                    cur[(col + 1) * stride + 2] = p2 as uint16;
                }

                col += 2;
            }

            let cur_snapshot = if cur_is_a { row_a.clone() } else { row_b.clone() };

            self.pm_put_row(&cur_snapshot, comps_in_scan as int32, num_col as int32)?;

            cur_is_a = !cur_is_a;
        }

        Ok(())
    }

    /// C++ `DecodeImage` second sRAW branch: `hSampFactor == 2 &&
    /// vSampFactor == 2` (Sony sRAW and Canon sRAW 2x2).
    fn decode_image_h2v2(
        &mut self,
        ht: [usize; 4],
        num_col: usize,
        num_row: usize,
        comps_in_scan: usize,
    ) -> DngResult<()> {
        let stride = self.comps_in_scan_stride;

        let sony_sraw = qSupportSony_sRAW
            && self.info.num_components == 3
            && self.comp_info[0].h_samp_factor == 2
            && self.comp_info[1].h_samp_factor == 1
            && self.comp_info[2].h_samp_factor == 1
            && self.comp_info[0].v_samp_factor == 2
            && self.comp_info[1].v_samp_factor == 1
            && self.comp_info[2].v_samp_factor == 1
            && self.info.data_precision == 16
            && self.info.ss == 1
            && (self.info.image_width & 1) == 0
            && (self.info.image_height & 1) == 0;

        let mut row_a = vec![0u16; num_col * stride];
        let mut row_b = vec![0u16; num_col * stride];

        let mut prev_is_a = true;

        let mut row = 0usize;
        while row < num_row {
            if sony_sraw {
                // Sony sRaw.

                // Initialize predictors.

                let (mut p00, mut p01, mut p10, mut p11);
                let (mut p1, mut p2);

                let prev_row: &Vec<uint16> = if prev_is_a { &row_a } else { &row_b };

                if row == 0 {
                    p00 = 1 << 15;
                    p1 = 1 << 14;
                    p2 = 1 << 14;
                } else {
                    p00 = prev_row[0] as int32;
                    p1 = prev_row[1] as int32;
                    p2 = prev_row[2] as int32;
                }

                p01 = 0;
                p10 = 0;
                p11 = 0;

                let mut col = 0usize;
                while col < num_col {
                    // Read first luminance component.
                    {
                        let d = self.decode_diff_sraw(ht[0])?;

                        p00 = d + if col == 0 { p00 } else { p01 };

                        row_a[col * stride] = p00 as uint16;
                    }

                    // Read second luminance component.
                    {
                        let d = self.decode_diff_sraw(ht[0])?;

                        p01 = p00 + d;

                        row_a[(col + 1) * stride] = p01 as uint16;
                    }

                    // Read third luminance component.
                    {
                        let d = self.decode_diff_sraw(ht[0])?;

                        p10 = d + if col == 0 { p00 } else { p11 };

                        row_b[col * stride] = p10 as uint16;
                    }

                    // Read fourth luminance component.
                    {
                        let d = self.decode_diff_sraw(ht[0])?;

                        p11 = p10 + d;

                        row_b[(col + 1) * stride] = p11 as uint16;
                    }

                    // Read first chroma component.
                    {
                        let d = self.decode_diff_sraw(ht[1])?;

                        p1 += d;

                        row_a[col * stride + 1] = p1 as uint16;
                        row_a[(col + 1) * stride + 1] = p1 as uint16;
                        row_b[col * stride + 1] = p1 as uint16;
                        row_b[(col + 1) * stride + 1] = p1 as uint16;
                    }

                    // Read second chroma component.
                    {
                        let d = self.decode_diff_sraw(ht[2])?;

                        p2 += d;

                        row_a[col * stride + 2] = p2 as uint16;
                        row_a[(col + 1) * stride + 2] = p2 as uint16;
                        row_b[col * stride + 2] = p2 as uint16;
                        row_b[(col + 1) * stride + 2] = p2 as uint16;
                    }

                    col += 2;
                }

                let a = row_a.clone();
                let b = row_b.clone();

                self.pm_put_row(&a, comps_in_scan as int32, num_col as int32)?;
                self.pm_put_row(&b, comps_in_scan as int32, num_col as int32)?;
            } else {
                // Canon sRaw.

                // Initialize predictors.

                let (mut p0, mut p1, mut p2);

                let prev_row: &Vec<uint16> = if prev_is_a { &row_a } else { &row_b };

                if row == 0 {
                    p0 = 1 << 14;
                    p1 = 1 << 14;
                    p2 = 1 << 14;
                } else {
                    p0 = prev_row[0] as int32;
                    p1 = prev_row[1] as int32;
                    p2 = prev_row[2] as int32;
                }

                let mut col = 0usize;
                while col < num_col {
                    // Read four luminance components: prevRow[col],
                    // prevRow[col+1], curRow[col], curRow[col+1].

                    for k in 0..4usize {
                        let d = self.decode_diff_sraw(ht[0])?;

                        p0 += d;

                        let (dst, c) = match k {
                            0 => (&mut row_a, col),
                            1 => (&mut row_a, col + 1),
                            2 => (&mut row_b, col),
                            _ => (&mut row_b, col + 1),
                        };

                        dst[c * stride] = p0 as uint16;
                    }

                    // Read first chroma component.
                    {
                        let d = self.decode_diff_sraw(ht[1])?;

                        p1 += d;

                        row_a[col * stride + 1] = p1 as uint16;
                        row_a[(col + 1) * stride + 1] = p1 as uint16;
                        row_b[col * stride + 1] = p1 as uint16;
                        row_b[(col + 1) * stride + 1] = p1 as uint16;
                    }

                    // Read second chroma component.
                    {
                        let d = self.decode_diff_sraw(ht[2])?;

                        p2 += d;

                        row_a[col * stride + 2] = p2 as uint16;
                        row_a[(col + 1) * stride + 2] = p2 as uint16;
                        row_b[col * stride + 2] = p2 as uint16;
                        row_b[(col + 1) * stride + 2] = p2 as uint16;
                    }

                    col += 2;
                }

                let a = row_a.clone();
                let b = row_b.clone();

                self.pm_put_row(&a, comps_in_scan as int32, num_col as int32)?;
                self.pm_put_row(&b, comps_in_scan as int32, num_col as int32)?;
            }

            prev_is_a = !prev_is_a;
            row += 2;
        }

        Ok(())
    }

    /// C++ `DecodeImage` Hasselblad 3FR branch (`Ss == 8`, even width).
    fn decode_image_hasselblad(
        &mut self,
        ht: [usize; 4],
        num_col: usize,
        num_row: usize,
        comps_in_scan: usize,
    ) -> DngResult<()> {
        self.hasselblad_3fr = true;

        let stride = self.comps_in_scan_stride;

        let mut cur_row = vec![0u16; num_col * stride];

        for _row in 0..num_row {
            let mut p0 = 32768;
            let mut p1 = 32768;

            let mut col = 0usize;
            while col < num_col {
                let s0 = self.huff_decode(ht[0])?;
                let s1 = self.huff_decode(ht[0])?;

                if s0 != 0 {
                    let mut d = self.get_bits(s0)?;
                    if s0 == 16 {
                        d = -32768;
                    } else {
                        d = self.huff_extend(d, s0);
                    }
                    p0 += d;
                }

                if s1 != 0 {
                    let mut d = self.get_bits(s1)?;
                    if s1 == 16 {
                        d = -32768;
                    } else {
                        d = self.huff_extend(d, s1);
                    }
                    p1 += d;
                }

                cur_row[col * stride] = p0 as uint16;
                cur_row[(col + 1) * stride] = p1 as uint16;

                col += 2;
            }

            let cur = cur_row.clone();
            self.pm_put_row(&cur, comps_in_scan as int32, num_col as int32)?;
        }

        Ok(())
    }

    // -- public driver methods ----------------------------------------------------

    /// `StartRead` — reads the file header and scan header and initializes
    /// the decoder; returns `(imageWidth, imageHeight, imageChannels)`.
    pub fn start_read(&mut self) -> DngResult<(uint32, uint32, uint32)> {
        self.read_file_header()?;
        self.read_scan_header()?;
        self.decoder_struct_init()?;
        self.huff_decoder_init()?;

        Ok((
            self.info.image_width as uint32,
            self.info.image_height as uint32,
            self.info.comps_in_scan as uint32,
        ))
    }

    /// `FinishRead` — decode the image data.
    pub fn finish_read(&mut self) -> DngResult<()> {
        self.decode_image()
    }
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// Port of `dng_lossless_encoder<simd>` (Scalar specialization).
pub struct LosslessEncoder<'s, 'a> {
    /// Source samples (`fSrcData`); addressed as
    /// `src_data[(row * row_step + col * col_step + channel) as usize]`.
    src_data: &'s [uint16],

    src_rows: uint32,
    src_cols: uint32,
    src_channels: uint32,
    src_bit_depth: uint32,

    src_row_step: int32,
    src_col_step: int32,

    stream: &'s mut DngStream<'a>,

    huff_table: [HuffmanTable; 4],

    freq_count: [[uint32; 257]; 4],

    // Current bit-accumulation buffer.
    huff_put_buffer: uint64,
    huff_put_bits: int32,

    /// Lookup table for number of bits in an 8 bit value.
    num_bits_table: [int32; 256],

    /// C++ `streamBuffer` staging area.
    stream_buffer: Vec<uint8>,
}

impl<'s, 'a> LosslessEncoder<'s, 'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        src_data: &'s [uint16],
        src_rows: uint32,
        src_cols: uint32,
        src_channels: uint32,
        src_bit_depth: uint32,
        src_row_step: int32,
        src_col_step: int32,
        stream: &'s mut DngStream<'a>,
    ) -> Self {
        // Initialize number of bits lookup table.

        let mut num_bits_table = [0 as int32; 256];

        for i in 1..256usize {
            let mut temp = i;
            let mut nbits = 1;

            loop {
                temp >>= 1;
                if temp == 0 {
                    break;
                }
                nbits += 1;
            }

            num_bits_table[i] = nbits;
        }

        // Get an upper bound of the size of encoded output for one chunk of
        // output. Chunks are the headers and then one per row.

        let stream_buffer_extent = ((src_cols as usize
            * src_channels as usize
            * ((src_bit_depth + 7) / 8) as usize
            * 2
            * 2)
            + 1)
        .max(src_channels as usize * 296 + 64);

        LosslessEncoder {
            src_data,
            src_rows,
            src_cols,
            src_channels,
            src_bit_depth,
            src_row_step,
            src_col_step,
            stream,
            huff_table: [HuffmanTable::new(), HuffmanTable::new(), HuffmanTable::new(), HuffmanTable::new()],
            freq_count: [[0; 257]; 4],
            huff_put_buffer: 0,
            huff_put_bits: 0,
            num_bits_table,
            stream_buffer: Vec::with_capacity(stream_buffer_extent),
        }
    }

    /// Sample accessor matching the C++ pointer walk
    /// `sPtr = fSrcData + row * fSrcRowStep; ... sPtr += fSrcColStep`.
    #[inline]
    fn sample(&self, row: int32, col: int32, channel: int32) -> DngResult<uint16> {
        let index = row as int64 * self.src_row_step as int64
            + col as int64 * self.src_col_step as int64
            + channel as int64;

        if index < 0 || index >= self.src_data.len() as int64 {
            return Err(dng_core::dng_exceptions::bad_format());
        }

        Ok(self.src_data[index as usize])
    }

    #[inline]
    fn emit_byte(&mut self, value: uint8) {
        self.stream_buffer.push(value);
    }

    /// Force bits in the internal shift register out to the I/O buffer
    /// (`PushBits`).
    fn push_bits(&mut self) {
        while self.huff_put_bits >= 8 {
            let c = (self.huff_put_buffer >> (self.huff_put_bits - 8)) as uint8;
            self.emit_byte(c);
            if c == 0xff {
                self.emit_byte(0x00);
            }
            self.huff_put_bits -= 8;
        }
    }

    /// Code for outputting bits to the file (`EmitBits`).
    #[inline]
    fn emit_bits(&mut self, code: int32, size: int32) -> DngResult<()> {
        debug_assert!(size != 0, "Bad Huffman table entry");

        if self.huff_put_bits + size > 64 {
            self.push_bits();
        }

        self.huff_put_buffer <<= size;
        self.huff_put_buffer |= code as uint64;
        self.huff_put_bits += size;

        Ok(())
    }

    /// Flush any remaining bits in the bit buffer; used before emitting a
    /// marker (`FlushBits`).
    fn flush_bits(&mut self) -> DngResult<()> {
        // The first call forces output of any partial bytes.

        self.emit_bits(0x007F, 7)?;

        self.push_bits();

        // We can then zero the buffer.

        self.huff_put_buffer = 0;
        self.huff_put_bits = 0;

        Ok(())
    }

    /// Flush internal buffer to file stream (`FlushBuffer`).
    fn flush_buffer(&mut self) -> DngResult<()> {
        let data = std::mem::take(&mut self.stream_buffer);
        self.stream.put(&data)?;
        Ok(())
    }

    /// Count the difference value in countTable (`CountOneDiff`).
    #[inline]
    fn count_one_diff(num_bits_table: &[int32; 256], diff: int32, count_table: &mut [uint32; 257]) {
        // Encode the DC coefficient difference per section F.1.2.1

        let mut temp = diff;

        if temp < 0 {
            temp = -temp;
        }

        // Find the number of bits needed for the magnitude of the coefficient

        let nbits = if temp >= 256 {
            num_bits_table[(temp >> 8) as usize] + 8
        } else {
            num_bits_table[(temp & 0xFF) as usize]
        };

        // Update count for this bit length

        count_table[nbits as usize] += 1;
    }

    /// Encode a single difference value (`EncodeOneDiff`).
    #[inline]
    fn encode_one_diff(&mut self, diff: int32, dctbl: usize) -> DngResult<()> {
        // Encode the DC coefficient difference per section F.1.2.1

        let mut temp = diff;
        let mut temp2 = diff;

        if temp < 0 {
            temp = -temp;

            // For a negative input, want temp2 = bitwise complement of
            // abs (input).

            temp2 -= 1;
        }

        // Find the number of bits needed for the magnitude of the coefficient

        let nbits = if temp >= 256 {
            self.num_bits_table[(temp >> 8) as usize] + 8
        } else {
            self.num_bits_table[(temp & 0xFF) as usize]
        };

        // Emit the Huffman-coded symbol for the number of bits

        let (co, si) = {
            let t = &self.huff_table[dctbl];
            (t.ehufco[nbits as usize] as int32, t.ehufsi[nbits as usize] as int32)
        };

        self.emit_bits(co, si)?;

        // Emit that number of bits of the value, if positive, or the
        // complement of its magnitude, if negative.
        //
        // If the number of bits is 16, there is only one possible difference
        // value (-32768), so the lossless JPEG spec says not to output
        // anything in that case.

        if (nbits & 15) != 0 {
            self.emit_bits((temp2 & (0x0FFFF >> (16 - nbits))) as int32, nbits)?;
        }

        Ok(())
    }

    /// Buffered variant (`EmitBitsToBuffer`).
    #[inline]
    fn emit_bits_to_buffer(&mut self, mut buffered_bits: int32, bit_buffer: uint64) -> int32 {
        debug_assert!(buffered_bits < 64, "buffered_bits too big(3)");

        while buffered_bits >= 8 {
            let c = (bit_buffer >> (buffered_bits - 8)) as uint8;
            self.stream_buffer.push(c);
            if c == 0xff {
                self.stream_buffer.push(0x00);
            }
            buffered_bits -= 8;
        }

        buffered_bits
    }

    /// Buffered variant (`EncodeOneDiffToBuffer`).
    #[inline]
    fn encode_one_diff_to_buffer(
        &mut self,
        diff: int32,
        dctbl: usize,
        mut buffered_bits: int32,
        mut bit_buffer: uint64,
    ) -> DngResult<(int32, uint64)> {
        debug_assert!(buffered_bits < 64, "buffered_bits too big(1)");

        if buffered_bits > 32 {
            buffered_bits = self.emit_bits_to_buffer(buffered_bits, bit_buffer);
        }

        // Encode the DC coefficient difference per section F.1.2.1

        let mut temp = diff;
        let mut temp2 = diff;

        if temp < 0 {
            temp = -temp;
            temp2 -= 1;
        }

        // Find the number of bits needed for the magnitude of the coefficient

        let nbits = if temp >= 256 {
            self.num_bits_table[(temp >> 8) as usize] + 8
        } else {
            self.num_bits_table[(temp & 0xFF) as usize]
        };

        // Emit the Huffman-coded symbol for the number of bits

        let (co, si) = {
            let t = &self.huff_table[dctbl];
            (t.ehufco[nbits as usize] as uint64, t.ehufsi[nbits as usize] as int32)
        };

        bit_buffer <<= si;
        bit_buffer |= co;
        buffered_bits += si;

        // Emit that number of bits of the value, if positive, or the
        // complement of its magnitude, if negative.

        if (nbits & 15) != 0 {
            bit_buffer <<= nbits;
            bit_buffer |= (temp2 & (0x0FFFF >> (16 - nbits))) as uint64;
            buffered_bits += nbits;
        }

        debug_assert!(buffered_bits < 64, "buffered_bits too big(2)");

        Ok((buffered_bits, bit_buffer))
    }

    /// Count the times each category symbol occurs in this image
    /// (`FreqCountSet`).
    fn freq_count_set(&mut self) -> DngResult<()> {
        self.freq_count = [[0; 257]; 4];

        for row in 0..self.src_rows as int32 {
            // Initialize predictors for this row.

            let mut predictor = [0 as int32; 4];

            for channel in 0..self.src_channels as int32 {
                if row == 0 {
                    predictor[channel as usize] = 1 << (self.src_bit_depth - 1);
                } else {
                    // C++: predictor[channel] = sPtr[channel - fSrcRowStep]
                    // (previous row, column 0, same channel).
                    predictor[channel as usize] =
                        self.sample(row, 0, channel - self.src_row_step)? as int32;
                }
            }

            // Unroll most common case of two channels

            if self.src_channels == 2 {
                let mut pred0 = predictor[0];
                let mut pred1 = predictor[1];

                for col in 0..self.src_cols as int32 {
                    let pixel0 = self.sample(row, col, 0)? as int32;
                    let pixel1 = self.sample(row, col, 1)? as int32;

                    let diff0 = (pixel0 - pred0) as int16;
                    let diff1 = (pixel1 - pred1) as int16;

                    Self::count_one_diff(&self.num_bits_table, diff0 as int32, &mut self.freq_count[0]);
                    Self::count_one_diff(&self.num_bits_table, diff1 as int32, &mut self.freq_count[1]);

                    pred0 = pixel0;
                    pred1 = pixel1;
                }
            }
            // General case.
            else {
                for col in 0..self.src_cols as int32 {
                    for channel in 0..self.src_channels as int32 {
                        let pixel = self.sample(row, col, channel)? as int32;

                        let diff = (pixel - predictor[channel as usize]) as int16;

                        let ch = channel as usize;
                        Self::count_one_diff(&self.num_bits_table, diff as int32, &mut self.freq_count[ch]);

                        predictor[channel as usize] = pixel;
                    }
                }
            }
        }

        Ok(())
    }

    /// Encode and output Huffman-compressed image data (`HuffEncode`).
    fn huff_encode(&mut self) -> DngResult<()> {
        let mut bit_buffer: uint64 = 0;
        let mut buffered_bits: int32 = 0;

        if self.src_channels == 2 {
            bit_buffer = self.huff_put_buffer;
            buffered_bits = self.huff_put_bits;
        }

        for row in 0..self.src_rows as int32 {
            // Initialize predictors for this row.

            let mut predictor = [0 as int32; 4];

            for channel in 0..self.src_channels as int32 {
                if row == 0 {
                    predictor[channel as usize] = 1 << (self.src_bit_depth - 1);
                } else {
                    // C++: predictor[channel] = sPtr[channel - fSrcRowStep]
                    // (previous row, column 0, same channel).
                    predictor[channel as usize] =
                        self.sample(row, 0, channel - self.src_row_step)? as int32;
                }
            }

            // Unroll most common case of two channels

            if self.src_channels == 2 {
                let mut pred0 = predictor[0];
                let mut pred1 = predictor[1];

                for col in 0..self.src_cols as int32 {
                    let pixel0 = self.sample(row, col, 0)? as int32;
                    let pixel1 = self.sample(row, col, 1)? as int32;

                    let diff0 = (pixel0 - pred0) as int16;
                    let diff1 = (pixel1 - pred1) as int16;

                    let (b, buf) = self.encode_one_diff_to_buffer(
                        diff0 as int32,
                        0,
                        buffered_bits,
                        bit_buffer,
                    )?;
                    buffered_bits = b;
                    bit_buffer = buf;

                    let (b, buf) = self.encode_one_diff_to_buffer(
                        diff1 as int32,
                        1,
                        buffered_bits,
                        bit_buffer,
                    )?;
                    buffered_bits = b;
                    bit_buffer = buf;

                    pred0 = pixel0;
                    pred1 = pixel1;
                }

                buffered_bits = self.emit_bits_to_buffer(buffered_bits, bit_buffer);
            }
            // General case.
            else {
                for col in 0..self.src_cols as int32 {
                    for channel in 0..self.src_channels as int32 {
                        let pixel = self.sample(row, col, channel)? as int32;

                        let diff = (pixel - predictor[channel as usize]) as int16;

                        self.encode_one_diff(diff as int32, channel as usize)?;

                        predictor[channel as usize] = pixel;
                    }
                }
            }

            self.flush_buffer()?;
        }

        if self.src_channels == 2 {
            self.huff_put_buffer = bit_buffer;
            self.huff_put_bits = buffered_bits;
        }

        self.flush_bits()?;

        Ok(())
    }

    /// Generate the optimal coding for the given counts (JPEG section K.2)
    /// (`GenHuffCoding`).
    fn gen_huff_coding(htbl: &mut HuffmanTable, freq: &mut [uint32; 257]) -> DngResult<()> {
        const MAX_CLEN: usize = 32; // assumed maximum initial code length

        let mut bits = [0u8; MAX_CLEN + 1]; // bits[k] = # of symbols with code length k
        let mut codesize = [0i16; 257]; // codesize[k] = code length of symbol k
        let mut others = [-1i16; 257]; // next symbol in current branch of tree

        // Including the pseudo-symbol 256 in the Huffman procedure guarantees
        // that no real symbol is given code-value of all ones, because 256
        // will be placed in the largest codeword category.

        freq[256] = 1; // make sure there is a nonzero count

        // Huffman's basic algorithm to assign optimal code lengths to symbols

        loop {
            // Find the smallest nonzero frequency, set c1 = its symbol.
            // In case of ties, take the larger symbol number.

            let mut c1: i32 = -1;

            let mut v: uint32 = 0xFFFFFFFF;

            for i in 0..=256usize {
                if freq[i] != 0 && freq[i] <= v {
                    v = freq[i];
                    c1 = i as i32;
                }
            }

            // Find the next smallest nonzero frequency, set c2 = its symbol.
            // In case of ties, take the larger symbol number.

            let mut c2: i32 = -1;

            v = 0xFFFFFFFF;

            for i in 0..=256usize {
                if freq[i] != 0 && freq[i] <= v && i as i32 != c1 {
                    v = freq[i];
                    c2 = i as i32;
                }
            }

            // Done if we've merged everything into one frequency.

            if c2 < 0 {
                break;
            }

            // Else merge the two counts/trees.

            freq[c1 as usize] += freq[c2 as usize];
            freq[c2 as usize] = 0;

            // Increment the codesize of everything in c1's tree branch.

            codesize[c1 as usize] += 1;

            let mut j = c1;
            while others[j as usize] >= 0 {
                j = others[j as usize] as i32;
                codesize[j as usize] += 1;
            }

            // chain c2 onto c1's tree branch

            others[j as usize] = c2 as int16;

            // Increment the codesize of everything in c2's tree branch.

            codesize[c2 as usize] += 1;

            let mut j = c2;
            while others[j as usize] >= 0 {
                j = others[j as usize] as i32;
                codesize[j as usize] += 1;
            }
        }

        // Now count the number of symbols of each code length.

        for i in 0..=256usize {
            if codesize[i] != 0 {
                // The JPEG standard seems to think that this can't happen,
                // but I'm paranoid...

                if codesize[i] as i32 > MAX_CLEN as i32 {
                    return Err(dng_core::dng_exceptions::overflow_error());
                }

                bits[codesize[i] as usize] += 1;
            }
        }

        // JPEG doesn't allow symbols with code lengths over 16 bits, so if
        // the pure Huffman procedure assigned any such lengths, the C++
        // reports "Info: Optimal huffman table bigger than 16 bits" and
        // throws a program error, which HuffOptimize catches to fall back to
        // a default table.

        for i in (17..=MAX_CLEN).rev() {
            if bits[i] > 0 {
                return Err(dng_core::dng_exceptions::program_error());
            }
        }

        // Remove the count for the pseudo-symbol 256 from the largest
        // codelength.

        let mut i = MAX_CLEN as i32;

        while bits[i as usize] == 0 {
            i -= 1;
        }

        bits[i as usize] -= 1;

        // Return final symbol counts (only for lengths 0..16).

        htbl.bits.copy_from_slice(&bits[0..17]);

        // Return a list of the symbols sorted by code length.

        let mut p = 0usize;

        for i in 1..=MAX_CLEN {
            for j in 0..=255usize {
                if codesize[j] as usize == i {
                    htbl.huffval[p] = j as uint8;
                    p += 1;
                }
            }
        }

        Ok(())
    }

    /// Find the best coding parameters for a Huffman-coded scan
    /// (`HuffOptimize`).
    fn huff_optimize(&mut self) -> DngResult<()> {
        // Collect the frequency counts.

        self.freq_count_set()?;

        // Generate Huffman encoding tables.

        for channel in 0..self.src_channels as usize {
            let mut table = HuffmanTable::new();

            let mut freq = self.freq_count[channel];

            if Self::gen_huff_coding(&mut table, &mut freq).is_err() {
                // C++: DNG_REPORT ("Info: Reverting to default huffman table");

                for (j, f) in freq.iter_mut().enumerate() {
                    *f = if j <= 16 { 1 } else { 0 };
                }

                Self::gen_huff_coding(&mut table, &mut freq).map_err(|_| DngError::Unknown)?;
            }

            self.freq_count[channel] = freq;
            self.huff_table[channel] = table;

            fix_huff_tbl(&mut self.huff_table[channel])?;
        }

        Ok(())
    }

    /// Emit a marker code into the output stream (`EmitMarker`).
    fn emit_marker(&mut self, mark: uint32) {
        self.emit_byte(0xFF);
        self.emit_byte(mark as uint8);
    }

    /// Emit a 2-byte integer; these are always MSB first in JPEG files
    /// (`Emit2bytes`).
    fn emit2bytes(&mut self, value: int32) {
        self.emit_byte(((value >> 8) & 0xFF) as uint8);
        self.emit_byte((value & 0xFF) as uint8);
    }

    /// Emit a DHT marker, followed by the Huffman data (`EmitDht`).
    fn emit_dht(&mut self, index: usize) {
        // PORT-NOTE: copy the table fields up front to satisfy the borrow
        // checker (the C++ interleaves reads from the table with writes to
        // the output buffer).

        let bits = self.huff_table[index].bits;
        let huffval = self.huff_table[index].huffval;

        self.emit_marker(M_DHT);

        let mut length = 0u32;

        for i in 1..=16usize {
            length += bits[i] as u32;
        }

        self.emit2bytes((length + 2 + 1 + 16) as int32);

        self.emit_byte(index as uint8);

        for i in 1..=16usize {
            self.emit_byte(bits[i]);
        }

        for i in 0..length as usize {
            self.emit_byte(huffval[i]);
        }
    }

    /// Emit a SOF marker plus data (`EmitSof`).
    fn emit_sof(&mut self, code: uint32) {
        self.emit_marker(code);

        self.emit2bytes(3 * self.src_channels as int32 + 2 + 5 + 1); // length

        self.emit_byte(self.src_bit_depth as uint8);

        self.emit2bytes(self.src_rows as int32);
        self.emit2bytes(self.src_cols as int32);

        self.emit_byte(self.src_channels as uint8);

        for i in 0..self.src_channels {
            self.emit_byte(i as uint8);

            self.emit_byte((1 << 4) + 1); // Not subsampled.

            self.emit_byte(0); // Tq shall be 0 for lossless.
        }
    }

    /// Emit a SOS marker plus data (`EmitSos`).
    fn emit_sos(&mut self) {
        self.emit_marker(M_SOS);

        self.emit2bytes(2 * self.src_channels as int32 + 2 + 1 + 3); // length

        self.emit_byte(self.src_channels as uint8); // Ns

        for i in 0..self.src_channels {
            // Cs,Td,Ta

            self.emit_byte(i as uint8);
            self.emit_byte((i << 4) as uint8);
        }

        self.emit_byte(1); // PSV - hardcoded - tknoll
        self.emit_byte(0); // Spectral selection end - Se
        self.emit_byte(0); // The point transform parameter
    }

    /// Write the file header (`WriteFileHeader`).
    fn write_file_header(&mut self) {
        self.emit_marker(M_SOI); // first the SOI

        self.emit_sof(M_SOF3);
    }

    /// Write the start of a scan, everything through the SOS marker
    /// (`WriteScanHeader`).
    fn write_scan_header(&mut self) {
        // Emit Huffman tables.

        for i in 0..self.src_channels as usize {
            self.emit_dht(i);
        }

        self.emit_sos();
    }

    /// Write the End of image marker at the end of a JPEG file
    /// (`WriteFileTrailer`).
    fn write_file_trailer(&mut self) {
        self.emit_marker(M_EOI);
    }

    /// `Encode ()`.
    pub fn encode(&mut self) -> DngResult<()> {
        debug_assert!(self.src_channels <= 4, "Too many components in scan");

        // Count the times each difference category occurs.
        // Construct the optimal Huffman table.

        self.huff_optimize()?;

        // Write the frame and scan headers.

        self.write_file_header();

        self.write_scan_header();

        self.flush_buffer()?;

        // Encode the image.

        self.huff_encode()?;

        self.flush_buffer()?;

        // Clean up everything.

        self.write_file_trailer();

        self.flush_buffer()?;

        Ok(())
    }
}



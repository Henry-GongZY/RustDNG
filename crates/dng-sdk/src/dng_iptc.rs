// Port of dng_iptc.h/.cpp — IPTC IIM metadata: record parse from a block and
// IIM binary spooling (always UTF-8 with the encoding marker, per the
// metadata working group change the C++ describes).

use crate::dng_core::{
    DngDateTimeInfo, DngMemoryBlock, DngMemoryStream, DngStream, DngString, DngStringList,
};

/// IPTC record 2 data-set codes (`DataSet` enum) and the charset marker
/// (`CharSet` enum).
pub mod data_set {
    pub const kRecordVersionSet: u8 = 0;
    pub const kObjectNameSet: u8 = 5;
    pub const kUrgencySet: u8 = 10;
    pub const kCategorySet: u8 = 15;
    pub const kSupplementalCategoriesSet: u8 = 20;
    pub const kKeywordsSet: u8 = 25;
    pub const kSpecialInstructionsSet: u8 = 40;
    pub const kDateCreatedSet: u8 = 55;
    pub const kTimeCreatedSet: u8 = 60;
    pub const kDigitalCreationDateSet: u8 = 62;
    pub const kDigitalCreationTimeSet: u8 = 63;
    pub const kBylineSet: u8 = 80;
    pub const kBylineTitleSet: u8 = 85;
    pub const kCitySet: u8 = 90;
    pub const kSublocationSet: u8 = 92;
    pub const kProvinceStateSet: u8 = 95;
    pub const kCountryCodeSet: u8 = 100;
    pub const kCountryNameSet: u8 = 101;
    pub const kOriginalTransmissionReferenceSet: u8 = 103;
    pub const kHeadlineSet: u8 = 105;
    pub const kCreditSet: u8 = 110;
    pub const kSourceSet: u8 = 115;
    pub const kCopyrightNoticeSet: u8 = 116;
    pub const kCaptionSet: u8 = 120;
    pub const kCaptionWriterSet: u8 = 122;

    pub const kCharSetUnknown: u32 = 0;
    pub const kCharSetUTF8: u32 = 1;
}

use data_set::*;

/// Port of `dng_iptc`.
#[derive(Debug, Clone, Default)]
pub struct DngIptc {
    pub title: DngString,
    /// -1 == not set.
    pub urgency: i32,
    pub category: DngString,
    pub supplemental_categories: DngStringList,
    pub keywords: DngStringList,
    pub instructions: DngString,
    pub date_time_created: DngDateTimeInfo,
    pub digital_creation_date_time: DngDateTimeInfo,
    pub authors: DngStringList,
    pub authors_position: DngString,
    pub city: DngString,
    pub state: DngString,
    pub country: DngString,
    pub country_code: DngString,
    pub location: DngString,
    pub transmission_reference: DngString,
    pub headline: DngString,
    pub credit: DngString,
    pub source: DngString,
    pub copyright_notice: DngString,
    pub description: DngString,
    pub description_writer: DngString,
}

impl DngIptc {
    /// `dng_iptc ()` — urgency defaults to -1.
    pub fn new() -> Self {
        DngIptc {
            urgency: -1,
            ..Default::default()
        }
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        if self.title.not_empty() {
            return false;
        }
        if self.urgency >= 0 {
            return false;
        }
        if self.category.not_empty() {
            return false;
        }
        if self.supplemental_categories.count() > 0 {
            return false;
        }
        if self.keywords.count() > 0 {
            return false;
        }
        if self.instructions.not_empty() {
            return false;
        }
        if self.date_time_created.is_valid() {
            return false;
        }
        if self.digital_creation_date_time.is_valid() {
            return false;
        }
        if self.authors.count() != 0 || self.authors_position.not_empty() {
            return false;
        }
        if self.city.not_empty() || self.state.not_empty() || self.country.not_empty() {
            return false;
        }
        if self.country_code.not_empty() {
            return false;
        }
        if self.location.not_empty() {
            return false;
        }
        if self.transmission_reference.not_empty() {
            return false;
        }
        if self.headline.not_empty() {
            return false;
        }
        if self.credit.not_empty() {
            return false;
        }
        if self.source.not_empty() {
            return false;
        }
        if self.copyright_notice.not_empty() {
            return false;
        }
        if self.description.not_empty() || self.description_writer.not_empty() {
            return false;
        }

        true
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `ParseString (stream, s, charSet)`.
    fn parse_string(stream: &mut DngStream, s: &mut DngString, char_set: u32) -> crate::dng_core::DngResult<()> {
        let length = stream.get_uint16()?;

        let mut c = stream.get(length as uint32)?;
        c.push(0);

        // C++ passes a NUL-terminated char*, so the string ends at the first
        // NUL; truncate before decoding.
        let nul = c.iter().position(|&b| b == 0).unwrap_or(c.len());
        c.truncate(nul);

        match char_set {
            kCharSetUTF8 => s.set_utf8(&c),
            _ => s.set_system_encoding(&c),
        }

        s.set_line_endings_to_new_lines();
        s.strip_low_ascii();
        s.trim_trailing_blanks();

        Ok(())
    }

    /// `Parse (blockData, blockSize, offsetInOriginalFile)`.
    pub fn parse(&mut self, block_data: &[u8], offset_in_original_file: u64) {
        let mut stream = DngStream::from_static(block_data, offset_in_original_file);

        stream.set_big_endian(true);

        // First pass: figure out the character set.
        let mut char_set = kCharSetUnknown;
        let mut is_valid_utf8 = true;
        let mut has_encoding_marker = false;

        let first_offset = stream.position();
        let mut next_offset = first_offset;

        let total = stream.length();

        while next_offset + 5 < total {
            if stream.set_read_position(next_offset).is_err() {
                break;
            }

            let first_byte = match stream.get_uint8() {
                Ok(b) => b,
                Err(_) => break,
            };

            if first_byte != 0x1C {
                break;
            }

            let record = stream.get_uint8().unwrap_or(0);
            let data_set = stream.get_uint8().unwrap_or(0);
            let data_size = stream.get_uint16().unwrap_or(0);

            next_offset = stream.position() + data_size as u64;

            if record == 1 && data_set == 90 {
                has_encoding_marker = true;

                if data_size == 3 {
                    let byte1 = stream.get_uint8().unwrap_or(0);
                    let byte2 = stream.get_uint8().unwrap_or(0);
                    let byte3 = stream.get_uint8().unwrap_or(0);

                    if byte1 == 27 /* Escape */ && byte2 == 0x25 && byte3 == 0x47 {
                        char_set = kCharSetUTF8;
                    }
                }
            } else if record == 2 {
                if let Ok(s) = stream.get(data_size as uint32) {
                    is_valid_utf8 = is_valid_utf8 && DngString::is_utf8_bytes(&s);
                }
            }
        }

        // No encoding marker and valid UTF-8: assume UTF-8.
        if !has_encoding_marker && is_valid_utf8 {
            char_set = kCharSetUTF8;
        }

        // Second pass: actually read the data.
        let mut next_offset = first_offset;

        while next_offset + 5 < total {
            if stream.set_read_position(next_offset).is_err() {
                break;
            }

            let first_byte = match stream.get_uint8() {
                Ok(b) => b,
                Err(_) => break,
            };

            if first_byte != 0x1C {
                break;
            }

            let record = stream.get_uint8().unwrap_or(0);
            let data_set_code = stream.get_uint8().unwrap_or(0);
            let data_size = stream.get_uint16().unwrap_or(0);

            next_offset = stream.position() + data_size as u64;

            if record != 2 {
                continue;
            }

            // Rewind to the string length so ParseString can read it.
            stream.set_read_position(stream.position() - 2).ok();

            match data_set_code {
                x if x == kObjectNameSet => {
                    Self::parse_string(&mut stream, &mut self.title, char_set).ok();
                }
                x if x == kUrgencySet => {
                    let size = stream.get_uint16().unwrap_or(0);

                    if size == 1 {
                        let c = stream.get_uint8().unwrap_or(0);

                        if c.is_ascii_digit() {
                            self.urgency = (c - b'0') as i32;
                        }
                    }
                }
                x if x == kCategorySet => {
                    Self::parse_string(&mut stream, &mut self.category, char_set).ok();
                }
                x if x == kSupplementalCategoriesSet => {
                    let mut category = DngString::new();
                    Self::parse_string(&mut stream, &mut category, char_set).ok();
                    if category.not_empty() {
                        self.supplemental_categories.append(&category);
                    }
                }
                x if x == kKeywordsSet => {
                    let mut keyword = DngString::new();
                    Self::parse_string(&mut stream, &mut keyword, char_set).ok();
                    if keyword.not_empty() {
                        self.keywords.append(&keyword);
                    }
                }
                x if x == kSpecialInstructionsSet => {
                    Self::parse_string(&mut stream, &mut self.instructions, char_set).ok();
                }
                x if x == kDateCreatedSet => {
                    let length = stream.get_uint16().unwrap_or(0);

                    if length == 8 {
                        let date = stream.get(8).unwrap_or_default();
                        self.date_time_created.decode_iptc_date(&date);
                    }
                }
                x if x == kTimeCreatedSet => {
                    let length = stream.get_uint16().unwrap_or(0);

                    if (4..=11).contains(&length) {
                        let time = stream.get(length as uint32).unwrap_or_default();
                        self.date_time_created.decode_iptc_time(&time);
                    }
                }
                x if x == kDigitalCreationDateSet => {
                    let length = stream.get_uint16().unwrap_or(0);

                    if length == 8 {
                        let date = stream.get(8).unwrap_or_default();
                        self.digital_creation_date_time.decode_iptc_date(&date);
                    }
                }
                x if x == kDigitalCreationTimeSet => {
                    let length = stream.get_uint16().unwrap_or(0);

                    if (4..=11).contains(&length) {
                        let time = stream.get(length as uint32).unwrap_or_default();
                        self.digital_creation_date_time.decode_iptc_time(&time);
                    }
                }
                x if x == kBylineSet => {
                    let mut author = DngString::new();
                    Self::parse_string(&mut stream, &mut author, char_set).ok();
                    if author.not_empty() {
                        self.authors.append(&author);
                    }
                }
                x if x == kBylineTitleSet => {
                    Self::parse_string(&mut stream, &mut self.authors_position, char_set).ok();
                }
                x if x == kCitySet => {
                    Self::parse_string(&mut stream, &mut self.city, char_set).ok();
                }
                x if x == kProvinceStateSet => {
                    Self::parse_string(&mut stream, &mut self.state, char_set).ok();
                }
                x if x == kCountryNameSet => {
                    Self::parse_string(&mut stream, &mut self.country, char_set).ok();
                }
                x if x == kCountryCodeSet => {
                    Self::parse_string(&mut stream, &mut self.country_code, char_set).ok();
                }
                x if x == kSublocationSet => {
                    Self::parse_string(&mut stream, &mut self.location, char_set).ok();
                }
                x if x == kOriginalTransmissionReferenceSet => {
                    Self::parse_string(&mut stream, &mut self.transmission_reference, char_set)
                        .ok();
                }
                x if x == kHeadlineSet => {
                    Self::parse_string(&mut stream, &mut self.headline, char_set).ok();
                }
                x if x == kCreditSet => {
                    Self::parse_string(&mut stream, &mut self.credit, char_set).ok();
                }
                x if x == kSourceSet => {
                    Self::parse_string(&mut stream, &mut self.source, char_set).ok();
                }
                x if x == kCopyrightNoticeSet => {
                    Self::parse_string(&mut stream, &mut self.copyright_notice, char_set).ok();
                }
                x if x == kCaptionSet => {
                    Self::parse_string(&mut stream, &mut self.description, char_set).ok();
                }
                x if x == kCaptionWriterSet => {
                    Self::parse_string(&mut stream, &mut self.description_writer, char_set).ok();
                }
                // All other IPTC records are not IPTC core / not synced with
                // XMP: ignored.
                _ => {}
            }
        }
    }

    /// `SpoolString (stream, s, dataSet, maxChars, charSet)`.
    fn spool_string(
        stream: &mut DngStream,
        s: &DngString,
        data_set: u8,
        max_chars: uint32,
        char_set: u32,
    ) -> crate::dng_core::DngResult<()> {
        if s.is_empty() {
            return Ok(());
        }

        stream.put_uint16(0x1C02)?;
        stream.put_uint8(data_set)?;

        let mut ss = s.clone();

        ss.set_line_endings_to_returns();

        if char_set == kCharSetUTF8 {
            // UTF-8 encoding.
            if ss.length() > max_chars {
                ss.truncate(max_chars);
            }

            let len = ss.length();

            stream.put_uint16(len as uint16)?;
            stream.put(ss.get())?;
        } else {
            // System character set encoding, binary-search truncated to fit.
            let mut len = ss.get_system_encoding().len() as uint32;

            if len > max_chars {
                let mut lower: uint32 = 0;
                let mut upper: uint32 = ss.length() - 1;

                while upper > lower {
                    let middle = (upper + lower + 1) >> 1;

                    let mut sss = ss.clone();
                    sss.truncate(middle);

                    len = sss.get_system_encoding().len() as uint32;

                    if len <= max_chars {
                        lower = middle;
                    } else {
                        upper = middle - 1;
                    }
                }

                ss.truncate(lower);

                len = ss.get_system_encoding().len() as uint32;
            }

            stream.put_uint16(len as uint16)?;
            stream.put(&ss.get_system_encoding()[..len as usize])?;
        }

        Ok(())
    }

    /// `Spool (allocator, padForTIFF)` — returns the IIM block bytes.
    pub fn spool(&self, pad_for_tiff: bool) -> crate::dng_core::DngResult<DngMemoryBlock> {
        let mut stream = DngMemoryStream::with_page_size(2048).stream();

        stream.set_big_endian(true);

        // Metadata working group: always write UTF-8.
        let char_set = kCharSetUTF8;

        // UTF-8 encoding marker.
        stream.put_uint16(0x1C01)?;
        stream.put_uint8(90)?;
        stream.put_uint16(3)?;
        stream.put_uint8(27)?;
        stream.put_uint8(0x25)?;
        stream.put_uint8(0x47)?;

        // Record version.
        stream.put_uint16(0x1C02)?;
        stream.put_uint8(kRecordVersionSet)?;
        stream.put_uint16(2)?;
        stream.put_uint16(4)?;

        Self::spool_string(&mut stream, &self.title, kObjectNameSet, 64, char_set)?;

        if self.urgency >= 0 {
            let s = format!("{:01}", self.urgency);

            stream.put_uint16(0x1C02)?;
            stream.put_uint8(kUrgencySet)?;
            stream.put_uint16(1)?;
            stream.put(&s.as_bytes()[..1])?;
        }

        Self::spool_string(&mut stream, &self.category, kCategorySet, 3, char_set)?;

        for j in 0..self.supplemental_categories.count() {
            Self::spool_string(
                &mut stream,
                self.supplemental_categories.get(j),
                kSupplementalCategoriesSet,
                32,
                char_set,
            )?;
        }

        for j in 0..self.keywords.count() {
            Self::spool_string(&mut stream, self.keywords.get(j), kKeywordsSet, 64, char_set)?;
        }

        Self::spool_string(
            &mut stream,
            &self.instructions,
            kSpecialInstructionsSet,
            255,
            char_set,
        )?;

        if self.date_time_created.is_valid() {
            let date_string = self.date_time_created.encode_iptc_date();

            if date_string.not_empty() {
                debug_assert_eq!(date_string.length(), 8, "Wrong length IPTC date");

                stream.put_uint16(0x1C02)?;
                stream.put_uint8(kDateCreatedSet)?;
                stream.put_uint16(8)?;
                stream.put(date_string.get())?;
            }

            let time_string = self.date_time_created.encode_iptc_time();

            if time_string.not_empty() {
                stream.put_uint16(0x1C02)?;
                stream.put_uint8(kTimeCreatedSet)?;
                stream.put_uint16(time_string.length() as uint16)?;
                stream.put(time_string.get())?;
            }
        }

        if self.digital_creation_date_time.is_valid() {
            let date_string = self.digital_creation_date_time.encode_iptc_date();

            if date_string.not_empty() {
                debug_assert_eq!(date_string.length(), 8, "Wrong length IPTC date");

                stream.put_uint16(0x1C02)?;
                stream.put_uint8(kDigitalCreationDateSet)?;
                stream.put_uint16(8)?;
                stream.put(date_string.get())?;
            }

            let time_string = self.digital_creation_date_time.encode_iptc_time();

            if time_string.not_empty() {
                stream.put_uint16(0x1C02)?;
                stream.put_uint8(kDigitalCreationTimeSet)?;
                stream.put_uint16(time_string.length() as uint16)?;
                stream.put(time_string.get())?;
            }
        }

        for j in 0..self.authors.count() {
            Self::spool_string(&mut stream, self.authors.get(j), kBylineSet, 32, char_set)?;
        }

        Self::spool_string(&mut stream, &self.authors_position, kBylineTitleSet, 32, char_set)?;
        Self::spool_string(&mut stream, &self.city, kCitySet, 32, char_set)?;
        Self::spool_string(&mut stream, &self.location, kSublocationSet, 32, char_set)?;
        Self::spool_string(&mut stream, &self.state, kProvinceStateSet, 32, char_set)?;
        Self::spool_string(&mut stream, &self.country_code, kCountryCodeSet, 3, char_set)?;
        Self::spool_string(&mut stream, &self.country, kCountryNameSet, 64, char_set)?;
        Self::spool_string(
            &mut stream,
            &self.transmission_reference,
            kOriginalTransmissionReferenceSet,
            32,
            char_set,
        )?;
        Self::spool_string(&mut stream, &self.headline, kHeadlineSet, 255, char_set)?;
        Self::spool_string(&mut stream, &self.credit, kCreditSet, 32, char_set)?;
        Self::spool_string(&mut stream, &self.source, kSourceSet, 32, char_set)?;
        Self::spool_string(
            &mut stream,
            &self.copyright_notice,
            kCopyrightNoticeSet,
            128,
            char_set,
        )?;
        Self::spool_string(&mut stream, &self.description, kCaptionSet, 2000, char_set)?;
        Self::spool_string(
            &mut stream,
            &self.description_writer,
            kCaptionWriterSet,
            32,
            char_set,
        )?;

        if pad_for_tiff {
            while stream.length() & 3 != 0 {
                stream.put_uint8(0)?;
            }
        }

        stream.flush()?;

        stream.as_memory_block()
    }
}

type uint16 = u16;
type uint32 = u32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_by_default() {
        let iptc = DngIptc::new();
        assert!(iptc.is_empty());
        assert_eq!(iptc.urgency, -1);
    }

    #[test]
    fn spool_parse_roundtrip() {
        let mut iptc = DngIptc::new();
        iptc.title.set(b"Test Title");
        iptc.urgency = 3;
        iptc.category.set(b"NEWS");
        iptc.keywords.append(&DngString::from_bytes(b"alpha"));
        iptc.keywords.append(&DngString::from_bytes(b"beta"));
        iptc.authors.append(&DngString::from_bytes(b"Photographer"));
        iptc.city.set(b"San Jose");
        iptc.description.set(b"A description");

        let block = iptc.spool(false).unwrap();

        let mut parsed = DngIptc::new();
        parsed.parse(block.buffer(), 0);

        assert_eq!(parsed.title.get(), b"Test Title");
        assert_eq!(parsed.urgency, 3);
        // Category is capped at 3 chars by SpoolString (C++ maxChars).
        assert_eq!(parsed.category.get(), b"NEW");
        assert_eq!(parsed.keywords.count(), 2);
        assert_eq!(parsed.keywords.get(0).get(), b"alpha");
        assert_eq!(parsed.keywords.get(1).get(), b"beta");
        assert_eq!(parsed.authors.count(), 1);
        assert_eq!(parsed.authors.get(0).get(), b"Photographer");
        assert_eq!(parsed.city.get(), b"San Jose");
        assert_eq!(parsed.description.get(), b"A description");
        assert!(!parsed.is_empty());
    }

    #[test]
    fn utf8_encoding_marker() {
        let mut iptc = DngIptc::new();
        iptc.title.set("Pokémon".as_bytes());

        let block = iptc.spool(false).unwrap();
        let bytes = block.buffer();

        // Starts with the 0x1C01 record-1 encoding marker.
        assert_eq!(&bytes[..2], &[0x1C, 0x01]);
        assert_eq!(bytes[2], 90);

        // Round trip keeps UTF-8.
        let mut parsed = DngIptc::new();
        parsed.parse(bytes, 0);
        assert_eq!(parsed.title.get(), "Pokémon".as_bytes());
    }

    #[test]
    fn parse_minimal_block() {
        // Hand-built block: object name only.
        let mut block: Vec<u8> = vec![];
        block.extend_from_slice(&[0x1C, 0x02, 0x05]);
        block.extend_from_slice(&[0, 9]); // length 9
        block.extend_from_slice(b"My Object");

        let mut iptc = DngIptc::new();
        iptc.parse(&block, 0);
        assert_eq!(iptc.title.get(), b"My Object");
    }

    #[test]
    fn urgency_parse() {
        let mut block: Vec<u8> = vec![];
        block.extend_from_slice(&[0x1C, 0x02, 0x0A, 0x00, 0x01, b'7']);

        let mut iptc = DngIptc::new();
        iptc.parse(&block, 0);
        assert_eq!(iptc.urgency, 7);
    }

    #[test]
    fn tiff_padding() {
        let mut iptc = DngIptc::new();
        iptc.title.set(b"abc"); // 3 chars; block 6+... not multiple of 4

        let block = iptc.spool(true).unwrap();
        assert_eq!(block.logical_size() % 4, 0);

        let block2 = iptc.spool(false).unwrap();
        // Unpadded may be shorter.
        assert!(block2.logical_size() <= block.logical_size());
    }

    #[test]
    fn date_time_created_roundtrip() {
        let mut iptc = DngIptc::new();
        let mut info = DngDateTimeInfo::new();
        info.decode_iptc_date(b"20040119");
        info.decode_iptc_time(b"183451+0800");
        iptc.date_time_created = info;

        let block = iptc.spool(false).unwrap();

        let mut parsed = DngIptc::new();
        parsed.parse(block.buffer(), 0);

        assert!(parsed.date_time_created.is_valid());
        assert_eq!(parsed.date_time_created.date_time().year, 2004);
        assert_eq!(parsed.date_time_created.time_zone().offset_minutes(), 480);
    }
}

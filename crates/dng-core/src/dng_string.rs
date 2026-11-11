// Port of dng_string.h/.cpp — the SDK string type.
//
// C++ `dng_string` stores arbitrary bytes (not necessarily valid UTF-8) in a
// copy-on-write `std::string`; a null shared pointer represents the cleared
// state.  The Rust port stores `Option<Vec<u8>>`:
//   * `None`  == cleared (C++ null `fData`)  — `is_empty()` is exactly `None`.
//   * `Some`  == the raw byte string (`Get ()`).
// This keeps byte fidelity for EXIF/IPTC payloads that are not valid UTF-8;
// `get_str()` provides a UTF-8 view when the contents are valid text.
//
// System-encoding portability (see PORTING_NOTES.md): the C++ uses the OS
// API (Mac TextEncoding / Win `GetACP`).  The Rust port uses the Windows-1252
// code page for the non-ASCII path (deterministic, matches `GetACP` on
// Western-locale Windows where most such DNG metadata originates) and the
// C++ "other platform" fallbacks for `Set_JIS_X208_1990` and `Compare`.

use crate::dng_types::{int32, uint32, uint16};
use crate::dng_utils::force_uppercase;

pub const kREPLACEMENT_CHARACTER: uint32 = 0x0000FFFD;

/// C++ `IsASCII` helper.
pub fn is_ascii_bytes(s: &[u8]) -> bool {
    s.iter().all(|&c| c & 0x80 == 0)
}

/// C++ `strlenAsUint32` equivalent: length of a NUL-terminated slice
/// (the Rust port works on slices without interior NULs, so this is `len`).
#[inline]
fn strlen_as_uint32(s: &[u8]) -> uint32 {
    s.len() as uint32
}

/// Port of `dng_string`.
#[derive(Debug, Clone, Default, Eq)]
pub struct DngString {
    data: Option<Vec<u8>>,
}

impl DngString {
    /// `dng_string ()` — cleared.
    pub fn new() -> Self {
        DngString { data: None }
    }

    /// `dng_string (const char *s)`.
    pub fn from_bytes(s: &[u8]) -> Self {
        let mut out = DngString::new();
        out.set(s);
        out
    }

    /// `Get ()` — raw bytes (empty slice when cleared).
    pub fn get(&self) -> &[u8] {
        match &self.data {
            Some(d) => d,
            None => &[],
        }
    }

    /// UTF-8 view of `Get ()` when the bytes are valid UTF-8.
    pub fn get_str(&self) -> Option<&str> {
        std::str::from_utf8(self.get()).ok()
    }

    /// `IsASCII ()`.
    pub fn is_ascii(&self) -> bool {
        is_ascii_bytes(self.get())
    }

    /// `Set (const char *s)` — an empty input clears the string.
    pub fn set(&mut self, s: &[u8]) {
        if s.is_empty() {
            self.data = None;
        } else {
            self.data = Some(s.to_vec());
        }
    }

    /// `Set_ASCII (const char *s)`.
    pub fn set_ascii(&mut self, s: &[u8]) {
        if is_ascii_bytes(s) {
            self.set(s);
        } else {
            self.set_system_encoding(s);
        }
    }

    /// `Set_UTF8 (const char *s)` — decodes/repairs UTF-8 exactly like C++
    /// (invalid sequences become U+FFFD, then re-encoded).
    pub fn set_utf8(&mut self, s: &[u8]) {
        let len = s.len();
        let mut buffer: Vec<u8> = Vec::with_capacity(len * 3);

        let mut pos = 0usize;
        while pos < len {
            let remaining = (len - pos) as uint32;
            let (mut a_char, _valid) = Self::decode_utf8_at(s, &mut pos, remaining);

            if a_char > 0x7FFF_FFFF {
                a_char = kREPLACEMENT_CHARACTER;
            }

            append_unicode_char32(&mut buffer, a_char);
        }

        self.set(&buffer);
    }

    /// `Get_SystemEncoding (dng_memory_data &buffer)` — returns the encoded
    /// bytes (the C++ additionally NUL-terminates its out-buffer, which is
    /// not needed with slices; see PORTING_NOTES.md).
    pub fn get_system_encoding(&self) -> Vec<u8> {
        if self.is_ascii() {
            return self.get().to_vec();
        }
        extract_system_encoding(self)
    }

    /// `Set_SystemEncoding (const char *s)`.
    pub fn set_system_encoding(&mut self, s: &[u8]) {
        if is_ascii_bytes(s) {
            self.set(s);
        } else {
            let decoded = assign_system_encoding(s);
            self.set(&decoded);
        }
    }

    /// `ValidSystemEncoding ()`.
    pub fn valid_system_encoding(&self) -> bool {
        if self.is_ascii() {
            return true;
        }
        let buffer = self.get_system_encoding();
        let mut temp = DngString::new();
        temp.set_system_encoding(&buffer);
        *self == temp
    }

    /// `Set_JIS_X208_1990 (const char *s)`.
    /// PORT-NOTE(dng_string.cpp:786): the JIS X 0208-1990 conversion tables
    /// are OS-provided in C++ (Mac TextEncoding / Win code page 20932); the
    /// Rust port uses the C++ non-Mac/non-Win fallback (ASCII extraction via
    /// Set_SystemEncoding), which is itself a sanctioned C++ code path.
    pub fn set_jis_x208_1990(&mut self, s: &[u8]) {
        if is_ascii_bytes(s) {
            self.set(s);
        } else {
            self.set_system_encoding(s);
        }
    }

    /// `DecodeUTF8 (const char *&s, uint32 maxBytes, bool *isValid)` —
    /// static decoder.  Operates on `bytes` starting at `*offset`; on return
    /// `*offset` is advanced and the second tuple element reports validity.
    pub fn decode_utf8_at(bytes: &[u8], offset: &mut usize, max_bytes: uint32) -> (uint32, bool) {
        static gUTF8Bytes: [u8; 256] = [
            1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
            1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
            1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
            1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,1,
            0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
            0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,
            2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,2,
            3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,3,4,4,4,4,4,4,4,4,5,5,5,5,6,6,6,6
        ];

        let start = *offset;
        let n_buf = &bytes[start..];
        if n_buf.is_empty() {
            // Defensive: C++ relies on the NUL terminator; empty input yields
            // the replacement char without advancing.
            return (kREPLACEMENT_CHARACTER, false);
        }

        let mut a_char = n_buf[0] as uint32;
        let a_size = gUTF8Bytes[a_char as usize] as uint32;

        if a_size > max_bytes {
            *offset = start + max_bytes as usize;
            return (kREPLACEMENT_CHARACTER, false);
        }

        // C++ reads the continuation bytes even past the string end in some
        // malformed cases; we bound-check and treat out-of-range as invalid.
        if (a_size as usize) > n_buf.len() {
            *offset = start + n_buf.len();
            return (kREPLACEMENT_CHARACTER, false);
        }

        *offset = start + a_size as usize;

        for extra in 1..a_size as usize {
            if (n_buf[extra] & 0xC0) != 0x80 {
                return (kREPLACEMENT_CHARACTER, false);
            }
        }

        match a_size {
            0 => {
                *offset = start + 1; // Don't get stuck in infinite loop
                (kREPLACEMENT_CHARACTER, false)
            }
            1 => (a_char, true),
            2 => {
                a_char = ((a_char << 6).wrapping_add(n_buf[1] as uint32))
                    .wrapping_sub(0x00003080);
                Self::validate_decoded(a_char, *offset)
            }
            3 => {
                a_char = ((((a_char << 6).wrapping_add(n_buf[1] as uint32)) << 6)
                    .wrapping_add(n_buf[2] as uint32))
                .wrapping_sub(0x000E2080);
                Self::validate_decoded(a_char, *offset)
            }
            4 => {
                a_char = ((((((a_char << 6).wrapping_add(n_buf[1] as uint32)) << 6)
                    .wrapping_add(n_buf[2] as uint32))
                << 6)
                    .wrapping_add(n_buf[3] as uint32))
                .wrapping_sub(0x03C82080);
                Self::validate_decoded(a_char, *offset)
            }
            5 => {
                a_char = ((((((((a_char << 6).wrapping_add(n_buf[1] as uint32)) << 6)
                    .wrapping_add(n_buf[2] as uint32))
                << 6)
                    .wrapping_add(n_buf[3] as uint32))
                << 6)
                    .wrapping_add(n_buf[4] as uint32))
                .wrapping_sub(0xFA082080);
                Self::validate_decoded(a_char, *offset)
            }
            6 => {
                a_char = ((((((((((a_char << 6).wrapping_add(n_buf[1] as uint32)) << 6)
                    .wrapping_add(n_buf[2] as uint32))
                << 6)
                    .wrapping_add(n_buf[3] as uint32))
                << 6)
                    .wrapping_add(n_buf[4] as uint32))
                << 6)
                    .wrapping_add(n_buf[5] as uint32))
                .wrapping_sub(0x82082080);
                Self::validate_decoded(a_char, *offset)
            }
            _ => (kREPLACEMENT_CHARACTER, false),
        }
    }

    fn validate_decoded(a_char: uint32, _offset: usize) -> (uint32, bool) {
        if a_char < 0x7F || a_char > 0x0010_FFFF {
            (kREPLACEMENT_CHARACTER, false)
        } else {
            (a_char, true)
        }
    }

    /// `IsUTF8 (const char *s)`.
    pub fn is_utf8_bytes(s: &[u8]) -> bool {
        let len = s.len();
        let mut pos = 0usize;
        while pos < len {
            let remaining = (len - pos) as uint32;
            let (_, valid) = Self::decode_utf8_at(s, &mut pos, remaining);
            if !valid {
                return false;
            }
        }
        true
    }

    /// `Set_UTF8_or_System (const char *s)`.
    pub fn set_utf8_or_system(&mut self, s: &[u8]) {
        if is_ascii_bytes(s) {
            self.set(s);
        } else if Self::is_utf8_bytes(s) {
            self.set_utf8(s);
        } else {
            self.set_system_encoding(s);
        }
    }

    /// `Get_UTF16 (dng_memory_data &buffer)` — returns the UTF-16 code units
    /// (the C++ count excludes its NUL terminator; see PORTING_NOTES.md).
    pub fn get_utf16(&self) -> Vec<uint16> {
        let mut out: Vec<uint16> = Vec::new();

        let bytes = self.get();
        let len = bytes.len();
        let mut pos = 0usize;
        while pos < len {
            let (x, _) = Self::decode_utf8_at(bytes, &mut pos, 6);
            if x <= 0x0000_FFFF || x > 0x0010_FFFF {
                out.push(x as uint16);
            } else {
                out.push((((x - 0x0001_0000) >> 10) + 0x0000_D800) as uint16);
                out.push(((x & 0x0000_03FF) + 0x0000_DC00) as uint16);
            }
        }

        out
    }

    /// `Set_UTF16 (const uint16 *s)` — handles optional BOM and surrogate pairs.
    pub fn set_utf16(&mut self, s: &[uint16]) {
        // C++ checks for null; empty slice behaves as a terminator-only string.
        let mut s = s;
        if s.is_empty() {
            self.clear();
            return;
        }

        let mut swap = false;

        if s[0] == 0xFFFE {
            // Swapped byte order marker
            swap = true;
            s = &s[1..];
        } else if s[0] == 0xFEFF {
            // Non-swapped byte order marker
            s = &s[1..];
        }

        let mut buffer: Vec<u8> = Vec::with_capacity(s.len() * 6);

        let mut i = 0usize;
        while i < s.len() {
            let mut a_char = s[i] as uint32;
            i += 1;

            if swap {
                a_char = ((a_char << 8) | (a_char >> 8)) & 0x0000_FFFF;
            }

            if (0x0000_D800..=0x0000_DBFF).contains(&a_char) && i < s.len() {
                let mut a_low = s[i] as uint32;
                if swap {
                    a_low = ((a_low << 8) | (a_low >> 8)) & 0x0000_FFFF;
                }
                if (0x0000_DC00..=0x0000_DFFF).contains(&a_low) {
                    a_char = ((a_char - 0x0000_D800) << 10) + (a_low - 0x0000_DC00) + 0x0001_0000;
                    i += 1;
                }
            }

            if a_char > 0x7FFF_FFFF {
                a_char = kREPLACEMENT_CHARACTER;
            }

            append_unicode_char32(&mut buffer, a_char);
        }

        self.set(&buffer);
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        self.data = None;
    }

    /// `Truncate (uint32 maxBytes)`.
    pub fn truncate(&mut self, max_bytes: uint32) {
        let len = self.length();
        if len > max_bytes {
            let mut data = self.data.clone().unwrap_or_default();

            // Don't truncate on an extension character.  Extension characters
            // in UTF-8 have the 0x80 bit set and the 0x40 bit clear.
            let mut max_bytes = max_bytes as usize;
            while max_bytes > 0 && (data[max_bytes] & 0xC0) == 0x80 {
                max_bytes -= 1;
            }

            data.truncate(max_bytes);
            self.set(&data);
        }
    }

    /// `TrimTrailingBlanks ()`.
    pub fn trim_trailing_blanks(&mut self) -> bool {
        let mut did_trim = false;

        if let Some(data) = self.data.clone() {
            if *data.last().unwrap_or(&0) == b' ' {
                let mut len = data.len();
                while len > 0 && data[len - 1] == b' ' {
                    len -= 1;
                    did_trim = true;
                }
                self.set(&data[..len]);
            }
        }

        did_trim
    }

    /// `TrimLeadingBlanks ()`.
    pub fn trim_leading_blanks(&mut self) -> bool {
        let mut did_trim = false;
        let s = self.get().to_vec();
        let mut pos = 0usize;
        while pos < s.len() && s[pos] == b' ' {
            pos += 1;
            did_trim = true;
        }
        if did_trim {
            self.set(&s[pos..]);
        }
        did_trim
    }

    /// `IsEmpty ()`.
    pub fn is_empty(&self) -> bool {
        self.data.is_none()
    }

    /// `NotEmpty ()`.
    pub fn not_empty(&self) -> bool {
        !self.is_empty()
    }

    /// `Length ()`.
    pub fn length(&self) -> uint32 {
        self.data.as_ref().map_or(0, |d| d.len() as uint32)
    }

    /// `dng_string::Matches (t, s, case_sensitive)` — static form: does `t`
    /// exactly match `s`?
    pub fn matches_bytes(t: &[u8], s: &[u8], case_sensitive: bool) -> bool {
        if t.len() < s.len() {
            return false;
        }
        if case_sensitive {
            &t[..s.len()] == s
        } else {
            t[..s.len()]
                .iter()
                .zip(s.iter())
                .all(|(a, b)| force_uppercase(*a) == force_uppercase(*b))
        }
    }

    /// `Matches (s, case_sensitive)`.
    pub fn matches(&self, s: &[u8], case_sensitive: bool) -> bool {
        Self::matches_bytes(self.get(), s, case_sensitive)
    }

    /// `StartsWith (s, case_sensitive)`.
    pub fn starts_with(&self, s: &[u8], case_sensitive: bool) -> bool {
        let t = self.get();
        if t.len() < s.len() {
            return false;
        }
        if case_sensitive {
            &t[..s.len()] == s
        } else {
            t[..s.len()]
                .iter()
                .zip(s.iter())
                .all(|(a, b)| force_uppercase(*a) == force_uppercase(*b))
        }
    }

    /// `EndsWith (s, case_sensitive)`.
    pub fn ends_with(&self, s: &[u8], case_sensitive: bool) -> bool {
        let t = self.get();
        let len1 = t.len();
        let len2 = s.len();
        if len1 < len2 {
            return false;
        }
        let tail = &t[len1 - len2..];
        if case_sensitive {
            tail == s
        } else {
            tail.iter()
                .zip(s.iter())
                .all(|(a, b)| force_uppercase(*a) == force_uppercase(*b))
        }
    }

    /// `Contains (s, case_sensitive, match_offset)`.
    pub fn contains_with_offset(&self, s: &[u8], case_sensitive: bool) -> (bool, int32) {
        let t = self.get();
        let len1 = t.len();
        let len2 = s.len();
        if len1 < len2 {
            return (false, -1);
        }
        for offset in 0..=(len1 - len2) {
            let window = &t[offset..offset + len2];
            let matched = if case_sensitive {
                window == s
            } else {
                window
                    .iter()
                    .zip(s.iter())
                    .all(|(a, b)| force_uppercase(*a) == force_uppercase(*b))
            };
            if matched {
                return (true, offset as int32);
            }
        }
        (false, -1)
    }

    /// `Contains` without the offset out-param.
    pub fn contains(&self, s: &[u8], case_sensitive: bool) -> bool {
        self.contains_with_offset(s, case_sensitive).0
    }

    /// `Replace (old_string, new_string, case_sensitive)`.
    pub fn replace(&mut self, old_string: &[u8], new_string: &[u8], case_sensitive: bool) -> bool {
        let (found, match_offset) = self.contains_with_offset(old_string, case_sensitive);
        if found {
            let mut new_data = self.get().to_vec();
            let _ = new_data.len();
            let start = match_offset as usize;
            new_data.splice(start..start + old_string.len(), new_string.iter().copied());
            self.set(&new_data);
            true
        } else {
            false
        }
    }

    /// `ReplaceChars (oldChar, newChar)`.
    pub fn replace_chars(&mut self, old_char: u8, new_char: u8) {
        if let Some(data) = &mut self.data {
            for c in data.iter_mut() {
                if *c == old_char {
                    *c = new_char;
                }
            }
        }
    }

    /// `TrimLeading (s, case_sensitive)`.
    pub fn trim_leading(&mut self, s: &[u8], case_sensitive: bool) -> bool {
        if self.starts_with(s, case_sensitive) {
            let rest = self.get()[s.len()..].to_vec();
            self.set(&rest);
            true
        } else {
            false
        }
    }

    /// `Append (s)`.
    pub fn append(&mut self, s: &[u8]) {
        if !s.is_empty() {
            if let Some(data) = self.data.clone() {
                let mut new_data = data;
                new_data.extend_from_slice(s);
                self.data = Some(new_data);
            } else {
                self.set(s);
            }
        }
    }

    /// `SetUppercase ()` — ASCII a-z only, exactly like C++.
    pub fn set_uppercase(&mut self) {
        if let Some(data) = &mut self.data {
            for c in data.iter_mut() {
                if (b'a'..=b'z').contains(c) {
                    *c = *c - b'a' + b'A';
                }
            }
        }
    }

    /// `SetLowercase ()` — ASCII A-Z only, exactly like C++.
    pub fn set_lowercase(&mut self) {
        if let Some(data) = &mut self.data {
            for c in data.iter_mut() {
                if (b'A'..=b'Z').contains(c) {
                    *c = *c - b'A' + b'a';
                }
            }
        }
    }

    /// `SetLineEndings (ending)`.
    pub fn set_line_endings(&mut self, ending: u8) {
        debug_assert!(ending == b'\n' || ending == b'\r', "Unexpected line ending in dng_string::SetLineEndings");

        if let Some(data) = &self.data {
            let other_ending: u8 = if ending == b'\n' { b'\r' } else { b'\n' };

            if data.contains(&other_ending) {
                let mut new_data: Vec<u8> = Vec::with_capacity(data.len());

                let mut i = 0usize;
                while i < data.len() {
                    let c = data[i];
                    let nc = data.get(i + 1).copied().unwrap_or(0);
                    i += 1;

                    if (c == b'\r' && nc == b'\n') || (c == b'\n' && nc == b'\r') {
                        i += 1;
                        new_data.push(ending);
                    } else if c == b'\n' || c == b'\r' {
                        new_data.push(ending);
                    } else {
                        new_data.push(c);
                    }
                }

                self.set(&new_data);
            }
        }
    }

    /// `SetLineEndingsToNewLines ()`.
    pub fn set_line_endings_to_new_lines(&mut self) {
        self.set_line_endings(b'\n');
    }

    /// `SetLineEndingsToReturns ()`.
    pub fn set_line_endings_to_returns(&mut self) {
        self.set_line_endings(b'\r');
    }

    /// `StripLowASCII ()`.
    pub fn strip_low_ascii(&mut self) {
        if let Some(data) = self.data.clone() {
            let has_low_ascii = data
                .iter()
                .any(|&c| !(c == b'\r' || c == b'\n' || c >= b' '));

            if has_low_ascii {
                let new_data: Vec<u8> = data
                    .iter()
                    .copied()
                    .filter(|&c| c == b'\r' || c == b'\n' || c >= b' ')
                    .collect();
                self.set(&new_data);
            }
        }
    }

    /// `ForceASCII ()`.
    pub fn force_ascii(&mut self) {
        if !self.is_ascii() {
            let mut buffer: Vec<u8> = Vec::with_capacity(self.length() as usize * 3);

            let bytes = self.get();
            let len = bytes.len();
            let mut pos = 0usize;
            while pos < len {
                let (x, _) = Self::decode_utf8_at(bytes, &mut pos, 6);

                if x <= 0x007F {
                    buffer.push(x as u8);
                } else if let Some(ascii) = unicode_to_low_ascii(x) {
                    buffer.extend_from_slice(ascii);
                } else {
                    buffer.push(b'?');
                }
            }

            self.set(&buffer);
        }
    }

    /// `Compare (s, digitsAsNumber)` — uses the C++ pure-Unicode fallback
    /// algorithm (see module docs); two passes, case-insensitive first,
    /// optional digit runs as numbers.
    pub fn compare(&self, s: &DngString, digits_as_number: bool) -> int32 {
        let a_bytes = self.get();
        let b_bytes = s.get();

        for pass in 0..2 {
            let mut a_ptr = 0usize;
            let mut b_ptr = 0usize;

            loop {
                let a_done = a_ptr >= a_bytes.len();
                let b_done = b_ptr >= b_bytes.len();
                if a_done && b_done {
                    break;
                }
                if b_done {
                    return 1;
                }
                if a_done {
                    return -1;
                }

                let a_remaining = (a_bytes.len() - a_ptr) as uint32;
                let (mut a, _) = Self::decode_utf8_at(a_bytes, &mut a_ptr, a_remaining);
                let b_remaining = (b_bytes.len() - b_ptr) as uint32;
                let (mut b, _) = Self::decode_utf8_at(b_bytes, &mut b_ptr, b_remaining);

                // Ignore case on first compare pass.
                if pass == 0 {
                    if (b'a' as uint32..=b'z' as uint32).contains(&a) {
                        a = a - 'a' as uint32 + 'A' as uint32;
                    }
                    if (b'a' as uint32..=b'z' as uint32).contains(&b) {
                        b = b - 'a' as uint32 + 'A' as uint32;
                    }
                }

                if digits_as_number {
                    let mut a_number: uint32 = 0;
                    let mut a_digits: uint32 = 0;

                    if (b'0' as uint32..=b'9' as uint32).contains(&a) {
                        a_number = a - '0' as uint32;
                        a_digits = 1;
                        while a_digits < 6
                            && a_ptr < a_bytes.len()
                            && a_bytes[a_ptr].is_ascii_digit()
                        {
                            a_number = a_number * 10 + (a_bytes[a_ptr] as uint32) - '0' as uint32;
                            a_digits += 1;
                            a_ptr += 1;
                        }
                    }

                    let mut b_number: uint32 = 0;
                    let mut b_digits: uint32 = 0;

                    if (b'0' as uint32..=b'9' as uint32).contains(&b) {
                        b_number = b - '0' as uint32;
                        b_digits = 1;
                        while b_digits < 6
                            && b_ptr < b_bytes.len()
                            && b_bytes[b_ptr].is_ascii_digit()
                        {
                            b_number = b_number * 10 + (b_bytes[b_ptr] as uint32) - '0' as uint32;
                            b_digits += 1;
                            b_ptr += 1;
                        }
                    }

                    if a_digits > 0 && b_digits > 0 {
                        if a_number > b_number {
                            return 1;
                        }
                        if a_number < b_number {
                            return -1;
                        }
                        if a_digits > b_digits {
                            return 1;
                        }
                        if a_digits < b_digits {
                            return -1;
                        }
                        continue;
                    }
                }

                if a > b {
                    return 1;
                } else if a < b {
                    return -1;
                }
            }
        }

        0
    }

    /// `NormalizeAsCommaSeparatedNumbers ()`.
    pub fn normalize_as_comma_separated_numbers(&mut self) {
        if let Some(data) = self.data.clone() {
            let mut new_data: Vec<u8> = Vec::with_capacity(data.len());

            let mut comma_inserted = false;

            let mut pos = 0usize;
            while pos < data.len() {
                let remaining = (data.len() - pos) as uint32;
                let (c, _) = Self::decode_utf8_at(&data, &mut pos, remaining);

                // Support number formats such as "3", "+3.0", "-3.1416",
                // "314.16e-2", "0.31416E1", but no hex/octal representations.
                if (b'0' as uint32..=b'9' as uint32).contains(&c)
                    || c == '.' as uint32
                    || c == '-' as uint32
                    || c == '+' as uint32
                    || c == 'e' as uint32
                    || c == 'E' as uint32
                {
                    // C++ pushes (char) c — the truncated low byte; the
                    // accepted set is ASCII so this is the same byte.
                    new_data.push(c as u8);
                    if comma_inserted {
                        comma_inserted = false;
                    }
                } else if !comma_inserted {
                    new_data.push(b',');
                    comma_inserted = true;
                }
            }

            self.set(&new_data);
        }
    }
}

impl PartialEq for DngString {
    fn eq(&self, other: &Self) -> bool {
        match (&self.data, &other.data) {
            (None, None) => true,
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

impl std::fmt::Display for DngString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.get_str() {
            Some(s) => f.write_str(s),
            None => write!(f, "{}", String::from_utf8_lossy(self.get())),
        }
    }
}

impl From<&str> for DngString {
    fn from(s: &str) -> Self {
        DngString::from_bytes("s".as_bytes())
    }
}

/// `AppendUnicodeChar32` — encodes a code point as UTF-8 (up to 6-byte form,
/// exactly like C++, including overlong encodings for the paths that use it).
pub fn append_unicode_char32(buffer: &mut Vec<u8>, a_char: uint32) {
    if a_char < 0x0000_0080 {
        buffer.push(a_char as u8);
    } else if a_char < 0x0000_0800 {
        buffer.push(((a_char >> 6) | 0x0000_00C0) as u8);
        buffer.push(((a_char & 0x0000_003F) | 0x0000_0080) as u8);
    } else if a_char < 0x0001_0000 {
        buffer.push(((a_char >> 12) | 0x0000_00E0) as u8);
        buffer.push((((a_char >> 6) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push(((a_char & 0x0000_003F) | 0x0000_0080) as u8);
    } else if a_char < 0x0020_0000 {
        buffer.push(((a_char >> 18) | 0x0000_00F0) as u8);
        buffer.push((((a_char >> 12) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 6) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push(((a_char & 0x0000_003F) | 0x0000_0080) as u8);
    } else if a_char < 0x0400_0000 {
        buffer.push(((a_char >> 24) | 0x0000_00F8) as u8);
        buffer.push((((a_char >> 18) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 12) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 6) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push(((a_char & 0x0000_003F) | 0x0000_0080) as u8);
    } else {
        buffer.push(((a_char >> 30) | 0x0000_00FC) as u8);
        buffer.push((((a_char >> 24) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 18) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 12) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push((((a_char >> 6) & 0x0000_003F) | 0x0000_0080) as u8);
        buffer.push(((a_char & 0x0000_003F) | 0x0000_0080) as u8);
    }
}

// ---------------------------------------------------------------------------
// System encoding: Windows-1252 (see module docs / PORTING_NOTES.md).
// ---------------------------------------------------------------------------

/// Unicode → Windows-1252 for the 0x80..0x9F range that differs from Latin-1.
const WIN1252_HIGH: [Option<char>; 32] = [
    Some('\u{20AC}'), // 80
    None,             // 81 (undefined)
    Some('\u{201A}'), // 82
    Some('\u{0192}'), // 83
    Some('\u{201E}'), // 84
    Some('\u{2026}'), // 85
    Some('\u{2020}'), // 86
    Some('\u{2021}'), // 87
    Some('\u{02C6}'), // 88
    Some('\u{2030}'), // 89
    Some('\u{0160}'), // 8A
    Some('\u{2039}'), // 8B
    Some('\u{0152}'), // 8C
    None,             // 8D
    Some('\u{017D}'), // 8E
    None,             // 8F
    None,             // 90
    Some('\u{2018}'), // 91
    Some('\u{2019}'), // 92
    Some('\u{201C}'), // 93
    Some('\u{201D}'), // 94
    Some('\u{2022}'), // 95
    Some('\u{2013}'), // 96
    Some('\u{2014}'), // 97
    Some('\u{02DC}'), // 98
    Some('\u{2122}'), // 99
    Some('\u{0161}'), // 9A
    Some('\u{203A}'), // 9B
    Some('\u{0153}'), // 9C
    None,             // 9D
    Some('\u{017E}'), // 9E
    Some('\u{0178}'), // 9F
];

/// `Assign_SystemEncoding` — decode system-encoding bytes to a UTF-8 byte
/// string (C++ converts to UTF-16 first, then `Set_UTF16`).
fn assign_system_encoding(s: &[u8]) -> Vec<u8> {
    let mut utf16: Vec<uint16> = Vec::with_capacity(s.len() + 1);

    for &b in s {
        if b < 0x80 || b >= 0xA0 {
            utf16.push(b as uint16);
        } else {
            match WIN1252_HIGH[(b - 0x80) as usize] {
                Some(ch) => utf16.push(ch as uint16),
                // Undefined 1252 positions map to the byte value (best-fit),
                // mirroring MultiByteToWideChar with best-fit mapping.
                None => utf16.push(b as uint16),
            }
        }
    }

    let mut out = DngString::new();
    out.set_utf16(&utf16);
    out.get().to_vec()
}

/// `Extract_SystemEncoding` — encode the UTF-8 string into system-encoding
/// bytes (unmappable characters become '?', mirroring WideCharToMultiByte
/// default fallback behavior).
fn extract_system_encoding(s: &DngString) -> Vec<u8> {
    let utf16 = s.get_utf16();
    let mut out: Vec<u8> = Vec::with_capacity(utf16.len());

    let mut i = 0usize;
    while i < utf16.len() {
        let mut x = utf16[i] as uint32;
        i += 1;

        // Combine surrogate pairs.
        if (0xD800..=0xDBFF).contains(&x) && i < utf16.len() {
            let low = utf16[i] as uint32;
            if (0xDC00..=0xDFFF).contains(&low) {
                x = ((x - 0xD800) << 10) + (low - 0xDC00) + 0x10000;
                i += 1;
            }
        }

        if x < 0x80 {
            out.push(x as u8);
        } else if x < 0x100 && !(0x80..0xA0).contains(&x) {
            out.push(x as u8);
        } else if x < 0x100 {
            // In 0x80..0xA0: check the 1252 high table.
            if let Some(ch) = WIN1252_HIGH[(x - 0x80) as usize] {
                if (ch as u32) == x {
                    out.push(x as u8);
                } else {
                    out.push(b'?');
                }
            } else {
                out.push(b'?');
            }
        } else {
            // Search the high table for a 1252 byte mapping to this code point.
            let mut found = false;
            for (idx, entry) in WIN1252_HIGH.iter().enumerate() {
                if let Some(ch) = entry {
                    if (*ch as u32) == x {
                        out.push(0x80 + idx as u8);
                        found = true;
                        break;
                    }
                }
            }
            if !found {
                // Latin-1 supplement is directly mapped in 1252.
                if (0x00A0..=0x00FF).contains(&x) {
                    out.push(x as u8);
                } else {
                    out.push(b'?');
                }
            }
        }
    }

    out
}

/// Unicode → low-ASCII table (`kUnicodeToLowASCII`), used by `ForceASCII`.
fn unicode_to_low_ascii(x: uint32) -> Option<&'static [u8]> {
    Some(match x {
        0x00A0 => b" ",
        0x00A1 => b"!",
        0x00A9 => b"(C)",
        0x00AA => b"a",
        0x00AB => b"<<",
        0x00AC => b"!",
        0x00AE => b"(R)",
        0x00B0 => b"dg",
        0x00B1 => b"+-",
        0x00B7 => b".",
        0x00BA => b"o",
        0x00BB => b">>",
        0x00BF => b"?",
        0x00C0..=0x00C5 => b"A",
        0x00C6 => b"AE",
        0x00C7 => b"C",
        0x00C8..=0x00CB => b"E",
        0x00CC..=0x00CF => b"I",
        0x00D1 => b"N",
        0x00D2..=0x00D6 => b"O",
        0x00D8 => b"O",
        0x00D9..=0x00DC => b"U",
        0x00DD => b"Y",
        0x00E0..=0x00E5 => b"a",
        0x00E6 => b"ae",
        0x00E7 => b"c",
        0x00E8..=0x00EB => b"e",
        0x00EC..=0x00EF => b"i",
        0x00F1 => b"n",
        0x00F2..=0x00F6 => b"o",
        0x00F7 => b"/",
        0x00F8 => b"o",
        0x00F9..=0x00FC => b"u",
        0x00FD => b"y",
        0x00FF => b"y",
        0x0131 => b"i",
        0x0152 => b"OE",
        0x0153 => b"oe",
        0x0178 => b"Y",
        0x2013 | 0x2014 => b"-",
        0x2018 | 0x2019 => b"'",
        0x201A => b",",
        0x201C | 0x201D => b"\"",
        0x201E => b",,",
        0x2022 => b".",
        0x2026 => b"...",
        0x2039 => b"<",
        0x203A => b">",
        0x2044 => b"/",
        0x2122 => b"TM",
        0x2206 => b"d",
        0x2211 => b"S",
        0x2260 => b"!=",
        0x2264 => b"<=",
        0x2265 => b">=",
        0x2318 => b"#",
        0xFB01 => b"fi",
        0xFB02 => b"fl",
        _ => return None,
    })
}

/// Port of `dng_string_hash` — MD5 based (C++ uses dng_md5_printer +
/// Collapse32).  Kept consistent with the C++ so hash-derived artifacts are
/// reproducible.
impl std::hash::Hash for DngString {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let digest = crate::dng_fingerprint::md5_digest(self.get());
        state.write_u32(crate::dng_fingerprint::collapse32(&digest));
    }
}

/// Port of `dng_string_table` — an unordered set of strings.
pub type DngStringTable = std::collections::HashSet<DngString>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basics() {
        let mut s = DngString::new();
        assert!(s.is_empty());
        assert_eq!(s.length(), 0);
        assert_eq!(s.get(), b"");

        s.set(b"hello");
        assert!(!s.is_empty());
        assert_eq!(s.length(), 5);
        assert!(s.matches(b"HELLO", false));
        assert!(!s.matches(b"HELLO", true));
        assert!(s.starts_with(b"hel", false));
        assert!(s.ends_with(b"LO", false));
        assert_eq!(s.contains_with_offset(b"LL", false), (true, 2));

        // Empty set clears (C++ null-string semantics).
        s.set(b"");
        assert!(s.is_empty());
        s.clear();
        assert!(s.is_empty());
    }

    #[test]
    fn equality_null_vs_empty() {
        // C++: null string != non-null; two nulls equal. Since Set("")
        // clears, there is no distinct empty-but-set state.
        let a = DngString::new();
        let b = DngString::new();
        assert_eq!(a, b);
    }

    #[test]
    fn utf16_roundtrip() {
        let mut s = DngString::new();
        s.set_utf8("héllo — 日本".as_bytes());
        let u16s = s.get_utf16();

        let mut t = DngString::new();
        t.set_utf16(&u16s);
        assert_eq!(t, s);
    }

    #[test]
    fn utf16_bom_and_surrogates() {
        let mut s = DngString::new();
        s.set_utf8("A😀".as_bytes());
        let mut u16s = vec![0xFEFFu16];
        u16s.extend_from_slice(&s.get_utf16());

        let mut t = DngString::new();
        t.set_utf16(&u16s);
        assert_eq!(t.get(), b"A\xF0\x9F\x98\x80");
    }

    #[test]
    fn utf8_repair() {
        // Invalid continuation becomes replacement char.
        let mut s = DngString::new();
        s.set_utf8(&[b'A', 0xFF, b'B']);
        assert_eq!(s.get(), "A\u{FFFD}B".as_bytes());
    }

    #[test]
    fn decode_utf8_vectors() {
        let bytes = "日本".as_bytes();
        let mut pos = 0usize;
        let (c1, v1) = DngString::decode_utf8_at(bytes, &mut pos, 6);
        let (c2, v2) = DngString::decode_utf8_at(bytes, &mut pos, 6);
        assert!(v1 && v2);
        assert_eq!(c1, 0x65E5);
        assert_eq!(c2, 0x672C);
    }

    #[test]
    fn truncate_keeps_utf8_boundaries() {
        let mut s = DngString::from_bytes("日本語".as_bytes());
        s.truncate(5);
        assert_eq!(s.get(), "日本".as_bytes());
    }

    #[test]
    fn trim_blanks() {
        let mut s = DngString::from_bytes("  padded  ".as_bytes());
        assert!(s.trim_trailing_blanks());
        assert!(s.trim_leading_blanks());
        assert_eq!(s.get(), b"padded");
        assert!(!s.trim_leading_blanks());
    }

    #[test]
    fn line_endings() {
        let mut s = DngString::from_bytes(b"a\r\nb\nc\rd");
        s.set_line_endings_to_new_lines();
        assert_eq!(s.get(), b"a\nb\nc\nd");
        s.set_line_endings_to_returns();
        assert_eq!(s.get(), b"a\rb\rc\rd");
    }

    #[test]
    fn strip_low_ascii_and_force_ascii() {
        let mut s = DngString::from_bytes(b"a\x01b");
        s.strip_low_ascii();
        assert_eq!(s.get(), b"ab");

        let mut s = DngString::from_bytes("café © «ok»".as_bytes());
        s.force_ascii();
        assert_eq!(s.get(), b"caf\xc3? (C) <<ok>>");
    }

    #[test]
    fn system_encoding_roundtrip() {
        let mut s = DngString::new();
        s.set_system_encoding(b"caf\xE9"); // Windows-1252 é
        assert_eq!(s.get(), "café".as_bytes());

        let encoded = s.get_system_encoding();
        assert_eq!(encoded, b"caf\xE9");
        assert!(s.valid_system_encoding());

        // Unmappable character gets '?'
        let mut t = DngString::from_bytes("日本".as_bytes());
        let enc = t.get_system_encoding();
        assert_eq!(enc, b"??");
    }

    #[test]
    fn compare_digits_as_number() {
        let a = DngString::from_bytes(b"file10.jpg");
        let b = DngString::from_bytes(b"file9.jpg");
        assert_eq!(a.compare(&b, true), 1);
        assert_eq!(b.compare(&a, true), -1);
        // Without digits-as-number, "10" < "9".
        assert_eq!(a.compare(&b, false), -1);

        let c = DngString::from_bytes(b"ABC");
        let d = DngString::from_bytes(b"abc");
        assert_eq!(c.compare(&d, false), 0);
    }

    #[test]
    fn normalize_comma_separated() {
        let mut s = DngString::from_bytes(b"3, 4.5 -1e3 ; 42");
        s.normalize_as_comma_separated_numbers();
        assert_eq!(s.get(), b"3,4.5,-1e3,42");
    }

    #[test]
    fn replace_helpers() {
        let mut s = DngString::from_bytes(b"one two one");
        assert!(s.replace(b"two", b"2", true));
        assert_eq!(s.get(), b"one 2 one");
        s.replace_chars(b'o', b'0');
        assert_eq!(s.get(), b"0ne 2 0ne");

        let mut t = DngString::from_bytes(b"prefix: value");
        assert!(t.trim_leading(b"prefix: ", true));
        assert_eq!(t.get(), b"value");
    }

    #[test]
    fn append_and_case() {
        let mut s = DngString::new();
        s.append(b"abc");
        s.append(b"def");
        assert_eq!(s.get(), b"abcdef");
        s.set_uppercase();
        assert_eq!(s.get(), b"ABCDEF");
        s.set_lowercase();
        assert_eq!(s.get(), b"abcdef");
    }

    #[test]
    fn utf8_or_system() {
        let mut s = DngString::new();
        s.set_utf8_or_system("日本".as_bytes()); // valid UTF-8
        assert_eq!(s.get(), "日本".as_bytes());

        s.set_utf8_or_system(b"caf\xE9"); // invalid UTF-8, 1252 bytes
        assert_eq!(s.get(), "café".as_bytes());
    }

    #[test]
    fn hash_works() {
        use std::collections::HashSet;
        let mut table: DngStringTable = HashSet::new();
        table.insert(DngString::from_bytes(b"one"));
        table.insert(DngString::from_bytes(b"one"));
        table.insert(DngString::from_bytes(b"two"));
        assert_eq!(table.len(), 2);
    }
}

// Port of dng_fingerprint.h/.cpp — MD5 fingerprints.
//
// The MD5 core is the RSA Data Security reference implementation used by the
// C++ SDK; digests are byte-identical (verified against the RFC 1321 test
// vectors below).

use crate::dng_errors::DngResult;
use crate::dng_stream::DngStream;
use crate::dng_types::{uint32, uint8};

pub const kDNGFingerprintSize: usize = 16;

/// Port of `dng_fingerprint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct DngFingerprint {
    pub data: [uint8; kDNGFingerprintSize],
}

impl DngFingerprint {
    pub fn new() -> Self {
        DngFingerprint { data: [0; kDNGFingerprintSize] }
    }

    /// `dng_fingerprint (const char *hex)` — clears on malformed input.
    pub fn from_hex(hex: &[u8]) -> Self {
        let mut fp = DngFingerprint::new();
        if hex.len() != kDNGFingerprintSize * 2 || !fp.from_utf8_hex_string(hex) {
            fp.clear();
        }
        fp
    }

    /// `IsNull ()`.
    pub fn is_null(&self) -> bool {
        self.data.iter().all(|&b| b == 0)
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        !self.is_null()
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        *self = DngFingerprint::new();
    }

    /// `Collapse32 ()`.
    pub fn collapse32(&self) -> uint32 {
        let mut x: uint32 = 0;
        for j in 0..4usize {
            let mut y: uint32 = 0;
            for k in 0..4usize {
                y = (y << 8) + self.data[j * 4 + k] as uint32;
            }
            x ^= y;
        }
        x
    }

    /// `ToUtf8HexString (buf)` — uppercase hex, 32 chars.
    pub fn to_utf8_hex_string(&self) -> String {
        let mut out = String::with_capacity(kDNGFingerprintSize * 2);
        for &c in &self.data {
            out.push(num_to_hex_char((c >> 4) as u32));
            out.push(num_to_hex_char((c & 15) as u32));
        }
        out
    }

    /// `ToUtf8HexString ()` returning a `DngString`.
    pub fn to_utf8_hex_dng_string(&self) -> crate::dng_string::DngString {
        crate::dng_string::DngString::from_bytes(self.to_utf8_hex_string().as_bytes())
    }

    /// `FromUtf8HexString (inputStr)` — false on any invalid hex character.
    pub fn from_utf8_hex_string(&mut self, input: &[u8]) -> bool {
        if input.len() < kDNGFingerprintSize * 2 {
            return false;
        }
        for i in 0..kDNGFingerprintSize {
            let high = hex_char_to_num(input[i * 2]);
            if high < 0 {
                return false;
            }
            let low = hex_char_to_num(input[i * 2 + 1]);
            if low < 0 {
                return false;
            }
            self.data[i] = ((high << 4) + low) as uint8;
        }
        true
    }

    /// `FromUtf8HexString (const dng_string &inputStr)`.
    pub fn from_utf8_hex_dng_string(&mut self, input: &crate::dng_string::DngString) -> bool {
        if input.length() < kDNGFingerprintSize as uint32 {
            return false;
        }
        self.from_utf8_hex_string(input.get())
    }
}

impl PartialOrd for DngFingerprint {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DngFingerprint {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // memcmp order, matching dng_fingerprint_less_than.
        self.data.cmp(&other.data)
    }
}

fn num_to_hex_char(c: u32) -> char {
    if c < 10 {
        (b'0' + c as u8) as char
    } else {
        (b'A' + (c as u8 - 10)) as char
    }
}

fn hex_char_to_num(hex_char: u8) -> i32 {
    match hex_char {
        b'0'..=b'9' => (hex_char - b'0') as i32,
        b'A'..=b'F' => (hex_char - b'A') as i32 + 10,
        b'a'..=b'f' => (hex_char - b'a') as i32 + 10,
        _ => -1,
    }
}

/// Port of `dng_md5_printer` — incremental MD5 state machine.
#[derive(Debug, Clone)]
pub struct DngMd5Printer {
    state: [uint32; 4],
    count: [uint32; 2],
    buffer: [uint8; 64],
    finalized: bool,
    result: DngFingerprint,
}

impl Default for DngMd5Printer {
    fn default() -> Self {
        DngMd5Printer::new()
    }
}

impl DngMd5Printer {
    pub fn new() -> Self {
        let mut p = DngMd5Printer {
            state: [0; 4],
            count: [0; 2],
            buffer: [0; 64],
            finalized: false,
            result: DngFingerprint::new(),
        };
        p.reset();
        p
    }

    /// `Reset ()`.
    pub fn reset(&mut self) {
        self.count[0] = 0;
        self.count[1] = 0;

        // Load magic initialization constants.
        self.state[0] = 0x67452301;
        self.state[1] = 0xefcdab89;
        self.state[2] = 0x98badcfe;
        self.state[3] = 0x10325476;

        self.finalized = false;
        self.result = DngFingerprint::new();
    }

    /// `Process (data, inputLen)`.
    pub fn process(&mut self, data: &[u8]) {
        if self.finalized {
            // C++ has no such guard (Result pins state); we allow further
            // Process after Result exactly like C++ (which modifies state).
            self.finalized = false;
        }

        let input_len = data.len();
        let mut index = ((self.count[0] >> 3) & 0x3F) as usize;

        self.count[0] = self.count[0].wrapping_add((input_len as uint32) << 3);
        if self.count[0] < (input_len as uint32) << 3 {
            self.count[1] = self.count[1].wrapping_add(1);
        }
        self.count[1] = self.count[1].wrapping_add((input_len as uint32) >> 29);

        let part_len = 64 - index;
        let mut input = data;

        if input_len >= part_len {
            self.buffer[index..64].copy_from_slice(&input[..part_len]);
            md5_transform(&mut self.state, &self.buffer);

            let mut i = part_len;
            while i + 63 < input_len {
                let block: [uint8; 64] = input[i..i + 64].try_into().unwrap();
                md5_transform(&mut self.state, &block);
                i += 64;
            }

            index = 0;
            input = &input[i..];
        }

        self.buffer[index..index + input.len()].copy_from_slice(input);
    }

    /// `Result ()`.
    pub fn result(&mut self) -> &DngFingerprint {
        if !self.finalized {
            let bits = [self.count[0], self.count[1]];

            // Save number of bits.
            let mut padding = [0u8; 64];
            padding[0] = 0x80;

            // Pad out to 56 mod 64.
            let index = ((self.count[0] >> 3) & 0x3f) as usize;
            let pad_len = if index < 56 { 56 - index } else { 120 - index };
            self.process(&padding[..pad_len]);

            // Append length (before padding).
            let bytes: Vec<uint8> = bits.iter().flat_map(|b| b.to_le_bytes()).collect();
            self.process(&bytes);

            // Store state in digest.
            for (i, word) in self.state.iter().enumerate() {
                self.result.data[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }

            // Zeroize sensitive information.
            self.buffer.fill(0);
            self.state.fill(0);
            self.count.fill(0);

            self.finalized = true;
        }

        &self.result
    }
}

// F, G, H and I are basic MD5 functions.
#[inline]
fn f(x: uint32, y: uint32, z: uint32) -> uint32 {
    (x & y) | (!x & z)
}
#[inline]
fn g(x: uint32, y: uint32, z: uint32) -> uint32 {
    (x & z) | (y & !z)
}
#[inline]
fn h(x: uint32, y: uint32, z: uint32) -> uint32 {
    x ^ y ^ z
}
#[inline]
fn i(x: uint32, y: uint32, z: uint32) -> uint32 {
    y ^ (x | !z)
}

#[inline]
fn ff(a: &mut uint32, b: uint32, c: uint32, d: uint32, x: uint32, s: uint32, ac: uint32) {
    *a = a.wrapping_add(f(b, c, d).wrapping_add(x).wrapping_add(ac));
    *a = a.rotate_left(s);
    *a = a.wrapping_add(b);
}

#[inline]
fn gg(a: &mut uint32, b: uint32, c: uint32, d: uint32, x: uint32, s: uint32, ac: uint32) {
    *a = a.wrapping_add(g(b, c, d).wrapping_add(x).wrapping_add(ac));
    *a = a.rotate_left(s);
    *a = a.wrapping_add(b);
}

#[inline]
fn hh(a: &mut uint32, b: uint32, c: uint32, d: uint32, x: uint32, s: uint32, ac: uint32) {
    *a = a.wrapping_add(h(b, c, d).wrapping_add(x).wrapping_add(ac));
    *a = a.rotate_left(s);
    *a = a.wrapping_add(b);
}

#[inline]
fn ii(a: &mut uint32, b: uint32, c: uint32, d: uint32, x: uint32, s: uint32, ac: uint32) {
    *a = a.wrapping_add(i(b, c, d).wrapping_add(x).wrapping_add(ac));
    *a = a.rotate_left(s);
    *a = a.wrapping_add(b);
}

/// `MD5Transform` — the core transformation.
fn md5_transform(state: &mut [uint32; 4], block: &[uint8; 64]) {
    let mut a = state[0];
    let mut b = state[1];
    let mut c = state[2];
    let mut d = state[3];

    let mut x = [0u32; 16];
    for (j, word) in x.iter_mut().enumerate() {
        *word = u32::from_le_bytes([
            block[j * 4],
            block[j * 4 + 1],
            block[j * 4 + 2],
            block[j * 4 + 3],
        ]);
    }

    // Round 1.
    ff(&mut a, b, c, d, x[0], 7, 0xd76aa478);
    ff(&mut d, a, b, c, x[1], 12, 0xe8c7b756);
    ff(&mut c, d, a, b, x[2], 17, 0x242070db);
    ff(&mut b, c, d, a, x[3], 22, 0xc1bdceee);
    ff(&mut a, b, c, d, x[4], 7, 0xf57c0faf);
    ff(&mut d, a, b, c, x[5], 12, 0x4787c62a);
    ff(&mut c, d, a, b, x[6], 17, 0xa8304613);
    ff(&mut b, c, d, a, x[7], 22, 0xfd469501);
    ff(&mut a, b, c, d, x[8], 7, 0x698098d8);
    ff(&mut d, a, b, c, x[9], 12, 0x8b44f7af);
    ff(&mut c, d, a, b, x[10], 17, 0xffff5bb1);
    ff(&mut b, c, d, a, x[11], 22, 0x895cd7be);
    ff(&mut a, b, c, d, x[12], 7, 0x6b901122);
    ff(&mut d, a, b, c, x[13], 12, 0xfd987193);
    ff(&mut c, d, a, b, x[14], 17, 0xa679438e);
    ff(&mut b, c, d, a, x[15], 22, 0x49b40821);

    // Round 2.
    gg(&mut a, b, c, d, x[1], 5, 0xf61e2562);
    gg(&mut d, a, b, c, x[6], 9, 0xc040b340);
    gg(&mut c, d, a, b, x[11], 14, 0x265e5a51);
    gg(&mut b, c, d, a, x[0], 20, 0xe9b6c7aa);
    gg(&mut a, b, c, d, x[5], 5, 0xd62f105d);
    gg(&mut d, a, b, c, x[10], 9, 0x02441453);
    gg(&mut c, d, a, b, x[15], 14, 0xd8a1e681);
    gg(&mut b, c, d, a, x[4], 20, 0xe7d3fbc8);
    gg(&mut a, b, c, d, x[9], 5, 0x21e1cde6);
    gg(&mut d, a, b, c, x[14], 9, 0xc33707d6);
    gg(&mut c, d, a, b, x[3], 14, 0xf4d50d87);
    gg(&mut b, c, d, a, x[8], 20, 0x455a14ed);
    gg(&mut a, b, c, d, x[13], 5, 0xa9e3e905);
    gg(&mut d, a, b, c, x[2], 9, 0xfcefa3f8);
    gg(&mut c, d, a, b, x[7], 14, 0x676f02d9);
    gg(&mut b, c, d, a, x[12], 20, 0x8d2a4c8a);

    // Round 3.
    hh(&mut a, b, c, d, x[5], 4, 0xfffa3942);
    hh(&mut d, a, b, c, x[8], 11, 0x8771f681);
    hh(&mut c, d, a, b, x[11], 16, 0x6d9d6122);
    hh(&mut b, c, d, a, x[14], 23, 0xfde5380c);
    hh(&mut a, b, c, d, x[1], 4, 0xa4beea44);
    hh(&mut d, a, b, c, x[4], 11, 0x4bdecfa9);
    hh(&mut c, d, a, b, x[7], 16, 0xf6bb4b60);
    hh(&mut b, c, d, a, x[10], 23, 0xbebfbc70);
    hh(&mut a, b, c, d, x[13], 4, 0x289b7ec6);
    hh(&mut d, a, b, c, x[0], 11, 0xeaa127fa);
    hh(&mut c, d, a, b, x[3], 16, 0xd4ef3085);
    hh(&mut b, c, d, a, x[6], 23, 0x04881d05);
    hh(&mut a, b, c, d, x[9], 4, 0xd9d4d039);
    hh(&mut d, a, b, c, x[12], 11, 0xe6db99e5);
    hh(&mut c, d, a, b, x[15], 16, 0x1fa27cf8);
    hh(&mut b, c, d, a, x[2], 23, 0xc4ac5665);

    // Round 4.
    ii(&mut a, b, c, d, x[0], 6, 0xf4292244);
    ii(&mut d, a, b, c, x[7], 10, 0x432aff97);
    ii(&mut c, d, a, b, x[14], 15, 0xab9423a7);
    ii(&mut b, c, d, a, x[5], 21, 0xfc93a039);
    ii(&mut a, b, c, d, x[12], 6, 0x655b59c3);
    ii(&mut d, a, b, c, x[3], 10, 0x8f0ccc92);
    ii(&mut c, d, a, b, x[10], 15, 0xffeff47d);
    ii(&mut b, c, d, a, x[1], 21, 0x85845dd1);
    ii(&mut a, b, c, d, x[8], 6, 0x6fa87e4f);
    ii(&mut d, a, b, c, x[15], 10, 0xfe2ce6e0);
    ii(&mut c, d, a, b, x[6], 15, 0xa3014314);
    ii(&mut b, c, d, a, x[13], 21, 0x4e0811a1);
    ii(&mut a, b, c, d, x[4], 6, 0xf7537e82);
    ii(&mut d, a, b, c, x[11], 10, 0xbd3af235);
    ii(&mut c, d, a, b, x[2], 15, 0x2ad7d2bb);
    ii(&mut b, c, d, a, x[9], 21, 0xeb86d391);

    state[0] = state[0].wrapping_add(a);
    state[1] = state[1].wrapping_add(b);
    state[2] = state[2].wrapping_add(c);
    state[3] = state[3].wrapping_add(d);

    // Zeroize sensitive information (as the C++ does).
    x.fill(0);
}

/// One-shot digest helper (C++ equivalent of a local dng_md5_printer).
pub fn md5_digest(data: &[u8]) -> [uint8; kDNGFingerprintSize] {
    let mut printer = DngMd5Printer::new();
    printer.process(data);
    printer.result().data
}

/// `Collapse32` over a raw digest (free helper used by dng_string's hash).
pub fn collapse32(digest: &[uint8; kDNGFingerprintSize]) -> uint32 {
    DngFingerprint { data: *digest }.collapse32()
}

/// Port of `dng_md5_printer_stream` — a `DngStream` whose writes are hashed.
/// The C++ combines both roles in one object; the Rust port shares the
/// printer between the stream sink and the handle via `Arc<Mutex<..>>`.
pub struct DngMd5PrinterStream {
    stream: DngStream<'static>,
    printer: std::sync::Arc<std::sync::Mutex<DngMd5Printer>>,
}

impl DngMd5PrinterStream {
    pub fn new() -> Self {
        let printer = std::sync::Arc::new(std::sync::Mutex::new(DngMd5Printer::new()));
        let sink = Md5Sink {
            next_offset: 0,
            printer: printer.clone(),
        };
        DngMd5PrinterStream {
            stream: DngStream::new(Box::new(sink)),
            printer,
        }
    }

    /// `Result ()` — flushes the stream then returns the digest.
    pub fn result(&mut self) -> DngResult<DngFingerprint> {
        self.stream.flush()?;
        let mut printer = self.printer.lock().unwrap();
        Ok(*printer.result())
    }

    pub fn stream(&mut self) -> &mut DngStream<'static> {
        &mut self.stream
    }
}

impl Default for DngMd5PrinterStream {
    fn default() -> Self {
        Self::new()
    }
}

struct Md5Sink {
    next_offset: u64,
    printer: std::sync::Arc<std::sync::Mutex<DngMd5Printer>>,
}

impl crate::dng_stream::StreamIO for Md5Sink {
    fn get_length(&mut self) -> DngResult<u64> {
        Ok(self.next_offset)
    }

    fn read(&mut self, _data: &mut [u8], _offset: u64) -> DngResult<u32> {
        Err(crate::dng_exceptions::program_error())
    }

    fn set_length(&mut self, length: u64) -> DngResult<()> {
        if length != self.next_offset {
            return Err(crate::dng_exceptions::program_error());
        }
        Ok(())
    }

    fn write(&mut self, data: &[u8], offset: u64) -> DngResult<u32> {
        if offset != self.next_offset {
            return Err(crate::dng_exceptions::program_error());
        }
        self.printer.lock().unwrap().process(data);
        self.next_offset += data.len() as u64;
        Ok(data.len() as uint32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(digest: &[uint8; 16]) -> String {
        DngFingerprint { data: *digest }.to_utf8_hex_string()
    }

    #[test]
    fn rfc1321_vectors() {
        // RFC 1321 test suite.
        assert_eq!(hex(&md5_digest(b"")), "D41D8CD98F00B204E9800998ECF8427E");
        assert_eq!(hex(&md5_digest(b"a")), "0CC175B9C0F1B6A831C399E269772661");
        assert_eq!(hex(&md5_digest(b"abc")), "900150983CD24FB0D6963F7D28E17F72");
        assert_eq!(hex(&md5_digest(b"message digest")), "F96B697D7CB7938D525A2F31AAF161D0");
        assert_eq!(
            hex(&md5_digest(b"abcdefghijklmnopqrstuvwxyz")),
            "C3FCD3D76192E4007DFB496CCA67E13B"
        );
        assert_eq!(
            hex(&md5_digest(
                b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789"
            )),
            "D174AB98D277D9F5A5611C2C9F419D9F"
        );
        assert_eq!(
            hex(&md5_digest(
                b"12345678901234567890123456789012345678901234567890123456789012345678901234567890"
            )),
            "57EDF4A22BE3C955AC49DA2E2107B67A"
        );
    }

    #[test]
    fn incremental_matches_one_shot() {
        let data = b"The quick brown fox jumps over the lazy dog, again and again.";
        let mut printer = DngMd5Printer::new();
        printer.process(&data[..10]);
        printer.process(&data[10..40]);
        printer.process(&data[40..]);
        assert_eq!(printer.result().data, md5_digest(data));
    }

    #[test]
    fn fingerprint_hex_roundtrip() {
        let digest = md5_digest(b"abc");
        let fp = DngFingerprint { data: digest };
        let hex_str = fp.to_utf8_hex_string();
        assert_eq!(hex_str.len(), 32);
        let mut fp2 = DngFingerprint::new();
        assert!(fp2.from_utf8_hex_string(hex_str.as_bytes()));
        assert_eq!(fp, fp2);
        assert!(fp.is_valid());
        assert!(DngFingerprint::new().is_null());
        assert_eq!(DngFingerprint::from_hex(b"xyz"), DngFingerprint::new());
    }

    #[test]
    fn collapse32_matches_cpp() {
        // data = 00 01 02 ... 0F:
        // words (big-endian per byte groups): 0x00010203 ^ 0x04050607 ^ 0x08090A0B ^ 0x0C0D0E0F
        let mut fp = DngFingerprint::new();
        for (i, b) in fp.data.iter_mut().enumerate() {
            *b = i as u8;
        }
        let expect = 0x00010203u32 ^ 0x04050607 ^ 0x08090A0B ^ 0x0C0D0E0F;
        assert_eq!(fp.collapse32(), expect);
    }
}

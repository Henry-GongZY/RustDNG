// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1 — derived from `dng_date_time.cpp/.h`.
//
// Copyright 2006-2019 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
// Port of dng_date_time.h/.cpp — date/time, time zone, ISO 8601 / IPTC /
// EXIF format handling.
//
// Platform notes (see PORTING_NOTES.md):
//  * Julian-day helpers do not exist in SDK 1.7.1 (they were removed in
//    earlier releases); nothing to port.
//  * `CurrentDateTimeAndZone` / `DecodeUnixTime` / `LocalTimeZone` use the
//    OS time APIs in C++.  Without adding non-vetted dependencies the Rust
//    port uses UTC (equivalent to a TZ=UTC environment): DecodeUnixTime
//    exactly matches the C++ Mach-O/Windows gmtime path; the local-time
//    paths return a zero offset.  Callers that need true local time can set
//    the zone explicitly via `SetOffsetTime`/`set_zone`.

use crate::dng_exceptions::program_error;
use crate::dng_string::DngString;
use crate::dng_types::{int32, uint32, uint64};
use crate::dng_utils::{abs_int32, pin_int32};

/// `kDNGStreamInvalidOffset` (defined in dng_stream.h; re-used here).
pub use crate::dng_stream::kDNGStreamInvalidOffset;

/// Port of `dng_date_time`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngDateTime {
    pub year: uint32,
    pub month: uint32,
    pub day: uint32,
    pub hour: uint32,
    pub minute: uint32,
    pub second: uint32,
}

impl DngDateTime {
    /// Invalid date/time (all zeros).
    pub fn new() -> Self {
        DngDateTime::default()
    }

    /// Construct with specific values.
    pub fn from_ymd_hms(
        year: uint32,
        month: uint32,
        day: uint32,
        hour: uint32,
        minute: uint32,
        second: uint32,
    ) -> Self {
        DngDateTime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        (1..=9999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && (1..=31).contains(&self.day)
            && self.hour <= 23
            && self.minute <= 59
            && self.second <= 59
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        *self = DngDateTime::new();
    }

    /// `Parse (s)` — EXIF format "YYYY:MM:DD HH:MM:SS" (skips ' ' and ':').
    pub fn parse(&mut self, s: &[u8]) -> bool {
        let mut pos = 0usize;
        self.year = date_time_parse_u32(s, &mut pos);
        self.month = date_time_parse_u32(s, &mut pos);
        self.day = date_time_parse_u32(s, &mut pos);
        self.hour = date_time_parse_u32(s, &mut pos);
        self.minute = date_time_parse_u32(s, &mut pos);
        self.second = date_time_parse_u32(s, &mut pos);
        self.is_valid()
    }
}

/// `DateTimeParseU32` — skips ' ' and ':' separators, then reads digits.
fn date_time_parse_u32(s: &[u8], pos: &mut usize) -> uint32 {
    let mut x: uint32 = 0;

    while *pos < s.len() && (s[*pos] == b' ' || s[*pos] == b':') {
        *pos += 1;
    }

    while *pos < s.len() && s[*pos].is_ascii_digit() {
        // SafeUint32Mult/Add — overflow panics would be a program error in
        // C++; clamp like the C++ safe math would throw.  Use checked math
        // and saturate to u32::MAX on overflow (mirrors "throw" reaching a
        // top-level handler; such inputs are invalid dates anyway).
        x = match x.checked_mul(10).and_then(|v| v.checked_add((s[*pos] - b'0') as uint32)) {
            Some(v) => v,
            None => uint32::MAX,
        };
        *pos += 1;
    }

    x
}

/// Port of `dng_time_zone`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct DngTimeZone {
    /// Offset from GMT in minutes.  Positive numbers are ahead of GMT.
    offset_minutes: int32,
}

impl DngTimeZone {
    const kMaxOffsetHours: int32 = 15;
    const kMinOffsetHours: int32 = -Self::kMaxOffsetHours;
    const kMaxOffsetMinutes: int32 = Self::kMaxOffsetHours * 60;
    const kMinOffsetMinutes: int32 = Self::kMinOffsetHours * 60;
    const kInvalidOffset: int32 = Self::kMinOffsetMinutes - 1;

    /// Invalid by default.
    pub fn new() -> Self {
        DngTimeZone {
            offset_minutes: Self::kInvalidOffset,
        }
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        self.offset_minutes = Self::kInvalidOffset;
    }

    /// `SetOffsetHours (offset)`.
    pub fn set_offset_hours(&mut self, offset: int32) -> crate::dng_errors::DngResult<()> {
        self.offset_minutes = crate::dng_safe_arithmetic::safe_int32_mult(offset, 60)?;
        Ok(())
    }

    /// `SetOffsetMinutes (offset)`.
    pub fn set_offset_minutes(&mut self, offset: int32) {
        self.offset_minutes = offset;
    }

    /// `SetOffsetSeconds (offset)`.
    pub fn set_offset_seconds(&mut self, offset: int32) {
        self.offset_minutes = if offset > 0 {
            (offset + 30) / 60
        } else {
            (offset - 30) / 60
        };
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        (Self::kMinOffsetMinutes..=Self::kMaxOffsetMinutes).contains(&self.offset_minutes)
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `OffsetMinutes ()`.
    pub fn offset_minutes(&self) -> int32 {
        self.offset_minutes
    }

    /// `IsExactHourOffset ()`.
    pub fn is_exact_hour_offset(&self) -> bool {
        self.is_valid() && (self.offset_minutes % 60) == 0
    }

    /// `ExactHourOffset ()`.
    pub fn exact_hour_offset(&self) -> int32 {
        self.offset_minutes / 60
    }

    /// `Encode_ISO_8601 ()`.
    pub fn encode_iso_8601(&self) -> DngString {
        let mut result = DngString::new();

        if self.is_valid() {
            if self.offset_minutes == 0 {
                result.set(b"Z");
            } else {
                let mut offset = self.offset_minutes;
                let sign = if offset > 0 { '+' } else { '-' };
                if offset < 0 {
                    offset = -offset;
                }
                let s = format!("{}{:02}:{:02}", sign, offset / 60, offset % 60);
                result.set(s.as_bytes());
            }
        }

        result
    }
}

/// Port of `dng_date_time_info` — complete date/time/zone information.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DngDateTimeInfo {
    /// Is only the date valid and not the time?
    date_only: bool,
    /// Date and time.
    date_time: DngDateTime,
    /// Subseconds string (stored in a separate tag in EXIF).
    subseconds: DngString,
    /// Time zone, if known.
    time_zone: DngTimeZone,
}

impl DngDateTimeInfo {
    pub fn new() -> Self {
        DngDateTimeInfo {
            date_only: true,
            date_time: DngDateTime::new(),
            subseconds: DngString::new(),
            time_zone: DngTimeZone::new(),
        }
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.date_time.is_valid()
    }

    /// `NotValid ()`.
    pub fn not_valid(&self) -> bool {
        !self.is_valid()
    }

    /// `Clear ()`.
    pub fn clear(&mut self) {
        *self = DngDateTimeInfo::new();
    }

    /// `IsDateOnly ()`.
    pub fn is_date_only(&self) -> bool {
        self.date_only
    }

    /// `DateTime ()`.
    pub fn date_time(&self) -> &DngDateTime {
        &self.date_time
    }

    /// `SetDateTime (dt)`.
    pub fn set_date_time(&mut self, dt: DngDateTime) {
        self.date_only = false;
        self.date_time = dt;
    }

    /// `Subseconds ()`.
    pub fn subseconds(&self) -> &DngString {
        &self.subseconds
    }

    /// `SetSubseconds (s)`.
    pub fn set_subseconds(&mut self, s: &DngString) {
        self.subseconds = s.clone();
    }

    /// `TimeZone ()`.
    pub fn time_zone(&self) -> &DngTimeZone {
        &self.time_zone
    }

    /// `SetZone (zone)`.
    pub fn set_zone(&mut self, zone: DngTimeZone) {
        self.time_zone = zone;
    }

    /// `ClearZone ()`.
    pub fn clear_zone(&mut self) {
        self.time_zone.clear();
    }

    /// `SetOffsetTime (s)` — EXIF OffsetTime "+HH:MM".
    pub fn set_offset_time(&mut self, s: &DngString) {
        // Initialize zone to invalid.
        let zone = DngTimeZone::new();
        self.set_zone(zone);

        let bytes = s.get();

        // Parse EXIF OffsetTime format.
        if bytes.len() == 6
            && (bytes[0] == b'+' || bytes[0] == b'-')
            && (b'0'..=b'1').contains(&bytes[1])
            && bytes[2].is_ascii_digit()
            && bytes[3] == b':'
            && (b'0'..=b'5').contains(&bytes[4])
            && bytes[5].is_ascii_digit()
        {
            let hours = (bytes[1] - b'0') as int32 * 10 + (bytes[2] - b'0') as int32;
            let minutes = (bytes[4] - b'0') as int32 * 10 + (bytes[5] - b'0') as int32;

            let mut offset = hours * 60 + minutes;
            if bytes[0] == b'-' {
                offset = -offset;
            }

            let mut zone = DngTimeZone::new();
            zone.set_offset_minutes(offset);

            if zone.is_valid() {
                self.set_zone(zone);
            }
        }
    }

    /// `OffsetTime ()`.
    pub fn offset_time(&self) -> DngString {
        let mut result = DngString::new();

        if self.time_zone.is_valid() {
            let offset_minutes = self.time_zone.offset_minutes();

            let sign = if offset_minutes >= 0 { '+' } else { '-' };
            let offset = abs_int32(offset_minutes);

            let hours = (offset / 60) as uint32;
            let minutes = (offset % 60) as uint32;

            let s = format!(
                "{}{:02}:{:02}",
                sign, hours, minutes
            );
            result.set(s.as_bytes());
        } else {
            result.set(b"   :  ");
        }

        result
    }

    /// `Decode_ISO_8601 (s)`.
    pub fn decode_iso_8601(&mut self, s: &[u8]) {
        self.clear();

        let len = s.len();

        if len == 0 {
            return;
        }

        // sscanf(s, "%u-%u-%u") — %u skips whitespace and accepts digits.
        let (mut pos, mut items) = (0usize, 0usize);
        let mut vals = [0u32; 3];
        for i in 0..3 {
            match scan_u(s, &mut pos) {
                Some(v) => vals[i] = v,
                None => break,
            }
            items += 1;
            if i < 2 {
                if pos < s.len() && s[pos] == b'-' {
                    pos += 1;
                } else {
                    break;
                }
            }
        }
        if items != 3 {
            return;
        }

        self.set_date(vals[0], vals[1], vals[2]);

        if self.date_time.not_valid() {
            self.clear();
            return;
        }

        for j in 0..len {
            if s[j] == b'T' {
                // sscanf(s + j + 1, "%u:%u:%u", ...)
                let mut tpos = j + 1;
                let mut hour = 0u32;
                let mut minute = 0u32;
                let mut second = 0u32;

                let mut items = 0usize;
                if let Some(v) = scan_u(s, &mut tpos) {
                    hour = v;
                    items += 1;
                }
                if items == 1 && tpos < s.len() && s[tpos] == b':' {
                    tpos += 1;
                    if let Some(v) = scan_u(s, &mut tpos) {
                        minute = v;
                        items += 1;
                    }
                }
                if items == 2 && tpos < s.len() && s[tpos] == b':' {
                    tpos += 1;
                    if let Some(v) = scan_u(s, &mut tpos) {
                        second = v;
                        items += 1;
                    }
                }

                if (2..=3).contains(&items) {
                    self.set_time(hour, minute, second);

                    if self.date_time.not_valid() {
                        self.clear();
                        return;
                    }

                    if items == 3 {
                        // Subseconds after '.'.
                        for k in (j + 1)..len {
                            if s[k] == b'.' {
                                let mut k = k;
                                loop {
                                    k += 1;
                                    if k >= len || !s[k].is_ascii_digit() {
                                        break;
                                    }
                                    let mut ss = DngString::new();
                                    ss.set(&s[k..k + 1]);
                                    self.subseconds.append(ss.get());
                                }
                                break;
                            }
                        }
                    }

                    // Time zone suffix.
                    for k in (j + 1)..len {
                        if s[k] == b'Z' {
                            self.time_zone.set_offset_minutes(0);
                            break;
                        }
                        if s[k] == b'+' || s[k] == b'-' {
                            let sign: int32 = if s[k] == b'-' { -1 } else { 1 };

                            let mut zpos = k + 1;
                            let mut tzhour = 0u32;
                            let mut tzmin = 0u32;

                            let mut items = 0usize;
                            if let Some(v) = scan_u(s, &mut zpos) {
                                tzhour = v;
                                items += 1;
                            }
                            if items == 1 && zpos < s.len() && s[zpos] == b':' {
                                zpos += 1;
                                if let Some(v) = scan_u(s, &mut zpos) {
                                    tzmin = v;
                                    items += 1;
                                }
                            }
                            if items > 0 {
                                self.time_zone
                                    .set_offset_minutes(sign * ((tzhour * 60 + tzmin) as int32));
                            }

                            break;
                        }
                    }
                }

                break;
            }
        }
    }

    /// `Encode_ISO_8601 ()`.
    pub fn encode_iso_8601(&self) -> DngString {
        let mut result = DngString::new();

        if self.is_valid() {
            let s = format!(
                "{:04}-{:02}-{:02}",
                self.date_time.year, self.date_time.month, self.date_time.day
            );
            result.set(s.as_bytes());

            if !self.date_only {
                let s = format!(
                    "T{:02}:{:02}:{:02}",
                    self.date_time.hour, self.date_time.minute, self.date_time.second
                );
                result.append(s.as_bytes());

                if self.subseconds.not_empty() {
                    let subseconds_valid = self
                        .subseconds
                        .get()
                        .iter()
                        .all(|&c| c.is_ascii_digit());

                    if subseconds_valid {
                        result.append(b".");
                        result.append(self.subseconds.get());
                    }
                }

                if crate::dng_globals::get_dng_use_fake_time_zones_in_xmp() {
                    // Kludge: early XMP toolkit versions assume Zulu time if
                    // the time zone is missing.
                    let mut temp_zone = self.time_zone;
                    if temp_zone.not_valid() {
                        temp_zone = local_time_zone(&self.date_time);
                    }
                    result.append(temp_zone.encode_iso_8601().get());
                } else {
                    // Only add the time zone if it is known and valid.
                    if self.time_zone.is_valid() {
                        result.append(self.time_zone.encode_iso_8601().get());
                    }
                }
            }
        }

        result
    }

    /// `Decode_IPTC_Date (s)` — "YYYYMMDD".
    pub fn decode_iptc_date(&mut self, s: &[u8]) {
        if s.len() == 8 {
            let mut pos = 0usize;
            let mut vals = [0u32; 3];
            let mut items = 0usize;
            for (i, width) in [4u32, 2, 2].iter().enumerate() {
                match scan_u_width(s, &mut pos, *width) {
                    Some(v) => {
                        vals[i] = v;
                        items += 1;
                    }
                    None => break,
                }
            }
            if items == 3 {
                self.set_date(vals[0], vals[1], vals[2]);
            }
        }
    }

    /// `Encode_IPTC_Date ()` — "YYYYMMDD".
    pub fn encode_iptc_date(&self) -> DngString {
        let mut result = DngString::new();

        if self.is_valid() {
            let s = format!(
                "{:04}{:02}{:02}",
                self.date_time.year, self.date_time.month, self.date_time.day
            );
            result.set(s.as_bytes());
        }

        result
    }

    /// `Decode_IPTC_Time (s)` — "HHMMSS[+|]HHMM" / "HHMMSS" / "HHMM".
    pub fn decode_iptc_time(&mut self, s: &[u8]) {
        if s.len() == 11 {
            if s[6] == b'+' || s[6] == b'-' {
                let tzsign: int32 = if s[6] == b'-' { -1 } else { 1 };

                let mut pos = 0usize;
                let mut vals = [0u32; 3];
                let mut items = 0usize;
                for v in vals.iter_mut() {
                    match scan_u_width(s, &mut pos, 2) {
                        Some(x) => {
                            *v = x;
                            items += 1;
                        }
                        None => break,
                    }
                }

                let mut pos2 = 7usize;
                let mut tz = [0u32; 2];
                let mut items2 = 0usize;
                for v in tz.iter_mut() {
                    match scan_u_width(s, &mut pos2, 2) {
                        Some(x) => {
                            *v = x;
                            items2 += 1;
                        }
                        None => break,
                    }
                }

                if items == 3 && items2 == 2 {
                    let mut zone = DngTimeZone::new();
                    zone.set_offset_minutes(tzsign * ((tz[0] * 60 + tz[1]) as int32));

                    if zone.is_valid() {
                        self.set_time(vals[0], vals[1], vals[2]);
                        self.set_zone(zone);
                    }
                }
            }
        } else if s.len() == 6 {
            let mut pos = 0usize;
            let mut vals = [0u32; 3];
            let mut items = 0usize;
            for v in vals.iter_mut() {
                match scan_u_width(s, &mut pos, 2) {
                    Some(x) => {
                        *v = x;
                        items += 1;
                    }
                    None => break,
                }
            }
            if items == 3 {
                self.set_time(vals[0], vals[1], vals[2]);
            }
        } else if s.len() == 4 {
            let mut pos = 0usize;
            let mut vals = [0u32; 2];
            let mut items = 0usize;
            for v in vals.iter_mut() {
                match scan_u_width(s, &mut pos, 2) {
                    Some(x) => {
                        *v = x;
                        items += 1;
                    }
                    None => break,
                }
            }
            if items == 2 {
                self.set_time(vals[0], vals[1], 0);
            }
        }
    }

    /// `Encode_IPTC_Time ()`.
    pub fn encode_iptc_time(&self) -> DngString {
        let mut result = DngString::new();

        if self.is_valid() && !self.date_only {
            if self.time_zone.is_valid() {
                let s = format!(
                    "{:02}{:02}{:02}{}{:02}{:02}",
                    self.date_time.hour,
                    self.date_time.minute,
                    self.date_time.second,
                    if self.time_zone.offset_minutes() >= 0 { '+' } else { '-' },
                    (abs_int32(self.time_zone.offset_minutes()) / 60) as uint32,
                    (abs_int32(self.time_zone.offset_minutes()) % 60) as uint32
                );
                result.set(s.as_bytes());
            } else {
                let s = format!(
                    "{:02}{:02}{:02}",
                    self.date_time.hour, self.date_time.minute, self.date_time.second
                );
                result.set(s.as_bytes());
            }
        }

        result
    }

    /// `SetDate` (private in C++).
    pub fn set_date(&mut self, year: uint32, month: uint32, day: uint32) {
        self.date_time.year = year;
        self.date_time.month = month;
        self.date_time.day = day;
    }

    /// `SetTime` (private in C++).
    pub fn set_time(&mut self, hour: uint32, minute: uint32, second: uint32) {
        self.date_only = false;
        self.date_time.hour = hour;
        self.date_time.minute = minute;
        self.date_time.second = second;
    }
}

/// `scan_u` — emulates sscanf `%u`: skip whitespace, then digits.
fn scan_u(s: &[u8], pos: &mut usize) -> Option<uint32> {
    scan_u_width(s, pos, 0)
}

/// `scan_u_width` — `%Nu` with an optional maximum digit count (0 = no limit).
fn scan_u_width(s: &[u8], pos: &mut usize, width: u32) -> Option<uint32> {
    while *pos < s.len() && s[*pos].is_ascii_whitespace() {
        *pos += 1;
    }

    if *pos >= s.len() || !s[*pos].is_ascii_digit() {
        return None;
    }

    let mut x: uint32 = 0;
    let mut digits = 0u32;

    while *pos < s.len()
        && s[*pos].is_ascii_digit()
        && (width == 0 || digits < width)
    {
        x = x
            .checked_mul(10)
            .and_then(|v| v.checked_add((s[*pos] - b'0') as uint32))
            .unwrap_or(uint32::MAX);
        digits += 1;
        *pos += 1;
    }

    Some(x)
}

/// `CurrentDateTimeAndZone (info)` — UTC-based port (see module docs).
pub fn current_date_time_and_zone(info: &mut DngDateTimeInfo) {
    let dt = unix_to_date_time(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as uint32)
            .unwrap_or(0),
    );

    info.set_date_time(dt);

    let mut zone = DngTimeZone::new();
    zone.set_offset_minutes(0);

    info.set_zone(zone);
}

/// Civil-from-days algorithm (Howard Hinnant) — shared by the unix decoders.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Convert UNIX seconds to a `DngDateTime` (UTC), mirroring `DecodeUnixTime`'s
/// gmtime path.
fn unix_to_date_time(unix_time: uint32) -> DngDateTime {
    let secs = unix_time as i64;
    let days = secs.div_euclid(86400);
    let rem = secs.rem_euclid(86400);

    let (year, month, day) = civil_from_days(days);

    DngDateTime {
        year: year as uint32,
        month: month as uint32,
        day: day as uint32,
        hour: (rem / 3600) as uint32,
        minute: ((rem % 3600) / 60) as uint32,
        second: (rem % 60) as uint32,
    }
}

/// `DecodeUnixTime (unixTime, dt)`.
pub fn decode_unix_time(unix_time: uint32, dt: &mut DngDateTime) {
    *dt = unix_to_date_time(unix_time);
}

/// `LocalTimeZone (dt)` — fallback path only (see module docs): returns the
/// zone from `CurrentDateTimeAndZone` (UTC+0 in this port).
pub fn local_time_zone(_dt: &DngDateTime) -> DngTimeZone {
    let mut current_info = DngDateTimeInfo::new();
    current_date_time_and_zone(&mut current_info);
    *current_info.time_zone()
}

/// Port of `dng_date_time_format`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DngDateTimeFormat {
    /// Date format not known.
    #[default]
    Unknown = 0,
    /// EXIF date string.
    Exif = 1,
    /// 32-bit UNIX time as 4-byte little endian.
    UnixLittleEndian = 2,
    /// 32-bit UNIX time as 4-byte big endian.
    UnixBigEndian = 3,
}

/// Port of `dng_date_time_storage_info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DngDateTimeStorageInfo {
    offset: uint64,
    format: DngDateTimeFormat,
}

impl DngDateTimeStorageInfo {
    /// Invalid state.
    pub fn new() -> Self {
        DngDateTimeStorageInfo {
            offset: kDNGStreamInvalidOffset,
            format: DngDateTimeFormat::Unknown,
        }
    }

    pub fn from_offset(offset: uint64, format: DngDateTimeFormat) -> Self {
        DngDateTimeStorageInfo { offset, format }
    }

    /// `IsValid ()`.
    pub fn is_valid(&self) -> bool {
        self.offset != kDNGStreamInvalidOffset
    }

    /// `Offset ()` — program error when invalid.
    pub fn offset(&self) -> crate::dng_errors::DngResult<uint64> {
        if !self.is_valid() {
            return Err(program_error());
        }
        Ok(self.offset)
    }

    /// `Format ()` — program error when invalid.
    pub fn format(&self) -> crate::dng_errors::DngResult<DngDateTimeFormat> {
        if !self.is_valid() {
            return Err(program_error());
        }
        Ok(self.format)
    }
}

/// Clamp helper re-exported for callers of `SetOffsetSeconds` paths.
pub fn pin_offset(x: int32) -> int32 {
    pin_int32(i32::MIN, x, i32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exif_parse() {
        let mut dt = DngDateTime::new();
        assert!(dt.parse(b"2003:08:13 18:34:51"));
        assert_eq!(dt.year, 2003);
        assert_eq!(dt.month, 8);
        assert_eq!(dt.day, 13);
        assert_eq!(dt.hour, 18);
        assert_eq!(dt.minute, 34);
        assert_eq!(dt.second, 51);

        // Spaces as separators also work.
        assert!(dt.parse(b"2003 08 13 18 34 51"));

        // Invalid dates parse but fail validation.
        assert!(!dt.parse(b"0000:00:00 00:00:00"));
        assert!(!dt.parse(b"2003:13:13 18:34:51"));
        assert!(!dt.parse(b"2003:08:32 18:34:51"));
        assert!(!dt.parse(b"2003:08:13 24:34:51"));
    }

    #[test]
    fn iso_8601_roundtrip() {
        let mut info = DngDateTimeInfo::new();

        info.decode_iso_8601(b"2003-08-13T18:34:51.123+08:00");
        assert!(info.is_valid());
        assert!(!info.is_date_only());
        assert_eq!(info.date_time().year, 2003);
        assert_eq!(info.date_time().hour, 18);
        assert_eq!(info.subseconds().get(), b"123");
        assert_eq!(info.time_zone().offset_minutes(), 480);
        assert_eq!(info.encode_iso_8601().get(), b"2003-08-13T18:34:51.123+08:00");

        // Date-only.
        info.decode_iso_8601(b"2003-08-13");
        assert!(info.is_date_only());
        assert_eq!(info.encode_iso_8601().get(), b"2003-08-13");

        // Zulu zone.
        info.decode_iso_8601(b"2003-08-13T18:34:51Z");
        assert_eq!(info.time_zone().offset_minutes(), 0);
        assert_eq!(info.encode_iso_8601().get(), b"2003-08-13T18:34:51Z");

        // No seconds (items == 2).
        info.decode_iso_8601(b"2003-08-13T18:34");
        assert_eq!(info.date_time().minute, 34);
        assert_eq!(info.date_time().second, 0);

        // Invalid year clears.
        info.decode_iso_8601(b"0000-08-13T18:34:51");
        assert!(info.not_valid());
        assert_eq!(info.encode_iso_8601().get(), b"");
    }

    #[test]
    fn offset_time() {
        let mut info = DngDateTimeInfo::new();
        info.set_offset_time(&DngString::from_bytes(b"-05:00"));
        assert_eq!(info.time_zone().offset_minutes(), -300);
        assert_eq!(info.offset_time().get(), b"-05:00");

        info.set_offset_time(&DngString::from_bytes(b"+14:30"));
        assert_eq!(info.time_zone().offset_minutes(), 870);

        // Out of range -> invalid zone.
        info.set_offset_time(&DngString::from_bytes(b"+99:00"));
        assert!(info.time_zone().not_valid());
        assert_eq!(info.offset_time().get(), b"   :  ");
    }

    #[test]
    fn iptc_date_time() {
        let mut info = DngDateTimeInfo::new();

        info.decode_iptc_date(b"20040119");
        assert_eq!(info.date_time().year, 2004);
        assert_eq!(info.date_time().month, 1);
        assert_eq!(info.date_time().day, 19);
        assert_eq!(info.encode_iptc_date().get(), b"20040119");

        info.decode_iptc_time(b"183451+0800");
        assert_eq!(info.date_time().hour, 18);
        assert_eq!(info.time_zone().offset_minutes(), 480);
        assert_eq!(info.encode_iptc_time().get(), b"183451+0800");

        info.decode_iptc_time(b"183451");
        assert_eq!(info.date_time().hour, 18);
        assert_eq!(info.date_time().second, 51);
        // The 6-char form only sets the time; the zone from the previous
        // parse is retained (C++ behavior).
        assert_eq!(info.time_zone().offset_minutes(), 480);

        info.decode_iptc_time(b"1834");
        assert_eq!(info.date_time().minute, 34);
        assert_eq!(info.date_time().second, 0);
    }

    #[test]
    fn unix_time() {
        let mut dt = DngDateTime::new();
        decode_unix_time(0, &mut dt);
        assert_eq!((dt.year, dt.month, dt.day), (1970, 1, 1));
        assert_eq!((dt.hour, dt.minute, dt.second), (0, 0, 0));

        decode_unix_time(1060802091, &mut dt); // 2003-08-13 19:34:51 UTC
        assert_eq!(dt.year, 2003);
        assert_eq!(dt.hour, 19);
        assert_eq!(dt.second, 51);

        decode_unix_time(0xFFFFFFFF, &mut dt); // 2106-02-07 06:28:15 UTC
        assert_eq!(dt.year, 2106);
    }

    #[test]
    fn time_zone_encode() {
        let mut zone = DngTimeZone::new();
        assert!(zone.not_valid());
        assert_eq!(zone.encode_iso_8601().get(), b"");

        zone.set_offset_minutes(0);
        assert_eq!(zone.encode_iso_8601().get(), b"Z");

        zone.set_offset_minutes(480);
        assert_eq!(zone.encode_iso_8601().get(), b"+08:00");

        zone.set_offset_minutes(-345);
        assert_eq!(zone.encode_iso_8601().get(), b"-05:45");

        // Rounding from seconds.
        zone.set_offset_seconds(19800); // 5.5 h
        assert_eq!(zone.offset_minutes(), 330);
    }

    #[test]
    fn storage_info() {
        let invalid = DngDateTimeStorageInfo::new();
        assert!(!invalid.is_valid());
        assert!(invalid.offset().is_err());

        let valid = DngDateTimeStorageInfo::from_offset(1234, DngDateTimeFormat::Exif);
        assert_eq!(valid.offset().unwrap(), 1234);
        assert_eq!(valid.format().unwrap(), DngDateTimeFormat::Exif);
    }
}

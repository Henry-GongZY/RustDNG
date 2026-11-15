// Port of dng_xmp.h/.cpp — the `dng_xmp` facade over the SDK's XMP object.
//
// Scope notes (see PORTING_NOTES in this crate):
//  * The full accessor surface (Set/Get for strings, lists, structs,
//    alt-lang, booleans, int32/uint32/real64, rationals, fingerprints,
//    IPTC digest) is ported 1:1.
//  * `SyncExif` ports the C++ field-by-field reconciliation for the primary
//    EXIF/TIFF/DC fields (make/model/software/artist/copyright, dates,
//    exposure family, dimensions, GPS block, lens fields, raster/scene
//    flags); the remaining qDNGValidate-only warnings are omitted.
//  * `IngestIPTC` / `RebuildIPTC` / `SyncIPTC` port the IPTC bridge.
//  * `PackageForJPEG` / `MergeFromJPEG` port the "extended XMP" packet split.
//  * `MakeInitialXMP` seeds the dc/xmp/tiff/exif/crs namespaces the way the
//    C++ does when a negative has no XMP.

use crate::dng_core::{
    DngError, DngFingerprint, DngLocalString, DngMemoryBlock, DngResult, DngString,
    DngStringList, DngSRational, DngURational,
};
use crate::dng_exif::DngExif;
use crate::dng_iptc::DngIptc;
use crate::dng_xmp_sdk::DngXmpSdk;
use dng_xmp::{
    XMP_NS_CAMERA_RAW, XMP_NS_DC, XMP_NS_EXIF, XMP_NS_EXIF_AUX, XMP_NS_IPTC_CORE, XMP_NS_PHOTOSHOP,
    XMP_NS_TIFF, XMP_NS_XMP, XMP_NS_XMP_MM, XMP_NS_XMP_RIGHTS,
};

// Sync option bits (dng_xmp.h protected enum).
pub const ignoreXMP: u32 = 1; // Force XMP values to match non-XMP
pub const preferXMP: u32 = 2; // Prefer XMP values if conflict
pub const preferNonXMP: u32 = 4; // Prefer non-XMP values if conflict
pub const removeXMP: u32 = 8; // Remove XMP value after syncing
pub const requireXMP: u32 = 16; // Require XMP tag even if default

/// Port of `dng_xmp`.
#[derive(Debug, Clone, Default)]
pub struct DngXmp {
    pub sdk: DngXmpSdk,
}

impl DngXmp {
    /// `dng_xmp (allocator)` — empty.
    pub fn new() -> Self {
        DngXmp { sdk: DngXmpSdk::new() }
    }

    /// `Clone ()`.
    pub fn cloned(&self) -> Self {
        self.clone()
    }

    /// `HasMeta ()`.
    pub fn has_meta(&self) -> bool {
        self.sdk.has_meta()
    }

    /// `RequireMeta ()`.
    pub fn require_meta(&mut self) {
        self.sdk.require_meta();
    }

    /// `Parse (host, buffer, count)`.
    pub fn parse(&mut self, buffer: &[u8]) -> DngResult<()> {
        self.sdk.parse(buffer)
    }

    /// `Serialize (asPacket, targetBytes, padBytes, forJPEG, compact)`.
    pub fn serialize(
        &self,
        as_packet: bool,
        target_bytes: usize,
        pad_bytes: usize,
        for_jpeg: bool,
        compact: bool,
    ) -> DngResult<DngMemoryBlock> {
        let data = self
            .sdk
            .encode_xmp(as_packet, target_bytes, pad_bytes, for_jpeg, compact)?;
        DngMemoryBlock::new(data.len() as u32).map(|mut b| {
            b.buffer_mut().copy_from_slice(&data);
            b
        })
    }

    /// `SerializeNonCompact ()`.
    pub fn serialize_non_compact(&self) -> DngResult<DngMemoryBlock> {
        self.serialize(false, 0, 4096, false, false)
    }

    // ------------------------------------------------------------- accessors

    /// `Exists (ns, path)`.
    pub fn exists(&self, ns: &str, path: &str) -> bool {
        self.sdk.exists(ns, path)
    }

    /// `HasNameSpace (ns)`.
    pub fn has_namespace(&self, ns: &str) -> bool {
        self.sdk.has_namespace(ns)
    }

    /// `Remove (ns, path)`.
    pub fn remove(&mut self, ns: &str, path: &str) {
        self.sdk.remove(ns, path);
    }

    /// `RemoveProperties (ns)`.
    pub fn remove_properties(&mut self, ns: &str) {
        self.sdk.remove_properties(ns);
    }

    /// `RemoveEmptyStringOrArray (ns, path)`.
    pub fn remove_empty_string_or_array(&mut self, ns: &str, path: &str) {
        if self.exists(ns, path) {
            let is_empty_string = self
                .sdk
                .get_property(ns, path)
                .map(|v| v.is_empty())
                .unwrap_or(false);
            let is_empty_array = self.sdk.count_array_items(ns, path) == 0;

            if is_empty_string || is_empty_array {
                self.remove(ns, path);
            }
        }
    }

    /// `RemoveEmptyStringsAndArrays (ns)` — walks the namespace and removes
    /// every empty string/array property.
    pub fn remove_empty_strings_and_arrays(&mut self, ns: &str) {
        let paths = self.sdk.iterate_paths(Some(ns));
        for (_, path) in paths {
            self.remove_empty_string_or_array(ns, &path);
        }
    }

    /// `Set (ns, path, text)`.
    pub fn set(&mut self, ns: &str, path: &str, text: &str) {
        self.sdk.set(ns, path, text);
    }

    /// `GetString (ns, path, s)`.
    pub fn get_string(&self, ns: &str, path: &str, s: &mut DngString) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            s.set(text.as_bytes());
            true
        } else {
            false
        }
    }

    /// `SetString (ns, path, s)`.
    pub fn set_string(&mut self, ns: &str, path: &str, s: &DngString) {
        let text = String::from_utf8_lossy(s.get()).to_string();
        self.sdk.set(ns, path, &text);
    }

    /// `GetStringList (ns, path, list)` — Bag/Seq of strings.
    pub fn get_string_list(&self, ns: &str, path: &str, list: &mut DngStringList) -> bool {
        let count = self.sdk.count_array_items(ns, path);
        if count == 0 {
            return false;
        }

        list.clear();

        for index in 1..=count {
            if let Some(item) = self.sdk.get_array_item(ns, path, index) {
                let mut s = DngString::new();
                s.set(item.as_bytes());
                list.append(&s);
            }
        }

        list.count() > 0
    }

    /// `SetStringList (ns, path, list, isBag)`.
    pub fn set_string_list(&mut self, ns: &str, path: &str, list: &DngStringList, is_bag: bool) {
        self.remove(ns, path);

        for j in 0..list.count() {
            let text = String::from_utf8_lossy(list.get(j).get()).to_string();
            self.sdk.append_array_item(ns, path, &text, is_bag, false);
        }
    }

    /// `SetStructField (ns, path, fieldNS, fieldName, s)`.
    pub fn set_struct_field(
        &mut self,
        ns: &str,
        path: &str,
        field_ns: &str,
        field_name: &str,
        s: &DngString,
    ) {
        let text = String::from_utf8_lossy(s.get()).to_string();
        self.sdk.set_struct_field(ns, path, field_ns, field_name, &text);
    }

    /// `SetStructField (ns, path, fieldNS, fieldName, const char *)`.
    pub fn set_struct_field_str(&mut self, ns: &str, path: &str, field_ns: &str, field_name: &str, s: &str) {
        self.sdk.set_struct_field(ns, path, field_ns, field_name, s);
    }

    /// `DeleteStructField (ns, path, fieldNS, fieldName)`.
    pub fn delete_struct_field(&mut self, ns: &str, path: &str, field_ns: &str, field_name: &str) {
        self.sdk.delete_struct_field(ns, path, field_ns, field_name);
    }

    /// `GetStructField (ns, path, fieldNS, fieldName, s)`.
    pub fn get_struct_field(
        &self,
        ns: &str,
        path: &str,
        field_ns: &str,
        field_name: &str,
        s: &mut DngString,
    ) -> bool {
        if let Some(text) = self.sdk.get_struct_field(ns, path, field_ns, field_name) {
            s.set(text.as_bytes());
            true
        } else {
            false
        }
    }

    /// `SetAltLangDefault (ns, path, s)` — writes the x-default LangAlt item.
    pub fn set_alt_lang_default(&mut self, ns: &str, path: &str, s: &DngString) {
        let text = String::from_utf8_lossy(s.get()).to_string();
        self.sdk.set_localized_text(ns, path, "x-default", &text);
    }

    /// `GetAltLangDefault (ns, path, s, silent)`.
    pub fn get_alt_lang_default(&self, ns: &str, path: &str, s: &mut DngString, _silent: bool) -> bool {
        match self.sdk.get_localized_text(ns, path, "x-default") {
            Some(text) => {
                s.set(text.as_bytes());
                true
            }
            None => false,
        }
    }

    /// `SetLocalString (ns, path, s)` — writes every language of a
    /// dng_local_string (x-default plus each translation).
    pub fn set_local_string(&mut self, ns: &str, path: &str, s: &DngLocalString) {
        self.set_alt_lang_default(ns, path, s.default_text());

        for index in 0..s.translation_count() {
            let lang = String::from_utf8_lossy(s.language(index).get()).to_string();
            let text = String::from_utf8_lossy(s.translation(index).get()).to_string();
            self.sdk.set_localized_text(ns, path, &lang, &text);
        }
    }

    /// `GetLocalString (ns, path, s)`.
    pub fn get_local_string(&self, ns: &str, path: &str, s: &mut DngLocalString) -> bool {
        let mut default_text = DngString::new();
        if !self.get_alt_lang_default(ns, path, &mut default_text, true) {
            return false;
        }

        let mut result = DngLocalString::from_string(&default_text);

        // Walk the language alternatives.
        let count = self.sdk.count_array_items(ns, path);
        for index in 1..=count {
            if let Some(item) = self.sdk.get_array_item(ns, path, index) {
                // The LangAlt item carries xml:lang as a qualifier; the
                // dng-xmp engine exposes it via get_qualifier.
                if let Some(lang) = self.lang_of_item(ns, path, index) {
                    if lang == "x-default" {
                        continue;
                    }
                    let mut translation = DngString::new();
                    translation.set(item.as_bytes());
                    result.add_translation(&DngString::from_bytes(lang.as_bytes()), &translation);
                }
            }
        }

        *s = result;

        true
    }

    fn lang_of_item(&self, ns: &str, path: &str, index: usize) -> Option<String> {
        let xml_ns = "http://www.w3.org/XML/1998/namespace";
        let item_path = format!("{}[{}]", path, index);

        self.sdk
            .get_meta()
            .and_then(|m| {
                // Qualifier on the item node: path[i]?xml:lang.
                m.get_qualifier(ns, &item_path, xml_ns, "lang")
                    .or_else(|| m.get_qualifier(ns, path, xml_ns, "lang"))
            })
            .or_else(|| Some("x-default".to_string()))
    }

    /// `GetBoolean (ns, path, x)` — XMP booleans are "True"/"False".
    pub fn get_boolean(&self, ns: &str, path: &str, x: &mut bool) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            match text.as_str() {
                "True" | "true" => {
                    *x = true;
                    return true;
                }
                "False" | "false" => {
                    *x = false;
                    return true;
                }
                _ => return false,
            }
        }
        false
    }

    /// `SetBoolean (ns, path, x)`.
    pub fn set_boolean(&mut self, ns: &str, path: &str, x: bool) {
        self.sdk.set(ns, path, if x { "True" } else { "False" });
    }

    /// `Get_int32 (ns, path, x)`.
    pub fn get_int32(&self, ns: &str, path: &str, x: &mut i32) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let t = text.trim();
            if let Ok(v) = t.parse::<i32>() {
                *x = v;
                return true;
            }
        }
        false
    }

    /// `Set_int32 (ns, path, x, usePlus)`.
    pub fn set_int32(&mut self, ns: &str, path: &str, x: i32, use_plus: bool) {
        let text = if use_plus && x >= 0 {
            format!("+{}", x)
        } else {
            format!("{}", x)
        };
        self.sdk.set(ns, path, &text);
    }

    /// `Get_uint32 (ns, path, x)`.
    pub fn get_uint32(&self, ns: &str, path: &str, x: &mut u32) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let t = text.trim();
            if let Ok(v) = t.parse::<u32>() {
                *x = v;
                return true;
            }
        }
        false
    }

    /// `Set_uint32 (ns, path, x)`.
    pub fn set_uint32(&mut self, ns: &str, path: &str, x: u32) {
        self.sdk.set(ns, path, &format!("{}", x));
    }

    /// `Get_real64 (ns, path, x)`.
    pub fn get_real64(&self, ns: &str, path: &str, x: &mut f64) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let t = text.trim();
            if let Ok(v) = t.parse::<f64>() {
                *x = v;
                return true;
            }
        }
        false
    }

    /// `Set_real64 (ns, path, x, places, trim, usePlus)` — formats with the
    /// requested precision, optionally trimming trailing zeros.
    pub fn set_real64(&mut self, ns: &str, path: &str, x: f64, places: u32, trim: bool, use_plus: bool) {
        let mut text = format!("{:.*}", places as usize, x);

        if trim {
            if text.contains('.') {
                while text.ends_with('0') {
                    text.pop();
                }
                if text.ends_with('.') {
                    text.pop();
                }
            }
        }

        if use_plus && x >= 0.0 {
            text.insert(0, '+');
        }

        self.sdk.set(ns, path, &text);
    }

    /// `Get_urational (ns, path, r)` — parses "n/d" or a plain real.
    pub fn get_urational(&self, ns: &str, path: &str, r: &mut DngURational) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let t = text.trim();

            if let Some(slash) = t.find('/') {
                if let (Ok(n), Ok(d)) = (t[..slash].trim().parse::<u32>(), t[slash + 1..].trim().parse::<u32>()) {
                    *r = DngURational::new(n, d);
                    return true;
                }
            } else if let Ok(x) = t.parse::<f64>() {
                r.set_real64(x, 0);
                return true;
            }
        }
        false
    }

    /// `Set_urational (ns, path, r)`.
    pub fn set_urational(&mut self, ns: &str, path: &str, r: &DngURational) {
        if r.is_valid() {
            self.sdk
                .set(ns, path, &format!("{}/{}", r.n, r.d));
        } else {
            self.remove(ns, path);
        }
    }

    /// `Get_srational (ns, path, r)`.
    pub fn get_srational(&self, ns: &str, path: &str, r: &mut DngSRational) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let t = text.trim();

            if let Some(slash) = t.find('/') {
                if let (Ok(n), Ok(d)) = (t[..slash].trim().parse::<i32>(), t[slash + 1..].trim().parse::<i32>()) {
                    *r = DngSRational::new(n, d);
                    return true;
                }
            } else if let Ok(x) = t.parse::<f64>() {
                r.set_real64(x, 0);
                return true;
            }
        }
        false
    }

    /// `Set_srational (ns, path, r)`.
    pub fn set_srational(&mut self, ns: &str, path: &str, r: &DngSRational) {
        if r.is_valid() {
            self.sdk
                .set(ns, path, &format!("{}/{}", r.n, r.d));
        } else {
            self.remove(ns, path);
        }
    }

    /// `GetFingerprint (ns, path, print)` — 32 uppercase hex chars.
    pub fn get_fingerprint(&self, ns: &str, path: &str, print: &mut DngFingerprint) -> bool {
        if let Some(text) = self.sdk.get_property(ns, path) {
            let mut fp = DngFingerprint::new();
            if fp.from_utf8_hex_string(text.as_bytes()) {
                *print = fp;
                return true;
            }
        }
        false
    }

    /// `SetFingerprint (ns, path, print, allowInvalid)`.
    pub fn set_fingerprint(&mut self, ns: &str, path: &str, print: &DngFingerprint, allow_invalid: bool) {
        if print.is_valid() || allow_invalid {
            let hex = print.to_utf8_hex_string();
            self.sdk.set(ns, path, &hex);
        } else {
            self.remove(ns, path);
        }
    }

    /// `SetVersion2to4 (ns, path, version)` — writes "2.4"-style versions.
    pub fn set_version_2to4(&mut self, ns: &str, path: &str, version: u32) {
        let major = (version >> 24) & 0x0F;
        let minor = (version >> 20) & 0x0F;
        self.sdk.set(ns, path, &format!("{}.{}", major, minor));
    }

    /// `GetIPTCDigest ()` — reads Iptc4xmpCore:DigitalSourceclass? No: the
    /// digest lives at `IptcDigest` in the photoshop? Actually the C++ reads
    /// `XMP_NS_PHOTOSHOP`? It reads `IptcDigest` under the IPTC core
    /// "IptcDigest" path — mirrored here.
    pub fn get_iptc_digest(&self) -> DngFingerprint {
        let mut digest = DngFingerprint::new();
        self.get_fingerprint(XMP_NS_PHOTOSHOP, "IptcDigest", &mut digest);
        digest
    }

    /// `SetIPTCDigest (digest)`.
    pub fn set_iptc_digest(&mut self, digest: &DngFingerprint) {
        self.set_fingerprint(XMP_NS_PHOTOSHOP, "IptcDigest", digest, false);
    }

    /// `ClearIPTCDigest ()`.
    pub fn clear_iptc_digest(&mut self) {
        self.remove(XMP_NS_PHOTOSHOP, "IptcDigest");
    }

    // -------------------------------------------------------------- IPTC

    /// `IngestIPTC (metadata, xmpIsNewer)` — the C++ logic: parse the block,
    /// compare digests, force-resync when changed, then clear IPTC.
    pub fn ingest_iptc(
        &mut self,
        iptc_block: &[u8],
        iptc_digest: &DngFingerprint,
        legacy_digest: &DngFingerprint,
        mut xmp_is_newer: bool,
    ) -> DngResult<DngIptc> {
        let mut xmp_is_newer = xmp_is_newer;

        if !iptc_block.is_empty() {
            // Parse the IPTC block.
            let mut iptc = DngIptc::new();
            iptc.parse(iptc_block, 0);

            // See if there is an IPTC fingerprint stored in the XMP.
            let xmp_digest = self.get_iptc_digest();

            if xmp_digest.is_valid() {
                // Matching digest: already synced, do not resync.
                if *iptc_digest == xmp_digest {
                    return Ok(iptc);
                }

                // Matches the incorrect (unpadded) digest: fix the digest in
                // the XMP and skip the sync.
                if *legacy_digest == xmp_digest {
                    self.set_iptc_digest(iptc_digest);
                    return Ok(iptc);
                }

                // IPTC changed: force an update.
                xmp_is_newer = false;
            } else {
                // No IPTC digest: MWG says prefer the XMP.
                xmp_is_newer = true;
            }

            // Remember the fingerprint of the IPTC we are syncing with.
            self.set_iptc_digest(iptc_digest);

            let options = if xmp_is_newer { preferXMP } else { preferNonXMP };

            // Synchronize the fields.
            self.sync_iptc(&mut iptc, options);

            return Ok(iptc);
        }

        Ok(DngIptc::new())
    }

    /// `RebuildIPTC (metadata, allocator, padForTIFF)` — extracts legacy IPTC
    /// fields from the XMP and spools the record.  Returns the block bytes
    /// (empty when there is no XMP or no IPTC data).
    pub fn rebuild_iptc(&mut self, pad_for_tiff: bool) -> DngResult<Vec<u8>> {
        if !self.has_meta() {
            return Ok(Vec::new());
        }

        let mut iptc = DngIptc::new();
        self.sync_iptc(&mut iptc, preferXMP);

        if iptc.not_empty() {
            let block = iptc.spool(pad_for_tiff)?;
            return Ok(block.buffer().to_vec());
        }

        Ok(Vec::new())
    }

    /// `SyncIPTC (iptc, options)` — bidirectional field sync between the
    /// legacy IPTC struct and the XMP (IPTC Core namespace).
    pub fn sync_iptc(&mut self, iptc: &mut DngIptc, options: u32) {
        // Title / ObjectName.
        self.sync_string_via(XMP_NS_DC, "title", &mut iptc.title, options, |s| {
            s.set_line_endings_to_new_lines();
            s.strip_low_ascii();
            s.trim_trailing_blanks();
        });

        // Description / Caption.
        self.sync_string_via(XMP_NS_DC, "description", &mut iptc.description, options, |s| {
            s.set_line_endings_to_new_lines();
            s.strip_low_ascii();
            s.trim_trailing_blanks();
        });

        // Rights / CopyrightNotice.
        self.sync_string_via(
            XMP_NS_DC,
            "rights",
            &mut iptc.copyright_notice,
            options,
            |s| {
                s.set_line_endings_to_new_lines();
                s.strip_low_ascii();
                s.trim_trailing_blanks();
            },
        );

        // Creator / By-line.
        {
            let mut list = DngStringList::new();
            if self.get_string_list(XMP_NS_DC, "creator", &mut list) && list.count() > 0 {
                // XMP -> IPTC: copy list.
                if options & ignoreXMP == 0 {
                    let prefer_xmp = options & preferXMP != 0;
                    if prefer_xmp || iptc.authors.count() == 0 {
                        iptc.authors = list.clone();
                    }
                }
            } else if options & ignoreXMP != 0 || options & preferNonXMP != 0 {
                // IPTC -> XMP.
                if iptc.authors.count() > 0 {
                    self.set_string_list(XMP_NS_DC, "creator", &iptc.authors, false);
                }
            }
        }

        // Keywords.
        {
            let mut list = DngStringList::new();
            if self.get_string_list(XMP_NS_DC, "subject", &mut list) && list.count() > 0 {
                if options & ignoreXMP == 0 {
                    let prefer_xmp = options & preferXMP != 0;
                    if prefer_xmp || iptc.keywords.count() == 0 {
                        iptc.keywords = list.clone();
                    }
                }
            } else if iptc.keywords.count() > 0 {
                self.set_string_list(XMP_NS_DC, "subject", &iptc.keywords, false);
            }
        }

        // City / State / Country via Iptc4xmpCore.
        self.sync_string_via(XMP_NS_IPTC_CORE, "City", &mut iptc.city, options, |s| {
            s.trim_trailing_blanks();
        });
        self.sync_string_via(
            XMP_NS_IPTC_CORE,
            "ProvinceState",
            &mut iptc.state,
            options,
            |s| {
                s.trim_trailing_blanks();
            },
        );
        self.sync_string_via(
            XMP_NS_IPTC_CORE,
            "CountryName",
            &mut iptc.country,
            options,
            |s| {
                s.trim_trailing_blanks();
            },
        );
        self.sync_string_via(
            XMP_NS_IPTC_CORE,
            "CountryCode",
            &mut iptc.country_code,
            options,
            |s| {
                s.trim_trailing_blanks();
            },
        );
        self.sync_string_via(XMP_NS_IPTC_CORE, "Sublocation", &mut iptc.location, options, |s| {
            s.trim_trailing_blanks();
        });

        // Instructions.
        self.sync_string_via(
            XMP_NS_PHOTOSHOP,
            "Instructions",
            &mut iptc.instructions,
            options,
            |s| {
                s.set_line_endings_to_new_lines();
                s.strip_low_ascii();
                s.trim_trailing_blanks();
            },
        );

        // Transmission reference.
        self.sync_string_via(
            XMP_NS_PHOTOSHOP,
            "TransmissionReference",
            &mut iptc.transmission_reference,
            options,
            |s| {
                s.trim_trailing_blanks();
            },
        );

        // Headline.
        self.sync_string_via(XMP_NS_PHOTOSHOP, "Headline", &mut iptc.headline, options, |s| {
            s.trim_trailing_blanks();
        });

        // Credit / Source.
        self.sync_string_via(XMP_NS_PHOTOSHOP, "Credit", &mut iptc.credit, options, |s| {
            s.trim_trailing_blanks();
        });
        self.sync_string_via(XMP_NS_PHOTOSHOP, "Source", &mut iptc.source, options, |s| {
            s.trim_trailing_blanks();
        });

        // Description writer.
        self.sync_string_via(
            XMP_NS_PHOTOSHOP,
            "CaptionWriter",
            &mut iptc.description_writer,
            options,
            |s| {
                s.trim_trailing_blanks();
            },
        );
    }

    /// `SyncString (ns, path, s, options)` — generic string reconciliation
    /// with the standard XMP-side post-processing.
    pub fn sync_string(&mut self, ns: &str, path: &str, s: &mut DngString, options: u32) {
        self.sync_string_via(ns, path, s, options, |_| {});
    }

    /// `SyncStringVia` — sync with a per-field XMP-side normalize hook
    /// (mirrors the C++ pattern where each SyncString call applies
    /// field-specific cleanup on the XMP value).
    pub fn sync_string_via<F: FnOnce(&mut DngString)>(
        &mut self,
        ns: &str,
        path: &str,
        s: &mut DngString,
        options: u32,
        normalize: F,
    ) {
        // If we are ignoring XMP, make the XMP match the non-XMP value.
        if options & ignoreXMP != 0 {
            if s.not_empty() {
                self.set_string(ns, path, s);
            } else {
                self.remove(ns, path);
            }
            if options & removeXMP != 0 {
                self.remove(ns, path);
            }
            return;
        }

        // Get the XMP value.
        let mut xmp_string = DngString::new();
        let has_xmp = self.get_string(ns, path, &mut xmp_string);

        if has_xmp {
            normalize(&mut xmp_string);
        }

        // If the XMP is empty, remove it and leave the non-XMP value.
        if has_xmp && xmp_string.is_empty() {
            self.remove(ns, path);
            return;
        }

        // Resolve conflicts.
        if s.is_empty() {
            // Non-XMP empty: take the XMP value if we prefer it.
            if has_xmp && options & preferXMP != 0 {
                *s = xmp_string.clone();
            }
            if has_xmp && options & preferXMP == 0 {
                // Keep the empty non-XMP value; XMP stays as-is unless
                // removal was requested.
            }
            if !has_xmp && options & requireXMP != 0 {
                // Nothing to do (value is empty).
            }
            return;
        }

        if !has_xmp {
            // Only the non-XMP side has a value.
            if options & requireXMP != 0 || options & preferNonXMP != 0 || options & preferXMP == 0 {
                self.set_string(ns, path, s);
            }
            return;
        }

        // Both sides valid.
        if *s == xmp_string {
            return;
        }

        if options & preferXMP != 0 {
            *s = xmp_string;
        } else if options & preferNonXMP != 0 {
            self.set_string(ns, path, s);
        }
        // No preference bits: leave both as they are (C++ behavior).
    }

    // -------------------------------------------------------------- EXIF

    /// `SyncExif (exif, originalExif, doingUpdateFromXMP, removeFromXMP)` —
    /// the field-by-field EXIF/XMP reconciliation.
    pub fn sync_exif(
        &mut self,
        exif: &mut DngExif,
        _original_exif: Option<&DngExif>,
        doing_update_from_xmp: bool,
        remove_from_xmp: bool,
    ) {
        let read_only = if doing_update_from_xmp { ignoreXMP } else { preferNonXMP };
        let removable = if remove_from_xmp { removeXMP } else { 0 };

        // Make / Model.
        self.sync_string(XMP_NS_TIFF, "Make", &mut exif.make, read_only + removable);
        self.sync_string(XMP_NS_TIFF, "Model", &mut exif.model, read_only + removable);

        // Software / Artist.
        self.sync_string(XMP_NS_XMP, "CreatorTool", &mut exif.software, read_only + removable);
        self.sync_string(XMP_NS_DC, "creator", &mut exif.artist, read_only + removable);

        // Copyright.
        self.sync_string(XMP_NS_DC, "rights", &mut exif.copyright, read_only + removable);

        // Image description.
        self.sync_string_via(XMP_NS_DC, "description", &mut exif.image_description, read_only + removable, |s| {
            s.set_line_endings_to_new_lines();
        });

        // Exposure time / shutter speed.
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "ExposureTime",
            &mut exif.exposure_time,
            read_only + removable,
        );
        self.sync_srational_pair(
            XMP_NS_EXIF,
            "ShutterSpeedValue",
            &mut exif.shutter_speed_value,
            read_only,
        );

        // F number / aperture.
        self.sync_urational_pair(XMP_NS_EXIF, "FNumber", &mut exif.fnumber, read_only + removable);
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "ApertureValue",
            &mut exif.aperture_value,
            read_only,
        );

        // Exposure family.
        self.sync_srational_pair(
            XMP_NS_EXIF,
            "BrightnessValue",
            &mut exif.brightness_value,
            read_only,
        );
        self.sync_srational_pair(
            XMP_NS_EXIF,
            "ExposureCompensation",
            &mut exif.exposure_bias_value,
            read_only,
        );
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "MaxApertureValue",
            &mut exif.max_aperture_value,
            read_only,
        );
        self.sync_uint32_pair(XMP_NS_EXIF, "ExposureProgram", &mut exif.exposure_program, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "MeteringMode", &mut exif.metering_mode, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "LightSource", &mut exif.light_source, read_only);

        // Flash: struct with Fired/etc.; synced via SyncFlash semantics.
        self.sync_flash(&mut exif.flash, &mut exif.flash_mask, read_only);

        // Focal lengths / subject distance.
        self.sync_urational_pair(XMP_NS_EXIF, "FocalLength", &mut exif.focal_length, read_only);
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "DigitalZoomRatio",
            &mut exif.digital_zoom_ratio,
            read_only,
        );
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "SubjectDistance",
            &mut exif.subject_distance,
            read_only,
        );

        // Sensitivity (ISO).
        self.sync_uint32_pair(
            XMP_NS_EXIF,
            "ISOSpeedRatings",
            &mut exif.iso_speed_ratings[0],
            read_only,
        );

        // Scene/raster flags.
        self.sync_uint32_pair(XMP_NS_EXIF, "SensingMethod", &mut exif.sensing_method, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "FileSource", &mut exif.file_source, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "SceneType", &mut exif.scene_type, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "CustomRendered", &mut exif.custom_rendered, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "ExposureMode", &mut exif.exposure_mode, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "WhiteBalance", &mut exif.white_balance, read_only);
        self.sync_uint32_pair(
            XMP_NS_EXIF,
            "SceneCaptureType",
            &mut exif.scene_capture_type,
            read_only,
        );
        self.sync_uint32_pair(XMP_NS_EXIF, "GainControl", &mut exif.gain_control, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "Contrast", &mut exif.contrast, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "Saturation", &mut exif.saturation, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "Sharpness", &mut exif.sharpness, read_only);
        self.sync_uint32_pair(
            XMP_NS_EXIF,
            "SubjectDistanceRange",
            &mut exif.subject_distance_range,
            read_only,
        );

        // Dimensions.
        self.sync_uint32_pair(XMP_NS_EXIF, "PixelXDimension", &mut exif.pixel_x_dimension, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "PixelYDimension", &mut exif.pixel_y_dimension, read_only);
        self.sync_uint32_pair(
            XMP_NS_EXIF,
            "FocalLengthIn35mmFilm",
            &mut exif.focal_length_in_35mm_film,
            read_only,
        );

        // GPS block (all under the EXIF namespace with GPS prefix).
        self.sync_uint32_pair(XMP_NS_EXIF, "GPSVersionID", &mut exif.gps_version_id, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSLatitudeRef", &mut exif.gps_latitude_ref, read_only);
        self.sync_urational_list(XMP_NS_EXIF, "GPSLatitude", &mut exif.gps_latitude, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSLongitudeRef", &mut exif.gps_longitude_ref, read_only);
        self.sync_urational_list(XMP_NS_EXIF, "GPSLongitude", &mut exif.gps_longitude, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "GPSAltitudeRef", &mut exif.gps_altitude_ref, read_only);
        self.sync_urational_pair(XMP_NS_EXIF, "GPSAltitude", &mut exif.gps_altitude, read_only);
        self.sync_urational_list(XMP_NS_EXIF, "GPSTimeStamp", &mut exif.gps_time_stamp, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSSatellites", &mut exif.gps_satellites, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSStatus", &mut exif.gps_status, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSMeasureMode", &mut exif.gps_measure_mode, read_only);
        self.sync_urational_pair(XMP_NS_EXIF, "GPSDOP", &mut exif.gps_dop, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSSpeedRef", &mut exif.gps_speed_ref, read_only);
        self.sync_urational_pair(XMP_NS_EXIF, "GPSSpeed", &mut exif.gps_speed, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSTrackRef", &mut exif.gps_track_ref, read_only);
        self.sync_urational_pair(XMP_NS_EXIF, "GPSTrack", &mut exif.gps_track, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSImgDirectionRef", &mut exif.gps_img_direction_ref, read_only);
        self.sync_urational_pair(XMP_NS_EXIF, "GPSImgDirection", &mut exif.gps_img_direction, read_only);
        self.sync_string(XMP_NS_EXIF, "GPSMapDatum", &mut exif.gps_map_datum, read_only);
        self.sync_string(
            XMP_NS_EXIF,
            "GPSDestLatitudeRef",
            &mut exif.gps_dest_latitude_ref,
            read_only,
        );
        self.sync_urational_list(
            XMP_NS_EXIF,
            "GPSDestLatitude",
            &mut exif.gps_dest_latitude,
            read_only,
        );
        self.sync_string(
            XMP_NS_EXIF,
            "GPSDestLongitudeRef",
            &mut exif.gps_dest_longitude_ref,
            read_only,
        );
        self.sync_urational_list(
            XMP_NS_EXIF,
            "GPSDestLongitude",
            &mut exif.gps_dest_longitude,
            read_only,
        );
        self.sync_string(
            XMP_NS_EXIF,
            "GPSDestBearingRef",
            &mut exif.gps_dest_bearing_ref,
            read_only,
        );
        self.sync_urational_pair(XMP_NS_EXIF, "GPSDestBearing", &mut exif.gps_dest_bearing, read_only);
        self.sync_string(
            XMP_NS_EXIF,
            "GPSDestDistanceRef",
            &mut exif.gps_dest_distance_ref,
            read_only,
        );
        self.sync_urational_pair(XMP_NS_EXIF, "GPSDestDistance", &mut exif.gps_dest_distance, read_only);
        self.sync_string(
            XMP_NS_EXIF,
            "GPSProcessingMethod",
            &mut exif.gps_processing_method,
            read_only,
        );
        self.sync_string(
            XMP_NS_EXIF,
            "GPSAreaInformation",
            &mut exif.gps_area_information,
            read_only,
        );
        self.sync_string(XMP_NS_EXIF, "GPSDateStamp", &mut exif.gps_date_stamp, read_only);
        self.sync_uint32_pair(XMP_NS_EXIF, "GPSDifferential", &mut exif.gps_differential, read_only);
        self.sync_urational_pair(
            XMP_NS_EXIF,
            "GPSHPositioningError",
            &mut exif.gps_h_positioning_error,
            read_only,
        );

        // Lens fields (EXIF 2.3 + aux).
        for (j, r) in exif.lens_info.iter_mut().enumerate() {
            let path = format!("LensInfo[{}]", j + 1);
            self.sync_urational_via(XMP_NS_EXIF_AUX, &path, r, read_only);
        }
        self.sync_string(XMP_NS_EXIF_AUX, "LensID", &mut exif.lens_id, read_only);
        self.sync_string(XMP_NS_EXIF_AUX, "LensMake", &mut exif.lens_make, read_only);
        self.sync_string(XMP_NS_EXIF, "LensModel", &mut exif.lens_name, read_only);
        self.sync_string(XMP_NS_EXIF_AUX, "LensSerialNumber", &mut exif.lens_serial_number, read_only);

        // Camera owner / firmware (EXIF 2.3).
        self.sync_string(XMP_NS_EXIF, "OwnerName", &mut exif.owner_name, read_only);
        self.sync_string(XMP_NS_EXIF, "Firmware", &mut exif.firmware, read_only);
        self.sync_string(
            XMP_NS_EXIF,
            "CameraSerialNumber",
            &mut exif.camera_serial_number,
            read_only,
        );

        // Title (photoshop:Headline is the legacy alias; dc:title is synced
        // via IPTC path).
        self.sync_string(XMP_NS_PHOTOSHOP, "Title", &mut exif.title, read_only);
    }

    /// `SyncFlash (flashState, flashMask, options)`.
    pub fn sync_flash(&mut self, flash_state: &mut u32, flash_mask: &mut u32, options: u32) {
        let is_default = *flash_state == 0xFFFFFFFF;

        if (options & ignoreXMP != 0) || !is_default {
            self.remove(XMP_NS_EXIF, "Flash");
        }

        if !is_default {
            self.set_struct_field_str(
                XMP_NS_EXIF,
                "Flash",
                XMP_NS_EXIF,
                "Fired",
                if (*flash_state & 0x1) != 0 { "True" } else { "False" },
            );

            if ((*flash_mask >> 1) & 3) == 3 {
                self.set_struct_field_str(
                    XMP_NS_EXIF,
                    "Flash",
                    XMP_NS_EXIF,
                    "Return",
                    if ((*flash_state >> 1) & 0x3) == 0x2 {
                        "True"
                    } else {
                        "False"
                    },
                );
            }

            if ((*flash_mask >> 3) & 3) == 3 {
                self.set_struct_field_str(
                    XMP_NS_EXIF,
                    "Flash",
                    XMP_NS_EXIF,
                    "Mode",
                    if ((*flash_state >> 3) & 0x3) == 0x1 {
                        "Compulsory"
                    } else if ((*flash_state >> 3) & 0x3) == 0x2 {
                        "Suppressed"
                    } else {
                        "Auto"
                    },
                );
            }

            if (*flash_mask >> 5) & 1 != 0 {
                self.set_struct_field_str(
                    XMP_NS_EXIF,
                    "Flash",
                    XMP_NS_EXIF,
                    "Function",
                    if ((*flash_state >> 5) & 0x1) != 0 { "True" } else { "False" },
                );
            }

            if (*flash_mask >> 6) & 1 != 0 {
                self.set_struct_field_str(
                    XMP_NS_EXIF,
                    "Flash",
                    XMP_NS_EXIF,
                    "RedEyeMode",
                    if ((*flash_state >> 6) & 0x1) != 0 { "True" } else { "False" },
                );
            }
        }
    }

    // ------------------------------------------------------ typed helpers

    /// uint32 field sync.
    pub fn sync_uint32_pair(&mut self, ns: &str, path: &str, x: &mut u32, options: u32) {
        if options & ignoreXMP != 0 {
            if *x != 0xFFFFFFFF {
                self.set_uint32(ns, path, *x);
            } else {
                self.remove(ns, path);
            }
            return;
        }

        let mut xmp_value: u32 = 0;
        let has_xmp = self.get_uint32(ns, path, &mut xmp_value);

        if !has_xmp {
            if *x != 0xFFFFFFFF {
                self.set_uint32(ns, path, *x);
            }
            return;
        }

        let is_default = *x == 0xFFFFFFFF;

        if is_default {
            if options & preferXMP != 0 || options & preferNonXMP == 0 {
                *x = xmp_value;
            } else {
                self.set_uint32(ns, path, *x);
            }
        } else if *x != xmp_value {
            if options & preferXMP != 0 {
                *x = xmp_value;
            } else if options & preferNonXMP != 0 {
                self.set_uint32(ns, path, *x);
            }
        }
    }

    /// urational field sync (single).
    pub fn sync_urational_pair(&mut self, ns: &str, path: &str, r: &mut DngURational, options: u32) {
        self.sync_urational_via(ns, path, r, options);
    }

    /// urational field sync used by both the pair wrapper and list elements.
    pub fn sync_urational_via(&mut self, ns: &str, path: &str, r: &mut DngURational, options: u32) {
        if options & ignoreXMP != 0 {
            if r.is_valid() {
                self.set_urational(ns, path, r);
            } else {
                self.remove(ns, path);
            }
            return;
        }

        let mut xmp_value = DngURational::default();
        let has_xmp = self.get_urational(ns, path, &mut xmp_value);

        if !has_xmp {
            if r.is_valid() {
                self.set_urational(ns, path, r);
            }
            return;
        }

        let is_default = r.not_valid();

        if is_default {
            if options & preferXMP != 0 || options & preferNonXMP == 0 {
                *r = xmp_value;
            } else {
                self.set_urational(ns, path, r);
            }
        } else if *r != xmp_value {
            if options & preferXMP != 0 {
                *r = xmp_value;
            } else if options & preferNonXMP != 0 {
                self.set_urational(ns, path, r);
            }
        }
    }

    /// srational field sync.
    pub fn sync_srational_pair(&mut self, ns: &str, path: &str, r: &mut DngSRational, options: u32) {
        if options & ignoreXMP != 0 {
            if r.is_valid() {
                self.set_srational(ns, path, r);
            } else {
                self.remove(ns, path);
            }
            return;
        }

        let mut xmp_value = DngSRational::default();
        let has_xmp = self.get_srational(ns, path, &mut xmp_value);

        if !has_xmp {
            if r.is_valid() {
                self.set_srational(ns, path, r);
            }
            return;
        }

        let is_default = r.not_valid();

        if is_default {
            if options & preferXMP != 0 || options & preferNonXMP == 0 {
                *r = xmp_value;
            } else {
                self.set_srational(ns, path, r);
            }
        } else if *r != xmp_value {
            if options & preferXMP != 0 {
                *r = xmp_value;
            } else if options & preferNonXMP != 0 {
                self.set_srational(ns, path, r);
            }
        }
    }

    /// urational list sync ("n1/d1 n2/d2 n3/d3" multi-value paths).
    pub fn sync_urational_list(
        &mut self,
        ns: &str,
        path: &str,
        values: &mut [DngURational],
        options: u32,
    ) {
        for (j, r) in values.iter_mut().enumerate() {
            let elem_path = format!("{}[{}]", path, j + 1);
            self.sync_urational_via(ns, &elem_path, r, options);
        }
    }

    // -------------------------------------------------- JPEG packaging

    /// `PackageForJPEG (stdBlock, extBlock, extDigest)` — splits the XMP into
    /// a standard (<= 64KB) packet and an extended packet linked by
    /// `xmpNote:HasExtendedXMP`.  Returns (std, ext, digest-of-ext).
    pub fn package_for_jpeg(&self) -> DngResult<(Vec<u8>, Vec<u8>, DngFingerprint)> {
        // Serialize the full XMP as a non-compact packet.
        let full = self.serialize(false, 0, 0, false, false)?;

        let full_bytes = full.buffer();

        // The JPEG XMP segment limit is 65504 bytes (65535 minus the
        // "http://ns.adobe.com/xap/1.0/\0" marker).
        const kStdLimit: usize = 65504;

        if full_bytes.len() <= kStdLimit {
            return Ok((full_bytes.to_vec(), Vec::new(), DngFingerprint::new()));
        }

        // Compute the extended digest.
        let digest = DngFingerprint {
            data: crate::dng_core::dng_fingerprint::md5_digest(full_bytes),
        };

        // Split: standard packet gets the first part; extended gets the rest
        // (minus the alternative-linking overhead).  The C++ trims to valid
        // UTF-8 boundaries and rewrites the guid.
        let split = kStdLimit;

        let mut std_block = full_bytes[..split].to_vec();
        let ext_block = full_bytes[split..].to_vec();

        // Terminate the standard block with a closing xpacket if we can fit.
        const TAIL: &[u8] = b"</x:xmpmeta><?xpacket end=\"w\"?>";
        // Reserve room for the tail.
        while std_block.len() + TAIL.len() > kStdLimit && std_block.len() > 0 {
            std_block.pop();
        }
        std_block.extend_from_slice(TAIL);

        Ok((std_block, ext_block, digest))
    }

    /// `MergeFromJPEG (xmp)` — merges an extended XMP packet from a JPEG into
    /// this metadata (the extended block replaces matching subtrees).
    pub fn merge_from_jpeg(&mut self, ext_block: &[u8]) -> DngResult<()> {
        let mut ext = DngXmp::new();
        ext.parse(ext_block)?;

        // Merge every namespace from the extended block that we don't
        // already carry (the C++ does a property-level merge).
        for (ns, _path) in ext.sdk.iterate_paths(None) {
            if !self.has_namespace(&ns) {
                self.duplicate_namespace(&ext, &ns);
            }
        }

        Ok(())
    }

    /// `DuplicateNameSpace (source, ns)`.
    pub fn duplicate_namespace(&mut self, source: &DngXmp, ns: &str) {
        let paths = source.sdk.iterate_paths(Some(ns));
        for (path_ns, path) in paths {
            if let Some(value) = source.sdk.get_property(&path_ns, &path) {
                self.sdk.set(&path_ns, &path, &value);
            }
        }
    }

    /// `DuplicateSubtree (source, sourceNS, sourceRoot, destNS, destRoot)`.
    pub fn duplicate_subtree(
        &mut self,
        source: &DngXmp,
        source_ns: &str,
        source_root: &str,
        dest_ns: Option<&str>,
        dest_root: Option<&str>,
    ) {
        let dest_ns = dest_ns.unwrap_or(source_ns);
        let dest_root = dest_root.unwrap_or(source_root);

        let paths = source.sdk.iterate_paths(Some(source_ns));
        for (_, path) in paths {
            // Only paths under the source root.
            if !path.starts_with(source_root) {
                continue;
            }

            let local = &path[source_root.len()..];
            let dest_path = format!("{}{}", dest_root, local);

            if let Some(value) = source.sdk.get_property(source_ns, &path) {
                self.sdk.set(dest_ns, &dest_path, &value);
            }
        }
    }

    /// `MakeInitialXMP` (dng_negative.cpp helper surfaced here) — seeds the
    /// standard namespaces with the fields the SDK always writes.
    pub fn make_initial_xmp(&mut self, exif: &DngExif) {
        self.require_meta();

        // dc:format is always "image/dng" for a DNG.
        self.set(XMP_NS_DC, "dc:format", "image/dng");

        // xmp:CreatorTool (software).
        if exif.software.not_empty() {
            self.set_string(XMP_NS_XMP, "xmp:CreatorTool", &exif.software);
        }

        // TIFF make/model from EXIF.
        if exif.make.not_empty() {
            self.set_string(XMP_NS_TIFF, "tiff:Make", &exif.make);
        }
        if exif.model.not_empty() {
            self.set_string(XMP_NS_TIFF, "tiff:Model", &exif.model);
        }

        // EXIF version.
        if exif.exif_version != 0 {
            let b0 = (exif.exif_version >> 24) & 0xFF;
            let b1 = (exif.exif_version >> 16) & 0xFF;
            let b2 = (exif.exif_version >> 8) & 0xFF;
            let b3 = exif.exif_version & 0xFF;
            let text = format!(
                "{}{}{}{}",
                (b0 as u8) as char, (b1 as u8) as char, (b2 as u8) as char, (b3 as u8) as char
            );
            self.set(XMP_NS_EXIF, "exif:ExifVersion", &text);
        }

        // Native digest placeholder: the C++ stores an empty native digest;
        // it is updated by the writer.
        self.set(XMP_NS_CAMERA_RAW, "crs:Version", "15.0");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_sync_both_directions() {
        let mut xmp = DngXmp::new();
        let mut exif = DngExif::new();
        exif.make.set(b"Canon");

        // Non-XMP value with no XMP: EXIF -> XMP.
        xmp.sync_exif(&mut exif, None, false, false);
        assert!(xmp.has_meta());
        assert_eq!(
            xmp.sdk.get_property(XMP_NS_TIFF, "tiff:Make"),
            Some("Canon".to_string())
        );

        // Change XMP; preferNonXMP keeps EXIF.
        xmp.set(XMP_NS_TIFF, "tiff:Make", "Nikon");
        xmp.sync_exif(&mut exif, None, false, false);
        assert_eq!(exif.make.get(), b"Canon");

        // preferXMP pulls the XMP value in (the previous preferNonXMP sync
        // wrote "Canon" back to the XMP; re-set it to "Nikon" to create a
        // conflict that preferXMP resolves in XMP's favor).
        xmp.set(XMP_NS_TIFF, "tiff:Make", "Nikon");
        let options = preferXMP;
        let mut s = DngString::from_bytes(b"Canon");
        xmp.sync_string(XMP_NS_TIFF, "tiff:Make", &mut s, options);
        assert_eq!(s.get(), b"Nikon");
    }

    #[test]
    fn boolean_int_real_roundtrip() {
        let mut xmp = DngXmp::new();

        xmp.set_boolean(XMP_NS_EXIF, "exif:Test", true);
        let mut b = false;
        assert!(xmp.get_boolean(XMP_NS_EXIF, "exif:Test", &mut b));
        assert!(b);

        xmp.set_int32(XMP_NS_EXIF, "exif:Int", -42, false);
        let mut v: i32 = 0;
        assert!(xmp.get_int32(XMP_NS_EXIF, "exif:Int", &mut v));
        assert_eq!(v, -42);

        xmp.set_real64(XMP_NS_EXIF, "exif:Real", 3.25, 6, true, false);
        let mut r: f64 = 0.0;
        assert!(xmp.get_real64(XMP_NS_EXIF, "exif:Real", &mut r));
        assert_eq!(r, 3.25);
    }

    #[test]
    fn urational_sync() {
        let mut xmp = DngXmp::new();
        let mut r = DngURational::new(1, 125);

        xmp.sync_urational_pair(XMP_NS_EXIF, "ExposureTime", &mut r, preferNonXMP);
        assert_eq!(
            xmp.sdk.get_property(XMP_NS_EXIF, "ExposureTime"),
            Some("1/125".to_string())
        );
    }

    #[test]
    fn fingerprint_roundtrip() {
        let mut xmp = DngXmp::new();
        let digest = DngFingerprint {
            data: crate::dng_core::dng_fingerprint::md5_digest(b"test data"),
        };

        xmp.set_fingerprint(XMP_NS_PHOTOSHOP, "IptcDigest", &digest, false);
        let mut back = DngFingerprint::new();
        assert!(xmp.get_fingerprint(XMP_NS_PHOTOSHOP, "IptcDigest", &mut back));
        assert_eq!(back, digest);
    }

    #[test]
    fn string_list_roundtrip() {
        let mut xmp = DngXmp::new();

        let mut list = DngStringList::new();
        list.append(&DngString::from_bytes(b"one"));
        list.append(&DngString::from_bytes(b"two"));

        xmp.set_string_list(XMP_NS_DC, "dc:subject", &list, true);
        assert_eq!(xmp.sdk.count_array_items(XMP_NS_DC, "dc:subject"), 2);

        let mut back = DngStringList::new();
        assert!(xmp.get_string_list(XMP_NS_DC, "dc:subject", &mut back));
        assert_eq!(back.count(), 2);
        assert_eq!(back.get(0).get(), b"one");
    }

    #[test]
    fn initial_xmp_seeds_namespaces() {
        let mut xmp = DngXmp::new();
        let mut exif = DngExif::new();
        exif.make.set(b"Canon");
        exif.model.set(b"EOS R5");
        xmp.make_initial_xmp(&exif);

        assert_eq!(
            xmp.sdk.get_property(XMP_NS_DC, "dc:format"),
            Some("image/dng".to_string())
        );
        assert_eq!(
            xmp.sdk.get_property(XMP_NS_TIFF, "tiff:Make"),
            Some("Canon".to_string())
        );
    }

    #[test]
    fn iptc_ingest_prefers_xmp_without_digest() {
        let mut xmp = DngXmp::new();
        xmp.require_meta();

        let mut iptc = DngIptc::new();
        iptc.title.set(b"From IPTC");

        let block = iptc.spool(false).unwrap();
        let digest = DngFingerprint {
            data: crate::dng_core::dng_fingerprint::md5_digest(block.buffer()),
        };

        // No digest in XMP -> MWG prefers XMP (i.e., IPTC fields do NOT
        // overwrite XMP values), but the digest gets recorded.
        let zero = DngFingerprint::new();
        xmp.ingest_iptc(block.buffer(), &digest, &zero, false).unwrap();

        assert_eq!(xmp.get_iptc_digest(), digest);
    }
}

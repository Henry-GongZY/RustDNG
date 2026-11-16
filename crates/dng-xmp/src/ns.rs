// This file is part of RustDNG, a Rust translation (with modifications) of
// the Adobe DNG SDK 1.7.1.
//
// Copyright 2005-2023 Adobe Systems Incorporated. All Rights Reserved.
// Rust translation and modifications: Copyright 2026 RustDNG contributors.
//
// NOTICE: Adobe permits you to use, modify, and distribute this file in
// accordance with the terms of the Adobe DNG SDK License Agreement
// accompanying this distribution (see LICENSE-ADOBE-DNG-SDK.txt).
//
//! Namespace constants mirroring the Adobe XMP Toolkit's `kXMP_NS_*` values
//! (from `XMP_Const.h`) plus the DNG-SDK-specific registrations from
//! `dng_xmp_sdk.cpp`, and the global prefix registry.

/// Packet namespace (`x:` prefix).
pub const NS_X: &str = "adobe:ns:meta/";
/// RDF namespace.
pub const NS_RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
/// xml: namespace (kXMP_NS_XML).
pub const NS_XML: &str = "http://www.w3.org/XML/1998/namespace";
/// XML namespace prefix constant name used by the toolkit.
pub const NS_XMP_NS_XML: &str = NS_XML;

// --- Toolkit core namespaces (XMP_Const.h) ---

pub const XMP_NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
pub const XMP_NS_XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/";
pub const XMP_NS_XMP_MM: &str = "http://ns.adobe.com/xap/1.0/mm/";
pub const XMP_NS_XMP_BJ: &str = "http://ns.adobe.com/xap/1.0/bj/";
pub const XMP_NS_XMP_NOTE: &str = "http://ns.adobe.com/xmp/note/";
pub const XMP_NS_PDF: &str = "http://ns.adobe.com/pdf/1.3/";
pub const XMP_NS_PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
pub const XMP_NS_PSALBUM: &str = "http://ns.adobe.com/album/1.0/";
pub const XMP_NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";
pub const XMP_NS_EXIF_AUX: &str = "http://ns.adobe.com/exif/1.0/aux/";
pub const XMP_NS_EXIFEX: &str = "http://cipa.jp/exif/1.0/";
pub const XMP_NS_TIFF: &str = "http://ns.adobe.com/tiff/1.0/";
pub const XMP_NS_PNG: &str = "http://ns.adobe.com/png/1.0/";
pub const XMP_NS_JPEG: &str = "http://ns.adobe.com/jpeg/1.0/";
pub const XMP_NS_JP2K: &str = "http://ns.adobe.com/jp2k/1.0/";
pub const XMP_NS_CAMERA_RAW: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
pub const XMP_NS_DM: &str = "http://ns.adobe.com/xmp/1.0/DynamicMedia/";
pub const XMP_NS_SCRIPT: &str = "http://ns.adobe.com/xmp/1.0/Script/";
pub const XMP_NS_ASF: &str = "http://ns.adobe.com/asf/1.0/";
pub const XMP_NS_WAV: &str = "http://ns.adobe.com/xmp/wav/1.0/";
pub const XMP_NS_TRANSIENT: &str = "http://ns.adobe.com/xmp/transient/1.0/";
pub const XMP_NS_DC: &str = "http://purl.org/dc/elements/1.1/";
pub const XMP_NS_IPTC_CORE: &str = "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/";
pub const XMP_NS_IPTC_EXT: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
pub const XMP_NS_DICOM: &str = "http://ns.adobe.com/DICOM/";
pub const XMP_NS_PLUS: &str = "http://ns.useplus.org/ldf/xmp/1.0/";

// Common struct field namespaces used by the toolkit.
pub const XMP_NS_XMP_DIMENSIONS: &str = "http://ns.adobe.com/xap/1.0/sType/Dimensions#";
pub const XMP_NS_XMP_TEXT: &str = "http://ns.adobe.com/xap/1.0/t/";
pub const XMP_NS_XMP_RESOURCE_EVENT: &str = "http://ns.adobe.com/xap/1.0/sType/ResourceEvent#";
pub const XMP_NS_XMP_RESOURCE_REF: &str = "http://ns.adobe.com/xap/1.0/sType/ResourceRef#";
pub const XMP_NS_XMP_ST_VERSION: &str = "http://ns.adobe.com/xap/1.0/sType/Version#";
pub const XMP_NS_XMP_ST_JOB: &str = "http://ns.adobe.com/xap/1.0/sType/Job#";
pub const XMP_NS_XMP_MANIFEST_ITEM: &str = "http://ns.adobe.com/xap/1.0/sType/ManifestItem#";
pub const XMP_NS_XMP_IDENTIFIER_QUAL: &str = "http://ns.adobe.com/xmp/Identifier/qual/1.0/";

// --- DNG SDK specific registrations (dng_xmp_sdk.cpp InitializeSDK) ---

pub const XMP_NS_CRS: &str = "http://ns.adobe.com/camera-raw-settings/1.0/";
pub const XMP_NS_CRSS: &str = "http://ns.adobe.com/camera-raw-saved-settings/1.0/";
pub const XMP_NS_CRD: &str = "http://ns.adobe.com/camera-raw-defaults/1.0/";
pub const XMP_NS_CRLCP: &str = "http://ns.adobe.com/camera-raw-embedded-lens-profile/1.0/";
pub const XMP_NS_LR: &str = "http://ns.adobe.com/lightroom/1.0/";
pub const XMP_NS_LCP: &str = "http://ns.adobe.com/photoshop/1.0/camera-profile";
pub const XMP_NS_CRX: &str = "http://ns.adobe.com/lightroom-settings-experimental/1.0/";
pub const XMP_NS_DNG: &str = "http://ns.adobe.com/dng/1.0/";
pub const XMP_NS_PANO: &str = "http://ns.adobe.com/photoshop/1.0/panorama-profile";
pub const XMP_NS_GPANO: &str = "http://ns.google.com/photos/1.0/panorama/";
pub const XMP_NS_REGIONS: &str = "http://www.metadataworkinggroup.com/schemas/regions/";
pub const XMP_NS_HDRGM: &str = "http://ns.adobe.com/hdr-gain-map/1.0/";
pub const XMP_NS_HDR_META: &str = "http://ns.adobe.com/hdr-metadata/1.0/";
pub const XMP_NS_APPLE_HDRGM: &str = "http://ns.apple.com/HDRGainMap/1.0/";
pub const XMP_NS_APPLE_PIXELDATA: &str = "http://ns.apple.com/pixeldatainfo/1.0/";

/// The default prefix registry, mirroring the registrations the DNG SDK
/// performs in `dng_xmp_sdk::InitializeSDK` plus the toolkit's built-ins
/// it relies on. `prefix -> uri` in registration order.
pub const DEFAULT_PREFIXES: &[(&str, &str)] = &[
    ("xml", NS_XML),
    ("rdf", NS_RDF),
    ("x", NS_X),
    ("dc", XMP_NS_DC),
    ("xmp", XMP_NS_XMP),
    ("xmpRights", XMP_NS_XMP_RIGHTS),
    ("xmpMM", XMP_NS_XMP_MM),
    ("xmpBJ", XMP_NS_XMP_BJ),
    ("xmpNote", XMP_NS_XMP_NOTE),
    ("pdf", XMP_NS_PDF),
    ("photoshop", XMP_NS_PHOTOSHOP),
    ("psAlbum", XMP_NS_PSALBUM),
    ("exif", XMP_NS_EXIF),
    ("aux", XMP_NS_EXIF_AUX),
    ("exifEX", XMP_NS_EXIFEX),
    ("tiff", XMP_NS_TIFF),
    ("png", XMP_NS_PNG),
    ("jpeg", XMP_NS_JPEG),
    ("jp2k", XMP_NS_JP2K),
    ("crs", XMP_NS_CRS),
    ("crss", XMP_NS_CRSS),
    ("crd", XMP_NS_CRD),
    ("crlcp", XMP_NS_CRLCP),
    ("lr", XMP_NS_LR),
    ("stCamera", XMP_NS_LCP),
    ("Iptc4xmpCore", XMP_NS_IPTC_CORE),
    ("Iptc4xmpExt", XMP_NS_IPTC_EXT),
    ("crx", XMP_NS_CRX),
    ("dng", XMP_NS_DNG),
    ("panorama", XMP_NS_PANO),
    ("GPano", XMP_NS_GPANO),
    ("mwg-rs", XMP_NS_REGIONS),
    ("hdrgm", XMP_NS_HDRGM),
    ("hdr_metadata", XMP_NS_HDR_META),
    ("HDRGainMap", XMP_NS_APPLE_HDRGM),
    ("apdi", XMP_NS_APPLE_PIXELDATA),
    ("stDim", XMP_NS_XMP_DIMENSIONS),
    ("stEvt", XMP_NS_XMP_RESOURCE_EVENT),
    ("stRef", XMP_NS_XMP_RESOURCE_REF),
    ("stVer", XMP_NS_XMP_ST_VERSION),
    ("stJob", XMP_NS_XMP_ST_JOB),
    ("xmpDM", XMP_NS_DM),
    ("xmpScript", XMP_NS_SCRIPT),
    ("xmpidq", XMP_NS_XMP_IDENTIFIER_QUAL),
];

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

static REGISTRY: OnceLock<Mutex<BTreeMap<String, String>>> = OnceLock::new();

fn registry() -> &'static Mutex<BTreeMap<String, String>> {
    REGISTRY.get_or_init(|| {
        let mut map = BTreeMap::new();
        for (prefix, uri) in DEFAULT_PREFIXES {
            map.insert(uri.to_string(), prefix.to_string());
        }
        Mutex::new(map)
    })
}

/// Register a namespace prefix (mirrors `SXMPMeta::RegisterNamespace`).
/// The first registration for a URI wins, matching toolkit behavior.
pub fn register_namespace(prefix: &str, uri: &str) {
    let mut reg = registry().lock().unwrap();
    reg.entry(uri.to_string())
        .or_insert_with(|| prefix.to_string());
}

/// Look up the preferred prefix for a URI (mirrors `GetNamespacePrefix`).
pub fn namespace_prefix(uri: &str) -> Option<String> {
    let reg = registry().lock().unwrap();
    reg.get(uri).cloned()
}

/// Reverse lookup: find the URI a registered prefix maps to.
pub fn lookup_prefix_uri(prefix: &str) -> Option<String> {
    let reg = registry().lock().unwrap();
    reg.iter()
        .find(|(_, p)| p.as_str() == prefix)
        .map(|(u, _)| u.clone())
}

/// (Testing helper) Reset the registry to defaults.
#[cfg(test)]
pub fn reset_registry() {
    let mut reg = registry().lock().unwrap();
    reg.clear();
    for (prefix, uri) in DEFAULT_PREFIXES {
        reg.insert(uri.to_string(), prefix.to_string());
    }
}

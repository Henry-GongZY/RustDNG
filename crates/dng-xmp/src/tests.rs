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
//! Tests for the dng-xmp engine.

use crate::*;

fn build_sample() -> XmpMeta {
    let mut m = XmpMeta::new();
    m.set_object_name("");
    m.set_property(ns::XMP_NS_XMP, "CreatorTool", "RustDNG Test");
    m.set_property(ns::XMP_NS_DC, "format", "image/dng");
    m.set_property(ns::XMP_NS_EXIF, "ExifVersion", "0232");
    m.set_property(ns::XMP_NS_TIFF, "Make", "Test Camera");
    // Array.
    m.append_array_item(ns::XMP_NS_DC, "creator", true, "Alice");
    m.append_array_item(ns::XMP_NS_DC, "creator", true, "Bob");
    // LangAlt.
    m.set_localized_text(ns::XMP_NS_DC, "title", "x-default", "A Title");
    m.set_localized_text(ns::XMP_NS_DC, "title", "de", "Ein Titel");
    // Struct.
    m.set_struct_field(ns::XMP_NS_EXIF, "Flash", ns::XMP_NS_EXIF, "Fired", "True");
    m.set_struct_field(ns::XMP_NS_EXIF, "Flash", ns::XMP_NS_EXIF, "Mode", "2");
    m
}

#[test]
fn build_serialize_parse_tree_equality() {
    let built = build_sample();
    let bytes = built
        .serialize_to_buffer(&XmpSerializeOptions::default())
        .unwrap();
    let reparsed = XmpMeta::parse_from_buffer(&bytes).unwrap();

    assert_eq!(built, reparsed, "round-trip must preserve the node tree");

    // Spot checks.
    assert_eq!(
        reparsed.get_property(ns::XMP_NS_XMP, "CreatorTool").unwrap(),
        "RustDNG Test"
    );
    assert_eq!(
        reparsed.get_property(ns::XMP_NS_EXIF, "ExifVersion").unwrap(),
        "0232"
    );
    assert_eq!(
        reparsed.get_struct_field(ns::XMP_NS_EXIF, "Flash", ns::XMP_NS_EXIF, "Fired"),
        Some("True".to_string())
    );
    assert_eq!(
        reparsed.get_array_item(ns::XMP_NS_DC, "creator", 2),
        Some("Bob".to_string())
    );
    assert_eq!(
        reparsed.get_localized_text(ns::XMP_NS_DC, "title", "de"),
        Some("Ein Titel".to_string())
    );
    assert_eq!(reparsed.count_array_items(ns::XMP_NS_DC, "creator"), 2);
}

#[test]
fn serialize_output_is_spec_valid() {
    let built = build_sample();
    let opts = XmpSerializeOptions::default();
    let bytes = built.serialize_to_buffer(&opts).unwrap();
    let s = String::from_utf8(bytes.clone()).unwrap();

    // Packet wrapper conventions (XMP Part 3).
    assert!(s.starts_with("<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>"));
    assert!(s.contains("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\""));
    assert!(s.contains("x:xmptk=\"RustDNG XMP Core"));
    assert!(s.contains("<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">"));
    assert!(s.trim_end().ends_with("<?xpacket end=\"w\"?>"));
    // One rdf:Description per schema in normal form (xmp, dc, exif, tiff).
    assert_eq!(s.matches("<rdf:Description").count(), 4);

    // Reparse.
    assert!(XmpMeta::parse_from_buffer(&bytes).is_ok());

    // Read-only packet.
    let mut ro = opts.clone();
    ro.read_only_packet = true;
    let s2 = String::from_utf8(built.serialize_to_buffer(&ro).unwrap()).unwrap();
    assert!(s2.contains("<?xpacket end=\"r\"?>"));
}

#[test]
fn parse_attribute_form_and_packet() {
    // Modeled after a real DNG packet (compact syntax).
    let packet = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
   xmp:CreatorTool="cr_validate 16.0"
   xmpMM:DocumentID="xmp.did:be617578"
   dc:format="image/dng">
   <xmpMM:History>
    <rdf:Seq>
     <rdf:li stEvt:action="saved"
      stEvt:when="2023-05-22T14:39:20-04:00"
      stEvt:softwareAgent="cr_validate"/>
    </rdf:Seq>
   </xmpMM:History>
   <xmpMM:DerivedFrom rdf:parseType="Resource"/>
   <dc:subject>
    <rdf:Bag>
     <rdf:li>tag one</rdf:li>
     <rdf:li>tag two</rdf:li>
    </rdf:Bag>
   </dc:subject>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    let meta = XmpMeta::parse_from_buffer(packet.as_bytes()).unwrap();

    // Attribute-form simple properties.
    assert_eq!(
        meta.get_property(ns::XMP_NS_XMP, "CreatorTool").unwrap(),
        "cr_validate 16.0"
    );
    assert_eq!(
        meta.get_property(ns::XMP_NS_XMP_MM, "DocumentID").unwrap(),
        "xmp.did:be617578"
    );
    assert_eq!(meta.get_property(ns::XMP_NS_DC, "format").unwrap(), "image/dng");

    // Attribute-form struct item inside an array.
    assert_eq!(
        meta.get_struct_field(
            ns::XMP_NS_XMP_MM,
            "History[1]",
            ns::XMP_NS_XMP_RESOURCE_EVENT,
            "action"
        ),
        Some("saved".to_string())
    );
    assert_eq!(
        meta.get_struct_field(
            ns::XMP_NS_XMP_MM,
            "History[1]",
            ns::XMP_NS_XMP_RESOURCE_EVENT,
            "softwareAgent"
        ),
        Some("cr_validate".to_string())
    );

    // Empty struct (parseType="Resource" with no children).
    assert!(meta.does_property_exist(ns::XMP_NS_XMP_MM, "DerivedFrom"));

    // Bag array.
    assert_eq!(meta.count_array_items(ns::XMP_NS_DC, "subject"), 2);
    assert_eq!(
        meta.get_array_item(ns::XMP_NS_DC, "subject", 1).unwrap(),
        "tag one"
    );
}

#[test]
fn lang_alt_x_default_rules() {
    let mut m = XmpMeta::new();

    // First set creates the Alt array with x-default first.
    m.set_localized_text(ns::XMP_NS_DC, "description", "en-US", "Hello");
    m.set_localized_text(ns::XMP_NS_DC, "description", "x-default", "Default");
    m.set_localized_text(ns::XMP_NS_DC, "description", "fr", "Bonjour");

    // x-default must be item 1.
    assert_eq!(
        m.get_array_item(ns::XMP_NS_DC, "description", 1).unwrap(),
        "Default"
    );
    assert_eq!(
        m.get_localized_text(ns::XMP_NS_DC, "description", "x-default").unwrap(),
        "Default"
    );
    assert_eq!(
        m.get_localized_text(ns::XMP_NS_DC, "description", "en-US").unwrap(),
        "Hello"
    );
    assert_eq!(
        m.get_localized_text(ns::XMP_NS_DC, "description", "fr").unwrap(),
        "Bonjour"
    );

    // Generic prefix match: "en" matches "en-us".
    assert_eq!(
        m.get_localized_text(ns::XMP_NS_DC, "description", "en").unwrap(),
        "Hello"
    );

    // Replace x-default keeps it first.
    m.set_localized_text(ns::XMP_NS_DC, "description", "x-default", "Default 2");
    assert_eq!(
        m.get_array_item(ns::XMP_NS_DC, "description", 1).unwrap(),
        "Default 2"
    );
    assert_eq!(m.count_array_items(ns::XMP_NS_DC, "description"), 3);

    // LangAlt serializes as Alt with xml:lang and round-trips.
    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(
        p.get_localized_text(ns::XMP_NS_DC, "description", "fr"),
        Some("Bonjour".to_string())
    );
    assert_eq!(p, m);

    // The "empty alt array" special case from dng_xmp_sdk.cpp.
    let mut m2 = XmpMeta::new();
    m2.set_property(ns::XMP_NS_DC, "description", "");
    assert_eq!(
        m2.get_localized_text(ns::XMP_NS_DC, "description", "x-default"),
        Some(String::new())
    );
}

#[test]
fn array_operations() {
    let mut m = XmpMeta::new();

    // Bag.
    m.append_array_item(ns::XMP_NS_DC, "subject", false, "one");
    m.append_array_item(ns::XMP_NS_DC, "subject", false, "two");
    m.append_array_item(ns::XMP_NS_DC, "subject", false, "three");
    assert_eq!(m.count_array_items(ns::XMP_NS_DC, "subject"), 3);
    assert_eq!(m.get_array_item(ns::XMP_NS_DC, "subject", 3).unwrap(), "three");

    // Set in place.
    m.set_array_item(ns::XMP_NS_DC, "subject", 2, "TWO");
    assert_eq!(m.get_array_item(ns::XMP_NS_DC, "subject", 2).unwrap(), "TWO");

    // Delete middle item renumbers.
    m.delete_property(ns::XMP_NS_DC, "subject[2]");
    assert_eq!(m.count_array_items(ns::XMP_NS_DC, "subject"), 2);
    assert_eq!(m.get_array_item(ns::XMP_NS_DC, "subject", 2).unwrap(), "three");

    // Append to nonexistent array creates it.
    m.append_array_item(ns::XMP_NS_DC, "creator", true, "X");
    assert_eq!(m.count_array_items(ns::XMP_NS_DC, "creator"), 1);

    // The "emptied array" dummy-item convention: an array with a single
    // empty item gets the item replaced on append.
    m.set_array_item(ns::XMP_NS_DC, "creator", 1, "");
    m.append_array_item(ns::XMP_NS_DC, "creator", true, "Real");
    assert_eq!(m.count_array_items(ns::XMP_NS_DC, "creator"), 1);
    assert_eq!(m.get_array_item(ns::XMP_NS_DC, "creator", 1).unwrap(), "Real");
}

#[test]
fn struct_operations() {
    let mut m = XmpMeta::new();

    m.set_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCity", "Kyoto");
    m.set_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCtry", "JP");
    assert_eq!(
        m.get_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCity"),
        Some("Kyoto".to_string())
    );

    // Field in a different namespace than the struct.
    m.set_struct_field(ns::XMP_NS_XMP_MM, "Task", ns::XMP_NS_XMP_ST_JOB, "JobRef", "abc");
    assert_eq!(
        m.get_struct_field(ns::XMP_NS_XMP_MM, "Task", ns::XMP_NS_XMP_ST_JOB, "JobRef"),
        Some("abc".to_string())
    );

    // Round-trip keeps both field namespaces.
    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p, m);

    // Delete one field.
    m.delete_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCtry");
    assert_eq!(
        m.get_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCtry"),
        None
    );
    assert_eq!(
        m.get_struct_field(ns::XMP_NS_IPTC_CORE, "CreatorContactInfo", ns::XMP_NS_IPTC_CORE, "CiAdrCity"),
        Some("Kyoto".to_string())
    );
}

#[test]
fn nested_array_of_structs() {
    // The xmpMM:History shape: Seq of ResourceEvent structs, struct fields
    // in the stEvt namespace.
    let mut m = XmpMeta::new();
    m.append_array_item(
        ns::XMP_NS_XMP_MM,
        "History",
        true,
        "",
    );
    // Replace item 1 with a struct.
    m.set_struct_field(ns::XMP_NS_XMP_MM, "History[1]", ns::XMP_NS_XMP_RESOURCE_EVENT, "action", "converted");
    m.set_struct_field(ns::XMP_NS_XMP_MM, "History[1]", ns::XMP_NS_XMP_RESOURCE_EVENT, "when", "2024-01-01");
    m.append_array_item(ns::XMP_NS_XMP_MM, "History", true, "");
    m.set_struct_field(ns::XMP_NS_XMP_MM, "History[2]", ns::XMP_NS_XMP_RESOURCE_EVENT, "action", "saved");

    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p, m);
    assert_eq!(
        p.get_struct_field(ns::XMP_NS_XMP_MM, "History[2]", ns::XMP_NS_XMP_RESOURCE_EVENT, "action"),
        Some("saved".to_string())
    );
}

#[test]
fn xml_escaping_both_ways() {
    let mut m = XmpMeta::new();
    let tricky = "a<b>c&d\"e'f\n\r\tg";
    m.set_property(ns::XMP_NS_DC, "title", tricky);

    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let s = String::from_utf8(bytes.clone()).unwrap();
    // No raw < & inside element text.
    assert!(s.contains("a&lt;b&gt;c&amp;d&quot;e&apos;f&#10;&#13;\tg"));

    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p.get_property(ns::XMP_NS_DC, "title").unwrap(), tricky);
    assert_eq!(p, m);

    // Parsing escaped input.
    let doc = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/"
   dc:rights="&lt;tag&gt; &amp; &#xA9; 2024"/>
 </rdf:RDF>
</x:xmpmeta>"#;
    let p2 = XmpMeta::parse_from_buffer(doc.as_bytes()).unwrap();
    assert_eq!(
        p2.get_property(ns::XMP_NS_DC, "rights").unwrap(),
        "<tag> & \u{A9} 2024"
    );
}

#[test]
fn unknown_namespace_preservation() {
    // Parse a packet containing namespaces this engine does not know about;
    // round-trip must keep them.
    let doc = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:weird="http://example.com/unknown/schema/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
   dc:format="image/dng">
   <weird:CustomProp>custom value</weird:CustomProp>
   <weird:CustomStruct weird:PartA="x" weird:PartB="y"/>
   <weird:List>
    <rdf:Seq>
     <rdf:li>u1</rdf:li>
     <rdf:li>u2</rdf:li>
    </rdf:Seq>
   </weird:List>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let m = XmpMeta::parse_from_buffer(doc.as_bytes()).unwrap();
    assert_eq!(
        m.get_property("http://example.com/unknown/schema/1.0/", "CustomProp").unwrap(),
        "custom value"
    );
    assert_eq!(
        m.get_struct_field(
            "http://example.com/unknown/schema/1.0/",
            "CustomStruct",
            "http://example.com/unknown/schema/1.0/",
            "PartA"
        ),
        Some("x".to_string())
    );
    assert_eq!(
        m.count_array_items("http://example.com/unknown/schema/1.0/", "List"),
        2
    );

    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p, m);

    // The unknown namespace got an xmlns declaration on output.
    let s = String::from_utf8(bytes).unwrap();
    assert!(s.contains("http://example.com/unknown/schema/1.0/"));
}

#[test]
fn packet_wrap_unwrap_forms() {
    let mut m = XmpMeta::new();
    m.set_property(ns::XMP_NS_DC, "format", "image/dng");

    // 1. Full packet.
    let packet = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    assert_eq!(XmpMeta::parse_from_buffer(&packet).unwrap(), m);

    // 2. Bare rdf:RDF.
    let rdf = m.serialize_rdf_only(&XmpSerializeOptions::default()).unwrap();
    assert_eq!(XmpMeta::parse_from_buffer(&rdf).unwrap(), m);
    let s = String::from_utf8(rdf).unwrap();
    assert!(s.starts_with(" <rdf:RDF"));
    assert!(!s.contains("xpacket"));

    // 3. x:xmpmeta without xpacket wrapper.
    let rdf = String::from_utf8(m.serialize_rdf_only(&XmpSerializeOptions::default()).unwrap())
        .unwrap();
    let wrapped = format!(
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n{rdf}</x:xmpmeta>"
    );
    assert_eq!(XmpMeta::parse_from_buffer(wrapped.as_bytes()).unwrap(), m);

    // 4. xpacket wrapper without x:xmpmeta (some tools emit that).
    let packet2 = format!(
        "<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n{rdf}<?xpacket end=\"w\"?>"
    );
    assert_eq!(XmpMeta::parse_from_buffer(packet2.as_bytes()).unwrap(), m);
}

#[test]
fn padding_and_newline_options() {
    let mut m = XmpMeta::new();
    m.set_property(ns::XMP_NS_DC, "format", "image/dng");

    // Default: 2048 bytes of padding.
    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let s = String::from_utf8(bytes).unwrap();
    let footer = s.find("<?xpacket end").unwrap();
    let head_end = s.find("</x:xmpmeta>").unwrap() + "</x:xmpmeta>".len();
    let between = &s[head_end..footer];
    // Whitespace block between the meta end and footer.
    assert!(between.len() >= 2048);
    assert!(between.chars().all(|c| c == '\n' || c == ' '));

    // No padding.
    let mut opts = XmpSerializeOptions::default();
    opts.include_padding = false;
    let s2 = String::from_utf8(m.serialize_to_buffer(&opts).unwrap()).unwrap();
    let footer2 = s2.find("<?xpacket end").unwrap();
    let head_end2 = s2.find("</x:xmpmeta>").unwrap() + "</x:xmpmeta>".len();
    assert!(s2[head_end2..footer2].trim().is_empty());

    // CRLF newline option.
    let mut opts = XmpSerializeOptions::default();
    opts.new_line = "\r\n".into();
    opts.padding = 100;
    let s3 = String::from_utf8(m.serialize_to_buffer(&opts).unwrap()).unwrap();
    assert!(s3.contains("\r\n"));
    assert!(!s3[0..s3.find("<rdf:RDF").unwrap()].contains("\n\n"));

    // Reparse all three.
    assert!(XmpMeta::parse_from_buffer(s.as_bytes()).is_ok());
    assert!(XmpMeta::parse_from_buffer(s2.as_bytes()).is_ok());
    assert!(XmpMeta::parse_from_buffer(s3.as_bytes()).is_ok());
}

#[test]
fn compact_syntax_roundtrip() {
    let built = build_sample();
    let mut opts = XmpSerializeOptions::default();
    opts.use_compact_syntax = true;

    let bytes = built.serialize_to_buffer(&opts).unwrap();
    let s = String::from_utf8(bytes.clone()).unwrap();

    // Compact: simple properties as attributes on a single Description.
    assert_eq!(s.matches("<rdf:Description").count(), 1);
    assert!(s.contains("dc:format=\"image/dng\""));
    assert!(s.contains("exif:ExifVersion=\"0232\""));
    assert!(s.contains("tiff:Make=\"Test Camera\""));
    // Complex properties stay element-form.
    assert!(s.contains("<dc:creator>"));

    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p, built);
}

#[test]
fn namespace_registry() {
    ns::reset_registry();

    // Toolkit-registered URIs (from dng_xmp_sdk.cpp).
    assert_eq!(
        XmpMeta::namespace_prefix(ns::XMP_NS_CRS),
        Some("crs".to_string())
    );
    assert_eq!(
        XmpMeta::namespace_prefix(ns::XMP_NS_LCP),
        Some("stCamera".to_string())
    );
    assert_eq!(
        XmpMeta::namespace_prefix(ns::XMP_NS_GPANO),
        Some("GPano".to_string())
    );

    // Register a new namespace.
    XmpMeta::register_namespace("wd", "http://example.com/widget/1.0/");
    assert_eq!(
        XmpMeta::namespace_prefix("http://example.com/widget/1.0/"),
        Some("wd".to_string())
    );
}

#[test]
fn delete_property_and_has_namespace() {
    let built = build_sample();
    assert!(built.has_namespace(ns::XMP_NS_TIFF));
    assert!(!built.has_namespace(ns::XMP_NS_DNG));

    let mut m = built.clone();
    m.delete_property(ns::XMP_NS_TIFF, "Make");
    assert!(!m.does_property_exist(ns::XMP_NS_TIFF, "Make"));
    assert!(!m.has_namespace(ns::XMP_NS_TIFF));
}

#[test]
fn object_name_roundtrip() {
    let mut m = XmpMeta::new();
    m.set_object_name("uuid:1234-5678");
    m.set_property(ns::XMP_NS_DC, "format", "image/dng");
    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p.object_name(), "uuid:1234-5678");
    assert_eq!(p, m);
}

#[test]
fn parse_real_adobe_packet() {
    // Byte-identical excerpt of the packet in
    // vendor/…/sample_files/14_hdr_sdr_profiles.dng (truncated history).
    let doc = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:xmpMM="http://ns.adobe.com/xap/1.0/mm/"
    xmlns:stEvt="http://ns.adobe.com/xap/1.0/sType/ResourceEvent#"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
   xmp:CreatorTool="cr_validate 16.0 (Macintosh)"
   xmp:MetadataDate="2023-05-22T14:39:20-04:00"
   xmp:ModifyDate="2023-05-22T14:39:20-04:00"
   xmpMM:DocumentID="xmp.did:be617578-bc5d-4a79-8fc4-dadb31145514"
   xmpMM:InstanceID="xmp.iid:be617578-bc5d-4a79-8fc4-dadb31145514"
   xmpMM:OriginalDocumentID="xmp.did:be617578-bc5d-4a79-8fc4-dadb31145514"
   dc:format="image/dng">
   <xmpMM:History>
    <rdf:Seq>
     <rdf:li
      stEvt:action="derived"
      stEvt:parameters="saved to new location"/>
     <rdf:li
      stEvt:action="saved"
      stEvt:instanceID="xmp.iid:be617578-bc5d-4a79-8fc4-dadb31145514"
      stEvt:when="2023-05-22T14:39:20-04:00"
      stEvt:softwareAgent="cr_validate 16.0 (Macintosh)"
      stEvt:changed="/"/>
    </rdf:Seq>
   </xmpMM:History>
   <xmpMM:DerivedFrom rdf:parseType="Resource"/>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>"#;

    let m = XmpMeta::parse_from_buffer(doc.as_bytes()).unwrap();
    assert_eq!(m.get_property(ns::XMP_NS_DC, "format").unwrap(), "image/dng");
    assert_eq!(
        m.get_property(ns::XMP_NS_XMP, "CreatorTool").unwrap(),
        "cr_validate 16.0 (Macintosh)"
    );
    assert_eq!(
        m.get_struct_field(
            ns::XMP_NS_XMP_MM,
            "History[1]",
            ns::XMP_NS_XMP_RESOURCE_EVENT,
            "parameters"
        ),
        Some("saved to new location".to_string())
    );
    assert_eq!(
        m.get_struct_field(
            ns::XMP_NS_XMP_MM,
            "History[2]",
            ns::XMP_NS_XMP_RESOURCE_EVENT,
            "changed"
        ),
        Some("/".to_string())
    );

    // Re-serialize and reparse: tree must be stable.
    let bytes = m.serialize_to_buffer(&XmpSerializeOptions::default()).unwrap();
    let p = XmpMeta::parse_from_buffer(&bytes).unwrap();
    assert_eq!(p, m);
}

#[test]
fn bare_and_error_inputs() {
    assert!(XmpMeta::parse_from_buffer(b"not xml at all").is_err());
    assert!(XmpMeta::parse_from_buffer(b"<foo></bar>").is_err());
    assert!(XmpMeta::parse_from_buffer(b"").is_err());

    // Empty meta with only about.
    let doc = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about="x"/>
 </rdf:RDF>"#;
    let m = XmpMeta::parse_from_buffer(doc.as_bytes()).unwrap();
    assert_eq!(m.object_name(), "x");
}

#[test]
fn error_display() {
    let e = XmpError::BadFormat("x".into());
    assert!(e.to_string().contains("bad XMP format"));
    let _ = Box::new(e) as Box<dyn std::error::Error>;
}

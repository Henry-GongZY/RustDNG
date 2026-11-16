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
//! RDF/XML serialization following the XMP specification:
//! - Part 2: "normal form" (one `rdf:Description` per schema, element-form
//!   properties) and "compact form" (single `rdf:Description`, simple
//!   properties as attributes).
//! - Part 3: packet wrapper (`<?xpacket begin…?>` / `<?xpacket end?>`),
//!   padding whitespace conventions, newline handling.

use crate::error::XmpError;
use crate::node::{NodeValue, XmpMeta, XmpNode};
use crate::ns;

/// Serialization options (mirrors the knobs `dng_xmp_sdk::Serialize` passes
/// to `SerializeToBuffer`).
#[derive(Debug, Clone)]
pub struct XmpSerializeOptions {
    /// Use `<?xpacket end="r"?>` (read-only packet) instead of `"w"`.
    pub read_only_packet: bool,
    /// Emit the compact (attribute-form) syntax instead of the normal form.
    pub use_compact_syntax: bool,
    /// Amount of padding whitespace (bytes) between `</x:xmpmeta>` and the
    /// packet footer.
    pub padding: usize,
    /// Whether to emit the padding block at all.
    pub include_padding: bool,
    /// Newline string used in the output (default `"\n"`).
    pub new_line: String,
}

impl Default for XmpSerializeOptions {
    fn default() -> Self {
        XmpSerializeOptions {
            read_only_packet: false,
            use_compact_syntax: false,
            padding: 2048,
            include_padding: true,
            new_line: "\n".to_string(),
        }
    }
}

/// The XMP Toolkit toolkit-tag string.
const XMPTK: &str = "RustDNG XMP Core 1.7.1";

/// Serialize an [`XmpMeta`] into an XMP packet (or bare `rdf:RDF` when
/// `include_packet_wrapper` is... always packet here — the contract API
/// always serializes with the packet wrapper; bare form is exposed via
/// `serialize_rdf_only`).
pub fn serialize_to_buffer(
    meta: &XmpMeta,
    opts: &XmpSerializeOptions,
) -> Result<Vec<u8>, XmpError> {
    let body = serialize_body(meta, opts)?;
    let mut out = String::with_capacity(body.len() + opts.padding + 256);

    // Packet header: begin attribute holds the UTF-8 BOM, per XMP Part 3.
    out.push_str("<?xpacket begin=\"\u{FEFF}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>");
    out.push_str(&opts.new_line);
    out.push_str("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"");
    out.push_str(XMPTK);
    out.push_str("\">");
    out.push_str(&opts.new_line);
    out.push_str(&body);
    out.push_str("</x:xmpmeta>");

    // Padding + footer.
    let end_flag = if opts.read_only_packet { 'r' } else { 'w' };
    if opts.include_padding {
        out.push_str(&opts.new_line);
        push_padding(&mut out, opts.padding, &opts.new_line);
        out.push_str(&opts.new_line);
    }
    out.push_str(&format!("<?xpacket end=\"{end_flag}\"?>"));

    Ok(out.into_bytes())
}

/// Serialize just the `rdf:RDF` (no packet wrapper) — mirrors
/// `SerializeToBuffer` with `kXMP_OmitPacketWrapper`.
pub fn serialize_rdf_only(
    meta: &XmpMeta,
    opts: &XmpSerializeOptions,
) -> Result<Vec<u8>, XmpError> {
    let body = serialize_body(meta, opts)?;
    Ok(body.into_bytes())
}

fn push_padding(out: &mut String, padding: usize, new_line: &str) {
    if padding == 0 {
        return;
    }
    // Lines of 100 spaces separated by the newline, per the convention used
    // by the Adobe toolkit and observed in real DNG packets.
    const LINE: &str = "                                                                                                    "; // 100
    let mut remaining = padding;
    while remaining > 0 {
        if remaining >= LINE.len() {
            out.push_str(LINE);
            remaining -= LINE.len();
        } else {
            out.push_str(&LINE[..remaining]);
            remaining = 0;
        }
        if remaining > 0 {
            out.push_str(new_line);
        }
    }
}

// ---------------------------------------------------------------------------
// Body
// ---------------------------------------------------------------------------

/// A prefix assignment map: uri -> prefix (chosen per serialization run so
/// that prefixes are unique and deterministic).
struct Prefixes {
    map: std::collections::BTreeMap<String, String>,
}

impl Prefixes {
    fn new() -> Self {
        Prefixes {
            map: std::collections::BTreeMap::new(),
        }
    }

    fn assign(&mut self, uri: &str) -> String {
        if let Some(p) = self.map.get(uri) {
            return p.clone();
        }
        // Preferred prefix from the registry.
        let candidate = ns::namespace_prefix(uri).unwrap_or_else(|| {
            // Auto-generated: ns0, ns1, ... — find a free one.
            let mut i = 0;
            loop {
                let candidate = format!("ns{i}");
                if !self.map.values().any(|v| *v == candidate) {
                    return candidate;
                }
                i += 1;
            }
        });
        // Ensure uniqueness (registry prefix may collide with one already
        // assigned to a different URI).
        let mut final_prefix = candidate.clone();
        let mut i = 0;
        while self.map.values().any(|v| *v == final_prefix) {
            final_prefix = format!("{candidate}{i}");
            i += 1;
        }
        self.map.insert(uri.to_string(), final_prefix.clone());
        final_prefix
    }
}

/// Collect all namespace URIs used by a subtree (struct fields + qualifiers).
fn collect_uris(node: &XmpNode, uris: &mut Vec<String>) {
    match &node.value {
        NodeValue::Simple(_) => {}
        NodeValue::Struct(fields) => {
            for f in fields {
                if !uris.contains(&f.ns) {
                    uris.push(f.ns.clone());
                }
                collect_uris(&f.node, uris);
            }
        }
        NodeValue::Array { items, .. } => {
            for it in items {
                collect_uris(it, uris);
            }
        }
    }
    for q in &node.qualifiers {
        if !uris.contains(&q.ns) {
            uris.push(q.ns.clone());
        }
    }
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push('\t'),
            _ => out.push(c),
        }
    }
    out
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\n' => out.push_str("&#10;"),
            '\r' => out.push_str("&#13;"),
            '\t' => out.push_str("&#9;"),
            _ => out.push(c),
        }
    }
    out
}

fn serialize_body(meta: &XmpMeta, opts: &XmpSerializeOptions) -> Result<String, XmpError> {
    let nl = &opts.new_line;
    let mut prefixes = Prefixes::new();

    // Collect all schema URIs (skip empty schemas).
    let schemas: Vec<(&String, &std::collections::BTreeMap<String, XmpNode>)> = meta
        .schemas
        .iter()
        .filter(|(_, props)| !props.is_empty())
        .collect();

    let mut body = String::with_capacity(4096);
    body.push_str(" <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">");
    body.push_str(nl);

    if opts.use_compact_syntax {
        // Single rdf:Description carrying every schema.
        let mut all_uris: Vec<&String> = schemas.iter().map(|(u, _)| *u).collect();
        let mut extra: Vec<String> = Vec::new();
        for (_, props) in &schemas {
            for node in props.values() {
                collect_uris(node, &mut extra);
            }
        }
        for u in &extra {
            if !all_uris.iter().any(|x| *x == u) {
                all_uris.push(u);
            }
        }

        let mut attr_lines: Vec<String> = Vec::new();
        let mut elem_lines: Vec<String> = Vec::new();

        for (schema_uri, props) in &schemas {
            let prefix = prefixes.assign(schema_uri);
            for (name, node) in props.iter() {
                let qname = format!("{prefix}:{name}");
                match serialize_property_compact(&mut prefixes, &qname, node, nl) {
                    CompactProp::Attribute(line) => attr_lines.push(line),
                    CompactProp::Element(lines) => elem_lines.extend(lines),
                }
            }
        }

        body.push_str("  <rdf:Description rdf:about=\"");
        body.push_str(&escape_attr(&meta.about));
        body.push('"');
        for uri in &all_uris {
            let p = prefixes.assign(uri);
            body.push_str(nl);
            body.push_str("    xmlns:");
            body.push_str(&p);
            body.push_str("=\"");
            body.push_str(&escape_attr(uri));
            body.push('"');
        }
        for line in &attr_lines {
            body.push_str(nl);
            body.push_str("   ");
            body.push_str(line);
        }
        body.push_str(">");
        body.push_str(nl);
        for line in &elem_lines {
            body.push_str(line);
            body.push_str(nl);
        }
        body.push_str("  </rdf:Description>");
        body.push_str(nl);
    } else {
        // Normal form: one rdf:Description per schema.
        for (schema_uri, props) in &schemas {
            let prefix = prefixes.assign(schema_uri);

            // URIs needed by this schema's subtree (excluding the schema's
            // own declaration, emitted separately).
            let mut field_uris: Vec<String> = Vec::new();
            for node in props.values() {
                collect_uris(node, &mut field_uris);
            }
            field_uris.retain(|u| u.as_str() != schema_uri.as_str());

            body.push_str("  <rdf:Description rdf:about=\"");
            body.push_str(&escape_attr(&meta.about));
            body.push('"');
            body.push_str(nl);
            body.push_str("    xmlns:");
            body.push_str(&prefix);
            body.push_str("=\"");
            body.push_str(&escape_attr(schema_uri));
            body.push('"');
            for u in &field_uris {
                let p = prefixes.assign(u);
                body.push_str(nl);
                body.push_str("    xmlns:");
                body.push_str(&p);
                body.push_str("=\"");
                body.push_str(&escape_attr(u));
                body.push('"');
            }
            body.push('>');
            body.push_str(nl);

            for (name, node) in props.iter() {
                let lines = serialize_property(&mut prefixes, &prefix, name, node, nl);
                for l in lines {
                    body.push_str(&l);
                    body.push_str(nl);
                }
            }

            body.push_str("  </rdf:Description>");
            body.push_str(nl);
        }
    }

    body.push_str(" </rdf:RDF>");
    body.push_str(nl);

    Ok(body)
}

enum CompactProp {
    Attribute(String),
    Element(Vec<String>),
}

fn serialize_property_compact(
    prefixes: &mut Prefixes,
    name: &str,
    node: &XmpNode,
    nl: &str,
) -> CompactProp {
    // Simple properties without qualifiers become attributes.
    if let NodeValue::Simple(v) = &node.value {
        if node.qualifiers.is_empty() {
            return CompactProp::Attribute(format!("{name}=\"{}\"", escape_attr(v)));
        }
    }
    CompactProp::Element(serialize_property_inner(prefixes, name, node, nl, "   "))
}

/// Serialize one property (element form) as one or more output lines.
fn serialize_property(
    prefixes: &mut Prefixes,
    schema_prefix: &str,
    name: &str,
    node: &XmpNode,
    nl: &str,
) -> Vec<String> {
    let qname = format!("{schema_prefix}:{name}");
    serialize_property_inner(prefixes, &qname, node, nl, "   ")
}

fn serialize_property_inner(
    prefixes: &mut Prefixes,
    qname: &str,
    node: &XmpNode,
    nl: &str,
    indent: &str,
) -> Vec<String> {
    let child_indent = format!("{indent} ");

    // Qualifier wrapper form: <q rdf:parseType="Resource"><rdf:value>…
    if !node.qualifiers.is_empty() {
        let mut lines = Vec::new();
        lines.push(format!(
            "{indent}<{qname} rdf:parseType=\"Resource\">"
        ));
        if let NodeValue::Simple(v) = &node.value {
            lines.push(format!(
                "{child_indent}<rdf:value>{}</rdf:value>",
                escape_xml(v)
            ));
        } else {
            // Complex value with qualifiers: nested via inner serialization.
            let nested = serialize_value(prefixes, qname, node, nl, &child_indent);
            lines.extend(nested);
        }
        for q in &node.qualifiers {
            let qp = prefixes.assign(&q.ns);
            lines.push(format!(
                "{child_indent}<{}:{}>{}</{}:{}>",
                qp,
                q.name,
                escape_xml(&q.value),
                qp,
                q.name
            ));
        }
        lines.push(format!("{indent}</{qname}>"));
        return lines;
    }

    match &node.value {
        NodeValue::Simple(v) => {
            if v.is_empty() {
                vec![format!("{indent}<{qname}/>")]
            } else {
                vec![format!(
                    "{indent}<{qname}>{}</{qname}>",
                    escape_xml(v)
                )]
            }
        }
        NodeValue::Struct(fields) => {
            if fields.is_empty() {
                vec![format!("{indent}<{qname} rdf:parseType=\"Resource\"/>")]
            } else {
                let mut lines = vec![format!("{indent}<{qname} rdf:parseType=\"Resource\">")];
                for f in fields {
                    let fp = prefixes.assign(&f.ns);
                    let fq = format!("{fp}:{}", f.name);
                    lines.extend(serialize_property_inner(
                        prefixes,
                        &fq,
                        &f.node,
                        nl,
                        &child_indent,
                    ));
                }
                lines.push(format!("{indent}</{qname}>"));
                lines
            }
        }
        NodeValue::Array { form, items } => {
            if items.is_empty() {
                vec![
                    format!("{indent}<{qname}>"),
                    format!("{child_indent}<{}/>", form.element_name()),
                    format!("{indent}</{qname}>"),
                ]
            } else {
                let mut lines = vec![format!("{indent}<{qname}>")];
                lines.push(format!("{child_indent}<{}>", form.element_name()));
                let li_indent = format!("{child_indent} ");
                for item in items {
                    lines.extend(serialize_li(prefixes, item, nl, &li_indent));
                }
                lines.push(format!("{child_indent}</{}>", form.element_name()));
                lines.push(format!("{indent}</{qname}>"));
                lines
            }
        }
    }
}

fn serialize_li(prefixes: &mut Prefixes, item: &XmpNode, nl: &str, indent: &str) -> Vec<String> {
    // LangAlt items carry xml:lang qualifiers.
    if let Some(lang) = item.qualifier(ns::NS_XML, "lang") {
        if let NodeValue::Simple(v) = &item.value {
            if item.qualifiers.len() == 1 {
                return vec![format!(
                    "{indent}<rdf:li xml:lang=\"{}\">{}</rdf:li>",
                    escape_attr(lang),
                    escape_xml(v)
                )];
            }
        }
    }
    // Otherwise: reuse property serialization with rdf:li element.
    let mut lines = Vec::new();
    let li_has_qualifiers = !item.qualifiers.is_empty();
    if li_has_qualifiers {
        lines.push(format!("{indent}<rdf:li rdf:parseType=\"Resource\">"));
        let ci = format!("{indent} ");
        if let NodeValue::Simple(v) = &item.value {
            lines.push(format!("{ci}<rdf:value>{}</rdf:value>", escape_xml(v)));
        }
        for q in &item.qualifiers {
            let qp = prefixes.assign(&q.ns);
            lines.push(format!(
                "{ci}<{}:{}>{}</{}:{}>",
                qp,
                q.name,
                escape_xml(&q.value),
                qp,
                q.name
            ));
        }
        lines.push(format!("{indent}</rdf:li>"));
    } else {
        match &item.value {
            NodeValue::Simple(v) => {
                if v.is_empty() {
                    lines.push(format!("{indent}<rdf:li/>"));
                } else {
                    lines.push(format!("{indent}<rdf:li>{}</rdf:li>", escape_xml(v)));
                }
            }
            NodeValue::Struct(fields) => {
                if fields.is_empty() {
                    lines.push(format!("{indent}<rdf:li rdf:parseType=\"Resource\"/>"));
                } else {
                    lines.push(format!("{indent}<rdf:li rdf:parseType=\"Resource\">"));
                    let ci = format!("{indent} ");
                    for f in fields {
                        let fp = prefixes.assign(&f.ns);
                        let fq = format!("{fp}:{}", f.name);
                        lines.extend(serialize_property_inner(prefixes, &fq, &f.node, nl, &ci));
                    }
                    lines.push(format!("{indent}</rdf:li>"));
                }
            }
            NodeValue::Array { form, items } => {
                lines.push(format!("{indent}<rdf:li>"));
                let ci = format!("{indent} ");
                if items.is_empty() {
                    lines.push(format!("{ci}<{}/>", form.element_name()));
                } else {
                    lines.push(format!("{ci}<{}>", form.element_name()));
                    let li2 = format!("{ci} ");
                    for it in items {
                        lines.extend(serialize_li(prefixes, it, nl, &li2));
                    }
                    lines.push(format!("{ci}</{}>", form.element_name()));
                }
                lines.push(format!("{indent}</rdf:li>"));
            }
        }
    }
    lines
}

/// Serialize a complex value inline (used for qualified complex properties).
fn serialize_value(
    prefixes: &mut Prefixes,
    _qname: &str,
    node: &XmpNode,
    nl: &str,
    indent: &str,
) -> Vec<String> {
    match &node.value {
        NodeValue::Simple(_) => Vec::new(),
        NodeValue::Struct(fields) => {
            let mut lines = Vec::new();
            for f in fields {
                let fp = prefixes.assign(&f.ns);
                let fq = format!("{fp}:{}", f.name);
                lines.extend(serialize_property_inner(prefixes, &fq, &f.node, nl, indent));
            }
            lines
        }
        NodeValue::Array { form, items } => {
            let mut lines = vec![format!("{indent}<{0}>", form.element_name())];
            let ci = format!("{indent} ");
            for it in items {
                lines.extend(serialize_li(prefixes, it, nl, &ci));
            }
            lines.push(format!("{indent}</{0}>", form.element_name()));
            lines
        }
    }
}

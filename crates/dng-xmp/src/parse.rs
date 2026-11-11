//! XMP packet parsing: XML bytes -> generic element tree -> XMP node tree.
//!
//! Accepts a full `<?xpacket …?>` packet, an `x:xmpmeta` wrapper, or a bare
//! `rdf:RDF` (also bare `rdf:Description` for convenience). Handles element
//! form, attribute form, `rdf:parseType="Resource"`, nested
//! `rdf:Description` struct shorthand, `rdf:Bag/Seq/Alt` arrays,
//! `xml:lang` qualifiers, and `rdf:value`/`rdf:type`.

use std::collections::HashMap;

use quick_xml::events::Event;
use quick_xml::Reader;

use crate::error::XmpError;
use crate::ns;
use crate::node::{ArrayForm, NodeValue, XmpField, XmpMeta, XmpNode};

// ---------------------------------------------------------------------------
// Generic element tree (namespace-resolved)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Elem {
    /// Resolved namespace URI of the element name.
    ns: String,
    /// Local name.
    local: String,
    /// Namespace-resolved attributes: (ns, local, value).
    /// Namespace declarations (xmlns*) are stripped.
    attrs: Vec<(String, String, String)>,
    /// Concatenated character data (unescaped).
    text: String,
    children: Vec<Elem>,
}

impl Elem {
    fn q(&self) -> (String, String) {
        (self.ns.clone(), self.local.clone())
    }

    fn is_rdf(&self, local: &str) -> bool {
        self.ns == ns::NS_RDF && self.local == local
    }

    fn attr(&self, ns_uri: &str, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(a, n, _)| a == ns_uri && n == local)
            .map(|(_, _, v)| v.as_str())
    }
}

/// Parse XML bytes into a namespace-resolved element tree, skipping PIs
/// (xpacket), comments, doctypes, and declarations.
fn parse_xml_tree(data: &[u8]) -> Result<Vec<Elem>, XmpError> {
    let mut reader = Reader::from_reader(data);
    // Keep text untrimmed; interpretation decides what is whitespace noise.
    reader.config_mut().trim_text(false);
    // Do not require a single root (packets have PIs before/after content).

    // Namespace scopes stack: each scope maps prefix ("" = default) -> uri.
    let mut scopes: Vec<HashMap<String, String>> = vec![HashMap::new()];
    // Element stack: (elem, scope_index_at_open).
    let mut stack: Vec<(Elem, usize)> = Vec::new();
    let mut roots: Vec<Elem> = Vec::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| XmpError::BadFormat(format!("XML parse error: {e}")))?;
        match event {
            Event::Start(bs) => {
                // Collect declarations into a new scope.
                let mut scope: HashMap<String, String> = HashMap::new();
                let mut attrs: Vec<(String, String, String)> = Vec::new();
                for a in bs.attributes() {
                    let a = a.map_err(|e| XmpError::BadFormat(format!("bad attribute: {e}")))?;
                    let key = a.key.as_ref().to_vec();
                    let key_str = String::from_utf8_lossy(&key).to_string();
                    let value = a
                        .unescape_value()
                        .map_err(|e| XmpError::BadFormat(format!("bad attribute value: {e}")))?
                        .to_string();

                    if key_str == "xmlns" {
                        scope.insert(String::new(), value);
                        continue;
                    }
                    if let Some(p) = key_str.strip_prefix("xmlns:") {
                        scope.insert(p.to_string(), value);
                        continue;
                    }
                    let (prefix, local) = match key_str.split_once(':') {
                        Some((p, n)) => (Some(p.to_string()), n.to_string()),
                        None => (None, key_str.clone()),
                    };
                    attrs.push((prefix.unwrap_or_default(), local, value));
                }

                let name_bytes = bs.name();
                let name_str = String::from_utf8_lossy(name_bytes.as_ref()).to_string();
                let (prefix, local) = match name_str.split_once(':') {
                    Some((p, n)) => (Some(p.to_string()), n.to_string()),
                    None => (None, name_str.clone()),
                };

                // The element's own namespace declarations apply to the
                // element name itself and to its attributes, so push the
                // scope before resolving anything.
                scopes.push(scope);
                let ns_uri = resolve_prefix(&scopes, prefix.as_deref());
                let resolved_attrs: Vec<(String, String, String)> = attrs
                    .into_iter()
                    .map(|(p, n, v)| {
                        let ns = if p.is_empty() {
                            String::new() // unprefixed attribute: no namespace
                        } else {
                            resolve_prefix(&scopes, Some(&p))
                        };
                        (ns, n, v)
                    })
                    .collect();

                stack.push((
                    Elem {
                        ns: ns_uri,
                        local,
                        attrs: resolved_attrs,
                        text: String::new(),
                        children: Vec::new(),
                    },
                    scopes.len() - 1,
                ));
            }
            Event::Empty(bs) => {
                // Self-closing element: same handling, immediately complete.
                let mut scope: HashMap<String, String> = HashMap::new();
                let mut attrs: Vec<(String, String, String)> = Vec::new();
                for a in bs.attributes() {
                    let a = a.map_err(|e| XmpError::BadFormat(format!("bad attribute: {e}")))?;
                    let key_str = String::from_utf8_lossy(a.key.as_ref()).to_string();
                    let value = a
                        .unescape_value()
                        .map_err(|e| XmpError::BadFormat(format!("bad attribute value: {e}")))?
                        .to_string();
                    if key_str == "xmlns" {
                        scope.insert(String::new(), value);
                        continue;
                    }
                    if let Some(p) = key_str.strip_prefix("xmlns:") {
                        scope.insert(p.to_string(), value);
                        continue;
                    }
                    let (prefix, local) = match key_str.split_once(':') {
                        Some((p, n)) => (Some(p.to_string()), n.to_string()),
                        None => (None, key_str.clone()),
                    };
                    attrs.push((prefix.unwrap_or_default(), local, value));
                }
                let name_str = String::from_utf8_lossy(bs.name().as_ref()).to_string();
                let (prefix, local) = match name_str.split_once(':') {
                    Some((p, n)) => (Some(p.to_string()), n.to_string()),
                    None => (None, name_str.clone()),
                };

                // Own declarations apply to the element name and attributes.
                scopes.push(scope);
                let ns_uri = resolve_prefix(&scopes, prefix.as_deref());
                let resolved_attrs: Vec<(String, String, String)> = attrs
                    .into_iter()
                    .map(|(p, n, v)| {
                        let ns = if p.is_empty() {
                            String::new()
                        } else {
                            resolve_prefix(&scopes, Some(&p))
                        };
                        (ns, n, v)
                    })
                    .collect();
                scopes.pop();

                let elem = Elem {
                    ns: ns_uri,
                    local,
                    attrs: resolved_attrs,
                    text: String::new(),
                    children: Vec::new(),
                };
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(elem);
                } else {
                    roots.push(elem);
                }
            }
            Event::End(_) => {
                let (elem, scope_idx) =
                    stack.pop().ok_or_else(|| XmpError::BadFormat("unbalanced XML".into()))?;
                while scopes.len() > scope_idx {
                    scopes.pop();
                }
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(elem);
                } else {
                    roots.push(elem);
                }
            }
            Event::Text(t) => {
                let text = t
                    .unescape()
                    .map_err(|e| XmpError::BadFormat(format!("bad text: {e}")))?
                    .to_string();
                if let Some((elem, _)) = stack.last_mut() {
                    elem.text.push_str(&text);
                }
            }
            Event::CData(t) => {
                // quick-xml exposes CDATA content raw.
                let text = String::from_utf8_lossy(t.as_ref()).to_string();
                if let Some((elem, _)) = stack.last_mut() {
                    elem.text.push_str(&text);
                }
            }
            Event::Eof => break,
            // PI (xpacket), Comment, Decl, DocType — skip.
            _ => {}
        }
    }

    if !stack.is_empty() {
        return Err(XmpError::BadFormat("unbalanced XML elements".into()));
    }

    Ok(roots)
}

fn resolve_prefix(scopes: &[HashMap<String, String>], prefix: Option<&str>) -> String {
    match prefix {
        None => scopes
            .iter()
            .rev()
            .find_map(|s| s.get("").cloned())
            .unwrap_or_default(),
        Some("xml") => ns::NS_XML.to_string(),
        Some("xmlns") => ns::NS_XML.to_string(), // rare xmlns:xml redecl
        Some(p) => scopes
            .iter()
            .rev()
            .find_map(|s| s.get(p).cloned())
            // Undeclared prefixes fall back to the global registry, matching
            // the Adobe toolkit's parser behavior (real packets occasionally
            // carry undeclared attribute prefixes such as `stEvt:`).
            .or_else(|| ns::lookup_prefix_uri(p))
            .unwrap_or_default(),
    }
}

// ---------------------------------------------------------------------------
// Interpretation: element tree -> XMP node tree
// ---------------------------------------------------------------------------

const RDF_ABOUT: &str = "about";
const RDF_PARSETYPE: &str = "parseType";
const RDF_VALUE: &str = "value";
const RDF_TYPE: &str = "type";
const RDF_ID: &str = "ID";
const RDF_NODEID: &str = "nodeID";
const RDF_DATATYPE: &str = "datatype";
const RDF_LI: &str = "li";

/// Is this attribute structural (not a property / field / qualifier)?
fn is_rdf_meta_attr(ns_uri: &str, local: &str) -> bool {
    (ns_uri == ns::NS_RDF
        && (local == RDF_ABOUT
            || local == RDF_PARSETYPE
            || local == RDF_ID
            || local == RDF_NODEID
            || local == RDF_DATATYPE))
    || (ns_uri == ns::NS_XML && local == "lang" && false) // xml:lang IS a qualifier
    || (ns_uri.is_empty() && (local == "xmlns"))
}

/// Interpret an `rdf:RDF` root (or a bare `rdf:Description` list).
fn interpret(roots: &[Elem]) -> Result<XmpMeta, XmpError> {
    let mut meta = XmpMeta::new();

    // Find the rdf:RDF element (possibly wrapped in x:xmpmeta).
    let mut rdf_elems: Vec<&Elem> = Vec::new();
    for r in roots {
        if r.is_rdf("RDF") {
            rdf_elems.push(r);
        } else {
            // The x:xmpmeta wrapper contributes children.
            for c in &r.children {
                if c.is_rdf("RDF") {
                    rdf_elems.push(c);
                }
            }
        }
    }

    // Bare rdf:Description handling.
    if rdf_elems.is_empty() {
        for r in roots {
            if r.is_rdf("Description") {
                interpret_description(r, &mut meta);
                return Ok(meta);
            }
            for c in &r.children {
                if c.is_rdf("Description") {
                    interpret_description(c, &mut meta);
                    return Ok(meta);
                }
            }
        }
        return Err(XmpError::BadFormat("no rdf:RDF element found".into()));
    }

    for rdf in rdf_elems {
        for desc in &rdf.children {
            if desc.is_rdf("Description") {
                interpret_description(desc, &mut meta);
            }
            // Other children of rdf:RDF are not part of the XMP data model
            // (per spec, rdf:Description is the only legal child we model).
        }
    }

    Ok(meta)
}

fn interpret_description(desc: &Elem, meta: &mut XmpMeta) {
    // rdf:about of the first description wins.
    if meta.about.is_empty() {
        if let Some(about) = desc.attr(ns::NS_RDF, RDF_ABOUT) {
            meta.about = about.to_string();
        }
    }

    // Attribute-form properties.
    for (a_ns, a_local, value) in &desc.attrs {
        if is_rdf_meta_attr(a_ns, a_local) || (a_ns == ns::NS_XML && a_local == "lang") {
            continue;
        }
        if a_ns.is_empty() {
            continue; // unprefixed attribute: not a namespaced property
        }
        set_property_node(meta, a_ns, a_local, XmpNode::simple(value.clone()));
    }

    // Element-form properties.
    for child in &desc.children {
        let (prop_ns, prop_name) = child.q();
        if prop_ns.is_empty() {
            continue;
        }
        let node = interpret_property(child);
        set_property_node(meta, &prop_ns, &prop_name, node);
    }
}

fn set_property_node(meta: &mut XmpMeta, schema_ns: &str, name: &str, node: XmpNode) {
    meta.schema_mut(schema_ns).insert(name.to_string(), node);
}

/// Interpret a property element (direct child of rdf:Description) or an
/// rdf:li item or a struct field — all share the same content model.
fn interpret_property(el: &Elem) -> XmpNode {
    // Qualifier attributes (xml:lang, rdf:ID etc.).
    let mut qualifiers: Vec<crate::node::XmpQualifier> = Vec::new();
    let mut struct_attrs: Vec<(String, String, String)> = Vec::new();

    for (a_ns, a_local, value) in &el.attrs {
        if a_ns == ns::NS_XML && a_local == "lang" {
            qualifiers.push(crate::node::XmpQualifier {
                ns: ns::NS_XML.to_string(),
                name: "lang".to_string(),
                value: value.clone(),
            });
        } else if a_ns == ns::NS_RDF
            && (local_matches(a_local, RDF_ID) || local_matches(a_local, RDF_NODEID) || local_matches(a_local, RDF_DATATYPE))
        {
            // rdf:ID / nodeID / datatype are annotations, not fields.
        } else if !a_ns.is_empty() {
            struct_attrs.push((a_ns.clone(), a_local.clone(), value.clone()));
        }
    }

    // rdf:parseType="Resource": struct container.
    if el.attr(ns::NS_RDF, RDF_PARSETYPE).map(|v| v.trim()) == Some("Resource") {
        let mut node = interpret_struct_content(el);
        node.qualifiers = qualifiers;
        return node;
    }

    // Children analysis.
    let mut array_form: Option<ArrayForm> = None;
    let mut rdf_value: Option<String> = None;
    let mut extra_children: Vec<&Elem> = Vec::new();
    let mut value_qualifiers: Vec<crate::node::XmpQualifier> = qualifiers;

    for child in &el.children {
        if child.is_rdf("Bag") {
            array_form = Some(ArrayForm::Bag);
        } else if child.is_rdf("Seq") {
            array_form = Some(ArrayForm::Seq);
        } else if child.is_rdf("Alt") {
            array_form = Some(ArrayForm::Alt);
        } else if child.is_rdf(RDF_VALUE) {
            rdf_value = Some(child.text.trim().to_string());
        } else if child.is_rdf(RDF_TYPE) {
            value_qualifiers.push(crate::node::XmpQualifier {
                ns: ns::NS_RDF.to_string(),
                name: RDF_TYPE.to_string(),
                value: child.attr(ns::NS_RDF, RDF_VALUE).unwrap_or("").to_string(),
            });
        } else {
            extra_children.push(child);
        }
    }

    // Array property.
    if let Some(form) = array_form {
        // Find the actual container element (Bag/Seq/Alt).
        let container = el
            .children
            .iter()
            .find(|c| c.is_rdf("Bag") || c.is_rdf("Seq") || c.is_rdf("Alt"))
            .unwrap();
        let items: Vec<XmpNode> = container
            .children
            .iter()
            .filter(|c| c.is_rdf(RDF_LI))
            .map(|li| interpret_li(li))
            .collect();
        let mut node = XmpNode {
            value: NodeValue::Array { form, items },
            qualifiers: value_qualifiers,
        };
        // An array with attribute-form struct fields on the container is
        // invalid XMP; ignore attributes in that case.
        let _ = &mut node;
        return node;
    }

    // rdf:value form: <prop><rdf:value>text</rdf:value><ns:q>…</ns:q></prop>
    if rdf_value.is_some() || (!extra_children.is_empty() && el.attr(ns::NS_RDF, RDF_PARSETYPE).is_some())
    {
        // Value + qualifier form.
        let mut node = XmpNode {
            value: NodeValue::Simple(rdf_value.unwrap_or_default()),
            qualifiers: value_qualifiers,
        };
        for q in extra_children {
            node.qualifiers.push(crate::node::XmpQualifier {
                ns: q.ns.clone(),
                name: q.local.clone(),
                value: q.text.trim().to_string(),
            });
        }
        return node;
    }

    // Attribute-form struct: attributes present and no relevant children.
    if !struct_attrs.is_empty() && extra_children.is_empty() {
        let fields: Vec<XmpField> = struct_attrs
            .into_iter()
            .map(|(f_ns, f_name, v)| XmpField {
                ns: f_ns,
                name: f_name,
                node: XmpNode::simple(v),
            })
            .collect();
        return XmpNode {
            value: NodeValue::Struct(fields),
            qualifiers: value_qualifiers,
        };
    }

    // Element-form struct: children are fields.
    if !extra_children.is_empty() {
        let mut node = interpret_struct_content(el);
        node.qualifiers = value_qualifiers;
        return node;
    }

    // Nested rdf:Description shorthand: struct from its attributes.
    // (Handled by attribute-form branch above when attrs are on this element.)

    // Simple: text content.
    let text = el.text.trim().to_string();
    XmpNode {
        value: NodeValue::Simple(text),
        qualifiers: value_qualifiers,
    }
}

/// Interpret `rdf:li` items.
fn interpret_li(li: &Elem) -> XmpNode {
    interpret_property(li)
}

/// Build a struct node from a container's children (element-form fields).
fn interpret_struct_content(el: &Elem) -> XmpNode {
    let mut fields: Vec<XmpField> = Vec::new();

    // Nested rdf:Description children contribute attribute-form fields.
    for child in &el.children {
        if child.is_rdf("Description") {
            for (a_ns, a_local, value) in &child.attrs {
                if is_rdf_meta_attr(a_ns, a_local) || (a_ns == ns::NS_XML && a_local == "lang") {
                    continue;
                }
                if !a_ns.is_empty() {
                    fields.push(XmpField {
                        ns: a_ns.clone(),
                        name: a_local.clone(),
                        node: XmpNode::simple(value.clone()),
                    });
                }
            }
            for gc in &child.children {
                let (f_ns, f_name) = gc.q();
                if !f_ns.is_empty() {
                    fields.push(XmpField {
                        ns: f_ns,
                        name: f_name,
                        node: interpret_property(gc),
                    });
                }
            }
        }
    }

    // Element-form fields.
    for child in &el.children {
        if child.is_rdf("Description") {
            continue; // handled above
        }
        let (f_ns, f_name) = child.q();
        if f_ns.is_empty() {
            continue;
        }
        fields.push(XmpField {
            ns: f_ns,
            name: f_name,
            node: interpret_property(child),
        });
    }

    // Attribute-form fields on the same element.
    for (a_ns, a_local, value) in &el.attrs {
        if is_rdf_meta_attr(a_ns, a_local) || (a_ns == ns::NS_XML && a_local == "lang") {
            continue;
        }
        if !a_ns.is_empty() {
            fields.push(XmpField {
                ns: a_ns.clone(),
                name: a_local.clone(),
                node: XmpNode::simple(value.clone()),
            });
        }
    }

    XmpNode {
        value: NodeValue::Struct(fields),
        qualifiers: Vec::new(),
    }
}

fn local_matches(a: &str, b: &str) -> bool {
    a == b
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse an XMP buffer into an [`XmpMeta`].
pub fn parse_from_buffer(data: &[u8]) -> Result<XmpMeta, XmpError> {
    // Strip a UTF-8 BOM if present.
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(data);
    let roots = parse_xml_tree(data)?;
    interpret(&roots)
}

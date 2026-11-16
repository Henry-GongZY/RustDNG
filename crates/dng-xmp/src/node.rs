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
//! XMP node tree data model.

use crate::error::XmpError;
use crate::ns;
use crate::path::PathStep;
use crate::serialize::XmpSerializeOptions;

/// Array form (XMP `rdf:Bag` / `rdf:Seq` / `rdf:Alt`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrayForm {
    Bag,
    Seq,
    Alt,
}

impl ArrayForm {
    pub fn element_name(self) -> &'static str {
        match self {
            ArrayForm::Bag => "rdf:Bag",
            ArrayForm::Seq => "rdf:Seq",
            ArrayForm::Alt => "rdf:Alt",
        }
    }
}

/// A property value: simple string, struct, or array.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeValue {
    Simple(String),
    /// Struct fields in document order; each field carries its own namespace.
    Struct(Vec<XmpField>),
    Array {
        form: ArrayForm,
        items: Vec<XmpNode>,
    },
}

impl Default for NodeValue {
    fn default() -> Self {
        NodeValue::Simple(String::new())
    }
}

/// A qualifier (`?ns:name`), e.g. `xml:lang` on a LangAlt item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmpQualifier {
    pub ns: String,
    pub name: String,
    pub value: String,
}

/// A struct field.
#[derive(Debug, Clone, PartialEq)]
pub struct XmpField {
    pub ns: String,
    pub name: String,
    pub node: XmpNode,
}

/// An XMP property node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XmpNode {
    pub value: NodeValue,
    pub qualifiers: Vec<XmpQualifier>,
}

impl XmpNode {
    pub fn simple(value: impl Into<String>) -> Self {
        XmpNode {
            value: NodeValue::Simple(value.into()),
            qualifiers: Vec::new(),
        }
    }

    pub fn as_simple(&self) -> Option<&str> {
        match &self.value {
            NodeValue::Simple(s) => Some(s),
            _ => None,
        }
    }

    pub fn is_simple(&self) -> bool {
        matches!(self.value, NodeValue::Simple(_))
    }

    pub fn items(&self) -> Option<&[XmpNode]> {
        match &self.value {
            NodeValue::Array { items, .. } => Some(items),
            _ => None,
        }
    }

    pub fn fields(&self) -> Option<&[XmpField]> {
        match &self.value {
            NodeValue::Struct(fields) => Some(fields),
            _ => None,
        }
    }

    /// Find a qualifier value by (ns, name).
    pub fn qualifier(&self, ns_uri: &str, name: &str) -> Option<&str> {
        self.qualifiers
            .iter()
            .find(|q| q.ns == ns_uri && q.name == name)
            .map(|q| q.value.as_str())
    }

    /// Set (replace or add) a qualifier.
    pub fn set_qualifier(&mut self, ns_uri: &str, name: &str, value: &str) {
        if let Some(q) = self
            .qualifiers
            .iter_mut()
            .find(|q| q.ns == ns_uri && q.name == name)
        {
            q.value = value.to_string();
        } else {
            self.qualifiers.push(XmpQualifier {
                ns: ns_uri.to_string(),
                name: name.to_string(),
                value: value.to_string(),
            });
        }
    }
}

/// The full XMP metadata tree: `rdf:about` plus one property map per schema.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct XmpMeta {
    /// The `rdf:about` value.
    pub about: String,
    /// Schema namespace -> (property local name -> node), both sorted for
    /// deterministic serialization.
    pub schemas: std::collections::BTreeMap<String, std::collections::BTreeMap<String, XmpNode>>,
}

impl XmpMeta {
    pub fn new() -> Self {
        XmpMeta::default()
    }

    /// Access the property map of a schema, creating it if missing.
    pub fn schema_mut(
        &mut self,
        schema_ns: &str,
    ) -> &mut std::collections::BTreeMap<String, XmpNode> {
        self.schemas.entry(schema_ns.to_string()).or_default()
    }

    pub fn schema(&self, schema_ns: &str) -> Option<&std::collections::BTreeMap<String, XmpNode>> {
        self.schemas.get(schema_ns)
    }

    /// True if the schema exists and holds at least one property.
    pub fn schema_has_properties(&self, schema_ns: &str) -> bool {
        self.schemas
            .get(schema_ns)
            .map(|m| !m.is_empty())
            .unwrap_or(false)
    }

    /// Remove all properties of a schema (mirrors `RemoveProperties(ns)`).
    pub fn remove_schema_properties(&mut self, schema_ns: &str) {
        self.schemas.remove(schema_ns);
    }
}

// ---------------------------------------------------------------------------
// Tree navigation shared by the path-based accessors (fully safe).
// ---------------------------------------------------------------------------

/// Immutable walk of `steps` from the schema root. Returns `None` when the
/// path does not exist.
pub(crate) fn follow_read<'a>(
    meta: &'a XmpMeta,
    schema_ns: &str,
    steps: &[PathStep],
) -> Option<&'a XmpNode> {
    let root = steps.first()?;
    let root_name = match root {
        PathStep::Root { name, .. } => name,
        _ => return None,
    };
    let mut current = meta.schema(schema_ns)?.get(root_name)?;

    for step in &steps[1..] {
        current = match step {
            PathStep::Index(i) => {
                if *i == 0 {
                    return None;
                }
                current.items()?.get(*i - 1)?
            }
            PathStep::Field { ns, name } => {
                let field_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
                let f = current
                    .fields()?
                    .iter()
                    .find(|f| f.ns == field_ns && f.name == *name)?;
                &f.node
            }
            PathStep::Qualifier { ns, name } => {
                let q_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
                // Qualifier steps resolve to the node that carries the
                // qualifier; dedicated accessors handle the qualifier value.
                if current.qualifier(&q_ns, name).is_some() {
                    current
                } else {
                    return None;
                }
            }
            PathStep::Root { .. } => return None,
        };
    }

    Some(current)
}

/// Mutable walk of `steps` from the schema root, creating intermediate
/// containers as needed.
pub(crate) fn follow_create<'a>(
    meta: &'a mut XmpMeta,
    schema_ns: &str,
    steps: &[PathStep],
) -> Option<&'a mut XmpNode> {
    let root = steps.first()?;
    let root_name = match root {
        PathStep::Root { name, .. } => name.clone(),
        _ => return None,
    };

    let props = meta.schema_mut(schema_ns);
    let mut current = props.entry(root_name).or_default();

    for step in &steps[1..] {
        current = match step {
            PathStep::Index(i) => {
                if *i == 0 {
                    return None;
                }
                let idx = *i - 1;
                // Ensure an array of the right size exists.
                if !matches!(current.value, NodeValue::Array { .. }) {
                    current.value = NodeValue::Array {
                        form: ArrayForm::Bag,
                        items: Vec::new(),
                    };
                }
                if let NodeValue::Array { items, .. } = &mut current.value {
                    while items.len() <= idx {
                        items.push(XmpNode::simple(""));
                    }
                    &mut items[idx]
                } else {
                    unreachable!()
                }
            }
            PathStep::Field { ns, name } => {
                let field_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
                let name = name.clone();
                if !matches!(current.value, NodeValue::Struct(_)) {
                    current.value = NodeValue::Struct(Vec::new());
                }
                if let NodeValue::Struct(fields) = &mut current.value {
                    let pos = fields
                        .iter()
                        .position(|f| f.ns == field_ns && f.name == name);
                    match pos {
                        Some(p) => &mut fields[p].node,
                        None => {
                            fields.push(XmpField {
                                ns: field_ns,
                                name,
                                node: XmpNode::default(),
                            });
                            &mut fields.last_mut().unwrap().node
                        }
                    }
                } else {
                    unreachable!()
                }
            }
            PathStep::Qualifier { ns, name } => {
                let q_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
                let name = name.clone();
                // Create the qualifier (empty value); navigation stops here.
                current.set_qualifier(&q_ns, &name, "");
                return Some(current);
            }
            PathStep::Root { .. } => return None,
        };
    }

    Some(current)
}

/// Navigate to the node, plus its parent container, for deletion.
/// Returns `(parent_kind, handle)` such that the caller can remove the
/// final step from the parent.
pub(crate) fn remove_last_step(
    meta: &mut XmpMeta,
    schema_ns: &str,
    steps: &[PathStep],
) -> Result<bool, XmpError> {
    if steps.is_empty() {
        return Err(XmpError::BadPath("empty path".into()));
    }

    let root = &steps[0];
    let root_name = match root {
        PathStep::Root { name, .. } => name.clone(),
        _ => return Err(XmpError::BadPath("path must start with a root property".into())),
    };

    // Deleting the root step: remove from schema map directly.
    if steps.len() == 1 {
        if let Some(schema) = meta.schemas.get_mut(schema_ns) {
            return Ok(schema.remove(&root_name).is_some());
        }
        return Ok(false);
    }

    // Walk to the parent of the last step (creating intermediates is NOT
    // done for deletion).
    let parent_steps = &steps[..steps.len() - 1];
    let last = &steps[steps.len() - 1];

    let props = meta.schema_mut(schema_ns);
    let mut current = match props.get_mut(&root_name) {
        Some(n) => n,
        None => return Ok(false),
    };

    for step in parent_steps.iter().skip(1) {
        current = match step {
            PathStep::Index(i) => {
                if *i == 0 {
                    return Ok(false);
                }
                match current.items_mut() {
                    Some(items) if items.len() >= *i => &mut items[*i - 1],
                    _ => return Ok(false),
                }
            }
            PathStep::Field { ns, name } => {
                let field_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
                match current.fields_mut() {
                    Some(fields) => {
                        let pos = fields
                            .iter()
                            .position(|f| f.ns == field_ns && f.name == *name);
                        match pos {
                            Some(p) => &mut fields[p].node,
                            None => return Ok(false),
                        }
                    }
                    None => return Ok(false),
                }
            }
            PathStep::Qualifier { .. } => return Ok(false),
            PathStep::Root { .. } => return Ok(false),
        };
    }

    // `current` is now the parent of the last step.
    Ok(match last {
        PathStep::Root { .. } => false, // unreachable
        PathStep::Index(i) => {
            if *i == 0 {
                return Ok(false);
            }
            if let NodeValue::Array { items, .. } = &mut current.value {
                if items.len() >= *i {
                    items.remove(*i - 1);
                    true
                } else {
                    false
                }
            } else {
                false
            }
        }
        PathStep::Field { ns, name } => {
            let field_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
            if let NodeValue::Struct(fields) = &mut current.value {
                let before = fields.len();
                fields.retain(|f| !(f.ns == field_ns && f.name == *name));
                fields.len() != before
            } else {
                false
            }
        }
        PathStep::Qualifier { ns, name } => {
            let q_ns = ns.clone().unwrap_or_else(|| schema_ns.to_string());
            let before = current.qualifiers.len();
            current
                .qualifiers
                .retain(|q| !(q.ns == q_ns && q.name == *name));
            current.qualifiers.len() != before
        }
    })
}

impl XmpNode {
    pub(crate) fn items_mut(&mut self) -> Option<&mut Vec<XmpNode>> {
        match &mut self.value {
            NodeValue::Array { items, .. } => Some(items),
            _ => None,
        }
    }

    pub(crate) fn fields_mut(&mut self) -> Option<&mut Vec<XmpField>> {
        match &mut self.value {
            NodeValue::Struct(fields) => Some(fields),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Public accessor API (mirrors the SXMPMeta subset the DNG SDK uses)
// ---------------------------------------------------------------------------

impl XmpMeta {
    /// Parse an XMP buffer (packet, `x:xmpmeta` wrapper, or bare `rdf:RDF`).
    pub fn parse_from_buffer(data: &[u8]) -> Result<Self, XmpError> {
        crate::parse::parse_from_buffer(data)
    }

    /// Serialize to a full XMP packet (`<?xpacket …?>` wrapper).
    pub fn serialize_to_buffer(&self, opts: &XmpSerializeOptions) -> Result<Vec<u8>, XmpError> {
        crate::serialize::serialize_to_buffer(self, opts)
    }

    /// Serialize to a bare `rdf:RDF` (no packet wrapper) — mirrors
    /// `SerializeToBuffer` with `kXMP_OmitPacketWrapper`.
    pub fn serialize_rdf_only(&self, opts: &XmpSerializeOptions) -> Result<Vec<u8>, XmpError> {
        crate::serialize::serialize_rdf_only(self, opts)
    }

    // -- simple properties ------------------------------------------------

    /// Get a simple property value by path. Returns `None` if the path does
    /// not exist or the property is not a simple leaf.
    pub fn get_property(&self, schema_ns: &str, prop: &str) -> Option<String> {
        let steps = crate::path::parse_path(prop).ok()?;
        let node = crate::node::follow_read(self, schema_ns, &steps)?;
        match &node.value {
            NodeValue::Simple(s) => Some(s.clone()),
            _ => Some(String::new()),
        }
    }

    /// Set a simple property by path (creating intermediates as needed).
    pub fn set_property(&mut self, schema_ns: &str, prop: &str, value: &str) {
        let steps = match crate::path::parse_path(prop) {
            Ok(s) => s,
            Err(_) => return,
        };
        if let Some(node) = crate::node::follow_create(self, schema_ns, &steps) {
            node.value = NodeValue::Simple(value.to_string());
        }
    }

    /// Does the property exist (any form)?
    pub fn does_property_exist(&self, schema_ns: &str, prop: &str) -> bool {
        match crate::path::parse_path(prop) {
            Ok(steps) => crate::node::follow_read(self, schema_ns, &steps).is_some(),
            Err(_) => false,
        }
    }

    /// Delete a property (and its subtree).
    pub fn delete_property(&mut self, schema_ns: &str, prop: &str) {
        if let Ok(steps) = crate::path::parse_path(prop) {
            let _ = crate::node::remove_last_step(self, schema_ns, &steps);
        }
    }

    // -- qualifiers --------------------------------------------------------

    /// Get a qualifier value (`GetQualifier` in the toolkit).
    pub fn get_qualifier(
        &self,
        schema_ns: &str,
        prop: &str,
        qual_ns: &str,
        qual: &str,
    ) -> Option<String> {
        let steps = crate::path::parse_path(prop).ok()?;
        let node = crate::node::follow_read(self, schema_ns, &steps)?;
        node.qualifier(qual_ns, qual).map(|s| s.to_string())
    }

    /// Set a qualifier value (`SetQualifier` in the toolkit).
    pub fn set_qualifier(
        &mut self,
        schema_ns: &str,
        prop: &str,
        qual_ns: &str,
        qual: &str,
        value: &str,
    ) {
        if let Ok(steps) = crate::path::parse_path(prop) {
            if let Some(node) = crate::node::follow_create(self, schema_ns, &steps) {
                node.set_qualifier(qual_ns, qual, value);
            }
        }
    }

    // -- structs -----------------------------------------------------------

    /// Get a struct field value.
    pub fn get_struct_field(
        &self,
        schema_ns: &str,
        prop: &str,
        field_ns: &str,
        field: &str,
    ) -> Option<String> {
        let steps = crate::path::parse_path(prop).ok()?;
        let node = crate::node::follow_read(self, schema_ns, &steps)?;
        match &node.value {
            NodeValue::Struct(fields) => fields
                .iter()
                .find(|f| f.ns == field_ns && f.name == field)
                .and_then(|f| match &f.node.value {
                    NodeValue::Simple(s) => Some(s.clone()),
                    _ => Some(String::new()),
                }),
            _ => None,
        }
    }

    /// Set a struct field (creating the struct if needed).
    pub fn set_struct_field(
        &mut self,
        schema_ns: &str,
        prop: &str,
        field_ns: &str,
        field: &str,
        value: &str,
    ) {
        if let Ok(steps) = crate::path::parse_path(prop) {
            if let Some(node) = crate::node::follow_create(self, schema_ns, &steps) {
                if !matches!(node.value, NodeValue::Struct(_)) {
                    node.value = NodeValue::Struct(Vec::new());
                }
                if let NodeValue::Struct(fields) = &mut node.value {
                    let pos = fields
                        .iter()
                        .position(|f| f.ns == field_ns && f.name == field);
                    match pos {
                        Some(p) => fields[p].node.value = NodeValue::Simple(value.to_string()),
                        None => fields.push(XmpField {
                            ns: field_ns.to_string(),
                            name: field.to_string(),
                            node: XmpNode::simple(value),
                        }),
                    }
                }
            }
        }
    }

    /// Delete a struct field (`DeleteStructField`).
    pub fn delete_struct_field(
        &mut self,
        schema_ns: &str,
        prop: &str,
        field_ns: &str,
        field: &str,
    ) {
        // Compose the full path (prop steps + field step) and remove the
        // last step from its parent.
        if let Ok(mut steps) = crate::path::parse_path(prop) {
            steps.push(crate::path::PathStep::Field {
                ns: Some(field_ns.to_string()),
                name: field.to_string(),
            });
            let _ = crate::node::remove_last_step(self, schema_ns, &steps);
        }
    }

    // -- arrays ------------------------------------------------------------

    /// Get an array item (1-based index). Non-leaf items yield `Some("")`.
    pub fn get_array_item(&self, schema_ns: &str, prop: &str, index: usize) -> Option<String> {
        if index == 0 {
            return None;
        }
        let steps = crate::path::parse_path(prop).ok()?;
        let node = crate::node::follow_read(self, schema_ns, &steps)?;
        match &node.value {
            NodeValue::Array { items, .. } => {
                items.get(index - 1).map(|n| match &n.value {
                    NodeValue::Simple(s) => s.clone(),
                    _ => String::new(),
                })
            }
            _ => None,
        }
    }

    /// Set an array item (1-based). Appends when `index == count + 1`;
    /// out-of-bounds otherwise is a no-op (toolkit throws).
    pub fn set_array_item(&mut self, schema_ns: &str, prop: &str, index: usize, value: &str) {
        if index == 0 {
            return;
        }
        if let Ok(steps) = crate::path::parse_path(prop) {
            if let Some(node) = crate::node::follow_create(self, schema_ns, &steps) {
                if let NodeValue::Array { items, .. } = &mut node.value {
                    if index <= items.len() {
                        items[index - 1].value = NodeValue::Simple(value.to_string());
                    } else if index == items.len() + 1 {
                        items.push(XmpNode::simple(value));
                    }
                }
            }
        }
    }

    /// Append an item to an array, creating the array if needed
    /// (`AppendArrayItem`). `ordered == true` creates a Seq (ordered),
    /// `false` a Bag. Mirrors the toolkit's handling of "emptied" arrays:
    /// if the array holds a single empty dummy item it is replaced.
    pub fn append_array_item(
        &mut self,
        schema_ns: &str,
        prop: &str,
        ordered: bool,
        value: &str,
    ) {
        if let Ok(steps) = crate::path::parse_path(prop) {
            if let Some(node) = crate::node::follow_create(self, schema_ns, &steps) {
                if !matches!(node.value, NodeValue::Array { .. }) {
                    node.value = NodeValue::Array {
                        form: if ordered { ArrayForm::Seq } else { ArrayForm::Bag },
                        items: Vec::new(),
                    };
                }
                if let NodeValue::Array { items, .. } = &mut node.value {
                    // Dummy-item handling: single empty item gets replaced.
                    let is_dummy = items.len() == 1
                        && items[0].is_simple()
                        && items[0].as_simple() == Some("")
                        && items[0].qualifiers.is_empty();
                    if is_dummy {
                        items[0].value = NodeValue::Simple(value.to_string());
                    } else {
                        items.push(XmpNode::simple(value));
                    }
                }
            }
        }
    }

    /// Count the items of an array (0 when the property is not an array).
    pub fn count_array_items(&self, schema_ns: &str, prop: &str) -> usize {
        match crate::path::parse_path(prop) {
            Ok(steps) => match crate::node::follow_read(self, schema_ns, &steps) {
                Some(node) => match &node.value {
                    NodeValue::Array { items, .. } => items.len(),
                    _ => 0,
                },
                None => 0,
            },
            Err(_) => 0,
        }
    }

    // -- LangAlt / localized text -------------------------------------------

    /// Get the localized text for `lang` (exact match, then RFC-3066 style
    /// prefix match). `x-default` additionally understands the "empty
    /// array / empty item" equivalence noted in `dng_xmp_sdk.cpp`.
    pub fn get_localized_text(&self, schema_ns: &str, prop: &str, lang: &str) -> Option<String> {
        let steps = crate::path::parse_path(prop).ok()?;
        let node = crate::node::follow_read(self, schema_ns, &steps)?;
        let lang_norm = normalize_lang(lang);

        if let NodeValue::Array { form, items } = &node.value {
            if *form != ArrayForm::Alt {
                return None;
            }
            // Exact match (case-insensitive).
            for it in items {
                if let Some(l) = it.qualifier(ns::NS_XML, "lang") {
                    if normalize_lang(l) == lang_norm {
                        return Some(it.as_simple().unwrap_or("").to_string());
                    }
                }
            }
            // Generic prefix match: requested "en" matches "en-us".
            if lang_norm != "x-default" {
                for it in items {
                    if let Some(l) = it.qualifier(ns::NS_XML, "lang") {
                        let ln = normalize_lang(l);
                        if ln.starts_with(&format!("{lang_norm}-")) {
                            return Some(it.as_simple().unwrap_or("").to_string());
                        }
                    }
                }
                return None;
            }
            // x-default special cases (see dng_xmp_sdk.cpp GetAltLangDefault):
            // an empty alt-lang array (no x-default item) or an empty simple
            // property are both reported as an empty default text.
            if items.is_empty() {
                return Some(String::new());
            }
            if let Some(only) = items.first() {
                if items.len() == 1 && only.is_simple() && only.as_simple() == Some("") {
                    return Some(String::new());
                }
            }
            None
        } else if let NodeValue::Simple(s) = &node.value {
            // Toolkit fallback: a simple empty value behaves like an empty
            // default language item.
            if lang_norm == "x-default" && s.is_empty() {
                return Some(String::new());
            }
            None
        } else {
            None
        }
    }

    /// Set the localized text for `lang`, following the XMP LangAlt rules:
    /// creates the Alt array when missing; replaces the existing item for
    /// that language; `x-default` items stay first; other languages are
    /// appended after `x-default`.
    pub fn set_localized_text(&mut self, schema_ns: &str, prop: &str, lang: &str, value: &str) {
        let steps = match crate::path::parse_path(prop) {
            Ok(s) => s,
            Err(_) => return,
        };
        let lang_norm = normalize_lang(lang);

        let node = match crate::node::follow_create(self, schema_ns, &steps) {
            Some(n) => n,
            None => return,
        };

        if !matches!(node.value, NodeValue::Array { form: ArrayForm::Alt, .. }) {
            node.value = NodeValue::Array {
                form: ArrayForm::Alt,
                items: Vec::new(),
            };
        }

        if let NodeValue::Array { form, items } = &mut node.value {
            debug_assert!(*form == ArrayForm::Alt);
            // Replace existing item for this language.
            for it in items.iter_mut() {
                if let Some(l) = it.qualifier(ns::NS_XML, "lang") {
                    if normalize_lang(l) == lang_norm {
                        it.value = NodeValue::Simple(value.to_string());
                        return;
                    }
                }
            }
            let mut new_item = XmpNode::simple(value);
            new_item.set_qualifier(ns::NS_XML, "lang", lang);
            if lang_norm == "x-default" {
                items.insert(0, new_item);
            } else {
                items.push(new_item);
            }
        }
    }

    // -- object name ---------------------------------------------------------

    /// Set the `rdf:about` value (`set_object_name`).
    pub fn set_object_name(&mut self, about: &str) {
        self.about = about.to_string();
    }

    /// Get the `rdf:about` value (`object_name`).
    pub fn object_name(&self) -> String {
        self.about.clone()
    }

    // -- namespaces -----------------------------------------------------------

    /// Register a namespace prefix (global registry, mirrors
    /// `SXMPMeta::RegisterNamespace`).
    pub fn register_namespace(prefix: &str, uri: &str) {
        ns::register_namespace(prefix, uri);
    }

    /// Look up the preferred prefix for a URI.
    pub fn namespace_prefix(uri: &str) -> Option<String> {
        ns::namespace_prefix(uri)
    }

    // -- schema-level helpers the SDK uses -------------------------------------

    /// Does this schema hold at least one property? (`HasNameSpace`)
    pub fn has_namespace(&self, schema_ns: &str) -> bool {
        self.schema_has_properties(schema_ns)
    }

    /// Remove every property of a schema (`SXMPUtils::RemoveProperties` with
    /// a schema name).
    pub fn remove_properties(&mut self, schema_ns: &str) {
        self.remove_schema_properties(schema_ns);
    }

    /// Iterate `(schema_ns, path)` pairs of all properties, depth-first
    /// (mirrors `IteratePaths` with `justChildren=false`). Paths are
    /// schema-prefixed forms like `dc:creator[1]`.
    pub fn iterate_paths(&self, schema_ns: Option<&str>) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let schemas: Vec<(&String, &std::collections::BTreeMap<String, XmpNode>)> =
            match schema_ns {
                Some(ns) => self.schemas.iter().filter(|(u, _)| *u == ns).collect(),
                None => self.schemas.iter().collect(),
            };
        for (uri, props) in schemas {
            let prefix = ns::namespace_prefix(uri).unwrap_or_else(|| "ns".to_string());
            for (name, node) in props {
                collect_iter_paths(&prefix, name, node, &mut out);
            }
        }
        out
    }
}

/// Depth-first path collection for `iterate_paths`.
fn collect_iter_paths(prefix: &str, name: &str, node: &XmpNode, out: &mut Vec<(String, String)>) {
    let root = format!("{prefix}:{name}");
    match &node.value {
        NodeValue::Simple(_) => out.push((root.clone(), String::new())),
        NodeValue::Struct(fields) => {
            out.push((root.clone(), "struct".into()));
            for f in fields {
                let fp = ns::namespace_prefix(&f.ns).unwrap_or_else(|| "ns".to_string());
                let child = format!("{root}/{fp}:{}", f.name);
                collect_iter_paths_child(&child, &f.node, out);
            }
        }
        NodeValue::Array { items, .. } => {
            out.push((root.clone(), "array".into()));
            for (i, it) in items.iter().enumerate() {
                let child = format!("{root}[{}]", i + 1);
                collect_iter_paths_child(&child, it, out);
            }
        }
    }
}

fn collect_iter_paths_child(path: &str, node: &XmpNode, out: &mut Vec<(String, String)>) {
    match &node.value {
        NodeValue::Simple(_) => out.push((path.to_string(), String::new())),
        NodeValue::Struct(fields) => {
            out.push((path.to_string(), "struct".into()));
            for f in fields {
                let fp = ns::namespace_prefix(&f.ns).unwrap_or_else(|| "ns".to_string());
                let child = format!("{path}/{fp}:{}", f.name);
                collect_iter_paths_child(&child, &f.node, out);
            }
        }
        NodeValue::Array { items, .. } => {
            out.push((path.to_string(), "array".into()));
            for (i, it) in items.iter().enumerate() {
                let child = format!("{path}[{}]", i + 1);
                collect_iter_paths_child(&child, it, out);
            }
        }
    }
}

/// Normalize a language code for comparison (lowercase, keep x-default).
fn normalize_lang(lang: &str) -> String {
    if lang.eq_ignore_ascii_case("x-default") {
        "x-default".to_string()
    } else {
        lang.to_lowercase()
    }
}

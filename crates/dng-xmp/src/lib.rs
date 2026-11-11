//! `dng-xmp` — pure-Rust XMP engine for RustDNG.
//!
//! Replaces the Adobe XMP Toolkit (`SXMPMeta`) usage in the DNG SDK
//! (see `dng_xmp_sdk.cpp`, `dng_xmp.cpp`, `dng_exif.cpp` in the SDK source).
//!
//! Implements the subset of the toolkit API surface the SDK actually uses:
//!
//! - node-tree data model (simple / struct / Bag-Seq-Alt / LangAlt + qualifiers)
//! - `parse_from_buffer`: full packet (`<?xpacket …?>`), `x:xmpmeta` wrapper,
//!   or bare `rdf:RDF`; element form, attribute form, `rdf:parseType="Resource"`,
//!   nested `rdf:Description` struct shorthand, `rdf:value`/`rdf:type`
//! - `serialize_to_buffer`: spec-conformant RDF/XML (XMP spec part 2 "normal"
//!   and "compact" forms, part 3 packet wrapper/padding conventions)
//! - the namespace registry with the URIs the SDK registers (`XMPCore`
//!   constants verbatim)
//! - property access by path: `ns:root`, bare local names, `ns:root[i]`
//!   (1-based), `ns:root/ns:field`, `?ns:qualifier`, and combinations
//!
//! Unknown namespaces/properties are preserved on round-trip (the node tree
//! stores everything the parser sees).

#![allow(non_upper_case_globals)]

pub mod error;
pub mod node;
pub mod ns;
pub mod parse;
pub mod path;
pub mod serialize;

pub use error::XmpError;
pub use node::{ArrayForm, NodeValue, XmpField, XmpMeta, XmpNode, XmpQualifier};
pub use serialize::XmpSerializeOptions;

// Re-export the namespace constants at crate root for convenience.
pub use ns::*;

#[cfg(test)]
mod tests;

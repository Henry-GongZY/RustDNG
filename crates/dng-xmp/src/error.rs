//! Error type for the dng-xmp engine.

use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmpError {
    /// Input is not valid XMP / RDF-XML.
    BadFormat(String),
    /// A path expression could not be parsed.
    BadPath(String),
    /// Serialization failed.
    Serialize(String),
    /// Feature out of scope for this engine.
    Unsupported(String),
}

impl fmt::Display for XmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            XmpError::BadFormat(s) => write!(f, "bad XMP format: {s}"),
            XmpError::BadPath(s) => write!(f, "bad XMP path: {s}"),
            XmpError::Serialize(s) => write!(f, "XMP serialization error: {s}"),
            XmpError::Unsupported(s) => write!(f, "unsupported XMP feature: {s}"),
        }
    }
}

impl std::error::Error for XmpError {}

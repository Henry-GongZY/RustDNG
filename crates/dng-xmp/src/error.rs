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

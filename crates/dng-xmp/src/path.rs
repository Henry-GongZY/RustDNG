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
//! XMP path expression parsing (the subset used by the DNG SDK).
//!
//! Grammar (steps separated by `/`):
//! - root step: `name` (bare local name) or `ns:name`
//! - array step: `[i]` (1-based index, appended to a root step)
//! - field step: `ns:name` or `name` (after a `/`)
//! - qualifier step: `?ns:name` or `?name`
//!
//! Examples the SDK exercises:
//! - `Flash`                         (bare root)
//! - `dc:creator`                    (prefixed root)
//! - `dc:creator[2]`                 (array item)
//! - `Iptc4xmpCore:CreatorContactInfo/Iptc4xmpCore:CiAdrCity` (struct field)
//! - `dc:creator[1]?xmpidq:Role`     (array item qualifier)

use crate::error::XmpError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathStep {
    Root { ns: Option<String>, name: String },
    Index(usize),
    Field { ns: Option<String>, name: String },
    Qualifier { ns: Option<String>, name: String },
}

/// Split a step into optional `?` marker, optional `prefix:`, and the name.
fn parse_step_text(text: &str, is_first: bool) -> Result<PathStep, XmpError> {
    if text.is_empty() {
        return Err(XmpError::BadPath("empty path step".into()));
    }

    let (qualifier, rest) = if let Some(r) = text.strip_prefix('?') {
        (true, r)
    } else {
        (false, text)
    };

    let (ns, name) = match rest.split_once(':') {
        Some((p, n)) => {
            if p.is_empty() || n.is_empty() {
                return Err(XmpError::BadPath(format!("malformed step '{text}'")));
            }
            (Some(p.to_string()), n.to_string())
        }
        None => (None, rest.to_string()),
    };

    if qualifier {
        if !is_first {
            Ok(PathStep::Qualifier { ns, name })
        } else {
            Err(XmpError::BadPath("path cannot start with a qualifier".into()))
        }
    } else if is_first {
        Ok(PathStep::Root { ns, name })
    } else {
        Ok(PathStep::Field { ns, name })
    }
}

/// Parse a path expression into steps.
///
/// `ns:root[i]` array indices become separate `Index` steps; qualifier
/// suffixes `?ns:qual` (attached to a step, per toolkit syntax) become
/// `Qualifier` steps.
pub fn parse_path(prop: &str) -> Result<Vec<PathStep>, XmpError> {
    let prop = prop.trim();
    if prop.is_empty() {
        return Err(XmpError::BadPath("empty path".into()));
    }

    let mut steps = Vec::new();
    // Split on '/', but keep [i] and ?q attached to the preceding step.
    let segments: Vec<&str> = prop.split('/').collect();

    for (seg_i, seg) in segments.iter().enumerate() {
        let seg = seg.trim();
        if seg.is_empty() {
            return Err(XmpError::BadPath(format!("empty segment in '{prop}'")));
        }

        // Split off any qualifier suffix `?...` (may repeat).
        let mut head = seg;
        let mut qual_texts: Vec<&str> = Vec::new();
        while let Some(qpos) = head.rfind('?') {
            qual_texts.push(&head[qpos..]);
            head = &head[..qpos];
        }
        qual_texts.reverse();

        // Handle `[i]` index suffix(es) on the head: `name[1][2]` etc.
        let (head_name, mut rest) = match head.find('[') {
            Some(pos) => (&head[..pos], &head[pos..]),
            None => (head, ""),
        };

        if seg_i == 0 {
            steps.push(parse_step_text(head_name, true)?);
        } else if !head_name.is_empty() {
            steps.push(parse_step_text(head_name, false)?);
        } else if rest.is_empty() && qual_texts.is_empty() {
            return Err(XmpError::BadPath(format!("empty segment in '{prop}'")));
        }

        // Parse any number of [i] indices in the remainder.
        while !rest.is_empty() {
            if !rest.starts_with('[') {
                return Err(XmpError::BadPath(format!(
                    "unexpected text after index in '{prop}'"
                )));
            }
            let close = rest
                .find(']')
                .ok_or_else(|| XmpError::BadPath(format!("unclosed '[' in '{prop}'")))?;
            let inside = &rest[1..close];
            let idx: usize = inside
                .parse()
                .map_err(|_| XmpError::BadPath(format!("bad array index '[{inside}]'")))?;
            if idx == 0 {
                return Err(XmpError::BadPath("array indices are 1-based".into()));
            }
            steps.push(PathStep::Index(idx));
            rest = &rest[close + 1..];
        }

        // Qualifier steps.
        for q in qual_texts {
            steps.push(parse_step_text(q, false)?);
        }
    }

    if steps.is_empty() {
        return Err(XmpError::BadPath(format!("no steps in '{prop}'")));
    }

    Ok(steps)
}

/// Compose an array item path, mirroring `TXMPUtils::ComposeArrayItemPath`:
/// the 1-based index is appended as `[i]` to the array name (which may be a
/// composed path itself). The toolkit does not include the schema prefix in
/// the composed path — callers pass whatever prefix form the array name uses.
pub fn compose_array_item_path(
    schema_prefix: &str,
    array_name: &str,
    index: usize,
) -> String {
    let _ = schema_prefix;
    format!("{array_name}[{index}]")
}

/// Compose a struct field path, mirroring
/// `TXMPUtils::ComposeStructFieldPath`. The field is addressed with its own
/// namespace prefix: `ns:struct/ns2:field`.
pub fn compose_struct_field_path(
    struct_path: &str,
    field_prefix: &str,
    field_name: &str,
) -> String {
    format!("{struct_path}/{field_prefix}:{field_name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bare_root() {
        let s = parse_path("Flash").unwrap();
        assert_eq!(
            s,
            vec![PathStep::Root {
                ns: None,
                name: "Flash".into()
            }]
        );
    }

    #[test]
    fn parse_prefixed_root() {
        let s = parse_path("dc:format").unwrap();
        assert_eq!(
            s,
            vec![PathStep::Root {
                ns: Some("dc".into()),
                name: "format".into()
            }]
        );
    }

    #[test]
    fn parse_array_index() {
        let s = parse_path("dc:creator[2]").unwrap();
        assert_eq!(
            s,
            vec![
                PathStep::Root {
                    ns: Some("dc".into()),
                    name: "creator".into()
                },
                PathStep::Index(2)
            ]
        );
    }

    #[test]
    fn parse_struct_field() {
        let s = parse_path("Iptc4xmpCore:CreatorContactInfo/Iptc4xmpCore:CiAdrCity").unwrap();
        assert_eq!(
            s,
            vec![
                PathStep::Root {
                    ns: Some("Iptc4xmpCore".into()),
                    name: "CreatorContactInfo".into()
                },
                PathStep::Field {
                    ns: Some("Iptc4xmpCore".into()),
                    name: "CiAdrCity".into()
                }
            ]
        );
    }

    #[test]
    fn parse_nested_combination() {
        let s = parse_path("ns:a[1]/ns2:b[2]/ns3:c?ns4:q").unwrap();
        assert_eq!(s.len(), 6);
        assert!(matches!(s[0], PathStep::Root { .. }));
        assert!(matches!(s[1], PathStep::Index(1)));
        assert!(matches!(s[2], PathStep::Field { .. }));
        assert!(matches!(s[3], PathStep::Index(2)));
        assert!(matches!(s[4], PathStep::Field { .. }));
        assert!(matches!(s[5], PathStep::Qualifier { .. }));
    }

    #[test]
    fn parse_errors() {
        assert!(parse_path("").is_err());
        assert!(parse_path("a[]").is_err());
        assert!(parse_path("a[0]").is_err());
        assert!(parse_path("a[x]").is_err());
        assert!(parse_path("a[b").is_err());
        assert!(parse_path(":name").is_err());
        assert!(parse_path("ns:").is_err());
    }
}

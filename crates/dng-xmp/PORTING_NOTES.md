# dng-xmp PORTING NOTES

Pure-Rust replacement for the Adobe XMP Toolkit subset used by the DNG SDK
(`dng_xmp_sdk.cpp`, `dng_xmp.cpp`, `dng_exif.cpp`).

## Architecture

- **Data model** (`node.rs`): `XmpMeta` = `rdf:about` + per-schema property
  maps; `XmpNode` = simple / struct (ordered fields with per-field namespace)
  / Bag-Seq-Alt array + qualifiers (`xml:lang`, …). This is the Rust
  equivalent of the toolkit's `XMP_Node` tree.
- **Parser** (`parse.rs`, quick-xml): XML → namespace-resolved element tree →
  interpretation pass. Accepts full packet (`<?xpacket…?>`), `x:xmpmeta`
  wrapper, bare `rdf:RDF`, or bare `rdf:Description`. Handles element form,
  attribute form, `rdf:parseType="Resource"`, nested `rdf:Description`
  shorthand, `rdf:value`/`rdf:type`, CDATA, empty arrays.
- **Serializer** (`serialize.rs`): hand-written RDF/XML output for exact
  control over whitespace conventions. Normal form (one `rdf:Description` per
  schema) and compact form (attributes for qualifier-free simple props) per
  XMP spec Part 2; packet wrapper/padding per Part 3.
- **Namespace registry** (`ns.rs`): global `uri → prefix` map pre-seeded with
  the toolkit's `kXMP_NS_*` constants the SDK uses plus the DNG-SDK-specific
  registrations from `dng_xmp_sdk::InitializeSDK` (crx, crss, crd, crlcp, lr,
  stCamera, dng, panorama, GPano, mwg-rs, hdrgm, HDRGainMap, apdi,
  hdr_metadata).

## API surface notes

The SDK passes **bare local names** as paths (`Set(XMP_NS_DC, "format", …)`)
as well as toolkit-composed prefixed paths
(`ComposeStructFieldPath` → `ns:struct/ns2:field`). Both forms are accepted;
an undeclared root prefix resolves against the schema namespace argument.

Implemented toolkit behaviors:
- 1-based array indexing; `AppendArrayItem` creates Bag (`ordered=false`) or
  Seq (`ordered=true`), replaces a single dummy empty item ("emptied array"
  convention).
- `SetLocalizedText` keeps `x-default` first, replaces per-language items,
  normalizes language case; `GetLocalizedText` does exact + generic prefix
  matching and the "empty alt array / empty simple value" equivalence noted
  in `dng_xmp_sdk.cpp GetAltLangDefault`.
- `SetProperty` with a path creates implicit intermediates (arrays are
  created as Bag, matching no-op toolkit default form).
- Undeclared prefixes in packets fall back to the global registry (the
  toolkit's XML parser does this; real packets occasionally rely on it,
  e.g. `stEvt:` attributes without a declaration).
- `IteratePaths`/`HasNameSpace`/`RemoveProperties(ns)` equivalents exist as
  `iterate_paths` / `has_namespace` / `remove_properties`.

## Deviations / simplifications (PORT-NOTEs)

1. **No doc-mode / incremental reconciliation** — `qDNGXMPDocOps` is off in
   the SDK by default; DocOps APIs not ported.
2. **`PackageForJPEG` / `MergeFromJPEG` / `DuplicateSubtree`** (utility
   functions used by the SDK's JPEG-incremental metadata path) are not part
   of the ARCHITECTURE.md contract and are not implemented; the SDK glue
   layer (phase 2) will port their logic over this engine if needed.
3. **Prefix stability**: the serializer re-derives prefixes from the global
   registry (first-registered wins) instead of preserving the input packet's
   prefixes byte-for-byte. Tree data (URIs) is always preserved.
4. **Compact-form ordering**: in compact mode simple properties appear as
   attributes sorted per schema iteration order (BTreeMap), not in original
   packet order. Reparse produces the identical tree.
5. **rdf:ID / rdf:nodeID / rdf:datatype attributes** are parsed and ignored
   (annotations without effect in the SDK's usage). `rdf:type` qualifiers
   are preserved.
6. **Whitespace**: element text is trimmed at the edges (XMP values are
   trimmed by the toolkit as well); internal whitespace including newlines
   is preserved and re-escaped (`&#10;`/`&#13;`) on output.
7. **Padding convention**: lines of 100 spaces separated by the configured
   newline, matching the pattern observed in Adobe-produced DNG packets.
   The `x:xmptk` attribute is branded "RustDNG XMP Core 1.7.1".
8. **No dependency on dng-core or other crates**; only `quick-xml`.

## Purity

`cargo tree -p dng-xmp | grep -E "cc |cmake|-sys"` is empty. No `unsafe`.

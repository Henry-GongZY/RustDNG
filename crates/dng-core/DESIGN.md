# dng-core design notes (Agent A, phase 1)

Binding decisions that later crates (dng-sdk phase 2) code against.

## Ownership model — images

- `pub trait DngImage: Send + Sync` (crates/dng-core/src/dng_image.rs).
  All trait methods take `&self`; mutable image state (pixel buffers) lives
  behind interior mutability in the concrete types (`DngSimpleImage` wraps its
  state in `std::sync::RwLock<SimpleImageState>`).
- `pub type ImageRef = Arc<RwLock<dyn DngImage>>` — the replacement for C++
  `dng_image_ref` / `dng_image *`.  `DngNegative`-style containers in later
  crates store images as `Option<ImageRef>`.
- Methods that returned `dng_image_ref` in C++ (`Clone`) return `ImageRef`
  here (`DngImage::clone_image`).
- No `clone_image` convenience beyond the C++ virtual; free-function
  extensions in dng_image.rs (`image_get`, `image_get_edge`,
  `image_get_repeat`, `image_copy_area`, `image_equal_area`,
  `image_set_constant`) implement the C++ base-class default bodies and are
  generic over `I: DngImage + ?Sized` so the trait stays object-safe.

## Ownership model — pixel buffers

`DngPixelBuffer` OWNS its pixels (C++ holds a raw `void *`).  Storage is a
typed enum `PixelData` (`U8/I8/U16/I16/U32/I32/U64/I64/F16/F32/F64`) so
aligned typed views are safe without `unsafe`.  The C++ "move `fData`,
negate step" tricks (`FlipH/V/Z`, `Rotate`, `Trim`) are modelled with a
private base-element offset; `pixel_element_index_public` exposes it for
zero-copy fast paths inside the crate.  Out-of-range pixel access returns
`DngError` (C++ reads out of bounds / has a #if-0 check).

## Ownership model — streams

- `DngStream<'a>` drives a boxed `StreamIO` backend (the C++ `DoGetLength /
  DoRead / DoWrite / DoSetLength` virtuals).  All typical uses are
  `DngStream<'static>`; the C++ `dng_stream_double_buffered` subclass becomes
  the free function `dng_stream_double_buffered(&mut DngStream) -> DngStream`
  whose IO backend borrows the parent stream.
- RAII helpers (`TempBigEndian`, `TempStreamSniffer`,
  `PreserveStreamReadPosition`, `dng_stream_contiguous_read_hint`) are guard
  structs with `Drop`; `TempBigEndian::stream()` and delegate methods give
  scoped access that the C++ got implicitly through reference members.
- `DngMemoryStream` keeps the C++ page-list layout (`Vec<Vec<u8>>`) and
  `length_limit`; the page-direct `CopyToStream` fast path is omitted (one
  extra memcpy per block; noted in PORTING_NOTES.md).
- `DngMemoryBlock` is `Vec<u8>` with logical/physical sizes; typed accessors
  are native-endian element get/set (no unsafe reinterpret casts).

## Abort sniffing

C++ passes raw `dng_abort_sniffer *` everywhere.  Rust uses
`Option<Arc<dyn AbortSniffer>>` (sniffers are shared between host, streams
and tasks).  `Sniff ()` returns `DngResult<()>` with `Err(DngError::UserCancel)`
replacing `ThrowUserCanceled`.

## Area tasks

Serial execution only (ARCHITECTURE.md §9).  `DngAreaTask` keeps the C++
method names (`Start/Process/Finish/FindTileSize/ProcessOnThread`);
`process_on_single_thread` + free fn `perform_area_task` /
`process_on_single_thread` mirror `dng_area_task::Perform` on thread 0.
`DngFilterTaskBase` (in dng_area_task.rs, re-exported as `DngFilterTask` by
dng_filter_task.rs) implements the C++ filter-task driver: per-tile
`Get(edge_repeat)` → user `ProcessArea` → `Put`.

## Errors

`DngError` mirrors dng_errors.h 1:1 with the ARCHITECTURE.md contract names
(`ReadFailure` = dng_error_read_file etc.) plus the contract's compatibility
variants (`NotThorough/NotSupported/Syntax/AlreadyExists/MissingSubImages`)
and `Unsupported(String)` as the message-carrying catch-all.

## Threading

Single-threaded everywhere (`kMaxMPThreads` kept as a constant for API
parity; `DngHost::perform_area_task_threads` returns 1).  Atomics used only
for the dng_globals mirrors.

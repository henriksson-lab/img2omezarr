# Rust Bio-Formats to OME-Zarr Implementation Plan

## Scope

Build a Rust converter that reads microscopy images with `bioformats-rs` and
writes OME-Zarr with `zarrs`. The converter must have:

- a shared core library;
- a CLI mode gated behind a default-off feature;
- a Slint GUI mode gated behind a default-off feature;
- drag/drop or file-picker batch conversion in the GUI;
- adjustable conversion and upload settings in both CLI and GUI.

The priority output target is the latest released OME-Zarr/NGFF version:
**0.5**. OME-Zarr **0.4** compatibility should be supported if practical, but
0.5 should drive the data model and tests. OME-Zarr **0.6rc0** exists as a
release candidate/editor draft and should be tracked, not used as the default
until it is released.

## Existing Code To Reuse

Use `/home/mahogny/github/claude/clearmap-rs` as the Rust reference.

Useful files:

- `src/bin/tile_to_zarr.rs`: streaming slab conversion pattern from image tile
  to Zarr.
- `src/io/zarr.rs`: `zarrs` v3 filesystem writer helpers, chunking, sharding,
  and compression options.
- `src/io/tile_source.rs`: source abstraction pattern over Bio-Formats-backed
  TIFF and Zarr input.
- `clearmap-gui/src/run.rs`: Slint worker-thread/progress event pattern.
- `clearmap-gui/ui/app.slint`: Slint list/progress/settings UI patterns.
- `clearmap-gui/Cargo.toml`: separate GUI workspace-member pattern.

Do not copy `TileSource` wholesale. It is a ClearMap acquisition-tile contract
with fixed 3D `(X, Y, Z)` semantics. This converter needs a Bio-Formats
series/plane reader capable of 2D through 5D OME-Zarr image output.

## Repository Layout

Create a Rust crate at the root of this repository, beside the cloned Java
`bioformats2raw/` directory:

```text
Cargo.toml
src/
  lib.rs
  convert/
    mod.rs
    config.rs
    reader.rs
    planner.rs
    axes.rs
    dtype.rs
    downsample.rs
    metadata.rs
    writer.rs
    upload.rs
    progress.rs
  bin/
    img2omezarr.rs
gui/
  Cargo.toml
  build.rs
  src/main.rs
  src/run.rs
  ui/app.slint
tests/
  ...
```

Keep the GUI as a separate workspace member. Slint brings a build-time code
generator and a desktop graphics stack; the core library should not depend on
either.

## Cargo Features

All user-facing modes should be default-off:

```toml
[features]
default = []
core-bioformats = ["dep:bioformats-rs"]
cli = ["core-bioformats", "dep:clap", "dep:tracing-subscriber"]
gui = ["core-bioformats"]
upload-s3 = ["dep:object_store", "dep:tokio"]

[[bin]]
name = "img2omezarr"
path = "src/bin/img2omezarr.rs"
required-features = ["cli"]
```

The GUI crate can depend on the core crate with `features = ["core-bioformats"]`
instead of exposing Slint through the core crate.

## Core API

Expose one shared conversion API used by CLI and GUI:

```rust
pub fn convert_many(
    jobs: Vec<ConvertJob>,
    settings: ConversionSettings,
    progress: impl ProgressSink,
) -> anyhow::Result<Vec<ConversionReport>>;
```

Key types:

```rust
pub struct ConvertJob {
    pub input: PathBuf,
    pub output: OutputTarget,
}

pub enum OutputTarget {
    Local(PathBuf),
    Upload(UploadTarget),
}

pub struct ConversionSettings {
    pub ngff_version: NgffVersion,
    pub series: SeriesSelection,
    pub tile_width: usize,
    pub tile_height: usize,
    pub chunk_depth: usize,
    pub resolution_policy: ResolutionPolicy,
    pub downsampling: Downsampling,
    pub compression: CompressionSettings,
    pub overwrite: bool,
    pub write_omero_metadata: bool,
    pub write_ome_xml: bool,
    pub max_workers: usize,
}

pub enum NgffVersion {
    V05,
    V04,
}
```

Default `NgffVersion` must be `V05`.

## Conversion Semantics

Port behavior conceptually from Java `bioformats2raw`, not line-by-line.

First milestone semantics:

- Open input through `bioformats-rs`.
- Enumerate series.
- Use OME-Zarr 0.5 / Zarr v3 output by default.
- Store axes in NGFF-compatible order: time, channel, spatial axes.
- Write each resolution as a separate Zarr array.
- Stream chunks/slabs to keep peak memory bounded.
- Generate multiscale pyramids by 2x XY downsampling.
- Compute resolution count from a target minimum XY size unless explicitly set.
- Write `multiscales` metadata.
- Write axis metadata and coordinate transformations.
- Map Bio-Formats pixel types to Zarr data types.
- Write basic OMERO rendering metadata when enough metadata is available.

Second milestone semantics:

- Optional OME-Zarr 0.4 / Zarr v2 compatibility if `zarrs` support is adequate
  or if a practical compatibility writer can be implemented.
- Preserve source pyramids with a `use_existing_resolutions` option.
- HCS plate/well metadata.
- More complete channel naming/color heuristics.
- More downsampling algorithms.

Defer:

- Java `bioformats2raw` custom readers such as BioTek, ND2 plate grouping,
  Metaxpress, Phenix, MCD, Mirax, and PyramidTiff. Revisit only after checking
  whether `bioformats-rs` already covers the real target inputs.
- Raw2ometiff compatibility quirks unless explicitly needed.

## Writer Design

Adapt the `clearmap-rs/src/io/zarr.rs` design:

- create arrays up front with full shape;
- write chunk/subset regions incrementally;
- support zstd compression by default for OME-Zarr 0.5;
- expose chunk and optional shard settings;
- keep writer APIs independent from CLI/GUI types.

OME-Zarr 0.5 means the primary writer should use Zarr v3 metadata and
`zarr.json` group attributes with the OME metadata under the `ome` key.

## Metadata Design

Build metadata through typed Rust structs serialized with `serde`, not ad hoc
JSON string construction.

Required 0.5 metadata:

- root/image group `ome.version = "0.5"`;
- `multiscales`;
- datasets ordered from full resolution to lowest resolution;
- dataset `coordinateTransformations`;
- axes with unique names and valid types;
- array dimension names where required by the spec;
- optional `omero` rendering metadata.

Use conformance fixtures/tests from the OME-NGFF repository once the first
writer path exists.

## CLI Plan

Initial CLI:

```text
img2omezarr convert INPUT OUTPUT.ome.zarr
img2omezarr batch --output-dir OUT INPUT...
```

Flags:

```text
--ngff-version 0.5|0.4
--series all|0,2,3
--tile-width N
--tile-height N
--chunk-depth N
--resolutions N
--target-min-size N
--compression zstd|blosc|none
--compression-level N
--workers N
--overwrite
--no-omero
--no-ome-xml
--upload-target ...
```

The CLI should only parse settings, initialize logging/progress, and call the
shared core API.

## GUI Plan

Use Slint in a separate `gui` workspace member.

Core GUI behavior:

- drag/drop image files into a queue;
- file picker as a fallback path;
- remove/reorder queue items;
- show detected status per input;
- settings panel for conversion/upload settings;
- `Convert` for local output;
- `Upload` or `Convert and Upload` for remote target;
- worker-thread execution;
- progress and logs streamed back to the UI event loop;
- per-file success/failure state.

Use the worker pattern from `clearmap-gui/src/run.rs`: conversion runs off the
UI thread, progress events are marshalled back through Slint's event loop.

## Upload Plan

Start with a two-phase upload:

```text
input image -> local temporary .ome.zarr -> upload/copy to target
```

This is easier to validate than direct remote Zarr writes and avoids making
conversion correctness depend on object-store behavior.

Upload settings:

```rust
pub enum UploadTarget {
    Local(PathBuf),
    S3 {
        bucket: String,
        prefix: String,
        region: Option<String>,
        endpoint: Option<String>,
        profile: Option<String>,
    },
}
```

Later, add direct object-store-backed Zarr writing if `zarrs` store support and
concurrency semantics are clean enough.

## Testing Plan

Pure unit tests first:

- resolution count calculation;
- axis ordering;
- array shape/chunk mapping;
- Bio-Formats pixel type to Zarr dtype mapping;
- output path planning;
- OME-Zarr 0.5 metadata serialization;
- upload target validation.

Integration tests next:

- tiny single-series conversion;
- multiseries conversion;
- RGB/multichannel metadata;
- physical pixel sizes in coordinate transforms;
- basic OMERO rendering metadata;
- overwrite and failure cleanup behavior.

GUI tests:

- settings-to-`ConversionSettings` mapping;
- queue operations;
- worker progress events without launching a full desktop session.

## Milestones

1. Create Rust workspace and copy/adapt the `clearmap-rs` Zarr writer helpers.
   Status: implemented.
2. Implement local OME-Zarr 0.5 conversion for one input/one series.
   Status: implemented and smoke-tested with `test_8x8_gray8.tif`.
3. Add CLI `convert`.
   Status: implemented behind default-off `cli` feature.
4. Add multiseries and batch conversion.
   Status: implemented for selected/all series and batch output planning.
5. Add Slint GUI queue/settings/progress around the same core API.
   Status: implemented as separate `gui` workspace member with file picker,
   native file drop through Slint `DropArea` where the platform exposes a
   plain-text drop payload, queue removal/reordering, per-file status updates,
   conversion settings, local/S3 upload settings, staged upload toggle, worker
   thread, progress, and logs.
6. Add two-phase upload.
   Status: implemented for local staged copy and S3 object upload behind
   default-off `upload-s3`. S3 output is selected with `s3://bucket/prefix` when
   the feature is enabled; credentials come from `object_store`/AWS environment.
7. Add OME-Zarr 0.4 compatibility if still needed.
   Status: implemented for the practical local-filesystem subset: uncompressed
   and Blosc-compressed Zarr V2 arrays/groups with legacy NGFF 0.4 `.zattrs`
   metadata. Zstd-compressed 0.4 output is rejected with a clear error; 0.5
   remains the priority latest released OME-Zarr target.
8. Add HCS/plate metadata and source-pyramid reuse.
   Status: partially implemented. Native Bio-Formats source resolutions are
   reused when `--use-existing-resolutions` or the GUI source-pyramid toggle is
   selected and the reader exposes multiple resolutions. OME plate/well samples
   are mapped to NGFF 0.5 plate/well paths and metadata when present.

## Current Implementation Notes

Implemented files:

- `Cargo.toml`: root library plus default-off `cli`, `core-bioformats`, and
  `upload-s3` features.
- `src/convert/*`: shared conversion core, reader adapter, pyramid planner,
  axes/dtype/downsampling/metadata/writer/upload/progress modules.
- `src/bin/img2omezarr.rs`: `convert` and `batch` CLI.
- `gui/*`: Slint desktop GUI workspace member.
- GUI unit tests in `gui/src/main.rs`: coverage for dropped `file://` URI-list
  and plain-path parsing.
- `tests/hcs_fake.rs`: integration coverage for Bio-Formats SPW/HCS metadata
  flowing into NGFF 0.5 plate/well hierarchy metadata.
- `tests/ngff_v04.rs`: integration coverage for uncompressed OME-Zarr 0.4 /
  Zarr V2 metadata, Blosc-compressed 0.4 output, zstd rejection, and readable
  pixel data.
- `tests/parallel_batch.rs`: integration coverage for multi-worker batch
  conversion and stable report ordering.
- `tests/physical_metadata.rs`: integration coverage for physical pixel sizes
  in NGFF coordinate transforms and downsampling method metadata.
- `tests/source_pyramid.rs`: integration coverage proving native Bio-Formats
  source pyramid levels are written rather than regenerated when requested.
- `tests/blosc.rs`: integration coverage for NGFF 0.5 / Zarr V3 Blosc codec
  metadata and readable pixel data.

The OME-Zarr 0.5 writer currently:

- writes Zarr v3 groups/arrays through `zarrs`;
- writes `ome.version = "0.5"` root and series metadata;
- writes TCZYX arrays with dimension names;
- writes generated 2x XY pyramid levels with the selected downsampling method;
- supports nearest-neighbor and averaging downsampling for generated pyramid
  levels;
- writes basic OMERO channel rendering metadata;
- writes `OME/METADATA.ome.xml` when `bioformats-rs` provides OME metadata;
- respects source endian when converting multi-byte sample buffers.
- can reuse existing source pyramid levels instead of generated downsampling
  when requested;
- can emit NGFF 0.5 HCS plate/well metadata when OME plate/well samples are
  available.
- can write uncompressed or Blosc-compressed NGFF 0.4 / Zarr V2 output when
  requested.
- runs independent batch conversion jobs through a bounded worker pool while
  preserving report order.

Verified commands:

```text
cargo check
cargo check --features cli
cargo check --features 'cli upload-s3'
cargo test --features 'cli upload-s3'
cargo check -p img2omezarr-gui
cargo check -p img2omezarr-gui --features upload-s3
cargo test -p img2omezarr-gui --features upload-s3
cargo fmt --check
cargo run --features cli --bin img2omezarr -- convert /home/mahogny/github/claude/bioformats-rs/tests/fixtures/test_8x8_gray8.tif /tmp/b2oz-smoke.ome.zarr --overwrite --tile-width 4 --tile-height 4 --target-min-size 4 --compression none
cargo run --features cli --bin img2omezarr -- convert /home/mahogny/github/claude/bioformats-rs/tests/fixtures/test_8x8_gray8.tif /tmp/b2oz-smoke.ome.zarr --overwrite --use-existing-resolutions --tile-width 4 --tile-height 4 --target-min-size 4 --compression none
```

Known limitations:

- Native GUI drag/drop is wired through Slint `DropArea`, but exact external
  file-drop payload support depends on the Slint backend/platform. The file
  picker remains the fallback.
- S3 upload is compile-tested only here, not credential/integration-tested.
- Existing source-pyramid reuse has integration coverage with a generated
  pyramidal OME-TIFF fixture.
- HCS plate/well metadata has unit coverage plus an end-to-end FakeReader SPW
  conversion smoke test. A real vendor HCS fixture is still useful before
  treating plate conversion as production-hardened.
- 0.4 output supports uncompressed and Blosc-compressed local Zarr V2. Zstd is
  intentionally rejected for 0.4 because it is not the legacy-compatible
  default expected by most OME-Zarr 0.4 readers.
- `max_workers` is used for independent batch jobs. Chunks within a single
  image are still written serially because Bio-Formats reader access is mutable
  and needs a more careful reader-pool design.

## Design Principle

The converter core owns conversion policy: pyramid planning, chunk scheduling,
axis mapping, metadata generation, progress, and upload orchestration.

`bioformats-rs` owns image reading.

`zarrs` owns Zarr storage.

CLI and GUI own only presentation and settings collection.

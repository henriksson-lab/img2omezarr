# img2omezarr Product Completion Plan

This plan covers the remaining work needed before treating `img2omezarr` as a
complete product. Each item includes objective done criteria so progress can be
verified without relying on vague status labels.

## 1. Performance And Scalability

### 1.1 Add Benchmark Fixtures

Done when:

- [x] A `benches/` benchmark suite exists for at least one small, one medium, and
  one synthetic large multi-resolution conversion workload.
- [x] Benchmarks report wall time, peak resident memory when available, input
  dimensions, output size, compression mode, and worker count.
- [x] Benchmark commands are documented in `README.md` or `docs/benchmarks.md`.

### 1.2 Define Performance Targets

Done when:

- [x] A documented baseline exists for local SSD conversion of at least one
  representative OME-TIFF or generated pyramidal fixture.
- [ ] A documented baseline exists for S3 upload to MinIO or another S3-compatible
  target.
- [x] The repository records target thresholds for acceptable memory use and
  throughput, even if initial thresholds are conservative.

### 1.3 Parallelize Single-Image Work Safely

Done when:

- Single-image conversion can write chunks or planes concurrently without
  sharing one mutable Bio-Formats reader unsafely.
- The implementation has tests proving output equality between serial and
  parallel conversion for at least one Z stack, one multichannel image, and one
  source-pyramid image.
- `--workers N` affects single-image conversion as well as independent batch
  jobs, or the CLI/GUI clearly expose separate controls for batch workers and
  chunk workers.

### 1.4 Add Large-Image Memory Tests

Done when:

- [x] A test or benchmark verifies that conversion peak memory stays bounded for a
  synthetic large image and does not scale with the full uncompressed output
  volume.
- [x] The test fails if the implementation accidentally buffers all planes or all
  pyramid levels in memory.

### 1.5 Optimize Upload Path

Done when:

- S3 upload supports retry with bounded backoff for transient failures.
- Upload reports per-file or per-object progress in the GUI log/progress path.
- A benchmark compares current two-phase temp-dir upload with any optimized
  upload path that is added.

## 2. Real Dataset Validation

### 2.1 Build A Real-World Fixture Matrix

Done when:

- [x] A documented fixture matrix lists at least these formats: OME-TIFF, ND2, CZI,
  LIF, SVS or another whole-slide pyramid format, and one HCS/plate dataset.
- [x] Each fixture row records source, license/usage permission, dimensions,
  series count, channel count, Z/T dimensions, pixel type, physical pixel sizes,
  pyramid levels, and expected output characteristics.
- [x] Fixtures that cannot be committed are covered by a download/preparation
  script or clear manual instructions.

### 2.2 Add Real Dataset Smoke Tests

Done when:

- At least one smoke test converts every fixture in the matrix.
- Smoke tests verify that expected series, axes, shapes, pixel types, pyramid
  levels, and core metadata are present in the output.
- Tests can be run locally with one documented command.

### 2.3 Validate Edge Cases

Done when:

- Tests cover RGB data, multi-channel non-RGB data, Z stacks, time series,
  multi-series files, physical pixel sizes, and files with missing or partial
  metadata.
- Each covered edge case has an assertion on both metadata and readable pixel
  data.

### 2.4 Produce A Validation Report

Done when:

- A checked-in report summarizes every real fixture tested, pass/fail status,
  known limitations, and the command used to generate the result.
- The README links to the report.

## 3. OME-Zarr Conformance

### 3.1 Add Conformance Tooling

Done when:

- The repository has a documented command that runs official OME-NGFF
  validation or an accepted conformance checker against generated outputs.
- The command works on Linux without manual setup beyond documented fixture
  preparation.

### 3.2 Add Conformance CI

Done when:

- CI validates at least one NGFF 0.5 output and one NGFF 0.4 output.
- CI fails when generated OME-Zarr metadata violates the selected conformance
  rules.
- Any intentionally unsupported warnings are documented with issue links or
  explicit rationale.

### 3.3 Track NGFF Version Decisions

Done when:

- [x] README or docs clearly state which NGFF versions are supported, which is the
  default, and what compression options are valid for each.
- [x] There is a documented process for revisiting NGFF 0.6 once it is released.

## 4. S3 Integration Testing

### 4.1 Add MinIO-Based Test Harness

Done when:

- [x] A script or CI job starts MinIO, creates a bucket, configures credentials,
  and runs upload tests without external cloud dependencies.
- [x] The harness works from a clean checkout without a committed `Cargo.lock`.

### 4.2 Test CLI S3 Upload

Done when:

- [x] Tests verify CLI upload to `s3://bucket/prefix`.
- [x] Tests verify custom endpoint, region, access key, and secret key behavior.
- [x] Tests verify overwrite behavior and failure messages for invalid bucket or
  credentials.

### 4.3 Test GUI Saved S3 Profiles

Done when:

- [x] Tests verify profile metadata is persisted to the platform config path.
- [x] Tests verify access key and secret key are stored in the separate GUI
  credentials file or a controlled test credential backend.
- [x] Tests verify conversion uses saved profile credentials rather than requiring
  environment variables.
- [x] Tests verify delete removes both profile metadata and saved credential
  entries.

### 4.4 Test Upload Output Integrity

Done when:

- [x] Uploaded S3 object keys exactly match expected OME-Zarr layout paths.
- [x] A downloaded or object-store-read output can be opened and pixel data matches
  the local conversion for the same input.

## 5. GUI Product Polish

### 5.1 Replace Log-Only Errors With User-Facing Dialogs

Done when:

- [x] Missing output folder, invalid S3 profile, failed conversion, failed upload,
  and failed credential-file operations show visible GUI errors.
- [x] The same errors are still written to the log panel.

### 5.2 Add Cancellation

Done when:

- [x] The GUI has a cancel button while work is running.
- [x] Cancellation stops queued work before starting the next file.
- [x] Cancellation does not leave a corrupt final local output directory.
- [x] Cancellation does not upload a partially completed S3 dataset as if it
  succeeded.

### 5.3 Persist General GUI Settings

Done when:

- [x] NGFF version, tile size, chunk depth, target minimum size, downsampling,
  compression, overwrite, OMERO metadata, staged upload, upload target, and
  last output directory are restored on restart.
- [x] The persisted settings file contains no access secret or secret key material.

### 5.4 Improve S3 Profile UX

Done when:

- [x] The GUI shows whether a selected profile has saved credentials.
- [x] Saving a profile with empty credential fields does not accidentally delete
  existing credentials unless the user explicitly chooses to clear them.
- [x] `Test` shows a clear pending/success/failure state.

### 5.5 Add Output Preview

Done when:

- [x] The GUI shows the exact local or S3 output target that will be produced for
  each queued input.
- [x] Batch naming collisions are detected before conversion starts.

### 5.6 Add Real GUI Screenshots

Done when:

- README screenshots are generated from a real running GUI or a Slint-supported
  offscreen/test renderer, not a manually drawn preview.
- A documented command regenerates screenshots.
- Screenshot generation is reproducible on Linux CI or documented as a release
  task.

## 6. Packaging And Releases

### 6.1 Package CLI Binaries

Done when:

- GitHub Actions produces downloadable CLI artifacts for Linux, macOS, and
  Windows.
- Each artifact includes the `img2omezarr` binary and license/readme files.
- A smoke test runs `img2omezarr --help` for each built artifact.

### 6.2 Package GUI Applications

Done when:

- GitHub Actions produces a Linux archive or AppImage, a macOS `.app` or DMG,
  and a Windows zip or installer containing the GUI.
- Each GUI package starts successfully on its target OS in a smoke test where
  automation permits it.
- Package names and app titles use `img2omezarr`.

### 6.3 Define Release Process

Done when:

- A release workflow builds from a tag and uploads artifacts to GitHub Releases.
- Version numbers are sourced consistently from Cargo metadata.
- The release checklist includes license review, fixture validation,
  conformance validation, S3 validation, and changelog update.

### 6.4 Document Installation

Done when:

- README documents how to install or run the CLI and GUI on Linux, macOS, and
  Windows.
- README documents saved GUI S3 profile credential storage.
- README documents common S3-compatible endpoint examples such as MinIO.

# img2omezarr Execution Plan: Items 1-5 Plus 7

This plan covers implementation-plan milestones 1-5 plus milestone 7, with
milestone 7 moved earlier so OME-Zarr 0.4 behavior is defined before validation
and conformance work depends on it.

## Completion Order

1. Confirm workspace, crate names, and build surfaces.
2. Confirm OME-Zarr 0.4 compatibility behavior.
3. Finish performance and scalability readiness.
4. Finish real dataset validation.
5. Finish OME-Zarr conformance coverage.
6. Finish S3 integration testing.
7. Finish GUI product polish.

## 1. Workspace And Build Surface

Done when:

- [ ] `cargo metadata` reports a workspace containing the core crate
  `img2omezarr` and GUI crate `img2omezarr-gui`.
- [ ] The root package name, CLI binary name, README examples, and GUI window
  title use `img2omezarr`.
- [ ] `Cargo.lock` is ignored and not staged for commit.
- [ ] CI builds the CLI on Linux, macOS, and Windows with
  `cargo build -p img2omezarr --features "cli upload-s3" --bin img2omezarr`.
- [ ] CI builds the GUI on Linux, macOS, and Windows with
  `cargo build -p img2omezarr-gui`.
- [ ] CI builds the GUI with S3 support on Linux, macOS, and Windows with
  `cargo build -p img2omezarr-gui --features upload-s3`.

## 2. OME-Zarr 0.4 Compatibility

Done when:

- [ ] The supported OME-Zarr versions are explicit in CLI help, GUI controls,
  README, and `docs/ngff-support.md`.
- [ ] OME-Zarr 0.4 output writes Zarr v2-compatible group and array metadata for
  the supported compression modes.
- [ ] Unsupported 0.4 feature combinations fail before conversion starts with a
  specific error message.
- [ ] Tests verify 0.4 metadata, array readability, compression behavior, and
  rejection of unsupported options.
- [ ] CI runs at least one OME-Zarr 0.4 conversion test and one OME-Zarr 0.5
  conversion test.

## 3. Performance And Scalability

Done when:

- [ ] Benchmarks cover a small, medium, and synthetic large conversion workload.
- [ ] Benchmark output records wall time, peak memory when available, input
  dimensions, output size, compression mode, worker count, and output target.
- [ ] `docs/benchmarks.md` contains a local SSD baseline from a reproducible
  command.
- [ ] `docs/benchmarks.md` contains an S3 or MinIO upload baseline from a
  reproducible command.
- [ ] A memory-bound test proves peak memory does not scale with full
  uncompressed output volume.
- [ ] Single-image worker behavior is either implemented and equality-tested for
  Z stack, multichannel, and source-pyramid inputs, or the CLI and GUI clearly
  label workers as batch-level workers only.
- [ ] S3 upload retries transient failures with bounded backoff and reports
  upload progress in the CLI or GUI progress path.

## 4. Real Dataset Validation

Done when:

- [ ] `docs/validation.md` lists fixtures for OME-TIFF, ND2, CZI, LIF, whole
  slide or pyramidal image data, and HCS or plate data.
- [ ] Every fixture row records source, license or usage permission, dimensions,
  series count, channel count, Z/T dimensions, pixel type, physical pixel sizes,
  pyramid levels, and expected output characteristics.
- [ ] Non-committed fixtures have a script or exact manual preparation command.
- [ ] A documented smoke-test command converts every available fixture.
- [ ] Smoke tests assert expected series, axes, shapes, pixel types, pyramid
  levels, core metadata, and readable pixel data.
- [ ] Edge-case tests cover RGB, non-RGB multichannel, Z stacks, time series,
  multi-series files, physical pixel sizes, and missing or partial metadata.
- [ ] `docs/validation.md` contains a generated or manually updated validation
  report with pass/fail status and known limitations.

## 5. OME-Zarr Conformance

Done when:

- [ ] The repository documents one command that runs accepted OME-NGFF
  validation tooling against generated outputs.
- [ ] The conformance command works on Linux after documented fixture
  preparation.
- [ ] CI validates at least one OME-Zarr 0.5 output.
- [ ] CI validates at least one OME-Zarr 0.4 output if 0.4 support remains in
  the product.
- [ ] CI fails when generated metadata violates selected conformance rules.
- [ ] Any accepted conformance warning is documented with a rationale or issue
  link.

## 6. S3 Integration Testing

Done when:

- [ ] CI starts MinIO, creates a bucket, configures credentials, and runs S3
  upload tests without external cloud dependencies.
- [ ] CLI tests verify upload to `s3://bucket/prefix`.
- [ ] CLI tests verify custom endpoint, region, access key, secret key,
  overwrite behavior, invalid bucket errors, and invalid credential errors.
- [ ] Uploaded object keys exactly match expected OME-Zarr layout paths.
- [ ] A downloaded or object-store-read S3 output opens successfully and pixel
  data matches a local conversion for the same input.
- [ ] GUI profile tests verify config-file persistence, keyring storage, use of
  saved credentials during conversion, and profile deletion cleanup.

## 7. GUI Product Polish

Done when:

- [ ] Missing output folder, invalid S3 profile, failed conversion, failed upload,
  and failed keyring operations show visible GUI errors and are also logged.
- [ ] The GUI has a cancel button while work is running.
- [ ] Cancellation stops queued work before starting the next file.
- [ ] Cancellation does not leave a corrupt final local output directory.
- [ ] Cancellation does not upload a partial S3 dataset as a successful result.
- [ ] General GUI settings persist across restarts without storing secret values.
- [ ] S3 profile UX shows saved-credential state, preserves existing credentials
  unless explicitly cleared, and shows pending/success/failure test states.
- [ ] The queue shows the exact local or S3 output target for each input.
- [ ] Batch naming collisions are detected before conversion starts.
- [ ] README GUI screenshots are generated from a real running GUI or supported
  offscreen renderer, and the regeneration command is documented.

## Verification Command Set

The plan is complete only when these commands pass from a clean checkout without
a committed `Cargo.lock`:

```text
cargo fmt --check
cargo test -p img2omezarr --features "cli upload-s3"
cargo test -p img2omezarr-gui --features upload-s3
cargo build -p img2omezarr --features "cli upload-s3" --bin img2omezarr
cargo build -p img2omezarr-gui
cargo build -p img2omezarr-gui --features upload-s3
```

CI must run the build commands on Linux, macOS, and Windows.

# Benchmarks

The benchmark suite is a stable Rust custom bench target. It exercises the real
conversion API with synthetic Bio-Formats `.fake` inputs so it can run without
downloading large microscopy files.

Run:

```sh
cargo bench -p img2omezarr --features core-bioformats --bench conversion_bench
```

The output is CSV with these fields:

```text
workload,output_target,size_x,size_y,size_z,size_c,target_min_size,tile_width,tile_height,compression,workers,elapsed_ms,output_bytes,peak_rss_kib
```

`peak_rss_kib` is reported on Linux through `/proc/self/status` and is
`unavailable` on platforms where that value is not exposed by this benchmark.

## Workloads

- `small`: 256 x 256 x 1 z x 1 channel, uncompressed, one worker.
- `medium`: 1024 x 1024 x 4 z x 2 channels, zstd compression, two workers.
- `synthetic-large`: 4096 x 4096 x 2 z x 3 channels, zstd compression, four
  workers.

## Initial Performance Targets

These targets are intentionally conservative until real-world fixture baselines
are checked in.

- Peak memory for `synthetic-large` should stay below 1 GiB on Linux.
- The `memory_bound` integration test enforces a default 768 MiB Linux peak RSS
  ceiling for a synthetic 2048 x 2048 x 4 z x 2 channel conversion. Override
  with `IMG2OMEZARR_MEMORY_TEST_LIMIT_KIB` when validating a different
  environment.
- Peak memory should not grow in proportion to full uncompressed pyramid output
  size.
- `--workers` should improve total throughput for independent batch jobs today.
- A later reader-pool implementation should make `--workers` improve
  single-image throughput without changing output bytes or metadata.
- S3 upload to MinIO should complete without retry exhaustion for the benchmark
  output produced by `small` and `medium`.

Run the memory guard with:

```sh
cargo test -p img2omezarr --features core-bioformats --test memory_bound
```

## S3 Baseline Recording

Build the benchmark with `upload-s3` and provide an S3-compatible endpoint. The
benchmark always emits local rows. When `IMG2OMEZARR_S3_BENCH_BUCKET` is set, it
also emits S3 rows for the `small` and `medium` workloads.

```sh
IMG2OMEZARR_S3_BENCH_BUCKET=img2omezarr-bench \
IMG2OMEZARR_S3_BENCH_PREFIX=bench-run-001 \
IMG2OMEZARR_S3_BENCH_ENDPOINT=http://127.0.0.1:9000 \
AWS_ACCESS_KEY_ID=minioadmin \
AWS_SECRET_ACCESS_KEY=minioadmin \
AWS_REGION=us-east-1 \
cargo bench -p img2omezarr --features "core-bioformats upload-s3" --bench conversion_bench
```

## Baseline Recording

When recording a baseline, include:

- git commit;
- host OS and CPU;
- storage type for local output;
- S3-compatible endpoint if upload is measured;
- exact benchmark command;
- full CSV output.

## Recorded Baselines

### Local Generated Fixture Baseline

- Date: 2026-08-28
- Git commit: `b0eeb4a` plus uncommitted product-completion changes
- Host OS: Linux `beagle` 6.8.0-58-generic x86_64
- CPU: Intel Xeon Gold 6138 @ 2.00 GHz, 40 logical CPUs visible to the process
- Local output storage: benchmark-created temporary directories under `/tmp`;
  `/tmp` was on `/dev/sda3` and nearly full during this run, so use these values
  as an initial regression baseline rather than a clean hardware capacity
  benchmark.
- Command:

```sh
CARGO_TARGET_DIR=/tmp/img2omezarr-check-target cargo bench -p img2omezarr --features core-bioformats --bench conversion_bench
```

CSV output:

```text
workload,output_target,size_x,size_y,size_z,size_c,target_min_size,tile_width,tile_height,compression,workers,elapsed_ms,output_bytes,peak_rss_kib
small,local,256,256,1,1,64,128,128,none,1,13,91962,16640
medium,local,1024,1024,4,2,128,256,256,zstd,2,458,50023,18240
synthetic-large,local,4096,4096,2,3,512,512,512,zstd,4,6330,139101,37508
```

### S3/MinIO Baseline

Not recorded yet. Record this with a live MinIO or S3-compatible endpoint using
the same CSV fields plus endpoint type, network locality, bucket/prefix, and
whether the upload path used staged temporary output.

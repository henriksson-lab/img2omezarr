# img2omezarr

`img2omezarr` converts microscopy image files to OME-Zarr. It has both a command line user interface and a GUI aimed to make it easy
for regular users to upload their data to OME-Zarr.

**Testing needed; work in progress**

## Build

```sh
cargo build -p img2omezarr --features cli --bin img2omezarr
```

## Benchmarks

Performance benchmarks are documented in [docs/benchmarks.md](docs/benchmarks.md).

## Validation And Compatibility

NGFF version support is documented in [docs/ngff-support.md](docs/ngff-support.md).
The real-world fixture matrix is tracked in [docs/validation.md](docs/validation.md).

## CLI Usage

```sh
img2omezarr convert INPUT OUTPUT.ome.zarr
img2omezarr batch --output-dir OUT INPUT...
```

Run `img2omezarr --help` for the full CLI options.

## GUI S3 Profiles

The GUI can save S3 profiles between runs. Profile metadata is written to the
platform config directory. Access keys are stored separately in
`credentials.toml` in the same app config directory, not in the main settings
file.

Build the GUI with S3 support:

```sh
cargo run -p img2omezarr-gui --features upload-s3
```

Select `s3` as the upload target, enter a profile name, bucket, prefix, region,
endpoint, and credentials, then press `Save`.

General GUI settings are persisted between runs. S3 profile metadata is stored
with those settings, but access keys are written only to the separate
credentials file. On Unix, the GUI writes that file with owner read/write
permissions.

## Screenshots

![img2omezarr GUI](docs/screenshots/gui-main.svg)

## License

This code is under MIT. But note that some libraries it depends on are GPL.

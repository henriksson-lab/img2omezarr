# NGFF Support Policy

`img2omezarr` currently targets OME-Zarr / OME-NGFF 0.5 by default.

## Supported Outputs

| NGFF version | Zarr version | Status | Compression support |
| --- | --- | --- | --- |
| 0.5 | Zarr v3 | Default supported output | `zstd`, `blosc`, `none` |
| 0.4 | Zarr v2 | Compatibility output | `blosc`, `none` |

NGFF 0.4 with `zstd` is rejected intentionally because it is not the
legacy-compatible default expected by most OME-Zarr 0.4 readers.

## Default Behavior

- CLI and GUI default to NGFF 0.5.
- Generated pyramid metadata records the downsampling method.
- Source pyramid levels can be reused when requested and exposed by the reader.

## Revisiting Future NGFF Versions

Before changing the default NGFF version:

1. Confirm the version is a final release, not only a release candidate or
   editor draft.
2. Add conformance tests for the new version.
3. Add at least one reader interoperability check with a common OME-Zarr
   consumer.
4. Document any metadata or compression changes in this file and the README.
5. Keep the previous default version available for one release cycle unless it
   is technically impossible.

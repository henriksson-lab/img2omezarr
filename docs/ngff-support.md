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
- Tile positions are written as a level-0 `translation`, with provenance in
  the `img2omezarr` group attribute, in both versions; see
  [tile-positions.md](tile-positions.md).

## Initial Rendering Windows

When OMERO rendering metadata is enabled, every channel gets a standard
`omero.channels[].window`:

- `min` and `max` are the exact finite bounds observed while level zero is
  written.
- `start` and `end` preserve an explicit source display window when
  Bio-Formats exposes one. ImageJ TIFF display limits are currently supported.
- Without a source display window, `start` and `end` use Fiji/ImageJ's
  256-bin auto-contrast rule. At most 1,048,576 evenly spaced values per
  channel are retained for this calculation, so memory use is bounded.
- If a channel contains no finite values, its pixel type and declared
  significant bits provide the fallback range.

Statistics are collected in the normal level-zero write pass. Conversion does
not reread the source to produce rendering metadata. These values are display
defaults only and do not alter stored pixels.

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

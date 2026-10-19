# Tile Positions

A tiled acquisition (a light-sheet mosaic, a stage scan) is converted to one
OME-Zarr image per tile. Stitching needs to know where each tile lies. This
page is the contract for recording that in standard OME-Zarr metadata, so a
stitcher (such as blockflow's `tile-stitching`) needs no instrument-specific
metadata and no hand-given overlap, and how `img2omezarr` writes it.

## Contract

- A tile is one OME-Zarr image: a multiscale group.
- Its level-0 dataset's `coordinateTransformations` hold a `scale` and then a
  `translation`, both over every axis of the image, in the axes' own units
  (`micrometer` for a calibrated `img2omezarr` conversion). Non-spatial axes
  (`t`, `c`) get 0.
- The translation is the physical position of the tile's first voxel in a
  frame that all tiles of the sample share, and the sample's other images
  (e.g. an autofluorescence channel) too, when that is known. The stage-to-image
  axis mapping, including signs, is already applied: the translation is in
  image axes `z`, `y`, `x`.
- Coarser levels keep their own relation to level 0. A level with no offset of
  its own gets the same translation as level 0; a pyramid whose levels carry
  half-voxel offsets keeps them, moved by the same amount.
- No multiscale-level `coordinateTransformations` are used.
- Only group metadata carries this: `zarr.json` (NGFF 0.5) or `.zattrs`
  (NGFF 0.4). Chunk data and array metadata do not change.
- The group attributes record where the position came from, beside `ome`
  (NGFF 0.5) or beside `multiscales` (NGFF 0.4):

```json
"img2omezarr": {
  "positions": {
    "source": "ome-plane",
    "detail": "Plane TheZ=0 TheC=0 TheT=0: stage x 103, y -20, z 3 (unit not reported by the reader; micrometer assumed), micrometer",
    "stage_axes": "x,y,z",
    "grid": [0, 1]
  }
}
```

| key | meaning |
| --- | --- |
| `source` | `table`, `grid`, `lavision`, `ome-plane`, or `none` (no translation was written). |
| `detail` | Free text: which plane, table line, or grid parameters gave the position. |
| `stage_axes` | The stage-to-image mapping used (`lavision`, `ome-plane`, `table`). |
| `grid_axes` | The image axes rows and columns run along (`grid`). |
| `grid` | `[row, column]`, from the grid pattern, or for other sources from the default pattern when the name has one. |

Any producer can meet the contract: `img2omezarr`, bioformats2raw plus a
script, or a vendor exporter. Consumers should read only the translation; the
provenance is for people.

A stage position's reference point (the tile's corner or its centre) is the
instrument's. For tiles of one size it is the same offset on every tile, so the
relative placement, which is what stitching uses, is right either way.

## Sources

`convert` and `batch` take the position of each series from the first of:

1. `--positions FILE`, a table, when given;
2. `--overlap` or `--pitch`, a grid, when given;
3. a format's own metadata, where the standard fields are empty: LaVision
   UltraMicroscope OME-TIFFs (`lavision`, below);
4. the standard OME metadata the reader returns: `Plane.PositionX/Y/Z` and
   `StageLabel`;
5. none: no translation, and `"source": "none"` in the attributes.

`--no-positions` goes straight to 5. `set-positions` takes 1 or 2, or, with
`--from-originals`, 3 from the files the images were converted from.

### Standard OME metadata (`ome-plane`)

The plane used is the image's first, `TheZ=0 TheC=0 TheT=0` (else the first
listed); its z is the first voxel's. An axis the plane lacks is taken from the
image's `StageLabel`; an axis neither gives is 0, and the detail says so.
bioformats-rs does not report `PositionXUnit`; Bio-Formats readers store stage
positions in micrometres, so micrometres are assumed. An image whose axes are
uncalibrated (no physical size) gets no translation.

The default mapping is the identity: stage x, y, z are image x, y, z.

### LaVision (`lavision`)

LaVision's UltraMicroscope (Imspector) writes OME-TIFFs with no `<Plane>` and
no `<StageLabel>`, so Bio-Formats, and bioformats-rs with it, reports no
position. The stage positions are in LaVision's own elements of the OME-XML
(`OME/Image/CustomAttributes`, namespace `Schemas/CA/2008-02`), which
`src/convert/positions/lavision.rs` reads:

```xml
<Image Name="13-01-56_FOS_UltraII[00 x 01]_C00.ome.tif" ...>
  <Offset Offset_0="1395.453979" Offset_1="1584.480957" Offset_2="0.0" Offset_3="0.0"/>
  <IsPartOfMosaic IsPartOfMosaic="1"/>
  <TileConfiguration TileConfiguration="2
13-01-56_FOS_UltraII[00 x 00]_C00.ome.tif;;(-72.895996, 1584.480957)
13-01-56_FOS_UltraII[00 x 01]_C00.ome.tif;;(1395.453979, 1584.480957)
..."/>
```

- A file's stage x and y are **its own line** of a `TileConfiguration`: the
  one in its own XML, or in any other file of the batch (the configuration is
  the whole mosaic's, repeated in each file that has XML). Only when no
  configuration lists the file is its `<Offset>` used, and only when the XML's
  `<Image Name>` is the file's own. So a tile whose XML was copied from
  another file, or that has none (on osf-sa3x8, 3 of the 18 tile files carry
  XML), still gets its own position, never the other file's.
- z is `Offset_2` of the file's own XML, else 0. A third value in a
  configuration line numbers the channel (`Filter0000` 0, `Filter0001` 1), so
  it is not read as z.
- A file that is not part of a mosaic (`IsPartOfMosaic="0"`, e.g. a single
  autofluorescence stack) has an all-zero `<Offset>`, which is no position:
  it gets none from this source.
- Micrometres. The default mapping is `x,-y,-z`: stage x is image x, stage y
  runs against image y (mosaic rows step down in stage y and are stacked
  along +y), and stage z decreases plane by plane.
- A batch in which some tiles get a LaVision position and others do not is
  refused (unless `--force`): they would not share a frame.

**The mapping, checked against stitched mosaics.** osf-cztq2 (35 tiles of
rows 00-06; row 07 is excluded, see below) against `tile-stitching`'s rigid
positions of the earlier `--overlap 120,120` run
(`/husky/henriksson/lightsheet-reference/pipeline/osf-cztq2/stitch/align.positions.json`),
after removing the common offset:

| mapping (image x, y from stage) | fitted pitch scale y, x | residual max y, x (scale 1) | residual max (rms) y, x, scale fitted |
|---|---|---|---|
| `x,-y` (default) | 0.980, 0.979 | 93, 72 µm | 6.2 (2.2), 11.9 (6.5) µm |
| `x,y` | -0.980, 0.979 | 8753, 72 µm | same, with the sign of y reversed |
| `-x,-y`, `-x,y` | ..., -0.979 | ..., 5821 µm | |
| `y,x` and the other swaps | about 0.001 | > 7000 µm | > 2800 µm |

Only `x,-y` matches with positive scales; the vendor pitch (1468.35 µm in x,
1472.74 µm in y) is 2.0-2.1% longer than the stitched one (1436.5, 1441.4
µm), so scale-1 residuals grow to 72-93 µm across the 5 x 7 tiles, within the
stitch's search. osf-sa3x8's nine tiles, stitched from their LaVision
positions alone (no overlap given), agree the same way: y pitch scale 1.001,
x 0.973; residuals 2.2 (y) and 6.7 (x) µm max with the scale fitted, 4.3 and
20 µm at scale 1.

**osf-cztq2's row 07.** Its eight files' metadata repeat row 00 shifted by
one column (`[07 x 01]` has `[00 x 00]`'s position, ...; `[07 x 00]` has (35,
0)), in every file's `TileConfiguration` and in their own `<Offset>`. The
layout check refuses it: four duplicate positions, and five row steps and one
column step that run the wrong way.

`set-positions --from-originals DIR` matches each image to its original in
`DIR` by name without extension, else by grid index (the default `<row> x
<column>` pattern on both names, so `tile_03x04.zarr` matches
`..._UltraII[03 x 04]_C00_UltraII Filter0000.ome.tif`). Several files with
one index (a tile's channels, a renamed copy) are fine when all that give a
position give the same; otherwise the match is refused, and
`--originals-match REGEX` narrows the files. Only the first TIFF directory of
each file in `DIR` is read (opening a whole UltraMicroscope OME-TIFF walks an
IFD per plane through gigabytes, 20-60 s a file on a network disk; this takes
0.2 s for osf-cztq2's 40).

### Table (`table`)

```text
# comments and blank lines are skipped
image,series,z,y,x,unit
tile_00x00.zarr,,0,0,0,micrometer
tile_00x01.zarr,,0,0,1436.5,micrometer
scan.czi,3,0,1441.375,0,um
tile_01x01,,0,887,884,voxel
```

- Comma-separated; the header names the columns, in any order. `image`, `z`,
  `y`, `x` and `unit` are required; `series` is optional (empty is series 0).
  No quoting: names cannot contain commas.
- `image` matches the image's path as given on the command line, its file
  name, or its file name without the extension (`.ome.zarr`, `.zarr`,
  `.ome.tif`, or the last extension). For `convert`, the image is the input
  file. Every image needs exactly one row.
- `unit` is `voxel` (the image's level-0 voxels) or a length: `micrometer`
  (`um`, `µm`, `micron`), `nanometer` (`nm`), `millimeter` (`mm`),
  `centimeter`, `meter`, `angstrom`, `picometer`. Lengths are converted to the
  image's axis units; an axis without a unit takes only `voxel`.
- `z`, `y`, `x` are image axes. `--stage-axes` reinterprets them as stage
  axes (the `x` column as stage x, and so on) and maps them; voxels are
  converted after the mapping, with the image axis's voxel size.

### Grid (`grid`)

```sh
img2omezarr set-positions tiles/tile_*.zarr --overlap 120,120 --grid-unit voxel
img2omezarr batch --output-dir out --pitch 1441.375,1436.5 --grid-unit um raw/*.tif
```

- `--grid-pattern REGEX` reads the row and column from each image's file name:
  a regex with named groups `row` and `col` (or `column`); the last match in
  the name counts. The default is `(?P<row>\d+) *[xX] *(?P<col>\d+)`, which
  reads `tile_03x04.zarr` and `scan[03 x 04]_C00.ome.tif` as row 3, column 4.
- `--overlap ROW,COLUMN` (neighbouring tiles overlap by this much) or
  `--pitch ROW,COLUMN` (the distance between neighbours' first voxels); one
  value is used for both. The pitch from an overlap is the tile's extent minus
  the overlap.
- `--grid-unit` is required with either: `voxel` (level-0 voxels) or a length,
  as in the table.
- `--grid-axes` (default `y,x`) gives the image axis that rows run along, then
  the one columns run along, each optionally negated: tile `(row, column)` is at
  `row * pitch_row` along the first and `column * pitch_column` along the
  second (`-y,x` puts row 1 at negative y). This matches `tile-stitching`'s
  reading of names (row along +y, column along +x).
- Indices are used as they are, not rebased to 0, so a sub-grid keeps its
  place.
- All tiles must have the same size and voxel size along the grid axes (one
  pitch has to fit them all). `--stage-axes` does not apply to a grid.

## Sanity checks

All tiles of one call (every series of every input of a `batch`, or every
image given to `set-positions`) are placed and checked together, before
anything is written:

- two tiles at the same position (within half a voxel on every axis), e.g.
  a row whose metadata repeats another row's (osf-cztq2's row 07 repeats row
  00);
- neighbours that do not overlap (a gap, overlap < 0), or whose step runs the
  other way from the other neighbours' along that grid direction (an overlap
  beyond the tile, or a misplaced tile);
- for a grid, an overlap or pitch that leaves a pitch outside `0..tile`.

Neighbours are grid neighbours when every tile has a row and column (from the
grid, or the default pattern on its name); otherwise, for each tile and axis,
the nearest tile further along that axis that lies within half a tile on the
other axes. Failed checks are printed as warnings, and the call is refused
unless `--force`; nothing is written when it is refused.

## `img2omezarr set-positions`

```sh
img2omezarr set-positions [--positions FILE | --overlap R,C | --pitch R,C |
                           --from-originals DIR [--originals-match REGEX]]
    [--grid-unit UNIT] [--grid-pattern REGEX] [--grid-axes y,x]
    [--stage-axes x,y,z] [--force] [--dry-run] IMAGE...
```

Writes positions into existing OME-Zarr images, from any producer: fixing an
earlier conversion without converting again, or third-party OME-Zarr.

- Each `IMAGE` is a multiscale image, or a bioformats2raw collection (each
  series is a tile, matched in a table by its `series`).
- NGFF 0.5 (`zarr.json`, `attributes.ome`) and 0.4 (`.zattrs`) are both
  rewritten in place; nothing else is opened for writing. An existing level-0
  translation is replaced, and every level's translation moves by the same
  amount. The provenance replaces `img2omezarr.positions`, keeping any other
  `img2omezarr` keys.
- Every image's new metadata is built before the first is written, and each
  file is replaced by a rename, so a failure leaves all of them as they were.
  A metadata file that is a symlink is replaced by a file; its target is left
  alone. So positions can be tried on a mirror of shared data: copy the group
  and array metadata files and symlink the chunk directories.
- Prints each image's translation; `--dry-run` only prints.

## Validation

`tests/lavision_positions.rs` and the `lavision` unit tests: LaVision
OME-TIFFs through `convert` (a tile with another tile's XML placed by its own
configuration line; a configuration that repeats a position refused before
anything is written) and through `set-positions --from-originals` (tiles
whose originals carry no XML found in a sibling's configuration, two
channel files of one tile agreeing, disagreeing ones refused unless
`--originals-match` narrows them, chunks and arrays untouched); the
first-IFD reader on classic and big-endian BigTIFF.

osf-sa3x8 (9 tiles) on a metadata-only mirror,
`/husky/henriksson/lightsheet-reference/positions/osf-sa3x8/tiles`:
`set-positions --from-originals .../downloads/osf-sa3x8` places all nine from
the one configuration that lists them (tile x pitch 590.85 µm, y 1907.1 µm),
and `tile-stitching --tile-dir` stitches them from these translations with no
`--overlap`. osf-cztq2 on `.../positions/osf-cztq2-lavision` is refused (row
07); `--dry-run --force` prints the positions the table above was measured
with.

`tests/tile_positions.rs`: synthetic tiles in NGFF 0.5 and 0.4, a grid and a
table through `set-positions` (chunks and array metadata byte-identical
before and after, half-voxel level offsets kept, refusal leaves every file as
it was, collections), and OME-XML tiles with `<Plane PositionX=…>` through
`batch` (positions, `--stage-axes`, duplicate and gap refusal before any
output, grid and `--no-positions` overrides).

osf-cztq2 (40 tiles, 8 x 5, 1030 x 1007 x 1004 voxels of 6 x 1.625 x 1.625
um), on a metadata-only mirror: `set-positions --overlap 120,120 --grid-unit
voxel` gives pitches of 1441.375 um (y, 887 voxels) and 1436.5 um (x, 884
voxels). `tile-stitching --tile-dir --level 1` then reports
`positions_from: "translation"` without `--overlap`. Against the earlier
`--overlap 120,120` run, x is identical; y differs by half a level-1 voxel per
row (up to 3 at row 7). That run scaled the overlap to level 1 against the
level-1 tile height, ceil(1007 / 2) = 504, so its level-1 pitch was 444
(888 level-0 voxels), not 443.5. With that pitch written as the translation
(`--overlap 119,120`), every nominal, rigid and per-slice position and every
alignment is identical to the earlier run.

# Real-World Validation Matrix

This matrix tracks the real datasets needed before `img2omezarr` should be
treated as production-complete. Fixtures are intentionally not committed to the
repository. Use `scripts/prepare_real_world_fixtures.sh` to create the local
fixture directory and download selected files after reviewing source license
terms.

| Format | Source | Usage permission | Dimensions | Series | C | Z | T | Pixel type | Physical sizes | Pyramid levels | Expected output | Status |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OME-TIFF | `https://downloads.openmicroscopy.org/images/OME-TIFF/2016-06/bioformats-artificial/single-channel.ome.tif` | OME public sample image set; use requires preserving the source attribution and reviewing the OME sample-image terms before redistribution | 439 x 167 | 1 | 1 | 1 | 1 | Int8 8-bit | X=none, Y=none, Z=none | 1 | NGFF 0.5 image with axes, scales, readable pixels | Measured with `img2omezarr inspect` on 2026-08-28 |
| ND2 | `https://downloads.openmicroscopy.org/images/ND2/aryeh/MeOh_high_fluo_003.nd2` | Source README credits Aryeh Weiss and states Creative Commons Attribution 4.0 | 800 x 600 | 1 | 1 | 1 | 13 | Uint16 12-bit | X=0.37, Y=0.37, Z=none | 1 | NGFF 0.5 image preserving core dimensions and metadata available from `bioformats-rs` | Measured with `img2omezarr inspect` on 2026-08-28 |
| CZI | `https://downloads.openmicroscopy.org/images/Zeiss-CZI/idr0011/Plate1-Blue-A_TS-Stinger/Plate1-Blue-A-02-Scene-1-P2-E1-01.czi` | Source README credits Ledesma-Fernandez et al. and states Creative Commons Attribution 4.0 | 672 x 512 | 1 | 3 | 21 | 1 | Uint16 12-bit | X=0.20476190476190476, Y=0.20476190476190476, Z=0.35 | 1 | NGFF 0.5 image preserving core dimensions and metadata available from `bioformats-rs` | Measured with `img2omezarr inspect` on 2026-08-28 |
| LIF | `https://downloads.openmicroscopy.org/images/Leica-LIF/michael/PR2729_frameOrderCombinedScanTypes.lif` | Source README credits Michael Goelzer and states Creative Commons Attribution 4.0 | 4 series, each 64 x 64 | 4 | 2 | 3 | 2 | Uint8 8-bit | X=3.968253968253968, Y=3.968253968253968, Z=6.227614999999999 | 1 per series | NGFF 0.5 image preserving core dimensions and metadata available from `bioformats-rs` | Measured with `img2omezarr inspect` on 2026-08-28 |
| SVS or whole-slide pyramid | `https://downloads.openmicroscopy.org/images/SVS/77917.svs` | Source README credits Paul Felts and states Creative Commons Attribution 4.0 | Series 0: 96999 x 45667; series 1: 428 x 424; series 2: 1280 x 431 | 3 | 3 | 1 | 1 | Uint8 8-bit | Series 0: X=0.253, Y=0.253, Z=none; associated images: none | Series 0 has 5; associated images have 1 | NGFF 0.5 multiscale image using source pyramid levels when requested | Measured with `img2omezarr inspect` on 2026-08-28 |
| HCS/plate dataset | `https://downloads.openmicroscopy.org/images/OME-TIFF/2016-06/plate-companion/` fileset with `hcs.companion.ome` and well TIFFs | Source README credits the OME Consortium and states Creative Commons Attribution 4.0 | Measured `well-A2.ome.tiff`: 5 series, each 96 x 96; source plate contains wells A2, B1, B3, and C2 with 5 fields total | 5 image series in measured well file | 1 | 1 | 1 | Int8 8-bit | X=none, Y=none, Z=none | 1 per series | NGFF 0.5 plate/well hierarchy with readable image arrays | Measured with `img2omezarr inspect` on 2026-08-28 |

## Fixture Preparation

Real fixtures live outside git under `fixtures/real-world/`. The preparation
script creates that directory and downloads any URLs supplied through
environment variables:

```sh
OME_TIFF_FIXTURE_URL=https://downloads.openmicroscopy.org/images/OME-TIFF/... \
ND2_FIXTURE_URL=https://downloads.openmicroscopy.org/images/ND2/... \
CZI_FIXTURE_URL=https://downloads.openmicroscopy.org/images/Zeiss-CZI/... \
LIF_FIXTURE_URL=https://downloads.openmicroscopy.org/images/Leica-LIF/... \
SVS_FIXTURE_URL=https://downloads.openmicroscopy.org/images/SVS/77917.svs \
HCS_FIXTURE_URL=https://downloads.openmicroscopy.org/images/HCS/... \
scripts/prepare_real_world_fixtures.sh
```

After downloading, verify or refresh matrix fields with `img2omezarr inspect`:

```sh
cargo run -p img2omezarr --features cli --bin img2omezarr -- \
  inspect fixtures/real-world/<fixture-file> --format markdown
```

To inspect every downloaded fixture in one pass:

```sh
scripts/inspect_real_world_fixtures.sh
```

After updating this matrix with measured values and reviewed license terms,
verify that every required row is complete:

```sh
scripts/check_real_world_fixture_matrix.sh
```

If Bio-Formats command-line tools are installed, `showinf -nopix -omexml` is
also useful for cross-checking the raw OME metadata, but it is not required for
the matrix.

Do not commit downloaded fixtures unless their license explicitly allows
redistribution in this repository and the file size is acceptable.

## Required Smoke Test Assertions

Each real fixture smoke test should verify:

- output opens as Zarr;
- NGFF version metadata matches requested output version;
- axes are present and ordered as expected;
- array shapes match the source dimensions;
- chunk shapes match configured tile/chunk settings;
- pixel type matches the source pixel type;
- at least one pixel subset can be read;
- physical pixel sizes are present when source metadata provides them;
- pyramid level count and scale transforms match expectations;
- HCS fixtures produce plate/well metadata and expected image paths.

## Local Validation Command

The final validation command should be a single documented command, for example:

```sh
cargo test -p img2omezarr --features "cli upload-s3" --test real_world
```

The `real_world` test target is not implemented yet. This document is the
fixture contract that target should satisfy.

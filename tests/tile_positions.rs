#![cfg(feature = "core-bioformats")]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, NgffVersion,
    OutputTarget, ResolutionPolicy,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::positions::zarr::set_positions;
use img2omezarr::convert::positions::{
    GridSpec, GridStep, LengthUnit, PositionSettings, StageAxes,
};
use img2omezarr::convert::progress::NoProgress;
use serde_json::{json, Value};

fn settings(version: NgffVersion) -> ConversionSettings {
    ConversionSettings {
        ngff_version: version,
        tile_width: 4,
        tile_height: 4,
        resolution_policy: ResolutionPolicy::ExplicitLevels(2),
        compression: CompressionSettings {
            codec: CompressionCodec::None,
            level: None,
        },
        overwrite: true,
        ..ConversionSettings::default()
    }
}

/// Tiny tiles `tile_RRxCC.ome.zarr`, 2 x 8 x 8 voxels of 2 x 0.5 x 0.5 um.
fn fake_tiles(dir: &Path, version: NgffVersion, names: &[&str], extra: &str) -> Vec<PathBuf> {
    let jobs = names
        .iter()
        .map(|name| {
            let input = dir.join(format!(
                "{name}&sizeX=8&sizeY=8&sizeZ=2&physicalSizeX=0.5&physicalSizeY=0.5&physicalSizeZ=2{extra}.fake"
            ));
            fs::write(&input, b"").expect("write fake marker");
            ConvertJob {
                input,
                output: OutputTarget::Local(dir.join(format!("{name}.ome.zarr"))),
            }
        })
        .collect::<Vec<_>>();
    let outputs = jobs
        .iter()
        .map(|job| match &job.output {
            OutputTarget::Local(path) => path.clone(),
            _ => unreachable!(),
        })
        .collect();
    convert_many(jobs, settings(version), NoProgress).expect("convert fake tiles");
    outputs
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("parse json")
}

/// Every file under `root` but the image groups' metadata, by contents.
fn snapshot(root: &Path, skip: &[&str]) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if !skip.contains(&path.strip_prefix(root).unwrap().to_str().unwrap()) {
                files.insert(path.clone(), fs::read(&path).expect("read file"));
            }
        }
    }
    files
}

/// `(attributes, multiscale)` of an image group, NGFF 0.5 or 0.4.
fn image_attrs(group: &Path) -> (Value, Value) {
    if group.join("zarr.json").exists() {
        let attrs = read_json(&group.join("zarr.json"))["attributes"].clone();
        let multiscale = attrs["ome"]["multiscales"][0].clone();
        (attrs, multiscale)
    } else {
        let attrs = read_json(&group.join(".zattrs"));
        let multiscale = attrs["multiscales"][0].clone();
        (attrs, multiscale)
    }
}

fn translation(multiscale: &Value, level: usize) -> Option<Value> {
    multiscale["datasets"][level]["coordinateTransformations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["type"] == "translation")
        .map(|row| row["translation"].clone())
}

fn grid(overlap: f64) -> PositionSettings {
    PositionSettings {
        grid: Some(
            GridSpec::new(
                None,
                GridStep::Overlap([overlap, overlap]),
                LengthUnit::Voxel,
                None,
            )
            .unwrap(),
        ),
        ..Default::default()
    }
}

fn warn(_: &str) {}

fn set_positions_from_a_grid(version: NgffVersion, metadata: &str) {
    let temp = tempfile::tempdir().expect("tempdir");
    let names = ["tile_00x00", "tile_00x01", "tile_01x00", "tile_01x01"];
    let tiles = fake_tiles(temp.path(), version, &names, "");

    // the writer converted without positions to place
    let (attrs, multiscale) = image_attrs(&tiles[3]);
    assert_eq!(attrs["img2omezarr"]["positions"]["source"], "none");
    assert!(translation(&multiscale, 0).is_none());

    // a pyramid with half-voxel offsets between levels, as other writers make
    let metadata_path = tiles[3].join(metadata);
    let mut document = read_json(&metadata_path);
    let pointer = if version == NgffVersion::V05 {
        "/attributes/ome/multiscales/0/datasets/1/coordinateTransformations"
    } else {
        "/multiscales/0/datasets/1/coordinateTransformations"
    };
    document
        .pointer_mut(pointer)
        .unwrap()
        .as_array_mut()
        .unwrap()
        .push(json!({
            "type": "translation", "translation": [0.0, 0.0, 0.0, 0.25, 0.25]
        }));
    fs::write(
        &metadata_path,
        serde_json::to_vec_pretty(&document).unwrap(),
    )
    .unwrap();

    let before = snapshot(temp.path(), &[]);
    let skip: Vec<String> = names
        .iter()
        .map(|name| format!("{name}.ome.zarr/{metadata}"))
        .collect();
    let skip: Vec<&str> = skip.iter().map(String::as_str).collect();
    let chunks_before = snapshot(temp.path(), &skip);

    // a dry run writes nothing
    let placed = set_positions(&tiles, &grid(2.0), true, &warn).expect("dry run");
    assert_eq!(placed.len(), 4);
    assert_eq!(snapshot(temp.path(), &[]), before);

    set_positions(&tiles, &grid(2.0), false, &warn).expect("set positions");
    // chunks and array metadata are byte-identical, and no file was added
    assert_eq!(snapshot(temp.path(), &skip), chunks_before);
    assert_eq!(snapshot(temp.path(), &[]).len(), before.len());

    // pitch (8 - 2) x 0.5 um = 3 um; the coarser level keeps its offset
    let (attrs, multiscale) = image_attrs(&tiles[3]);
    assert_eq!(
        translation(&multiscale, 0),
        Some(json!([0.0, 0.0, 0.0, 3.0, 3.0]))
    );
    assert_eq!(
        translation(&multiscale, 1),
        Some(json!([0.0, 0.0, 0.0, 3.25, 3.25]))
    );
    let order: Vec<_> = multiscale["datasets"][0]["coordinateTransformations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["type"].clone())
        .collect();
    assert_eq!(order, [json!("scale"), json!("translation")]);
    let provenance = &attrs["img2omezarr"]["positions"];
    assert_eq!(provenance["source"], "grid");
    assert_eq!(provenance["grid"], json!([1, 1]));
    assert_eq!(provenance["grid_axes"], "y,x");
    let (_, multiscale) = image_attrs(&tiles[1]);
    assert_eq!(
        translation(&multiscale, 0),
        Some(json!([0.0, 0.0, 0.0, 0.0, 3.0]))
    );
    assert_eq!(
        translation(&multiscale, 1),
        Some(json!([0.0, 0.0, 0.0, 0.0, 3.0]))
    );

    // replacing: a second run sets the new position, offsets still kept
    set_positions(&tiles, &grid(4.0), false, &warn).expect("set positions again");
    let (_, multiscale) = image_attrs(&tiles[3]);
    assert_eq!(
        translation(&multiscale, 0),
        Some(json!([0.0, 0.0, 0.0, 2.0, 2.0]))
    );
    assert_eq!(
        translation(&multiscale, 1),
        Some(json!([0.0, 0.0, 0.0, 2.25, 2.25]))
    );
    assert_eq!(snapshot(temp.path(), &skip), chunks_before);

    // an overlap beyond the tile is refused, and nothing is written
    let written = snapshot(temp.path(), &[]);
    let error = set_positions(&tiles, &grid(9.0), false, &warn).expect_err("refused");
    assert!(error.to_string().contains("--force"), "{error}");
    assert_eq!(snapshot(temp.path(), &[]), written);
    let mut forced = grid(9.0);
    forced.force = true;
    set_positions(&tiles, &forced, false, &warn).expect("forced");
}

#[test]
fn set_positions_grid_ngff_05() {
    set_positions_from_a_grid(NgffVersion::V05, "zarr.json");
}

#[test]
fn set_positions_grid_ngff_04() {
    set_positions_from_a_grid(NgffVersion::V04, ".zattrs");
}

#[test]
fn set_positions_table_places_collections_and_refuses_duplicates() {
    for version in [NgffVersion::V05, NgffVersion::V04] {
        let temp = tempfile::tempdir().expect("tempdir");
        // a two-series collection (`0`, `1`) and a single image
        let collection = fake_tiles(temp.path(), version, &["scan"], "&series=2").remove(0);
        let single = fake_tiles(temp.path(), version, &["tile_00x00"], "").remove(0);
        let table = temp.path().join("positions.csv");
        fs::write(
            &table,
            "image,series,z,y,x,unit\n\
             scan.ome.zarr,0,0,0,0,micrometer\n\
             scan,1,0,0,0.003,mm\n\
             tile_00x00,,1,0,-4,voxel\n",
        )
        .unwrap();
        let settings = PositionSettings {
            table: Some(table.clone()),
            ..Default::default()
        };
        let before = snapshot(temp.path(), &[]);
        set_positions(
            &[collection.clone(), single.clone()],
            &settings,
            false,
            &warn,
        )
        .expect("set positions from a table");
        assert_eq!(snapshot(temp.path(), &[]).len(), before.len());

        let (attrs, multiscale) = image_attrs(&collection.join("1"));
        assert_eq!(
            translation(&multiscale, 0),
            Some(json!([0.0, 0.0, 0.0, 0.0, 3.0]))
        );
        assert_eq!(attrs["img2omezarr"]["positions"]["source"], "table");
        assert!(attrs["img2omezarr"]["positions"]["detail"]
            .as_str()
            .unwrap()
            .contains("line 3"));
        // voxels of the image: z 1 x 2 um, x -4 x 0.5 um
        let (_, multiscale) = image_attrs(&single);
        assert_eq!(
            translation(&multiscale, 0),
            Some(json!([0.0, 0.0, 2.0, 0.0, -2.0]))
        );

        // the same position twice is refused
        fs::write(
            &table,
            "image,series,z,y,x,unit\nscan,0,0,0,0,um\nscan,1,0,0,0,um\ntile_00x00,,0,0,9,um\n",
        )
        .unwrap();
        let written = snapshot(temp.path(), &[]);
        let error = set_positions(
            &[collection.clone(), single.clone()],
            &settings,
            false,
            &warn,
        )
        .expect_err("duplicates refused");
        assert!(error.to_string().contains("same position"), "{error}");
        assert_eq!(snapshot(temp.path(), &[]), written);

        // a table without a row for an image is an error
        fs::write(&table, "image,z,y,x,unit\ntile_00x00,0,0,0,um\n").unwrap();
        assert!(set_positions(&[collection], &settings, false, &warn).is_err());
    }
}

/// An 8 x 8 uint8 OME-XML image with its first plane's stage position.
fn ome_xml_tile(path: &Path, position: [f64; 3]) {
    const CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let pixels: Vec<u8> = (0..64).collect();
    let mut b64 = String::new();
    for chunk in pixels.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                b64.push(CHARS[((n >> shift) & 63) as usize] as char);
            } else {
                b64.push('=');
            }
        }
    }
    let [x, y, z] = position;
    fs::write(
        path,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<OME xmlns="http://www.openmicroscopy.org/Schemas/OME/2016-06">
  <Image ID="Image:0" Name="tile">
    <Pixels ID="Pixels:0" DimensionOrder="XYZCT" Type="uint8" SizeX="8" SizeY="8" SizeZ="1" SizeC="1" SizeT="1"
            PhysicalSizeX="0.5" PhysicalSizeY="0.5" PhysicalSizeZ="2" BigEndian="false">
      <Channel ID="Channel:0:0" SamplesPerPixel="1"/>
      <BinData Compression="none" BigEndian="false">{b64}</BinData>
      <Plane TheZ="0" TheC="0" TheT="0" PositionX="{x}" PositionXUnit="µm" PositionY="{y}" PositionYUnit="µm" PositionZ="{z}" PositionZUnit="µm"/>
    </Pixels>
  </Image>
</OME>"#
        ),
    )
    .expect("write OME-XML");
}

fn convert_ome_tiles(
    dir: &Path,
    version: NgffVersion,
    positions: &[[f64; 3]],
    placement: PositionSettings,
) -> anyhow::Result<Vec<PathBuf>> {
    let mut jobs = Vec::new();
    let mut outputs = Vec::new();
    for (column, position) in positions.iter().enumerate() {
        let input = dir.join(format!("tile_00x{column:02}.ome.xml"));
        ome_xml_tile(&input, *position);
        let output = dir.join(format!("tile_00x{column:02}.ome.zarr"));
        outputs.push(output.clone());
        jobs.push(ConvertJob {
            input,
            output: OutputTarget::Local(output),
        });
    }
    let mut settings = settings(version);
    settings.positions = placement;
    convert_many(jobs, settings, NoProgress)?;
    Ok(outputs)
}

#[test]
fn convert_writes_ome_plane_positions() {
    for version in [NgffVersion::V05, NgffVersion::V04] {
        let temp = tempfile::tempdir().expect("tempdir");
        let tiles = convert_ome_tiles(
            temp.path(),
            version,
            &[[100.0, -20.0, 3.0], [103.0, -20.0, 3.0]],
            PositionSettings::default(),
        )
        .expect("convert");
        let (attrs, multiscale) = image_attrs(&tiles[1]);
        assert_eq!(
            translation(&multiscale, 0),
            Some(json!([0.0, 0.0, 3.0, -20.0, 103.0]))
        );
        assert_eq!(
            translation(&multiscale, 1),
            Some(json!([0.0, 0.0, 3.0, -20.0, 103.0]))
        );
        let provenance = &attrs["img2omezarr"]["positions"];
        assert_eq!(provenance["source"], "ome-plane");
        assert_eq!(provenance["stage_axes"], "x,y,z");
        assert!(
            provenance["detail"]
                .as_str()
                .unwrap()
                .contains("Plane TheZ=0"),
            "{provenance}"
        );

        // a flipped stage axis
        let flipped = PositionSettings {
            stage_axes: Some(StageAxes::parse("-x,y,z").unwrap()),
            ..Default::default()
        };
        let tiles = convert_ome_tiles(
            temp.path(),
            version,
            &[[100.0, -20.0, 3.0], [103.0, -20.0, 3.0]],
            flipped,
        )
        .expect("convert flipped");
        let (attrs, multiscale) = image_attrs(&tiles[1]);
        assert_eq!(
            translation(&multiscale, 0),
            Some(json!([0.0, 0.0, 3.0, -20.0, -103.0]))
        );
        assert_eq!(attrs["img2omezarr"]["positions"]["stage_axes"], "-x,y,z");

        // the batch is checked as a whole: two tiles at one position, or a gap
        for bad in [[100.0, -20.0, 3.0], [110.0, -20.0, 3.0]] {
            let dir = temp.path().join(format!("bad{}", bad[0]));
            fs::create_dir(&dir).unwrap();
            let error = convert_ome_tiles(
                &dir,
                version,
                &[[100.0, -20.0, 3.0], bad],
                PositionSettings::default(),
            )
            .expect_err("refused");
            assert!(error.to_string().contains("--force"), "{error}");
            assert!(
                !dir.join("tile_00x00.ome.zarr").exists(),
                "nothing written before the check"
            );
            let forced = PositionSettings {
                force: true,
                ..Default::default()
            };
            convert_ome_tiles(&dir, version, &[[100.0, -20.0, 3.0], bad], forced).expect("forced");
        }

        // a grid replaces the plane positions; --no-positions writes none
        let tiles = convert_ome_tiles(temp.path(), version, &[[0.0; 3], [0.0; 3]], grid(2.0))
            .expect("grid");
        let (attrs, multiscale) = image_attrs(&tiles[1]);
        assert_eq!(
            translation(&multiscale, 0),
            Some(json!([0.0, 0.0, 0.0, 0.0, 3.0]))
        );
        assert_eq!(attrs["img2omezarr"]["positions"]["source"], "grid");
        let disabled = PositionSettings {
            disabled: true,
            ..Default::default()
        };
        let tiles = convert_ome_tiles(temp.path(), version, &[[0.0; 3], [0.0; 3]], disabled)
            .expect("disabled");
        let (attrs, multiscale) = image_attrs(&tiles[0]);
        assert!(translation(&multiscale, 0).is_none());
        assert_eq!(attrs["img2omezarr"]["positions"]["source"], "none");
    }
}

#[cfg(feature = "cli")]
#[test]
fn cli_set_positions_and_convert_flags() {
    use std::process::Command;

    let temp = tempfile::tempdir().expect("tempdir");
    let tiles = fake_tiles(
        temp.path(),
        NgffVersion::V05,
        &["s[00 x 00]", "s[00 x 01]"],
        "",
    );
    let output = Command::new(env!("CARGO_BIN_EXE_img2omezarr"))
        .arg("set-positions")
        .args(&tiles)
        .args(["--overlap", "1,2", "--grid-unit", "um"])
        .output()
        .expect("run set-positions");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("s[00 x 01].ome.zarr: z 0 y 0 x 2 micrometer (grid)"),
        "{stdout}"
    );

    // --overlap needs an explicit unit
    let status = Command::new(env!("CARGO_BIN_EXE_img2omezarr"))
        .arg("set-positions")
        .args(&tiles)
        .args(["--overlap", "1,2"])
        .output()
        .expect("run set-positions");
    assert!(!status.status.success());

    let input = temp.path().join("tile_00x00.ome.xml");
    ome_xml_tile(&input, [5.0, 6.0, 7.0]);
    let out = temp.path().join("flipped.ome.zarr");
    let status = Command::new(env!("CARGO_BIN_EXE_img2omezarr"))
        .args(["convert", "--stage-axes", "x,-y,z", "--compression", "none"])
        .arg(&input)
        .arg(&out)
        .status()
        .expect("run convert");
    assert!(status.success());
    let (_, multiscale) = image_attrs(&out);
    assert_eq!(
        translation(&multiscale, 0),
        Some(json!([0.0, 0.0, 7.0, -6.0, 5.0]))
    );
}

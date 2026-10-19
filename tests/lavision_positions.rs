//! The LaVision source (docs/tile-positions.md): positions from the vendor
//! XML of LaVision UltraMicroscope OME-TIFFs, through `convert` and through
//! `set-positions --from-originals`.

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
use img2omezarr::convert::positions::{Originals, PositionSettings};
use img2omezarr::convert::progress::NoProgress;
use serde_json::{json, Value};

fn settings() -> ConversionSettings {
    ConversionSettings {
        ngff_version: NgffVersion::V05,
        tile_width: 4,
        tile_height: 4,
        resolution_policy: ResolutionPolicy::ExplicitLevels(1),
        compression: CompressionSettings {
            codec: CompressionCodec::None,
            level: None,
        },
        overwrite: true,
        ..ConversionSettings::default()
    }
}

/// LaVision's XML for `image_name`, with its `<Offset>` and a
/// `TileConfiguration` of `(name, stage x, stage y)`.
fn lavision_xml(image_name: &str, offset: [f64; 2], configuration: &[(&str, [f64; 2])]) -> String {
    let lines: String = configuration
        .iter()
        .map(|(name, [x, y])| format!("\n{name};;({x}, {y})"))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<OME xmlns="http://www.openmicroscopy.org/Schemas/OME/2016-06" xmlns:ca="http://www.openmicroscopy.org/Schemas/CA/2008-02">
  <Image ID="Image:0" Name="{image_name}">
    <Pixels ID="Pixels:0" DimensionOrder="XYZCT" Type="uint8" SizeX="8" SizeY="8" SizeZ="1" SizeC="1" SizeT="1"
            PhysicalSizeX="0.5" PhysicalSizeY="0.5" PhysicalSizeZ="2" BigEndian="false">
      <Channel ID="Channel:0:0" SamplesPerPixel="1"/>
      <TiffData FirstZ="0" IFD="0" PlaneCount="1"/>
    </Pixels>
    <ca:CustomAttributes>
      <Offset Offset_0="{}" Offset_1="{}" Offset_2="0.0" Offset_3="0.0"/>
      <IsPartOfMosaic IsPartOfMosaic="1"/>
      <TileConfiguration TileConfiguration="2{lines}
"/>
    </ca:CustomAttributes>
  </Image>
  <ca:CustomAttributes><PropArray><xyz-Table_X_Overlap Value="0.5"/></PropArray></ca:CustomAttributes>
</OME>"#,
        offset[0], offset[1]
    )
}

/// An 8 x 8 uint8 single-plane TIFF, with `description` as its
/// ImageDescription (an OME-TIFF when that is OME-XML).
fn tiff(path: &Path, description: Option<&str>) {
    let mut entries: Vec<(u16, u16, u32, u32)> = vec![
        (256, 3, 1, 8),
        (257, 3, 1, 8),
        (258, 3, 1, 8),
        (259, 3, 1, 1),
        (262, 3, 1, 1),
    ];
    let header = 8u32;
    let count = 10 + u32::from(description.is_some());
    let ifd_size = 2 + 12 * count + 4;
    let text = description.map(|text| format!("{text}\0"));
    let text_at = header + ifd_size;
    let pixels_at = text_at + text.as_ref().map_or(0, |text| text.len() as u32);
    if let Some(text) = &text {
        entries.push((270, 2, text.len() as u32, text_at));
    }
    entries.extend([
        (273, 4, 1, pixels_at),
        (277, 3, 1, 1),
        (278, 3, 1, 8),
        (279, 4, 1, 64),
        (284, 3, 1, 1),
    ]);
    let mut bytes = Vec::new();
    bytes.extend(b"II");
    bytes.extend(42u16.to_le_bytes());
    bytes.extend(header.to_le_bytes());
    bytes.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, n, value) in &entries {
        bytes.extend(tag.to_le_bytes());
        bytes.extend(kind.to_le_bytes());
        bytes.extend(n.to_le_bytes());
        if *kind == 3 && *n == 1 {
            bytes.extend((*value as u16).to_le_bytes());
            bytes.extend(0u16.to_le_bytes());
        } else {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes.extend(0u32.to_le_bytes());
    if let Some(text) = &text {
        bytes.extend(text.as_bytes());
    }
    bytes.extend((0..64u8).collect::<Vec<_>>());
    fs::write(path, bytes).expect("write TIFF");
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).expect("read json")).expect("parse json")
}

fn level0_translation(group: &Path) -> Option<Value> {
    let attrs = read_json(&group.join("zarr.json"))["attributes"].clone();
    attrs["ome"]["multiscales"][0]["datasets"][0]["coordinateTransformations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["type"] == "translation")
        .map(|row| row["translation"].clone())
}

fn provenance(group: &Path) -> Value {
    read_json(&group.join("zarr.json"))["attributes"]["img2omezarr"]["positions"].clone()
}

const A: &str = "acq[00 x 00]_C00.ome.tif";
const B: &str = "acq[00 x 01]_C00.ome.tif";
const C: &str = "acq[00 x 02]_C00.ome.tif";

/// Three tiles in a row along stage x, 3.5 um apart (4 um tiles).
fn configuration() -> Vec<(&'static str, [f64; 2])> {
    vec![(A, [10.0, 20.0]), (B, [13.5, 20.0]), (C, [17.0, 20.0])]
}

fn convert(
    dir: &Path,
    inputs: &[PathBuf],
    positions: PositionSettings,
) -> anyhow::Result<Vec<PathBuf>> {
    let jobs: Vec<_> = inputs
        .iter()
        .map(|input| ConvertJob {
            input: input.clone(),
            output: OutputTarget::Local(dir.join(format!(
                "{}.ome.zarr",
                input.file_name().unwrap().to_string_lossy()
            ))),
        })
        .collect();
    let outputs = jobs
        .iter()
        .map(|job| match &job.output {
            OutputTarget::Local(path) => path.clone(),
            _ => unreachable!(),
        })
        .collect();
    let mut settings = settings();
    settings.positions = positions;
    convert_many(jobs, settings, NoProgress)?;
    Ok(outputs)
}

#[test]
fn convert_places_lavision_tiles_by_their_own_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = configuration();
    // A and C carry their own XML; B carries a copy of A's
    let inputs: Vec<PathBuf> = [(A, A), (B, A), (C, C)]
        .iter()
        .map(|(file, xml_of)| {
            let offset = config.iter().find(|(name, _)| name == xml_of).unwrap().1;
            let path = temp.path().join(file);
            tiff(&path, Some(&lavision_xml(xml_of, offset, &config)));
            path
        })
        .collect();
    let out = temp.path().join("out");
    fs::create_dir(&out).unwrap();
    let tiles = convert(&out, &inputs, PositionSettings::default()).expect("convert");
    // x,-y,-z: image x = stage x, image y = -stage y; t, c, z, y, x
    for (tile, x) in tiles.iter().zip([10.0, 13.5, 17.0]) {
        let translation = level0_translation(tile).expect("a translation");
        let values: Vec<f64> = translation
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect();
        assert_eq!(
            &values[values.len() - 3..],
            &[0.0, -20.0, x],
            "{}",
            tile.display()
        );
        let provenance = provenance(tile);
        assert_eq!(provenance["source"], "lavision");
        assert_eq!(provenance["stage_axes"], "x,-y,-z");
    }
    let detail = provenance(&tiles[1])["detail"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        detail.contains(&format!("TileConfiguration line for {B}")),
        "{detail}"
    );

    // a configuration that repeats a position (osf-cztq2's row 07 repeats
    // row 00) is refused before anything is written
    let repeated = vec![(A, [10.0, 20.0]), (B, [13.5, 20.0]), (C, [10.0, 20.0])];
    let dir = temp.path().join("repeated");
    fs::create_dir(&dir).unwrap();
    let inputs: Vec<PathBuf> = [A, B, C]
        .iter()
        .map(|file| {
            let offset = repeated.iter().find(|(name, _)| name == file).unwrap().1;
            let path = dir.join(file);
            tiff(&path, Some(&lavision_xml(file, offset, &repeated)));
            path
        })
        .collect();
    let error = convert(&dir, &inputs, PositionSettings::default()).expect_err("refused");
    assert!(error.to_string().contains("same position"), "{error}");
    assert!(!dir.join(format!("{A}.ome.zarr")).exists());
}

/// Tiny OME-Zarr tiles `tile_RRxCC.ome.zarr`, 2 x 8 x 8 voxels of 2 x 0.5 x
/// 0.5 um, at the origin.
fn zarr_tiles(dir: &Path, names: &[&str]) -> Vec<PathBuf> {
    let inputs: Vec<PathBuf> = names
        .iter()
        .map(|name| {
            let input = dir.join(format!(
                "{name}&sizeX=8&sizeY=8&sizeZ=2&physicalSizeX=0.5&physicalSizeY=0.5&physicalSizeZ=2.fake"
            ));
            fs::write(&input, b"").expect("write fake marker");
            input
        })
        .collect();
    let jobs: Vec<_> = inputs
        .iter()
        .zip(names)
        .map(|(input, name)| ConvertJob {
            input: input.clone(),
            output: OutputTarget::Local(dir.join(format!("{name}.ome.zarr"))),
        })
        .collect();
    convert_many(jobs, settings(), NoProgress).expect("convert fake tiles");
    names
        .iter()
        .map(|name| dir.join(format!("{name}.ome.zarr")))
        .collect()
}

/// Every file under `root` but the groups' `zarr.json`, by contents.
fn chunks_and_arrays(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if !(path.ends_with("zarr.json") && path.parent().unwrap().join("0").is_dir()) {
                files.insert(path.clone(), fs::read(&path).expect("read file"));
            }
        }
    }
    files
}

#[test]
fn set_positions_reads_the_originals_matched_by_grid_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let zarr = temp.path().join("zarr");
    let originals = temp.path().join("originals");
    fs::create_dir_all(&zarr).unwrap();
    fs::create_dir_all(&originals).unwrap();
    let tiles = zarr_tiles(&zarr, &["tile_00x00", "tile_00x01", "tile_00x02"]);
    let config = configuration();
    // only A has XML (as on osf-sa3x8, where 3 of 18 files carry it); B has a
    // second channel file, also without
    tiff(
        &originals.join(A),
        Some(&lavision_xml(A, config[0].1, &config)),
    );
    tiff(&originals.join(B), None);
    tiff(&originals.join("acq[00 x 01]_C01.ome.tif"), None);
    tiff(&originals.join(C), None);
    let before = chunks_and_arrays(&zarr);
    let settings = PositionSettings {
        originals: Some(Originals {
            dir: originals.clone(),
            matching: None,
        }),
        ..Default::default()
    };
    let placed = set_positions(&tiles, &settings, false, &|_| {}).expect("set positions");
    let translations: Vec<_> = placed
        .iter()
        .map(|(_, placement)| placement.translation.unwrap())
        .collect();
    assert_eq!(
        translations,
        vec![[0.0, -20.0, 10.0], [0.0, -20.0, 13.5], [0.0, -20.0, 17.0]]
    );
    assert_eq!(
        level0_translation(&tiles[2]),
        Some(json!([0.0, 0.0, 0.0, -20.0, 17.0]))
    );
    assert_eq!(provenance(&tiles[2])["source"], "lavision");
    assert_eq!(
        chunks_and_arrays(&zarr),
        before,
        "only group metadata changed"
    );

    // two originals for one tile that disagree are refused, unless narrowed
    let other = vec![
        (A, [10.0, 20.0]),
        ("acq[00 x 01]_C01.ome.tif", [99.0, 20.0]),
    ];
    tiff(
        &originals.join("acq[00 x 01]_C01.ome.tif"),
        Some(&lavision_xml(
            "acq[00 x 01]_C01.ome.tif",
            [99.0, 20.0],
            &other,
        )),
    );
    let error = set_positions(&tiles, &settings, true, &|_| {}).expect_err("ambiguous");
    assert!(error.to_string().contains("--originals-match"), "{error}");
    let narrowed = PositionSettings {
        originals: Some(Originals {
            dir: originals.clone(),
            matching: Some(regex::Regex::new("_C00").unwrap()),
        }),
        ..Default::default()
    };
    set_positions(&tiles, &narrowed, true, &|_| {}).expect("narrowed");

    // an image no original matches is refused
    let lone = zarr_tiles(&zarr, &["tile_05x05"]);
    let error = set_positions(&lone, &narrowed, true, &|_| {}).expect_err("no original");
    assert!(error.to_string().contains("no file under"), "{error}");
}

#[cfg(feature = "cli")]
#[test]
fn cli_from_originals() {
    use std::process::Command;

    let temp = tempfile::tempdir().expect("tempdir");
    let tiles = zarr_tiles(temp.path(), &["tile_00x00", "tile_00x01"]);
    let originals = temp.path().join("originals");
    fs::create_dir(&originals).unwrap();
    let config = configuration();
    tiff(
        &originals.join(A),
        Some(&lavision_xml(A, config[0].1, &config)),
    );
    tiff(
        &originals.join(B),
        Some(&lavision_xml(A, config[0].1, &config)),
    );
    let output = Command::new(env!("CARGO_BIN_EXE_img2omezarr"))
        .arg("set-positions")
        .arg("--from-originals")
        .arg(&originals)
        .args(["--originals-match", "C00", "--dry-run"])
        .args(&tiles)
        .output()
        .expect("run set-positions");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("tile_00x01.ome.zarr: z 0 y -20 x 13.5 micrometer (lavision)"),
        "{stdout}"
    );
    // --from-originals does not combine with another source
    let status = Command::new(env!("CARGO_BIN_EXE_img2omezarr"))
        .arg("set-positions")
        .arg("--from-originals")
        .arg(&originals)
        .args(["--overlap", "1", "--grid-unit", "voxel"])
        .args(&tiles)
        .output()
        .expect("run set-positions");
    assert!(!status.status.success());
}

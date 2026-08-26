#![cfg(feature = "core-bioformats")]

use std::fs;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, OutputTarget,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;

#[test]
fn fake_spw_metadata_writes_ngff_05_plate_well_layout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join(
        "SPW&sizeX=4&sizeY=4&sizeC=1&plates=1&plateRows=1&plateCols=1&fields=2&plateAcqs=1.fake",
    );
    let output = temp.path().join("spw.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.tile_width = 4;
    settings.tile_height = 4;
    settings.compression = CompressionSettings {
        codec: CompressionCodec::None,
        level: None,
    };
    settings.overwrite = true;

    convert_many(
        vec![ConvertJob {
            input,
            output: OutputTarget::Local(output.clone()),
        }],
        settings,
        NoProgress,
    )
    .expect("convert fake SPW");

    let root = read_json(&output.join("zarr.json"));
    let plate = &root["attributes"]["ome"]["plate"];
    assert_eq!(plate["version"], "0.5");
    assert_eq!(plate["name"], "Plate 0");
    assert_eq!(plate["field_count"], 2);
    assert_eq!(plate["rows"][0]["name"], "A");
    assert_eq!(plate["columns"][0]["name"], "1");
    assert_eq!(plate["wells"][0]["path"], "A/1");
    assert_eq!(plate["wells"][0]["rowIndex"], 0);
    assert_eq!(plate["wells"][0]["columnIndex"], 0);

    let ome_group = read_json(&output.join("OME/zarr.json"));
    assert_eq!(ome_group["attributes"]["ome"]["series"][0], "A/1/0");
    assert_eq!(ome_group["attributes"]["ome"]["series"][1], "A/1/1");

    let well = read_json(&output.join("A/1/zarr.json"));
    let images = &well["attributes"]["ome"]["well"]["images"];
    assert_eq!(images[0]["path"], "0");
    assert_eq!(images[0]["acquisition"], 0);
    assert_eq!(images[1]["path"], "1");
    assert_eq!(images[1]["acquisition"], 0);

    assert!(output.join("A/1/0/0/zarr.json").exists());
    assert!(output.join("A/1/1/0/zarr.json").exists());
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("read zarr metadata"))
        .expect("parse zarr metadata")
}

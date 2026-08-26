#![cfg(feature = "core-bioformats")]

use std::fs;
use std::sync::Arc;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, OutputTarget,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;
use zarrs::array::{Array as ZarrArray, ArraySubset};
use zarrs::filesystem::FilesystemStore;

#[test]
fn ngff_05_writes_blosc_codec_metadata_and_readable_pixels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("v05_blosc&sizeX=4&sizeY=4.fake");
    let output = temp.path().join("v05_blosc.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.tile_width = 4;
    settings.tile_height = 4;
    settings.compression = CompressionSettings {
        codec: CompressionCodec::Blosc,
        level: Some(5),
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
    .expect("convert NGFF 0.5 with blosc");

    let array_meta: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("0/0/zarr.json")).expect("read array meta"))
            .expect("parse array meta");
    let codec = array_meta["codecs"]
        .as_array()
        .expect("codec array")
        .iter()
        .find(|codec| codec["name"] == "blosc")
        .expect("blosc codec metadata");
    assert_eq!(codec["configuration"]["cname"], "zstd");
    assert_eq!(codec["configuration"]["clevel"], 5);

    let store = Arc::new(FilesystemStore::new(&output).expect("open store"));
    let array = ZarrArray::open(store, "/0/0").expect("open v3 blosc array");
    let subset =
        ArraySubset::new_with_start_shape(vec![0; 5], vec![1, 1, 1, 4, 4]).expect("subset");
    let pixels = array
        .retrieve_array_subset::<Vec<u8>>(&subset)
        .expect("read blosc pixels");
    assert_eq!(pixels.len(), 16);
}

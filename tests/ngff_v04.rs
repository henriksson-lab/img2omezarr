#![cfg(feature = "core-bioformats")]

use std::fs;
use std::sync::Arc;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, NgffVersion,
    OutputTarget,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;
use zarrs::array::{Array as ZarrArray, ArraySubset};
use zarrs::filesystem::FilesystemStore;

#[test]
fn ngff_04_writes_zarr_v2_metadata_and_readable_pixels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("v04&sizeX=4&sizeY=4&sizeC=1.fake");
    let output = temp.path().join("v04.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.ngff_version = NgffVersion::V04;
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
    .expect("convert NGFF 0.4");

    assert!(output.join(".zgroup").exists());
    assert!(output.join(".zattrs").exists());
    assert!(output.join("0/.zgroup").exists());
    assert!(output.join("0/.zattrs").exists());
    assert!(output.join("0/0/.zarray").exists());
    assert!(!output.join("zarr.json").exists());

    let root_attrs = read_json(&output.join(".zattrs"));
    assert_eq!(root_attrs["bioformats2raw.layout"], 3);
    assert!(root_attrs.get("ome").is_none());

    let image_attrs = read_json(&output.join("0/.zattrs"));
    assert_eq!(image_attrs["multiscales"][0]["version"], "0.4");
    assert_eq!(image_attrs["multiscales"][0]["datasets"][0]["path"], "0");

    let array_meta = read_json(&output.join("0/0/.zarray"));
    assert_eq!(array_meta["zarr_format"], 2);
    assert_eq!(array_meta["shape"], serde_json::json!([1, 1, 1, 4, 4]));
    assert_eq!(array_meta["dimension_separator"], "/");
    assert!(array_meta["compressor"].is_null());

    let store = Arc::new(FilesystemStore::new(&output).expect("open store"));
    let array = ZarrArray::open(store, "/0/0").expect("open v2 array");
    let subset =
        ArraySubset::new_with_start_shape(vec![0; 5], vec![1, 1, 1, 4, 4]).expect("subset");
    let pixels = array
        .retrieve_array_subset::<Vec<u8>>(&subset)
        .expect("read pixels");
    assert_eq!(pixels.len(), 16);
}

#[test]
fn ngff_04_rejects_zstd_compression() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("v04_zstd&sizeX=2&sizeY=2.fake");
    let output = temp.path().join("v04_zstd.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.ngff_version = NgffVersion::V04;
    settings.compression = CompressionSettings {
        codec: CompressionCodec::Zstd,
        level: Some(3),
    };

    let err = convert_many(
        vec![ConvertJob {
            input,
            output: OutputTarget::Local(output),
        }],
        settings,
        NoProgress,
    )
    .expect_err("0.4 zstd should be rejected");
    assert!(err.to_string().contains("--compression blosc or none"));
}

#[test]
fn ngff_04_writes_blosc_compressor_metadata_and_readable_pixels() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("v04_blosc&sizeX=4&sizeY=4.fake");
    let output = temp.path().join("v04_blosc.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.ngff_version = NgffVersion::V04;
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
    .expect("convert NGFF 0.4 with blosc");

    let array_meta = read_json(&output.join("0/0/.zarray"));
    assert_eq!(array_meta["compressor"]["id"], "blosc");
    assert_eq!(array_meta["compressor"]["cname"], "zstd");
    assert_eq!(array_meta["compressor"]["clevel"], 5);

    let store = Arc::new(FilesystemStore::new(&output).expect("open store"));
    let array = ZarrArray::open(store, "/0/0").expect("open v2 blosc array");
    let subset =
        ArraySubset::new_with_start_shape(vec![0; 5], vec![1, 1, 1, 4, 4]).expect("subset");
    let pixels = array
        .retrieve_array_subset::<Vec<u8>>(&subset)
        .expect("read blosc pixels");
    assert_eq!(pixels.len(), 16);
}

fn read_json(path: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(path).expect("read zarr metadata"))
        .expect("parse zarr metadata")
}

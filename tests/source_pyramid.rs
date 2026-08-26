#![cfg(feature = "core-bioformats")]

use std::fs;
use std::sync::Arc;

use bioformats_rs::tiff::PyramidOmeTiffWriter;
use bioformats_rs::{FormatWriter, ImageMetadata, PixelType};
use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, OutputTarget,
    ResolutionPolicy,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;
use zarrs::array::{Array as ZarrArray, ArraySubset};
use zarrs::filesystem::FilesystemStore;

#[test]
fn existing_source_resolutions_are_written_without_generated_downsampling() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join("native_pyramid.ome.tif");
    let output = temp.path().join("native_pyramid.ome.zarr");

    let mut meta = ImageMetadata::default();
    meta.size_x = 4;
    meta.size_y = 4;
    meta.pixel_type = PixelType::Uint8;
    meta.size_z = 1;
    meta.size_c = 1;
    meta.size_t = 1;
    meta.image_count = 1;

    let full_plane: Vec<u8> = (0u8..16).collect();
    let native_reduced_plane = vec![101, 102, 103, 104];

    let mut writer = PyramidOmeTiffWriter::new();
    writer.set_metadata(&meta).expect("set metadata");
    writer.set_id(&input).expect("set id");
    writer.save_bytes(0, &full_plane).expect("write full plane");
    writer.add_resolution_level(vec![native_reduced_plane.clone()]);
    writer.close().expect("close pyramid tiff");

    let mut settings = ConversionSettings::default();
    settings.resolution_policy = ResolutionPolicy::ExistingOrTargetMinSize(2);
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
    .expect("convert native pyramid");

    let level0 = read_u8_array(&output, "/0/0", [1, 1, 1, 4, 4]);
    let level1 = read_u8_array(&output, "/0/1", [1, 1, 1, 2, 2]);
    assert_eq!(level0, full_plane);
    assert_eq!(level1, native_reduced_plane);

    let image_group: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("0/zarr.json")).expect("read image group"))
            .expect("parse image group");
    let datasets = &image_group["attributes"]["ome"]["multiscales"][0]["datasets"];
    assert_eq!(datasets.as_array().expect("datasets").len(), 2);
    assert_eq!(datasets[0]["path"], "0");
    assert_eq!(datasets[1]["path"], "1");
}

fn read_u8_array<const N: usize>(root: &std::path::Path, node: &str, shape: [usize; N]) -> Vec<u8> {
    let store = Arc::new(FilesystemStore::new(root).expect("open store"));
    let array = ZarrArray::open(store, node).expect("open array");
    let subset = ArraySubset::new_with_start_shape(
        vec![0; N],
        shape.iter().map(|&dim| dim as u64).collect::<Vec<_>>(),
    )
    .expect("subset");
    array
        .retrieve_array_subset::<Vec<u8>>(&subset)
        .expect("read array")
}

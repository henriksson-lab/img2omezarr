#![cfg(feature = "core-bioformats")]

use std::fs;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    OutputTarget, ResolutionPolicy,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;
use serde_json::json;

#[test]
fn physical_sizes_are_written_to_coordinate_transform_scales() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp.path().join(
        "physical&sizeX=4&sizeY=4&physicalSizeX=0.25&physicalSizeY=0.5&physicalSizeZ=1.5.fake",
    );
    let output = temp.path().join("physical.ome.zarr");
    fs::write(&input, b"").expect("write fake marker");

    let mut settings = ConversionSettings::default();
    settings.tile_width = 4;
    settings.tile_height = 4;
    settings.resolution_policy = ResolutionPolicy::TargetMinSize(2);
    settings.downsampling = Downsampling::Average;
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
    .expect("convert physical metadata fixture");

    let image_group: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("0/zarr.json")).expect("read image group"))
            .expect("parse image group");
    let multiscale = &image_group["attributes"]["ome"]["multiscales"][0];
    assert_eq!(
        multiscale["metadata"]["method"],
        "img2omezarr average downsampling"
    );
    assert_eq!(
        multiscale["datasets"][0]["coordinateTransformations"][0]["scale"],
        json!([1.0, 1.0, 1.5, 0.5, 0.25])
    );
    assert_eq!(
        multiscale["datasets"][1]["coordinateTransformations"][0]["scale"],
        json!([1.0, 1.0, 1.5, 1.0, 0.5])
    );
}

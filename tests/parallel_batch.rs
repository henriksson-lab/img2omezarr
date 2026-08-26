#![cfg(feature = "core-bioformats")]

use std::fs;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, OutputTarget,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;

#[test]
fn batch_jobs_can_run_with_multiple_workers_and_keep_report_order() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input_a = temp.path().join("a&sizeX=2&sizeY=2.fake");
    let input_b = temp.path().join("b&sizeX=2&sizeY=2.fake");
    let output_a = temp.path().join("a.ome.zarr");
    let output_b = temp.path().join("b.ome.zarr");
    fs::write(&input_a, b"").expect("write fake input a");
    fs::write(&input_b, b"").expect("write fake input b");

    let mut settings = ConversionSettings::default();
    settings.tile_width = 2;
    settings.tile_height = 2;
    settings.compression = CompressionSettings {
        codec: CompressionCodec::None,
        level: None,
    };
    settings.max_workers = 2;

    let reports = convert_many(
        vec![
            ConvertJob {
                input: input_a,
                output: OutputTarget::Local(output_a.clone()),
            },
            ConvertJob {
                input: input_b,
                output: OutputTarget::Local(output_b.clone()),
            },
        ],
        settings,
        NoProgress,
    )
    .expect("parallel batch conversion");

    assert_eq!(reports.len(), 2);
    assert_eq!(reports[0].output, output_a);
    assert_eq!(reports[1].output, output_b);
    assert!(reports[0].output.join("0/0/zarr.json").exists());
    assert!(reports[1].output.join("0/0/zarr.json").exists());
}

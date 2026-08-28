#![cfg(all(target_os = "linux", feature = "core-bioformats"))]

use std::fs;

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    OutputTarget, ResolutionPolicy,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;

#[test]
fn synthetic_large_conversion_stays_under_memory_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let input = temp
        .path()
        .join("memory&sizeX=2048&sizeY=2048&sizeZ=4&sizeC=2.fake");
    let output = temp.path().join("memory.ome.zarr");
    fs::write(&input, b"").expect("write fake input");

    let settings = ConversionSettings {
        tile_width: 256,
        tile_height: 256,
        chunk_depth: 1,
        resolution_policy: ResolutionPolicy::TargetMinSize(256),
        downsampling: Downsampling::Average,
        compression: CompressionSettings {
            codec: CompressionCodec::None,
            level: None,
        },
        overwrite: true,
        max_workers: 1,
        ..ConversionSettings::default()
    };

    convert_many(
        vec![ConvertJob {
            input,
            output: OutputTarget::Local(output),
        }],
        settings,
        NoProgress,
    )
    .expect("convert synthetic large image");

    let peak_rss_kib = peak_rss_kib().expect("read VmHWM from /proc/self/status");
    let limit_kib = std::env::var("IMG2OMEZARR_MEMORY_TEST_LIMIT_KIB")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(768 * 1024);
    assert!(
        peak_rss_kib <= limit_kib,
        "peak RSS {peak_rss_kib} KiB exceeded limit {limit_kib} KiB"
    );
}

fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?;
        value.split_whitespace().next()?.parse().ok()
    })
}

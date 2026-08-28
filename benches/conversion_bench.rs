use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    OutputTarget, ResolutionPolicy,
};
use img2omezarr::convert::convert_many;
use img2omezarr::convert::progress::NoProgress;

#[derive(Clone, Copy)]
struct Workload {
    name: &'static str,
    size_x: u32,
    size_y: u32,
    size_z: u32,
    size_c: u32,
    target_min_size: u32,
    tile_width: usize,
    tile_height: usize,
    compression: CompressionCodec,
    workers: usize,
}

enum BenchTarget {
    Local,
    #[cfg(feature = "upload-s3")]
    S3(S3BenchConfig),
}

impl BenchTarget {
    fn name(&self) -> &'static str {
        match self {
            Self::Local => "local",
            #[cfg(feature = "upload-s3")]
            Self::S3(_) => "s3",
        }
    }
}

#[cfg(feature = "upload-s3")]
#[derive(Clone)]
struct S3BenchConfig {
    bucket: String,
    prefix: String,
    region: Option<String>,
    endpoint: Option<String>,
}

struct BenchReport {
    workload: Workload,
    output_target: &'static str,
    elapsed: Duration,
    output_bytes: u64,
    peak_rss_kib: Option<u64>,
}

fn main() -> anyhow::Result<()> {
    let workloads = [
        Workload {
            name: "small",
            size_x: 256,
            size_y: 256,
            size_z: 1,
            size_c: 1,
            target_min_size: 64,
            tile_width: 128,
            tile_height: 128,
            compression: CompressionCodec::None,
            workers: 1,
        },
        Workload {
            name: "medium",
            size_x: 1024,
            size_y: 1024,
            size_z: 4,
            size_c: 2,
            target_min_size: 128,
            tile_width: 256,
            tile_height: 256,
            compression: CompressionCodec::Zstd,
            workers: 2,
        },
        Workload {
            name: "synthetic-large",
            size_x: 4096,
            size_y: 4096,
            size_z: 2,
            size_c: 3,
            target_min_size: 512,
            tile_width: 512,
            tile_height: 512,
            compression: CompressionCodec::Zstd,
            workers: 4,
        },
    ];

    println!(
        "workload,output_target,size_x,size_y,size_z,size_c,target_min_size,tile_width,tile_height,compression,workers,elapsed_ms,output_bytes,peak_rss_kib"
    );
    #[cfg(feature = "upload-s3")]
    let s3_config = s3_bench_config();
    for workload in workloads.iter().copied() {
        let report = run_workload(workload, &BenchTarget::Local)?;
        println!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            report.workload.name,
            report.output_target,
            report.workload.size_x,
            report.workload.size_y,
            report.workload.size_z,
            report.workload.size_c,
            report.workload.target_min_size,
            report.workload.tile_width,
            report.workload.tile_height,
            compression_name(report.workload.compression),
            report.workload.workers,
            report.elapsed.as_millis(),
            report.output_bytes,
            report
                .peak_rss_kib
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unavailable".to_string())
        );
        #[cfg(feature = "upload-s3")]
        if let Some(config) = &s3_config {
            if matches!(workload.name, "small" | "medium") {
                let report = run_workload(workload, &BenchTarget::S3(config.clone()))?;
                println!(
                    "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                    report.workload.name,
                    report.output_target,
                    report.workload.size_x,
                    report.workload.size_y,
                    report.workload.size_z,
                    report.workload.size_c,
                    report.workload.target_min_size,
                    report.workload.tile_width,
                    report.workload.tile_height,
                    compression_name(report.workload.compression),
                    report.workload.workers,
                    report.elapsed.as_millis(),
                    report.output_bytes,
                    report
                        .peak_rss_kib
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "unavailable".to_string())
                );
            }
        }
    }

    Ok(())
}

fn run_workload(workload: Workload, target: &BenchTarget) -> anyhow::Result<BenchReport> {
    let temp = tempfile::tempdir()?;
    let input = fake_input_path(temp.path(), workload);
    let local_output = temp.path().join(format!("{}.ome.zarr", workload.name));
    fs::write(&input, b"")?;

    let settings = ConversionSettings {
        tile_width: workload.tile_width,
        tile_height: workload.tile_height,
        chunk_depth: 1,
        resolution_policy: ResolutionPolicy::TargetMinSize(workload.target_min_size),
        downsampling: Downsampling::Average,
        compression: CompressionSettings {
            codec: workload.compression,
            level: match workload.compression {
                CompressionCodec::None => None,
                CompressionCodec::Zstd | CompressionCodec::Blosc => Some(3),
            },
        },
        overwrite: true,
        max_workers: workload.workers,
        ..ConversionSettings::default()
    };

    let start = Instant::now();
    convert_many(
        vec![ConvertJob {
            input,
            output: output_target(target, &local_output, workload)?,
        }],
        settings,
        NoProgress,
    )?;
    let elapsed = start.elapsed();
    let output_bytes = dir_size(&local_output).unwrap_or(0);

    Ok(BenchReport {
        workload,
        output_target: target.name(),
        elapsed,
        output_bytes,
        peak_rss_kib: peak_rss_kib(),
    })
}

fn output_target(
    target: &BenchTarget,
    local_output: &Path,
    #[cfg_attr(not(feature = "upload-s3"), allow(unused_variables))] workload: Workload,
) -> anyhow::Result<OutputTarget> {
    match target {
        BenchTarget::Local => Ok(OutputTarget::Local(local_output.to_path_buf())),
        #[cfg(feature = "upload-s3")]
        BenchTarget::S3(config) => Ok(OutputTarget::Upload(
            img2omezarr::convert::config::UploadTarget::S3 {
                bucket: config.bucket.clone(),
                prefix: format!("{}/{}.ome.zarr", config.prefix, workload.name),
                region: config.region.clone(),
                endpoint: config.endpoint.clone(),
                credentials: None,
            },
        )),
    }
}

#[cfg(feature = "upload-s3")]
fn s3_bench_config() -> Option<S3BenchConfig> {
    Some(S3BenchConfig {
        bucket: std::env::var("IMG2OMEZARR_S3_BENCH_BUCKET").ok()?,
        prefix: std::env::var("IMG2OMEZARR_S3_BENCH_PREFIX")
            .unwrap_or_else(|_| "img2omezarr-benchmark".to_string())
            .trim_matches('/')
            .to_string(),
        region: std::env::var("AWS_REGION")
            .ok()
            .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok()),
        endpoint: std::env::var("IMG2OMEZARR_S3_BENCH_ENDPOINT")
            .ok()
            .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok()),
    })
}

fn fake_input_path(root: &Path, workload: Workload) -> PathBuf {
    root.join(format!(
        "{}&sizeX={}&sizeY={}&sizeZ={}&sizeC={}.fake",
        workload.name, workload.size_x, workload.size_y, workload.size_z, workload.size_c
    ))
}

fn dir_size(path: &Path) -> anyhow::Result<u64> {
    let mut size = 0;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let metadata = entry.metadata()?;
        if metadata.is_dir() {
            size += dir_size(&entry.path())?;
        } else if metadata.is_file() {
            size += metadata.len();
        }
    }
    Ok(size)
}

fn compression_name(codec: CompressionCodec) -> &'static str {
    match codec {
        CompressionCodec::Zstd => "zstd",
        CompressionCodec::Blosc => "blosc",
        CompressionCodec::None => "none",
    }
}

#[cfg(target_os = "linux")]
fn peak_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmHWM:")?;
        value.split_whitespace().next()?.parse().ok()
    })
}

#[cfg(not(target_os = "linux"))]
fn peak_rss_kib() -> Option<u64> {
    None
}

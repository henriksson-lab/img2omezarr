use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};
use img2omezarr::convert::config::{
    CompressionCodec, CompressionSettings, ConversionSettings, ConvertJob, Downsampling,
    NgffVersion, OutputTarget, ResolutionPolicy, SeriesSelection, UploadTarget,
};
use img2omezarr::convert::progress::ProgressSink;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Convert(ConvertArgs),
    Batch(BatchArgs),
    Inspect(InspectArgs),
}

#[derive(Debug, Parser)]
struct ConvertArgs {
    input: PathBuf,
    output: PathBuf,
    #[arg(long)]
    upload_local: bool,
    #[arg(long)]
    s3_region: Option<String>,
    #[arg(long)]
    s3_endpoint: Option<String>,
    #[command(flatten)]
    settings: SettingsArgs,
}

#[derive(Debug, Parser)]
struct BatchArgs {
    #[arg(long)]
    output_dir: PathBuf,
    #[arg(long)]
    upload_local: bool,
    #[arg(long)]
    s3_region: Option<String>,
    #[arg(long)]
    s3_endpoint: Option<String>,
    inputs: Vec<PathBuf>,
    #[command(flatten)]
    settings: SettingsArgs,
}

#[derive(Debug, Parser)]
struct InspectArgs {
    input: PathBuf,
    #[arg(long, default_value = "json")]
    format: InspectFormatArg,
}

#[derive(Debug, Clone, Parser)]
struct SettingsArgs {
    #[arg(long, default_value = "0.5")]
    ngff_version: NgffArg,
    #[arg(long, default_value = "all")]
    series: String,
    #[arg(long, default_value_t = 1024)]
    tile_width: usize,
    #[arg(long, default_value_t = 1024)]
    tile_height: usize,
    #[arg(long, default_value_t = 1)]
    chunk_depth: usize,
    #[arg(long)]
    resolutions: Option<usize>,
    #[arg(long)]
    use_existing_resolutions: bool,
    #[arg(long, default_value_t = 256)]
    target_min_size: u32,
    #[arg(long, default_value = "zstd")]
    compression: CompressionArg,
    #[arg(long, default_value = "nearest")]
    downsampling: DownsamplingArg,
    #[arg(long, default_value_t = 3)]
    compression_level: i32,
    #[arg(long)]
    workers: Option<usize>,
    #[arg(long)]
    overwrite: bool,
    #[arg(long)]
    no_omero: bool,
    #[arg(long)]
    no_ome_xml: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum NgffArg {
    #[value(name = "0.5")]
    V05,
    #[value(name = "0.4")]
    V04,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CompressionArg {
    Zstd,
    Blosc,
    None,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DownsamplingArg {
    Nearest,
    Average,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum InspectFormatArg {
    Json,
    Markdown,
}

impl From<NgffArg> for NgffVersion {
    fn from(value: NgffArg) -> Self {
        match value {
            NgffArg::V05 => Self::V05,
            NgffArg::V04 => Self::V04,
        }
    }
}

impl SettingsArgs {
    fn to_settings(&self) -> anyhow::Result<ConversionSettings> {
        let series = if self.series.eq_ignore_ascii_case("all") {
            SeriesSelection::All
        } else {
            let indices = self
                .series
                .split(',')
                .map(|part| part.trim().parse::<usize>())
                .collect::<Result<Vec<_>, _>>()?;
            SeriesSelection::Indices(indices)
        };
        Ok(ConversionSettings {
            ngff_version: self.ngff_version.into(),
            series,
            tile_width: self.tile_width,
            tile_height: self.tile_height,
            chunk_depth: self.chunk_depth,
            resolution_policy: if let Some(resolutions) = self.resolutions {
                ResolutionPolicy::ExplicitLevels(resolutions)
            } else if self.use_existing_resolutions {
                ResolutionPolicy::ExistingOrTargetMinSize(self.target_min_size)
            } else {
                ResolutionPolicy::TargetMinSize(self.target_min_size)
            },
            compression: CompressionSettings {
                codec: match self.compression {
                    CompressionArg::Zstd => CompressionCodec::Zstd,
                    CompressionArg::Blosc => CompressionCodec::Blosc,
                    CompressionArg::None => CompressionCodec::None,
                },
                level: match self.compression {
                    CompressionArg::Zstd | CompressionArg::Blosc => Some(self.compression_level),
                    CompressionArg::None => None,
                },
            },
            downsampling: match self.downsampling {
                DownsamplingArg::Nearest => Downsampling::Nearest,
                DownsamplingArg::Average => Downsampling::Average,
            },
            overwrite: self.overwrite,
            write_omero_metadata: !self.no_omero,
            write_ome_xml: !self.no_ome_xml,
            max_workers: self.workers.unwrap_or_else(|| {
                std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1)
            }),
            ..ConversionSettings::default()
        })
    }
}

struct CliProgress;

impl ProgressSink for CliProgress {
    fn job_started(&self, _index: usize, input: &std::path::Path) {
        eprintln!("converting {}", input.display());
    }

    fn series_started(&self, series: usize, levels: usize, chunks: usize) {
        eprintln!("series {series}: {levels} levels, {chunks} chunks");
    }

    fn chunk_finished(&self, series: usize, level: usize, done: usize, total: usize) {
        eprintln!("series {series} level {level}: {done}/{total}");
    }

    fn job_finished(&self, _index: usize, output: &std::path::Path) {
        eprintln!("wrote {}", output.display());
    }
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();
    let cli = Cli::parse();
    match cli.command {
        Command::Convert(args) => {
            let settings = args.settings.to_settings()?;
            let output = output_target(
                args.output,
                args.upload_local,
                args.s3_region.as_deref(),
                args.s3_endpoint.as_deref(),
            )?;
            img2omezarr::convert::convert_many(
                vec![ConvertJob {
                    input: args.input,
                    output,
                }],
                settings,
                CliProgress,
            )?;
        }
        Command::Batch(args) => {
            let settings = args.settings.to_settings()?;
            let jobs = args
                .inputs
                .into_iter()
                .map(|input| {
                    let stem = input
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("image")
                        .to_string();
                    let output = args.output_dir.join(format!("{stem}.ome.zarr"));
                    Ok(ConvertJob {
                        input,
                        output: output_target(
                            output,
                            args.upload_local,
                            args.s3_region.as_deref(),
                            args.s3_endpoint.as_deref(),
                        )?,
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            img2omezarr::convert::convert_many(jobs, settings, CliProgress)?;
        }
        Command::Inspect(args) => inspect_input(&args.input, args.format)?,
    }
    Ok(())
}

fn inspect_input(input: &std::path::Path, format: InspectFormatArg) -> anyhow::Result<()> {
    let report = inspect_report(input)?;
    match format {
        InspectFormatArg::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        InspectFormatArg::Markdown => print_inspect_markdown(&report),
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct InspectReport {
    input: String,
    series_count: usize,
    series: Vec<InspectSeries>,
}

#[derive(serde::Serialize)]
struct InspectSeries {
    index: usize,
    size_x: u32,
    size_y: u32,
    size_z: u32,
    size_c: u32,
    size_t: u32,
    pixel_type: String,
    bits_per_pixel: u16,
    image_count: u32,
    dimension_order: String,
    is_rgb: bool,
    physical_size_x: Option<String>,
    physical_size_y: Option<String>,
    physical_size_z: Option<String>,
    pyramid_levels: usize,
}

fn inspect_report(input: &std::path::Path) -> anyhow::Result<InspectReport> {
    let mut source = img2omezarr::convert::reader::BioformatsImageSource::open(input)?;
    let series_count = source.series_count();
    let mut series = Vec::with_capacity(series_count);
    let ome_metadata = source.ome_metadata();
    for index in 0..series_count {
        source.set_series(index)?;
        source.set_resolution(0)?;
        let meta = source.metadata();
        let image = ome_metadata.as_ref().and_then(|ome| ome.images.get(index));
        series.push(InspectSeries {
            index,
            size_x: meta.size_x,
            size_y: meta.size_y,
            size_z: meta.size_z,
            size_c: meta.size_c,
            size_t: meta.size_t,
            pixel_type: format!("{:?}", meta.pixel_type),
            bits_per_pixel: meta.bits_per_pixel,
            image_count: meta.image_count,
            dimension_order: format!("{:?}", meta.dimension_order),
            is_rgb: meta.is_rgb,
            physical_size_x: physical_size_value(
                image.and_then(|image| image.physical_size_x),
                &meta,
                &["physicalSizeX", "physical_size_x"],
            ),
            physical_size_y: physical_size_value(
                image.and_then(|image| image.physical_size_y),
                &meta,
                &["physicalSizeY", "physical_size_y"],
            ),
            physical_size_z: physical_size_value(
                image.and_then(|image| image.physical_size_z),
                &meta,
                &["physicalSizeZ", "physical_size_z"],
            ),
            pyramid_levels: source.resolution_count(),
        });
    }
    Ok(InspectReport {
        input: input.display().to_string(),
        series_count,
        series,
    })
}

fn physical_size_value(
    ome_value: Option<f64>,
    meta: &bioformats_rs::ImageMetadata,
    keys: &[&str],
) -> Option<String> {
    ome_value.map(|value| value.to_string()).or_else(|| {
        keys.iter()
            .find_map(|key| meta.series_metadata.get(*key))
            .map(|value| format!("{value:?}"))
    })
}

fn print_inspect_markdown(report: &InspectReport) {
    println!("| Series | Dimensions | C | Z | T | Pixel type | Physical sizes | Pyramid levels |");
    println!("| --- | --- | --- | --- | --- | --- | --- | --- |");
    for series in &report.series {
        let physical_sizes = format!(
            "X={}, Y={}, Z={}",
            series.physical_size_x.as_deref().unwrap_or("none"),
            series.physical_size_y.as_deref().unwrap_or("none"),
            series.physical_size_z.as_deref().unwrap_or("none")
        );
        println!(
            "| {} | {} x {} | {} | {} | {} | {} {}-bit | {} | {} |",
            series.index,
            series.size_x,
            series.size_y,
            series.size_c,
            series.size_z,
            series.size_t,
            series.pixel_type,
            series.bits_per_pixel,
            physical_sizes,
            series.pyramid_levels
        );
    }
}

fn output_target(
    path: PathBuf,
    upload_local: bool,
    s3_region: Option<&str>,
    s3_endpoint: Option<&str>,
) -> anyhow::Result<OutputTarget> {
    #[cfg(feature = "upload-s3")]
    if let Some(target) = s3_output_target(&path, s3_region, s3_endpoint)? {
        return Ok(target);
    }
    reject_s3_flags(s3_region, s3_endpoint)?;
    if upload_local {
        Ok(OutputTarget::Upload(UploadTarget::Local(path)))
    } else {
        Ok(OutputTarget::Local(path))
    }
}

#[cfg(feature = "upload-s3")]
fn s3_output_target(
    path: &std::path::Path,
    s3_region: Option<&str>,
    s3_endpoint: Option<&str>,
) -> anyhow::Result<Option<OutputTarget>> {
    let value = path.to_string_lossy();
    let Some(rest) = value.strip_prefix("s3://") else {
        return Ok(None);
    };
    let (bucket, prefix) = rest
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("S3 output must be s3://bucket/prefix"))?;
    if bucket.is_empty() || prefix.is_empty() {
        anyhow::bail!("S3 output must be s3://bucket/prefix");
    }
    Ok(Some(OutputTarget::Upload(UploadTarget::S3 {
        bucket: bucket.to_string(),
        prefix: prefix.trim_matches('/').to_string(),
        region: s3_region.map(str::to_string).or_else(|| {
            std::env::var("AWS_REGION")
                .ok()
                .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
        }),
        endpoint: s3_endpoint
            .map(str::to_string)
            .or_else(|| std::env::var("AWS_ENDPOINT_URL").ok()),
        credentials: None,
    })))
}

#[cfg(not(feature = "upload-s3"))]
fn reject_s3_flags(s3_region: Option<&str>, s3_endpoint: Option<&str>) -> anyhow::Result<()> {
    if s3_region.is_some() || s3_endpoint.is_some() {
        anyhow::bail!("S3 flags require building with --features upload-s3");
    }
    Ok(())
}

#[cfg(feature = "upload-s3")]
fn reject_s3_flags(_s3_region: Option<&str>, _s3_endpoint: Option<&str>) -> anyhow::Result<()> {
    Ok(())
}

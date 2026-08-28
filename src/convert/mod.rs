pub mod axes;
pub mod config;
pub mod downsample;
pub mod dtype;
pub mod metadata;
pub mod planner;
pub mod progress;
pub mod upload;

#[cfg(feature = "core-bioformats")]
pub mod reader;
#[cfg(feature = "core-bioformats")]
pub mod writer;

use config::{ConversionSettings, ConvertJob};
use progress::ProgressSink;

#[derive(Debug, Clone)]
pub struct ConversionReport {
    pub input: std::path::PathBuf,
    pub output: std::path::PathBuf,
    pub series_written: Vec<usize>,
}

pub fn convert_many<P>(
    jobs: Vec<ConvertJob>,
    settings: ConversionSettings,
    progress: P,
) -> anyhow::Result<Vec<ConversionReport>>
where
    P: ProgressSink,
{
    if settings.max_workers > 1 && jobs.len() > 1 {
        return convert_many_parallel(jobs, settings, progress);
    }
    convert_many_serial(jobs, &settings, &progress)
}

fn convert_many_serial<P>(
    jobs: Vec<ConvertJob>,
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<Vec<ConversionReport>>
where
    P: ProgressSink,
{
    let mut reports = Vec::with_capacity(jobs.len());
    for (index, job) in jobs.into_iter().enumerate() {
        if progress.is_cancelled() {
            anyhow::bail!("conversion cancelled");
        }
        progress.job_started(index, &job.input);
        let report = convert_one(job, settings, progress)?;
        progress.job_finished(index, &report.output);
        reports.push(report);
    }
    Ok(reports)
}

fn convert_many_parallel<P>(
    jobs: Vec<ConvertJob>,
    settings: ConversionSettings,
    progress: P,
) -> anyhow::Result<Vec<ConversionReport>>
where
    P: ProgressSink,
{
    use rayon::prelude::*;

    let worker_count = settings.max_workers.max(1);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(worker_count)
        .build()?;
    let progress = &progress;
    let settings = &settings;
    let mut indexed = pool.install(|| {
        jobs.into_par_iter()
            .enumerate()
            .map(|(index, job)| {
                if progress.is_cancelled() {
                    anyhow::bail!("conversion cancelled");
                }
                progress.job_started(index, &job.input);
                let result = convert_one(job, settings, progress);
                if let Ok(report) = &result {
                    progress.job_finished(index, &report.output);
                }
                result.map(|report| (index, report))
            })
            .collect::<Vec<_>>()
    });

    let mut reports = Vec::with_capacity(indexed.len());
    for item in indexed.drain(..) {
        reports.push(item?);
    }
    reports.sort_by_key(|(index, _)| *index);
    Ok(reports.into_iter().map(|(_, report)| report).collect())
}

#[cfg(feature = "core-bioformats")]
fn convert_one<P>(
    job: ConvertJob,
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<ConversionReport>
where
    P: ProgressSink,
{
    use crate::convert::reader::BioformatsImageSource;
    use crate::convert::upload::{final_output_path, materialize_output_target};
    use crate::convert::writer::write_omezarr;

    let local_output = materialize_output_target(&job.output)?;
    let mut source = BioformatsImageSource::open(&job.input)?;
    let series_written = write_omezarr(&mut source, &local_output, settings, progress)?;
    upload::finish_output_target(&local_output, &job.output, settings.overwrite)?;
    Ok(ConversionReport {
        input: job.input,
        output: final_output_path(&job.output),
        series_written,
    })
}

#[cfg(not(feature = "core-bioformats"))]
fn convert_one<P>(
    _job: ConvertJob,
    _settings: &ConversionSettings,
    _progress: &P,
) -> anyhow::Result<ConversionReport>
where
    P: ProgressSink,
{
    anyhow::bail!("conversion requires the `core-bioformats` feature")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::convert::config::OutputTarget;

    struct CancelledProgress(AtomicBool);

    impl ProgressSink for CancelledProgress {
        fn is_cancelled(&self) -> bool {
            self.0.load(Ordering::SeqCst)
        }
    }

    #[test]
    fn conversion_stops_before_starting_job_when_cancelled() {
        let err = convert_many(
            vec![ConvertJob {
                input: PathBuf::from("input.fake"),
                output: OutputTarget::Local(PathBuf::from("output.ome.zarr")),
            }],
            ConversionSettings::default(),
            CancelledProgress(AtomicBool::new(true)),
        )
        .expect_err("cancelled conversion should fail before opening input");

        assert!(err.to_string().contains("conversion cancelled"));
    }
}

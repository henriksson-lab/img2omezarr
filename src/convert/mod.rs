pub mod axes;
pub mod config;
pub mod downsample;
pub mod dtype;
pub mod metadata;
pub mod planner;
pub mod positions;
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
    // A batch's tile positions are placed and checked together, before any
    // output is written. A single job places its own series as it opens.
    let placements = if jobs.len() > 1 && !settings.positions.disabled {
        place_jobs(&jobs, &settings, &progress)?
    } else {
        vec![None; jobs.len()]
    };
    let jobs: Vec<PlacedJob> = jobs.into_iter().zip(placements).collect();
    if settings.max_workers > 1 && jobs.len() > 1 {
        return convert_many_parallel(jobs, settings, progress);
    }
    convert_many_serial(jobs, &settings, &progress)
}

type PlacedJob = (ConvertJob, Option<Vec<positions::Placement>>);

/// Every job's tiles, placed as one batch: per job, its series' placements.
#[cfg(feature = "core-bioformats")]
fn place_jobs<P>(
    jobs: &[ConvertJob],
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<Vec<Option<Vec<positions::Placement>>>>
where
    P: ProgressSink,
{
    use rayon::prelude::*;

    let per_job = jobs
        .par_iter()
        .map(|job| -> anyhow::Result<Vec<positions::Tile>> {
            let mut source = crate::convert::reader::BioformatsImageSource::open(&job.input)?;
            let selected = planner::selected_series(&settings.series, source.series_count())?;
            positions::ome::tiles(&mut source, &job.input, &selected)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let tiles: Vec<positions::Tile> = per_job.iter().flatten().cloned().collect();
    let mut placements = positions::place_batch(&tiles, &settings.positions, &|message| {
        progress.message(message)
    })?
    .into_iter();
    Ok(per_job
        .iter()
        .map(|job_tiles| Some(placements.by_ref().take(job_tiles.len()).collect()))
        .collect())
}

#[cfg(not(feature = "core-bioformats"))]
fn place_jobs<P>(
    jobs: &[ConvertJob],
    _settings: &ConversionSettings,
    _progress: &P,
) -> anyhow::Result<Vec<Option<Vec<positions::Placement>>>>
where
    P: ProgressSink,
{
    Ok(vec![None; jobs.len()])
}

fn convert_many_serial<P>(
    jobs: Vec<PlacedJob>,
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<Vec<ConversionReport>>
where
    P: ProgressSink,
{
    let mut reports = Vec::with_capacity(jobs.len());
    for (index, (job, placements)) in jobs.into_iter().enumerate() {
        if progress.is_cancelled() {
            anyhow::bail!("conversion cancelled");
        }
        progress.job_started(index, &job.input);
        let report = convert_one(job, placements, settings, progress)?;
        progress.job_finished(index, &report.output);
        reports.push(report);
    }
    Ok(reports)
}

fn convert_many_parallel<P>(
    jobs: Vec<PlacedJob>,
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
            .map(|(index, (job, placements))| {
                if progress.is_cancelled() {
                    anyhow::bail!("conversion cancelled");
                }
                progress.job_started(index, &job.input);
                let result = convert_one(job, placements, settings, progress);
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
    placements: Option<Vec<positions::Placement>>,
    settings: &ConversionSettings,
    progress: &P,
) -> anyhow::Result<ConversionReport>
where
    P: ProgressSink,
{
    use crate::convert::reader::BioformatsImageSource;
    use crate::convert::upload::{final_output_path, materialize_output_target};
    use crate::convert::writer::write_omezarr_placed;

    let mut source = BioformatsImageSource::open(&job.input)?;
    let placements = match placements {
        Some(placements) => placements,
        None => {
            let selected = planner::selected_series(&settings.series, source.series_count())?;
            let tiles = positions::ome::tiles(&mut source, &job.input, &selected)?;
            positions::place_batch(&tiles, &settings.positions, &|message| {
                progress.message(message)
            })?
        }
    };
    let local_output = materialize_output_target(&job.output)?;
    let series_written = write_omezarr_placed(
        &mut source,
        &local_output,
        settings,
        Some(&placements),
        progress,
    )?;
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
    _placements: Option<Vec<positions::Placement>>,
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

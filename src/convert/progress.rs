use std::path::Path;

pub trait ProgressSink: Send + Sync {
    fn job_started(&self, _index: usize, _input: &Path) {}
    fn job_finished(&self, _index: usize, _output: &Path) {}
    fn series_started(&self, _series: usize, _levels: usize, _chunks: usize) {}
    fn chunk_finished(&self, _series: usize, _level: usize, _done: usize, _total: usize) {}
    fn message(&self, _message: &str) {}
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoProgress;

impl ProgressSink for NoProgress {}

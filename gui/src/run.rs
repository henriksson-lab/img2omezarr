use std::path::Path;

use img2omezarr::convert::config::{ConversionSettings, ConvertJob};
use img2omezarr::convert::progress::ProgressSink;
use slint::{Model, SharedString};

use crate::{AppWindow, QueueFile};

#[derive(Clone)]
struct GuiProgress {
    window: slint::Weak<AppWindow>,
}

impl ProgressSink for GuiProgress {
    fn job_started(&self, index: usize, input: &Path) {
        let input = input.display().to_string();
        let window = self.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                set_row_status(&window, index, "running");
                window.set_progress_label(SharedString::from(format!("converting {input}")));
                append_log(&window, &format!("converting {input}"));
            }
        });
    }

    fn chunk_finished(&self, series: usize, level: usize, done: usize, total: usize) {
        let window = self.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                if total > 0 {
                    window.set_progress(done as f32 / total as f32);
                }
                window.set_progress_label(SharedString::from(format!(
                    "series {series} level {level}: {done}/{total}"
                )));
            }
        });
    }

    fn job_finished(&self, index: usize, output: &Path) {
        let output = output.display().to_string();
        let window = self.window.clone();
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(window) = window.upgrade() {
                set_row_status(&window, index, "done");
                append_log(&window, &format!("wrote {output}"));
            }
        });
    }
}

pub fn start_conversion(
    window: slint::Weak<AppWindow>,
    jobs: Vec<ConvertJob>,
    settings: ConversionSettings,
) {
    std::thread::Builder::new()
        .name("img2omezarr-convert".into())
        .spawn(move || {
            let result = img2omezarr::convert::convert_many(
                jobs,
                settings,
                GuiProgress {
                    window: window.clone(),
                },
            );
            let _ = slint::invoke_from_event_loop(move || {
                if let Some(window) = window.upgrade() {
                    match result {
                        Ok(_) => {
                            window.set_progress(1.0);
                            window.set_progress_label(SharedString::from("done"));
                            append_log(&window, "done");
                        }
                        Err(err) => {
                            mark_unfinished_failed(&window);
                            window.set_progress(-1.0);
                            window.set_progress_label(SharedString::from("failed"));
                            append_log(&window, &format!("error: {err:#}"));
                        }
                    }
                    window.set_running(false);
                }
            });
        })
        .expect("start conversion thread");
}

fn set_row_status(window: &AppWindow, index: usize, status: &str) {
    let files = window.get_files();
    let Some(mut file) = files.row_data(index) else {
        return;
    };
    file.status = SharedString::from(status);
    files.set_row_data(index, file);
}

fn mark_unfinished_failed(window: &AppWindow) {
    let files = window.get_files();
    for index in 0..files.row_count() {
        let Some(QueueFile { path, status }) = files.row_data(index) else {
            continue;
        };
        if status == SharedString::from("queued") || status == SharedString::from("running") {
            files.set_row_data(
                index,
                QueueFile {
                    path,
                    status: SharedString::from("failed"),
                },
            );
        }
    }
}

fn append_log(window: &AppWindow, line: &str) {
    let current = window.get_log_text();
    let next = if current.is_empty() {
        line.to_string()
    } else {
        format!("{current}\n{line}")
    };
    window.set_log_text(SharedString::from(next));
}

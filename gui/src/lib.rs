use std::io;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::JoinHandle;

use std::fs::{self, File};
use std::io::Read;

use collector_core::{Meta, Options};
#[cfg(windows)]
use app::pipeline::{PipelineConfig, SessionPipeline};
#[cfg(windows)]
use capture::WgcCapture;
#[cfg(windows)]
use input::RawInputCollector;

pub struct GuiSessionConfig {
    pub dataset_root: PathBuf,
    pub session_name: String,
    pub ffmpeg_path: PathBuf,
    pub target_hwnd: isize,
    pub options: Options,
    pub meta: Meta,
    pub cursor_debug: bool,
}

pub struct GuiSessionRunner;

#[derive(Debug, Clone)]
pub enum GuiStatus {
    Started { session_name: String },
    Frame { step_index: u64, qpc_ts: u64, is_foreground: bool },
    Finished { output_dir: PathBuf },
    Error { message: String },
}

pub struct GuiSessionHandle {
    pub rx: mpsc::Receiver<GuiStatus>,
    join: JoinHandle<io::Result<PathBuf>>,
}

impl GuiSessionHandle {
    pub fn join(self) -> io::Result<PathBuf> {
        match self.join.join() {
            Ok(result) => result,
            Err(_) => Err(io::Error::new(
                io::ErrorKind::Other,
                "gui session thread panicked",
            )),
        }
    }
}

impl GuiSessionRunner {
    pub fn start_realtime_blocking(config: GuiSessionConfig) -> io::Result<PathBuf> {
        #[cfg(not(windows))]
        {
            let _ = config;
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "GUI capture requires Windows",
            ));
        }
        #[cfg(windows)]
        {
            let pipeline = SessionPipeline::create(PipelineConfig {
                dataset_root: config.dataset_root.clone(),
                session_name: config.session_name.clone(),
                ffmpeg_path: config.ffmpeg_path.clone(),
            })?;
            pipeline.write_options_meta(&config.options, &config.meta)?;

            let capture = WgcCapture::new(config.options.capture.clone(), config.target_hwnd)?;
            let input = RawInputCollector::new_with_target(Some(config.target_hwnd))?;

            let layout = app::pipeline::run_realtime_with_hwnd(
                capture,
                input,
                config.target_hwnd,
                config.cursor_debug,
                pipeline,
            )?;
            Ok(layout.root_dir)
        }
    }

    pub fn start_realtime_async(config: GuiSessionConfig) -> io::Result<GuiSessionHandle> {
        #[cfg(not(windows))]
        {
            let _ = config;
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "GUI capture requires Windows",
            ));
        }
        #[cfg(windows)]
        {
            let (tx, rx) = mpsc::channel();
            let handle = std::thread::spawn(move || {
                let pipeline = SessionPipeline::create(PipelineConfig {
                    dataset_root: config.dataset_root.clone(),
                    session_name: config.session_name.clone(),
                    ffmpeg_path: config.ffmpeg_path.clone(),
                })?;
                pipeline.write_options_meta(&config.options, &config.meta)?;
                let _ = tx.send(GuiStatus::Started {
                    session_name: config.session_name.clone(),
                });

                let capture = WgcCapture::new(config.options.capture.clone(), config.target_hwnd)?;
                let input = RawInputCollector::new_with_target(Some(config.target_hwnd))?;
                let tx_frame = tx.clone();

                let result = app::pipeline::run_realtime_with_hwnd_and_hook(
                    capture,
                    input,
                    config.target_hwnd,
                    config.cursor_debug,
                    pipeline,
                    |frame, is_foreground, _cursor| {
                        let _ = tx_frame.send(GuiStatus::Frame {
                            step_index: frame.step_index,
                            qpc_ts: frame.qpc_ts,
                            is_foreground,
                        });
                    },
                );

                match result {
                    Ok(layout) => {
                        let _ = tx.send(GuiStatus::Finished {
                            output_dir: layout.root_dir.clone(),
                        });
                        Ok(layout.root_dir)
                    }
                    Err(err) => {
                        let _ = tx.send(GuiStatus::Error {
                            message: err.to_string(),
                        });
                        Err(err)
                    }
                }
            });
            Ok(GuiSessionHandle { rx, join: handle })
        }
    }
}

pub struct PackageRequest {
    pub dataset_root: PathBuf,
    pub session_names: Vec<String>,
    pub output_zip: PathBuf,
    pub delete_after: bool,
}

pub fn package_sessions(request: PackageRequest) -> io::Result<PathBuf> {
    let sessions_dir = request.dataset_root.join("sessions");
    let targets = if request.session_names.is_empty() {
        list_session_dirs(&sessions_dir)?
    } else {
        request
            .session_names
            .iter()
            .map(|name| sessions_dir.join(name))
            .collect()
    };

    if targets.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no sessions found to package",
        ));
    }

    let file = File::create(&request.output_zip)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default();

    for target in &targets {
        if is_tmp_dir(target) {
            continue;
        }
        add_dir_to_zip(&mut zip, &request.dataset_root, target, options)?;
    }

    zip.finish()
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;

    if request.delete_after {
        for target in &targets {
            if target.exists() {
                fs::remove_dir_all(target)?;
            }
        }
    }

    Ok(request.output_zip)
}

fn list_session_dirs(root: &PathBuf) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !root.exists() {
        return Ok(out);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() && !is_tmp_dir(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

fn is_tmp_dir(path: &PathBuf) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.ends_with(".tmp"))
        .unwrap_or(false)
}

fn add_dir_to_zip(
    zip: &mut zip::ZipWriter<File>,
    base: &PathBuf,
    path: &PathBuf,
    options: zip::write::FileOptions,
) -> io::Result<()> {
    let mut stack = vec![path.clone()];
    while let Some(current) = stack.pop() {
        if is_tmp_dir(&current) {
            continue;
        }
        let rel = current.strip_prefix(base).map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "failed to compute relative path")
        })?;
        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if current.is_dir() {
            if !rel_str.is_empty() {
                let dir_name = format!("{}/", rel_str);
                zip.add_directory(dir_name, options)
                    .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
            }
            for entry in fs::read_dir(&current)? {
                let entry = entry?;
                stack.push(entry.path());
            }
        } else {
            zip.start_file(rel_str, options)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
            let mut file = File::open(&current)?;
            let mut buffer = Vec::new();
            file.read_to_end(&mut buffer)?;
            zip.write_all(&buffer)?;
        }
    }
    Ok(())
}

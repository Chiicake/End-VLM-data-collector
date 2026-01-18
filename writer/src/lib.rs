use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::time::{Duration, Instant};

use aggregator::AggregatedWindow;
use collector_core::ActionSnapshot;
use serde::Serialize;

pub struct SessionLayout {
    pub root_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub video_path: PathBuf,
    pub actions_path: PathBuf,
    pub compiled_path: PathBuf,
    pub thoughts_path: PathBuf,
    pub auto_events_path: PathBuf,
    pub options_path: PathBuf,
    pub meta_path: PathBuf,
}

impl SessionLayout {
    pub fn new(dataset_root: &Path, session_name: &str) -> Self {
        let sessions_dir = dataset_root.join("sessions");
        let root_dir = sessions_dir.join(session_name);
        let temp_dir = sessions_dir.join(format!("{}.tmp", session_name));
        Self {
            video_path: temp_dir.join("video.mp4"),
            actions_path: temp_dir.join("actions.jsonl"),
            compiled_path: temp_dir.join("compiled_actions.jsonl"),
            thoughts_path: temp_dir.join("thoughts.jsonl"),
            auto_events_path: temp_dir.join("auto_events.jsonl"),
            options_path: temp_dir.join("options.json"),
            meta_path: temp_dir.join("meta.json"),
            root_dir,
            temp_dir,
        }
    }
}

pub struct FfmpegConfig {
    pub ffmpeg_path: PathBuf,
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub crf: u32,
    pub gop: u32,
}

pub struct FfmpegWriter {
    child: Child,
    stdin: ChildStdin,
    frame_bytes: usize,
}

impl FfmpegWriter {
    pub fn spawn(config: &FfmpegConfig) -> io::Result<Self> {
        let mut cmd = Command::new(&config.ffmpeg_path);
        cmd.arg("-y")
            .arg("-f")
            .arg("rawvideo")
            .arg("-pix_fmt")
            .arg("bgra")
            .arg("-s")
            .arg(format!("{}x{}", config.width, config.height))
            .arg("-r")
            .arg(config.fps.to_string())
            .arg("-i")
            .arg("-")
            .arg("-c:v")
            .arg("libx264")
            .arg("-crf")
            .arg(config.crf.to_string())
            .arg("-g")
            .arg(config.gop.to_string())
            .arg("-pix_fmt")
            .arg("yuv420p")
            .arg(&config.output_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "ffmpeg stdin unavailable")
        })?;
        let frame_bytes = (config.width as usize)
            .saturating_mul(config.height as usize)
            .saturating_mul(4);
        Ok(Self {
            child,
            stdin,
            frame_bytes,
        })
    }

    pub fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        if frame.len() != self.frame_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "frame buffer size does not match expected BGRA size",
            ));
        }
        self.stdin.write_all(frame)
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.stdin.flush()?;
        drop(self.stdin);
        let status = self.child.wait()?;
        if !status.success() {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("ffmpeg exited with {}", status),
            ));
        }
        Ok(())
    }
}

pub fn default_ffmpeg_config(ffmpeg_path: &Path, output_path: &Path) -> FfmpegConfig {
    FfmpegConfig {
        ffmpeg_path: ffmpeg_path.to_path_buf(),
        output_path: output_path.to_path_buf(),
        width: 1280,
        height: 720,
        fps: 5,
        crf: 20,
        gop: 10,
    }
}

pub struct SessionWriter {
    layout: SessionLayout,
    ffmpeg: FfmpegWriter,
    actions: JsonlWriter<BufWriter<File>>,
    compiled: JsonlWriter<BufWriter<File>>,
    thoughts: JsonlWriter<BufWriter<File>>,
    auto_events: JsonlWriter<BufWriter<File>>,
}

impl SessionWriter {
    pub fn create(
        dataset_root: &Path,
        session_name: &str,
        ffmpeg_path: &Path,
        flush_every_lines: u64,
        flush_every: Duration,
    ) -> io::Result<Self> {
        let layout = SessionLayout::new(dataset_root, session_name);
        if layout.temp_dir.exists() || layout.root_dir.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "session directory already exists",
            ));
        }
        fs::create_dir_all(&layout.temp_dir)?;

        let actions = JsonlWriter::new(
            BufWriter::new(File::create(&layout.actions_path)?),
            flush_every_lines,
            flush_every,
        );
        let compiled = JsonlWriter::new(
            BufWriter::new(File::create(&layout.compiled_path)?),
            flush_every_lines,
            flush_every,
        );
        let thoughts = JsonlWriter::new(
            BufWriter::new(File::create(&layout.thoughts_path)?),
            flush_every_lines,
            flush_every,
        );
        let auto_events = JsonlWriter::new(
            BufWriter::new(File::create(&layout.auto_events_path)?),
            flush_every_lines,
            flush_every,
        );

        let ffmpeg_config = default_ffmpeg_config(ffmpeg_path, &layout.video_path);
        let ffmpeg = FfmpegWriter::spawn(&ffmpeg_config)?;

        Ok(Self {
            layout,
            ffmpeg,
            actions,
            compiled,
            thoughts,
            auto_events,
        })
    }

    pub fn layout(&self) -> &SessionLayout {
        &self.layout
    }

    pub fn write_window(&mut self, window: &AggregatedWindow) -> io::Result<()> {
        self.actions.write_json(&window.snapshot)?;
        self.compiled.write_line(&window.compiled_action)?;
        Ok(())
    }

    pub fn write_thought(&mut self, thought_line: &str) -> io::Result<()> {
        self.thoughts.write_line(thought_line)
    }

    pub fn write_auto_event<T: Serialize>(&mut self, event: &T) -> io::Result<()> {
        self.auto_events.write_json(event)
    }

    pub fn write_options<T: Serialize>(&self, options: &T) -> io::Result<()> {
        write_json_file(&self.layout.options_path, options)
    }

    pub fn write_meta<T: Serialize>(&self, meta: &T) -> io::Result<()> {
        write_json_file(&self.layout.meta_path, meta)
    }

    pub fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        self.ffmpeg.write_frame(frame)
    }

    pub fn finalize(self) -> io::Result<SessionLayout> {
        let SessionWriter {
            layout,
            ffmpeg,
            mut actions,
            mut compiled,
            mut thoughts,
            mut auto_events,
        } = self;

        actions.flush()?;
        compiled.flush()?;
        thoughts.flush()?;
        auto_events.flush()?;
        ffmpeg.finish()?;

        if layout.root_dir.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "final session directory already exists",
            ));
        }
        fs::rename(&layout.temp_dir, &layout.root_dir)?;
        Ok(layout)
    }
}

fn write_json_file<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let file = File::create(path)?;
    let writer = BufWriter::new(file);
    serde_json::to_writer(writer, value)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

pub struct JsonlWriter<W: Write> {
    writer: W,
    line_count: u64,
    last_flush: Instant,
    flush_every_lines: u64,
    flush_every: Duration,
}

impl<W: Write> JsonlWriter<W> {
    pub fn new(writer: W, flush_every_lines: u64, flush_every: Duration) -> Self {
        Self {
            writer,
            line_count: 0,
            last_flush: Instant::now(),
            flush_every_lines: flush_every_lines.max(1),
            flush_every,
        }
    }

    pub fn write_json<T: Serialize>(&mut self, value: &T) -> io::Result<()> {
        serde_json::to_writer(&mut self.writer, value)
            .map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
        self.writer.write_all(b"\n")?;
        self.after_write()
    }

    pub fn write_line(&mut self, line: &str) -> io::Result<()> {
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.after_write()
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.last_flush = Instant::now();
        self.writer.flush()
    }

    pub fn into_inner(self) -> W {
        self.writer
    }

    fn after_write(&mut self) -> io::Result<()> {
        self.line_count = self.line_count.saturating_add(1);
        if self.line_count % self.flush_every_lines == 0
            || self.last_flush.elapsed() >= self.flush_every
        {
            self.flush()?;
        }
        Ok(())
    }
}

pub struct SessionWriters<A: Write, C: Write> {
    pub actions: JsonlWriter<A>,
    pub compiled: JsonlWriter<C>,
}

impl<A: Write, C: Write> SessionWriters<A, C> {
    pub fn new(actions: A, compiled: C, flush_every_lines: u64, flush_every: Duration) -> Self {
        Self {
            actions: JsonlWriter::new(actions, flush_every_lines, flush_every),
            compiled: JsonlWriter::new(compiled, flush_every_lines, flush_every),
        }
    }

    pub fn write_window(&mut self, window: &AggregatedWindow) -> io::Result<()> {
        self.actions.write_json(&window.snapshot)?;
        self.compiled.write_line(&window.compiled_action)?;
        Ok(())
    }
}

pub fn write_snapshot<W: Write>(
    writer: &mut JsonlWriter<W>,
    snapshot: &ActionSnapshot,
) -> io::Result<()> {
    writer.write_json(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aggregator::{aggregate_window_with_compiled, AggregatorState, CursorProvider};
    use collector_core::{InputEvent, InputEventKind};

    #[test]
    fn writes_action_and_compiled_lines() {
        let events = vec![InputEvent {
            qpc_ts: 10,
            kind: InputEventKind::KeyDown {
                key: "W".to_string(),
            },
        }];
        let cursor = CursorProvider {
            visible: false,
            x_norm: 0.0,
            y_norm: 0.0,
        };
        let mut state = AggregatorState::new();
        let window = aggregate_window_with_compiled(&events, 0, 200, 0, true, &cursor, &mut state);

        let mut writers = SessionWriters::new(Vec::new(), Vec::new(), 10, Duration::from_secs(1));

        writers.write_window(&window).unwrap();
        let SessionWriters { actions, compiled } = writers;
        let actions_out = actions.into_inner();
        let compiled_out = compiled.into_inner();

        assert!(std::str::from_utf8(&actions_out)
            .unwrap()
            .contains("\"step_index\""));
        assert!(std::str::from_utf8(&compiled_out)
            .unwrap()
            .contains("<|action_start|>"));
    }
}

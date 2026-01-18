use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aggregator::{aggregate_window_with_compiled, AggregatorState, CursorProvider};
use capture::FrameSource;
use collector_core::{InputEvent, Meta, Options, QpcTimestamp, StepIndex, STEP_MS};
use input::InputCollector;
use writer::{SessionLayout, SessionWriter};

#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::GetCursorInfo;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{GetCursorPos, CURSORINFO, CURSOR_SHOWING};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, GetForegroundWindow, ScreenToClient};

const DEFAULT_FLUSH_LINES: u64 = 10;
const DEFAULT_FLUSH_SECS: u64 = 1;

pub struct PipelineConfig {
    pub dataset_root: PathBuf,
    pub session_name: String,
    pub ffmpeg_path: PathBuf,
}

pub struct SessionPipeline {
    writer: SessionWriter,
    state: AggregatorState,
}

impl SessionPipeline {
    pub fn create(config: PipelineConfig) -> io::Result<Self> {
        let writer = SessionWriter::create(
            &config.dataset_root,
            &config.session_name,
            &config.ffmpeg_path,
            DEFAULT_FLUSH_LINES,
            Duration::from_secs(DEFAULT_FLUSH_SECS),
        )?;
        Ok(Self {
            writer,
            state: AggregatorState::new(),
        })
    }

    pub fn write_options_meta(&self, options: &Options, meta: &Meta) -> io::Result<()> {
        self.writer.write_options(options)?;
        self.writer.write_meta(meta)?;
        Ok(())
    }

    pub fn process_window(
        &mut self,
        events: &[InputEvent],
        window_start: QpcTimestamp,
        window_end: QpcTimestamp,
        step_index: StepIndex,
        is_foreground: bool,
        cursor: &CursorProvider,
        frame: &[u8],
        thought_content: Option<&str>,
    ) -> io::Result<()> {
        let aggregated = aggregate_window_with_compiled(
            events,
            window_start,
            window_end,
            step_index,
            is_foreground,
            cursor,
            &mut self.state,
        );

        self.writer.write_window(&aggregated)?;
        self.writer.write_frame(frame)?;
        let thought_line = format_thought_line(thought_content.unwrap_or_default());
        self.writer.write_thought(&thought_line)?;
        Ok(())
    }

    pub fn finalize(self) -> io::Result<SessionLayout> {
        self.writer.finalize()
    }
}

#[allow(dead_code)]
pub fn run_realtime<S: FrameSource, I: InputCollector>(
    mut capture: S,
    mut input: I,
    cursor: &CursorProvider,
    mut pipeline: SessionPipeline,
) -> io::Result<SessionLayout> {
    loop {
        let frame = match capture.next_frame() {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        let window_end = frame.qpc_ts;
        let window_start = window_end.saturating_sub(STEP_MS);
        let events = input.drain_events(window_start, window_end)?;
        let is_foreground = true;
        let cursor_sample = cursor.clone();

        pipeline.process_window(
            &events,
            window_start,
            window_end,
            frame.step_index,
            is_foreground,
            &cursor_sample,
            &frame.data,
            None,
        )?;
    }

    pipeline.finalize()
}

#[cfg(windows)]
pub fn run_realtime_with_hwnd<S: FrameSource, I: InputCollector>(
    mut capture: S,
    mut input: I,
    target_hwnd: isize,
    mut pipeline: SessionPipeline,
) -> io::Result<SessionLayout> {
    loop {
        let frame = match capture.next_frame() {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        let window_end = frame.qpc_ts;
        let window_start = window_end.saturating_sub(STEP_MS);
        let events = input.drain_events(window_start, window_end)?;
        let (is_foreground, cursor) = sample_foreground_and_cursor(target_hwnd)?;

        pipeline.process_window(
            &events,
            window_start,
            window_end,
            frame.step_index,
            is_foreground,
            &cursor,
            &frame.data,
            None,
        )?;
    }

    pipeline.finalize()
}

#[cfg(windows)]
fn sample_foreground_and_cursor(target_hwnd: isize) -> io::Result<(bool, CursorProvider)> {
    unsafe {
        let target = HWND(target_hwnd);
        let fg = GetForegroundWindow();
        let is_foreground = fg == target;

        let mut ci = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        let mut visible = false;
        let mut x_norm = 0.0f32;
        let mut y_norm = 0.0f32;
        if GetCursorInfo(&mut ci).as_bool() {
            visible = (ci.flags & CURSOR_SHOWING) != 0;
        }

        let mut point = windows::Win32::Foundation::POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point).as_bool() {
            let mut client_point = point;
            if ScreenToClient(target, &mut client_point).as_bool() {
                let mut rect = windows::Win32::Foundation::RECT::default();
                if GetClientRect(target, &mut rect).as_bool() {
                    let width = (rect.right - rect.left) as f32;
                    let height = (rect.bottom - rect.top) as f32;
                    if width > 0.0 && height > 0.0 {
                        x_norm = (client_point.x as f32 / width).clamp(0.0, 1.0);
                        y_norm = (client_point.y as f32 / height).clamp(0.0, 1.0);
                    }
                }
            }
        }

        Ok((
            is_foreground,
            CursorProvider {
                visible,
                x_norm,
                y_norm,
            },
        ))
    }
}

#[allow(dead_code)]
pub fn default_session_name(now: &str, run_id: u32) -> String {
    format!("{}_run{:03}", now, run_id)
}

pub fn format_thought_line(content: &str) -> String {
    if content.is_empty() {
        "<|thought_start|><|thought_end|>".to_string()
    } else if content.contains("<|thought_start|>") && content.contains("<|thought_end|>") {
        content.to_string()
    } else {
        format!("<|thought_start|>{} <|thought_end|>", content)
    }
}

pub fn ensure_dataset_root(path: &Path) -> io::Result<()> {
    if !path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "dataset root does not exist",
        ));
    }
    Ok(())
}

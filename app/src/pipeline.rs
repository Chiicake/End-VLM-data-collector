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
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetForegroundWindow, ScreenToClient,
};

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
    debug_cursor: bool,
    mut pipeline: SessionPipeline,
) -> io::Result<SessionLayout> {
    set_per_monitor_dpi_awareness();
    loop {
        let frame = match capture.next_frame() {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        let window_end = frame.qpc_ts;
        let window_start = window_end.saturating_sub(STEP_MS);
        let events = input.drain_events(window_start, window_end)?;
        let (is_foreground, cursor, debug_info) =
            sample_foreground_and_cursor(
                target_hwnd,
                frame.src_width,
                frame.src_height,
                frame.width,
                frame.height,
            )?;
        if debug_cursor {
            if let Some(info) = debug_info {
                eprintln!(
                    "[cursor] step={} fg={} vis={} client=({}, {}) src={}x{} record={}x{} record_xy=({}, {}) norm=({:.4}, {:.4})",
                    frame.step_index,
                    is_foreground,
                    cursor.visible,
                    info.client_x,
                    info.client_y,
                    info.src_w,
                    info.src_h,
                    info.record_w,
                    info.record_h,
                    info.record_x,
                    info.record_y,
                    cursor.x_norm,
                    cursor.y_norm
                );
            }
        }

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
fn sample_foreground_and_cursor(
    target_hwnd: isize,
    src_width: u32,
    src_height: u32,
    record_width: u32,
    record_height: u32,
) -> io::Result<(bool, CursorProvider, Option<CursorDebug>)> {
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
        let mut debug_info = None;
        if GetCursorInfo(&mut ci).as_bool() {
            visible = (ci.flags & CURSOR_SHOWING) != 0;
        }

        let mut point = windows::Win32::Foundation::POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point).as_bool()
            && record_width > 0
            && record_height > 0
            && src_width > 0
            && src_height > 0
        {
            let mut client_point = point;
            if ScreenToClient(target, &mut client_point).as_bool() {
                let mut rect = windows::Win32::Foundation::RECT::default();
                if GetClientRect(target, &mut rect).as_bool() {
                    let client_w = (rect.right - rect.left).max(0) as f32;
                    let client_h = (rect.bottom - rect.top).max(0) as f32;
                    if client_w > 0.0 && client_h > 0.0 {
                        let dpi = GetDpiForWindow(target) as f32;
                        let src_w = src_width as f32;
                        let src_h = src_height as f32;
                        let dst_w = record_width as f32;
                        let dst_h = record_height as f32;
                        let scale_x = src_w / client_w;
                        let scale_y = src_h / client_h;
                        let dpi_scale = (dpi / 96.0).max(0.0001);
                        let src_x = (client_point.x as f32 / dpi_scale) * scale_x;
                        let src_y = (client_point.y as f32 / dpi_scale) * scale_y;

                        let scale = (dst_w / src_w).min(dst_h / src_h);
                        let scaled_w = src_w * scale;
                        let scaled_h = src_h * scale;
                        let pad_x = (dst_w - scaled_w) * 0.5;
                        let pad_y = (dst_h - scaled_h) * 0.5;
                        let record_x = (src_x * scale) + pad_x;
                        let record_y = (src_y * scale) + pad_y;
                        x_norm = (record_x / dst_w).clamp(0.0, 1.0);
                        y_norm = (record_y / dst_h).clamp(0.0, 1.0);
                        debug_info = Some(CursorDebug {
                            client_x: client_point.x,
                            client_y: client_point.y,
                            src_w: src_width,
                            src_h: src_height,
                            record_w: record_width,
                            record_h: record_height,
                            record_x: record_x.round() as i32,
                            record_y: record_y.round() as i32,
                        });
                    }
                }
            }
        }

        Ok((
            is_foreground,
            CursorProvider { visible, x_norm, y_norm },
            debug_info,
        ))
    }
}

#[cfg(windows)]
fn set_per_monitor_dpi_awareness() {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

#[cfg(windows)]
struct CursorDebug {
    client_x: i32,
    client_y: i32,
    src_w: u32,
    src_h: u32,
    record_w: u32,
    record_h: u32,
    record_x: i32,
    record_y: i32,
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

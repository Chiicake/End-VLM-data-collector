use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use aggregator::{aggregate_window_with_compiled, AggregatorState, CursorProvider};
use capture::FrameSource;
use collector_core::{InputEvent, Meta, Options, QpcTimestamp, StepIndex};

#[cfg(windows)]
use collector_core::FrameRecord;

#[cfg(windows)]
use collector_core::InputEventKind;
use input::InputCollector;
use writer::{SessionLayout, SessionWriter};

#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::System::Performance::QueryPerformanceFrequency;
#[cfg(windows)]
#[cfg(windows)]
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::ScreenToClient;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    GetClientRect, GetCursorInfo, GetCursorPos, GetForegroundWindow, CURSORINFO, CURSOR_SHOWING,
};

const DEFAULT_FLUSH_LINES: u64 = 10;
const DEFAULT_FLUSH_SECS: u64 = 1;
const THOUGHT_TEMPLATE: &str =
    "<|labeling_instruct_start|>Labeling Instruct <|labeling_instruct_end|>";

pub struct PipelineConfig {
    pub dataset_root: PathBuf,
    pub session_name: String,
    pub ffmpeg_path: PathBuf,
    pub record_width: u32,
    pub record_height: u32,
    pub fps: u32,
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
            config.record_width,
            config.record_height,
            config.fps,
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
    step_ms: u64,
) -> io::Result<SessionLayout> {
    let step_ticks = qpc_step_ticks(step_ms)?;
    loop {
        let frame = match capture.next_frame() {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        let window_end = frame.qpc_ts;
        let window_start = window_end.saturating_sub(step_ticks);
        let events = input.drain_events(window_start, window_end)?;
        if events.is_empty() {
            eprintln!(
                "[input] step={} events=0 window=({}-{})",
                frame.step_index, window_start, window_end
            );
        } else {
            eprintln!(
                "[input] step={} events={}",
                frame.step_index,
                events.len()
            );
        }
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
    capture: S,
    input: I,
    target_hwnd: isize,
    debug_cursor: bool,
    pipeline: SessionPipeline,
    step_ms: u64,
) -> io::Result<SessionLayout> {
    run_realtime_with_hwnd_and_hook(
        capture,
        input,
        target_hwnd,
        debug_cursor,
        pipeline,
        |_frame, _is_foreground, _cursor| {},
        step_ms,
    )
}

#[cfg(windows)]
pub fn run_realtime_with_hwnd_and_hook<
    S: FrameSource,
    I: InputCollector,
    F: FnMut(&FrameRecord, bool, &CursorProvider),
>(
    capture: S,
    input: I,
    target_hwnd: isize,
    debug_cursor: bool,
    pipeline: SessionPipeline,
    mut on_frame: F,
    step_ms: u64,
) -> io::Result<SessionLayout> {
    run_realtime_with_hwnd_and_hook_and_thought(
        capture,
        input,
        target_hwnd,
        debug_cursor,
        pipeline,
        &mut on_frame,
        &mut || String::new(),
        step_ms,
    )
}

#[cfg(windows)]
pub fn run_realtime_with_hwnd_and_hook_and_thought<
    S: FrameSource,
    I: InputCollector,
    F: FnMut(&FrameRecord, bool, &CursorProvider),
    T: FnMut() -> String,
>(
    mut capture: S,
    mut input: I,
    target_hwnd: isize,
    debug_cursor: bool,
    mut pipeline: SessionPipeline,
    on_frame: &mut F,
    thought_provider: &mut T,
    step_ms: u64,
) -> io::Result<SessionLayout> {
    run_realtime_with_hwnd_and_hook_and_thought_with_stop(
        capture,
        input,
        target_hwnd,
        debug_cursor,
        pipeline,
        on_frame,
        thought_provider,
        &mut || false,
        step_ms,
    )
}

#[cfg(windows)]
pub fn run_realtime_with_hwnd_and_hook_and_thought_with_stop<
    S: FrameSource,
    I: InputCollector,
    F: FnMut(&FrameRecord, bool, &CursorProvider),
    T: FnMut() -> String,
    P: FnMut() -> bool,
>(
    mut capture: S,
    mut input: I,
    target_hwnd: isize,
    debug_cursor: bool,
    mut pipeline: SessionPipeline,
    on_frame: &mut F,
    thought_provider: &mut T,
    should_stop: &mut P,
    step_ms: u64,
) -> io::Result<SessionLayout> {
    let step_ticks = qpc_step_ticks(step_ms)?;
    let mut cursor_test = CursorTestState::new();
    set_per_monitor_dpi_awareness();
    loop {
        if should_stop() {
            break;
        }
        let frame = match capture.next_frame() {
            Ok(frame) => frame,
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => break,
            Err(err) => return Err(err),
        };

        let window_end = frame.qpc_ts;
        let window_start = window_end.saturating_sub(step_ticks);
        let events = input.drain_events(window_start, window_end)?;
        if events.is_empty() {
            eprintln!(
                "[input] step={} events=0 window=({}-{})",
                frame.step_index, window_start, window_end
            );
        } else {
            eprintln!(
                "[input] step={} events={}",
                frame.step_index,
                events.len()
            );
        }
        let (is_foreground, cursor, debug_info) = sample_foreground_and_cursor(
            target_hwnd,
            frame.src_width,
            frame.src_height,
            frame.width,
            frame.height,
        )?;
        on_frame(&frame, is_foreground, &cursor);
        if debug_cursor && cursor_test.triggered(&events) {
            cursor_test.log_result(&cursor, debug_info.as_ref());
        }
        if debug_cursor {
            if let Some(info) = debug_info {
                eprintln!(
                    "[cursor] step={} fg={} vis={} dpi={} client=({}, {}) client_wh=({:.1}, {:.1}) src=({:.1}, {:.1}) src_wh=({:.1}, {:.1}) record_wh=({:.1}, {:.1}) scale={:.4} pad=({:.1}, {:.1}) record_xy=({:.1}, {:.1}) norm=({:.4}, {:.4})",
                    frame.step_index,
                    is_foreground,
                    cursor.visible,
                    info.dpi,
                    info.client_x,
                    info.client_y,
                    info.client_w,
                    info.client_h,
                    info.src_x,
                    info.src_y,
                    info.src_w,
                    info.src_h,
                    info.record_w,
                    info.record_h,
                    info.scale,
                    info.pad_x,
                    info.pad_y,
                    info.record_x,
                    info.record_y,
                    cursor.x_norm,
                    cursor.y_norm
                );
            }
        }

        let thought_line = thought_provider();
        pipeline.process_window(
            &events,
            window_start,
            window_end,
            frame.step_index,
            is_foreground,
            &cursor,
            &frame.data,
            Some(thought_line.as_str()),
        )?;
    }

    pipeline.finalize()
}

fn qpc_step_ticks(step_ms: u64) -> io::Result<u64> {
    #[cfg(windows)]
    {
        unsafe {
            let mut freq = 0i64;
            QueryPerformanceFrequency(&mut freq)
                .map_err(|err| io::Error::new(io::ErrorKind::Other, format!("{:?}", err)))?;
            let ticks = (freq as u64)
                .saturating_mul(step_ms)
                .saturating_div(1000)
                .max(1);
            Ok(ticks)
        }
    }
    #[cfg(not(windows))]
    {
        Ok(step_ms.max(1))
    }
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
        if GetCursorInfo(&mut ci).is_ok() {
            visible = (ci.flags.0 & CURSOR_SHOWING.0) != 0;
        }

        let mut point = windows::Win32::Foundation::POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point).is_ok()
            && record_width > 0
            && record_height > 0
            && src_width > 0
            && src_height > 0
        {
            let mut client_point = point;
            if ScreenToClient(target, &mut client_point).as_bool() {
                let mut rect = windows::Win32::Foundation::RECT::default();
                if GetClientRect(target, &mut rect).is_ok() {
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
                            dpi: dpi.round() as u32,
                            client_x: client_point.x,
                            client_y: client_point.y,
                            client_w,
                            client_h,
                            src_x,
                            src_y,
                            src_w: src_width,
                            src_h: src_height,
                            record_w: record_width,
                            record_h: record_height,
                            scale,
                            pad_x,
                            pad_y,
                            record_x,
                            record_y,
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
    dpi: u32,
    client_x: i32,
    client_y: i32,
    client_w: f32,
    client_h: f32,
    src_x: f32,
    src_y: f32,
    src_w: u32,
    src_h: u32,
    record_w: u32,
    record_h: u32,
    scale: f32,
    pad_x: f32,
    pad_y: f32,
    record_x: f32,
    record_y: f32,
}

#[cfg(windows)]
struct CursorTestState {
    index: usize,
}

#[cfg(windows)]
impl CursorTestState {
    fn new() -> Self {
        Self { index: 0 }
    }

    fn triggered(&self, events: &[InputEvent]) -> bool {
        const TEST_KEY: &str = "Seven"; // F7
        events.iter().any(|event| {
            matches!(
                event.kind,
                InputEventKind::KeyDown { ref key } if key == TEST_KEY
            )
        })
    }

    fn log_result(&mut self, cursor: &CursorProvider, debug: Option<&CursorDebug>) {
        let targets = [
            ("top_left", 0.0, 0.0),
            ("top_right", 1.0, 0.0),
            ("bottom_right", 1.0, 1.0),
            ("bottom_left", 0.0, 1.0),
            ("center", 0.5, 0.5),
        ];
        let target = targets[self.index % targets.len()];
        if let Some(debug) = debug {
            eprintln!(
                "[cursor-test] target={} expected=({:.2}, {:.2}) measured=({:.4}, {:.4}) record_xy=({:.1}, {:.1})",
                target.0,
                target.1,
                target.2,
                cursor.x_norm,
                cursor.y_norm,
                debug.record_x,
                debug.record_y
            );
        } else {
            eprintln!(
                "[cursor-test] target={} expected=({:.2}, {:.2}) measured=({:.4}, {:.4})",
                target.0,
                target.1,
                target.2,
                cursor.x_norm,
                cursor.y_norm
            );
        }
        self.index = self.index.saturating_add(1);
    }
}

#[allow(dead_code)]
pub fn default_session_name(now: &str, run_id: u32) -> String {
    format!("{}_run{:03}", now, run_id)
}

pub fn format_thought_line(content: &str) -> String {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        THOUGHT_TEMPLATE.to_string()
    } else if trimmed.contains("<|labeling_instruct_start|>")
        && trimmed.contains("<|labeling_instruct_end|>")
    {
        trimmed.to_string()
    } else {
        format!(
            "<|labeling_instruct_start|>{} <|labeling_instruct_end|>",
            trimmed
        )
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

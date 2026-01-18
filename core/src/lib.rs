use serde::{Deserialize, Serialize};

pub type QpcTimestamp = u64;
pub type StepIndex = u64;

pub const STEP_MS: u64 = 200;
pub const CAPTURE_FPS: u32 = 5;
pub const RECORD_WIDTH: u32 = 1280;
pub const RECORD_HEIGHT: u32 = 720;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    pub schema_version: u32,
    pub capture: CaptureOptions,
    pub input: InputOptions,
    pub timing: TimingOptions,
    pub auto_events: AutoEventsOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureOptions {
    pub api: CaptureApi,
    pub fps: u32,
    pub record_resolution: [u32; 2],
    pub resize_mode: ResizeMode,
    pub color_format: ColorFormat,
    pub include_cursor_in_video: bool,
    pub target: CaptureTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptureTarget {
    pub method: String,
    pub window_title: Option<String>,
    pub process_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CaptureApi {
    #[serde(rename = "WindowsGraphicsCapture")]
    WindowsGraphicsCapture,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ResizeMode {
    #[serde(rename = "letterbox")]
    Letterbox,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColorFormat {
    #[serde(rename = "BGRA8")]
    Bgra8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputOptions {
    pub keyboard: InputApi,
    pub mouse: InputApi,
    pub mouse_mode: MouseMode,
    pub dpi_awareness: DpiAwareness,
    pub foreground_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InputApi {
    #[serde(rename = "RawInput")]
    RawInput,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MouseMode {
    #[serde(rename = "relative_plus_pointer_mixed")]
    RelativePlusPointerMixed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DpiAwareness {
    #[serde(rename = "PerMonitorV2")]
    PerMonitorV2,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimingOptions {
    pub clock: ClockType,
    pub step_ms: u64,
    pub fps: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClockType {
    #[serde(rename = "QPC")]
    Qpc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoEventsOptions {
    pub enabled: bool,
    pub roi_config: String,
    pub stability_frames: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Meta {
    pub session_id: String,
    pub game: String,
    pub os: String,
    pub cpu: String,
    pub gpu: String,
    pub qpc_frequency_hz: u64,
    pub build: BuildInfo,
    pub notes: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildInfo {
    pub collector_version: String,
    pub git_commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameRecord {
    pub step_index: StepIndex,
    pub qpc_ts: QpcTimestamp,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionSnapshot {
    pub step_index: StepIndex,
    pub qpc_ts: QpcTimestamp,
    pub window: WindowState,
    pub mouse: MouseSnapshot,
    pub keyboard: KeyboardSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub is_foreground: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseSnapshot {
    pub dx: i32,
    pub dy: i32,
    pub wheel: i32,
    pub buttons: MouseButtons,
    pub cursor: CursorSample,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MouseButtons {
    pub left: bool,
    pub right: bool,
    pub middle: bool,
    pub x1: bool,
    pub x2: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CursorSample {
    pub visible: bool,
    pub x_norm: f32,
    pub y_norm: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyboardSnapshot {
    pub down: Vec<String>,
    pub pressed: Vec<String>,
    pub released: Vec<String>,
}

impl Options {
    pub fn default_v1() -> Self {
        Self {
            schema_version: 1,
            capture: CaptureOptions {
                api: CaptureApi::WindowsGraphicsCapture,
                fps: CAPTURE_FPS,
                record_resolution: [RECORD_WIDTH, RECORD_HEIGHT],
                resize_mode: ResizeMode::Letterbox,
                color_format: ColorFormat::Bgra8,
                include_cursor_in_video: false,
                target: CaptureTarget {
                    method: "gui".to_string(),
                    window_title: None,
                    process_name: None,
                },
            },
            input: InputOptions {
                keyboard: InputApi::RawInput,
                mouse: InputApi::RawInput,
                mouse_mode: MouseMode::RelativePlusPointerMixed,
                dpi_awareness: DpiAwareness::PerMonitorV2,
                foreground_only: true,
            },
            timing: TimingOptions {
                clock: ClockType::Qpc,
                step_ms: STEP_MS,
                fps: CAPTURE_FPS,
            },
            auto_events: AutoEventsOptions {
                enabled: false,
                roi_config: "rois_config_1280x720.json".to_string(),
                stability_frames: 3,
            },
        }
    }
}

impl Default for MouseButtons {
    fn default() -> Self {
        Self {
            left: false,
            right: false,
            middle: false,
            x1: false,
            x2: false,
        }
    }
}

impl Default for CursorSample {
    fn default() -> Self {
        Self {
            visible: false,
            x_norm: 0.0,
            y_norm: 0.0,
        }
    }
}

impl Default for KeyboardSnapshot {
    fn default() -> Self {
        Self {
            down: Vec::new(),
            pressed: Vec::new(),
            released: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct InputEvent {
    pub qpc_ts: QpcTimestamp,
    pub kind: InputEventKind,
}

#[derive(Debug, Clone)]
pub enum InputEventKind {
    KeyDown { key: String },
    KeyUp { key: String },
    MouseMove { dx: i32, dy: i32 },
    MouseWheel { delta: i32 },
    MouseButton { button: MouseButton, is_down: bool },
}

#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    X1,
    X2,
}

use std::collections::{HashSet, VecDeque};
use std::io;

use collector_core::{InputEvent, InputEventKind, MouseButton, QpcTimestamp};

mod rawinput;

pub trait InputCollector {
    fn drain_events(&mut self, start: QpcTimestamp, end: QpcTimestamp) -> io::Result<Vec<InputEvent>>;
}

const DEFAULT_MAX_EVENTS: usize = 20_000;

pub struct RawInputCollector {
    inner: rawinput::RawInputCollectorImpl,
    buffer: VecDeque<InputEvent>,
    max_events: usize,
    dropped_events: u64,
}

impl RawInputCollector {
    pub fn new() -> io::Result<Self> {
        Self::new_with_target(None)
    }

    pub fn new_with_target(target_hwnd: Option<isize>) -> io::Result<Self> {
        let inner = rawinput::RawInputCollectorImpl::new(target_hwnd)?;
        Ok(Self {
            inner,
            buffer: VecDeque::new(),
            max_events: DEFAULT_MAX_EVENTS,
            dropped_events: 0,
        })
    }

    pub fn with_limits(target_hwnd: Option<isize>, max_events: usize) -> io::Result<Self> {
        let inner = rawinput::RawInputCollectorImpl::new(target_hwnd)?;
        Ok(Self {
            inner,
            buffer: VecDeque::new(),
            max_events: max_events.max(1),
            dropped_events: 0,
        })
    }

    pub fn take_dropped_events(&mut self) -> u64 {
        let out = self.dropped_events;
        self.dropped_events = 0;
        out
    }

    fn enforce_limit(&mut self) {
        if self.buffer.len() <= self.max_events {
            return;
        }
        let excess = self.buffer.len() - self.max_events;
        for _ in 0..excess {
            self.buffer.pop_front();
        }
        self.dropped_events = self.dropped_events.saturating_add(excess as u64);
    }
}

impl InputCollector for RawInputCollector {
    fn drain_events(&mut self, start: QpcTimestamp, end: QpcTimestamp) -> io::Result<Vec<InputEvent>> {
        self.inner.drain_into(&mut self.buffer)?;
        self.enforce_limit();
        while matches!(self.buffer.front(), Some(ev) if ev.qpc_ts < start) {
            self.buffer.pop_front();
        }
        self.enforce_limit();
        let mut out = Vec::new();
        while matches!(self.buffer.front(), Some(ev) if ev.qpc_ts < end) {
            if let Some(ev) = self.buffer.pop_front() {
                out.push(ev);
            }
        }
        Ok(out)
    }
}

pub struct MockInputCollector {
    events: Vec<InputEvent>,
    index: usize,
}

impl MockInputCollector {
    pub fn new(events: Vec<InputEvent>) -> Self {
        Self { events, index: 0 }
    }
}

impl InputCollector for MockInputCollector {
    fn drain_events(&mut self, start: QpcTimestamp, end: QpcTimestamp) -> io::Result<Vec<InputEvent>> {
        let mut out = Vec::new();
        while self.index < self.events.len() && self.events[self.index].qpc_ts < start {
            self.index += 1;
        }
        while self.index < self.events.len() && self.events[self.index].qpc_ts < end {
            out.push(self.events[self.index].clone());
            self.index += 1;
        }
        Ok(out)
    }
}
#[derive(Debug, Default)]
pub struct InputState {
    pub down_keys: HashSet<String>,
}

impl InputState {
    pub fn new() -> Self {
        Self {
            down_keys: HashSet::new(),
        }
    }

    pub fn apply_event(&mut self, event: &InputEvent) {
        match &event.kind {
            InputEventKind::KeyDown { key } => {
                self.down_keys.insert(key.clone());
            }
            InputEventKind::KeyUp { key } => {
                self.down_keys.remove(key);
            }
            InputEventKind::MouseButton { button, is_down } => {
                let key = mouse_button_name(*button).to_string();
                if *is_down {
                    self.down_keys.insert(key);
                } else {
                    self.down_keys.remove(&key);
                }
            }
            _ => {}
        }
    }
}

pub fn keyboard_key_name(vk: u16) -> Option<&'static str> {
    match vk {
        0x41..=0x5A => {
            const LETTERS: [&str; 26] = [
                "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P",
                "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z",
            ];
            let idx = (vk - 0x41) as usize;
            Some(LETTERS[idx])
        }
        0x30..=0x39 => {
            const DIGITS: [&str; 10] = [
                "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
            ];
            let idx = (vk - 0x30) as usize;
            Some(DIGITS[idx])
        }
        0x60..=0x69 => {
            const NUMPAD: [&str; 10] = [
                "Numpad0",
                "Numpad1",
                "Numpad2",
                "Numpad3",
                "Numpad4",
                "Numpad5",
                "Numpad6",
                "Numpad7",
                "Numpad8",
                "Numpad9",
            ];
            let idx = (vk - 0x60) as usize;
            Some(NUMPAD[idx])
        }
        0x70 => Some("One"),
        0x71 => Some("Two"),
        0x72 => Some("Three"),
        0x73 => Some("Four"),
        0x74 => Some("Five"),
        0x75 => Some("Six"),
        0x76 => Some("Seven"),
        0x77 => Some("Eight"),
        0x78 => Some("Nine"),
        0x79 => Some("Ten"),
        0x7A => Some("Eleven"),
        0x7B => Some("Twelve"),
        0x10 => Some("Shift"),
        0x11 => Some("Ctrl"),
        0x12 => Some("Alt"),
        0x20 => Some("Space"),
        0x1B => Some("Esc"),
        0x09 => Some("Tab"),
        0x0D => Some("Enter"),
        0x08 => Some("Backspace"),
        0x2D => Some("Insert"),
        0x2E => Some("Delete"),
        0x24 => Some("Home"),
        0x23 => Some("End"),
        0x21 => Some("PageUp"),
        0x22 => Some("PageDown"),
        0x13 => Some("Pause"),
        0x2C => Some("PrintScreen"),
        0x14 => Some("CapsLock"),
        0x90 => Some("NumLock"),
        0x91 => Some("ScrollLock"),
        0x26 => Some("Up"),
        0x28 => Some("Down"),
        0x25 => Some("Left"),
        0x27 => Some("Right"),
        0x5B => Some("LWin"),
        0x5C => Some("RWin"),
        0x5D => Some("Menu"),
        0x6A => Some("NumpadMultiply"),
        0x6B => Some("NumpadAdd"),
        0x6D => Some("NumpadSubtract"),
        0x6E => Some("NumpadDecimal"),
        0x6F => Some("NumpadDivide"),
        _ => None,
    }
}

pub fn mouse_button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "MouseLeft",
        MouseButton::Right => "MouseRight",
        MouseButton::Middle => "MouseMiddle",
        MouseButton::X1 => "MouseX1",
        MouseButton::X2 => "MouseX2",
    }
}

pub fn make_key_event(qpc_ts: QpcTimestamp, key: &str, is_down: bool) -> InputEvent {
    let kind = if is_down {
        InputEventKind::KeyDown {
            key: key.to_string(),
        }
    } else {
        InputEventKind::KeyUp {
            key: key.to_string(),
        }
    };
    InputEvent { qpc_ts, kind }
}

pub fn make_mouse_button_event(
    qpc_ts: QpcTimestamp,
    button: MouseButton,
    is_down: bool,
) -> InputEvent {
    InputEvent {
        qpc_ts,
        kind: InputEventKind::MouseButton { button, is_down },
    }
}

pub fn make_mouse_move_event(qpc_ts: QpcTimestamp, dx: i32, dy: i32) -> InputEvent {
    InputEvent {
        qpc_ts,
        kind: InputEventKind::MouseMove { dx, dy },
    }
}

pub fn make_mouse_wheel_event(qpc_ts: QpcTimestamp, delta: i32) -> InputEvent {
    InputEvent {
        qpc_ts,
        kind: InputEventKind::MouseWheel { delta },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_state_tracks_down_keys() {
        let mut state = InputState::new();
        let down = make_key_event(10, "W", true);
        let up = make_key_event(20, "W", false);

        state.apply_event(&down);
        assert!(state.down_keys.contains("W"));

        state.apply_event(&up);
        assert!(!state.down_keys.contains("W"));
    }
}

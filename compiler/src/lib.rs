use std::collections::HashSet;

use collector_core::{InputEvent, InputEventKind, MouseButton, QpcTimestamp};

const BIN_COUNT: usize = 6;
const DX_CLAMP: i32 = 1000;
const WHEEL_CLAMP: i32 = 5;

#[derive(Debug, Default)]
pub struct KeyState {
    down: HashSet<String>,
}

impl KeyState {
    pub fn new() -> Self {
        Self {
            down: HashSet::new(),
        }
    }
}

pub fn compile_action_string(
    events: &[InputEvent],
    window_start: QpcTimestamp,
    window_end: QpcTimestamp,
    key_state: &mut KeyState,
) -> String {
    let (dx, dy, wheel, bins) = compile_window(events, window_start, window_end, key_state);
    format_action_string(dx, dy, wheel, &bins)
}

fn compile_window(
    events: &[InputEvent],
    window_start: QpcTimestamp,
    window_end: QpcTimestamp,
    key_state: &mut KeyState,
) -> (i32, i32, i32, Vec<Vec<String>>) {
    let duration = window_end.saturating_sub(window_start);
    let base = duration / BIN_COUNT as u64;
    let remainder = duration - (base * BIN_COUNT as u64);

    let mut dx = 0i32;
    let mut dy = 0i32;
    let mut wheel = 0i32;
    let mut bins = Vec::with_capacity(BIN_COUNT);

    let mut event_index = 0usize;
    while event_index < events.len() && events[event_index].qpc_ts < window_start {
        event_index += 1;
    }

    let mut bin_start = window_start;
    for bin_idx in 0..BIN_COUNT {
        let bin_end = if bin_idx == BIN_COUNT - 1 {
            bin_start.saturating_add(base + remainder)
        } else {
            bin_start.saturating_add(base)
        };

        let mut bin_keys: HashSet<String> = key_state.down.iter().cloned().collect();

        while event_index < events.len() && events[event_index].qpc_ts < bin_end {
            let event = &events[event_index];
            match &event.kind {
                InputEventKind::KeyDown { key } => {
                    key_state.down.insert(key.clone());
                    bin_keys.insert(key.clone());
                }
                InputEventKind::KeyUp { key } => {
                    key_state.down.remove(key);
                }
                InputEventKind::MouseMove { dx: edx, dy: edy } => {
                    dx = dx.saturating_add(*edx);
                    dy = dy.saturating_add(*edy);
                }
                InputEventKind::MouseWheel { delta } => {
                    wheel = wheel.saturating_add(*delta);
                }
                InputEventKind::MouseButton { button, is_down } => {
                    let key = mouse_button_name(*button).to_string();
                    if *is_down {
                        key_state.down.insert(key.clone());
                        bin_keys.insert(key);
                    } else {
                        key_state.down.remove(&key);
                    }
                }
            }
            event_index += 1;
        }

        let mut ordered = sort_keys(&bin_keys);
        if ordered.len() > 4 {
            ordered.truncate(4);
        }
        bins.push(ordered);
        bin_start = bin_end;
    }

    (
        clamp(dx, DX_CLAMP),
        clamp(dy, DX_CLAMP),
        clamp(wheel, WHEEL_CLAMP),
        bins,
    )
}

fn format_action_string(dx: i32, dy: i32, wheel: i32, bins: &[Vec<String>]) -> String {
    let mut out = format!("<|action_start|>{} {} {}", dx, dy, wheel);
    for bin in bins.iter().take(BIN_COUNT) {
        out.push_str(" ;");
        if !bin.is_empty() {
            out.push(' ');
            out.push_str(&bin.join(" "));
        }
    }
    out.push_str("<|action_end|>");
    out
}

fn clamp(value: i32, limit: i32) -> i32 {
    if value > limit {
        limit
    } else if value < -limit {
        -limit
    } else {
        value
    }
}

fn mouse_button_name(button: MouseButton) -> &'static str {
    match button {
        MouseButton::Left => "MouseLeft",
        MouseButton::Right => "MouseRight",
        MouseButton::Middle => "MouseMiddle",
        MouseButton::X1 => "MouseX1",
        MouseButton::X2 => "MouseX2",
    }
}

fn sort_keys(keys: &HashSet<String>) -> Vec<String> {
    let mut list: Vec<String> = keys.iter().cloned().collect();
    list.sort_by(|a, b| {
        let (ga, oa) = key_rank(a);
        let (gb, ob) = key_rank(b);
        ga.cmp(&gb).then(oa.cmp(&ob)).then(a.cmp(b))
    });
    list
}

fn key_rank(key: &str) -> (u8, u8) {
    const MOUSE_KEYS: [&str; 3] = ["MouseLeft", "MouseRight", "MouseMiddle"];
    const MOD_KEYS: [&str; 3] = ["Shift", "Ctrl", "Alt"];
    const MOVE_KEYS: [&str; 4] = ["W", "A", "S", "D"];
    const NAV_KEYS: [&str; 4] = ["Space", "Esc", "Tab", "Enter"];
    const NUM_KEYS: [&str; 9] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine",
    ];
    const FUNC_KEYS: [&str; 12] = [
        "One",
        "Two",
        "Three",
        "Four",
        "Five",
        "Six",
        "Seven",
        "Eight",
        "Nine",
        "Ten",
        "Eleven",
        "Twelve",
    ];

    if let Some(idx) = index_of(&MOUSE_KEYS, key) {
        return (0, idx);
    }
    if let Some(idx) = index_of(&MOD_KEYS, key) {
        return (1, idx);
    }
    if let Some(idx) = index_of(&MOVE_KEYS, key) {
        return (2, idx);
    }
    if let Some(idx) = index_of(&NAV_KEYS, key) {
        return (3, idx);
    }
    if let Some(idx) = index_of(&NUM_KEYS, key) {
        return (4, idx);
    }
    if let Some(idx) = index_of(&FUNC_KEYS, key) {
        return (4, (NUM_KEYS.len() as u8).saturating_add(idx));
    }
    (5, 0)
}

fn index_of(list: &[&str], key: &str) -> Option<u8> {
    list.iter()
        .position(|item| *item == key)
        .map(|idx| idx as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_core::InputEventKind;

    #[test]
    fn empty_window_formats_correctly() {
        let events = Vec::<InputEvent>::new();
        let mut state = KeyState::new();
        let out = compile_action_string(&events, 0, 200, &mut state);
        assert_eq!(
            out,
            "<|action_start|>0 0 0 ; ; ; ; ; ;<|action_end|>"
        );
    }

    #[test]
    fn output_has_six_bins() {
        let events = vec![InputEvent {
            qpc_ts: 10,
            kind: InputEventKind::KeyDown {
                key: "W".to_string(),
            },
        }];
        let mut state = KeyState::new();
        let out = compile_action_string(&events, 0, 200, &mut state);
        assert_eq!(out.matches(';').count(), 6);
    }
}

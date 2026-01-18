use std::collections::HashSet;

use collector_core::{
    ActionSnapshot, CursorSample, InputEvent, InputEventKind, KeyboardSnapshot, MouseButtons,
    MouseSnapshot, QpcTimestamp, StepIndex, WindowState,
};

#[derive(Debug, Clone)]
pub struct CursorProvider {
    pub visible: bool,
    pub x_norm: f32,
    pub y_norm: f32,
}

impl CursorProvider {
    pub fn sample(&self) -> CursorSample {
        CursorSample {
            visible: self.visible,
            x_norm: self.x_norm,
            y_norm: self.y_norm,
        }
    }
}

#[derive(Debug, Default)]
pub struct AggregatorState {
    down_keys: HashSet<String>,
}

impl AggregatorState {
    pub fn new() -> Self {
        Self {
            down_keys: HashSet::new(),
        }
    }
}

pub fn aggregate_window(
    events: &[InputEvent],
    window_start: QpcTimestamp,
    window_end: QpcTimestamp,
    step_index: StepIndex,
    is_foreground: bool,
    cursor_provider: &CursorProvider,
    state: &mut AggregatorState,
) -> ActionSnapshot {
    let mut dx = 0i32;
    let mut dy = 0i32;
    let mut wheel = 0i32;
    let mut pressed = HashSet::new();
    let mut released = HashSet::new();
    let mut buttons = MouseButtons::default();

    for event in events.iter() {
        if event.qpc_ts < window_start || event.qpc_ts >= window_end {
            continue;
        }
        match &event.kind {
            InputEventKind::KeyDown { key } => {
                state.down_keys.insert(key.clone());
                pressed.insert(key.clone());
            }
            InputEventKind::KeyUp { key } => {
                state.down_keys.remove(key);
                released.insert(key.clone());
            }
            InputEventKind::MouseMove { dx: edx, dy: edy } => {
                dx = dx.saturating_add(*edx);
                dy = dy.saturating_add(*edy);
            }
            InputEventKind::MouseWheel { delta } => {
                wheel = wheel.saturating_add(*delta);
            }
            InputEventKind::MouseButton { button, is_down } => {
                if *is_down {
                    mark_button(&mut buttons, *button);
                }
            }
        }
    }

    let cursor = cursor_provider.sample();

    if !is_foreground {
        return ActionSnapshot {
            step_index,
            qpc_ts: window_end,
            window: WindowState { is_foreground },
            mouse: MouseSnapshot {
                dx: 0,
                dy: 0,
                wheel: 0,
                buttons: MouseButtons::default(),
                cursor,
            },
            keyboard: KeyboardSnapshot::default(),
        };
    }

    ActionSnapshot {
        step_index,
        qpc_ts: window_end,
        window: WindowState { is_foreground },
        mouse: MouseSnapshot {
            dx,
            dy,
            wheel,
            buttons,
            cursor,
        },
        keyboard: KeyboardSnapshot {
            down: sorted_vec(&state.down_keys),
            pressed: sorted_vec(&pressed),
            released: sorted_vec(&released),
        },
    }
}

fn sorted_vec(input: &HashSet<String>) -> Vec<String> {
    let mut out: Vec<String> = input.iter().cloned().collect();
    out.sort();
    out
}

fn mark_button(buttons: &mut MouseButtons, button: collector_core::MouseButton) {
    match button {
        collector_core::MouseButton::Left => buttons.left = true,
        collector_core::MouseButton::Right => buttons.right = true,
        collector_core::MouseButton::Middle => buttons.middle = true,
        collector_core::MouseButton::X1 => buttons.x1 = true,
        collector_core::MouseButton::X2 => buttons.x2 = true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use collector_core::{InputEvent, InputEventKind};

    #[test]
    fn clears_inputs_when_not_foreground() {
        let events = vec![InputEvent {
            qpc_ts: 10,
            kind: InputEventKind::MouseMove { dx: 5, dy: -3 },
        }];
        let cursor = CursorProvider {
            visible: true,
            x_norm: 0.5,
            y_norm: 0.5,
        };
        let mut state = AggregatorState::new();
        let snapshot =
            aggregate_window(&events, 0, 200, 0, false, &cursor, &mut state);
        assert_eq!(snapshot.mouse.dx, 0);
        assert_eq!(snapshot.keyboard.down.len(), 0);
    }
}

# End-LVM Data Collector

## What It Does
This project records synchronized gameplay video and input data for training
and evaluation. It captures a target window at 5 FPS, aggregates RawInput
events into 200ms steps, and writes both raw actions and compiled action
strings into a session folder.

## Build & Run
Requirements:
- Rust toolchain (stable)
- `ffmpeg` available on PATH or provide a full path

CLI dry-run (replays events into a session):
```bash
cargo run -p app -- \
  --session-name 2026-01-18_10-30-00_run001 \
  --steps 30 \
  --dataset-root D:/dataset \
  --ffmpeg ffmpeg \
  --frame-raw path/to/frame.bgra \
  --events-jsonl path/to/events.jsonl \
  --thoughts-jsonl path/to/thoughts.jsonl
```

CLI realtime (capture a window by HWND):
```bash
cargo run -p app -- \
  --session-name 2026-01-18_10-30-00_run001 \
  --dataset-root D:/dataset \
  --ffmpeg ffmpeg \
  --target-hwnd 0x00123456
```

GUI (Tauri, Windows):
```bash
cargo run -p gui --features tauri
```

## Inputs
Realtime capture:
- Target HWND (window handle) chosen by GUI or CLI.
- Keyboard/mouse input via RawInput (foreground-only).

Dry-run capture (CLI):
- `events.jsonl` stream of `InputEvent` records.
- Optional raw BGRA frame (1280x720) reused for each step.
- Optional `thoughts.jsonl` (one line per step).

## Outputs
Each session is written under `dataset_root/sessions/<session_name>/`:
- `video.mp4` (5 FPS, 1280x720, H.264)
- `actions.jsonl` (5Hz snapshots with `step_index`)
- `compiled_actions.jsonl` (one action string per step)
- `thoughts.jsonl` (aligned with `actions.jsonl`)
- `auto_events.jsonl` (reserved, empty by default)
- `options.json`, `meta.json`

## Notes & Constraints
- Windows 10 21H2+ / Windows 11, x64.
- Capture API is Windows Graphics Capture only.
- Recording is fixed to 5 FPS and 1280x720 letterbox.
- Foreground-only input is enforced.
- Capture fails if the window is invalid, hidden, minimized, cloaked, or
  fullscreen-like.

## Input Events JSONL
Each line is a single `InputEvent` JSON object.

Examples:
```json
{"qpc_ts":10,"type":"key_down","key":"W"}
{"qpc_ts":20,"type":"key_up","key":"W"}
{"qpc_ts":30,"type":"mouse_move","dx":12,"dy":-5}
{"qpc_ts":40,"type":"mouse_wheel","delta":120}
{"qpc_ts":50,"type":"mouse_button","button":"left","is_down":true}
```

Notes:
- `type` values: `key_down`, `key_up`, `mouse_move`, `mouse_wheel`, `mouse_button`.
- `button` values: `left`, `right`, `middle`, `x1`, `x2`.
- `qpc_ts` should be in the same units used by the pipeline; the CLI treats it
  as an opaque timestamp and slices windows using `step_index * 200ms`.

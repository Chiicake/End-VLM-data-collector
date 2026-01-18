mod pipeline;

use std::env;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use aggregator::CursorProvider;
use capture::WgcCapture;
use collector_core::{BuildInfo, InputEvent, Meta, Options, RECORD_HEIGHT, RECORD_WIDTH, STEP_MS};
use pipeline::{ensure_dataset_root, PipelineConfig, SessionPipeline};

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {}", err);
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let args = parse_args().map_err(|msg| io::Error::new(io::ErrorKind::Other, msg))?;
    ensure_dataset_root(&args.dataset_root)?;

    let config = PipelineConfig {
        dataset_root: args.dataset_root.clone(),
        session_name: args.session_name.clone(),
        ffmpeg_path: args.ffmpeg_path.clone(),
    };

    let pipeline = SessionPipeline::create(config)?;
    let options = build_options();
    let meta = build_meta(&args.session_name);
    pipeline.write_options_meta(&options, &meta)?;

    let layout = if let Some(hwnd) = args.target_hwnd {
        let capture = WgcCapture::new(options.capture.clone(), hwnd)?;
        let input = input::RawInputCollector::new()?;
        let _cursor = CursorProvider {
            visible: false,
            x_norm: 0.0,
            y_norm: 0.0,
        };
        #[cfg(windows)]
        {
            pipeline::run_realtime_with_hwnd(capture, input, hwnd, pipeline)?
        }
        #[cfg(not(windows))]
        {
            let _ = hwnd;
            let _ = capture;
            let _ = input;
            let _ = pipeline;
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "--target-hwnd requires Windows",
            ));
        }
    } else {
        let events = if let Some(path) = args.events_jsonl.as_ref() {
            load_events(path)?
        } else {
            Vec::new()
        };
        let thoughts = if let Some(path) = args.thoughts_jsonl.as_ref() {
            load_lines(path)?
        } else {
            Vec::new()
        };
        let frame = load_frame(args.frame_raw.as_ref())?;

        let cursor = CursorProvider {
            visible: false,
            x_norm: 0.0,
            y_norm: 0.0,
        };

        let mut pipeline = pipeline;
        let mut event_index = 0usize;
        for step in 0..args.steps {
            let window_start = step.saturating_mul(STEP_MS);
            let window_end = window_start.saturating_add(STEP_MS);

            while event_index < events.len() && events[event_index].qpc_ts < window_start {
                event_index += 1;
            }
            let start_idx = event_index;
            while event_index < events.len() && events[event_index].qpc_ts < window_end {
                event_index += 1;
            }
            let window_events = &events[start_idx..event_index];
            let thought = thoughts.get(step as usize).map(|s| s.as_str());

            pipeline.process_window(
                window_events,
                window_start,
                window_end,
                step,
                true,
                &cursor,
                &frame,
                thought,
            )?;
        }
        pipeline.finalize()?
    };
    println!("session written to {}", layout.root_dir.display());
    Ok(())
}

struct Args {
    dataset_root: PathBuf,
    session_name: String,
    ffmpeg_path: PathBuf,
    steps: u64,
    frame_raw: Option<PathBuf>,
    events_jsonl: Option<PathBuf>,
    thoughts_jsonl: Option<PathBuf>,
    target_hwnd: Option<isize>,
}

fn parse_args() -> Result<Args, String> {
    let mut dataset_root: Option<PathBuf> = None;
    let mut session_name: Option<String> = None;
    let mut ffmpeg_path: Option<PathBuf> = None;
    let mut steps: Option<u64> = None;
    let mut frame_raw: Option<PathBuf> = None;
    let mut events_jsonl: Option<PathBuf> = None;
    let mut thoughts_jsonl: Option<PathBuf> = None;
    let mut target_hwnd: Option<isize> = None;

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--dataset-root" => {
                dataset_root = Some(next_value(&mut iter, &arg)?);
            }
            "--session-name" => {
                session_name = Some(next_string(&mut iter, &arg)?);
            }
            "--ffmpeg" => {
                ffmpeg_path = Some(next_value(&mut iter, &arg)?);
            }
            "--steps" => {
                let value = next_string(&mut iter, &arg)?;
                steps = Some(value.parse::<u64>().map_err(|_| {
                    format!("invalid --steps value: {}", value)
                })?);
            }
            "--frame-raw" => {
                frame_raw = Some(next_value(&mut iter, &arg)?);
            }
            "--events-jsonl" => {
                events_jsonl = Some(next_value(&mut iter, &arg)?);
            }
            "--thoughts-jsonl" => {
                thoughts_jsonl = Some(next_value(&mut iter, &arg)?);
            }
            "--target-hwnd" => {
                let value = next_string(&mut iter, &arg)?;
                target_hwnd = Some(parse_hwnd(&value)?);
            }
            "--help" | "-h" => {
                return Err(usage());
            }
            _ => {
                return Err(format!("unknown argument: {}\n{}", arg, usage()));
            }
        }
    }

    let dataset_root = dataset_root.unwrap_or_else(|| PathBuf::from("D:/dataset"));
    let session_name = session_name.ok_or_else(|| "missing --session-name".to_string())?;
    let ffmpeg_path = ffmpeg_path.unwrap_or_else(|| PathBuf::from("ffmpeg"));
    let steps = steps.unwrap_or(0);
    if target_hwnd.is_none() && steps == 0 {
        return Err("missing --steps (required for dry-run mode)".to_string());
    }

    Ok(Args {
        dataset_root,
        session_name,
        ffmpeg_path,
        steps,
        frame_raw,
        events_jsonl,
        thoughts_jsonl,
        target_hwnd,
    })
}

fn usage() -> String {
    let text = r#"Usage:
  collector-cli --session-name <name> --steps <n> [options]

Options:
  --dataset-root <path>   Dataset root directory (default: D:/dataset)
  --ffmpeg <path>         Path to ffmpeg executable (default: ffmpeg)
  --frame-raw <path>      Raw BGRA frame file (1280x720x4 bytes) to reuse
  --events-jsonl <path>   Input events JSONL with qpc_ts timestamps
  --thoughts-jsonl <path> Thoughts JSONL (one line per step)
  --target-hwnd <hex>     Capture target HWND (enables WGC capture)
  --help                  Show this help
"#;
    text.to_string()
}

fn next_value(
    iter: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<PathBuf, String> {
    let value = iter
        .next()
        .ok_or_else(|| format!("missing value for {}", flag))?;
    Ok(PathBuf::from(value))
}

fn next_string(
    iter: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, String> {
    iter.next()
        .ok_or_else(|| format!("missing value for {}", flag))
}

fn parse_hwnd(value: &str) -> Result<isize, String> {
    if let Some(stripped) = value.strip_prefix("0x") {
        isize::from_str_radix(stripped, 16).map_err(|_| "invalid hwnd hex".to_string())
    } else {
        value
            .parse::<isize>()
            .map_err(|_| "invalid hwnd value".to_string())
    }
}

fn load_events(path: &Path) -> io::Result<Vec<InputEvent>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut events = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: InputEvent = serde_json::from_str(&line)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        events.push(event);
    }
    Ok(events)
}

fn load_lines(path: &Path) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {
        lines.push(line?);
    }
    Ok(lines)
}

fn load_frame(path: Option<&PathBuf>) -> io::Result<Vec<u8>> {
    let size = (RECORD_WIDTH as usize)
        .saturating_mul(RECORD_HEIGHT as usize)
        .saturating_mul(4);
    let data = if let Some(path) = path {
        std::fs::read(path)?
    } else {
        vec![0u8; size]
    };
    if data.len() != size {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("frame size mismatch: expected {} bytes", size),
        ));
    }
    Ok(data)
}

fn build_options() -> Options {
    let mut options = Options::default_v1();
    options.capture.target.method = "cli".to_string();
    options
}

fn build_meta(session_id: &str) -> Meta {
    Meta {
        session_id: session_id.to_string(),
        game: "".to_string(),
        os: "unknown".to_string(),
        cpu: "unknown".to_string(),
        gpu: "unknown".to_string(),
        qpc_frequency_hz: 0,
        build: BuildInfo {
            collector_version: "0.1.0".to_string(),
            git_commit: "unknown".to_string(),
        },
        notes: "".to_string(),
    }
}

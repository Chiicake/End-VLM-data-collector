use std::io::{self, Write};
use std::time::{Duration, Instant};

use aggregator::AggregatedWindow;
use collector_core::ActionSnapshot;
use serde::Serialize;

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

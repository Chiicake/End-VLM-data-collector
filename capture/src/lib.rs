use std::io;

use collector_core::{CaptureOptions, FrameRecord};

mod wgc;

pub trait FrameSource {
    fn next_frame(&mut self) -> io::Result<FrameRecord>;
}

pub struct WgcCapture {
    inner: wgc::WgcCaptureImpl,
}

impl WgcCapture {
    pub fn new(options: CaptureOptions, target_hwnd: isize) -> io::Result<Self> {
        let inner = wgc::WgcCaptureImpl::new(&options, target_hwnd)?;
        Ok(Self { inner })
    }
}

impl FrameSource for WgcCapture {
    fn next_frame(&mut self) -> io::Result<FrameRecord> {
        self.inner.next_frame()
    }
}

pub struct MockCapture {
    frames: Vec<FrameRecord>,
    index: usize,
}

impl MockCapture {
    pub fn new(frames: Vec<FrameRecord>) -> Self {
        Self { frames, index: 0 }
    }
}

impl FrameSource for MockCapture {
    fn next_frame(&mut self) -> io::Result<FrameRecord> {
        if self.index >= self.frames.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "no more frames",
            ));
        }
        let frame = self.frames[self.index].clone();
        self.index += 1;
        Ok(frame)
    }
}

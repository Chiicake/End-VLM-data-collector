use std::io;

use collector_core::{CaptureOptions, FrameRecord};

#[cfg(windows)]
use windows::Win32::Foundation::HWND;

#[cfg(windows)]
pub struct WgcCaptureImpl {
    _options: CaptureOptions,
    _hwnd: HWND,
}

#[cfg(windows)]
impl WgcCaptureImpl {
    pub fn new(options: &CaptureOptions, target_hwnd: isize) -> io::Result<Self> {
        let hwnd = HWND(target_hwnd as isize);
        let _ = options;
        Err(io::Error::new(
            io::ErrorKind::Other,
            "WGC capture initialization not implemented yet",
        ))
    }

    pub fn next_frame(&mut self) -> io::Result<FrameRecord> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "WGC capture read not implemented yet",
        ))
    }
}

#[cfg(not(windows))]
pub struct WgcCaptureImpl;

#[cfg(not(windows))]
impl WgcCaptureImpl {
    pub fn new(_options: &CaptureOptions, _target_hwnd: isize) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "WGC capture requires Windows",
        ))
    }

    pub fn next_frame(&mut self) -> io::Result<FrameRecord> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "WGC capture requires Windows",
        ))
    }
}

use std::io;
use collector_core::{CaptureOptions, FrameRecord};

#[cfg(windows)]
use std::sync::mpsc::{self, Receiver};

#[cfg(windows)]
use windows::core::{Interface, Result as WinResult};
#[cfg(windows)]
use windows::Foundation::TypedEventHandler;
#[cfg(windows)]
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
#[cfg(windows)]
use windows::Graphics::DirectX::DirectXPixelFormat;
#[cfg(windows)]
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
#[cfg(windows)]
use windows::Graphics::SizeInt32;
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0};
#[cfg(windows)]
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
#[cfg(windows)]
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
#[cfg(windows)]
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
#[cfg(windows)]
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, GetDXGIInterfaceFromObject,
};
#[cfg(windows)]
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

#[cfg(windows)]
pub struct WgcCaptureImpl {
    options: CaptureOptions,
    item: GraphicsCaptureItem,
    _session: GraphicsCaptureSession,
    frame_pool: Direct3D11CaptureFramePool,
    frame_rx: Receiver<()>,
    d3d_device: IDirect3DDevice,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    content_size: SizeInt32,
    staging: Option<ID3D11Texture2D>,
    src_buffer: Vec<u8>,
    output_buffer: Vec<u8>,
    step_index: StepIndex,
    qpc_frequency: u64,
    next_capture_qpc: QpcTimestamp,
    step_ticks: u64,
}

#[cfg(windows)]
impl WgcCaptureImpl {
    pub fn new(options: &CaptureOptions, target_hwnd: isize) -> io::Result<Self> {
        let hwnd = HWND(target_hwnd as isize);
        let item = create_capture_item(hwnd).map_err(map_win_err)?;
        let (device, context, d3d_device) = create_d3d_device().map_err(map_win_err)?;
        let content_size = item.Size().map_err(map_win_err)?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            1,
            content_size,
        )
        .map_err(map_win_err)?;

        let (tx, rx) = mpsc::channel();
        let handler = TypedEventHandler::new(move |_sender, _| {
            let _ = tx.send(());
            Ok(())
        });
        frame_pool.FrameArrived(&handler).map_err(map_win_err)?;

        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(map_win_err)?;
        session.SetIsCursorCaptureEnabled(false).map_err(map_win_err)?;
        session.StartCapture().map_err(map_win_err)?;

        let qpc_frequency = qpc_frequency()?;
        let fps = options.fps.max(1) as u64;
        let step_ticks = (qpc_frequency / fps).max(1);

        Ok(Self {
            options: options.clone(),
            item,
            _session: session,
            frame_pool,
            frame_rx: rx,
            d3d_device,
            device,
            context,
            content_size,
            staging: None,
            src_buffer: Vec::new(),
            output_buffer: Vec::new(),
            step_index: 0,
            qpc_frequency,
            next_capture_qpc: 0,
            step_ticks,
        })
    }

    pub fn next_frame(&mut self) -> io::Result<FrameRecord> {
        loop {
            let _ = self.frame_rx.recv().map_err(|_| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "frame channel closed")
            })?;

            let frame = match self.frame_pool.TryGetNextFrame() {
                Ok(frame) => frame,
                Err(_) => continue,
            };
            let content_size = frame.ContentSize().map_err(map_win_err)?;
            if content_size.Width != self.content_size.Width
                || content_size.Height != self.content_size.Height
            {
                self.frame_pool
                    .Recreate(
                        &self.d3d_device,
                        DirectXPixelFormat::B8G8R8A8UIntNormalized,
                        1,
                        content_size,
                    )
                    .map_err(map_win_err)?;
                self.content_size = content_size;
            }

            let now = qpc_now()?;
            if self.next_capture_qpc == 0 {
                self.next_capture_qpc = now;
            }
            if now < self.next_capture_qpc {
                continue;
            }
            while now.saturating_sub(self.next_capture_qpc) >= self.step_ticks {
                self.next_capture_qpc = self.next_capture_qpc.saturating_add(self.step_ticks);
            }
            self.next_capture_qpc = self.next_capture_qpc.saturating_add(self.step_ticks);

            let texture = get_frame_texture(&frame).map_err(map_win_err)?;
            let (src_w, src_h) = (content_size.Width as u32, content_size.Height as u32);
            let src_bytes = read_texture(
                &self.device,
                &self.context,
                &texture,
                &mut self.staging,
                src_w,
                src_h,
                &mut self.src_buffer,
            )?;

            let dst_w = self.options.record_resolution[0];
            let dst_h = self.options.record_resolution[1];
            ensure_buffer_size(&mut self.output_buffer, dst_w, dst_h);
            letterbox_bgra(
                src_bytes,
                src_w,
                src_h,
                &mut self.output_buffer,
                dst_w,
                dst_h,
            );

            let record = FrameRecord {
                step_index: self.step_index,
                qpc_ts: now,
                width: dst_w,
                height: dst_h,
                data: self.output_buffer.clone(),
            };
            self.step_index = self.step_index.saturating_add(1);
            return Ok(record);
        }
    }
}

#[cfg(windows)]
fn create_capture_item(hwnd: HWND) -> WinResult<GraphicsCaptureItem> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    unsafe { interop.CreateForWindow(hwnd) }
}

#[cfg(windows)]
fn create_d3d_device() -> WinResult<(ID3D11Device, ID3D11DeviceContext, IDirect3DDevice)> {
    unsafe {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut feature_level = D3D_FEATURE_LEVEL_11_0;
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            Some(&[D3D_FEATURE_LEVEL_11_0]),
            D3D11_SDK_VERSION,
            &mut device,
            Some(&mut feature_level),
            &mut context,
        )?;
        let device = device.ok_or_else(|| windows::core::Error::from_win32())?;
        let context = context.ok_or_else(|| windows::core::Error::from_win32())?;
        let dxgi_device: IDXGIDevice = device.cast()?;
        let d3d_device = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)?;
        Ok((device, context, d3d_device))
    }
}

#[cfg(windows)]
fn get_frame_texture(frame: &windows::Graphics::Capture::Direct3D11CaptureFrame) -> WinResult<ID3D11Texture2D> {
    let surface = frame.Surface()?;
    unsafe { GetDXGIInterfaceFromObject(&surface) }
}

#[cfg(windows)]
fn read_texture(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    staging: &mut Option<ID3D11Texture2D>,
    width: u32,
    height: u32,
    buffer: &mut Vec<u8>,
) -> io::Result<&[u8]> {
    unsafe {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        texture.GetDesc(&mut desc);
        let needs_new = staging
            .as_ref()
            .map(|_| desc.Width != width || desc.Height != height)
            .unwrap_or(true);
        if needs_new {
            desc.BindFlags = 0;
            desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ;
            desc.Usage = D3D11_USAGE_STAGING;
            desc.MiscFlags = 0;
            desc.MipLevels = 1;
            desc.ArraySize = 1;
            desc.SampleDesc.Count = 1;
            desc.SampleDesc.Quality = 0;
            let staging_tex = device
                .CreateTexture2D(&desc, None)
                .map_err(map_win_err)?;
            *staging = Some(staging_tex);
        }
        let staging_tex = staging.as_ref().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "staging texture unavailable")
        })?;

        context.CopyResource(staging_tex, texture);

        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        context
            .Map(staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(map_win_err)?;

        let row_pitch = mapped.RowPitch as usize;
        let src_ptr = mapped.pData as *const u8;
        let dst_size = (width as usize)
            .saturating_mul(height as usize)
            .saturating_mul(4);
        buffer.resize(dst_size, 0);

        for y in 0..height as usize {
            let src_row = src_ptr.wrapping_add(y * row_pitch);
            let dst_row = buffer[y * width as usize * 4..].as_mut_ptr();
            std::ptr::copy_nonoverlapping(src_row, dst_row, width as usize * 4);
        }

        context.Unmap(staging_tex, 0);
    }
    Ok(buffer.as_slice())
}

#[cfg(windows)]
fn letterbox_bgra(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
) {
    dst.fill(0);
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return;
    }

    let scale_w = dst_w as f32 / src_w as f32;
    let scale_h = dst_h as f32 / src_h as f32;
    let scale = scale_w.min(scale_h);
    let mut scaled_w = (src_w as f32 * scale).round() as u32;
    let mut scaled_h = (src_h as f32 * scale).round() as u32;
    if scaled_w == 0 {
        scaled_w = 1;
    }
    if scaled_h == 0 {
        scaled_h = 1;
    }
    let pad_x = (dst_w.saturating_sub(scaled_w)) / 2;
    let pad_y = (dst_h.saturating_sub(scaled_h)) / 2;

    for y in 0..scaled_h {
        let src_y = (y as u64 * src_h as u64 / scaled_h as u64) as u32;
        for x in 0..scaled_w {
            let src_x = (x as u64 * src_w as u64 / scaled_w as u64) as u32;
            let src_idx = ((src_y * src_w + src_x) * 4) as usize;
            let dst_idx = (((y + pad_y) * dst_w + (x + pad_x)) * 4) as usize;
            if src_idx + 4 <= src.len() && dst_idx + 4 <= dst.len() {
                dst[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
            }
        }
    }
}

#[cfg(windows)]
fn ensure_buffer_size(buffer: &mut Vec<u8>, width: u32, height: u32) {
    let size = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if buffer.len() != size {
        buffer.clear();
        buffer.resize(size, 0);
    }
}

#[cfg(windows)]
fn qpc_frequency() -> io::Result<u64> {
    unsafe {
        let mut freq = 0i64;
        QueryPerformanceFrequency(&mut freq).map_err(map_win_err)?;
        Ok(freq as u64)
    }
}

#[cfg(windows)]
fn qpc_now() -> io::Result<QpcTimestamp> {
    unsafe {
        let mut counter = 0i64;
        QueryPerformanceCounter(&mut counter).map_err(map_win_err)?;
        Ok(counter as u64)
    }
}

#[cfg(windows)]
fn map_win_err(err: windows::core::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("{:?}", err))
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

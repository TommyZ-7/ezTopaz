//! WGC screen/window capture (design.md §3.1.1).
//!
//! A dedicated thread owns the D3D11 device and the capture pool/session; the
//! FrameArrived handler (free-threaded) copies the surface to a staging
//! texture, scales to the profile size and pushes into the [`VideoSink`].
//! Preview PNGs (1fps, 640x360) are emitted as `stream://preview` events.

use super::{co_init, err};
use base64::Engine;
use eztopaz_core::config::{Profile, ScreenTarget, ScreenTargetKind};
use eztopaz_core::ipc_types::PreviewFrame;
use eztopaz_core::video::{bgra_to_rgba, scale_bgra, VideoSink};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::Emitter;

const PREVIEW_W: u32 = 640;
const PREVIEW_H: u32 = 360;

pub struct ScreenCapture {
    pub sink: VideoSink,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ScreenCapture {
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        self.sink.stop();
    }
}

impl Drop for ScreenCapture {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn start_screen(
    app: tauri::AppHandle,
    target: &ScreenTarget,
    profile: &Profile,
    sink: VideoSink,
    cursor: bool,
) -> super::Result<ScreenCapture> {
    let stop = Arc::new(AtomicBool::new(false));
    let stop2 = stop.clone();
    let target = target.clone();
    let dst_w = profile.w;
    let dst_h = profile.h;
    let app2 = app.clone();

    let sink_for_capture = sink.clone();
    let handle = std::thread::Builder::new()
        .name("wgc-capture".into())
        .spawn(move || {
            if let Err(e) = run_capture(&app, &target, dst_w, dst_h, cursor, &sink_for_capture, &stop2) {
                let _ = app2.emit(
                    "stream://error",
                    eztopaz_core::ipc_types::StreamError {
                        code: "capture".into(),
                        msg: e.to_string(),
                    },
                );
            }
        })
        .map_err(err)?;

    Ok(ScreenCapture { sink, stop, handle: Some(handle) })
}

fn run_capture(
    app: &tauri::AppHandle,
    target: &ScreenTarget,
    dst_w: u32,
    dst_h: u32,
    cursor: bool,
    sink: &VideoSink,
    stop: &AtomicBool,
) -> super::Result<()> {
    use windows::Foundation::TypedEventHandler;
    use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem};
    use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
    use windows::Graphics::DirectX::DirectXPixelFormat;
    use windows::core::ComInterface;
    use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
    use windows::Win32::Graphics::Direct3D11::{
        D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CPU_ACCESS_READ, D3D11_MAP_READ,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;
    use windows::Win32::System::WinRT::Direct3D11::{
        CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
    };
    use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

    co_init();

    // D3D11 device
    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            windows::Win32::Foundation::HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(err)?;
    }
    let device = device.ok_or_else(|| err("no D3D11 device"))?;
    let context = context.ok_or_else(|| err("no D3D11 context"))?;

    let dxgi = device.cast::<IDXGIDevice>().map_err(err)?;
    let insp =
        unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }.map_err(err)?;
    let d3ddevice: IDirect3DDevice = insp.cast().map_err(err)?;

    // capture item via interop
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(err)?;
    let item: GraphicsCaptureItem = match target.kind {
        ScreenTargetKind::Display => {
            let hmon = super::enumerate::monitor_by_index(
                target.id.rsplit(':').next().and_then(|s| s.parse::<usize>().ok()).unwrap_or(0),
            )?;
            unsafe { interop.CreateForMonitor(hmon) }.map_err(err)?
        }
        ScreenTargetKind::Window => {
            let raw: usize = target
                .id
                .rsplit(':')
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let hwnd = windows::Win32::Foundation::HWND(raw as isize);
            unsafe { interop.CreateForWindow(hwnd) }.map_err(err)?
        }
    };
    let src_size = item.Size().map_err(err)?;
    let (_src_w, _src_h) = (src_size.Width.max(1) as u32, src_size.Height.max(1) as u32);

    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &d3ddevice,
        DirectXPixelFormat::B8G8R8A8UIntNormalized,
        2,
        windows::Graphics::SizeInt32 { Width: src_size.Width, Height: src_size.Height },
    )
    .map_err(err)?;
    let session = pool.CreateCaptureSession(&item).map_err(err)?;
    session.SetIsCursorCaptureEnabled(cursor).map_err(err)?;

    let sink2 = sink.clone();
    let stop2 = Arc::new(AtomicBool::new(false));
    let handler_stop = stop2.clone();
    let preview_last = Arc::new(Mutex::new(Instant::now() - Duration::from_secs(1)));
    let preview_last2 = preview_last.clone();
    let app2 = app.clone();

    let handler = TypedEventHandler::new(
        move |pool: &Option<Direct3D11CaptureFramePool>,
              _args: &Option<windows::core::IInspectable>| {
            let Some(pool) = pool else { return Ok(()) };
            let ctx = context.clone();
            loop {
                if handler_stop.load(Ordering::Relaxed) {
                    break;
                }
                let frame = match pool.TryGetNextFrame() {
                    Ok(f) => f,
                    Err(_) => break,
                };
                let size = match frame.ContentSize() {
                    Ok(s) => s,
                    Err(_) => break,
                };
                let w = size.Width.max(1) as u32;
                let h = size.Height.max(1) as u32;
                let Ok(surface) = frame.Surface() else { break };
                let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() else { break };
                let Ok(tex) = (unsafe { access.GetInterface::<ID3D11Texture2D>() }) else { break };

                let mut desc = Default::default();
                    unsafe { tex.GetDesc(&mut desc) };
                let staging_desc = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: desc.SampleDesc,
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: windows::Win32::Graphics::Direct3D11::D3D11_BIND_FLAG(0).0 as u32,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                unsafe {
                    let mut staging: Option<ID3D11Texture2D> = None;
                    if device.CreateTexture2D(&staging_desc, None, Some(&mut staging)).is_err() {
                        break;
                    }
                    let Some(staging) = staging else { break };
                    ctx.CopyResource(&staging, &tex);
                    let mut mapped = Default::default();
                    if ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped)).is_err() {
                        break;
                    }
                    let width = desc.Width as usize;
                    let height = desc.Height as usize;
                    let row_pitch = mapped.RowPitch as usize;
                    let mut buf = vec![0u8; width * height * 4];
                    let src = mapped.pData as *const u8;
                    for row in 0..height {
                        std::ptr::copy_nonoverlapping(
                            src.add(row * row_pitch),
                            buf.as_mut_ptr().add(row * width * 4),
                            width * 4,
                        );
                    }
                    ctx.Unmap(&staging, 0);

                    sink2.push(scale_bgra(&buf, w, h, dst_w, dst_h));

                    // 1fps preview (F-SC-03)
                    let mut last = preview_last2.lock().unwrap();
                    if last.elapsed() >= Duration::from_secs(1) {
                        *last = Instant::now();
                        let small = scale_bgra(&buf, w, h, PREVIEW_W, PREVIEW_H);
                        let rgba = bgra_to_rgba(&small);
                        if let Some(png) = encode_png(&rgba, PREVIEW_W, PREVIEW_H) {
                            let b64 =
                                base64::engine::general_purpose::STANDARD.encode(png);
                            let _ = app2.emit(
                                "stream://preview",
                                PreviewFrame {
                                    data_url: format!("data:image/png;base64,{b64}"),
                                    w: PREVIEW_W,
                                    h: PREVIEW_H,
                                },
                            );
                        }
                    }
                }
            }
            Ok(())
        },
    );
    pool.FrameArrived(&handler).map_err(err)?;
    session.StartCapture().map_err(err)?;

    // keep objects alive on this thread; close them on stop
    while !stop.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(100));
    }
    {
        let _ = session.Close();
        let _ = pool.Close();
    }
    Ok(())
}

fn encode_png(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    image::RgbaImage::from_raw(w, h, rgba.to_vec())?
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some(out.into_inner())
}

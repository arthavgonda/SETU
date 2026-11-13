use crate::ocr::Image;
use anyhow::{Result, anyhow};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Graphics::Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, HWND, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING, D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::DXGI_SAMPLE_DESC;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::System::WinRT::Direct3D11::{CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
use windows::core::Interface;

const LINGER: Duration = Duration::from_secs(10);
const FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

static NEXT: AtomicU64 = AtomicU64::new(1);
static REAPING: AtomicBool = AtomicBool::new(false);

struct Live {
    hwnd: isize,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    d3d: IDirect3DDevice,
    context: ID3D11DeviceContext,
    staging: Option<(ID3D11Texture2D, u32, u32)>,
    size: SizeInt32,
    last: Option<(u64, Arc<Image>)>,
    used: Instant,
    _device: ID3D11Device,
}

unsafe impl Send for Live {}

static LIVE: Mutex<Option<Live>> = Mutex::new(None);

pub struct Frame {
    pub img: Image,
    pub seq: u64,
    pub origin: (f64, f64),
}

fn devices() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    for kind in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let (mut d, mut c) = (None, None);
        let ok = unsafe { D3D11CreateDevice(None, kind, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut d), None, Some(&mut c)) }.is_ok();
        if ok && let (Some(d), Some(c)) = (d, c) {
            return Ok((d, c));
        }
    }
    Err(anyhow!("Direct3D isn't available for window pictures."))
}

fn open(hwnd: HWND) -> Result<Live> {
    if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
        return Err(anyhow!("Windows Graphics Capture isn't available on this Windows."));
    }
    let (device, context) = devices()?;
    let dxgi: IDXGIDevice = device.cast()?;
    let d3d: IDirect3DDevice = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi)? }.cast()?;
    let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;
    let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd)? };
    let size = item.Size()?;
    let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(&d3d, FORMAT, 2, size)?;
    let session = pool.CreateCaptureSession(&item)?;
    let _ = session.SetIsCursorCaptureEnabled(false);
    let _ = session.SetIsBorderRequired(false);
    session.StartCapture()?;
    Ok(Live { hwnd: hwnd.0 as isize, pool, session, d3d, context, staging: None, size, last: None, used: Instant::now(), _device: device })
}

fn close(l: Live) {
    let _ = l.session.Close();
    let _ = l.pool.Close();
}

fn staging(l: &mut Live, desc: &D3D11_TEXTURE2D_DESC) -> Result<ID3D11Texture2D> {
    if let Some((t, w, h)) = &l.staging
        && *w == desc.Width
        && *h == desc.Height
    {
        return Ok(t.clone());
    }
    let want = D3D11_TEXTURE2D_DESC {
        Width: desc.Width,
        Height: desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: desc.Format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let device: ID3D11Device = unsafe { l.context.GetDevice()? };
    let mut out = None;
    unsafe { device.CreateTexture2D(&want, None, Some(&mut out))? };
    let t = out.ok_or_else(|| anyhow!("no staging picture"))?;
    l.staging = Some((t.clone(), desc.Width, desc.Height));
    Ok(t)
}

fn grab(l: &mut Live) -> Result<Option<Image>> {
    let mut newest = None;
    while let Ok(f) = l.pool.TryGetNextFrame() {
        if let Some(old) = newest.replace(f) {
            let _ = old.Close();
        }
    }
    let Some(frame) = newest else { return Ok(None) };
    let content = frame.ContentSize()?;
    let surface = frame.Surface()?;
    let access: IDirect3DDxgiInterfaceAccess = surface.cast()?;
    let tex: ID3D11Texture2D = unsafe { access.GetInterface()? };
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { tex.GetDesc(&mut desc) };
    let stage = staging(l, &desc)?;
    unsafe { l.context.CopyResource(&stage, &tex) };
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { l.context.Map(&stage, 0, D3D11_MAP_READ, 0, Some(&mut m))? };
    let (w, h) = ((content.Width.max(1) as u32).min(desc.Width) as usize, (content.Height.max(1) as u32).min(desc.Height) as usize);
    let pitch = m.RowPitch as usize;
    let mut rgb = vec![0u8; w * h * 3];
    if !m.pData.is_null() && pitch >= w * 4 {
        let src = unsafe { std::slice::from_raw_parts(m.pData as *const u8, pitch * h) };
        for y in 0..h {
            let row = &src[y * pitch..y * pitch + w * 4];
            let out = &mut rgb[y * w * 3..(y + 1) * w * 3];
            for x in 0..w {
                out[x * 3] = row[x * 4 + 2];
                out[x * 3 + 1] = row[x * 4 + 1];
                out[x * 3 + 2] = row[x * 4];
            }
        }
    }
    unsafe { l.context.Unmap(&stage, 0) };
    let _ = frame.Close();
    if content.Width != l.size.Width || content.Height != l.size.Height {
        let _ = l.pool.Recreate(&l.d3d, FORMAT, 2, content);
        l.size = content;
    }
    Ok(Some(Image { w, h, rgb }))
}

fn bounds(hwnd: HWND) -> (f64, f64) {
    let mut r = RECT::default();
    let ok = unsafe { DwmGetWindowAttribute(hwnd, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut core::ffi::c_void, std::mem::size_of::<RECT>() as u32) }.is_ok();
    if ok { (r.left as f64, r.top as f64) } else { (0.0, 0.0) }
}

fn reap() {
    if REAPING.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new().name("setu-wincap".into()).spawn(|| {
        loop {
            std::thread::sleep(Duration::from_secs(2));
            let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
            if g.as_ref().is_none_or(|l| l.used.elapsed() >= LINGER) {
                if let Some(l) = g.take() {
                    close(l);
                }
                REAPING.store(false, Ordering::SeqCst);
                return;
            }
        }
    });
}

pub fn frame() -> Result<Frame> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return Err(anyhow!("No window is in front."));
    }
    let mut g = LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if g.as_ref().is_none_or(|l| l.hwnd != hwnd.0 as isize) {
        if let Some(old) = g.take() {
            close(old);
        }
        *g = Some(open(hwnd)?);
        reap();
    }
    let l = g.as_mut().ok_or_else(|| anyhow!("no window picture"))?;
    l.used = Instant::now();
    let until = Instant::now() + if l.last.is_none() { Duration::from_millis(1500) } else { Duration::ZERO };
    loop {
        if let Some(img) = grab(l)? {
            l.last = Some((NEXT.fetch_add(1, Ordering::SeqCst), Arc::new(img)));
            break;
        }
        if l.last.is_some() || Instant::now() >= until {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let (seq, img) = l.last.clone().ok_or_else(|| anyhow!("The window picture didn't arrive."))?;
    Ok(Frame { img: (*img).clone(), seq, origin: bounds(hwnd) })
}

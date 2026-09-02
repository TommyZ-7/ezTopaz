//! Display / window / audio-device enumeration (Win32).

use super::{err, Result};
use windows::core::ComInterface;
use eztopaz_core::ipc_types::{AppAudio, AudioDevices, DeviceInfo, Display, WindowInfo};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW};
use windows::Win32::Media::Audio::DEVICE_STATE_ACTIVE;
use windows::Win32::Media::Audio::{
    eCapture, eMultimedia, eRender, IAudioSessionControl2, IAudioSessionEnumerator,
    IAudioSessionManager2, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, EDataFlow,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::System::Com::StructuredStorage::{PropVariantToStringAlloc, PROPVARIANT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_NAME_WIN32,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::Win32::Foundation::BOOL;
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::core::GUID;

pub(crate) struct DisplayCtx {
    out: Vec<Display>,
}

struct WindowCtx {
    out: Vec<WindowInfo>,
}

const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

pub fn list_displays() -> Result<Vec<Display>> {
    let mut ctx = DisplayCtx { out: Vec::new() };
    let ctx_ptr = &mut ctx as *mut DisplayCtx as isize;

    unsafe {
        let ok = EnumDisplayMonitors(None, None, Some(enum_displays_cb), LPARAM(ctx_ptr));
        if !ok.as_bool() {
            return Err(err("EnumDisplayMonitors failed"));
        }
    }
    Ok(ctx.out)
}

/// Resolve `monitor:{idx}` to an HMONITOR (used by the WGC backend).
pub(crate) fn monitor_by_index(idx: usize) -> Result<HMONITOR> {
    struct Ctx {
        want: usize,
        cur: usize,
        hmon: Option<HMONITOR>,
    }
    let mut ctx = Ctx { want: idx, cur: 0, hmon: None };
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_by_index_cb),
            LPARAM(&mut ctx as *mut Ctx as isize),
        );
    }
    ctx.hmon.ok_or_else(|| err("monitor not found"))
}

unsafe extern "system" fn enum_monitor_by_index_cb(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    lparam: LPARAM,
) -> BOOL {
    struct ByIdx {
        want: usize,
        cur: usize,
        hmon: Option<HMONITOR>,
    }
    let ctx = unsafe { &mut *(lparam.0 as *mut ByIdx) };
    if ctx.cur == ctx.want {
        ctx.hmon = Some(hmon);
        return false.into();
    }
    ctx.cur += 1;
    true.into()
}

pub fn list_windows() -> Result<Vec<WindowInfo>> {
    let mut ctx = WindowCtx { out: Vec::new() };
    let ctx_ptr = &mut ctx as *mut WindowCtx as isize;

    unsafe {
        let _ = EnumWindows(Some(enum_windows_cb), LPARAM(ctx_ptr));
    }
    Ok(ctx.out)
}

pub(crate) fn process_image_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &mut len,
        ).is_ok();
        let _ = CloseHandle(handle);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/'])
            .next()
            .map(|s| s.trim_end_matches(".exe").to_string())
    }
}

unsafe extern "system" fn enum_displays_cb(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut windows::Win32::Foundation::RECT,
    lparam: LPARAM,
) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut DisplayCtx) };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize =
        u32::try_from(std::mem::size_of::<MONITORINFOEXW>()).unwrap_or(0);
    if GetMonitorInfoW(hmon, &mut info.monitorInfo).as_bool() {
        let w = (info.monitorInfo.rcMonitor.right - info.monitorInfo.rcMonitor.left) as u32;
        let h = (info.monitorInfo.rcMonitor.bottom - info.monitorInfo.rcMonitor.top) as u32;
        let idx = ctx.out.len();
        let label = if info.szDevice.is_empty() {
            format!("Monitor {idx}")
        } else {
            String::from_utf16_lossy(&info.szDevice)
        };
        ctx.out.push(Display { id: format!("monitor:{idx}"), label, w, h });
    }
    true.into()
}

unsafe extern "system" fn enum_windows_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = unsafe { &mut *(lparam.0 as *mut WindowCtx) };
    if !IsWindowVisible(hwnd).as_bool() {
        return true.into();
    }
    let len = GetWindowTextLengthW(hwnd);
    if len <= 0 {
        return true.into();
    }
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, &mut buf);
    let title =
        String::from_utf16_lossy(&buf[..(n as usize).min(buf.len())]).trim().to_string();
    if title.is_empty() {
        return true.into();
    }
    let mut pid = 0u32;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    let app = process_image_name(pid).unwrap_or_default();
    ctx.out.push(WindowInfo { id: format!("hwnd:{}", hwnd.0 as usize), title, app });
    true.into()
}

pub fn list_audio_devices() -> Result<AudioDevices> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(err)?;

        let outputs = devices_for_flow(&enumerator, eRender)?;
        let inputs = devices_for_flow(&enumerator, eCapture)?;

        let mut apps: Vec<AppAudio> = Vec::new();
        if let Ok(dev) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) {
            if let Ok(mgr2) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) {
                if let Ok(list) = mgr2.GetSessionEnumerator() {
                    apps = session_apps(&list);
                }
            }
        }

        Ok(AudioDevices { inputs, outputs, apps })
    }
}

unsafe fn devices_for_flow(
    enumerator: &IMMDeviceEnumerator,
    flow: EDataFlow,
) -> Result<Vec<DeviceInfo>> {
    let mut out = Vec::new();
    let Ok(collection) = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else {
        return Ok(out);
    };
    if let Ok(count) = collection.GetCount() {
        for i in 0..count {
            let Ok(dev) = collection.Item(i) else { continue };
            let id = device_id(&dev);
            let label = device_friendly_name(&dev);
            let default = enumerator
                .GetDefaultAudioEndpoint(flow, eMultimedia)
                .ok()
                .map(|d| device_id(&d) == id)
                .unwrap_or(false);
            out.push(DeviceInfo { id, label, is_default: default });
        }
    }
    Ok(out)
}

unsafe fn session_apps(sessions: &IAudioSessionEnumerator) -> Vec<AppAudio> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Ok(count) = sessions.GetCount() {
        for i in 0..count {
            let Ok(ctl) = sessions.GetSession(i) else { continue };
            let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
            let Ok(pid) = ctl2.GetProcessId() else { continue };
            if pid == 0 || !seen.insert(pid) {
                continue;
            }
            let Some(name) = process_image_name(pid) else { continue };
            out.push(AppAudio { id: format!("pid:{pid}"), label: name });
        }
    }
    out
}

unsafe fn device_id(dev: &IMMDevice) -> String {
    dev.GetId().map(|w| w.to_string().unwrap_or_default()).unwrap_or_default()
}

unsafe fn device_friendly_name(dev: &IMMDevice) -> String {
    let Ok(store) = dev.OpenPropertyStore(STGM_READ) else {
        return String::new();
    };
    let Ok(var) = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME) else {
        return String::new();
    };
    // GetValue returns PROPVARIANT (VT_LPWSTR for this key)
    let var: PROPVARIANT = var;
    let name = unsafe { PropVariantToStringAlloc(&var) }
        .map(|w| w.to_string().unwrap_or_default())
        .unwrap_or_default();
    name
}

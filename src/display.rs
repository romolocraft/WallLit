use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

const MONITORINFOF_PRIMARY: u32 = 1;

const EDD_GET_DEVICE_INTERFACE_NAME: u32 = 0x0000_0001;

#[derive(Clone)]
pub struct Monitor {
    pub device: String,

    pub id: String,
    pub rect: RECT,
    pub refresh_hz: u32,
    pub primary: bool,
}

impl Monitor {
    pub fn width(&self) -> u32 {
        (self.rect.right - self.rect.left) as u32
    }

    pub fn height(&self) -> u32 {
        (self.rect.bottom - self.rect.top) as u32
    }
}

fn from_wide(buffer: &[u16]) -> String {
    let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

pub fn normalize_device_path(path: &str) -> Option<String> {
    let core = path
        .strip_prefix(r"\\?\")?
        .rsplit_once('#')
        .map(|(head, _)| head)?;

    if core.is_empty() {
        return None;
    }

    Some(core.replace('#', "-"))
}

fn durable_id(gdi_device: &str, fallback_index: usize) -> String {
    let mut info = DISPLAY_DEVICEW {
        cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
        ..Default::default()
    };

    let wide: Vec<u16> = gdi_device.encode_utf16().chain(std::iter::once(0)).collect();

    let ok = unsafe {
        EnumDisplayDevicesW(
            PCWSTR(wide.as_ptr()),
            0,
            &mut info,
            EDD_GET_DEVICE_INTERFACE_NAME,
        )
    };

    if ok.as_bool() {
        let device_id = from_wide(&info.DeviceID);
        if let Some(id) = normalize_device_path(&device_id) {
            return id;
        }
        if !device_id.is_empty() {
            return device_id;
        }
    }

    if gdi_device.is_empty() {
        format!("DISPLAY{fallback_index}")
    } else {
        gdi_device.trim_start_matches(r"\\.\").to_string()
    }
}

unsafe extern "system" fn monitor_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let out = &mut *(lparam.0 as *mut Vec<Monitor>);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmon, &mut info.monitorInfo as *mut MONITORINFO).as_bool() {
        let device = from_wide(&info.szDevice);

        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        let refresh_hz = if EnumDisplaySettingsW(
            PCWSTR(info.szDevice.as_ptr()),
            ENUM_CURRENT_SETTINGS,
            &mut mode,
        )
        .as_bool()
        {
            mode.dmDisplayFrequency
        } else {
            60
        };

        let id = durable_id(&device, out.len());

        out.push(Monitor {
            device,
            id,
            rect: info.monitorInfo.rcMonitor,
            refresh_hz,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        });
    }

    TRUE
}

pub fn enumerate() -> Vec<Monitor> {
    let mut out: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(monitor_proc),
            LPARAM(&mut out as *mut Vec<Monitor> as isize),
        );
    }

    out.sort_by_key(|m| (!m.primary, m.rect.left, m.rect.top));
    out
}

pub fn virtual_screen() -> RECT {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        RECT {
            left: x,
            top: y,
            right: x + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            bottom: y + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        }
    }
}

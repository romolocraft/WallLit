use windows::core::*;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    RegisterPowerSettingNotification, UnregisterPowerSettingNotification, HPOWERNOTIFY,
    POWERBROADCAST_SETTING,
    GetSystemPowerStatus, SYSTEM_POWER_STATUS,
};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::System::SystemServices::{GUID_SESSION_DISPLAY_STATUS, GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS};
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
const WTS_SESSION_LOCK: usize = 0x7;
const WTS_SESSION_UNLOCK: usize = 0x8;

const PBT_APMSUSPEND: usize = 0x0004;
const PBT_APMRESUMESUSPEND: usize = 0x0007;
const PBT_APMRESUMEAUTOMATIC: usize = 0x0012;
const PBT_POWERSETTINGCHANGE: usize = 0x8013;

const DISPLAY_OFF: u8 = 0;

const WINDOW_CLASS: PCWSTR = w!("WallLitSession");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SessionState {
    pub locked: bool,
    pub display_off: bool,
    pub suspended: bool,
    pub on_battery: bool,
    pub energy_saver: bool,
    pub display_changed: bool,
    pub clock_changed: bool,
}

impl SessionState {
    pub fn is_dormant(&self) -> bool {
        self.locked || self.display_off || self.suspended
    }
}

pub struct SessionWatcher {
    hwnd: HWND,

    state: Box<SessionState>,
    power_handles: Vec<HPOWERNOTIFY>,
}

impl SessionWatcher {
    pub fn new() -> Result<Self> {
        let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }?;

        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };

        unsafe { RegisterClassExW(&class) };

        let mut state = Box::new(SessionState::default());
        update_power(&mut state);

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                WINDOW_CLASS,
                w!("WallLit"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                None,
                None,
                instance,
                None,
            )
        }?;

        unsafe {
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state.as_mut() as *mut SessionState as isize);
        }

        if let Err(e) = unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } {
            eprintln!("no session lock notification: {}", e.message());
        }

        let mut power_handles = Vec::new();
        for setting in [GUID_SESSION_DISPLAY_STATUS, GUID_ACDC_POWER_SOURCE, GUID_POWER_SAVING_STATUS] {
        match unsafe {
            RegisterPowerSettingNotification(
                windows::Win32::Foundation::HANDLE(hwnd.0),
                &setting,
                windows::Win32::UI::WindowsAndMessaging::DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        } {
            Ok(handle) => power_handles.push(handle),
            Err(e) => {
                eprintln!("no display off notification: {}", e.message());
            }
        }
        }

        Ok(Self { hwnd, state, power_handles })
    }

    pub fn state(&self) -> SessionState {
        *self.state
    }

    pub fn take_changes(&mut self) -> (bool, bool) {
        (std::mem::take(&mut self.state.display_changed), std::mem::take(&mut self.state.clock_changed))
    }
}

impl Drop for SessionWatcher {
    fn drop(&mut self) {
        unsafe {
            for handle in &self.power_handles {
                let _ = UnregisterPowerSettingNotification(*handle);
            }
            let _ = WTSUnRegisterSessionNotification(self.hwnd);

            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SessionState;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let state = &mut *state;

    match msg {
        WM_DISPLAYCHANGE => { state.display_changed = true; LRESULT(0) }
        WM_TIMECHANGE => { state.clock_changed = true; LRESULT(0) }
        WM_WTSSESSION_CHANGE => {
            match w.0 {
                WTS_SESSION_LOCK => state.locked = true,
                WTS_SESSION_UNLOCK => { state.locked = false; state.clock_changed = true; }
                _ => {}
            }
            LRESULT(0)
        }

        WM_POWERBROADCAST => {
            match w.0 {
                PBT_APMSUSPEND => state.suspended = true,
                PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => {
                    state.suspended = false;
                    state.clock_changed = true;
                    state.display_changed = true;
                    update_power(state);
                }
                0x000A => update_power(state),
                PBT_POWERSETTINGCHANGE => {
                    let setting = &*(l.0 as *const POWERBROADCAST_SETTING);
                    if setting.PowerSetting == GUID_SESSION_DISPLAY_STATUS
                        && setting.DataLength >= 1
                    {
                        state.display_off = setting.Data[0] == DISPLAY_OFF;
                        if !state.display_off { state.clock_changed = true; }
                    }
                    if setting.PowerSetting == GUID_ACDC_POWER_SOURCE || setting.PowerSetting == GUID_POWER_SAVING_STATUS {
                        update_power(state);
                    }
                }
                _ => {}
            }

            LRESULT(1)
        }

        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

fn update_power(state: &mut SessionState) {
    let mut power = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut power) }.is_ok() {
        state.on_battery = power.ACLineStatus == 0;
        state.energy_saver = power.SystemStatusFlag == 1;
    }
}

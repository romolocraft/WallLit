use std::path::PathBuf;

use windows::core::*;
use windows::Win32::Foundation::{E_FAIL, HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::*;

const WINDOW_CLASS: PCWSTR = w!("WallLitTray");

const WM_TRAY_CALLBACK: u32 = WM_APP + 1;

const APP_ICON_RESOURCE: usize = 1;

const ID_SETTINGS: usize = 1;
const ID_PAUSE: usize = 2;
const ID_QUIT: usize = 3;
const ID_NEXT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    OpenSettings,
    TogglePause,
    NextWallpaper,
    Quit,
}

struct TrayState {
    pending: Option<TrayCommand>,
    paused: bool,

    taskbar_created: u32,
}

pub struct Tray {
    hwnd: HWND,

    state: Box<TrayState>,
}

impl Tray {
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

        let taskbar_created = unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) };

        let mut state = Box::new(TrayState {
            pending: None,
            paused: false,
            taskbar_created,
        });

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
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, state.as_mut() as *mut TrayState as isize);
        }

        let tray = Self { hwnd, state };
        tray.add_icon()?;
        Ok(tray)
    }

    fn add_icon(&self) -> Result<()> {
        let data = notify_data(self.hwnd, self.state.paused);
        unsafe { Shell_NotifyIconW(NIM_ADD, &data) }
            .ok()
            .map_err(|_| Error::new(E_FAIL, "could not create the tray icon"))
    }

    pub fn take_command(&mut self) -> Option<TrayCommand> {
        self.state.pending.take()
    }

    pub fn set_paused(&mut self, paused: bool) {
        if self.state.paused == paused {
            return;
        }

        self.state.paused = paused;
        let data = notify_data(self.hwnd, paused);
        unsafe { let _ = Shell_NotifyIconW(NIM_MODIFY, &data); }
    }
}

fn notify_data(hwnd: HWND, paused: bool) -> NOTIFYICONDATAW {
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }.unwrap_or_default();
    let icon =
        unsafe { LoadIconW(instance, PCWSTR(APP_ICON_RESOURCE as *const u16)) }.unwrap_or_default();

    let mut data = NOTIFYICONDATAW {
        cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
        hWnd: hwnd,
        uID: 1,
        uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon,
        ..Default::default()
    };

    let tooltip = if paused {
        "WallLit - paused"
    } else {
        "WallLit"
    };
    for (slot, character) in data.szTip.iter_mut().zip(tooltip.encode_utf16()) {
        *slot = character;
    }

    data
}

impl Drop for Tray {
    fn drop(&mut self) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd,
            uID: 1,
            ..Default::default()
        };
        unsafe {
            let _ = Shell_NotifyIconW(NIM_DELETE, &mut data);
            SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn show_menu(hwnd: HWND, paused: bool) {
    let Ok(menu) = (unsafe { CreatePopupMenu() }) else { return };

    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, ID_SETTINGS, w!("Open WallLit"));
        let _ = SetMenuDefaultItem(menu, ID_SETTINGS as u32, 0);
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

        let pause_label = if paused {
            w!("Resume wallpaper")
        } else {
            w!("Pause wallpaper")
        };
        let _ = AppendMenuW(menu, MF_STRING, ID_PAUSE, pause_label);
        let _ = AppendMenuW(menu, MF_STRING, ID_NEXT, w!("Next wallpaper"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, ID_QUIT, w!("Quit"));

        let mut cursor = POINT::default();
        let _ = GetCursorPos(&mut cursor);

        let _ = SetForegroundWindow(hwnd);

        let _ = TrackPopupMenuEx(
            menu,
            (TPM_RIGHTBUTTON | TPM_RIGHTALIGN | TPM_BOTTOMALIGN).0,
            cursor.x,
            cursor.y,
            hwnd,
            None,
        );

        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));
        let _ = DestroyMenu(menu);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let state = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut TrayState;
    if state.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let state = &mut *state;

    if msg == state.taskbar_created {
        let data = notify_data(hwnd, state.paused);
        let _ = Shell_NotifyIconW(NIM_ADD, &data);
        return LRESULT(0);
    }

    match msg {
        WM_TRAY_CALLBACK => {
            match l.0 as u32 {
                WM_LBUTTONDBLCLK => state.pending = Some(TrayCommand::OpenSettings),
                WM_RBUTTONUP | WM_CONTEXTMENU => show_menu(hwnd, state.paused),
                _ => {}
            }
            LRESULT(0)
        }

        WM_COMMAND => {
            state.pending = match w.0 & 0xFFFF {
                ID_SETTINGS => Some(TrayCommand::OpenSettings),
                ID_PAUSE => Some(TrayCommand::TogglePause),
                ID_NEXT => Some(TrayCommand::NextWallpaper),
                ID_QUIT => Some(TrayCommand::Quit),
                _ => None,
            };
            LRESULT(0)
        }

        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

pub fn settings_path() -> Option<PathBuf> {
    let current = std::env::current_exe().ok()?;
    Some(current.parent()?.join("walllit-settings.exe"))
}

pub fn open_settings() {
    let Some(path) = settings_path() else { return };
    let _ = std::process::Command::new(path).spawn();
}

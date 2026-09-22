use std::ffi::c_void;

use windows::Win32::Foundation::{BOOL, FALSE, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

const WATCHED_EVENTS: &[(u32, u32)] = &[
    (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
    (EVENT_SYSTEM_MOVESIZEEND, EVENT_SYSTEM_MOVESIZEEND),
    (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),

    (EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE),
    (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
];

static mut LAYOUT_CHANGED: bool = true;

unsafe extern "system" fn event_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object: i32,
    child: i32,
    _thread: u32,
    _time: u32,
) {
    const OBJID_WINDOW: i32 = 0;
    const CHILDID_SELF: i32 = 0;

    if object != OBJID_WINDOW || child != CHILDID_SELF || hwnd.0.is_null() {
        return;
    }

    LAYOUT_CHANGED = true;
}

pub struct OcclusionWatcher {
    hooks: Vec<HWINEVENTHOOK>,
}

impl OcclusionWatcher {
    pub fn new() -> Self {
        let mut hooks = Vec::new();

        for (first, last) in WATCHED_EVENTS {
            let hook = unsafe {
                SetWinEventHook(
                    *first,
                    *last,
                    None,
                    Some(event_proc),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
                )
            };

            if !hook.is_invalid() {
                hooks.push(hook);
            }
        }

        Self { hooks }
    }

    pub fn take_changed(&self) -> bool {
        unsafe {
            let changed = LAYOUT_CHANGED;
            LAYOUT_CHANGED = false;
            changed
        }
    }

    pub fn invalidate(&self) {
        unsafe { LAYOUT_CHANGED = true };
    }
}

impl Drop for OcclusionWatcher {
    fn drop(&mut self) {
        for hook in self.hooks.drain(..) {
            unsafe {
                let _ = UnhookWinEvent(hook);
            }
        }
    }
}

struct Scan<'a> {
    areas: &'a [RECT],

    covering: Vec<Option<HWND>>,
}

fn is_shell_window(hwnd: HWND) -> bool {
    let mut buffer = [0u16; 64];
    let length = unsafe { GetClassNameW(hwnd, &mut buffer) };
    let class = String::from_utf16_lossy(&buffer[..length as usize]);

    matches!(
        class.as_str(),
        "Progman"
            | "WorkerW"
            | "Shell_TrayWnd"
            | "Shell_SecondaryTrayWnd"
            | "SHELLDLL_DefView"
            | "SysListView32"
            | "WallLitSurface"
    )
}

fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let result = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        )
    };

    result.is_ok() && cloaked != 0
}

fn is_see_through(hwnd: HWND) -> bool {
    const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
    const WS_EX_LAYERED: u32 = 0x0008_0000;

    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    ex_style & (WS_EX_LAYERED | WS_EX_TRANSPARENT) != 0
}

fn contains(outer: &RECT, inner: &RECT) -> bool {
    outer.left <= inner.left
        && outer.top <= inner.top
        && outer.right >= inner.right
        && outer.bottom >= inner.bottom
}

unsafe extern "system" fn scan_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let scan = &mut *(lparam.0 as *mut Scan);

    if scan.covering.iter().all(Option::is_some) {
        return FALSE;
    }

    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return TRUE;
    }

    let mut rect = RECT::default();
    if GetWindowRect(hwnd, &mut rect).is_err() {
        return TRUE;
    }

    let hits: Vec<usize> = scan
        .areas
        .iter()
        .enumerate()
        .filter(|(index, area)| scan.covering[*index].is_none() && contains(&rect, area))
        .map(|(index, _)| index)
        .collect();

    if hits.is_empty() {
        return TRUE;
    }

    if is_shell_window(hwnd) || is_see_through(hwnd) || is_cloaked(hwnd) {
        return TRUE;
    }

    for index in hits {
        scan.covering[index] = Some(hwnd);
    }

    TRUE
}

pub fn covered(areas: &[RECT]) -> Vec<bool> {
    coverage(areas).iter().map(Option::is_some).collect()
}

fn coverage(areas: &[RECT]) -> Vec<Option<HWND>> {
    let mut scan = Scan { areas, covering: vec![None; areas.len()] };

    unsafe {
        let _ = EnumWindows(Some(scan_proc), LPARAM(&mut scan as *mut Scan as isize));
    }

    scan.covering
}

pub fn describe(areas: &[RECT]) -> Vec<Option<String>> {
    coverage(areas)
        .into_iter()
        .map(|window| {
            let hwnd = window?;

            let mut class = [0u16; 64];
            let class_length = unsafe { GetClassNameW(hwnd, &mut class) };

            let mut title = [0u16; 128];
            let title_length = unsafe { GetWindowTextW(hwnd, &mut title) };

            let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;

            Some(format!(
                "{} \"{}\" ex=0x{:08x}",
                String::from_utf16_lossy(&class[..class_length as usize]),
                String::from_utf16_lossy(&title[..title_length as usize]),
                ex_style
            ))
        })
        .collect()
}

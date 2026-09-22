use windows::core::*;
use windows::Win32::Foundation::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_SPAWN_WORKERW: u32 = 0x052C;

fn class_name_of(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..n as usize])
}

fn spawn_worker_layer(progman: HWND) {
    for (w, l) in [(0usize, 0isize), (0x0D, 0x01), (0x0D, 0x00)] {
        unsafe {
            SendMessageTimeoutW(
                progman,
                WM_SPAWN_WORKERW,
                WPARAM(w),
                LPARAM(l),
                SMTO_NORMAL | SMTO_ABORTIFHUNG,
                120,
                None,
            );
        }
    }
}

struct Search {
    behind_icons: Option<HWND>,

    icon_host: Option<HWND>,
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let search = &mut *(lparam.0 as *mut Search);

    let has_icons =
        FindWindowExW(hwnd, HWND::default(), w!("SHELLDLL_DefView"), PCWSTR::null()).is_ok();

    if has_icons {
        search.icon_host = Some(hwnd);

        if let Ok(worker) = FindWindowExW(HWND::default(), hwnd, w!("WorkerW"), PCWSTR::null()) {
            search.behind_icons = Some(worker);
            return BOOL(0);
        }
    }

    BOOL(1)
}

pub struct DesktopAnchor {
    pub parent: HWND,

    pub kind: &'static str,
}

fn child_worker_w(parent: HWND) -> Option<HWND> {
    unsafe { FindWindowExW(parent, HWND::default(), w!("WorkerW"), PCWSTR::null()).ok() }
        .filter(|h| !h.0.is_null())
}

fn locate(progman: HWND) -> Option<DesktopAnchor> {
    if let Some(parent) = child_worker_w(progman) {
        return Some(DesktopAnchor { parent, kind: "WorkerW/child" });
    }

    let mut search = Search { behind_icons: None, icon_host: None };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize));
    }

    if let Some(parent) = search.behind_icons {
        return Some(DesktopAnchor { parent, kind: "WorkerW/sibling" });
    }

    if let Some(parent) = search.icon_host.and_then(child_worker_w) {
        return Some(DesktopAnchor { parent, kind: "WorkerW/nested" });
    }

    None
}

pub fn find_anchor() -> Result<DesktopAnchor> {
    let progman = unsafe { FindWindowW(w!("Progman"), PCWSTR::null()) }?;

    if let Some(anchor) = locate(progman) {
        return Ok(anchor);
    }

    spawn_worker_layer(progman);
    if let Some(anchor) = locate(progman) {
        return Ok(anchor);
    }

    Ok(DesktopAnchor { parent: progman, kind: "Progman" })
}

pub fn describe_tree(out: &mut String) {
    unsafe {
        let progman = FindWindowW(w!("Progman"), PCWSTR::null());
        out.push_str(&format!(
            "Progman: {:?}
",
            progman.as_ref().ok().map(|h| h.0)
        ));

        if let Ok(pm) = progman {
            spawn_worker_layer(pm);

            out.push_str("
--- filhos do Progman ---
");
            describe_children(pm, 0, out);
        }

        let mut search = Search { behind_icons: None, icon_host: None };
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut search as *mut Search as isize));

        out.push_str(&format!(
            "
icon_host    : {:?}
behind_icons : {:?}
",
            search.icon_host.map(|h| h.0),
            search.behind_icons.map(|h| h.0)
        ));
    }
}

pub fn describe_children(parent: HWND, depth: usize, out: &mut String) {
    unsafe {
        let mut child = HWND::default();
        loop {
            match FindWindowExW(parent, child, PCWSTR::null(), PCWSTR::null()) {
                Ok(h) if !h.0.is_null() => {
                    let mut r = RECT::default();
                    let _ = GetWindowRect(h, &mut r);
                    out.push_str(&format!(
                        "{:indent$}{} hwnd={:?} visible={} rect=({},{})-({},{})
",
                        "",
                        class_name_of(h),
                        h.0,
                        IsWindowVisible(h).as_bool(),
                        r.left,
                        r.top,
                        r.right,
                        r.bottom,
                        indent = depth * 2
                    ));
                    if depth < 2 {
                        describe_children(h, depth + 1, out);
                    }
                    child = h;
                }
                _ => break,
            }
        }
    }
}

use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, BLACK_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::Graphics::Gdi::ScreenToClient;

const WINDOW_CLASS: PCWSTR = w!("WallLitSurface");

fn ensure_class() -> Result<()> {
    use std::sync::OnceLock;
    static REGISTERED: OnceLock<()> = OnceLock::new();

    let mut result = Ok(());
    REGISTERED.get_or_init(|| unsafe {
        let instance = match GetModuleHandleW(PCWSTR::null()) {
            Ok(i) => i,
            Err(e) => {
                result = Err(e);
                return;
            }
        };

        let class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance.into(),

            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
            lpszClassName: WINDOW_CLASS,
            ..Default::default()
        };

        if RegisterClassExW(&class) == 0 {
            result = Err(Error::from_win32());
        }
    });

    result
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }

        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        WM_ERASEBKGND => LRESULT(1),
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

pub fn create_window(parent: HWND, screen_rect: RECT) -> Result<HWND> {
    ensure_class()?;

    let mut origin = POINT { x: screen_rect.left, y: screen_rect.top };
    unsafe {
        let _ = ScreenToClient(parent, &mut origin);
    }

    let width = screen_rect.right - screen_rect.left;
    let height = screen_rect.bottom - screen_rect.top;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
            WINDOW_CLASS,
            w!("WallLit"),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
            origin.x,
            origin.y,
            width,
            height,
            parent,
            None,
            GetModuleHandleW(PCWSTR::null())?,
            None,
        )
    }?;

    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND_BOTTOM,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }

    Ok(hwnd)
}

pub fn describe(hwnd: HWND) {
    unsafe {
        let mut rect = RECT::default();
        let _ = GetWindowRect(hwnd, &mut rect);
        println!(
            "window   : hwnd={:?} visible={} rect=({},{})-({},{})",
            hwnd.0,
            IsWindowVisible(hwnd).as_bool(),
            rect.left,
            rect.top,
            rect.right,
            rect.bottom
        );

        let mut current = hwnd;
        let mut depth = 0;
        while let Ok(parent) = GetParent(current) {
            if parent.0.is_null() || depth > 6 {
                break;
            }
            let mut r = RECT::default();
            let _ = GetWindowRect(parent, &mut r);
            println!(
                "  parent {} : {} hwnd={:?} rect=({},{})-({},{})",
                depth,
                class_name_of(parent),
                parent.0,
                r.left,
                r.top,
                r.right,
                r.bottom
            );
            current = parent;
            depth += 1;
        }
    }
}

pub fn is_alive(hwnd: HWND) -> bool {
    !hwnd.0.is_null() && unsafe { IsWindow(hwnd) }.as_bool()
}

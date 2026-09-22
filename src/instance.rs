use windows::core::*;
use windows::Win32::Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, HANDLE, WIN32_ERROR};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenEventW, SetEvent, EVENT_MODIFY_STATE,
};

const MUTEX_NAME: PCWSTR = w!(r"Local\WallLitEngine");
const RELOAD_EVENT_NAME: PCWSTR = w!(r"Local\WallLitReload");
const QUIT_EVENT_NAME: PCWSTR = w!(r"Local\WallLitQuit");
const SHUTDOWN_EVENT_NAME: PCWSTR = w!(r"Local\WallLitShutdown");

pub struct EngineLock {
    handle: HANDLE,
}

impl EngineLock {
    pub fn acquire() -> Option<Self> {
        Self::named(MUTEX_NAME)
    }

    pub fn settings() -> Option<Self> { Self::named(w!(r"Local\WallLitSettingsLock")) }

    fn named(name: PCWSTR) -> Option<Self> {
        let handle = unsafe { CreateMutexW(None, true, name) }.ok()?;

        let already_running =
            WIN32_ERROR(unsafe { windows::Win32::Foundation::GetLastError().0 }) == ERROR_ALREADY_EXISTS;

        if already_running {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return None;
        }

        Some(Self { handle })
    }
}

impl Drop for EngineLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

pub struct Signal {
    handle: HANDLE,
}

impl Signal {
    pub fn handle(&self) -> HANDLE {
        self.handle
    }
}

impl Drop for Signal {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

fn create(name: PCWSTR) -> Result<Signal> {
    let handle = unsafe { CreateEventW(None, false, false, name) }?;
    Ok(Signal { handle })
}

fn raise(name: PCWSTR) -> bool {
    let Ok(handle) = (unsafe { OpenEventW(EVENT_MODIFY_STATE, false, name) }) else {
        return false;
    };

    let signalled = unsafe { SetEvent(handle) }.is_ok();
    unsafe {
        let _ = CloseHandle(handle);
    }

    signalled
}

pub fn reload_signal() -> Result<Signal> {
    create(RELOAD_EVENT_NAME)
}

pub fn quit_signal() -> Result<Signal> {
    create(QUIT_EVENT_NAME)
}

pub fn request_reload() -> bool {
    raise(RELOAD_EVENT_NAME)
}

pub fn request_quit() -> bool {
    raise(QUIT_EVENT_NAME)
}

pub fn shutdown_signal() -> Result<Signal> {
    create(SHUTDOWN_EVENT_NAME)
}

pub fn request_shutdown() -> bool {
    raise(SHUTDOWN_EVENT_NAME)
}

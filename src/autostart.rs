use std::path::PathBuf;

use windows::core::*;
use windows::Win32::Foundation::{E_FAIL, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");
const VALUE_NAME: PCWSTR = w!("WallLit");

pub fn engine_path() -> Result<PathBuf> {
    let current = std::env::current_exe().map_err(|e| Error::new(E_FAIL, format!("{e}")))?;

    let directory = current
        .parent()
        .ok_or_else(|| Error::new(E_FAIL, "executavel sem diretorio"))?;

    Ok(directory.join("walllit.exe"))
}

fn command_line() -> Result<HSTRING> {
    Ok(HSTRING::from(format!("\"{}\"", engine_path()?.display())))
}

fn current_command() -> Option<String> {
    let mut buffer = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buffer) as u32;

    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    };

    if status != ERROR_SUCCESS {
        return None;
    }

    let characters = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..characters]))
}

pub fn is_enabled() -> bool {
    current_command().is_some()
}

pub fn enable() -> Result<()> {
    let command = command_line()?;

    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            VALUE_NAME,
            REG_SZ.0,
            Some(command.as_ptr() as *const _),

            ((command.len() + 1) * 2) as u32,
        )
    };

    if status != ERROR_SUCCESS {
        return Err(Error::from(status.to_hresult()));
    }

    Ok(())
}

pub fn disable() -> Result<()> {
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE_NAME) };

    if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
        return Err(Error::from(status.to_hresult()));
    }

    Ok(())
}

pub fn set(enabled: bool) -> Result<()> {
    if enabled {
        enable()
    } else {
        disable()
    }
}

pub fn repair_if_stale() {
    let Some(current) = current_command() else { return };
    let Ok(expected) = command_line() else { return };

    if current != expected.to_string() {
        let _ = enable();
    }
}

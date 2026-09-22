use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub fn backup(path: &Path) -> std::io::Result<std::path::PathBuf> {
    let bytes = std::fs::read(path)?;
    loop {
        let destination = path.with_extension(format!("broken.{}.{}.json", std::process::id(), NEXT_TEMP.fetch_add(1, Ordering::Relaxed)));
        match std::fs::OpenOptions::new().create_new(true).write(true).open(&destination) {
            Ok(mut file) => { file.write_all(&bytes)?; file.sync_all()?; return Ok(destination); }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> std::io::Result<()> {
    let text = serde_json::to_vec_pretty(value)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let (temp, mut file) = loop {
        let temp = path.with_extension(format!(
            "json.{}.{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        ));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
        {
            Ok(file) => break (temp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    };
    let result = (|| {
        file.write_all(&text)?;
        file.write_all(b"\n")?;
        file.sync_all()
    })();
    drop(file);
    let result = result.and_then(|()| {
        let from = HSTRING::from(temp.as_os_str());
        let to = HSTRING::from(path.as_os_str());
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|e| std::io::Error::other(e.message()))
    });
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_complete_json_and_cleans_temporary_file() {
        let dir = std::env::temp_dir().join(format!("walllit-storage-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("config.json");
        write_json(&path, &vec![1, 2, 3]).unwrap();
        write_json(&path, &vec![4]).unwrap();
        let actual: Vec<u32> = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(actual, [4]);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn recovery_keeps_original_bytes_in_unique_backups() {
        let dir = std::env::temp_dir().join(format!("walllit-backup-test-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("broken.json");
        std::fs::write(&path, b"{broken").unwrap();
        let first = backup(&path).unwrap();
        let second = backup(&path).unwrap();
        assert_ne!(first, second);
        write_json(&path, &vec![1]).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), b"{broken");
        for file in [path, first, second] { std::fs::remove_file(file).unwrap(); }
        std::fs::remove_dir(dir).unwrap();
    }
}

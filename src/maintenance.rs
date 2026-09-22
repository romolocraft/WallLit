use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use crate::{config::Config, library::Library};

#[derive(Clone, Debug)]
pub struct Candidate { pub path: PathBuf, pub bytes: u64 }
pub struct Usage { pub bytes: u64, pub candidates: Vec<Candidate> }

fn key(path: &Path) -> String {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf()).to_string_lossy().to_lowercase()
}

pub fn scan(draft: &Config) -> std::io::Result<Usage> {
    let disk = Config::read_or_default()?;
    let library = Library::try_load()?;
    let root = crate::config::data_dir().map_err(|e| std::io::Error::other(e.message()))?;
    scan_at(&root, &[&disk, draft], &library, 86400)
}

fn scan_at(root: &Path, configs: &[&Config], library: &Library, minimum_age_seconds: u64) -> std::io::Result<Usage> {
    let mut referenced = BTreeSet::new();
    for config in configs {
        for monitor in config.monitors.values() { for slide in &monitor.slides {
            referenced.insert(key(&slide.wallpaper));
            if let Some(source) = &slide.source { referenced.insert(key(source)); }
        } }
    }
    for item in library.items() {
        referenced.insert(key(&item.prepared));
        referenced.insert(key(&item.thumbnail));
        if let Some(source) = &item.source { referenced.insert(key(source)); }
    }
    let mut usage = Usage { bytes: 0, candidates: Vec::new() };
    std::fs::create_dir_all(root)?;
    let root = root.canonicalize()?;
    for name in ["wallpapers", "thumbs"] {
        let directory = root.join(name);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let canonical = directory.canonicalize()?;
        if canonical.parent() != Some(root.as_path()) { continue; }
        for entry in entries {
            let entry = entry?;
            if !entry.file_type()?.is_file() { continue; }
            let path = entry.path();
            let resolved = path.canonicalize()?;
            if resolved.parent() != Some(canonical.as_path()) { continue; }
            let metadata = entry.metadata()?;
            usage.bytes = usage.bytes.saturating_add(metadata.len());

            let old = metadata.modified()?.elapsed().is_ok_and(|age| age.as_secs() >= minimum_age_seconds)
                && metadata.created()?.elapsed().is_ok_and(|age| age.as_secs() >= minimum_age_seconds);
            if old && !referenced.contains(&key(&path)) {
                usage.candidates.push(Candidate { path, bytes: metadata.len() });
            }
        }
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protects_disconnected_monitors_library_and_recent_files() {
        let root = std::env::temp_dir().join(format!("walllit-maintenance-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let media = root.join("wallpapers");
        std::fs::create_dir(&media).unwrap();
        let used = media.join("used.mp4");
        let orphan = media.join("orphan.mp4");
        let indexed = media.join("indexed.mp4");
        for path in [&used, &orphan, &indexed] { std::fs::write(path, b"fixture").unwrap(); }
        let mut config = Config::default();
        config.monitors.insert("disconnected-monitor".into(), crate::config::MonitorConfig::from_slides(vec![crate::config::Slide { wallpaper: used.clone(), ..Default::default() }]));
        let library: Library = serde_json::from_value(serde_json::json!({"items":[{"name":"indexed","prepared":indexed,"thumbnail":"none.jpg","width":1,"height":1,"fps":0}]})).unwrap();
        let usage = scan_at(&root, &[&config], &library, 0).unwrap();
        assert_eq!(usage.candidates.len(), 1);
        assert_eq!(usage.candidates[0].path.file_name(), orphan.file_name());
        assert!(scan_at(&root, &[&config], &library, 86400).unwrap().candidates.is_empty());
        for path in [used, orphan, indexed] { std::fs::remove_file(path).unwrap(); }
        std::fs::remove_dir(media).unwrap(); std::fs::remove_dir(root).unwrap();
    }
}

pub fn clean(reviewed: &[Candidate], draft: &Config) -> std::io::Result<u64> {
    let current = scan(draft)?;
    let mut removed = 0;
    for candidate in reviewed {
        if current.candidates.iter().any(|c| c.path == candidate.path && c.bytes == candidate.bytes) {
            std::fs::remove_file(&candidate.path)?;
            removed += candidate.bytes;
        }
    }
    Ok(removed)
}

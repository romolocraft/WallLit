use std::io::Write;

pub fn record(message: &str) {
    eprintln!("{message}");
    let Ok(dir) = crate::config::data_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() { return; }
    let path = dir.join("diagnostic.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 256 * 1024) {
        let _ = std::fs::remove_file(dir.join("diagnostic.previous.log"));
        let _ = std::fs::rename(&path, dir.join("diagnostic.previous.log"));
    }
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let _ = writeln!(file, "{stamp} [{}] {}", std::process::id(), message.chars().take(2000).collect::<String>());
    }
}

pub fn publish(state: &str, previous: &mut String) {
    if state == previous { return; }
    if let Ok(dir) = crate::config::data_dir() {
        if crate::storage::write_json(&dir.join("engine-state.json"), &state).is_ok() {
            *previous = state.to_owned();
        }
    }
}

pub fn report() -> String {
    let Ok(dir) = crate::config::data_dir() else { return "Data unavailable".into() };
    let state = std::fs::read_to_string(dir.join("engine-state.json")).ok()
        .and_then(|s| serde_json::from_str::<String>(&s).ok()).unwrap_or_else(|| "Engine has not published any state.".into());
    let logs = std::fs::read_to_string(dir.join("diagnostic.log")).unwrap_or_default();
    let recent: Vec<_> = logs.lines().rev().take(8).collect();
    format!("Last known state (may belong to a run that has ended):\n{state}\n\nRecent events:\n{}\n\nFolder: {}", recent.into_iter().rev().collect::<Vec<_>>().join("\n"), dir.display())
}

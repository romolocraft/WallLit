use super::*;

pub(super) fn show_info(hwnd: HWND, message: &str) {
    unsafe { MessageBoxW(hwnd, &HSTRING::from(message), w!("WallLit"), MB_OK | MB_ICONINFORMATION); }
}

pub(super) fn recover_read<T: Default + serde::Serialize>(hwnd: HWND, path: &Path, read: impl Fn() -> std::io::Result<T>) -> Result<T> {
    match read() {
        Ok(value) => Ok(value),
        Err(e) => {
            let text = format!("Could not read {}:\n{e}\n\nKeep a copy of the damaged file and start with empty data?\nChoose No to exit without changing the file.", path.display());
            if unsafe { MessageBoxW(hwnd, &HSTRING::from(text), w!("Recover data — WallLit"), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) } != IDYES {
                return Err(poster::to_windows_error(e));
            }
            let backup = walllit::storage::backup(path).map_err(poster::to_windows_error)?;
            let value = T::default();
            walllit::storage::write_json(path, &value).map_err(poster::to_windows_error)?;
            show_info(hwnd, &format!("Original file kept at:\n{}", backup.display()));
            Ok(value)
        }
    }
}

impl App {
    pub(super) fn draft(&self) -> Config {
        let mut config = self.config.clone();
        for tab in &self.tabs { config.monitors.insert(tab.monitor.id.clone(), tab.entry.clone()); }
        config
    }

    pub(super) fn refresh_displays(&mut self) {
        let selected = self.tabs.get(self.selected).map(|tab| tab.monitor.id.clone());
        self.config = self.draft();
        self.tabs = display::enumerate().into_iter().map(|monitor| {
            let entry = self.config.monitors.get(&monitor.id).cloned().unwrap_or_default();
            Tab { monitor, entry, slide: 0 }
        }).collect();
        self.selected = selected.and_then(|id| self.tabs.iter().position(|t| t.monitor.id == id)).unwrap_or(0);
        self.preview = None;
        self.close_picker();
        self.ui.dirty = true;
    }

    pub(super) fn cleanup(&mut self) {
        if self.job.is_some() { show_info(self.hwnd, "Wait for the import to finish before cleaning the library."); return; }
        let draft = self.draft();
        let usage = match walllit::maintenance::scan(&draft) {
            Ok(usage) => usage,
            Err(e) => { show_info(self.hwnd, &format!("Limpeza cancelada: {e}")); return; }
        };
        let total = usage.bytes as f64 / 1048576.0;
        if usage.candidates.is_empty() {
            show_info(self.hwnd, &format!("Library and thumbnails: {total:.1} MB\nNo orphan files older than 24 hours.\n\nLibrary items and every monitor wallpaper are kept."));
            return;
        }

        let reviewed: Vec<_> = usage.candidates.into_iter().take(20).collect();
        let bytes: u64 = reviewed.iter().map(|c| c.bytes).sum();
        let files = reviewed.iter().map(|c| format!("{} ({:.1} MB)", c.path.file_name().unwrap_or_default().to_string_lossy(), c.bytes as f64 / 1048576.0)).collect::<Vec<_>>().join("\n");
        let message = format!("Total usage: {total:.1} MB\n\nDelete these {} unreferenced files ({:.1} MB)?\n\n{files}\n\nThis cannot be undone. Recent files and wallpapers in use are kept.", reviewed.len(), bytes as f64 / 1048576.0);
        if unsafe { MessageBoxW(self.hwnd, &HSTRING::from(message), w!("Clean up files — WallLit"), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) } != IDYES { return; }
        match walllit::maintenance::clean(&reviewed, &draft) {
            Ok(bytes) => show_info(self.hwnd, &format!("Liberados {:.1} MB.", bytes as f64 / 1048576.0)),
            Err(e) => show_info(self.hwnd, &format!("Cleanup interrupted: {e}")),
        }
    }

    pub(super) fn remove_selected(&mut self) {
        if unsafe { MessageBoxW(self.hwnd, w!("Remove the selected items from the library?\nThe media files and the monitor wallpapers are kept."), w!("WallLit"), MB_YESNO | MB_DEFBUTTON2) } != IDYES { return; }
        let before = self.library.clone();
        let mut selected = self.picker_selection.clone();
        selected.sort_unstable(); selected.dedup();
        for index in selected.into_iter().rev() { self.library.remove(index); }
        if let Err(e) = self.library.save() { self.library = before; self.status = e.to_string(); }
        self.picker_selection.clear();
        self.picker_preview = None; self.picker_hover = None; self.picker_failed = None;
        self.thumbnails.clear();
        self.ui.dirty = true;
    }

    pub(super) fn next_redraw(&self, minimized: bool) -> Option<i64> {
        let mut deadlines = Vec::with_capacity(5);
        if self.job.is_some() { deadlines.push(1_000_000); }
        if minimized { return deadlines.into_iter().min(); }
        if self.ui.dirty { deadlines.push(1); }
        if !self.options_open && !self.picker_open {
            if let Some(delay) = self.preview.as_ref().and_then(Preview::next_deadline) { deadlines.push(delay); }
        }
        if self.picker_open {
            if let Some((_, preview)) = &self.picker_preview {
                if let Some(delay) = preview.next_deadline() { deadlines.push(delay); }
            } else if let Some((index, since)) = self.picker_hover {
                if self.picker_failed != Some(index) { deadlines.push(((280.0 - since.elapsed_ms()).max(0.0) * 10_000.0) as i64 + 1); }
            }
            if self.transition_demo.as_ref().is_some_and(|demo| demo.started.elapsed_ms() < DEMO_100NS as f64 / 10_000.0) {
                deadlines.push(166_666);
            }
        }
        deadlines.into_iter().min()
    }
}

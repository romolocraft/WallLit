use crate::{config::Config, display::Monitor, instance, library::Library, poster, renderer::Gpu, video};
use windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager;

pub struct Applied {
    pub warnings: Vec<String>,
}

pub fn commit(config: &Config, previous: &Config, monitors: &[Monitor], gpu: &Gpu, manager: &IMFDXGIDeviceManager) -> std::io::Result<Applied> {
    config.save()?;
    let mut warnings = Vec::new();
    match Library::try_load() {
        Ok(mut library) => {
            let mut changed = false;
            for entry in config.monitors.values() {
                for slide in &entry.slides {
                    if library.items().iter().any(|i| i.prepared == slide.wallpaper) { continue; }
                    match crate::import::probe(&slide.wallpaper).and_then(|info| library.add(gpu, manager, &slide.wallpaper, slide.source.as_deref(), (info.width, info.height), info.fps as f32)) {
                        Ok(()) => changed = true,
                        Err(e) => warnings.push(format!("library: {}: {e}", slide.wallpaper.display())),
                    }
                }
            }
            if changed { if let Err(e) = library.save() { warnings.push(format!("library: {e}")); } }
        }
        Err(e) => warnings.push(format!("index preserved; could not read it: {e}")),
    }
    for monitor in monitors {
        let first = |config: &Config| config.monitors.get(&monitor.id).and_then(|m| m.slides.first()).cloned();
        let old = first(previous);
        let new = first(config);

        let poster_key = |s: &crate::config::Slide| {
            (
                s.wallpaper.clone(),
                s.mode,
                s.scale,
                s.x,
                s.y,
                s.stretch_x,
                s.stretch_y,
                s.filters,
                s.layers.clone(),
            )
        };
        if old.as_ref().map(poster_key) == new.as_ref().map(poster_key) { continue; }
        match new {
            Some(slide) => if let Err(e) = poster::apply(gpu, manager, monitor, &slide) {
                warnings.push(format!("still image: {}: {e}", monitor.device));
            },
            None => poster::clear(monitor),
        }
    }
    if !instance::request_reload() {
        match crate::autostart::engine_path().and_then(|path| std::process::Command::new(path).spawn().map_err(poster::to_windows_error)) {
            Ok(_) => {},
            Err(e) => warnings.push(format!("settings saved; engine not started: {e}")),
        }
    }
    for warning in &warnings { crate::diagnostics::record(warning); }
    Ok(Applied { warnings })
}

pub fn gpu() -> windows::core::Result<(Gpu, IMFDXGIDeviceManager)> {
    let gpu = Gpu::new()?;
    let manager = video::create_device_manager(&gpu.device)?;
    Ok((gpu, manager))
}

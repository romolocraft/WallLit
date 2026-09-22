use super::*;
use walllit::{diagnostics, recovery::Retry};

struct Resources {
    gpu: Gpu,
    manager: IMFDXGIDeviceManager,
    anchor: desktop::DesktopAnchor,
}

fn reconcile(resources: &mut Resources, wallpapers: &mut Vec<Wallpaper>, assignments: &[Assignment], args: &Args) -> Result<()> {
    if assignments.is_empty() { wallpapers.clear(); return Ok(()); }
    let anchor = desktop::find_anchor()?;
    let parent_changed = anchor.parent != resources.anchor.parent;
    resources.anchor = anchor;
    let mut old = std::mem::take(wallpapers);
    for assignment in assignments {
        let existing = old.iter().position(|w| w.monitor_id == assignment.monitor.id
            && w.monitor_rect == assignment.monitor.rect && desktop::is_alive(w.hwnd) && !parent_changed);
        if let Some(index) = existing {
            let mut wallpaper = old.remove(index);
            wallpaper.update_settings(assignment.settings.clone(), &resources.gpu, &resources.manager, args);
            wallpapers.push(wallpaper);
        } else {
            match Wallpaper::new(&resources.gpu, &resources.manager, resources.anchor.parent, &assignment.monitor, assignment.settings.clone(), args) {
                Ok(wallpaper) => wallpapers.push(wallpaper),
                Err(e) => diagnostics::record(&format!("{}: {e}", assignment.monitor.id)),
            }
        }
    }
    if wallpapers.len() != assignments.len() {
        return Err(Error::new(E_INVALIDARG, "some surfaces are unavailable"));
    }
    Ok(())
}

pub(super) fn run(args: &Args) -> Result<()> {
    let started = Instant::now();
    let mut reported = false;
    let mut first_frame = false;
    let Some(_lock) = instance::EngineLock::acquire() else { return Ok(()) };
    let reload = instance::reload_signal()?;
    let quit = instance::quit_signal()?;
    let mut config = Config::load();
    let mut assignments = resolve_assignments(args, &config).unwrap_or_default();
    let mut session = session::SessionWatcher::new()?;
    let occlusion = occlusion::OcclusionWatcher::new();
    let mut tray = tray::Tray::new().ok();
    let mut resources: Option<Resources> = None;
    let mut wallpapers = Vec::new();
    let mut recovery = Retry::default();
    recovery.request();
    let mut user_paused = false;
    let mut covered = BTreeMap::new();
    let mut monitoring = stats::Monitor::new(args.stats);
    let mut published = String::new();
    let mut last_status = None;
    let timer = unsafe { CreateWaitableTimerExW(None, None, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS.0) }?;

    struct Timer(HANDLE);
    impl Drop for Timer { fn drop(&mut self) { unsafe { let _ = windows::Win32::Foundation::CloseHandle(self.0); } } }
    let _timer = Timer(timer);

    loop {
        if !pump_messages() { break; }
        let (display_changed, clock_changed) = session.take_changes();
        if display_changed {
            if let Ok(next) = resolve_assignments(args, &config) { assignments = next; }
            recovery.request();
            occlusion.invalidate();
        }
        if occlusion.take_changed() {
            let areas: Vec<_> = assignments.iter().map(|a| a.monitor.rect).collect();
            covered = coverage_by_monitor(&assignments, occlusion::covered(&areas));
            if let Some(r) = &resources {
                if !assignments.is_empty() && desktop_needs_reattach(&r.anchor, &wallpapers, assignments.len()) {
                    recovery.request();
                }
            }
        }

        if recovery.ready() {
            let result = (|| -> Result<()> {
                if resources.is_none() {
                    let gpu = Gpu::new()?;
                    let manager = video::create_device_manager(&gpu.device)?;
                    let anchor = desktop::find_anchor()?;
                    resources = Some(Resources { gpu, manager, anchor });
                }
                reconcile(resources.as_mut().unwrap(), &mut wallpapers, &assignments, args)
            })();
            match result {
                Ok(()) => {
                    recovery.clear(); occlusion.invalidate();
                    if args.stats && !reported {
                        report_setup(&resources.as_ref().unwrap().anchor, &assignments, &wallpapers, started);
                        reported = true;
                    }
                }
                Err(e) => { diagnostics::record(&format!("recovery: {e}")); recovery.failed(); }
            }
        }

        if let Some(command) = tray.as_mut().and_then(tray::Tray::take_command) {
            match command {
                tray::TrayCommand::OpenSettings => tray::open_settings(),
                tray::TrayCommand::TogglePause => {
                    user_paused = !user_paused;
                    if let Some(tray) = tray.as_mut() { tray.set_paused(user_paused); }
                }
                tray::TrayCommand::NextWallpaper => {
                    if let Some(r) = &resources {
                        for w in &mut wallpapers { w.next_manual(&r.gpu, &r.manager, args); }
                    }
                }
                tray::TrayCommand::Quit => {
                    instance::request_shutdown();
                    break;
                }
            }
        }

        let state = session.state();
        let energy_pause = (config.pause_on_battery && state.on_battery) || (config.pause_on_energy_saver && state.energy_saver);
        let dormant = user_paused || state.is_dormant() || energy_pause;
        let mut device_lost = false;
        if let Some(r) = &resources {
            for w in &mut wallpapers {
                let next = Playback::decide(dormant, config.pause_when_fullscreen, covered.get(&w.monitor_id).copied().unwrap_or(false));
                let changed = w.set_state(next);
                if clock_changed || (changed && next == Playback::Playing) { w.refresh_schedule(&r.gpu, &r.manager, args); }
                match w.advance_if_due(&r.gpu, &r.manager, args) {
                    Ok(true) => {
                        monitoring.tick();
                        if args.stats && !first_frame { println!("first frame on screen in {:.1} ms", started.elapsed_ms()); first_frame = true; }
                    }
                    Ok(false) => {},
                    Err(e) => {
                        w.last_error = e.message();
                        diagnostics::record(&format!("{}: {e}", w.monitor_id));
                        w.retry.failed();
                        if unsafe { r.gpu.device.GetDeviceRemovedReason() }.is_err() { device_lost = true; break; }
                    }
                }
            }
            if (recovery.pending() || wallpapers.iter().any(|w| w.retry.pending())) && unsafe { r.gpu.device.GetDeviceRemovedReason() }.is_err() { device_lost = true; }
        }
        if device_lost {
            wallpapers.clear();
            resources = None;
            recovery.failed();
            diagnostics::record("graphics device lost; waiting for recovery");
        }

        use std::hash::{Hash, Hasher};
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        (user_paused, state.is_dormant(), state.on_battery, state.energy_saver, recovery.pending()).hash(&mut hash);
        for w in &wallpapers { (&w.monitor_id, w.state as u8, w.current, &w.last_error, w.released).hash(&mut hash); }
        let signature = hash.finish();
        if last_status != Some(signature) {
        last_status = Some(signature);
        let mut status = format!("PID {} | pausa manual: {} | sessao inativa: {} | bateria: {} | economia: {} | recuperando: {}\n", std::process::id(), user_paused, state.is_dormant(), state.on_battery, state.energy_saver, recovery.pending());
        for w in &wallpapers {
            status.push_str(&format!("{}: {} | item {} | {}\n", w.monitor_id, w.state.label(), w.current + 1, w.last_error));
        }
        diagnostics::publish(&status, &mut published);
        }

        let next_deadline = wallpapers.iter().filter_map(Wallpaper::next_deadline).chain(recovery.deadline()).min();
        let signal = match next_deadline {
            Some(delay) => unsafe {
                SetWaitableTimer(timer, &-delay.max(1), 0, None, None, false)?;
                MsgWaitForMultipleObjectsEx(Some(&[reload.handle(), quit.handle(), timer]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
            },
            None => unsafe { MsgWaitForMultipleObjectsEx(Some(&[reload.handle(), quit.handle()]), INFINITE, QS_ALLINPUT, MWMO_INPUTAVAILABLE) },
        };
        if signal == WAIT_EVENT(WAIT_OBJECT_0.0 + 1) { break; }
        if signal == WAIT_OBJECT_0 {
            match Config::try_load() {
                Ok(next) => match resolve_assignments(args, &next) {
                    Ok(next_assignments) => {
                        config = next;
                        assignments = next_assignments;

                        recovery.clear();
                        recovery.request();
                        occlusion.invalidate();
                    }
                    Err(e) => diagnostics::record(&format!("recarga: {e}")),
                },
                Err(e) => diagnostics::record(&format!("recarga rejeitada: {e}")),
            }
        }
        if signal == windows::Win32::Foundation::WAIT_FAILED { return Err(Error::from_win32()); }
    }
    diagnostics::publish("Motor encerrado.", &mut published);
    Ok(())
}

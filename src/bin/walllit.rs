#![windows_subsystem = "windows"]

use walllit::{
    autostart, config, desktop, display, image, import, instance, occlusion,
    renderer, session, stats, tray, video,
};

use windows::core::*;
use windows::Win32::Foundation::{E_INVALIDARG, HANDLE, HWND, WAIT_EVENT, WAIT_OBJECT_0};
use windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

mod engine;

use config::Config;
use display::Monitor;
use renderer::{Area, FitMode, FrameRef, Gpu, Placement, Surface};
use stats::Instant;
use video::{MediaFoundation, VideoInfo, VideoSource};
use walllit::playback::Playback;
use std::collections::BTreeMap;

struct Args {
    video: Option<String>,
    monitor: Option<usize>,
    mode: Option<FitMode>,
    stats: bool,
    probe: bool,
    swap: Option<String>,
    test_color: bool,

    apply: bool,

    import: bool,

    all: bool,
    speed: Option<f32>,

    autostart: Option<String>,
}

fn parse_args() -> std::result::Result<Args, String> {
    let mut args = Args {
        video: None,
        monitor: None,
        mode: None,
        stats: false,
        probe: false,
        swap: None,
        test_color: false,
        apply: false,
        import: false,
        all: false,
        speed: None,
        autostart: None,
    };

    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--stats" => args.stats = true,
            "--probe" => args.probe = true,
            "--test-color" => args.test_color = true,
            "--apply" => args.apply = true,
            "--import" => args.import = true,
            "--all" => args.all = true,
            "--speed" => {
                args.speed = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--speed needs a number, for example 0.5 or 2")?,
                );
            }
            "--autostart" => {
                args.autostart = Some(
                    it.next()
                        .ok_or("--autostart needs on, off or status")?,
                );
            }
            "--swap" => args.swap = it.next(),
            "--monitor" => {
                args.monitor = Some(
                    it.next()
                        .and_then(|v| v.parse().ok())
                        .ok_or("--monitor needs an index")?,
                );
            }
            "--mode" => {
                args.mode = Some(match it.next().as_deref() {
                    Some("fill") => FitMode::Fill,
                    Some("fit") => FitMode::Fit,
                    Some("stretch") => FitMode::Stretch,
                    Some("center") => FitMode::Center,
                    Some("custom") => FitMode::Custom,
                    other => return Err(format!("unknown mode: {other:?}")),
                });
            }
            other if other.starts_with("--") => return Err(format!("unknown option: {other}")),
            other => args.video = Some(other.to_string()),
        }
    }

    Ok(args)
}

const USAGE: &str = "\
usage: walllit                                    uses the saved settings
       walllit <video.mp4> [--monitor N] [--mode fill|fit|stretch|center|custom]
       walllit --probe                            inspects desktop and monitors

options: --stats  measures startup, frame rate, CPU and memory
         --swap   forces the presentation model (diagnostic)";

fn main() -> std::process::ExitCode {
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }

    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if args.probe {
        probe();
        return std::process::ExitCode::SUCCESS;
    }

    if let Some(mode) = args.autostart.as_deref() {
        return match set_autostart(mode) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            eprintln!("failed to initialise COM");
            return std::process::ExitCode::FAILURE;
        }
    }

    let _mf = match MediaFoundation::startup() {
        Ok(mf) => mf,
        Err(e) => {
            eprintln!("failed to start Media Foundation: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };

    if args.import {
        return match run_import(&args) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("import failed: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    if args.apply {
        return match apply(&args) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("apply failed: {e}");
                std::process::ExitCode::FAILURE
            }
        };
    }

    match engine::run(&args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("failed: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn set_autostart(mode: &str) -> std::result::Result<(), String> {
    match mode {
        "on" => {
            autostart::enable().map_err(|e| e.message())?;
            println!("start with Windows: on ({})",
                autostart::engine_path().map_err(|e| e.message())?.display());
        }
        "off" => {
            autostart::disable().map_err(|e| e.message())?;
            println!("start with Windows: off");
        }
        "status" => println!(
            "start with Windows: {}",
            if autostart::is_enabled() { "on" } else { "off" }
        ),
        other => return Err(format!("unknown value for --autostart: {other}")),
    }
    Ok(())
}

fn probe() {
    let mut report = String::new();

    desktop::describe_tree(&mut report);

    let monitors = display::enumerate();

    report.push_str("
--- monitores ---
");
    for (i, m) in monitors.iter().enumerate() {
        report.push_str(&format!(
            "[{}] {:<14} {}x{} @({},{})  {} Hz  primary={}
     id: {}
",
            i,
            m.device,
            m.width(),
            m.height(),
            m.rect.left,
            m.rect.top,
            m.refresh_hz,
            m.primary,
            m.id
        ));
    }

    let areas: Vec<_> = monitors.iter().map(|m| m.rect).collect();
    report.push_str("
--- cobertura neste instante ---
");
    for (monitor, covering) in monitors.iter().zip(occlusion::describe(&areas)) {
        match covering {
            Some(window) => {
                report.push_str(&format!("{:<14} covered by {}
", monitor.device, window))
            }
            None => report.push_str(&format!("{:<14} visivel
", monitor.device)),
        }
    }

    let vs = display::virtual_screen();
    report.push_str(&format!(
        "
desktop virtual: origem=({},{}) tamanho={}x{}
",
        vs.left,
        vs.top,
        vs.right - vs.left,
        vs.bottom - vs.top
    ));

    match config::config_path() {
        Ok(p) => report.push_str(&format!("
settings: {}
", p.display())),
        Err(e) => report.push_str(&format!("
settings: unavailable ({e})
")),
    }

    print!("{report}");

    if let Ok(directory) = config::data_dir() {
        let _ = std::fs::create_dir_all(&directory);
        let path = directory.join("probe.txt");
        if std::fs::write(&path, &report).is_ok() {
            println!("
relatorio gravado em {}", path.display());
        }
    }
}

fn run_import(args: &Args) -> std::result::Result<(), String> {
    let video = args
        .video
        .as_ref()
        .ok_or("--import needs the path to a video")?;

    let monitors = display::enumerate();
    let index = args.monitor.unwrap_or(0);
    let monitor = monitors
        .get(index)
        .ok_or("monitor index out of range")?;

    let target = import::Target::for_monitor(monitor.width(), monitor.height());
    println!("target: {} x {} at {} fps", target.width, target.height, target.fps);

    let progress = import::Progress::new();
    let started = Instant::now();

    let outcome = import::import(std::path::Path::new(video), target, &progress)
        .map_err(|e| e.message())?;

    println!("{}", outcome.summary());
    println!("file: {}", outcome.path().display());
    println!("levou {:.1} s", started.elapsed_ms() / 1000.0);
    Ok(())
}

fn apply(args: &Args) -> std::result::Result<(), String> {
    let video = args
        .video
        .as_ref()
        .ok_or("--apply needs the path to a video")?;

    let path = std::path::Path::new(video)
        .canonicalize()
        .map(|p| {
            let text = p.to_string_lossy();
            std::path::PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text).to_string())
        })
        .map_err(|e| format!("{video}: {e}"))?;

    let monitors = display::enumerate();
    if monitors.is_empty() {
        return Err("no active monitor".into());
    }

    let targets: Vec<&Monitor> = if args.all {
        monitors.iter().collect()
    } else {
        let index = args.monitor.unwrap_or(0);
        vec![monitors
            .get(index)
            .ok_or("monitor index out of range")?]
    };

    let mut config = Config::read_or_default().map_err(|e| e.to_string())?;
    let before = config.clone();
    let progress = import::Progress::new();
    for monitor in targets {
        let target = import::Target::for_monitor(monitor.width(), monitor.height());
        let outcome = import::prepare(&path, target, &progress, config.optimize_wallpaper).map_err(|e| e.message())?;
        let prepared = outcome.path().to_path_buf();
        let source = (prepared != path).then(|| path.clone());
        let mut settings = config.monitors.get(&monitor.id).cloned().unwrap_or_default();
        let previous = settings.slides.first().cloned().unwrap_or_default();

        settings.slides = vec![config::Slide {
            wallpaper: prepared.clone(),
            source,
            mode: args.mode.map(Into::into).unwrap_or(previous.mode),
            speed: args.speed.unwrap_or(previous.speed),
            ..previous
        }];

        config.monitors.insert(monitor.id.clone(), settings);
        println!("{} <- {}", monitor.device, prepared.display());
    }

    let (gpu, manager) = walllit::apply::gpu().map_err(|e| e.message())?;
    let applied = walllit::apply::commit(&config, &before, &monitors, &gpu, &manager).map_err(|e| e.to_string())?;
    println!("settings applied; {} warnings", applied.warnings.len());
    Ok(())
}

enum Media {
    Video(VideoSource),
    Image(image::StillImage),
}

impl Media {
    fn open(gpu: &Gpu, manager: &IMFDXGIDeviceManager, path: &std::path::Path) -> Result<Self> {
        if image::is_image(path) {
            Ok(Media::Image(image::StillImage::load(gpu, path)?))
        } else {
            Ok(Media::Video(VideoSource::open(&path.to_string_lossy(), manager)?))
        }
    }

    fn is_image(&self) -> bool {
        matches!(self, Media::Image(_))
    }
}

struct OpenLayer {
    media: Media,
    info: VideoInfo,
    placement: Placement,
    rect: [f32; 4],
    interval_100ns: i64,
    deadline: Instant,
    needs_redraw: bool,
    empty_reads: u32,
}

impl OpenLayer {
    fn area(&self, size: (u32, u32)) -> Area {
        let [x, y, w, h] = self.rect;
        Area {
            x: x * size.0 as f32,
            y: y * size.1 as f32,
            w: (w * size.0 as f32).max(1.0),
            h: (h * size.1 as f32).max(1.0),
        }
    }

    fn settled(&self) -> bool {
        !self.needs_redraw && self.media.is_image()
    }
}

struct Wallpaper {
    monitor_id: String,
    monitor_rect: windows::Win32::Foundation::RECT,
    hwnd: HWND,
    surface: Surface,

    slides: Vec<config::Slide>,
    settings: config::MonitorConfig,
    current: usize,

    source: Option<Media>,

    overlays: Vec<OpenLayer>,
    info: VideoInfo,

    needs_redraw: bool,
    placement: Placement,

    deadline: Instant,
    interval_100ns: i64,

    switch_at: Option<Instant>,

    state: Playback,
    retry: walllit::recovery::Retry,
    paused_remaining: Option<i64>,
    last_error: String,
    release_at: Option<Instant>,
    released: bool,
    empty_reads: u32,
}

impl Wallpaper {
    fn new(
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        parent: HWND,
        monitor: &Monitor,
        settings: config::MonitorConfig,
        args: &Args,
    ) -> Result<Self> {
        let hwnd = desktop::create_window(parent, monitor.rect)?;
        let surface = match Surface::new(gpu, hwnd, monitor.width(), monitor.height(), args.swap.as_deref()) {
            Ok(surface) => surface,
            Err(e) => { unsafe { let _ = DestroyWindow(hwnd); } return Err(e); }
        };

        let slides = settings.slides.clone();

        let mut wallpaper = Self {
            overlays: Vec::new(),
            monitor_id: monitor.id.clone(),
            monitor_rect: monitor.rect,
            hwnd,
            surface,
            slides,
            settings,
            current: 0,
            source: None,
            info: placeholder_info(monitor),
            needs_redraw: true,
            placement: Placement::default(),
            deadline: Instant::now(),
            interval_100ns: 333_333,
            switch_at: None,
            state: Playback::Playing,
            retry: Default::default(),
            paused_remaining: None,
            last_error: String::new(),
            release_at: None,
            released: false,
            empty_reads: 0,
        };

        if let Some((slide, _)) = wallpaper
            .settings
            .schedule
            .active_at(stats::local_minute_of_day())
        {
            wallpaper.current = slide.min(wallpaper.slides.len().saturating_sub(1));
        }

        wallpaper.load_available(gpu, manager, args);
        Ok(wallpaper)
    }

    fn load_available(&mut self, gpu: &Gpu, manager: &IMFDXGIDeviceManager, args: &Args) {
        for index in walllit::recovery::candidates(self.current, self.slides.len().max(1)) {
            self.current = index;
            match self.load_current(gpu, manager, args) {
                Ok(()) => { self.retry.clear(); self.last_error.clear(); self.released = false; self.arm_switch(args); return; }
                Err(e) => self.last_error = e.message(),
            }
        }
        self.source = None;
        self.overlays.clear();
        self.switch_at = None;
        self.retry.failed();
        walllit::diagnostics::record(&format!("{}: no media available: {}", self.monitor_id, self.last_error));
    }

    fn update_settings(&mut self, settings: config::MonitorConfig, gpu: &Gpu, manager: &IMFDXGIDeviceManager, args: &Args) {
        if self.settings == settings { return; }
        let old_path = self.slides.get(self.current).map(|s| s.wallpaper.clone());
        self.current = old_path.as_ref().and_then(|path| settings.slides.iter().position(|s| &s.wallpaper == path)).unwrap_or(0);
        self.slides = settings.slides.clone();
        self.settings = settings;
        if let Some((index, _)) = self.settings.schedule.active_at(stats::local_minute_of_day()) {
            self.current = index.min(self.slides.len().saturating_sub(1));
        }
        if self.source.is_some() && self.slides.get(self.current).map(|s| &s.wallpaper) == old_path.as_ref() {
            let slide = &self.slides[self.current];
            self.placement = slide.placement();
            self.interval_100ns = self.info.frame_interval_100ns(args.speed.unwrap_or(slide.speed));
            self.needs_redraw = true;
            self.deadline = Instant::now();
            self.arm_switch(args);
        } else {
            self.load_available(gpu, manager, args);
        }
    }

    fn arm_switch(&mut self, args: &Args) {
        self.switch_at = if self.slides.len() < 2 { None }
        else if let Some((_, minute)) = self.settings.schedule.active_at(stats::local_minute_of_day()) {
            Some(Instant::after(minutes_until(minute)))
        } else {
            self.slides.get(self.current).map(|slide| {
                let mut hold = self.settings.hold_for(slide);
                if self.source.as_ref().is_some_and(Media::is_image) && matches!(hold, config::Hold::Loops(_)) {
                    hold = config::Hold::IMAGE_DEFAULT;
                }
                let duration = (self.info.duration_100ns as f64 / args.speed.unwrap_or(slide.speed).max(0.05) as f64) as i64;
                Instant::after(hold.duration_100ns(duration))
            })
        };
        if self.state != Playback::Playing {
            self.paused_remaining = self.switch_at.map(|d| d.until_100ns().max(0));
        }
    }

    fn refresh_schedule(&mut self, gpu: &Gpu, manager: &IMFDXGIDeviceManager, args: &Args) {
        if let Some((index, _)) = self.settings.schedule.active_at(stats::local_minute_of_day()) {
            let index = index.min(self.slides.len().saturating_sub(1));
            if index != self.current { self.current = index; self.load_available(gpu, manager, args); }
            self.arm_switch(args);
        }
    }

    fn next_manual(&mut self, gpu: &Gpu, manager: &IMFDXGIDeviceManager, args: &Args) {
        if self.slides.len() > 1 {
            self.current = (self.current + 1) % self.slides.len();
            self.load_available(gpu, manager, args);
            self.arm_switch(args);
        }
    }

    fn load_current(
        &mut self,
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        args: &Args,
    ) -> Result<()> {
        let Some(slide) = self.slides.get(self.current) else {
            self.source = None;
            self.interval_100ns = 333_333;
            self.switch_at = None;
            return Ok(());
        };

        self.source = None;
        self.overlays.clear();

        let slide_layers = slide.layers.clone();
        let path = playable(slide);
        let source = Media::open(gpu, manager, path)?;

        self.info = match &source {
            Media::Video(video) => video.info,

            Media::Image(picture) => VideoInfo {
                width: picture.width,
                height: picture.height,
                fps_num: 1,
                fps_den: 1,
                matrix: renderer::ColorMatrix::Bt709,
                full_range: true,
                duration_100ns: 0,
            },
        };
        self.needs_redraw = true;
        self.placement = match args.mode {
            Some(mode) => Placement { mode, ..slide.placement() },
            None => slide.placement(),
        };

        let speed = args.speed.unwrap_or(slide.speed);
        self.interval_100ns = match &source {
            Media::Video(video) => video.info.frame_interval_100ns(speed),

            Media::Image(_) => 5_000_000,
        };
        self.deadline = Instant::now();

        self.switch_at = if self.slides.len() < 2 {
            None
        } else if let Some((_, next_minute)) = self
            .settings
            .schedule
            .active_at(stats::local_minute_of_day())
        {
            Some(Instant::after(minutes_until(next_minute)))
        } else {
            let mut hold = self.settings.hold_for(slide);

            if source.is_image() && matches!(hold, config::Hold::Loops(_)) {
                hold = config::Hold::IMAGE_DEFAULT;
            }

            let media = (self.info.duration_100ns as f64 / speed.max(0.05) as f64) as i64;
            Some(Instant::after(hold.duration_100ns(media)))
        };

        self.source = Some(source);
        self.empty_reads = 0;
        self.open_overlays(gpu, manager, slide_layers);
        Ok(())
    }

    fn open_overlays(
        &mut self,
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        layers: Vec<config::Layer>,
    ) {
        self.overlays.clear();

        for layer in layers {
            let path = layer_path(&layer);
            let media = match Media::open(gpu, manager, path) {
                Ok(media) => media,
                Err(e) => {
                    walllit::diagnostics::record(&format!(
                        "{}: layer {}: {e}",
                        self.monitor_id,
                        layer.display_name()
                    ));
                    continue;
                }
            };

            let info = match &media {
                Media::Video(video) => video.info,
                Media::Image(picture) => VideoInfo {
                    width: picture.width,
                    height: picture.height,
                    fps_num: 1,
                    fps_den: 1,
                    matrix: renderer::ColorMatrix::Bt709,
                    full_range: true,
                    duration_100ns: 0,
                },
            };

            let interval_100ns = match &media {
                Media::Video(video) => video.info.frame_interval_100ns(layer.speed),
                Media::Image(_) => 5_000_000,
            };

            self.overlays.push(OpenLayer {
                media,
                info,
                placement: layer.placement(),
                rect: layer.rect,
                interval_100ns,
                deadline: Instant::now(),
                needs_redraw: true,
                empty_reads: 0,
            });
        }
    }

    fn advance_slide(&mut self, gpu: &Gpu, manager: &IMFDXGIDeviceManager, args: &Args) {
        if self.slides.len() < 2 {
            return;
        }

        if self.settings.transition != config::Transition::None {
            let kind = self.settings.transition as u32;
            let layers = self.frozen_layers();
            if let Err(e) = self
                .surface
                .begin_transition(gpu, kind, TRANSITION_100NS, &layers)
            {
                eprintln!("{}: no transition ({})", self.monitor_id, e.message());
            }
        }

        let next = match self
            .settings
            .schedule
            .active_at(stats::local_minute_of_day())
        {
            Some((slide, _)) => slide.min(self.slides.len() - 1),
            None => (self.current + 1) % self.slides.len(),
        };

        if next == self.current {
            self.arm_switch(args);
            return;
        }

        if args.stats {
            println!(
                "{}: item {} of {} - {}",
                self.monitor_id,
                next + 1,
                self.slides.len(),
                self.slides[next].display_name()
            );
        }

        self.current = next;
        self.load_available(gpu, manager, args);
    }

    fn set_state(&mut self, state: Playback) -> bool {
        if self.state == state {
            return false;
        }

        if state == Playback::Playing {
            self.release_at = None;
            self.needs_redraw = true;
            self.deadline = Instant::now();
            if !self.settings.schedule.enabled {
                self.switch_at = self.paused_remaining.take().map(Instant::after);
            }
        } else if self.state == Playback::Playing {
            self.paused_remaining = self.switch_at.map(|at| at.until_100ns().max(0));

            self.release_at = matches!(self.source, Some(Media::Video(_))).then(|| Instant::after(1_200_000_000));
        }

        self.state = state;
        true
    }

    fn next_deadline(&self) -> Option<i64> {
        if self.state != Playback::Playing {
            return self.release_at.map(|at| at.until_100ns());
        }
        if self.retry.pending() { return self.retry.deadline(); }

        let settled = !self.needs_redraw
            && !self.surface.is_blending()
            && self.source.as_ref().is_some_and(Media::is_image)
            && self.overlays.iter().all(OpenLayer::settled);

        let frame = (!settled).then(|| {
            let base = self.deadline.until_100ns();
            self.overlays
                .iter()
                .map(|o| o.deadline.until_100ns())
                .fold(base, i64::min)
        });
        let switch = self.switch_at.map(|at| at.until_100ns());

        match (frame, switch) {
            (Some(frame), Some(switch)) => Some(frame.min(switch)),
            (Some(frame), None) => Some(frame),
            (None, Some(switch)) => Some(switch),
            (None, None) => None,
        }
    }

    fn advance_if_due(
        &mut self,
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        args: &Args,
    ) -> Result<bool> {
        if self.state != Playback::Playing {
            if self.release_at.is_some_and(|at| at.until_100ns() <= 0) {
                self.source = None;

                self.overlays.clear();
                self.release_at = None;
                self.released = true;
            }
            return Ok(false);
        }

        if self.released {
            let switch_at = self.switch_at;
            self.load_available(gpu, manager, args);
            if !self.settings.schedule.enabled { self.switch_at = switch_at; }
        }

        if self.retry.pending() {
            if !self.retry.ready() { return Ok(false); }
            self.load_available(gpu, manager, args);
            if self.retry.pending() { return Ok(false); }
        }

        if matches!(self.switch_at, Some(at) if at.until_100ns() <= 0) {
            self.advance_slide(gpu, manager, args);
        }

        if self.deadline.until_100ns() > 0 {
            return Ok(false);
        }

        let blending = self.surface.is_blending();
        let due = self.deadline.until_100ns() <= 0
            || self.overlays.iter().any(|o| o.deadline.until_100ns() <= 0);

        if !due && !blending {
            return Ok(false);
        }

        let settled = !self.needs_redraw
            && self.source.as_ref().is_some_and(Media::is_image)
            && self.overlays.iter().all(OpenLayer::settled);

        if settled && !blending {
            self.deadline.advance_100ns(self.interval_100ns);
            for overlay in &mut self.overlays {
                overlay.deadline.advance_100ns(overlay.interval_100ns);
            }
            return Ok(false);
        }

        let size = self.surface.size();
        let layers = 1 + self.overlays.len();
        self.surface.begin(gpu, layers);

        let drew = self.draw_background(gpu, size)?;
        self.draw_overlays(gpu, size)?;

        self.surface.finish(gpu)?;

        let step = if self.surface.is_blending() || blending {
            BLEND_INTERVAL_100NS
        } else {
            self.interval_100ns
        };
        self.deadline.advance_100ns(step);

        Ok(drew)
    }

    fn draw_background(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<bool> {
        let placement = self.placement;
        let info = self.info;

        match self.source.as_mut() {
            None => {
                self.surface.fill(gpu, [1.0, 0.0, 1.0, 1.0]);
                Ok(true)
            }

            Some(Media::Image(_)) => {
                let Some(Media::Image(picture)) = self.source.as_ref() else {
                    return Ok(false);
                };
                self.surface
                    .draw_image_layer(gpu, 0, Area::full(size), picture, placement)?;
                self.needs_redraw = false;
                Ok(true)
            }

            Some(Media::Video(video)) => {
                let Some(frame) = video.next_frame()? else {
                    self.empty_reads += 1;
                    if self.empty_reads >= 8 {
                        return Err(Error::new(
                            windows::Win32::Foundation::E_FAIL,
                            "decoder delivered no frames",
                        ));
                    }
                    self.deadline = Instant::after(100_000);
                    return Ok(false);
                };
                self.empty_reads = 0;

                self.surface.draw_layer(
                    gpu,
                    0,
                    Area::full(size),
                    FrameRef {
                        texture: &frame.texture,
                        subresource: frame.subresource,
                        size: (info.width, info.height),
                        matrix: info.matrix,
                        full_range: info.full_range,
                    },
                    placement,
                )?;
                Ok(true)
            }
        }
    }

    fn draw_overlays(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<()> {
        let Self { overlays, surface, .. } = self;

        for (index, overlay) in overlays.iter_mut().enumerate() {
            let area = overlay.area(size);
            let placement = overlay.placement;
            let info = overlay.info;
            let layer = index + 1;

            match &mut overlay.media {
                Media::Image(picture) => {
                    surface.draw_image_layer(gpu, layer, area, picture, placement)?;
                    overlay.needs_redraw = false;
                }

                Media::Video(video) => match video.next_frame()? {
                    Some(frame) => {
                        overlay.empty_reads = 0;
                        surface.draw_layer(
                            gpu,
                            layer,
                            area,
                            FrameRef {
                                texture: &frame.texture,
                                subresource: frame.subresource,
                                size: (info.width, info.height),
                                matrix: info.matrix,
                                full_range: info.full_range,
                            },
                            placement,
                        )?;
                    }
                    None => {
                        overlay.empty_reads += 1;
                        surface.repaint_layer(gpu, layer, area, placement)?;
                    }
                },
            }

            overlay.deadline.advance_100ns(overlay.interval_100ns);
        }

        Ok(())
    }

    fn frozen_layers(&self) -> Vec<(Area, Placement)> {
        let size = self.surface.size();
        let mut layers = vec![(Area::full(size), self.placement)];
        for overlay in &self.overlays {
            layers.push((overlay.area(size), overlay.placement));
        }
        layers
    }
}

fn minutes_until(target_minute: u16) -> i64 {
    const DAY: i64 = 24 * 60;
    let now = stats::local_minute_of_day() as i64;
    let mut delta = target_minute as i64 - now;
    if delta <= 0 {
        delta += DAY;
    }
    delta.saturating_mul(60).saturating_mul(10_000_000)
}

const TRANSITION_100NS: i64 = 10_000_000;

const BLEND_INTERVAL_100NS: i64 = 166_667;

fn placeholder_info(monitor: &Monitor) -> VideoInfo {
    VideoInfo {
        width: monitor.width(),
        height: monitor.height(),
        fps_num: 30,
        fps_den: 1,
        matrix: renderer::ColorMatrix::Bt709,
        full_range: false,
        duration_100ns: 0,
    }
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

struct Assignment {
    monitor: Monitor,
    settings: config::MonitorConfig,
}

fn layer_path(layer: &config::Layer) -> &std::path::Path {
    if layer.wallpaper.is_file() {
        return &layer.wallpaper;
    }

    match layer.source.as_deref() {
        Some(source) if source.is_file() => source,
        _ => &layer.wallpaper,
    }
}

fn playable(slide: &config::Slide) -> &std::path::Path {
    if slide.wallpaper.is_file() {
        return &slide.wallpaper;
    }

    match slide.source.as_deref() {
        Some(source) if source.is_file() => {
            eprintln!("{} is gone; using the original file", slide.wallpaper.display());
            source
        }
        _ => &slide.wallpaper,
    }
}

fn resolve_assignments(args: &Args, config: &Config) -> Result<Vec<Assignment>> {
    let monitors = display::enumerate();
    if monitors.is_empty() && (args.video.is_some() || args.test_color) {
        return Err(Error::new(E_INVALIDARG, "no active monitor"));
    }

    if args.video.is_some() || args.test_color {
        let index = args.monitor.unwrap_or(0);
        let monitor = monitors
            .into_iter()
            .nth(index)
            .ok_or_else(|| Error::new(E_INVALIDARG, "monitor index out of range"))?;

        let slides = args
            .video
            .as_ref()
            .map(|video| {
                vec![config::Slide {
                    wallpaper: std::path::PathBuf::from(video),
                    mode: args.mode.unwrap_or(FitMode::Fill).into(),
                    ..Default::default()
                }]
            })
            .unwrap_or_default();

        return Ok(vec![Assignment {
            monitor,
            settings: config::MonitorConfig::from_slides(slides),
        }]);
    }

    let assignments: Vec<_> = monitors
        .into_iter()
        .filter_map(|monitor| {
            let settings = config.monitors.get(&monitor.id)?.clone();
            (!settings.is_empty()).then_some(Assignment { monitor, settings })
        })
        .collect();

    Ok(assignments)
}

fn coverage_by_monitor(assignments: &[Assignment], covered: Vec<bool>) -> BTreeMap<String, bool> {
    assignments.iter().zip(covered)
        .map(|(assignment, covered)| (assignment.monitor.id.clone(), covered))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_surface_does_not_shift_other_monitors_coverage() {
        let assignments: Vec<_> = ["first", "second"].into_iter().map(|id| Assignment {
            monitor: Monitor {
                device: id.into(), id: id.into(), rect: Default::default(),
                refresh_hz: 60, primary: id == "first",
            },
            settings: Default::default(),
        }).collect();
        let covered = coverage_by_monitor(&assignments, vec![true, false]);

        assert_eq!(Playback::decide(false, true, covered["second"]), Playback::Playing);
        assert_eq!(Playback::decide(false, true, covered["first"]), Playback::Paused);
    }
}

fn desktop_needs_reattach(
    anchor: &desktop::DesktopAnchor,
    wallpapers: &[Wallpaper],
    expected: usize,
) -> bool {
    if !desktop::is_alive(anchor.parent) {
        return true;
    }

    if wallpapers.len() != expected || wallpapers.iter().any(|w| !desktop::is_alive(w.hwnd)) {
        return true;
    }

    match desktop::find_anchor() {
        Ok(current) => current.parent != anchor.parent,
        Err(_) => false,
    }
}

fn report_setup(
    anchor: &desktop::DesktopAnchor,
    assignments: &[Assignment],
    wallpapers: &[Wallpaper],
    started: Instant,
) {
    println!("desktop  : attached via {}", anchor.kind);

    for wallpaper in wallpapers {
        let Some(assignment) = assignments.iter().find(|a| a.monitor.id == wallpaper.monitor_id) else { continue };
        let m = &assignment.monitor;
        println!(
            "monitor  : {} {}x{} @{}Hz  [{}]",
            m.device,
            m.width(),
            m.height(),
            m.refresh_hz,
            m.id
        );
        println!(
            "  video  : {}x{} {:.3} fps  {:?} {}  | {:?} | presents {}",
            wallpaper.info.width,
            wallpaper.info.height,
            wallpaper.info.fps(),
            wallpaper.info.matrix,
            if wallpaper.info.full_range { "full" } else { "limited" },
            wallpaper.placement.mode,
            wallpaper.surface.present_model
        );
    }

    println!(
        "ready in {:.1} ms (before the first frame)",
        started.elapsed_ms()
    );
}

fn pump_messages() -> bool {
    let mut msg = MSG::default();
    unsafe {
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_QUIT {
                return false;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    true
}

#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};

use windows::core::*;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap1;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;
use windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager;
use windows::Win32::System::Com::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{FileOpenDialog, IFileOpenDialog, SIGDN_FILESYSPATH};
use windows::Win32::UI::WindowsAndMessaging::*;

use walllit::config::{self, Config, MonitorConfig};
use walllit::autostart;
use walllit::display::{self, Monitor};
use walllit::icon::{self, Icon};
use std::collections::HashMap;
use walllit::image::{self, StillImage};
use walllit::import;
use walllit::instance;
use walllit::language::{self, Language, Text};
use walllit::library::Library;
use walllit::poster;
use walllit::renderer::{FitMode, FrameRef, Gpu, Offscreen, Placement};
use walllit::stats::Instant;
use walllit::ui::{self, theme, Rect, Ui};
use walllit::video::{self, MediaFoundation, VideoInfo, VideoSource};
mod canvas;
mod settings_actions;
use settings_actions::{show_info, recover_read};

const WINDOW_CLASS: PCWSTR = w!("WallLitSettings");

const APP_ICON_RESOURCE: usize = 1;
const DEFAULT_SIZE: (i32, i32) = (1120, 720);
const MIN_SIZE: (i32, i32) = (940, 600);

const SIDEBAR_WIDTH: f32 = 330.0;
const TAB_HEIGHT: f32 = 38.0;
const FOOTER_HEIGHT: f32 = 56.0;

const GEAR_SIZE: f32 = 30.0;

#[derive(Default)]
struct Pending {
    input: ui::Input,
    resized: Option<(u32, u32)>,
    quit: bool,
    text: Vec<u16>,
    display_changed: bool,
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let pending = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Pending;
    if pending.is_null() {
        return DefWindowProcW(hwnd, msg, w, l);
    }
    let pending = &mut *pending;

    match msg {
        WM_CHAR => { pending.text.push(w.0 as u16); LRESULT(0) }
        WM_DISPLAYCHANGE => { pending.display_changed = true; LRESULT(0) }
        WM_DESTROY => {
            pending.quit = true;
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_CLOSE => {
            pending.quit = true;
            LRESULT(0)
        }
        WM_SIZE => {
            let width = (l.0 & 0xFFFF) as u32;
            let height = ((l.0 >> 16) & 0xFFFF) as u32;
            if width > 0 && height > 0 {
                pending.resized = Some((width, height));
            }
            LRESULT(0)
        }
        WM_GETMINMAXINFO => {
            let info = &mut *(l.0 as *mut MINMAXINFO);
            info.ptMinTrackSize.x = MIN_SIZE.0;
            info.ptMinTrackSize.y = MIN_SIZE.1;
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            pending.input.mouse = (
                (l.0 & 0xFFFF) as i16 as f32,
                ((l.0 >> 16) & 0xFFFF) as i16 as f32,
            );
            LRESULT(0)
        }
        WM_LBUTTONDOWN | WM_LBUTTONDBLCLK => {
            pending.input.mouse = (
                (l.0 & 0xFFFF) as i16 as f32,
                ((l.0 >> 16) & 0xFFFF) as i16 as f32,
            );
            pending.input.down = true;
            pending.input.pressed = true;
            if msg == WM_LBUTTONDBLCLK {
                pending.input.double_click = true;
            }

            SetCapture(hwnd);
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            pending.input.down = false;
            pending.input.released = true;
            let _ = ReleaseCapture();
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            pending.input.mouse = (
                (l.0 & 0xFFFF) as i16 as f32,
                ((l.0 >> 16) & 0xFFFF) as i16 as f32,
            );
            pending.input.right_pressed = true;
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            let delta = ((w.0 >> 16) & 0xFFFF) as i16 as f32 / WHEEL_DELTA as f32;

            let mut point = windows::Win32::Foundation::POINT {
                x: (l.0 & 0xFFFF) as i16 as i32,
                y: ((l.0 >> 16) & 0xFFFF) as i16 as i32,
            };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut point);
            pending.input.mouse = (point.x as f32, point.y as f32);
            pending.input.wheel += delta;
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PAINT => {
            let mut ps = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
            let _ = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut ps);
            let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, w, l),
    }
}

enum PreviewSource {
    Video(VideoSource),

    Image(StillImage),
}

struct Preview {
    source: PreviewSource,
    offscreen: Offscreen,
    info: VideoInfo,
    deadline: Instant,
    interval_100ns: i64,
    path: PathBuf,

    target_size: (u32, u32),
    image_dirty: bool,
}

impl Preview {
    fn open(
        gpu: &Gpu,
        manager: &IMFDXGIDeviceManager,
        path: &Path,
        size: (u32, u32),
        speed: f32,
    ) -> Result<Self> {
        let (source, info) = if image::is_image(path) {
            let picture = StillImage::load(gpu, path)?;
            let info = VideoInfo {
                width: picture.width,
                height: picture.height,
                fps_num: 1,
                fps_den: 1,
                matrix: walllit::renderer::ColorMatrix::Bt709,
                full_range: true,
                duration_100ns: 0,
            };
            (PreviewSource::Image(picture), info)
        } else {
            let video = VideoSource::open(&path.to_string_lossy(), manager)?;
            let info = video.info;
            (PreviewSource::Video(video), info)
        };

        let offscreen = Offscreen::new(gpu, size.0, size.1)?;

        Ok(Self {
            source,
            offscreen,
            info,
            deadline: Instant::now(),
            interval_100ns: info.frame_interval_100ns(speed),
            path: path.to_path_buf(),
            target_size: size,
            image_dirty: true,
        })
    }

    fn set_speed(&mut self, speed: f32) {
        self.interval_100ns = match &self.source {
            PreviewSource::Image(_) => 5_000_000,
            PreviewSource::Video(_) => self.info.frame_interval_100ns(speed),
        };
    }

    fn resize(&mut self, gpu: &Gpu, size: (u32, u32)) -> Result<()> {
        if size == self.target_size || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }
        self.offscreen = Offscreen::new(gpu, size.0, size.1)?;
        self.target_size = size;
        self.image_dirty = true;
        self.deadline = Instant::now();
        Ok(())
    }

    fn advance(&mut self, gpu: &Gpu, placement: Placement) -> Result<bool> {
        if matches!(self.source, PreviewSource::Image(_)) && !self.image_dirty { return Ok(false); }
        if self.deadline.until_100ns() > 0 {
            return Ok(false);
        }

        self.offscreen.clear(gpu, [0.0, 0.0, 0.0, 1.0]);

        let video = match &mut self.source {
            PreviewSource::Image(picture) => {
                self.offscreen.paint_image(gpu, picture, placement)?;
                self.image_dirty = false;
                self.deadline.advance_100ns(self.interval_100ns);
                return Ok(true);
            }
            PreviewSource::Video(video) => video,
        };

        let Some(frame) = video.next_frame()? else {
            self.deadline = Instant::after(100_000);
            return Ok(false);
        };

        self.offscreen.paint(
            gpu,
            FrameRef {
                texture: &frame.texture,
                subresource: frame.subresource,
                size: (self.info.width, self.info.height),
                matrix: self.info.matrix,
                full_range: self.info.full_range,
            },
            placement,
        )?;

        self.deadline.advance_100ns(self.interval_100ns);
        Ok(true)
    }

    fn repaint(&mut self, gpu: &Gpu, placement: Placement) -> Result<()> {
        self.offscreen.clear(gpu, [0.0, 0.0, 0.0, 1.0]);
        match &self.source {
            PreviewSource::Image(picture) => self.offscreen.paint_image(gpu, picture, placement),
            PreviewSource::Video(_) => {
                self.offscreen.repaint(gpu, placement)?;
                Ok(())
            }
        }
    }

    fn next_deadline(&self) -> Option<i64> {
        if matches!(self.source, PreviewSource::Image(_)) && !self.image_dirty { None }
        else { Some(self.deadline.until_100ns().max(1)) }
    }
}

struct ImportJob {
    tab: String,
    source: PathBuf,
    progress: import::Progress,
    worker: std::thread::JoinHandle<std::result::Result<import::Outcome, String>>,
}

impl ImportJob {
    fn start(tab: String, source: PathBuf, target: import::Target, optimize: bool) -> Self {
        let progress = import::Progress::new();
        let shared = progress.clone();
        let path = source.clone();

        let worker = std::thread::spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }

            let outcome = import::prepare(&path, target, &shared, optimize).map_err(|e| e.message());

            unsafe { CoUninitialize() };
            outcome
        });

        Self { tab, source, progress, worker }
    }
}

struct Tab {
    monitor: Monitor,
    entry: MonitorConfig,

    slide: usize,
}

impl Tab {
    fn has_video(&self) -> bool {
        !self.entry.slides.is_empty()
    }

    fn slide(&self) -> Option<&config::Slide> {
        self.entry.slides.get(self.slide.min(self.entry.slides.len().saturating_sub(1)))
    }

    fn slide_mut(&mut self) -> Option<&mut config::Slide> {
        let index = self.slide.min(self.entry.slides.len().saturating_sub(1));
        self.entry.slides.get_mut(index)
    }

    fn label(&self, index: usize, language: Language) -> String {
        format!("{} {}", language::t(language, Text::Monitor), index + 1)
    }

    fn set_media(&mut self, wallpaper: PathBuf, source: Option<PathBuf>) {
        if self.entry.slides.is_empty() {
            self.entry.slides.push(config::Slide::default());
            self.slide = 0;
        }

        let index = self.slide.min(self.entry.slides.len() - 1);
        self.slide = index;

        let slide = &mut self.entry.slides[index];
        slide.wallpaper = wallpaper;
        slide.source = source;
    }
}

struct App {
    hwnd: HWND,
    gpu: Gpu,
    manager: IMFDXGIDeviceManager,
    swapchain: IDXGISwapChain1,
    ui: Ui,
    size: (u32, u32),

    config: Config,
    applied_config: Config,
    tabs: Vec<Tab>,
    selected: usize,
    preview: Option<Preview>,

    status: String,
    saved: bool,
    job: Option<ImportJob>,

    unreadable: Option<PathBuf>,

    autostart: bool,

    library: Library,

    thumbnails: walllit::cache::BudgetCache<PathBuf, Option<ID2D1Bitmap1>>,
    icons: Icons,

    options_open: bool,

    picker_open: bool,
    picker_scroll: f32,
    search: String,
    search_active: bool,
    favorites_only: bool,
    picker_failed: Option<usize>,

    picker_selection: Vec<usize>,

    picker_hover: Option<(usize, Instant)>,

    picker_preview: Option<(usize, Preview)>,

    sequence_open: bool,

    sequence_scroll: f32,
    sequence_overflow: f32,

    transition_demo: Option<TransitionDemo>,

    selected_layer: Option<usize>,

    grab: Option<canvas::Grab>,

    layer_stills: HashMap<PathBuf, Option<StillImage>>,

    paste_mode: bool,

    sidebar_scroll: f32,

    sidebar_overflow: f32,

    menu: Option<(f32, f32)>,

    editing: bool,

    preview_dirty: bool,
}

struct TransitionDemo {
    from: Offscreen,
    to: Offscreen,
    out: Offscreen,
    kind: u32,
    started: Instant,
}

const DEMO_100NS: i64 = 10_000_000;

struct Icons {
    gear: Icon,
    plus: Icon,
    star: Icon,
}

impl App {
    fn new(hwnd: HWND, size: (u32, u32)) -> Result<Self> {
        let gpu = Gpu::new()?;
        let manager = video::create_device_manager(&gpu.device)?;

        let swapchain = create_swapchain(&gpu, hwnd, size)?;
        let mut ui = Ui::new(&gpu)?;
        ui.attach(&swapchain, size.0 as f32, size.1 as f32)?;

        let icons = Icons {
            gear: ui.load_icon(icon::GEAR, 640.0)?,
            plus: ui.load_icon(icon::PLUS, 640.0)?,
            star: ui.load_icon(icon::STAR, 640.0)?,
        };

        let library = recover_read(hwnd, &walllit::library::index_path()?, Library::try_load)?;
        let config = recover_read(hwnd, &config::config_path()?, Config::read_or_default)?;
        let tabs = display::enumerate()
            .into_iter()
            .map(|monitor| {
                let entry = config.monitors.get(&monitor.id).cloned().unwrap_or_default();
                Tab { monitor, entry, slide: 0 }
            })
            .collect();

        Ok(Self {
            hwnd,
            gpu,
            manager,
            swapchain,
            ui,
            size,
            applied_config: config.clone(),
            config,
            tabs,
            selected: 0,
            preview: None,
            status: String::new(),
            saved: true,
            job: None,
            unreadable: None,
            autostart: autostart::is_enabled(),
            library,
            thumbnails: walllit::cache::BudgetCache::new(16 * 1024 * 1024, 96),
            icons,
            options_open: false,
            picker_open: false,
            picker_scroll: 0.0,
            search: String::new(),
            search_active: false,
            favorites_only: false,
            picker_failed: None,
            picker_selection: Vec::new(),
            picker_hover: None,
            picker_preview: None,
            sequence_open: false,
            sequence_scroll: 0.0,
            sequence_overflow: 0.0,
            transition_demo: None,
            selected_layer: None,
            grab: None,
            layer_stills: HashMap::new(),
            paste_mode: false,
            sidebar_scroll: 0.0,
            sidebar_overflow: 0.0,
            menu: None,
            editing: false,
            preview_dirty: false,
        })
    }

    fn t(&self, key: Text) -> &'static str {
        language::t(self.config.language(), key)
    }

    fn tab(&self) -> &Tab {
        &self.tabs[self.selected]
    }

    fn placement(&self) -> Placement {
        self.tab()
            .slide()
            .map(config::Slide::placement)
            .unwrap_or_default()
    }

    fn resize(&mut self, size: (u32, u32)) -> Result<()> {
        if size == self.size || size.0 == 0 || size.1 == 0 {
            return Ok(());
        }

        self.ui.detach();
        unsafe {
            self.gpu.context.ClearState();
            self.swapchain.ResizeBuffers(
                0,
                size.0,
                size.1,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }
        self.ui.attach(&self.swapchain, size.0 as f32, size.1 as f32)?;
        self.size = size;
        Ok(())
    }

    fn sync_preview(&mut self, preview_size: (u32, u32)) {
        let tab = &self.tabs[self.selected];

        let Some(slide) = tab.slide() else {
            self.preview = None;
            return;
        };

        let wallpaper = slide.wallpaper.clone();
        let speed = slide.speed;

        if self.unreadable.as_deref() == Some(wallpaper.as_path()) {
            return;
        }

        let needs_open = match &self.preview {
            Some(p) => p.path != wallpaper,
            None => true,
        };

        if !needs_open {
            if let Some(p) = self.preview.as_mut() {
                let _ = p.resize(&self.gpu, preview_size);
            }
            return;
        }

        match Preview::open(&self.gpu, &self.manager, &wallpaper, preview_size, speed) {
            Ok(p) => {
                self.preview = Some(p);
                self.unreadable = None;
                self.status.clear();
            }
            Err(e) => {
                self.preview = None;
                self.unreadable = Some(wallpaper);
                self.status = format!("{}: {}", language::t(self.config.language(), Text::CannotOpenVideo), e.message());
            }
        }
    }

    fn poll_import(&mut self) {
        let Some(job) = self.job.as_ref() else { return };
        if !job.worker.is_finished() {
            return;
        }

        let job = self.job.take().unwrap();
        let tab = self.tabs.iter().position(|tab| tab.monitor.id == job.tab);

        match job.worker.join() {
            Ok(Ok(outcome)) => {
                self.status = outcome.summary();

                let prepared = outcome.path().to_path_buf();

                let source = (prepared != job.source).then(|| job.source.clone());

                if let Some(tab) = tab.and_then(|index| self.tabs.get_mut(index)) {
                    tab.set_media(prepared.clone(), source.clone());
                }

                self.register_in_library(&prepared, source.as_deref());
                self.preview = None;
                self.unreadable = None;
                self.saved = false;
            }
            Ok(Err(message)) => {
                self.status = format!("{} ({message})", language::t(self.config.language(), Text::NotOptimized));

                self.preview = None;
                self.unreadable = None;
                self.saved = false;
            }
            Err(_) => {
                self.status = language::t(self.config.language(), Text::ImportInterrupted).into()
            }
        }
    }

    fn choose_video(&mut self, tab: usize, path: PathBuf) {
        self.unreadable = None;

        let Some(monitor) = self.tabs.get(tab).map(|t| &t.monitor) else { return };
        let target = import::Target::for_monitor(monitor.width(), monitor.height());

        self.status = language::t(self.config.language(), Text::PreparingVideo).into();
        if self.job.is_some() { return; }
        self.job = Some(ImportJob::start(monitor.id.clone(), path, target, self.config.optimize_wallpaper));
    }

    fn save(&mut self) {
        for tab in &self.tabs {
            if tab.has_video() {
                self.config
                    .monitors
                    .insert(tab.monitor.id.clone(), tab.entry.clone());
            } else {
                self.config.monitors.remove(&tab.monitor.id);
            }
        }

        let monitors = display::enumerate();
        match walllit::apply::commit(&self.config, &self.applied_config, &monitors, &self.gpu, &self.manager) {
            Ok(result) => {
                self.saved = true;
                self.applied_config = self.config.clone();
                self.status = if result.warnings.is_empty() { self.t(Text::ConfigSavedApplied).into() } else { result.warnings.join("; ") };
                if let Ok(library) = Library::try_load() { self.library = library; }
            }
            Err(e) => self.status = format!("{}: {e}", self.t(Text::CannotSave)),
        }
    }

    fn start_transition_demo(&mut self, kind: u32, size: (u32, u32)) {
        let slides = &self.tabs[self.selected].entry.slides;
        let mut sources: Vec<PathBuf> =
            slides.iter().take(2).map(|s| s.wallpaper.clone()).collect();

        if sources.len() < 2 {
            for item in self.library.items().iter().take(2) {
                if sources.len() >= 2 {
                    break;
                }
                if !sources.contains(&item.prepared) {
                    sources.push(item.prepared.clone());
                }
            }
        }

        if sources.len() == 1 {
            sources.push(sources[0].clone());
        }

        let (Some(first), Some(second)) = (sources.first().cloned(), sources.get(1).cloned())
        else {
            self.transition_demo = None;
            return;
        };

        let reuse = matches!(&self.transition_demo, Some(demo)
            if (demo.out.width, demo.out.height) == size);

        if !reuse {
            let targets = (
                Offscreen::new(&self.gpu, size.0, size.1),
                Offscreen::new(&self.gpu, size.0, size.1),
                Offscreen::new(&self.gpu, size.0, size.1),
            );

            let (Ok(from), Ok(to), Ok(out)) = targets else {
                self.transition_demo = None;
                return;
            };

            self.transition_demo = Some(TransitionDemo {
                from,
                to,
                out,
                kind,
                started: Instant::now(),
            });

            let placement = Placement::default();
            let ok = {
                let demo = self.transition_demo.as_mut().unwrap();
                paint_first_frame(&self.gpu, &self.manager, &first, &mut demo.from, placement)
                    .is_ok()
                    && paint_first_frame(&self.gpu, &self.manager, &second, &mut demo.to, placement)
                        .is_ok()
            };

            if !ok {
                self.transition_demo = None;
            }
            return;
        }

        if let Some(demo) = self.transition_demo.as_mut() {
            demo.kind = kind;
            demo.started = Instant::now();
        }
    }

    fn close_picker(&mut self) {
        self.picker_open = false;
        self.sequence_open = false;
        self.sequence_scroll = 0.0;
        self.picker_selection.clear();

        self.picker_preview = None;
        self.picker_hover = None;
        self.picker_failed = None;
        self.search_active = false;

        self.transition_demo = None;
    }

    fn apply_single(&mut self, item: usize) {
        let Some(entry) = self.library.items().get(item) else { return };
        let prepared = entry.prepared.clone();
        let source = entry.source.clone();

        let tab = self.selected;
        if let Some(tab) = self.tabs.get_mut(tab) {
            tab.entry.slides = vec![config::Slide {
                wallpaper: prepared,
                source,
                ..Default::default()
            }];
            tab.slide = 0;
        }

        self.preview = None;
        self.unreadable = None;
        self.saved = false;
        self.close_picker();
    }

    fn apply_selection(&mut self) {
        let slides: Vec<_> = self
            .picker_selection
            .iter()
            .filter_map(|index| self.library.items().get(*index))
            .map(|entry| config::Slide {
                wallpaper: entry.prepared.clone(),
                source: entry.source.clone(),
                ..Default::default()
            })
            .collect();

        if slides.is_empty() {
            return;
        }

        let tab = self.selected;
        if let Some(tab) = self.tabs.get_mut(tab) {
            tab.entry.slides = slides;
            tab.slide = 0;
        }

        self.preview = None;
        self.unreadable = None;
        self.saved = false;
        self.close_picker();
    }

    fn update_hover_preview(&mut self, hovered: Option<usize>, size: (u32, u32)) {
        const DELAY_MS: f64 = 280.0;

        let Some(item) = hovered else {
            self.picker_hover = None;
            self.picker_preview = None;
            self.picker_failed = None;
            return;
        };

        match self.picker_hover {
            Some((current, _)) if current == item => {}
            _ => {
                self.picker_hover = Some((item, Instant::now()));
                self.picker_preview = None;
                self.picker_failed = None;
                return;
            }
        }

        if matches!(&self.picker_preview, Some((current, _)) if *current == item) {
            return;
        }
        if self.picker_failed == Some(item) { return; }

        let waited = self
            .picker_hover
            .map(|(_, since)| since.elapsed_ms())
            .unwrap_or(0.0);
        if waited < DELAY_MS {
            return;
        }

        let Some(entry) = self.library.items().get(item) else { return };
        let path = entry.prepared.clone();

        match Preview::open(&self.gpu, &self.manager, &path, size, 1.0) {
            Ok(preview) => { self.picker_preview = Some((item, preview)); self.ui.dirty = true; }
            Err(_) => self.picker_failed = Some(item),
        }
    }

    fn register_in_library(&mut self, prepared: &Path, source: Option<&Path>) {
        let Ok(info) = import::probe(prepared) else { return };

        let added = self.library.add(
            &self.gpu,
            &self.manager,
            prepared,
            source,
            (info.width, info.height),
            info.fps as f32,
        );

        if added.is_ok() {
            let _ = self.library.save();

            self.thumbnails.clear();
        }
    }
}

fn draw(app: &mut App, hwnd: HWND) -> Result<()> {
    app.poll_import();

    if app.tabs.is_empty() {
        app.ui.begin();
        app.ui.label_center(app.ui.bounds(), "No active monitor", theme::TEXT);
        return app.ui.end();
    }

    let bounds = app.ui.bounds();
    let mut body = bounds;

    let mut header = body.cut_top(TAB_HEIGHT);
    let gear_slot = header.cut_right(TAB_HEIGHT + 8.0);
    let language = app.config.language();
    let labels: Vec<String> = app
        .tabs
        .iter()
        .enumerate()
        .map(|(i, t)| t.label(i, language))
        .collect();

    app.ui.begin();

    let modal = app.options_open || app.picker_open || app.menu.is_some();
    app.ui.block_input(modal);

    if let Some(next) = app.ui.tabs("monitor", header, &labels, app.selected) {
        app.selected = next;
        app.preview = None;
    }

    app.ui.fill(gear_slot, theme::PANEL_SUNKEN);
    app.ui.fill(
        Rect::new(gear_slot.x, gear_slot.bottom() - 1.0, gear_slot.w, 1.0),
        theme::BORDER,
    );

    let gear = Rect::new(
        gear_slot.right() - GEAR_SIZE - 6.0,
        gear_slot.y + (gear_slot.h - GEAR_SIZE) / 2.0,
        GEAR_SIZE,
        GEAR_SIZE,
    );

    let footer = body.cut_bottom(FOOTER_HEIGHT);

    let sidebar = body.cut_right(SIDEBAR_WIDTH);
    app.ui.fill(sidebar, theme::PANEL);
    app.ui.fill(
        Rect::new(sidebar.x, sidebar.y, 1.0, sidebar.h),
        theme::BORDER,
    );

    let stage = body;
    app.ui.fill(stage, theme::WINDOW);

    let monitor = &app.tabs[app.selected].monitor;
    let aspect = monitor.width() as f32 / monitor.height().max(1) as f32;
    let preview_rect = stage.inset(28.0).fit_aspect(aspect);

    let preview_size = (
        preview_rect.w.max(16.0) as u32,
        preview_rect.h.max(16.0) as u32,
    );
    app.sync_preview(preview_size);

    let placement = app.placement();
    let mut placement_changed = std::mem::take(&mut app.preview_dirty);
    let mouse = app.ui.input.mouse;

    let drag = app.ui.drag_area("preview", preview_rect);

    if app.ui.input.right_pressed && !modal {
        app.menu = preview_rect
            .contains(app.ui.input.mouse)
            .then_some(app.ui.input.mouse);
    }

    if drag.double_click && !modal {
        app.editing = !app.editing;
        app.selected_layer = None;
        app.grab = None;
    }
    let editing = app.editing;
    let video_size = app.preview.as_ref().map(|p| (p.info.width, p.info.height));

    if let (Some(video), Some(entry)) = (video_size, app.tabs[app.selected].slide_mut()) {
        if drag.dragging && !editing && (drag.delta.0 != 0.0 || drag.delta.1 != 0.0) {
            let (sx, sy) = entry.placement().sampling_scale(video, preview_size);
            entry.mode = config::Mode::Custom;
            entry.x += sx * drag.delta.0 / preview_rect.w;
            entry.y += sy * drag.delta.1 / preview_rect.h;
            placement_changed = true;
        }

        if drag.wheel != 0.0 {
            let before = entry.placement().sampling_scale(video, preview_size);
            entry.mode = config::Mode::Custom;
            entry.scale = (entry.scale * 1.12f32.powf(drag.wheel)).clamp(0.2, 8.0);
            let after = entry.placement().sampling_scale(video, preview_size);

            let cx = (mouse.0 - preview_rect.x) / preview_rect.w - 0.5;
            let cy = (mouse.1 - preview_rect.y) / preview_rect.h - 0.5;
            entry.x -= cx * (after.0 - before.0);
            entry.y -= cy * (after.1 - before.1);
            placement_changed = true;
        }
    }

    let media_size = app.preview.as_ref().map(|p| (p.info.width, p.info.height));
    let (canvas_changed, guides) = if modal {
        (false, Vec::new())
    } else {
        canvas::interact(app, preview_rect, stage, media_size)
    };
    if canvas_changed {
        placement_changed = true;
        app.saved = false;
    }

    let placement = if placement_changed {
        app.placement()
    } else {
        placement
    };

    let mut rebuilt = false;

    if let Some(preview) = app.preview.as_mut().filter(|_| !modal) {
        let drew = match preview.advance(&app.gpu, placement) {
            Ok(drew) => drew,
            Err(e) => {
                app.status = e.message();
                app.unreadable = Some(preview.path.clone());
                false
            }
        };

        rebuilt = drew;
        if !drew && placement_changed {
            rebuilt = preview.repaint(&app.gpu, placement).is_ok();
        }
    }

    if placement_changed {
        app.saved = false;
    }

    if app.unreadable.is_some() { app.preview = None; }

    if rebuilt {
        canvas::paint_layers(app, preview_size);
    }
    draw_preview(app, preview_rect, drag.hovered);
    canvas::draw_overlay(app, preview_rect, stage, media_size, &guides);

    draw_sidebar(app, sidebar.inset_xy(18.0, 16.0));
    draw_footer(app, footer);

    app.ui.block_input(false);

    if app.menu.is_some() {
        draw_framing_menu(app);
    }

    if app.ui.icon_button("gear", gear, &app.icons.gear, 17.0, app.options_open) {
        app.options_open = !app.options_open;
        app.picker_open = false;
    }

    if app.options_open {
        draw_options(app, bounds, gear);
    }

    if app.picker_open {
        draw_picker(app, bounds, hwnd);
    }

    app.ui.end()
}

fn draw_options(app: &mut App, bounds: Rect, gear: Rect) {
    const WIDTH: f32 = 268.0;
    const ROW: f32 = 34.0;

    let height = ROW * 6.0 + 192.0;
    let panel = Rect::new(
        (gear.right() - WIDTH).max(8.0),
        gear.bottom() + 6.0,
        WIDTH,
        height,
    );

    if app.ui.input.pressed && !panel.contains(app.ui.input.mouse) && !gear.contains(app.ui.input.mouse)
    {
        app.options_open = false;
        return;
    }

    let _ = bounds;
    app.ui.fill_round(panel, 8.0, theme::PANEL_RAISED);
    app.ui.stroke_round(panel, 8.0, theme::BORDER_STRONG, 1.0);

    let mut body = panel.inset_xy(14.0, 10.0);

    let mut start_with_windows = app.autostart;
    if app.ui.checkbox(
        "autostart",
        body.cut_top(ROW),
        app.t(Text::StartWithWindows),
        &mut start_with_windows,
    ) {
        match autostart::set(start_with_windows) {
            Ok(()) => app.autostart = start_with_windows,
            Err(e) => app.status = format!("{}: {}", app.t(Text::CannotChangeStartup), e.message()),
        }
    }

    let mut close_to_tray = app.config.close_to_tray;
    if app.ui.checkbox(
        "tray",
        body.cut_top(ROW),
        app.t(Text::CloseToTray),
        &mut close_to_tray,
    ) {
        app.config.close_to_tray = close_to_tray;
        app.saved = false;
    }

    let mut pause = app.config.pause_when_fullscreen;
    if app.ui.checkbox(
        "pausar",
        body.cut_top(ROW),
        app.t(Text::PauseFullscreen),
        &mut pause,
    ) {
        app.config.pause_when_fullscreen = pause;
        app.saved = false;
    }

    let mut optimize = app.config.optimize_wallpaper;
    if app.ui.checkbox(
        "otimizar",
        body.cut_top(ROW),
        app.t(Text::OptimizeOnImport),
        &mut optimize,
    ) {
        app.config.optimize_wallpaper = optimize;
        app.saved = false;
    }

    let mut battery = app.config.pause_on_battery;
    if app.ui.checkbox("battery", body.cut_top(ROW), app.t(Text::PauseBattery), &mut battery) {
        app.config.pause_on_battery = battery; app.saved = false;
    }
    let mut saver = app.config.pause_on_energy_saver;
    if app.ui.checkbox("energy-saver", body.cut_top(ROW), app.t(Text::PauseEnergySaver), &mut saver) {
        app.config.pause_on_energy_saver = saver; app.saved = false;
    }
    if app.ui.button("diagnostics", body.cut_top(28.0), app.t(Text::Diagnostics)) {
        show_info(app.hwnd, &walllit::diagnostics::report());
    }
    body.skip(4.0);
    if app.ui.button("cleanup", body.cut_top(28.0), app.t(Text::Storage)) { app.cleanup(); }

    body.skip(6.0);
    app.ui.section(body.cut_top(18.0), app.t(Text::LanguageLabel));
    body.skip(6.0);

    let current = app.config.language();
    let mut chosen = None;

    let gap = 6.0;
    let column = (body.w - gap) / 2.0;
    let mut row = body.cut_top(28.0);

    for (index, language) in Language::ALL.iter().enumerate() {
        if index == 2 {
            body.skip(gap);
            row = body.cut_top(28.0);
        }

        let cell = row.cut_left(column);
        row.cut_left(gap);

        if app.ui.toggle(
            &format!("idioma{index}"),
            cell,
            language.label(),
            *language == current,
        ) {
            chosen = Some(*language);
        }
    }

    if let Some(language) = chosen {
        app.config.language = Some(language);
        app.saved = false;
    }
}

fn draw_preview(app: &mut App, rect: Rect, hovered: bool) {
    let frame = rect.inset(-1.0);
    app.ui.fill(frame, theme::PANEL_SUNKEN);

    match app.preview.as_ref() {
        Some(preview) => {
            let _ = app.ui.image(&preview.offscreen.texture, rect);
        }
        None => {
            app.ui.fill(rect, theme::PANEL_SUNKEN);
            let mut center = rect;
            let line = center.cut_top(rect.h * 0.5);
            app.ui.label_center(
                Rect::new(line.x, line.bottom() - 14.0, line.w, 28.0),
                app.t(Text::NoVideoChosen),
                theme::TEXT_FAINT,
            );
        }
    }

    app.ui.stroke(
        frame,
        if hovered { theme::BORDER_STRONG } else { theme::BORDER },
        1.0,
    );

    let monitor = &app.tabs[app.selected].monitor;
    let caption = format!(
        "{} × {}  ·  {} Hz",
        monitor.width(),
        monitor.height(),
        monitor.refresh_hz
    );
    let strip = Rect::new(rect.x, rect.bottom() + 8.0, rect.w, 18.0);
    app.ui.label_small(strip, &caption, theme::TEXT_FAINT);

    if app.preview.is_some() {
        app.ui
            .label_small_right(strip, app.t(Text::PreviewHint), theme::TEXT_FAINT);
    }
}

fn draw_sidebar(app: &mut App, panel: Rect) {
    let wheel = app.ui.wheel_over(panel);
    if wheel != 0.0 {
        app.sidebar_scroll =
            (app.sidebar_scroll - wheel * 48.0).clamp(0.0, app.sidebar_overflow);
    }

    app.ui.push_clip(panel);
    let visible = panel.h;
    let top = panel.y - app.sidebar_scroll;
    let mut panel = Rect::new(panel.x, top, panel.w, 20_000.0);

    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionSource));
    panel.skip(6.0);

    let busy = app.job.is_some();
    let choose = panel.cut_top(34.0);

    if busy {
        app.ui.fill_round(choose, 4.0, theme::PANEL_SUNKEN);
        app.ui.stroke_round(choose, 4.0, theme::BORDER, 1.0);
        app.ui
            .label_center(choose, app.t(Text::Preparing), theme::TEXT_DIM);
    } else if app.ui.button("choose", choose, app.t(Text::ChooseVideo)) {
        app.picker_open = true;
        app.picker_scroll = 0.0;
        app.options_open = false;
    }

    panel.skip(8.0);

    let tab = &app.tabs[app.selected];
    let name = match tab.slide().map(config::Slide::display_name) {
        Some(name) if !name.is_empty() => name,
        _ => app.t(Text::NoFile).to_string(),
    };
    app.ui.label_small(panel.cut_top(18.0), &name, theme::TEXT);

    let detail = match app.preview.as_ref() {
        Some(p) => format!(
            "{} × {}  ·  {:.0} fps",
            p.info.width,
            p.info.height,
            p.info.fps()
        ),
        None => "-".into(),
    };
    app.ui
        .label_small(panel.cut_top(18.0), &detail, theme::TEXT_FAINT);

    if let Some(job) = app.job.as_ref() {
        panel.skip(10.0);
        app.ui.progress(
            panel.cut_top(28.0),
            job.progress.fraction(),
            app.t(Text::Converting),
        );
    }

    if app.tabs[app.selected].has_video() {
        panel.skip(4.0);
        if app.ui.button("remove", panel.cut_top(28.0), app.t(Text::RemoveFromMonitor)) {
            app.tabs[app.selected].entry.slides.clear();
            app.tabs[app.selected].slide = 0;
            app.preview = None;
            app.saved = false;
        }
    }

    panel.skip(20.0);

    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionFraming));
    panel.skip(8.0);

    app.ui.label_small(
        panel.cut_top(18.0),
        app.t(Text::FramingHint),
        theme::TEXT_FAINT,
    );

    panel.skip(16.0);

    let mut changed = false;

    let (label_zoom, label_x, label_y) = (
        app.t(Text::Zoom),
        app.t(Text::Horizontal),
        app.t(Text::Vertical),
    );

    let current = app
        .tabs[app.selected]
        .slide()
        .map(|s| (s.scale, s.x, s.y));

    if let Some((mut scale, mut x, mut y)) = current {
        let mut touched = false;

        if app.ui.slider(
            "zoom",
            panel.cut_top(38.0),
            label_zoom,
            &mut scale,
            (0.2, 4.0),
            |v| format!("{v:.2}x"),
        ) {
            touched = true;
        }

        panel.skip(6.0);
        if app.ui.slider(
            "offx",
            panel.cut_top(38.0),
            label_x,
            &mut x,
            (-1.0, 1.0),
            |v| format!("{v:+.3}"),
        ) {
            touched = true;
        }

        panel.skip(6.0);
        if app.ui.slider(
            "offy",
            panel.cut_top(38.0),
            label_y,
            &mut y,
            (-1.0, 1.0),
            |v| format!("{v:+.3}"),
        ) {
            touched = true;
        }

        if touched {
            if let Some(entry) = app.tabs[app.selected].slide_mut() {
                entry.scale = scale;
                entry.x = x;
                entry.y = y;

                entry.mode = config::Mode::Custom;
            }
            changed = true;
        }
    }

    panel.skip(8.0);
    if app.ui.button("recenter", panel.cut_top(28.0), app.t(Text::Recenter)) {
        if let Some(entry) = app.tabs[app.selected].slide_mut() {
            entry.x = 0.0;
            entry.y = 0.0;
        }
        changed = true;
    }

    panel.skip(20.0);

    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionPlayback));
    panel.skip(8.0);

    {
        let mut speed = app
            .tabs[app.selected]
            .slide()
            .map(|s| s.speed)
            .unwrap_or(1.0);

        if app.ui.slider(
            "speed",
            panel.cut_top(38.0),
            app.t(Text::Speed),
            &mut speed,
            (0.1, 3.0),
            |v| format!("{v:.2}x"),
        ) {
            if let Some(entry) = app.tabs[app.selected].slide_mut() {
                entry.speed = speed;
            }
            changed = true;
            if let Some(preview) = app.preview.as_mut() {
                preview.set_speed(speed);
            }
        }
    }

    if changed {
        app.saved = false;
    }

    panel.skip(20.0);
    canvas::draw_panel(app, &mut panel);

    app.ui.pop_clip();

    app.sidebar_overflow = (panel.y - top - visible).max(0.0);
    app.sidebar_scroll = app.sidebar_scroll.clamp(0.0, app.sidebar_overflow);
}

fn draw_framing_menu(app: &mut App) {
    const ROW: f32 = 30.0;
    const WIDTH: f32 = 196.0;

    let Some(at) = app.menu else { return };

    let items = FitMode::ALL.len() as f32;
    let height = ROW * (items + 1.0) + 18.0;

    let bounds = Rect::new(0.0, 0.0, app.size.0 as f32, app.size.1 as f32);
    let x = at.0.min(bounds.right() - WIDTH - 8.0).max(8.0);
    let y = at.1.min(bounds.bottom() - height - 8.0).max(8.0);
    let panel = Rect::new(x, y, WIDTH, height);

    if app.ui.input.pressed && !panel.contains(app.ui.input.mouse) {
        app.menu = None;
        return;
    }
    if app.ui.input.right_pressed && !panel.contains(app.ui.input.mouse) {
        app.menu = None;
        return;
    }

    app.ui.fill_round(panel, 8.0, theme::PANEL_RAISED);
    app.ui.stroke_round(panel, 8.0, theme::BORDER_STRONG, 1.0);

    let mut body = panel.inset_xy(9.0, 9.0);
    let current: FitMode = app.placement().mode;
    let mut chosen = None;

    for (index, mode) in FitMode::ALL.iter().enumerate() {
        let label = language::t(
            app.config.language(),
            match mode {
                FitMode::Fill => Text::ModeFill,
                FitMode::Fit => Text::ModeFit,
                FitMode::Stretch => Text::ModeStretch,
                FitMode::Center => Text::ModeCenter,
                FitMode::Custom => Text::ModeFree,
            },
        );

        if app.ui.toggle(
            &format!("menu-mode{index}"),
            body.cut_top(ROW).inset_xy(0.0, 1.0),
            label,
            *mode == current,
        ) {
            chosen = Some(*mode);
        }
    }

    body.skip(4.0);
    let reset = app.ui.button("menu-reset", body.cut_top(ROW), app.t(Text::ResetFraming));

    if reset {
        if let Some(slide) = app.tabs[app.selected].slide_mut() {
            slide.mode = config::Mode::Fill;
            slide.scale = 1.0;
            slide.x = 0.0;
            slide.y = 0.0;
            slide.stretch_x = 1.0;
            slide.stretch_y = 1.0;
        }
        app.preview_dirty = true;
        app.saved = false;
        app.menu = None;
        return;
    }

    if let Some(mode) = chosen {
        if let Some(slide) = app.tabs[app.selected].slide_mut() {
            slide.mode = mode.into();
        }
        app.preview_dirty = true;
        app.saved = false;
        app.menu = None;
    }
}

fn draw_footer(app: &mut App, rect: Rect) {
    app.ui.fill(rect, theme::PANEL);
    app.ui
        .fill(Rect::new(rect.x, rect.y, rect.w, 1.0), theme::BORDER);

    let mut row = rect.inset_xy(18.0, 0.0);

    let apply = row.cut_right(110.0).inset_xy(0.0, 12.0);
    row.cut_right(8.0);
    let close = row.cut_right(100.0).inset_xy(0.0, 12.0);
    row.cut_right(16.0);

    if !app.status.is_empty() {
        app.ui.label_small(row, &app.status, theme::TEXT_FAINT);
    }

    if app.ui.button("close", close, app.t(Text::Close)) {
        unsafe { PostQuitMessage(0) };
    }

    let label = if app.saved {
        app.t(Text::Saved)
    } else {
        app.t(Text::Apply)
    };
    if app.ui.button_primary("apply", apply, label) {
        app.save();
    }
}

fn draw_picker(app: &mut App, bounds: Rect, hwnd: HWND) {
    const COLUMNS: usize = 3;
    const GAP: f32 = 14.0;
    const CAPTION: f32 = 38.0;

    app.ui.dim(bounds, 0.55);

    let panel = Rect::new(
        bounds.x + (bounds.w - 780.0).max(40.0) / 2.0,
        bounds.y + (bounds.h - 540.0).max(40.0) / 2.0,
        780.0_f32.min(bounds.w - 40.0),
        540.0_f32.min(bounds.h - 40.0),
    );

    let sub_open = app.sequence_open;
    if !sub_open && app.ui.input.pressed && !panel.contains(app.ui.input.mouse) {
        app.close_picker();
        return;
    }

    app.ui.fill_round(panel, 10.0, theme::PANEL);
    app.ui.stroke_round(panel, 10.0, theme::BORDER_STRONG, 1.0);

    let mut body = panel.inset_xy(20.0, 16.0);
    app.ui.block_input(sub_open);

    let mut head = body.cut_top(30.0);
    let close = head.cut_right(30.0);
    let title = head.cut_left(150.0);
    app.ui.title(title, app.t(Text::PickerTitle), theme::TEXT);

    head.cut_left(8.0);
    let sequence_button = head.cut_left(200.0);
    head.cut_left(8.0);
    let paste_button = head.cut_left(190.0);

    if app.ui.button("btn-sequence", sequence_button, app.t(Text::SequenceSettings)) {
        app.sequence_open = true;
        app.sequence_scroll = 0.0;
    }

    let pasting = app.paste_mode;
    if app.ui.toggle("btn-paste", paste_button, app.t(Text::PasteIntoComposition), pasting) {
        app.paste_mode = !pasting;
    }
    if app.ui.button("close-grid", close, "×") {
        app.close_picker();
        return;
    }

    body.skip(8.0);
    let mut tools = body.cut_top(30.0);
    let favorites = tools.cut_right(110.0);
    tools.cut_right(8.0);
    let query_label = if app.search.is_empty() { app.t(Text::Search).to_string() } else { app.search.clone() };
    if app.ui.input.pressed { app.search_active = tools.contains(app.ui.input.mouse) && !sub_open; }
    app.ui.button("search", tools, &query_label);
    if app.search_active { app.ui.stroke_round(tools, 4.0, theme::ACCENT, 1.0); }
    if app.ui.toggle("favorites", favorites, app.t(Text::Favorites), app.favorites_only) {
        app.favorites_only = !app.favorites_only; app.picker_scroll = 0.0;
    }
    body.skip(8.0);
    let hint = grid_hint(app);
    app.ui.label_small(body.cut_top(18.0), &hint, theme::TEXT_FAINT);
    body.skip(10.0);

    let mut save_button = None;
    if !app.picker_selection.is_empty() {
        let mut bar = body.cut_bottom(42.0);
        bar.skip(8.0);
        save_button = Some(bar.cut_right(150.0));

        let count = app.picker_selection.len();
        let text = if count == 1 {
            app.t(Text::OneInSequence).to_string()
        } else {
            app.t(Text::ManyInSequence).replace("{}", &count.to_string())
        };
        app.ui.label_small(bar, &text, theme::TEXT_DIM);
        let remove = body.cut_bottom(28.0);
        if app.ui.button("remove-from-library", remove, app.t(Text::RemoveLibrary)) {
            app.remove_selected();
            return;
        }
    }

    let viewport = body;
    let cell_width = (viewport.w - GAP * (COLUMNS - 1) as f32) / COLUMNS as f32;
    let cell_height = cell_width * 9.0 / 16.0 + CAPTION;

    let visible = app.library.matching(&app.search, app.favorites_only);
    let slots = visible.len() + 1;
    let rows = slots.div_ceil(COLUMNS);
    let content = rows as f32 * (cell_height + GAP);
    let overflow = (content - viewport.h).max(0.0);

    let wheel = app.ui.wheel_over(viewport);
    if wheel != 0.0 {
        app.picker_scroll -= wheel * 60.0;
    }
    app.picker_scroll = app.picker_scroll.clamp(0.0, overflow);

    app.ui.push_clip(viewport);

    let mut chosen = None;
    let mut toggled = None;
    let mut add_requested = false;
    let mut hovered_now = None;

    for index in 0..slots {
        let row = index / COLUMNS;
        let column = index % COLUMNS;

        let cell = Rect::new(
            viewport.x + column as f32 * (cell_width + GAP),
            viewport.y + row as f32 * (cell_height + GAP) - app.picker_scroll,
            cell_width,
            cell_height,
        );

        if cell.bottom() < viewport.y || cell.y > viewport.bottom() {
            continue;
        }

        if index == 0 {
            if draw_add_slot(app, cell) {
                add_requested = true;
            }
            continue;
        }

        let item = visible[index - 1];
        match draw_item_slot(app, cell, item) {
            SlotAction::Pick => chosen = Some(item),
            SlotAction::Toggle => toggled = Some(item),
            SlotAction::None => {}
        }

        if cell.contains(app.ui.input.mouse) && viewport.contains(app.ui.input.mouse) {
            hovered_now = Some(item);
        }
    }

    app.ui.pop_clip();
    app.update_hover_preview(hovered_now, (cell_width as u32, (cell_width * 9.0 / 16.0) as u32));

    if let Some(item) = toggled {
        match app.picker_selection.iter().position(|i| *i == item) {
            Some(at) => {
                app.picker_selection.remove(at);
            }
            None => app.picker_selection.push(item),
        }
    }

    if let Some(rect) = save_button {
        if app.ui.button_primary("save-list", rect, app.t(Text::Save)) {
            app.apply_selection();
        }
    }

    if let Some(index) = chosen {
        if app.paste_mode {
            canvas::paste_layer(app, index);
        } else {
            app.apply_single(index);
        }
    }

    if add_requested {
        if let Some(path) = pick_video(hwnd) {
            let tab = app.selected;
            app.close_picker();
            app.choose_video(tab, path);
        }
    }

    app.ui.block_input(false);

    if app.sequence_open {
        draw_sequence_panel(app, bounds);
    }
}

fn paint_first_frame(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    path: &Path,
    target: &mut Offscreen,
    placement: Placement,
) -> Result<()> {
    target.clear(gpu, [0.0, 0.0, 0.0, 1.0]);

    if image::is_image(path) {
        let picture = StillImage::load(gpu, path)?;
        return target.paint_image(gpu, &picture, placement);
    }

    let mut source = VideoSource::open(&path.to_string_lossy(), manager)?;
    let info = source.info;

    for _ in 0..32 {
        if let Some(frame) = source.next_frame()? {
            return target.paint(
                gpu,
                FrameRef {
                    texture: &frame.texture,
                    subresource: frame.subresource,
                    size: (info.width, info.height),
                    matrix: info.matrix,
                    full_range: info.full_range,
                },
                placement,
            );
        }
    }

    Ok(())
}

fn draw_sequence_panel(app: &mut App, over: Rect) {
    const WIDTH: f32 = 760.0;
    const HEIGHT: f32 = 560.0;
    const COLUMN: f32 = 320.0;

    app.ui.dim(over, 0.55);

    let panel = Rect::new(
        over.x + (over.w - WIDTH).max(20.0) / 2.0,
        over.y + (over.h - HEIGHT).max(20.0) / 2.0,
        WIDTH.min(over.w - 20.0),
        HEIGHT.min(over.h - 20.0),
    );

    if app.ui.input.pressed && !panel.contains(app.ui.input.mouse) {
        app.sequence_open = false;
        return;
    }

    app.ui.fill_round(panel, 10.0, theme::PANEL_RAISED);
    app.ui.stroke_round(panel, 10.0, theme::BORDER_STRONG, 1.0);

    let mut body = panel.inset_xy(18.0, 14.0);

    let mut head = body.cut_top(28.0);
    let close = head.cut_right(30.0);
    app.ui.title(head, app.t(Text::SequenceTitle), theme::TEXT);
    if app.ui.button("close-sequence", close, "×") {
        app.sequence_open = false;
        return;
    }

    body.skip(12.0);

    let left = body.cut_left(COLUMN.min(body.w * 0.5));
    let mut gap = body.cut_left(24.0);

    let rule = gap.cut_left(gap.w * 0.5).cut_right(1.0);
    app.ui.fill_round(rule, 0.0, theme::BORDER);

    draw_transition_column(app, left);
    draw_timing_column(app, body);
}

fn draw_transition_column(app: &mut App, mut body: Rect) {
    const ROW: f32 = 30.0;

    app.ui
        .section(body.cut_top(18.0), app.t(Text::TransitionTitle));
    body.skip(8.0);

    let stage = body.cut_top((body.w * 9.0 / 16.0).min(168.0));
    let demo_size = (stage.w.max(16.0) as u32, stage.h.max(16.0) as u32);

    app.ui.fill_round(stage, 4.0, theme::PANEL_SUNKEN);
    draw_transition_demo(app, stage);
    app.ui.stroke_round(stage, 4.0, theme::BORDER, 1.0);

    body.skip(10.0);

    let current = app.tabs[app.selected].entry.transition;
    let mut chosen = None;

    for (index, transition) in config::Transition::ALL.iter().enumerate() {
        let row = body.cut_top(ROW);
        let label = language::t(
            app.config.language(),
            match transition {
                config::Transition::None => Text::TransitionNone,
                config::Transition::Fade => Text::TransitionFade,
                config::Transition::LeftToRight => Text::TransitionLeftRight,
                config::Transition::RightToLeft => Text::TransitionRightLeft,
                config::Transition::TopToBottom => Text::TransitionTopBottom,
                config::Transition::BottomToTop => Text::TransitionBottomTop,
                config::Transition::Rebuild => Text::TransitionRebuild,
                config::Transition::Morph => Text::TransitionMorph,
            },
        );

        if app.ui.toggle(
            &format!("transition{index}"),
            row.inset_xy(0.0, 2.0),
            label,
            *transition == current,
        ) {
            chosen = Some(*transition);
        }
    }

    if let Some(transition) = chosen {
        app.tabs[app.selected].entry.transition = transition;
        app.saved = false;

        app.start_transition_demo(transition as u32, demo_size);
    }
}

fn draw_transition_demo(app: &mut App, rect: Rect) {
    let Some(demo) = app.transition_demo.as_ref() else {
        app.ui
            .label_center(rect, app.t(Text::TransitionPreviewHint), theme::TEXT_FAINT);
        return;
    };

    let elapsed_ms = demo.started.elapsed_ms();
    let total_ms = DEMO_100NS as f64 / 10_000.0;
    let progress = (elapsed_ms / total_ms).clamp(0.0, 1.0) as f32;

    let size = (demo.out.width, demo.out.height);
    let blended = walllit::renderer::blend_pass(
        &app.gpu,
        demo.out.view_target(),
        size,
        &demo.from,
        &demo.to,
        demo.kind,
        progress,
    );

    if blended.is_ok() {
        let _ = app.ui.image(&demo.out.texture, rect);
    }
}

fn draw_timing_column(app: &mut App, body: Rect) {
    let wheel = app.ui.wheel_over(body);
    if wheel != 0.0 {
        app.sequence_scroll =
            (app.sequence_scroll - wheel * 48.0).clamp(0.0, app.sequence_overflow);
    }

    app.ui.push_clip(body);

    let top = body.y - app.sequence_scroll;
    let mut flow = Rect::new(body.x, top, body.w, 20_000.0);

    app.ui.section(flow.cut_top(18.0), app.t(Text::TimerTitle));
    flow.skip(6.0);
    app.ui
        .label_small(flow.cut_top(32.0), app.t(Text::TimerHint), theme::TEXT_FAINT);
    flow.skip(4.0);

    let mut uniform = app.tabs[app.selected].entry.uniform_hold;
    if app.ui.checkbox(
        "hold-uniform",
        flow.cut_top(28.0),
        app.t(Text::ApplyToAll),
        &mut uniform,
    ) {
        app.tabs[app.selected].entry.uniform_hold = uniform;
        app.saved = false;
    }
    flow.skip(8.0);

    if uniform {
        let hold = app.tabs[app.selected].entry.hold;
        let label = app.t(Text::AllSources).to_string();

        if let Some(next) = draw_hold_card(app, &mut flow, "all", &label, None, hold, true) {
            app.tabs[app.selected].entry.hold = next;
            app.saved = false;
        }
    } else {
        let sources: Vec<(String, Option<PathBuf>, config::Hold, bool)> = app.tabs
            [app.selected]
            .entry
            .slides
            .iter()
            .map(|slide| {
                let hold = slide.hold.unwrap_or(app.tabs[app.selected].entry.hold);
                (
                    slide.display_name(),
                    thumbnail_of(app, &slide.wallpaper),
                    hold,
                    !image::is_image(&slide.wallpaper),
                )
            })
            .collect();

        if sources.is_empty() {
            app.ui
                .label_small(flow.cut_top(20.0), app.t(Text::NoSources), theme::TEXT_FAINT);
            flow.skip(8.0);
        }

        for (index, (name, thumbnail, hold, is_video)) in sources.into_iter().enumerate() {
            let next = draw_hold_card(
                app,
                &mut flow,
                &format!("source{index}"),
                &name,
                thumbnail.as_deref(),
                hold,
                is_video,
            );

            if let Some(next) = next {
                if let Some(slide) = app.tabs[app.selected].entry.slides.get_mut(index) {
                    slide.hold = Some(next);
                }
                app.saved = false;
            }
        }
    }

    draw_schedule(app, &mut flow);

    app.ui.pop_clip();

    app.sequence_overflow = (flow.y - top - body.h).max(0.0);
    app.sequence_scroll = app.sequence_scroll.clamp(0.0, app.sequence_overflow);
}

fn draw_hold_card(
    app: &mut App,
    flow: &mut Rect,
    key: &str,
    name: &str,
    thumbnail: Option<&Path>,
    hold: config::Hold,
    allow_loops: bool,
) -> Option<config::Hold> {
    let hold = match hold {
        config::Hold::Loops(_) if !allow_loops => config::Hold::IMAGE_DEFAULT,
        other => other,
    };

    let height = if allow_loops { 126.0 } else { 92.0 };
    let card = flow.cut_top(height);
    flow.skip(8.0);

    app.ui.fill_round(card, 6.0, theme::PANEL);
    app.ui.stroke_round(card, 6.0, theme::BORDER, 1.0);

    let mut body = card.inset_xy(10.0, 9.0);

    let mut head = body.cut_top(28.0);
    if let Some(path) = thumbnail {
        let art = head.cut_left(48.0);
        draw_thumbnail(app, path, art.inset_xy(0.0, 1.0));
        head.cut_left(8.0);
    }
    app.ui.label(head, name, theme::TEXT);
    body.skip(6.0);

    let mut next = hold;

    if allow_loops {
        let mut row = body.cut_top(26.0);
        let half = (row.w - 8.0) / 2.0;
        let loops_button = row.cut_left(half);
        row.cut_left(8.0);
        let seconds_button = row.cut_left(half);

        let is_loops = matches!(hold, config::Hold::Loops(_));

        if app.ui.toggle(&format!("{key}-loops"), loops_button, app.t(Text::UnitLoops), is_loops)
            && !is_loops
        {
            next = config::Hold::Loops(3);
        }
        if app.ui.toggle(
            &format!("{key}-seconds"),
            seconds_button,
            app.t(Text::UnitSeconds),
            !is_loops,
        ) && is_loops
        {
            next = config::Hold::IMAGE_DEFAULT;
        }

        body.skip(6.0);
    }

    match next {
        config::Hold::Loops(times) => {
            let mut value = times as f32;
            if app.ui.slider(
                &format!("{key}-value"),
                body.cut_top(38.0),
                app.t(Text::LoopsOfVideo),
                &mut value,
                (1.0, 20.0),
                |v| format!("{v:.0}x"),
            ) {
                next = config::Hold::Loops(value.round().max(1.0) as u32);
            }
        }
        config::Hold::Seconds(seconds) => {
            let mut value = seconds;
            if app.ui.slider(
                &format!("{key}-value"),
                body.cut_top(38.0),
                app.t(Text::TimeOnScreen),
                &mut value,
                (5.0, 600.0),
                |v| format!("{v:.0} s"),
            ) {
                next = config::Hold::Seconds(value.round());
            }
        }
    }

    (next != hold).then_some(next)
}

fn thumbnail_of(app: &App, wallpaper: &Path) -> Option<PathBuf> {
    app.library
        .items()
        .iter()
        .find(|item| item.prepared == wallpaper)
        .map(|item| item.thumbnail.clone())
}

fn draw_schedule(app: &mut App, flow: &mut Rect) {
    flow.skip(8.0);
    app.ui.section(flow.cut_top(18.0), app.t(Text::UseSchedule));
    flow.skip(6.0);

    let mut scheduled = app.tabs[app.selected].entry.schedule.enabled;
    if app.ui.checkbox(
        "schedule",
        flow.cut_top(28.0),
        app.t(Text::UseSchedule),
        &mut scheduled,
    ) {
        let entry = &mut app.tabs[app.selected].entry;
        entry.schedule.enabled = scheduled;

        if scheduled && entry.schedule.entries.is_empty() {
            entry.schedule.entries = config::Schedule::day_and_night();
        }
        app.saved = false;
    }

    if !scheduled {
        return;
    }

    flow.skip(4.0);
    app.ui
        .label_small(flow.cut_top(30.0), app.t(Text::ScheduleHint), theme::TEXT_FAINT);
    flow.skip(4.0);

    let entries = app.tabs[app.selected].entry.schedule.entries.clone();
    let slides = app.tabs[app.selected].entry.slides.len().max(1);
    let mut changed_entries = entries.clone();
    let mut touched = false;

    for (index, entry) in entries.iter().enumerate() {
        let mut row = flow.cut_top(30.0);

        let earlier = row.cut_left(28.0);
        let time = row.cut_left(70.0);
        let later = row.cut_left(28.0);
        row.cut_left(10.0);
        let which = row.cut_left(120.0);

        if app.ui.button(&format!("earlier{index}"), earlier, "−") {
            changed_entries[index].minute = (entry.minute + 1440 - 30) % 1440;
            touched = true;
        }
        app.ui.label_center(time, &entry.label(), theme::TEXT);
        if app.ui.button(&format!("later{index}"), later, "+") {
            changed_entries[index].minute = (entry.minute + 30) % 1440;
            touched = true;
        }

        if app.ui.button(
            &format!("which{index}"),
            which,
            &format!("#{}", entry.slide + 1),
        ) {
            changed_entries[index].slide = (entry.slide + 1) % slides;
            touched = true;
        }

        flow.skip(4.0);
    }

    if app.ui.button("add-time", flow.cut_top(28.0), app.t(Text::AddTime)) {
        let next = entries.last().map(|e| (e.minute + 180) % 1440).unwrap_or(420);
        changed_entries.push(config::ScheduleEntry { minute: next, slide: 0 });
        touched = true;
    }

    if touched {
        app.tabs[app.selected].entry.schedule.entries = changed_entries;
        app.saved = false;
    }
}

enum SlotAction {
    None,

    Pick,

    Toggle,
}

fn grid_hint(app: &App) -> String {
    match app.library.items().len() {
        0 => app.t(Text::LibraryEmpty).into(),
        1 => app.t(Text::OneImported).into(),
        count => app.t(Text::ManyImported).replace("{}", &count.to_string()),
    }
}

fn draw_add_slot(app: &mut App, cell: Rect) -> bool {
    let mut cell = cell;
    let art = cell.cut_top(cell.w * 9.0 / 16.0);

    let clicked = app.ui.button("add", art, "");

    let hovered = art.contains(app.ui.input.mouse);
    let color = if hovered { theme::ACCENT } else { theme::TEXT_FAINT };
    app.ui.icon(&app.icons.plus, art, 34.0, color);

    app.ui
        .label_center(cell, app.t(Text::AddVideo), theme::TEXT_DIM);

    clicked
}

fn draw_item_slot(app: &mut App, cell: Rect, index: usize) -> SlotAction {
    const MARK: f32 = 24.0;
    const STAR: f32 = 30.0;

    let mut cell = cell;
    let art = cell.cut_top(cell.w * 9.0 / 16.0);

    let item = &app.library.items()[index];
    let name = item.name.clone();
    let detail = item.detail();
    let thumbnail = item.thumbnail.clone();
    let prepared = item.prepared.clone();
    let favorite = item.favorite;

    let in_use = app
        .tabs
        .get(app.selected)
        .map(|t| t.entry.slides.iter().any(|s| s.wallpaper == prepared))
        .unwrap_or(false);

    let order = app.picker_selection.iter().position(|i| *i == index);

    let mark = Rect::new(art.right() - MARK - 8.0, art.y + 8.0, MARK, MARK);
    let marked = app.ui.button(&format!("mark{index}"), mark, "");
    let star = Rect::new(art.x + 6.0, art.y + 6.0, STAR, STAR);
    let favorited = app.ui.button(&format!("favorite{index}"), star, "");

    let picked = app.ui.button(&format!("item{index}"), art, "") && !mark.contains(app.ui.input.mouse) && !star.contains(app.ui.input.mouse);
    let hovered = art.contains(app.ui.input.mouse);

    app.ui.fill_round(art, 4.0, theme::PANEL_SUNKEN);

    let playing = matches!(&app.picker_preview, Some((current, _)) if *current == index);

    if playing {
        if let Some((_, preview)) = app.picker_preview.as_mut() {
            let _ = preview.advance(&app.gpu, Placement::default());
        }
        if let Some((_, preview)) = app.picker_preview.as_ref() {
            let _ = app.ui.image(&preview.offscreen.texture, art);
        }
    } else {
        draw_thumbnail(app, &thumbnail, art);
    }

    let border = if order.is_some() {
        theme::ACCENT
    } else if in_use {
        theme::ACCENT_SUNK
    } else if hovered {
        theme::BORDER_STRONG
    } else {
        theme::BORDER
    };
    let width = if order.is_some() || in_use { 2.0 } else { 1.0 };
    app.ui.stroke_round(art, 4.0, border, width);

    draw_mark(app, mark, order);
    draw_star(app, star, favorite);
    if favorited {
        app.library.toggle_favorite(index);
        if let Err(e) = app.library.save() { app.library.toggle_favorite(index); app.status = e.to_string(); }
    }

    let mut caption = cell;
    app.ui.label_small(caption.cut_top(19.0), &name, theme::TEXT);
    app.ui
        .label_small(caption.cut_top(17.0), &detail, theme::TEXT_FAINT);

    if marked {
        SlotAction::Toggle
    } else if picked {
        SlotAction::Pick
    } else {
        SlotAction::None
    }
}

fn draw_thumbnail(app: &mut App, path: &Path, rect: Rect) {
    let key = path.to_path_buf();
    if app.thumbnails.get(&key).is_none() {
        let mut bitmap = app.ui.load_image(path).ok();
        let bytes = bitmap.as_ref().map(|b| { let size = unsafe { b.GetPixelSize() }; size.width as usize * size.height as usize * 4 }).unwrap_or(1);
        if bytes > 16 * 1024 * 1024 { bitmap = None; }
        app.thumbnails.insert(key.clone(), bitmap, if bytes > 16 * 1024 * 1024 { 1 } else { bytes });
    }

    if let Some(Some(bitmap)) = app.thumbnails.get(&key) {
        app.ui.draw_image(bitmap, rect);
    } else {
        app.ui.label_center(rect, app.t(Text::NoThumbnail), theme::TEXT_FAINT);
    }
}

fn draw_star(app: &mut App, rect: Rect, favorite: bool) {
    let hovered = rect.contains(app.ui.input.mouse);

    if favorite || hovered {
        app.ui.fill_round(rect, 6.0, walllit::ui::rgba(0x00_00_00, 0.45));
    }

    let color = if favorite {
        theme::ACCENT
    } else if hovered {
        theme::TEXT
    } else {
        walllit::ui::rgba(0xFF_FF_FF, 0.75)
    };

    if favorite {
        app.ui.icon(&app.icons.star, rect, 19.0, color);
    } else {
        app.ui.icon_outline(&app.icons.star, rect, 19.0, color, 1.6);
    }
}

fn draw_mark(app: &mut App, rect: Rect, order: Option<usize>) {
    match order {
        Some(position) => {
            app.ui.fill_round(rect, 4.0, theme::ACCENT);
            app.ui
                .label_center(rect, &format!("{}", position + 1), theme::TEXT);
        }
        None => {
            app.ui.fill_round(rect, 4.0, ui::rgba(0x00_00_00, 0.45));
            app.ui.stroke_round(rect, 4.0, theme::BORDER_STRONG, 1.0);
        }
    }
}

fn pick_video(owner: HWND) -> Option<PathBuf> {
    unsafe {
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;

        let filters = [
            COMDLG_FILTERSPEC {
                pszName: w!("Videos and images"),
                pszSpec: w!(
                    "*.mp4;*.m4v;*.mov;*.mkv;*.webm;*.avi;*.wmv;*.gif;*.jpg;*.jpeg;*.png;*.bmp;*.tif;*.tiff;*.webp;*.heic;*.avif"
                ),
            },
            COMDLG_FILTERSPEC {
                pszName: w!("All files"),
                pszSpec: w!("*.*"),
            },
        ];
        dialog.SetFileTypes(&filters).ok()?;
        dialog.SetTitle(w!("Choose video")).ok()?;

        dialog.Show(owner).ok()?;

        let item = dialog.GetResult().ok()?;
        let wide = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = wide.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(wide.0 as *const _));
        path
    }
}

fn create_swapchain(gpu: &Gpu, hwnd: HWND, size: (u32, u32)) -> Result<IDXGISwapChain1> {
    let dxgi_device: IDXGIDevice = gpu.device.cast()?;
    let adapter = unsafe { dxgi_device.GetAdapter() }?;
    let factory: IDXGIFactory2 = unsafe { adapter.GetParent() }?;

    let desc = DXGI_SWAP_CHAIN_DESC1 {
        Width: size.0,
        Height: size.1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
        BufferCount: 2,
        Scaling: DXGI_SCALING_NONE,
        SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
        AlphaMode: DXGI_ALPHA_MODE_IGNORE,
        ..Default::default()
    };

    unsafe { factory.CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None) }
}

fn create_window(pending: *mut Pending) -> Result<HWND> {
    let instance = unsafe { GetModuleHandleW(PCWSTR::null()) }?;

    let icon = unsafe { LoadIconW(instance, PCWSTR(APP_ICON_RESOURCE as *const u16)) }
        .unwrap_or_default();

    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
        lpfnWndProc: Some(wnd_proc),
        hInstance: instance.into(),
        hCursor: unsafe { LoadCursorW(None, IDC_ARROW) }?,
        hIcon: icon,
        hIconSm: icon,
        lpszClassName: WINDOW_CLASS,
        ..Default::default()
    };

    if unsafe { RegisterClassExW(&class) } == 0 {
        return Err(Error::from_win32());
    }

    let mut work = RECT::default();
    unsafe {
        let _ = SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut _),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        );
    }
    let x = work.left + ((work.right - work.left) - DEFAULT_SIZE.0) / 2;
    let y = work.top + ((work.bottom - work.top) - DEFAULT_SIZE.1) / 2;

    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            WINDOW_CLASS,
            w!("WallLit"),
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            DEFAULT_SIZE.0,
            DEFAULT_SIZE.1,
            None,
            None,
            instance,
            None,
        )
    }?;

    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, pending as isize);

        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
        let border = COLORREF(0x00_1D_1A_18);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR,
            &border as *const COLORREF as *const _,
            std::mem::size_of::<COLORREF>() as u32,
        );
    }

    Ok(hwnd)
}

fn client_size(hwnd: HWND) -> (u32, u32) {
    let mut rect = RECT::default();
    unsafe {
        let _ = GetClientRect(hwnd, &mut rect);
    }
    (
        (rect.right - rect.left).max(1) as u32,
        (rect.bottom - rect.top).max(1) as u32,
    )
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            let message = HSTRING::from(format!("{}\n\n{:?}", e.message(), e.code()));
            unsafe {
                MessageBoxW(None, &message, w!("WallLit"), MB_ICONERROR | MB_OK);
            }
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let Some(_lock) = instance::EngineLock::settings() else {
        if let Ok(existing) = unsafe { FindWindowW(WINDOW_CLASS, PCWSTR::null()) } {
            unsafe { let _ = ShowWindow(existing, SW_RESTORE); let _ = SetForegroundWindow(existing); }
        }
        return Ok(());
    };
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);

        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
    }

    let _mf = MediaFoundation::startup()?;

    let mut pending = Pending::default();
    let hwnd = create_window(&mut pending as *mut Pending)?;

    if !cfg!(debug_assertions) || std::env::var_os("WALLLIT_TEST_DATA_DIR").is_none() {
        autostart::repair_if_stale();
    }

    let size = client_size(hwnd);
    let mut app = App::new(hwnd, size)?;

    if let Some(path) = std::env::args().nth(1).map(PathBuf::from) {
        if path.is_file() {
            let selected = app.selected;
            app.choose_video(selected, path);
        }
    }

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }

    let timer = unsafe {
        CreateWaitableTimerExW(
            None,
            None,
            CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
            TIMER_ALL_ACCESS.0,
        )
    }?;

    let shutdown = instance::shutdown_signal()?;

    let mut was_minimized = false;
    loop {
        let interval_100ns = app.next_redraw(unsafe { IsIconic(hwnd).as_bool() });
        let signal = match interval_100ns {
            Some(interval) => unsafe {
                SetWaitableTimer(timer, &-interval, 0, None, None, false)?;
                MsgWaitForMultipleObjectsEx(
                    Some(&[shutdown.handle(), timer]),
                    INFINITE,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            },
            None => unsafe {
                MsgWaitForMultipleObjectsEx(
                    Some(&[shutdown.handle()]),
                    INFINITE,
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                )
            },
        };

        if signal == WAIT_OBJECT_0 {
            pending.quit = true;
            break;
        }

        if interval_100ns.is_none()
            || signal != windows::Win32::Foundation::WAIT_EVENT(WAIT_OBJECT_0.0 + 1)
        {
            let mut msg = MSG::default();
            unsafe {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT { pending.quit = true; break; }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        }

        if pending.quit {
            break;
        }

        let needs_settle = pending.input.pressed || pending.input.released || pending.input.wheel != 0.0
            || pending.resized.is_some() || !pending.text.is_empty() || pending.display_changed
            || app.job.as_ref().is_some_and(|job| job.worker.is_finished());

        if std::mem::take(&mut pending.display_changed) { app.refresh_displays(); }
        let text = std::mem::take(&mut pending.text);
        if app.search_active && app.picker_open {
            for character in String::from_utf16_lossy(&text).chars() {
                match character {
                    '\u{8}' => { app.search.pop(); }
                    '\u{1b}' | '\r' => app.search_active = false,
                    c if !c.is_control() && app.search.chars().count() < 100 => app.search.push(c),
                    _ => {},
                }
            }
            if !text.is_empty() { app.picker_scroll = 0.0; }
        }

        if unsafe { IsIconic(hwnd).as_bool() } {
            was_minimized = true;
            app.poll_import();
            continue;
        }

        if was_minimized {
            if let Some(preview) = app.preview.as_mut() {
                preview.deadline = Instant::now();
            }
            if let Some((_, preview)) = app.picker_preview.as_mut() {
                preview.deadline = Instant::now();
            }
            was_minimized = false;
        }

        if let Some(size) = pending.resized.take() {
            app.resize(size)?;
        }

        app.ui.input.mouse = pending.input.mouse;
        app.ui.input.down = pending.input.down;
        app.ui.input.pressed = pending.input.pressed;
        app.ui.input.released = pending.input.released;
        app.ui.input.wheel = pending.input.wheel;
        app.ui.input.double_click = pending.input.double_click;
        app.ui.input.right_pressed = pending.input.right_pressed;
        pending.input.pressed = false;
        pending.input.released = false;
        pending.input.double_click = false;
        pending.input.right_pressed = false;
        pending.input.wheel = 0.0;

        app.ui.dirty = false;
        draw(&mut app, hwnd)?;
        app.ui.dirty |= needs_settle;
        unsafe { app.swapchain.Present(1, DXGI_PRESENT(0)).ok()? };
    }

    unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(timer);
        let _ = DestroyWindow(hwnd);
    }

    if !app.config.close_to_tray {
        instance::request_quit();
    }

    Ok(())
}

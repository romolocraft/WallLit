use windows::core::*;
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{IDXGIDevice, IDXGISurface, IDXGISwapChain1};

use crate::icon::{Icon, Matrix3x2};
use crate::renderer::Gpu;

pub const fn rgb(hex: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((hex >> 16) & 0xFF) as f32 / 255.0,
        g: ((hex >> 8) & 0xFF) as f32 / 255.0,
        b: (hex & 0xFF) as f32 / 255.0,
        a: 1.0,
    }
}

pub const fn rgba(hex: u32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { a, ..rgb(hex) }
}

pub mod theme {
    use super::{rgb, rgba, D2D1_COLOR_F};

    pub const WINDOW: D2D1_COLOR_F = rgb(0x18_1A_1D);
    pub const PANEL: D2D1_COLOR_F = rgb(0x21_24_28);
    pub const PANEL_RAISED: D2D1_COLOR_F = rgb(0x2A_2E_33);
    pub const PANEL_SUNKEN: D2D1_COLOR_F = rgb(0x14_16_18);
    pub const BORDER: D2D1_COLOR_F = rgb(0x34_39_3F);
    pub const BORDER_STRONG: D2D1_COLOR_F = rgb(0x45_4B_52);

    pub const TEXT: D2D1_COLOR_F = rgb(0xE4_E7_EA);
    pub const TEXT_DIM: D2D1_COLOR_F = rgb(0x95_9C_A4);
    pub const TEXT_FAINT: D2D1_COLOR_F = rgb(0x67_6E_76);

    pub const ACCENT: D2D1_COLOR_F = rgb(0x3D_8B_FD);
    pub const ACCENT_HOVER: D2D1_COLOR_F = rgb(0x5A_9F_FF);
    pub const ACCENT_SUNK: D2D1_COLOR_F = rgb(0x2E_71_D6);

    pub const HOVER: D2D1_COLOR_F = rgba(0xFF_FF_FF, 0.06);
    pub const PRESSED: D2D1_COLOR_F = rgba(0x00_00_00, 0.20);

    pub const DANGER: D2D1_COLOR_F = rgb(0xE0_5A_5A);
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w * 0.5, self.y + self.h * 0.5)
    }

    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.x && p.0 < self.right() && p.1 >= self.y && p.1 < self.bottom()
    }

    pub fn inset(self, d: f32) -> Self {
        Self {
            x: self.x + d,
            y: self.y + d,
            w: (self.w - d * 2.0).max(0.0),
            h: (self.h - d * 2.0).max(0.0),
        }
    }

    pub fn inset_xy(self, dx: f32, dy: f32) -> Self {
        Self {
            x: self.x + dx,
            y: self.y + dy,
            w: (self.w - dx * 2.0).max(0.0),
            h: (self.h - dy * 2.0).max(0.0),
        }
    }

    pub fn cut_top(&mut self, h: f32) -> Self {
        let h = h.min(self.h);
        let cut = Self { h, ..*self };
        self.y += h;
        self.h -= h;
        cut
    }

    pub fn cut_bottom(&mut self, h: f32) -> Self {
        let h = h.min(self.h);
        self.h -= h;
        Self { y: self.y + self.h, h, ..*self }
    }

    pub fn cut_left(&mut self, w: f32) -> Self {
        let w = w.min(self.w);
        let cut = Self { w, ..*self };
        self.x += w;
        self.w -= w;
        cut
    }

    pub fn cut_right(&mut self, w: f32) -> Self {
        let w = w.min(self.w);
        self.w -= w;
        Self { x: self.x + self.w, w, ..*self }
    }

    pub fn skip(&mut self, h: f32) {
        self.cut_top(h);
    }

    pub fn fit_aspect(self, aspect: f32) -> Self {
        if aspect <= 0.0 || self.w <= 0.0 || self.h <= 0.0 {
            return self;
        }
        let (w, h) = if self.w / self.h > aspect {
            (self.h * aspect, self.h)
        } else {
            (self.w, self.w / aspect)
        };
        Self {
            x: self.x + (self.w - w) * 0.5,
            y: self.y + (self.h - h) * 0.5,
            w,
            h,
        }
    }

    fn to_d2d(self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.right(),
            bottom: self.bottom(),
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Input {
    pub mouse: (f32, f32),
    previous_mouse: (f32, f32),

    pub down: bool,

    pub pressed: bool,

    pub released: bool,

    pub wheel: f32,
    pub double_click: bool,

    pub right_pressed: bool,
}

impl Input {
    pub fn drag_delta(&self) -> (f32, f32) {
        (
            self.mouse.0 - self.previous_mouse.0,
            self.mouse.1 - self.previous_mouse.1,
        )
    }
}

pub fn id(name: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

struct Fonts {
    body: IDWriteTextFormat,
    body_center: IDWriteTextFormat,
    small: IDWriteTextFormat,
    small_right: IDWriteTextFormat,
    section: IDWriteTextFormat,
    title: IDWriteTextFormat,
}

pub struct Ui {
    pub ctx: ID2D1DeviceContext,

    factory: ID2D1Factory1,
    target: Option<ID2D1Bitmap1>,
    brush: ID2D1SolidColorBrush,
    fonts: Fonts,

    pub input: Input,

    hot: u64,

    active: u64,

    blocked: bool,

    pub dirty: bool,
    pub size: (f32, f32),
}

impl Ui {
    pub fn new(gpu: &Gpu) -> Result<Self> {
        let factory: ID2D1Factory1 = unsafe {
            D2D1CreateFactory(
                D2D1_FACTORY_TYPE_SINGLE_THREADED,
                Some(&D2D1_FACTORY_OPTIONS::default()),
            )
        }?;

        let dxgi_device: IDXGIDevice = gpu.device.cast()?;
        let d2d_device = unsafe { factory.CreateDevice(&dxgi_device) }?;
        let ctx = unsafe { d2d_device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE) }?;

        let brush = unsafe { ctx.CreateSolidColorBrush(&theme::TEXT, None) }?;
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED) }?;

        Ok(Self {
            ctx,
            factory,
            target: None,
            brush,
            fonts: Fonts::new(&dwrite)?,
            input: Input::default(),
            hot: 0,
            active: 0,
            blocked: false,
            dirty: true,
            size: (0.0, 0.0),
        })
    }

    pub fn attach(&mut self, swapchain: &IDXGISwapChain1, width: f32, height: f32) -> Result<()> {
        unsafe { self.ctx.SetTarget(None) };
        self.target = None;

        let surface: IDXGISurface = unsafe { swapchain.GetBuffer(0) }?;
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            colorContext: std::mem::ManuallyDrop::new(None),
        };

        let bitmap = unsafe { self.ctx.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) }?;
        unsafe { self.ctx.SetTarget(&bitmap) };

        self.target = Some(bitmap);
        self.size = (width, height);
        Ok(())
    }

    pub fn detach(&mut self) {
        unsafe { self.ctx.SetTarget(None) };
        self.target = None;
    }

    pub fn begin(&mut self) {
        self.hot = 0;
        unsafe {
            self.ctx.BeginDraw();
            self.ctx.Clear(Some(&theme::WINDOW));
        }
    }

    pub fn end(&mut self) -> Result<()> {
        unsafe { self.ctx.EndDraw(None, None) }?;

        self.input.previous_mouse = self.input.mouse;
        self.input.pressed = false;
        self.input.released = false;
        self.input.double_click = false;
        self.input.right_pressed = false;
        self.input.wheel = 0.0;

        if !self.input.down {
            self.active = 0;
        }
        Ok(())
    }

    pub fn bounds(&self) -> Rect {
        Rect::new(0.0, 0.0, self.size.0, self.size.1)
    }

    pub fn block_input(&mut self, blocked: bool) {
        self.blocked = blocked;
    }

    pub fn push_clip(&self, rect: Rect) {
        unsafe {
            self.ctx
                .PushAxisAlignedClip(&rect.to_d2d(), D2D1_ANTIALIAS_MODE_ALIASED)
        };
    }

    pub fn pop_clip(&self) {
        unsafe { self.ctx.PopAxisAlignedClip() };
    }

    pub fn is_capturing(&self) -> bool {
        self.active != 0
    }

    fn set(&self, color: D2D1_COLOR_F) -> &ID2D1SolidColorBrush {
        unsafe { self.brush.SetColor(&color) };
        &self.brush
    }

    pub fn fill(&self, rect: Rect, color: D2D1_COLOR_F) {
        unsafe { self.ctx.FillRectangle(&rect.to_d2d(), self.set(color)) };
    }

    pub fn fill_round(&self, rect: Rect, radius: f32, color: D2D1_COLOR_F) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect.to_d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.ctx.FillRoundedRectangle(&rounded, self.set(color)) };
    }

    pub fn stroke_round(&self, rect: Rect, radius: f32, color: D2D1_COLOR_F, width: f32) {
        let rounded = D2D1_ROUNDED_RECT {
            rect: rect.inset(width * 0.5).to_d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe {
            self.ctx
                .DrawRoundedRectangle(&rounded, self.set(color), width, None)
        };
    }

    pub fn stroke(&self, rect: Rect, color: D2D1_COLOR_F, width: f32) {
        unsafe {
            self.ctx
                .DrawRectangle(&rect.inset(width * 0.5).to_d2d(), self.set(color), width, None)
        };
    }

    pub fn circle(&self, center: (f32, f32), radius: f32, color: D2D1_COLOR_F) {
        let ellipse = D2D1_ELLIPSE {
            point: D2D_POINT_2F { x: center.0, y: center.1 },
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.ctx.FillEllipse(&ellipse, self.set(color)) };
    }

    pub fn line(&self, a: (f32, f32), b: (f32, f32), color: D2D1_COLOR_F, width: f32) {
        unsafe {
            self.ctx.DrawLine(
                D2D_POINT_2F { x: a.0, y: a.1 },
                D2D_POINT_2F { x: b.0, y: b.1 },
                self.set(color),
                width,
                None,
            )
        };
    }

    fn draw_text(&self, rect: Rect, text: &str, format: &IDWriteTextFormat, color: D2D1_COLOR_F) {
        let utf16: Vec<u16> = text.encode_utf16().collect();
        if utf16.is_empty() {
            return;
        }
        unsafe {
            self.ctx.DrawText(
                &utf16,
                format,
                &rect.to_d2d(),
                self.set(color),
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            )
        };
    }

    pub fn label(&self, rect: Rect, text: &str, color: D2D1_COLOR_F) {
        self.draw_text(rect, text, &self.fonts.body, color);
    }

    pub fn label_center(&self, rect: Rect, text: &str, color: D2D1_COLOR_F) {
        self.draw_text(rect, text, &self.fonts.body_center, color);
    }

    pub fn label_small(&self, rect: Rect, text: &str, color: D2D1_COLOR_F) {
        self.draw_text(rect, text, &self.fonts.small, color);
    }

    pub fn label_small_right(&self, rect: Rect, text: &str, color: D2D1_COLOR_F) {
        self.draw_text(rect, text, &self.fonts.small_right, color);
    }

    pub fn title(&self, rect: Rect, text: &str, color: D2D1_COLOR_F) {
        self.draw_text(rect, text, &self.fonts.title, color);
    }

    pub fn section(&self, mut rect: Rect, text: &str) {
        let upper = text.to_uppercase();
        let label = rect.cut_left(measure(&upper, 7.2) + 8.0);
        self.draw_text(label, &upper, &self.fonts.section, theme::TEXT_FAINT);

        let (_, cy) = rect.center();
        if rect.w > 8.0 {
            self.line((rect.x, cy), (rect.right(), cy), theme::BORDER, 1.0);
        }
    }

    fn interact(&mut self, widget: u64, rect: Rect) -> Interaction {
        if self.blocked {
            return Interaction { hovered: false, held: false, clicked: false };
        }

        let hovered = rect.contains(self.input.mouse);

        if hovered && self.active == 0 {
            self.hot = widget;
        }

        if hovered && self.input.pressed && self.active == 0 {
            self.active = widget;
        }

        let held = self.active == widget;
        let clicked = held && self.input.released && hovered;

        Interaction { hovered, held, clicked }
    }

    pub fn button(&mut self, name: &str, rect: Rect, label: &str) -> bool {
        let state = self.interact(id(name), rect);

        let base = if state.held {
            theme::ACCENT_SUNK
        } else if state.hovered {
            theme::PANEL_RAISED
        } else {
            theme::PANEL
        };

        self.fill_round(rect, 4.0, base);
        self.stroke_round(rect, 4.0, theme::BORDER, 1.0);
        self.label_center(text_row(rect), label, theme::TEXT);

        state.clicked
    }

    pub fn button_primary(&mut self, name: &str, rect: Rect, label: &str) -> bool {
        let state = self.interact(id(name), rect);

        let base = if state.held {
            theme::ACCENT_SUNK
        } else if state.hovered {
            theme::ACCENT_HOVER
        } else {
            theme::ACCENT
        };

        self.fill_round(rect, 4.0, base);
        self.label_center(text_row(rect), label, theme::TEXT);

        state.clicked
    }

    pub fn toggle(&mut self, name: &str, rect: Rect, label: &str, selected: bool) -> bool {
        let state = self.interact(id(name), rect);

        let base = if selected {
            theme::ACCENT
        } else if state.held {
            theme::PANEL_SUNKEN
        } else if state.hovered {
            theme::PANEL_RAISED
        } else {
            theme::PANEL
        };

        self.fill_round(rect, 4.0, base);
        if !selected {
            self.stroke_round(rect, 4.0, theme::BORDER, 1.0);
        }

        let color = if selected { theme::TEXT } else { theme::TEXT_DIM };
        self.label_center(text_row(rect), label, color);

        state.clicked
    }

    pub fn checkbox(&mut self, name: &str, rect: Rect, label: &str, value: &mut bool) -> bool {
        let state = self.interact(id(name), rect);

        let box_size = 16.0;
        let box_rect = Rect::new(
            rect.x,
            rect.y + (rect.h - box_size) * 0.5,
            box_size,
            box_size,
        );

        let base = if *value {
            theme::ACCENT
        } else if state.hovered {
            theme::PANEL_RAISED
        } else {
            theme::PANEL_SUNKEN
        };

        self.fill_round(box_rect, 3.0, base);
        self.stroke_round(
            box_rect,
            3.0,
            if *value { theme::ACCENT } else { theme::BORDER_STRONG },
            1.0,
        );

        if *value {
            let (cx, cy) = box_rect.center();
            self.line((cx - 4.0, cy), (cx - 1.0, cy + 3.0), theme::TEXT, 1.8);
            self.line((cx - 1.0, cy + 3.0), (cx + 4.0, cy - 3.5), theme::TEXT, 1.8);
        }

        let text = Rect::new(
            box_rect.right() + 9.0,
            rect.y,
            rect.w - box_size - 9.0,
            rect.h,
        );
        self.label(text_row(text), label, theme::TEXT);

        if state.clicked {
            *value = !*value;
            return true;
        }
        false
    }

    pub fn slider(
        &mut self,
        name: &str,
        mut rect: Rect,
        label: &str,
        value: &mut f32,
        range: (f32, f32),
        format: impl Fn(f32) -> String,
    ) -> bool {
        let widget = id(name);

        let header = rect.cut_top(16.0);
        let mut header = header;
        let value_rect = header.cut_right(64.0);
        self.label_small(header, label, theme::TEXT_DIM);
        self.label_small_right(value_rect, &format(*value), theme::TEXT);

        rect.skip(2.0);
        let track_area = rect.cut_top(18.0);
        let state = self.interact(widget, track_area);

        let knob_radius = 6.0;
        let track = Rect::new(
            track_area.x + knob_radius,
            track_area.center().1 - 2.0,
            (track_area.w - knob_radius * 2.0).max(1.0),
            4.0,
        );

        let span = (range.1 - range.0).abs().max(f32::EPSILON);
        let mut changed = false;

        if state.held && (self.input.down || self.input.pressed) {
            let t = ((self.input.mouse.0 - track.x) / track.w).clamp(0.0, 1.0);
            let next = range.0 + t * (range.1 - range.0);
            if (next - *value).abs() > span * 1e-4 {
                *value = next;
                changed = true;
            }
        }

        let t = ((*value - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
        let knob_x = track.x + track.w * t;

        self.fill_round(track, 2.0, theme::PANEL_SUNKEN);
        if t > 0.0 {
            let filled = Rect::new(track.x, track.y, track.w * t, track.h);
            self.fill_round(filled, 2.0, theme::ACCENT);
        }

        let knob_color = if state.held {
            theme::ACCENT_HOVER
        } else if state.hovered {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        self.circle((knob_x, track.center().1), knob_radius, knob_color);

        changed
    }

    pub fn tabs(&mut self, name: &str, rect: Rect, labels: &[String], selected: usize) -> Option<usize> {
        self.fill(rect, theme::PANEL_SUNKEN);

        let mut row = rect;
        let mut result = None;

        for (index, label) in labels.iter().enumerate() {
            let width = (measure(label, 7.6) + 34.0).min(row.w);
            if width <= 0.0 {
                break;
            }
            let tab = row.cut_left(width);
            let state = self.interact(id(&format!("{name}:{index}")), tab);
            let is_selected = index == selected;

            if is_selected {
                self.fill(tab, theme::PANEL);
            } else if state.hovered {
                self.fill(tab, theme::HOVER);
            }

            let color = if is_selected { theme::TEXT } else { theme::TEXT_DIM };
            self.label_center(text_row(tab), label, color);

            if is_selected {
                let underline = Rect::new(tab.x, tab.bottom() - 2.0, tab.w, 2.0);
                self.fill(underline, theme::ACCENT);
            }

            if state.clicked && !is_selected {
                result = Some(index);
            }
        }

        let bottom = Rect::new(rect.x, rect.bottom() - 1.0, rect.w, 1.0);
        self.fill(bottom, theme::BORDER);

        result
    }

    pub fn load_icon(&self, path: &str, viewbox: f32) -> Result<Icon> {
        Icon::new(&self.factory, path, viewbox)
    }

    pub fn icon(&self, icon: &Icon, rect: Rect, size: f32, color: D2D1_COLOR_F) {
        let (cx, cy) = rect.center();
        let transform = icon.transform((cx - size / 2.0, cy - size / 2.0), size);

        unsafe {
            self.ctx.SetTransform(&transform);
            self.ctx.FillGeometry(icon.geometry(), self.set(color), None);
            self.ctx.SetTransform(&Matrix3x2::identity());
        }
    }

    pub fn icon_outline(&self, icon: &Icon, rect: Rect, size: f32, color: D2D1_COLOR_F, width: f32) {
        let (cx, cy) = rect.center();
        let transform = icon.transform((cx - size / 2.0, cy - size / 2.0), size);

        unsafe {
            self.ctx.SetTransform(&transform);

            self.ctx.DrawGeometry(
                icon.geometry(),
                self.set(color),
                width * icon.viewbox() / size,
                None,
            );
            self.ctx.SetTransform(&Matrix3x2::identity());
        }
    }

    pub fn icon_button(
        &mut self,
        name: &str,
        rect: Rect,
        icon: &Icon,
        size: f32,
        active: bool,
    ) -> bool {
        let state = self.interact(id(name), rect);

        if active || state.held {
            self.fill_round(rect, 5.0, theme::PANEL_RAISED);
        } else if state.hovered {
            self.fill_round(rect, 5.0, theme::HOVER);
        }

        let color = if active || state.hovered {
            theme::TEXT
        } else {
            theme::TEXT_DIM
        };
        self.icon(icon, rect, size, color);

        state.clicked
    }

    pub fn progress(&self, mut rect: Rect, fraction: f32, label: &str) {
        let header = rect.cut_top(18.0);
        let mut header = header;
        let percent = header.cut_right(52.0);
        self.label_small(header, label, theme::TEXT);
        self.label_small_right(percent, &format!("{:.0}%", fraction * 100.0), theme::TEXT_DIM);

        rect.skip(4.0);
        let track = rect.cut_top(6.0);
        self.fill_round(track, 3.0, theme::PANEL_SUNKEN);

        let filled = fraction.clamp(0.0, 1.0) * track.w;
        if filled > 1.0 {
            self.fill_round(Rect::new(track.x, track.y, filled, track.h), 3.0, theme::ACCENT);
        }
    }

    pub fn drag_area(&mut self, name: &str, rect: Rect) -> DragArea {
        let state = self.interact(id(name), rect);

        let hovered = !self.blocked && rect.contains(self.input.mouse);

        DragArea {
            hovered,
            dragging: state.held && self.input.down,
            delta: if state.held { self.input.drag_delta() } else { (0.0, 0.0) },
            wheel: if hovered { self.input.wheel } else { 0.0 },
            double_click: hovered && self.input.double_click,
        }
    }

    pub fn wheel_over(&self, rect: Rect) -> f32 {
        if self.blocked || !rect.contains(self.input.mouse) {
            0.0
        } else {
            self.input.wheel
        }
    }

    pub fn load_image(&self, path: &std::path::Path) -> Result<ID2D1Bitmap1> {
        use windows::Win32::Graphics::Imaging::*;
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

        let factory: IWICImagingFactory =
            unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;

        let decoder = unsafe {
            factory.CreateDecoderFromFilename(
                &HSTRING::from(path.as_os_str()),
                None,
                windows::Win32::Foundation::GENERIC_READ,
                WICDecodeMetadataCacheOnLoad,
            )
        }?;

        let frame = unsafe { decoder.GetFrame(0) }?;

        let converter = unsafe { factory.CreateFormatConverter() }?;
        unsafe {
            converter.Initialize(
                &frame,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }?;

        unsafe { self.ctx.CreateBitmapFromWicBitmap(&converter, None) }
    }

    pub fn draw_image(&self, bitmap: &ID2D1Bitmap1, rect: Rect) {
        unsafe {
            self.ctx.DrawBitmap(
                bitmap,
                Some(&rect.to_d2d()),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            )
        };
    }

    pub fn dim(&self, rect: Rect, amount: f32) {
        self.fill(rect, rgba(0x00_00_00, amount));
    }

    pub fn image(&self, texture: &ID3D11Texture2D, rect: Rect) -> Result<()> {
        let surface: IDXGISurface = texture.cast()?;
        let properties = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };

        let bitmap = unsafe { self.ctx.CreateBitmapFromDxgiSurface(&surface, Some(&properties)) }?;
        unsafe {
            self.ctx.DrawBitmap(
                &bitmap,
                Some(&rect.to_d2d()),
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                None,
                None,
            )
        };
        Ok(())
    }
}

struct Interaction {
    hovered: bool,
    held: bool,
    clicked: bool,
}

pub struct DragArea {
    pub hovered: bool,
    pub dragging: bool,
    pub delta: (f32, f32),
    pub wheel: f32,
    pub double_click: bool,
}

fn measure(text: &str, per_char: f32) -> f32 {
    text.chars().count() as f32 * per_char
}

fn text_row(rect: Rect) -> Rect {
    rect.inset_xy(10.0, 0.0)
}

impl Fonts {
    fn new(dwrite: &IDWriteFactory) -> Result<Self> {
        let body = make_format(dwrite, 13.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let body_center = make_format(dwrite, 13.0, DWRITE_FONT_WEIGHT_NORMAL)?;
        let small = make_format(dwrite, 11.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let small_right = make_format(dwrite, 11.5, DWRITE_FONT_WEIGHT_NORMAL)?;
        let section = make_format(dwrite, 10.5, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        let title = make_format(dwrite, 15.0, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;

        unsafe {
            body_center.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            small_right.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_TRAILING)?;
        }

        Ok(Self { body, body_center, small, small_right, section, title })
    }
}

fn make_format(
    dwrite: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    let format = unsafe {
        dwrite.CreateTextFormat(
            w!("Segoe UI"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!(""),
        )
    }?;

    unsafe {
        format.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        format.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
    }

    Ok(format)
}

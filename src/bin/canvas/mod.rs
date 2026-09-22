use super::*;

const PULL_CENTER: f32 = 20.0;

const PULL_EDGE: f32 = 15.0;

const PULL_LAYER: f32 = 9.0;

const HANDLE: f32 = 9.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Handle {
    Move,
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Handle {
    fn moves_left(self) -> bool {
        matches!(self, Handle::Left | Handle::TopLeft | Handle::BottomLeft)
    }
    fn moves_right(self) -> bool {
        matches!(self, Handle::Right | Handle::TopRight | Handle::BottomRight)
    }
    fn moves_top(self) -> bool {
        matches!(self, Handle::Top | Handle::TopLeft | Handle::TopRight)
    }
    fn moves_bottom(self) -> bool {
        matches!(self, Handle::Bottom | Handle::BottomLeft | Handle::BottomRight)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Target {
    Background,
    Layer(usize),
}

pub(super) struct Grab {
    pub target: Target,
    pub handle: Handle,

    pub start_rect: [f32; 4],
    pub start_stretch: (f32, f32),
    pub start_offset: (f32, f32),

    pub start_half: (f32, f32),
    pub origin: (f32, f32),
}

#[derive(Clone, Copy)]
pub(super) struct Guide {
    pub vertical: bool,

    pub at: f32,
}

fn content_rect(preview: Rect, placement: Placement, media: (u32, u32)) -> Rect {
    let size = (preview.w.max(1.0) as u32, preview.h.max(1.0) as u32);
    let (sx, sy) = placement.sampling_scale(media, size);

    if sx <= 0.0 || sy <= 0.0 {
        return preview;
    }

    let left = 0.5 + (placement.offset_x - 0.5) / sx;
    let top = 0.5 + (placement.offset_y - 0.5) / sy;

    Rect::new(
        preview.x + left * preview.w,
        preview.y + top * preview.h,
        preview.w / sx,
        preview.h / sy,
    )
}

fn reachable(rect: Rect, bounds: Rect) -> Rect {
    let left = rect.x.max(bounds.x);
    let top = rect.y.max(bounds.y);
    let right = rect.right().min(bounds.right());
    let bottom = rect.bottom().min(bounds.bottom());

    Rect::new(left, top, (right - left).max(1.0), (bottom - top).max(1.0))
}

fn layer_rect(preview: Rect, r: [f32; 4]) -> Rect {
    Rect::new(
        preview.x + r[0] * preview.w,
        preview.y + r[1] * preview.h,
        r[2] * preview.w,
        r[3] * preview.h,
    )
}

fn background_rect(app: &App, preview: Rect, stage: Rect, media: Option<(u32, u32)>) -> Rect {
    let Some(media) = media else { return preview };
    let Some(slide) = app.tabs[app.selected].slide() else {
        return preview;
    };
    reachable(content_rect(preview, slide.placement(), media), stage)
}

fn handles(rect: Rect) -> [(Handle, Rect); 8] {
    let h = HANDLE;
    let half = h / 2.0;
    let (cx, cy) = rect.center();
    let spot = |x: f32, y: f32| Rect::new(x - half, y - half, h, h);

    [
        (Handle::TopLeft, spot(rect.x, rect.y)),
        (Handle::Top, spot(cx, rect.y)),
        (Handle::TopRight, spot(rect.right(), rect.y)),
        (Handle::Right, spot(rect.right(), cy)),
        (Handle::BottomRight, spot(rect.right(), rect.bottom())),
        (Handle::Bottom, spot(cx, rect.bottom())),
        (Handle::BottomLeft, spot(rect.x, rect.bottom())),
        (Handle::Left, spot(rect.x, cy)),
    ]
}

fn handle_at(rect: Rect, mouse: (f32, f32)) -> Option<Handle> {
    handles(rect)
        .into_iter()
        .find(|(_, spot)| spot.inset(-2.0).contains(mouse))
        .map(|(handle, _)| handle)
}

#[derive(Clone, Copy)]
struct Magnet {
    at: f32,

    pull: f32,
}

fn magnets(slide: &config::Slide, skip: Option<usize>, vertical: bool) -> Vec<Magnet> {
    let mut values = vec![
        Magnet { at: 0.0, pull: PULL_EDGE },
        Magnet { at: 0.5, pull: PULL_CENTER },
        Magnet { at: 1.0, pull: PULL_EDGE },
    ];

    for (index, layer) in slide.layers.iter().enumerate() {
        if Some(index) == skip {
            continue;
        }
        let (start, size) = if vertical {
            (layer.rect[0], layer.rect[2])
        } else {
            (layer.rect[1], layer.rect[3])
        };
        for at in [start, start + size / 2.0, start + size] {
            values.push(Magnet { at, pull: PULL_LAYER });
        }
    }

    values
}

fn snap(value: f32, magnets: &[Magnet], pixels_per_unit: f32) -> Option<f32> {
    let unit = pixels_per_unit.max(1.0);

    magnets
        .iter()
        .filter_map(|m| {
            let reach = m.pull / unit;
            let distance = (m.at - value).abs();
            (distance <= reach).then(|| (distance / reach, m.at))
        })
        .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, at)| at)
}

pub(super) fn interact(
    app: &mut App,
    preview: Rect,
    stage: Rect,
    media: Option<(u32, u32)>,
) -> (bool, Vec<Guide>) {
    let mut guides = Vec::new();

    if !app.editing {
        app.grab = None;
        return (false, guides);
    }

    let input = app.ui.input;
    let mouse = input.mouse;

    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        return (false, guides);
    };

    let reach = bounds(app, preview, stage, media);

    if input.pressed && !reach.contains(mouse) {
        app.editing = false;
        app.grab = None;
        return (false, guides);
    }

    if input.pressed && reach.contains(mouse) {
        let mut picked = None;

        for index in (0..slide.layers.len()).rev() {
            let rect = layer_rect(preview, slide.layers[index].rect);
            let on_handle = app.selected_layer == Some(index) && handle_at(rect, mouse).is_some();

            if on_handle || rect.contains(mouse) {
                let handle = handle_at(rect, mouse).unwrap_or(Handle::Move);
                picked = Some((Target::Layer(index), handle));
                break;
            }
        }

        if picked.is_none() {
            let handle = handle_at(background_rect(app, preview, stage, media), mouse);
            picked = Some((Target::Background, handle.unwrap_or(Handle::Move)));
        }

        if let Some((target, handle)) = picked {
            app.selected_layer = match target {
                Target::Layer(index) => Some(index),
                Target::Background => None,
            };

            let start_rect = match target {
                Target::Layer(index) => slide.layers[index].rect,
                Target::Background => [0.0, 0.0, 1.0, 1.0],
            };
            let (start_stretch, start_offset) = match target {
                Target::Layer(index) => (
                    (slide.layers[index].stretch_x, slide.layers[index].stretch_y),
                    (slide.layers[index].x, slide.layers[index].y),
                ),
                Target::Background => ((slide.stretch_x, slide.stretch_y), (slide.x, slide.y)),
            };

            let content = match media {
                Some(media) => content_rect(preview, slide.placement(), media),
                None => preview,
            };

            app.grab = Some(Grab {
                target,
                handle,
                start_rect,
                start_stretch,
                start_offset,
                start_half: ((content.w * 0.5).max(1.0), (content.h * 0.5).max(1.0)),
                origin: mouse,
            });
        }
    }

    if input.released {
        app.grab = None;
    }

    let Some(grab) = app.grab.as_ref() else {
        return (false, guides);
    };

    if !input.down {
        app.grab = None;
        return (false, guides);
    }

    let dx = mouse.0 - grab.origin.0;
    let dy = mouse.1 - grab.origin.1;
    if dx == 0.0 && dy == 0.0 {
        return (false, guides);
    }

    let target = grab.target;
    let handle = grab.handle;
    let start_rect = grab.start_rect;
    let start_stretch = grab.start_stretch;
    let start_offset = grab.start_offset;
    let start_half = grab.start_half;

    let changed = match target {
        Target::Layer(index) => apply_to_layer(
            app,
            index,
            handle,
            start_rect,
            (dx, dy),
            preview,
            &slide,
            &mut guides,
        ),
        Target::Background => apply_to_background(
            app,
            handle,
            start_stretch,
            start_offset,
            start_half,
            (dx, dy),
            preview,
            media,
            &mut guides,
        ),
    };

    (changed, guides)
}

fn apply_to_layer(
    app: &mut App,
    index: usize,
    handle: Handle,
    start: [f32; 4],
    delta: (f32, f32),
    preview: Rect,
    slide: &config::Slide,
    guides: &mut Vec<Guide>,
) -> bool {
    let fx = delta.0 / preview.w;
    let fy = delta.1 / preview.h;
    let unit_x = preview.w;
    let unit_y = preview.h;

    let magnets_x = magnets(slide, Some(index), true);
    let magnets_y = magnets(slide, Some(index), false);

    let mut rect = start;

    if handle == Handle::Move {
        rect[0] = start[0] + fx;
        rect[1] = start[1] + fy;

        let edges_x = [
            (rect[0], 0.0),
            (rect[0] + rect[2] / 2.0, rect[2] / 2.0),
            (rect[0] + rect[2], rect[2]),
        ];
        for (value, shift) in edges_x {
            if let Some(at) = snap(value, &magnets_x, unit_x) {
                rect[0] = at - shift;
                guides.push(Guide { vertical: true, at });
                break;
            }
        }

        let edges_y = [
            (rect[1], 0.0),
            (rect[1] + rect[3] / 2.0, rect[3] / 2.0),
            (rect[1] + rect[3], rect[3]),
        ];
        for (value, shift) in edges_y {
            if let Some(at) = snap(value, &magnets_y, unit_y) {
                rect[1] = at - shift;
                guides.push(Guide { vertical: false, at });
                break;
            }
        }
    } else {
        const MIN: f32 = 0.05;

        if handle.moves_left() {
            let mut left = start[0] + fx;
            if let Some(at) = snap(left, &magnets_x, unit_x) {
                left = at;
                guides.push(Guide { vertical: true, at });
            }
            let right = start[0] + start[2];
            rect[0] = left.min(right - MIN);
            rect[2] = right - rect[0];
        }
        if handle.moves_right() {
            let mut right = start[0] + start[2] + fx;
            if let Some(at) = snap(right, &magnets_x, unit_x) {
                right = at;
                guides.push(Guide { vertical: true, at });
            }
            rect[2] = (right - start[0]).max(MIN);
        }
        if handle.moves_top() {
            let mut top = start[1] + fy;
            if let Some(at) = snap(top, &magnets_y, unit_y) {
                top = at;
                guides.push(Guide { vertical: false, at });
            }
            let bottom = start[1] + start[3];
            rect[1] = top.min(bottom - MIN);
            rect[3] = bottom - rect[1];
        }
        if handle.moves_bottom() {
            let mut bottom = start[1] + start[3] + fy;
            if let Some(at) = snap(bottom, &magnets_y, unit_y) {
                bottom = at;
                guides.push(Guide { vertical: false, at });
            }
            rect[3] = (bottom - start[1]).max(MIN);
        }
    }

    let Some(entry) = app.tabs[app.selected].slide_mut() else {
        return false;
    };
    let Some(layer) = entry.layers.get_mut(index) else {
        return false;
    };

    if layer.rect == rect {
        return false;
    }
    layer.rect = rect;
    true
}

#[allow(clippy::too_many_arguments)]
fn apply_to_background(
    app: &mut App,
    handle: Handle,
    start: (f32, f32),
    start_offset: (f32, f32),
    start_half: (f32, f32),
    delta: (f32, f32),
    preview: Rect,
    media: Option<(u32, u32)>,
    guides: &mut Vec<Guide>,
) -> bool {
    if handle == Handle::Move {
        return pan_background(app, start_offset, delta, preview, media, guides);
    }

    let mut stretch = start;

    if handle.moves_right() {
        stretch.0 = start.0 * (1.0 + delta.0 / start_half.0);
    } else if handle.moves_left() {
        stretch.0 = start.0 * (1.0 - delta.0 / start_half.0);
    }
    if handle.moves_bottom() {
        stretch.1 = start.1 * (1.0 + delta.1 / start_half.1);
    } else if handle.moves_top() {
        stretch.1 = start.1 * (1.0 - delta.1 / start_half.1);
    }

    let per_pixel = (start.0 / start_half.0, start.1 / start_half.1);

    if (stretch.0 - start.0).abs() > f32::EPSILON
        && (stretch.0 - 1.0).abs() < PULL_EDGE * per_pixel.0
    {
        stretch.0 = 1.0;
    }
    if (stretch.1 - start.1).abs() > f32::EPSILON
        && (stretch.1 - 1.0).abs() < PULL_EDGE * per_pixel.1
    {
        stretch.1 = 1.0;
    }

    stretch.0 = stretch.0.clamp(0.1, 8.0);
    stretch.1 = stretch.1.clamp(0.1, 8.0);

    if stretch.0 == 1.0 {
        guides.push(Guide { vertical: true, at: 0.5 });
    }
    if stretch.1 == 1.0 {
        guides.push(Guide { vertical: false, at: 0.5 });
    }

    let Some(entry) = app.tabs[app.selected].slide_mut() else {
        return false;
    };
    if (entry.stretch_x, entry.stretch_y) == stretch {
        return false;
    }

    entry.mode = config::Mode::Custom;
    entry.stretch_x = stretch.0;
    entry.stretch_y = stretch.1;
    true
}

fn pan_background(
    app: &mut App,
    start: (f32, f32),
    delta: (f32, f32),
    preview: Rect,
    media: Option<(u32, u32)>,
    guides: &mut Vec<Guide>,
) -> bool {
    let Some(media) = media else { return false };
    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        return false;
    };

    let size = (preview.w.max(1.0) as u32, preview.h.max(1.0) as u32);
    let (sx, sy) = slide.placement().sampling_scale(media, size);

    let mut x = start.0 + sx * delta.0 / preview.w;
    let mut y = start.1 + sy * delta.1 / preview.h;

    if x.abs() < PULL_CENTER * sx / preview.w {
        x = 0.0;
        guides.push(Guide { vertical: true, at: 0.5 });
    }
    if y.abs() < PULL_CENTER * sy / preview.h {
        y = 0.0;
        guides.push(Guide { vertical: false, at: 0.5 });
    }

    let Some(entry) = app.tabs[app.selected].slide_mut() else {
        return false;
    };
    if (entry.x, entry.y) == (x, y) {
        return false;
    }

    entry.mode = config::Mode::Custom;
    entry.x = x;
    entry.y = y;
    true
}

fn bounds(app: &App, preview: Rect, stage: Rect, media: Option<(u32, u32)>) -> Rect {
    let rect = background_rect(app, preview, stage, media);
    let left = preview.x.min(rect.x) - HANDLE;
    let top = preview.y.min(rect.y) - HANDLE;
    let right = preview.right().max(rect.right()) + HANDLE;
    let bottom = preview.bottom().max(rect.bottom()) + HANDLE;
    Rect::new(left, top, right - left, bottom - top)
}

pub(super) fn draw_overlay(
    app: &mut App,
    preview: Rect,
    stage: Rect,
    media: Option<(u32, u32)>,
    guides: &[Guide],
) {
    if !app.editing {
        return;
    }

    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        return;
    };

    for (index, layer) in slide.layers.iter().enumerate() {
        let rect = layer_rect(preview, layer.rect);
        let selected = app.selected_layer == Some(index);
        let color = if selected {
            theme::ACCENT
        } else {
            walllit::ui::rgba(0xFF_FF_FF, 0.30)
        };
        app.ui.stroke_round(rect, 2.0, color, if selected { 2.0 } else { 1.0 });
    }

    let target_rect = match app.selected_layer {
        Some(index) => slide.layers.get(index).map(|l| layer_rect(preview, l.rect)),
        None => Some(background_rect(app, preview, stage, media)),
    };

    if let Some(rect) = target_rect {
        if app.selected_layer.is_none() {
            app.ui
                .stroke_round(rect, 2.0, walllit::ui::rgba(0x3D_8B_FD, 0.45), 1.0);
        }

        let mouse = app.ui.input.mouse;
        for (_, spot) in handles(rect) {
            let hot = spot.inset(-2.0).contains(mouse);
            let grown = if hot { spot.inset(-1.5) } else { spot };

            app.ui.fill_round(grown, 2.0, walllit::ui::rgba(0xFF_FF_FF, 0.95));
            app.ui.stroke_round(
                grown,
                2.0,
                if hot { theme::ACCENT_HOVER } else { theme::ACCENT },
                if hot { 2.0 } else { 1.5 },
            );
        }
    }

    for guide in guides {
        let line = if guide.vertical {
            Rect::new(preview.x + guide.at * preview.w - 0.5, preview.y, 1.0, preview.h)
        } else {
            Rect::new(preview.x, preview.y + guide.at * preview.h - 0.5, preview.w, 1.0)
        };
        app.ui.fill(line, walllit::ui::rgba(0x3D_8B_FD, 0.55));
    }
}

fn build_still(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    path: &std::path::Path,
) -> Option<walllit::image::StillImage> {
    if image::is_image(path) {
        return StillImage::load(gpu, path).ok();
    }

    let pixels = walllit::poster::render_first_frame(
        gpu,
        manager,
        path,
        (640, 360),
        Placement { mode: FitMode::Fit, ..Default::default() },
    )
    .ok()?;

    StillImage::from_bgra(gpu, &pixels.data, pixels.width, pixels.height).ok()
}

pub(super) fn paint_layers(app: &mut App, size: (u32, u32)) {
    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        return;
    };
    if slide.layers.is_empty() {
        return;
    }

    for layer in &slide.layers {
        if !app.layer_stills.contains_key(&layer.wallpaper) {
            let still = build_still(&app.gpu, &app.manager, &layer.wallpaper);
            app.layer_stills.insert(layer.wallpaper.clone(), still);
        }
    }

    let App { preview, layer_stills, gpu, .. } = app;
    let Some(preview) = preview.as_mut() else {
        return;
    };

    for layer in &slide.layers {
        let Some(Some(still)) = layer_stills.get(&layer.wallpaper) else {
            continue;
        };
        let _ = preview
            .offscreen
            .overlay_image(gpu, layer.area(size), still, layer.placement());
    }
}

pub(super) fn draw_panel(app: &mut App, panel: &mut Rect) {
    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionLayers));
    panel.skip(6.0);

    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        app.ui.label_small(panel.cut_top(18.0), app.t(Text::NoFile), theme::TEXT_FAINT);
        return;
    };

    let background = panel.cut_top(26.0);
    if app.ui.toggle("layer-background", background, app.t(Text::LayerBackground), app.selected_layer.is_none())
    {
        app.selected_layer = None;

        app.editing = true;
    }
    panel.skip(4.0);

    let mut remove = None;
    let mut raise = None;

    for index in 0..slide.layers.len() {
        let mut row = panel.cut_top(26.0);
        let drop_button = row.cut_right(26.0);
        row.cut_right(4.0);
        let up = row.cut_right(26.0);
        row.cut_right(4.0);

        let name = slide.layers[index].display_name();
        if app.ui.toggle(&format!("layer{index}"), row, &name, app.selected_layer == Some(index)) {
            app.selected_layer = Some(index);
            app.editing = true;
        }
        if app.ui.button(&format!("raise{index}"), up, "▲") {
            raise = Some(index);
        }
        if app.ui.button(&format!("drop{index}"), drop_button, "×") {
            remove = Some(index);
        }
        panel.skip(4.0);
    }

    if slide.layers.is_empty() {
        app.ui.label_small(panel.cut_top(30.0), app.t(Text::LayersHint), theme::TEXT_FAINT);
    }

    if let Some(index) = raise {
        if let Some(entry) = app.tabs[app.selected].slide_mut() {
            if index + 1 < entry.layers.len() {
                entry.layers.swap(index, index + 1);
                app.selected_layer = Some(index + 1);
            }
        }
        app.preview_dirty = true;
        app.saved = false;
    }

    if let Some(index) = remove {
        if let Some(entry) = app.tabs[app.selected].slide_mut() {
            if index < entry.layers.len() {
                entry.layers.remove(index);
            }
        }
        app.selected_layer = None;
        app.preview_dirty = true;
        app.saved = false;
    }

    panel.skip(10.0);
    draw_adjustments(app, panel);
}

fn draw_adjustments(app: &mut App, panel: &mut Rect) {
    let Some(slide) = app.tabs[app.selected].slide().cloned() else {
        return;
    };

    let layer = app.selected_layer.and_then(|i| slide.layers.get(i).cloned());

    let (mut stretch_x, mut stretch_y, mut speed, mut filters, is_video) = match &layer {
        Some(layer) => (
            layer.stretch_x,
            layer.stretch_y,
            layer.speed,
            layer.filters,
            !image::is_image(&layer.wallpaper),
        ),
        None => (
            slide.stretch_x,
            slide.stretch_y,
            slide.speed,
            slide.filters,
            !image::is_image(&slide.wallpaper),
        ),
    };

    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionResize));
    panel.skip(4.0);

    let mut changed = false;

    changed |= app.ui.slider(
        "stretch-x",
        panel.cut_top(38.0),
        app.t(Text::StretchWidth),
        &mut stretch_x,
        (0.2, 4.0),
        |v| format!("{v:.2}x"),
    );
    changed |= app.ui.slider(
        "stretch-y",
        panel.cut_top(38.0),
        app.t(Text::StretchHeight),
        &mut stretch_y,
        (0.2, 4.0),
        |v| format!("{v:.2}x"),
    );

    if app.ui.button("stretch-reset", panel.cut_top(28.0), app.t(Text::ResetStretch)) {
        stretch_x = 1.0;
        stretch_y = 1.0;
        changed = true;
    }

    if is_video && layer.is_some() {
        panel.skip(6.0);
        changed |= app.ui.slider(
            "layer-speed",
            panel.cut_top(38.0),
            app.t(Text::Speed),
            &mut speed,
            (0.25, 4.0),
            |v| format!("{v:.2}x"),
        );
    }

    panel.skip(8.0);
    app.ui.section(panel.cut_top(20.0), app.t(Text::SectionFilters));
    panel.skip(4.0);

    changed |= app.ui.slider(
        "filter-brightness",
        panel.cut_top(38.0),
        app.t(Text::Brightness),
        &mut filters.brightness,
        (-0.5, 0.5),
        |v| format!("{v:+.2}"),
    );
    changed |= app.ui.slider(
        "filter-contrast",
        panel.cut_top(38.0),
        app.t(Text::Contrast),
        &mut filters.contrast,
        (0.0, 2.5),
        |v| format!("{v:.2}"),
    );
    changed |= app.ui.slider(
        "filter-saturation",
        panel.cut_top(38.0),
        app.t(Text::Saturation),
        &mut filters.saturation,
        (0.0, 3.0),
        |v| format!("{v:.2}"),
    );
    changed |= app.ui.slider(
        "filter-temperature",
        panel.cut_top(38.0),
        app.t(Text::Temperature),
        &mut filters.temperature,
        (-0.25, 0.25),
        |v| format!("{v:+.2}"),
    );

    if app.ui.button("filter-reset", panel.cut_top(28.0), app.t(Text::ResetFilters)) {
        filters = config::Filters::default();
        changed = true;
    }

    if !changed {
        return;
    }

    let selected = app.selected_layer;
    if let Some(entry) = app.tabs[app.selected].slide_mut() {
        match selected.and_then(|i| entry.layers.get_mut(i)) {
            Some(layer) => {
                layer.stretch_x = stretch_x;
                layer.stretch_y = stretch_y;
                layer.speed = speed;
                layer.filters = filters;
            }
            None => {
                if entry.stretch_x != stretch_x || entry.stretch_y != stretch_y {
                    entry.mode = config::Mode::Custom;
                }
                entry.stretch_x = stretch_x;
                entry.stretch_y = stretch_y;
                entry.filters = filters;
            }
        }
    }

    app.preview_dirty = true;
    app.saved = false;
}

pub(super) fn paste_layer(app: &mut App, item: usize) {
    let Some(entry) = app.library.items().get(item) else {
        return;
    };
    let wallpaper = entry.prepared.clone();
    let source = entry.source.clone();
    let aspect = entry.aspect().max(0.05);

    let Some(slide) = app.tabs[app.selected].slide_mut() else {
        return;
    };

    let step = 0.04 * slide.layers.len() as f32;
    let width = 0.4_f32;
    let height = (width / aspect).clamp(0.05, 0.9);

    slide.layers.push(config::Layer {
        wallpaper,
        source,
        rect: [
            (0.1 + step).min(0.6),
            (0.1 + step).min(0.6),
            width,
            height,
        ],
        ..Default::default()
    });

    let last = slide.layers.len() - 1;
    app.selected_layer = Some(last);

    app.editing = true;
    app.preview_dirty = true;
    app.saved = false;
}

use std::path::{Path, PathBuf};

use windows::core::*;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::Media::MediaFoundation::IMFDXGIDeviceManager;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER};
use windows::Win32::UI::Shell::{
    DesktopWallpaper, IDesktopWallpaper, DESKTOP_WALLPAPER_POSITION, DWPOS_FILL,
};

use crate::config;
use crate::display::{self, Monitor};
use crate::renderer::{FrameRef, Gpu, Offscreen, Pixels, Placement};
use crate::video::VideoSource;

const GENERIC_WRITE_FLAG: u32 = 0x4000_0000;

fn stage<T>(what: &str, result: Result<T>) -> Result<T> {
    result.map_err(|e| Error::new(e.code(), format!("{what}: {}", e.message())))
}

pub fn poster_dir() -> Result<PathBuf> {
    Ok(config::data_dir()?.join("poster"))
}

pub fn apply(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    monitor: &Monitor,
    slide: &config::Slide,
) -> Result<PathBuf> {
    let pixels = stage(
        "decode the first frame",
        render_slide(gpu, manager, slide, (monitor.width(), monitor.height())),
    )?;

    let directory = poster_dir()?;
    std::fs::create_dir_all(&directory).map_err(to_windows_error)?;

    let path = directory.join(format!("{}.jpg", sanitize(&monitor.id)));
    stage("write the image", write_jpeg(&pixels, &path))?;
    stage("aplicar como wallpaper", set_wallpaper(&monitor.id, &path))?;

    Ok(path)
}

pub fn clear(monitor: &Monitor) {
    if let Ok(directory) = poster_dir() {
        let path = directory.join(format!("{}.jpg", sanitize(&monitor.id)));
        let _ = std::fs::remove_file(path);
    }
}

fn paint_layer(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    target: &mut Offscreen,
    media: &Path,
    area: crate::renderer::Area,
    placement: Placement,
) -> Result<()> {
    if crate::image::is_image(media) {
        let picture = crate::image::StillImage::load(gpu, media)?;
        return target.overlay_image(gpu, area, &picture, placement);
    }

    let mut source = VideoSource::open(&media.to_string_lossy(), manager)?;
    let info = source.info;

    for _ in 0..32 {
        if let Some(frame) = source.next_frame()? {
            return target.paint_into(
                gpu,
                area,
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

pub fn render_slide(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    slide: &config::Slide,
    size: (u32, u32),
) -> Result<Pixels> {
    let mut target = Offscreen::new(gpu, size.0, size.1)?;
    target.clear(gpu, [0.0, 0.0, 0.0, 1.0]);

    paint_layer(
        gpu,
        manager,
        &mut target,
        playable(&slide.wallpaper, slide.source.as_deref()),
        crate::renderer::Area::full(size),
        slide.placement(),
    )?;

    for layer in &slide.layers {
        if let Err(e) = paint_layer(
            gpu,
            manager,
            &mut target,
            playable(&layer.wallpaper, layer.source.as_deref()),
            layer.area(size),
            layer.placement(),
        ) {
            crate::diagnostics::record(&format!(
                "still frame: {}: {e}",
                layer.display_name()
            ));
        }
    }

    target.read_pixels(gpu)
}

fn playable<'a>(wallpaper: &'a Path, source: Option<&'a Path>) -> &'a Path {
    if wallpaper.is_file() {
        return wallpaper;
    }
    match source {
        Some(original) if original.is_file() => original,
        _ => wallpaper,
    }
}

pub fn render_first_frame(
    gpu: &Gpu,
    manager: &IMFDXGIDeviceManager,
    video: &Path,
    size: (u32, u32),
    placement: Placement,
) -> Result<Pixels> {
    if crate::image::is_image(video) {
        return render_image(gpu, video, size, placement);
    }

    let mut source = VideoSource::open(&video.to_string_lossy(), manager)?;
    let info = source.info;

    let mut target = Offscreen::new(gpu, size.0, size.1)?;
    target.clear(gpu, [0.0, 0.0, 0.0, 1.0]);

    for _ in 0..32 {
        if let Some(frame) = source.next_frame()? {
            target.paint(
                gpu,
                FrameRef {
                    texture: &frame.texture,
                    subresource: frame.subresource,
                    size: (info.width, info.height),
                    matrix: info.matrix,
                    full_range: info.full_range,
                },
                placement,
            )?;
            return target.read_pixels(gpu);
        }
    }

    Err(Error::new(
        E_FAIL,
        "could not decode the first frame of the video",
    ))
}

fn render_image(gpu: &Gpu, path: &Path, size: (u32, u32), placement: Placement) -> Result<Pixels> {
    let picture = crate::image::StillImage::load(gpu, path)?;

    let mut target = Offscreen::new(gpu, size.0, size.1)?;
    target.clear(gpu, [0.0, 0.0, 0.0, 1.0]);
    target.paint_image(gpu, &picture, placement)?;
    target.read_pixels(gpu)
}

pub fn write_jpeg(pixels: &Pixels, path: &Path) -> Result<()> {
    encode(pixels, path, &GUID_ContainerFormatJpeg, &GUID_WICPixelFormat32bppBGR)
}

fn encode(
    pixels: &Pixels,
    path: &Path,
    container: &windows::core::GUID,
    source_format: &windows::core::GUID,
) -> Result<()> {
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;

    let stream = unsafe { factory.CreateStream() }?;
    unsafe { stream.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE_FLAG) }?;

    let encoder = unsafe { factory.CreateEncoder(container, std::ptr::null()) }?;
    unsafe { encoder.Initialize(&stream, WICBitmapEncoderNoCache) }?;

    let mut frame = None;
    unsafe { encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()) }?;
    let frame = frame.unwrap();

    unsafe {
        frame.Initialize(None)?;
        frame.SetSize(pixels.width, pixels.height)?;

        let source = factory.CreateBitmapFromMemory(
            pixels.width,
            pixels.height,
            source_format,
            pixels.stride,
            &pixels.data,
        )?;

        frame.WriteSource(&source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }

    Ok(())
}

pub fn write_png(pixels: &[u8], width: u32, height: u32, path: &Path) -> Result<()> {
    let image = Pixels {
        data: pixels.to_vec(),
        width,
        height,
        stride: width * 4,
    };
    encode(&image, path, &GUID_ContainerFormatPng, &GUID_WICPixelFormat32bppBGRA)
}

fn set_wallpaper(monitor_id: &str, image: &Path) -> Result<()> {
    let desktop: IDesktopWallpaper =
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_LOCAL_SERVER) }?;

    let path = HSTRING::from(image.as_os_str());
    let device = find_device_path(&desktop, monitor_id)?;

    unsafe {
        let _ = desktop.SetPosition(DESKTOP_WALLPAPER_POSITION(DWPOS_FILL.0));

        match &device {
            Some(device) => desktop.SetWallpaper(PCWSTR(device.as_ptr()), &path),

            None => desktop.SetWallpaper(PCWSTR::null(), &path),
        }
    }
}

fn find_device_path(desktop: &IDesktopWallpaper, monitor_id: &str) -> Result<Option<HSTRING>> {
    let count = unsafe { desktop.GetMonitorDevicePathCount() }?;

    for index in 0..count {
        let raw = unsafe { desktop.GetMonitorDevicePathAt(index) }?;
        let path = unsafe { raw.to_string() }.unwrap_or_default();
        unsafe { windows::Win32::System::Com::CoTaskMemFree(Some(raw.0 as *const _)) };

        if display::normalize_device_path(&path).as_deref() == Some(monitor_id) {
            return Ok(Some(HSTRING::from(path)));
        }
    }

    Ok(None)
}

fn sanitize(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => c,
            _ => '_',
        })
        .collect()
}

pub fn to_windows_error(error: std::io::Error) -> Error {
    Error::new(E_FAIL, format!("{error}"))
}

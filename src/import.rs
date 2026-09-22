use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use windows::core::*;
use windows::Win32::Foundation::E_FAIL;
use windows::Win32::Media::MediaFoundation::*;

use crate::config;

pub const MAX_FPS: u32 = 30;

const BITS_PER_PIXEL: f64 = 0.10;

const MIN_BITRATE: u32 = 2_000_000;
const MAX_BITRATE: u32 = 20_000_000;

const FIRST_VIDEO_STREAM: u32 = 0xFFFF_FFFC;
const ALL_STREAMS: u32 = 0xFFFF_FFFE;
const MEDIA_SOURCE: u32 = 0xFFFF_FFFF;
const READER_END_OF_STREAM: u32 = 0x0000_0002;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Target {
    pub fn for_monitor(width: u32, height: u32) -> Self {
        Self {
            width: width & !1,
            height: height & !1,
            fps: MAX_FPS,
        }
    }

    fn bitrate(&self, fps: u32) -> u32 {
        let estimate = self.width as f64 * self.height as f64 * fps as f64 * BITS_PER_PIXEL;
        (estimate as u32).clamp(MIN_BITRATE, MAX_BITRATE)
    }
}

#[derive(Clone, Debug)]
pub struct Probe {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub is_h264: bool,
    pub bytes: u64,

    pub is_image: bool,
}

impl Probe {
    pub fn is_adequate(&self, target: &Target) -> bool {
        if self.is_image {
            return self.width <= target.width && self.height <= target.height;
        }

        self.is_h264
            && self.width <= target.width
            && self.height <= target.height
            && self.fps <= target.fps as f64 + 0.5
    }
}

#[derive(Clone, Debug)]
pub enum Outcome {
    Reused { path: PathBuf, bytes: u64 },

    Stored { path: PathBuf, probe: Probe },

    Optimized {
        path: PathBuf,
        probe: Probe,
        bytes: u64,
    },
}

impl Outcome {
    pub fn path(&self) -> &Path {
        match self {
            Outcome::Reused { path, .. }
            | Outcome::Stored { path, .. }
            | Outcome::Optimized { path, .. } => path,
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Outcome::Reused { bytes, .. } => {
                format!("Already in the library · {}", human_size(*bytes))
            }
            Outcome::Stored { probe, .. } if probe.is_image => {
                format!("Guardado sem recodificar · {} × {}", probe.width, probe.height)
            }
            Outcome::Stored { probe, .. } => format!(
                "Guardado sem recodificar · {} × {} · {:.0} fps",
                probe.width, probe.height, probe.fps
            ),
            Outcome::Optimized { probe, bytes, .. } if probe.is_image => format!(
                "{} × {} · {} → {}",
                probe.width,
                probe.height,
                human_size(probe.bytes),
                human_size(*bytes)
            ),
            Outcome::Optimized { probe, bytes, .. } => format!(
                "{} × {} {:.0} fps · {} → {}",
                probe.width,
                probe.height,
                probe.fps,
                human_size(probe.bytes),
                human_size(*bytes)
            ),
        }
    }
}

fn human_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    if bytes as f64 >= MB {
        format!("{:.1} MB", bytes as f64 / MB)
    } else {
        format!("{:.0} kB", bytes as f64 / 1024.0)
    }
}

#[derive(Clone, Default)]
pub struct Progress(Arc<AtomicU32>);

impl Progress {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fraction(&self) -> f32 {
        self.0.load(Ordering::Relaxed) as f32 / 10_000.0
    }

    fn set(&self, fraction: f64) {
        let value = (fraction.clamp(0.0, 1.0) * 10_000.0) as u32;
        self.0.store(value, Ordering::Relaxed);
    }
}

pub fn import(source: &Path, target: Target, progress: &Progress) -> Result<Outcome> {
    prepare(source, target, progress, true)
}

pub fn prepare(source: &Path, target: Target, progress: &Progress, optimize: bool) -> Result<Outcome> {
    let probe = probe(source)?;
    let adequate = !optimize || probe.is_adequate(&target);
    let destination = stored_path(source, &target, adequate)?;

    if let Ok(metadata) = std::fs::metadata(&destination) {
        progress.set(1.0);
        return Ok(Outcome::Reused {
            path: destination,
            bytes: metadata.len(),
        });
    }

    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(io_error)?;
    }

    if adequate {
        let temporary = destination.with_extension("part");
        let _ = std::fs::remove_file(&temporary);

        std::fs::copy(source, &temporary).map_err(io_error)?;
        std::fs::rename(&temporary, &destination).map_err(io_error)?;

        progress.set(1.0);
        return Ok(Outcome::Stored { path: destination, probe });
    }

    if probe.is_image {
        let temporary = destination.with_extension("part.png");
        let _ = std::fs::remove_file(&temporary);

        let (pixels, width, height) =
            crate::image::decode_scaled(source, Some((target.width, target.height)))?;

        crate::poster::write_png(&pixels, width, height, &temporary)?;
        std::fs::rename(&temporary, &destination).map_err(io_error)?;

        let bytes = std::fs::metadata(&destination).map(|m| m.len()).unwrap_or(0);
        progress.set(1.0);
        return Ok(Outcome::Optimized { path: destination, probe, bytes });
    }

    let temporary = destination.with_extension("part.mp4");
    let _ = std::fs::remove_file(&temporary);

    transcode(source, &temporary, &probe, &target, progress)?;
    std::fs::rename(&temporary, &destination).map_err(io_error)?;

    let bytes = std::fs::metadata(&destination).map(|m| m.len()).unwrap_or(0);
    Ok(Outcome::Optimized {
        path: destination,
        probe,
        bytes,
    })
}

pub fn probe(source: &Path) -> Result<Probe> {
    if crate::image::is_image(source) {
        let (width, height) = crate::image::measure(source)?;
        return Ok(Probe {
            width,
            height,
            fps: 0.0,
            is_h264: false,
            bytes: std::fs::metadata(source).map(|m| m.len()).unwrap_or(0),
            is_image: true,
        });
    }

    let reader = unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(source.as_os_str()), None) }?;
    let native = unsafe { reader.GetNativeMediaType(FIRST_VIDEO_STREAM, 0) }?;

    let frame_size = unsafe { native.GetUINT64(&MF_MT_FRAME_SIZE) }?;
    let subtype = unsafe { native.GetGUID(&MF_MT_SUBTYPE) }?;

    let fps = match unsafe { native.GetUINT64(&MF_MT_FRAME_RATE) } {
        Ok(rate) => {
            let numerator = (rate >> 32) as u32;
            let denominator = rate as u32;
            if denominator == 0 {
                30.0
            } else {
                numerator as f64 / denominator as f64
            }
        }
        Err(_) => 30.0,
    };

    Ok(Probe {
        width: (frame_size >> 32) as u32,
        height: frame_size as u32,
        fps,
        is_h264: subtype == MFVideoFormat_H264,
        bytes: std::fs::metadata(source).map(|m| m.len()).unwrap_or(0),
        is_image: false,
    })
}

fn transcode(
    source: &Path,
    output: &Path,
    probe: &Probe,
    target: &Target,
    progress: &Progress,
) -> Result<()> {
    let fps = (probe.fps.round() as u32).min(target.fps).max(1);

    let (width, height) = fit_within(probe.width, probe.height, target.width, target.height);

    let device_manager = stage("create video device", create_device_manager())?;
    let reader = stage("open the source media", unsafe {
        MFCreateSourceReaderFromURL(
            &HSTRING::from(source.as_os_str()),
            &reader_attributes(&device_manager)?,
        )
    })?;

    unsafe {
        reader.SetStreamSelection(ALL_STREAMS, false)?;
        reader.SetStreamSelection(FIRST_VIDEO_STREAM, true)?;

        let decoded = MFCreateMediaType()?;
        decoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        decoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;

        decoded.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        stage(
            "request NV12 at the target resolution",
            reader.SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &decoded),
        )?;
    }

    let duration = unsafe { reader.GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION) }
        .ok()
        .and_then(|value| unsafe { windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64(&value) }.ok())
        .unwrap_or(0);

    let writer = stage("create the output file", unsafe {
        MFCreateSinkWriterFromURL(
            &HSTRING::from(output.as_os_str()),
            None,
            &writer_attributes(&device_manager)?,
        )
    })?;

    let stream = unsafe {
        let encoded = MFCreateMediaType()?;
        encoded.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        encoded.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        encoded.SetUINT32(&MF_MT_AVG_BITRATE, target.bitrate(fps))?;
        encoded.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        encoded.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        encoded.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        encoded.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;

        encoded.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)?;
        let index = stage("configure the H.264 encoder", writer.AddStream(&encoded))?;

        let input = MFCreateMediaType()?;
        input.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        input.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        input.SetUINT64(&MF_MT_FRAME_SIZE, pack(width, height))?;
        input.SetUINT64(&MF_MT_FRAME_RATE, pack(fps, 1))?;
        input.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack(1, 1))?;
        input.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        stage(
            "connect the NV12 input to the encoder",
            writer.SetInputMediaType(index, &input, None),
        )?;

        index
    };

    stage("iniciar a gravacao", unsafe { writer.BeginWriting() })?;

    let interval = 10_000_000i64 / fps as i64;
    let mut next_source_time = 0i64;
    let mut written = 0i64;

    loop {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample: Option<IMFSample> = None;

        unsafe {
            reader.ReadSample(
                FIRST_VIDEO_STREAM,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )
        }?;

        if flags & READER_END_OF_STREAM != 0 {
            break;
        }

        let Some(sample) = sample else { continue };

        if timestamp + 1 < next_source_time {
            continue;
        }
        next_source_time += interval;

        unsafe {
            sample.SetSampleTime(written * interval)?;
            sample.SetSampleDuration(interval)?;
            stage("write frame", writer.WriteSample(stream, &sample))?;
        }
        written += 1;

        if duration > 0 {
            progress.set(timestamp as f64 / duration as f64);
        }
    }

    stage("close the file", unsafe { writer.Finalize() })?;
    progress.set(1.0);

    if written == 0 {
        return Err(Error::new(E_FAIL, "the video has no usable frames"));
    }

    Ok(())
}

fn fit_within(width: u32, height: u32, max_width: u32, max_height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (max_width, max_height);
    }
    if width <= max_width && height <= max_height {
        return (width & !1, height & !1);
    }

    let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
    let scaled_width = ((width as f64 * scale).round() as u32).max(2) & !1;
    let scaled_height = ((height as f64 * scale).round() as u32).max(2) & !1;
    (scaled_width, scaled_height)
}

fn create_device_manager() -> Result<IMFDXGIDeviceManager> {
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Direct3D11::*;

    let mut device: Option<ID3D11Device> = None;
    let mut context: Option<ID3D11DeviceContext> = None;

    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            None,
            D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
    }?;

    let device = device.unwrap();
    let context = context.unwrap();

    let multithread: ID3D11Multithread = context.cast()?;
    unsafe {
        let _ = multithread.SetMultithreadProtected(true);
    }

    let mut token = 0u32;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }?;

    let manager = manager.unwrap();
    unsafe { manager.ResetDevice(&device, token) }?;
    Ok(manager)
}

fn reader_attributes(manager: &IMFDXGIDeviceManager) -> Result<IMFAttributes> {
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 3) }?;
    let attributes = attributes.unwrap();

    unsafe {
        attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
    }

    Ok(attributes)
}

fn writer_attributes(manager: &IMFDXGIDeviceManager) -> Result<IMFAttributes> {
    let mut attributes = None;
    unsafe { MFCreateAttributes(&mut attributes, 3) }?;
    let attributes = attributes.unwrap();

    unsafe {
        attributes.SetUnknown(&MF_SINK_WRITER_D3D_MANAGER, manager)?;
        attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;
        attributes.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1)?;
    }

    Ok(attributes)
}

fn stage<T>(what: &str, result: Result<T>) -> Result<T> {
    result.map_err(|e| Error::new(e.code(), format!("{what}: {}", e.message())))
}

fn stored_path(source: &Path, target: &Target, adequate: bool) -> Result<PathBuf> {
    let key = content_key(source, target, adequate)?;

    let stem: String = source
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "wallpaper".into())
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | ' '))
        .take(48)
        .collect();

    let stem = stem.trim();
    let stem = if stem.is_empty() { "wallpaper" } else { stem };

    let extension = if adequate {
        source
            .extension()
            .map(|e| e.to_string_lossy().into_owned())
            .unwrap_or_else(|| "mp4".into())
    } else if crate::image::is_image(source) {
        "png".into()
    } else {
        "mp4".into()
    };

    Ok(config::media_dir()?.join(format!("{stem}-{:08x}.{extension}", key as u32)))
}

fn content_key(source: &Path, target: &Target, adequate: bool) -> Result<u64> {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = OFFSET;
    let mut mix = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(PRIME);
        }
    };

    if !adequate {
        mix(&target.width.to_le_bytes());
        mix(&target.height.to_le_bytes());
        mix(&target.fps.to_le_bytes());
    }

    let mut file = std::fs::File::open(source).map_err(io_error)?;
    let mut buffer = vec![0u8; 1 << 20];

    loop {
        let read = file.read(&mut buffer).map_err(io_error)?;
        if read == 0 {
            break;
        }
        mix(&buffer[..read]);
    }

    Ok(hash)
}

fn pack(high: u32, low: u32) -> u64 {
    ((high as u64) << 32) | low as u64
}

fn io_error(error: std::io::Error) -> Error {
    Error::new(E_FAIL, format!("{error}"))
}

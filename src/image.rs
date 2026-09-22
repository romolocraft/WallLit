use std::path::Path;

use windows::core::*;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

use crate::renderer::Gpu;

const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "jfif", "png", "bmp", "tif", "tiff", "webp", "ico", "dib", "heic",
    "heif", "avif",
];

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .map(|e| IMAGE_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or(false)
}

pub struct StillImage {
    _texture: ID3D11Texture2D,
    pub view: ID3D11ShaderResourceView,
    pub width: u32,
    pub height: u32,
}

impl StillImage {
    pub fn load(gpu: &Gpu, path: &Path) -> Result<Self> {
        let (pixels, width, height) = decode(path)?;
        Self::from_bgra(gpu, &pixels, width, height)
    }

    pub fn from_bgra(gpu: &Gpu, pixels: &[u8], width: u32, height: u32) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_IMMUTABLE,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let initial = D3D11_SUBRESOURCE_DATA {
            pSysMem: pixels.as_ptr() as *const _,
            SysMemPitch: width * 4,
            SysMemSlicePitch: 0,
        };

        let mut texture = None;
        unsafe { gpu.device.CreateTexture2D(&desc, Some(&initial), Some(&mut texture)) }?;
        let texture = texture.unwrap();

        let mut view = None;
        unsafe { gpu.device.CreateShaderResourceView(&texture, None, Some(&mut view)) }?;

        Ok(Self {
            _texture: texture,
            view: view.unwrap(),
            width,
            height,
        })
    }
}

pub fn decode_scaled(path: &Path, limit: Option<(u32, u32)>) -> Result<(Vec<u8>, u32, u32)> {
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;

    let decoder = unsafe {
        factory.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }?;

    let frame = unsafe { decoder.GetFrame(0) }?;

    let mut width = 0u32;
    let mut height = 0u32;
    unsafe { frame.GetSize(&mut width, &mut height) }?;

    let (target_width, target_height) = match limit {
        Some((max_width, max_height)) if width > max_width || height > max_height => {
            let scale = (max_width as f64 / width as f64).min(max_height as f64 / height as f64);
            (
                ((width as f64 * scale).round() as u32).max(1),
                ((height as f64 * scale).round() as u32).max(1),
            )
        }
        _ => (width, height),
    };

    let converter = unsafe { factory.CreateFormatConverter() }?;

    if (target_width, target_height) == (width, height) {
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
    } else {
        let scaler = unsafe { factory.CreateBitmapScaler() }?;
        unsafe {
            scaler.Initialize(
                &frame,
                target_width,
                target_height,
                WICBitmapInterpolationModeHighQualityCubic,
            )
        }?;
        unsafe {
            converter.Initialize(
                &scaler,
                &GUID_WICPixelFormat32bppPBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
        }?;
    }

    let stride = target_width * 4;
    let mut pixels = vec![0u8; (stride * target_height) as usize];
    unsafe { converter.CopyPixels(std::ptr::null(), stride, &mut pixels) }?;

    Ok((pixels, target_width, target_height))
}

fn decode(path: &Path) -> Result<(Vec<u8>, u32, u32)> {
    decode_scaled(path, None)
}

pub fn measure(path: &Path) -> Result<(u32, u32)> {
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;

    let decoder = unsafe {
        factory.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )
    }?;

    let frame = unsafe { decoder.GetFrame(0) }?;

    let mut width = 0u32;
    let mut height = 0u32;
    unsafe { frame.GetSize(&mut width, &mut height) }?;
    Ok((width, height))
}

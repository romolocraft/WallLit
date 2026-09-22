use std::ffi::c_void;

use windows::core::*;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};
use windows::Win32::Media::MediaFoundation::*;

use crate::renderer::ColorMatrix;

const FIRST_VIDEO_STREAM: u32 = 0xFFFF_FFFC;
const ALL_STREAMS: u32 = 0xFFFF_FFFE;

const READER_END_OF_STREAM: u32 = 0x0000_0002;

pub struct MediaFoundation;

impl MediaFoundation {
    pub fn startup() -> Result<Self> {
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE) }?;
        Ok(Self)
    }
}

impl Drop for MediaFoundation {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

pub fn create_device_manager(device: &ID3D11Device) -> Result<IMFDXGIDeviceManager> {
    let mut token = 0u32;
    let mut manager = None;
    unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }?;

    let manager = manager.unwrap();
    unsafe { manager.ResetDevice(device, token) }?;
    Ok(manager)
}

pub struct Frame {
    pub texture: ID3D11Texture2D,

    pub subresource: u32,

    pub timestamp: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub matrix: ColorMatrix,
    pub full_range: bool,

    pub duration_100ns: i64,
}

impl VideoInfo {
    pub fn fps(&self) -> f64 {
        if self.fps_den == 0 {
            30.0
        } else {
            self.fps_num as f64 / self.fps_den as f64
        }
    }

    pub fn frame_interval_100ns(&self, speed: f32) -> i64 {
        let fps = self.fps() * speed.clamp(0.05, 8.0) as f64;
        if fps <= 0.0 {
            333_333
        } else {
            (10_000_000.0 / fps).round() as i64
        }
    }
}

pub struct VideoSource {
    reader: IMFSourceReader,
    pub info: VideoInfo,
}

impl VideoSource {
    pub fn open(path: &str, manager: &IMFDXGIDeviceManager) -> Result<Self> {
        let attributes = {
            let mut a = None;
            unsafe { MFCreateAttributes(&mut a, 4) }?;
            a.unwrap()
        };

        unsafe {
            attributes.SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, manager)?;
            attributes.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1)?;

            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_ADVANCED_VIDEO_PROCESSING, 1)?;
        }

        let reader =
            unsafe { MFCreateSourceReaderFromURL(&HSTRING::from(path), &attributes) }?;

        unsafe {
            reader.SetStreamSelection(ALL_STREAMS, false)?;
            reader.SetStreamSelection(FIRST_VIDEO_STREAM, true)?;

            let desired = MFCreateMediaType()?;
            desired.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            desired.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
            reader.SetCurrentMediaType(FIRST_VIDEO_STREAM, None, &desired)?;
        }

        let actual = unsafe { reader.GetCurrentMediaType(FIRST_VIDEO_STREAM) }?;
        let mut info = read_video_info(&actual)?;
        info.duration_100ns = read_duration(&reader);

        Ok(Self { reader, info })
    }

    pub fn next_frame(&mut self) -> Result<Option<Frame>> {
        let mut flags = 0u32;
        let mut timestamp = 0i64;
        let mut sample: Option<IMFSample> = None;

        unsafe {
            self.reader.ReadSample(
                FIRST_VIDEO_STREAM,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )
        }?;

        if flags & READER_END_OF_STREAM != 0 {
            self.restart()?;
            return Ok(None);
        }

        let Some(sample) = sample else { return Ok(None) };

        let buffer = unsafe { sample.GetBufferByIndex(0) }?;
        let dxgi_buffer: IMFDXGIBuffer = buffer.cast()?;

        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe {
            dxgi_buffer.GetResource(
                &ID3D11Texture2D::IID,
                &mut texture as *mut _ as *mut *mut c_void,
            )
        }?;

        let subresource = unsafe { dxgi_buffer.GetSubresourceIndex() }?;

        Ok(texture.map(|texture| Frame { texture, subresource, timestamp }))
    }

    pub fn restart(&mut self) -> Result<()> {
        let position = PROPVARIANT::from(0i64);
        unsafe { self.reader.SetCurrentPosition(&GUID::zeroed(), &position) }
    }
}

fn read_duration(reader: &IMFSourceReader) -> i64 {
    const MEDIA_SOURCE: u32 = 0xFFFF_FFFF;

    unsafe { reader.GetPresentationAttribute(MEDIA_SOURCE, &MF_PD_DURATION) }
        .ok()
        .and_then(|value| {
            unsafe { windows::Win32::System::Com::StructuredStorage::PropVariantToUInt64(&value) }
                .ok()
        })
        .unwrap_or(0) as i64
}

fn read_video_info(media_type: &IMFMediaType) -> Result<VideoInfo> {
    let frame_size = unsafe { media_type.GetUINT64(&MF_MT_FRAME_SIZE) }?;
    let width = (frame_size >> 32) as u32;
    let height = frame_size as u32;

    let (fps_num, fps_den) = match unsafe { media_type.GetUINT64(&MF_MT_FRAME_RATE) } {
        Ok(rate) => ((rate >> 32) as u32, rate as u32),
        Err(_) => (30, 1),
    };

    let matrix = match unsafe { media_type.GetUINT32(&MF_MT_YUV_MATRIX) } {
        Ok(v) if v == MFVideoTransferMatrix_BT601.0 as u32 => ColorMatrix::Bt601,
        Ok(v) if v == MFVideoTransferMatrix_BT709.0 as u32 => ColorMatrix::Bt709,
        Ok(v) if v == MFVideoTransferMatrix_BT2020_10.0 as u32 => ColorMatrix::Bt2020,
        Ok(v) if v == MFVideoTransferMatrix_BT2020_12.0 as u32 => ColorMatrix::Bt2020,
        _ => {
            if height >= 720 {
                ColorMatrix::Bt709
            } else {
                ColorMatrix::Bt601
            }
        }
    };

    let full_range = matches!(
        unsafe { media_type.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) },
        Ok(v) if v == MFNominalRange_0_255.0 as u32
    );

    Ok(VideoInfo {
        width,
        height,
        fps_num,
        fps_den,
        matrix,
        full_range,
        duration_100ns: 0,
    })
}

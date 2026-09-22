use windows::core::*;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS_BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper_vs.cso"));
const PS_BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/wallpaper_ps.cso"));
const IMAGE_PS_BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/image_ps.cso"));
const BLEND_VS_BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/transition_vs.cso"));
const BLEND_PS_BYTECODE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/transition_ps.cso"));

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitMode {
    Fill,

    Fit,

    Stretch,

    Center,

    Custom,
}

impl FitMode {
    pub const ALL: [FitMode; 5] = [
        FitMode::Fill,
        FitMode::Fit,
        FitMode::Stretch,
        FitMode::Center,
        FitMode::Custom,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
    pub brightness: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub temperature: f32,
}

impl Default for Look {
    fn default() -> Self {
        Self { brightness: 0.0, contrast: 1.0, saturation: 1.0, temperature: 0.0 }
    }
}

impl Look {
    pub fn is_neutral(&self) -> bool {
        *self == Self::default()
    }

    fn row(&self) -> [f32; 4] {
        [self.brightness, self.contrast, self.saturation, self.temperature]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Area {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Area {
    pub fn full(size: (u32, u32)) -> Self {
        Self { x: 0.0, y: 0.0, w: size.0 as f32, h: size.1 as f32 }
    }

    fn size(&self) -> (u32, u32) {
        (self.w.round().max(1.0) as u32, self.h.round().max(1.0) as u32)
    }

    fn viewport(&self) -> D3D11_VIEWPORT {
        D3D11_VIEWPORT {
            TopLeftX: self.x,
            TopLeftY: self.y,
            Width: self.w,
            Height: self.h,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Placement {
    pub mode: FitMode,
    pub scale: f32,
    pub offset_x: f32,
    pub offset_y: f32,

    pub stretch_x: f32,
    pub stretch_y: f32,
    pub look: Look,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            mode: FitMode::Fill,
            scale: 1.0,
            offset_x: 0.0,
            offset_y: 0.0,
            stretch_x: 1.0,
            stretch_y: 1.0,
            look: Look::default(),
        }
    }
}

impl Placement {
    pub fn sampling_scale(&self, video: (u32, u32), dst: (u32, u32)) -> (f32, f32) {
        if video.0 == 0 || video.1 == 0 || dst.0 == 0 || dst.1 == 0 {
            return (1.0, 1.0);
        }

        let va = video.0 as f32 / video.1 as f32;
        let wa = dst.0 as f32 / dst.1 as f32;

        let (sx, sy) = match self.mode {
            FitMode::Fill | FitMode::Custom => {
                if wa > va {
                    (1.0, va / wa)
                } else {
                    (wa / va, 1.0)
                }
            }
            FitMode::Fit => {
                if wa > va {
                    (wa / va, 1.0)
                } else {
                    (1.0, va / wa)
                }
            }
            FitMode::Stretch => (1.0, 1.0),
            FitMode::Center => (dst.0 as f32 / video.0 as f32, dst.1 as f32 / video.1 as f32),
        };

        if self.mode != FitMode::Custom {
            return (sx, sy);
        }

        let s = self.scale.max(0.01);
        let ex = self.stretch_x.max(0.05);
        let ey = self.stretch_y.max(0.05);
        (sx / (s * ex), sy / (s * ey))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorMatrix {
    Bt601,
    Bt709,
    Bt2020,
}

impl ColorMatrix {
    fn kr_kb(self) -> (f32, f32) {
        match self {
            ColorMatrix::Bt601 => (0.299, 0.114),
            ColorMatrix::Bt709 => (0.2126, 0.0722),
            ColorMatrix::Bt2020 => (0.2627, 0.0593),
        }
    }
}

fn color_rows(matrix: ColorMatrix, full_range: bool) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let (kr, kb) = matrix.kr_kb();
    let kg = 1.0 - kr - kb;

    let (y_gain, c_gain, black) = if full_range {
        (1.0, 1.0, 0.0)
    } else {
        (255.0 / 219.0, 255.0 / 224.0, 16.0 / 255.0)
    };

    let r_v = (2.0 - 2.0 * kr) * c_gain;
    let b_u = (2.0 - 2.0 * kb) * c_gain;
    let g_u = -(kb * (2.0 - 2.0 * kb) / kg) * c_gain;
    let g_v = -(kr * (2.0 - 2.0 * kr) / kg) * c_gain;

    let bias = y_gain * black;
    (
        [y_gain, 0.0, r_v, -(bias + r_v * 0.5)],
        [y_gain, g_u, g_v, -(bias + (g_u + g_v) * 0.5)],
        [y_gain, b_u, 0.0, -(bias + b_u * 0.5)],
    )
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Params {
    uv_scale_offset: [f32; 4],
    frame_frac: [f32; 4],
    look: [f32; 4],
    ycbcr_r: [f32; 4],
    ycbcr_g: [f32; 4],
    ycbcr_b: [f32; 4],
}

#[derive(Clone, Copy)]
pub struct FrameRef<'a> {
    pub texture: &'a ID3D11Texture2D,
    pub subresource: u32,
    pub size: (u32, u32),
    pub matrix: ColorMatrix,
    pub full_range: bool,
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    factory: IDXGIFactory2,
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    sampler: ID3D11SamplerState,
    cbuffer: ID3D11Buffer,
    raster: ID3D11RasterizerState,

    image_ps: ID3D11PixelShader,

    blend_vs: ID3D11VertexShader,
    blend_ps: ID3D11PixelShader,
    blend_cbuffer: ID3D11Buffer,

    layer_blend: ID3D11BlendState,
}

impl Gpu {
    pub fn new() -> Result<Self> {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;

        let flags = D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT;
        let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1];

        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                flags,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }?;

        let device = device.unwrap();
        let context = context.unwrap();

        let mt: ID3D11Multithread = context.cast()?;
        unsafe {
            let _ = mt.SetMultithreadProtected(true);
        }

        let dxgi_device: IDXGIDevice = device.cast()?;
        let adapter = unsafe { dxgi_device.GetAdapter() }?;
        let factory: IDXGIFactory2 = unsafe { adapter.GetParent() }?;

        let mut vs = None;
        unsafe { device.CreateVertexShader(VS_BYTECODE, None, Some(&mut vs)) }?;
        let mut ps = None;
        unsafe { device.CreatePixelShader(PS_BYTECODE, None, Some(&mut ps)) }?;

        let sampler_desc = D3D11_SAMPLER_DESC {
            Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR,
            AddressU: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressV: D3D11_TEXTURE_ADDRESS_CLAMP,
            AddressW: D3D11_TEXTURE_ADDRESS_CLAMP,
            ComparisonFunc: D3D11_COMPARISON_NEVER,
            MaxLOD: f32::MAX,
            ..Default::default()
        };
        let mut sampler = None;
        unsafe { device.CreateSamplerState(&sampler_desc, Some(&mut sampler)) }?;

        let cb_desc = D3D11_BUFFER_DESC {
            ByteWidth: std::mem::size_of::<Params>() as u32,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut cbuffer = None;
        unsafe { device.CreateBuffer(&cb_desc, None, Some(&mut cbuffer)) }?;

        let raster_desc = D3D11_RASTERIZER_DESC {
            FillMode: D3D11_FILL_SOLID,
            CullMode: D3D11_CULL_NONE,
            DepthClipEnable: true.into(),
            ..Default::default()
        };
        let mut raster = None;
        unsafe { device.CreateRasterizerState(&raster_desc, Some(&mut raster)) }?;

        let mut image_ps = None;
        unsafe { device.CreatePixelShader(IMAGE_PS_BYTECODE, None, Some(&mut image_ps)) }?;

        let mut blend_vs = None;
        unsafe { device.CreateVertexShader(BLEND_VS_BYTECODE, None, Some(&mut blend_vs)) }?;
        let mut blend_ps = None;
        unsafe { device.CreatePixelShader(BLEND_PS_BYTECODE, None, Some(&mut blend_ps)) }?;

        let blend_cb_desc = D3D11_BUFFER_DESC {
            ByteWidth: 16,
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let mut blend_cbuffer = None;
        unsafe { device.CreateBuffer(&blend_cb_desc, None, Some(&mut blend_cbuffer)) }?;

        let mut layer_target = D3D11_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            SrcBlend: D3D11_BLEND_ONE,
            DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOp: D3D11_BLEND_OP_ADD,
            SrcBlendAlpha: D3D11_BLEND_ONE,
            DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
            BlendOpAlpha: D3D11_BLEND_OP_ADD,
            RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let mut blend_desc = D3D11_BLEND_DESC::default();
        blend_desc.RenderTarget[0] = std::mem::take(&mut layer_target);
        let mut layer_blend = None;
        unsafe { device.CreateBlendState(&blend_desc, Some(&mut layer_blend)) }?;

        Ok(Self {
            device,
            context,
            factory,
            vs: vs.unwrap(),
            ps: ps.unwrap(),
            sampler: sampler.unwrap(),
            cbuffer: cbuffer.unwrap(),
            raster: raster.unwrap(),
            image_ps: image_ps.unwrap(),
            blend_vs: blend_vs.unwrap(),
            blend_ps: blend_ps.unwrap(),
            blend_cbuffer: blend_cbuffer.unwrap(),
            layer_blend: layer_blend.unwrap(),
        })
    }
}

struct Nv12Target {
    texture: ID3D11Texture2D,
    srv_y: ID3D11ShaderResourceView,
    srv_uv: ID3D11ShaderResourceView,
    width: u32,
    height: u32,
}

#[derive(Default)]
pub struct VideoPainter {
    nv12: Option<Nv12Target>,

    last: Option<((u32, u32), ColorMatrix, bool)>,

    last_image: Option<(ID3D11ShaderResourceView, (u32, u32))>,
}

impl VideoPainter {
    fn ensure_nv12(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<()> {
        if matches!(&self.nv12, Some(t) if t.width == width && t.height == height) {
            return Ok(());
        }

        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_NV12,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            ..Default::default()
        };

        let mut texture = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
        let texture = texture.unwrap();

        let srv_y = plane_view(&gpu.device, &texture, DXGI_FORMAT_R8_UNORM)?;
        let srv_uv = plane_view(&gpu.device, &texture, DXGI_FORMAT_R8G8_UNORM)?;

        self.nv12 = Some(Nv12Target { texture, srv_y, srv_uv, width, height });
        Ok(())
    }

    pub fn paint(
        &mut self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        frame: FrameRef,
        placement: Placement,
    ) -> Result<()> {
        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { frame.texture.GetDesc(&mut src_desc) };

        self.ensure_nv12(gpu, src_desc.Width, src_desc.Height)?;
        let nv12 = self.nv12.as_ref().unwrap();

        unsafe {
            gpu.context.CopySubresourceRegion(
                &nv12.texture,
                0,
                0,
                0,
                0,
                frame.texture,
                frame.subresource,
                None,
            );
        }

        self.last = Some((frame.size, frame.matrix, frame.full_range));
        self.last_image = None;
        self.render(gpu, target, area, placement)
    }

    pub fn paint_image(
        &mut self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        let size = (image.width, image.height);
        self.render_image(gpu, target, area, &image.view, size, placement)?;

        self.last = None;
        self.last_image = Some((image.view.clone(), size));
        Ok(())
    }

    fn render_image(
        &self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        view: &ID3D11ShaderResourceView,
        size: (u32, u32),
        placement: Placement,
    ) -> Result<()> {
        let (sx, sy) = placement.sampling_scale(size, area.size());

        let params = Params {
            uv_scale_offset: [sx, sy, placement.offset_x, placement.offset_y],

            frame_frac: [1.0, 1.0, 0.0, 0.0],
            look: placement.look.row(),
            ycbcr_r: [0.0; 4],
            ycbcr_g: [0.0; 4],
            ycbcr_b: [0.0; 4],
        };

        let ctx = &gpu.context;
        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&gpu.cbuffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(&params, mapped.pData as *mut Params, 1);
            ctx.Unmap(&gpu.cbuffer, 0);

            ctx.RSSetViewports(Some(&[area.viewport()]));
            ctx.RSSetState(&gpu.raster);
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);

            ctx.OMSetBlendState(&gpu.layer_blend, None, 0xFFFF_FFFF);

            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&gpu.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(gpu.cbuffer.clone())]));
            ctx.PSSetShader(&gpu.image_ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(gpu.cbuffer.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));

            ctx.PSSetShaderResources(2, Some(&[Some(view.clone())]));

            ctx.Draw(3, 0);

            ctx.PSSetShaderResources(2, Some(&[None]));
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.OMSetRenderTargets(None, None);
        }

        Ok(())
    }

    pub fn overlay_image(
        &self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        let size = (image.width, image.height);
        self.render_image(gpu, target, area, &image.view, size, placement)
    }

    pub fn repaint(
        &mut self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        placement: Placement,
    ) -> Result<bool> {
        if let Some((view, size)) = self.last_image.clone() {
            self.render_image(gpu, target, area, &view, size, placement)?;
            return Ok(true);
        }

        if self.nv12.is_none() || self.last.is_none() {
            return Ok(false);
        }
        self.render(gpu, target, area, placement)?;
        Ok(true)
    }

    fn render(
        &self,
        gpu: &Gpu,
        target: &ID3D11RenderTargetView,
        area: Area,
        placement: Placement,
    ) -> Result<()> {
        let nv12 = self.nv12.as_ref().unwrap();
        let (size, matrix, full_range) = self.last.unwrap();

        let ctx = &gpu.context;
        let (sx, sy) = placement.sampling_scale(size, area.size());
        let (r, g, b) = color_rows(matrix, full_range);

        let params = Params {
            uv_scale_offset: [sx, sy, placement.offset_x, placement.offset_y],
            frame_frac: [
                size.0 as f32 / nv12.width as f32,
                size.1 as f32 / nv12.height as f32,
                0.0,
                0.0,
            ],
            look: placement.look.row(),
            ycbcr_r: r,
            ycbcr_g: g,
            ycbcr_b: b,
        };

        unsafe {
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&gpu.cbuffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
            std::ptr::copy_nonoverlapping(&params, mapped.pData as *mut Params, 1);
            ctx.Unmap(&gpu.cbuffer, 0);

            ctx.RSSetViewports(Some(&[area.viewport()]));
            ctx.RSSetState(&gpu.raster);
            ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);

            ctx.OMSetBlendState(&gpu.layer_blend, None, 0xFFFF_FFFF);

            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.VSSetShader(&gpu.vs, None);
            ctx.VSSetConstantBuffers(0, Some(&[Some(gpu.cbuffer.clone())]));
            ctx.PSSetShader(&gpu.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(gpu.cbuffer.clone())]));
            ctx.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));
            ctx.PSSetShaderResources(
                0,
                Some(&[Some(nv12.srv_y.clone()), Some(nv12.srv_uv.clone())]),
            );

            ctx.Draw(3, 0);

            ctx.PSSetShaderResources(0, Some(&[None, None]));
            ctx.OMSetBlendState(None, None, 0xFFFF_FFFF);
            ctx.OMSetRenderTargets(None, None);
        }

        Ok(())
    }
}

struct Blending {
    kind: u32,
    started: crate::stats::Instant,
    duration_100ns: i64,
}

pub struct Surface {
    swapchain: IDXGISwapChain1,
    rtv: ID3D11RenderTargetView,
    width: u32,
    height: u32,

    painters: Vec<VideoPainter>,

    pub present_model: &'static str,

    outgoing: Option<Offscreen>,

    incoming: Option<Offscreen>,
    blending: Option<Blending>,
}

impl Surface {
    pub fn new(
        gpu: &Gpu,
        hwnd: HWND,
        width: u32,
        height: u32,
        preferred: Option<&str>,
    ) -> Result<Self> {
        let (swapchain, present_model) = create_swapchain(gpu, hwnd, width, height, preferred)?;
        let rtv = create_rtv(&gpu.device, &swapchain)?;
        Ok(Self {
            swapchain,
            rtv,
            width,
            height,
            painters: Vec::new(),
            present_model,
            outgoing: None,
            incoming: None,
            blending: None,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn target(&self) -> &ID3D11RenderTargetView {
        match (&self.blending, &self.incoming) {
            (Some(_), Some(incoming)) => &incoming.rtv,
            _ => &self.rtv,
        }
    }

    pub fn begin(&mut self, gpu: &Gpu, layers: usize) {
        if self.painters.len() < layers {
            self.painters.resize_with(layers, VideoPainter::default);
        }

        let target = self.target().clone();
        unsafe {
            gpu.context
                .ClearRenderTargetView(&target, &[0.0, 0.0, 0.0, 1.0]);
        }
    }

    pub fn draw_layer(
        &mut self,
        gpu: &Gpu,
        layer: usize,
        area: Area,
        frame: FrameRef,
        placement: Placement,
    ) -> Result<()> {
        let target = self.target().clone();
        let Some(painter) = self.painters.get_mut(layer) else {
            return Ok(());
        };
        painter.paint(gpu, &target, area, frame, placement)
    }

    pub fn draw_image_layer(
        &mut self,
        gpu: &Gpu,
        layer: usize,
        area: Area,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        let target = self.target().clone();
        let Some(painter) = self.painters.get_mut(layer) else {
            return Ok(());
        };
        painter.paint_image(gpu, &target, area, image, placement)
    }

    pub fn repaint_layer(
        &mut self,
        gpu: &Gpu,
        layer: usize,
        area: Area,
        placement: Placement,
    ) -> Result<()> {
        let target = self.target().clone();
        let Some(painter) = self.painters.get_mut(layer) else {
            return Ok(());
        };
        painter.repaint(gpu, &target, area, placement)?;
        Ok(())
    }

    pub fn fill(&mut self, gpu: &Gpu, rgba: [f32; 4]) {
        let target = self.target().clone();
        unsafe { gpu.context.ClearRenderTargetView(&target, &rgba) };
    }

    pub fn finish(&mut self, gpu: &Gpu) -> Result<()> {
        let Some(blending) = self.blending.as_ref() else {
            return self.present();
        };

        let elapsed_ms = blending.started.elapsed_ms();
        let total_ms = blending.duration_100ns as f64 / 10_000.0;
        let progress = (elapsed_ms / total_ms).clamp(0.0, 1.0) as f32;
        let kind = blending.kind;

        if self.incoming.is_none() {
            self.release_blend_targets();
            return self.present();
        }

        self.blend(gpu, kind, progress)?;

        if progress >= 1.0 {
            self.release_blend_targets();
        }

        self.present()
    }

    pub fn begin_transition(
        &mut self,
        gpu: &Gpu,
        kind: u32,
        duration_100ns: i64,
        layers: &[(Area, Placement)],
    ) -> Result<()> {
        if duration_100ns <= 0 {
            return Ok(());
        }

        self.ensure_blend_targets(gpu)?;

        let Some(outgoing) = self.outgoing.as_ref() else {
            return Ok(());
        };
        let target = outgoing.rtv.clone();

        unsafe {
            gpu.context
                .ClearRenderTargetView(&target, &[0.0, 0.0, 0.0, 1.0]);
        }

        let mut any = false;
        for (index, (area, placement)) in layers.iter().enumerate() {
            let Some(painter) = self.painters.get_mut(index) else {
                continue;
            };
            any |= painter.repaint(gpu, &target, *area, *placement)?;
        }

        if !any {
            return Ok(());
        }

        self.blending = Some(Blending {
            kind,
            started: crate::stats::Instant::now(),
            duration_100ns,
        });

        Ok(())
    }

    pub fn is_blending(&self) -> bool {
        self.blending.is_some()
    }

    fn ensure_blend_targets(&mut self, gpu: &Gpu) -> Result<()> {
        if self.outgoing.is_none() {
            self.outgoing = Some(Offscreen::new(gpu, self.width, self.height)?);
        }
        if self.incoming.is_none() {
            self.incoming = Some(Offscreen::new(gpu, self.width, self.height)?);
        }
        Ok(())
    }

    fn release_blend_targets(&mut self) {
        self.blending = None;
        self.outgoing = None;
        self.incoming = None;
    }

    fn blend(&self, gpu: &Gpu, kind: u32, progress: f32) -> Result<()> {
        let (Some(outgoing), Some(incoming)) = (self.outgoing.as_ref(), self.incoming.as_ref())
        else {
            return Ok(());
        };

        blend_pass(
            gpu,
            &self.rtv,
            (self.width, self.height),
            outgoing,
            incoming,
            kind,
            progress,
        )
    }

    pub fn clear_to(&mut self, gpu: &Gpu, rgba: [f32; 4]) -> Result<()> {
        unsafe { gpu.context.ClearRenderTargetView(&self.rtv, &rgba) };
        self.present()
    }

    fn present(&self) -> Result<()> {
        unsafe { self.swapchain.Present(0, DXGI_PRESENT(0)).ok() }
    }

    pub fn resize(&mut self, gpu: &Gpu, width: u32, height: u32) -> Result<()> {
        if width == self.width && height == self.height {
            return Ok(());
        }

        unsafe {
            gpu.context.OMSetRenderTargets(None, None);
            gpu.context.ClearState();
            self.swapchain.ResizeBuffers(
                0,
                width,
                height,
                DXGI_FORMAT_UNKNOWN,
                DXGI_SWAP_CHAIN_FLAG(0),
            )?;
        }

        self.rtv = create_rtv(&gpu.device, &self.swapchain)?;
        self.width = width;
        self.height = height;
        Ok(())
    }
}

pub struct Offscreen {
    pub texture: ID3D11Texture2D,
    rtv: ID3D11RenderTargetView,

    srv: ID3D11ShaderResourceView,
    pub width: u32,
    pub height: u32,
    painter: VideoPainter,
}

impl Offscreen {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Result<Self> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width.max(1),
            Height: height.max(1),
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            ..Default::default()
        };

        let mut texture = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut texture)) }?;
        let texture = texture.unwrap();

        let mut rtv = None;
        unsafe { gpu.device.CreateRenderTargetView(&texture, None, Some(&mut rtv)) }?;

        let mut srv = None;
        unsafe { gpu.device.CreateShaderResourceView(&texture, None, Some(&mut srv)) }?;

        Ok(Self {
            texture,
            rtv: rtv.unwrap(),
            srv: srv.unwrap(),
            width: desc.Width,
            height: desc.Height,
            painter: VideoPainter::default(),
        })
    }

    pub fn area(&self) -> Area {
        Area::full((self.width, self.height))
    }

    pub fn paint(&mut self, gpu: &Gpu, frame: FrameRef, placement: Placement) -> Result<()> {
        let area = self.area();
        self.painter.paint(gpu, &self.rtv, area, frame, placement)
    }

    pub fn paint_into(
        &mut self,
        gpu: &Gpu,
        area: Area,
        frame: FrameRef,
        placement: Placement,
    ) -> Result<()> {
        self.painter.paint(gpu, &self.rtv, area, frame, placement)
    }

    pub fn paint_image(
        &mut self,
        gpu: &Gpu,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        let area = self.area();
        self.painter
            .paint_image(gpu, &self.rtv, area, image, placement)
    }

    pub fn paint_image_into(
        &mut self,
        gpu: &Gpu,
        area: Area,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        self.painter
            .paint_image(gpu, &self.rtv, area, image, placement)
    }

    pub fn overlay_image(
        &self,
        gpu: &Gpu,
        area: Area,
        image: &crate::image::StillImage,
        placement: Placement,
    ) -> Result<()> {
        self.painter
            .overlay_image(gpu, &self.rtv, area, image, placement)
    }

    pub fn repaint(&mut self, gpu: &Gpu, placement: Placement) -> Result<bool> {
        let area = self.area();
        self.painter.repaint(gpu, &self.rtv, area, placement)
    }

    pub fn view_target(&self) -> &ID3D11RenderTargetView {
        &self.rtv
    }

    pub fn clear(&self, gpu: &Gpu, rgba: [f32; 4]) {
        unsafe { gpu.context.ClearRenderTargetView(&self.rtv, &rgba) };
    }

    pub fn read_pixels(&self, gpu: &Gpu) -> Result<Pixels> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.width,
            Height: self.height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_STAGING,
            BindFlags: 0,
            CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
            MiscFlags: 0,
        };

        let mut staging = None;
        unsafe { gpu.device.CreateTexture2D(&desc, None, Some(&mut staging)) }?;
        let staging = staging.unwrap();

        let stride = self.width as usize * 4;
        let mut data = vec![0u8; stride * self.height as usize];

        unsafe {
            gpu.context.CopyResource(&staging, &self.texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            gpu.context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))?;

            for y in 0..self.height as usize {
                let source = (mapped.pData as *const u8).add(y * mapped.RowPitch as usize);
                let target = data.as_mut_ptr().add(y * stride);
                std::ptr::copy_nonoverlapping(source, target, stride);
            }

            gpu.context.Unmap(&staging, 0);
        }

        Ok(Pixels { data, width: self.width, height: self.height, stride: stride as u32 })
    }
}

pub struct Pixels {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
}

fn create_rtv(device: &ID3D11Device, swapchain: &IDXGISwapChain1) -> Result<ID3D11RenderTargetView> {
    let back: ID3D11Texture2D = unsafe { swapchain.GetBuffer(0) }?;
    let mut rtv = None;
    unsafe { device.CreateRenderTargetView(&back, None, Some(&mut rtv)) }?;
    Ok(rtv.unwrap())
}

fn create_swapchain(
    gpu: &Gpu,
    hwnd: HWND,
    width: u32,
    height: u32,
    preferred: Option<&str>,
) -> Result<(IDXGISwapChain1, &'static str)> {
    let all = [
        (DXGI_SWAP_EFFECT_FLIP_DISCARD, "flip-discard"),
        (DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL, "flip-sequential"),
        (DXGI_SWAP_EFFECT_DISCARD, "bitblt-discard"),
        (DXGI_SWAP_EFFECT_SEQUENTIAL, "bitblt-sequential"),
    ];

    let effects: Vec<_> = match preferred {
        Some(name) => all.iter().filter(|(_, n)| *n == name).copied().collect(),
        None => all.to_vec(),
    };

    let mut last = Error::from_win32();
    for (effect, name) in effects {
        let flip =
            effect == DXGI_SWAP_EFFECT_FLIP_DISCARD || effect == DXGI_SWAP_EFFECT_FLIP_SEQUENTIAL;

        let desc = DXGI_SWAP_CHAIN_DESC1 {
            Width: width,
            Height: height,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: if flip { 2 } else { 1 },
            Scaling: DXGI_SCALING_STRETCH,
            SwapEffect: effect,
            AlphaMode: DXGI_ALPHA_MODE_IGNORE,
            ..Default::default()
        };

        match unsafe { gpu.factory.CreateSwapChainForHwnd(&gpu.device, hwnd, &desc, None, None) } {
            Ok(sc) => return Ok((sc, name)),
            Err(e) => last = e,
        }
    }

    Err(last)
}

fn plane_view(
    device: &ID3D11Device,
    texture: &ID3D11Texture2D,
    format: DXGI_FORMAT,
) -> Result<ID3D11ShaderResourceView> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
            Texture2D: D3D11_TEX2D_SRV { MostDetailedMip: 0, MipLevels: 1 },
        },
    };

    let mut srv = None;
    unsafe { device.CreateShaderResourceView(texture, Some(&desc), Some(&mut srv)) }?;
    Ok(srv.unwrap())
}

pub fn blend_pass(
gpu: &Gpu,
target: &ID3D11RenderTargetView,
size: (u32, u32),
outgoing: &Offscreen,
incoming: &Offscreen,
kind: u32,
progress: f32,
) -> Result<()> {
    let ctx = &gpu.context;
    let params = [progress, kind as f32, 0.0, 0.0];

    unsafe {
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(&gpu.blend_cbuffer, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut mapped))?;
        std::ptr::copy_nonoverlapping(&params, mapped.pData as *mut [f32; 4], 1);
        ctx.Unmap(&gpu.blend_cbuffer, 0);

        let viewport = D3D11_VIEWPORT {
            TopLeftX: 0.0,
            TopLeftY: 0.0,
            Width: size.0 as f32,
            Height: size.1 as f32,
            MinDepth: 0.0,
            MaxDepth: 1.0,
        };
        ctx.RSSetViewports(Some(&[viewport]));
        ctx.RSSetState(&gpu.raster);
        ctx.OMSetRenderTargets(Some(&[Some(target.clone())]), None);

        ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ctx.VSSetShader(&gpu.blend_vs, None);
        ctx.PSSetShader(&gpu.blend_ps, None);
        ctx.PSSetConstantBuffers(0, Some(&[Some(gpu.blend_cbuffer.clone())]));
        ctx.PSSetSamplers(0, Some(&[Some(gpu.sampler.clone())]));
        ctx.PSSetShaderResources(
            0,
            Some(&[Some(outgoing.srv.clone()), Some(incoming.srv.clone())]),
        );

        ctx.Draw(3, 0);

        ctx.PSSetShaderResources(0, Some(&[None, None]));
        ctx.OMSetRenderTargets(None, None);
    }

    Ok(())
}

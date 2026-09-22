cbuffer Params : register(b0)
{
    float4 uv_scale_offset;
    float4 frame_frac;
    float4 look;
    float4 ycbcr_r;
    float4 ycbcr_g;
    float4 ycbcr_b;
};

struct VsOut
{
    float4 pos : SV_POSITION;
    float2 uv  : TEXCOORD0;
};

VsOut vs_main(uint vid : SV_VertexID)
{
    float2 ndc = float2((vid == 1) ? 3.0 : -1.0, (vid == 2) ? 3.0 : -1.0);

    VsOut o;
    o.pos = float4(ndc, 0.0, 1.0);

    float2 dst = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);

    o.uv = (dst - 0.5) * uv_scale_offset.xy + 0.5 - uv_scale_offset.zw;
    return o;
}

Texture2D<float>  luma   : register(t0);
Texture2D<float2> chroma : register(t1);
SamplerState      samp   : register(s0);

float3 apply_look(float3 rgb)
{
    rgb += look.x;
    rgb = (rgb - 0.5) * look.y + 0.5;

    float gray = dot(rgb, float3(0.2126, 0.7152, 0.0722));
    rgb = lerp(gray.xxx, rgb, look.z);

    rgb.r += look.w;
    rgb.b -= look.w;

    return saturate(rgb);
}

float coverage(float2 uv)
{
    float2 inside = step(0.0, uv) * step(uv, 1.0);
    return inside.x * inside.y;
}

float4 ps_main(VsOut i) : SV_Target
{
    float2 uvt = i.uv * frame_frac.xy;

    float  y  = luma.Sample(samp, uvt);
    float2 cb = chroma.Sample(samp, uvt);

    float4 yuv1 = float4(y, cb.x, cb.y, 1.0);

    float3 rgb = float3(dot(ycbcr_r, yuv1),
                        dot(ycbcr_g, yuv1),
                        dot(ycbcr_b, yuv1));

    rgb = apply_look(saturate(rgb));

    float mask = coverage(i.uv);
    return float4(rgb * mask, mask);
}

Texture2D<float4> picture : register(t2);

float4 ps_image(VsOut i) : SV_Target
{
    float2 uvt = i.uv * frame_frac.xy;
    float3 rgb = apply_look(picture.Sample(samp, uvt).rgb);

    float mask = coverage(i.uv);
    return float4(rgb * mask, mask);
}

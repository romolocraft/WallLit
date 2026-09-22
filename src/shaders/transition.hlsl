cbuffer Params : register(b0)
{
    float4 params;
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
    o.uv = float2(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return o;
}

Texture2D<float4> outgoing : register(t0);
Texture2D<float4> incoming : register(t1);
SamplerState      samp     : register(s0);

float block_noise(float2 cell)
{
    return frac(sin(dot(cell, float2(12.9898, 78.233))) * 43758.5453);
}

float curtain(float coordinate, float progress)
{
    const float SOFT = 0.12;
    return smoothstep(progress - SOFT, progress + SOFT, coordinate);
}

float4 ps_main(VsOut i) : SV_Target
{
    float progress = saturate(params.x);
    int kind = (int)params.y;

    float2 uv = i.uv;

    float2 uv_out = uv;
    float2 uv_in = uv;

    if (kind == 7)
    {
        float leaving = 1.0 + progress * 0.12;
        float arriving = 1.12 - progress * 0.12;
        uv_out = (uv - 0.5) / leaving + 0.5;
        uv_in = (uv - 0.5) / arriving + 0.5;
    }

    float4 a = outgoing.Sample(samp, uv_out);
    float4 b = incoming.Sample(samp, uv_in);

    float mix = 0.0;

    if (kind == 0)
    {
        mix = step(0.5, progress);
    }
    else if (kind == 1)
    {
        mix = progress;
    }
    else if (kind == 2)
    {
        mix = 1.0 - curtain(uv.x, progress * 1.24 - 0.12);
    }
    else if (kind == 3)
    {
        mix = curtain(uv.x, 1.0 - (progress * 1.24 - 0.12));
    }
    else if (kind == 4)
    {
        mix = 1.0 - curtain(uv.y, progress * 1.24 - 0.12);
    }
    else if (kind == 5)
    {
        mix = curtain(uv.y, 1.0 - (progress * 1.24 - 0.12));
    }
    else if (kind == 6)
    {
        const float BLOCKS = 24.0;
        const float TURN = 0.3;
        float2 cell = floor(uv * float2(BLOCKS, BLOCKS * 9.0 / 16.0));
        float start = block_noise(cell) * (1.0 - TURN);
        mix = smoothstep(start, start + TURN, progress);
    }
    else
    {
        mix = progress;
    }

    return float4(lerp(a.rgb, b.rgb, saturate(mix)), 1.0);
}

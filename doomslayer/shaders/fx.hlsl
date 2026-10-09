// DOOM RING muzzle flashes: Doom Eternal's own flash textures (premultiplied, coloured offline by
// tools/convert_fx.py) on camera-facing quads at the gun's barrel tip, additive blending.
// Compile: dxc -T vs_6_0 -E VSMain fx.hlsl -Fo fx_vs.cso ; dxc -T ps_6_0 -E PSMain fx.hlsl -Fo fx_ps.cso

cbuffer Frame : register(b0)
{
    row_major float4x4 proj;
};

Texture2D tex_flash : register(t0);
SamplerState samp : register(s0);

struct VSIn
{
    float3 pos : POSITION;
    float2 uv  : TEXCOORD0;
    float4 col : COLOR0;
};

struct PSIn
{
    float4 sv  : SV_Position;
    float2 uv  : TEXCOORD0;
    float4 col : COLOR0;
};

PSIn VSMain(VSIn v)
{
    PSIn o;
    o.sv = mul(proj, float4(v.pos, 1.0));
    o.uv = v.uv;
    o.col = v.col;
    return o;
}

float4 PSMain(PSIn i) : SV_Target
{
    // The back buffer is display-referred (gamma), the texture is premultiplied colour.
    float4 t = tex_flash.Sample(samp, i.uv);
    return float4(t.rgb * i.col.rgb * i.col.a, 0.0);
}

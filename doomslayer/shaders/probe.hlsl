// DOOM RING viewmodel light probe: once per frame, measures the game's frame (t5) into a 2x1
// target - x 0 = upper (sky / walls), x 1 = lower (ground) - from a 16x8 grid of filtered taps,
// and eases it towards the new value (t6 = last frame's probe), so the gun's light follows the
// scene smoothly instead of jumping with the few pixels it used to read.
// Compile: dxc -T vs_6_0 -E VSMain probe.hlsl -Fo probe_vs.cso ; dxc -T ps_6_0 -E PSMain probe.hlsl -Fo probe_ps.cso

Texture2D scene : register(t5);
Texture2D<float4> prev : register(t6);
SamplerState samp_clamp : register(s1);

float4 VSMain(uint id : SV_VertexID) : SV_Position
{
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float3 lin(float3 c) { return pow(saturate(c), 2.2); }

float4 PSMain(float4 pos : SV_Position) : SV_Target
{
    bool lower = pos.x >= 1.0;
    // same regions as before: upper = y 0.08..0.42, lower = y 0.55..0.75 of the frame
    float y0 = lower ? 0.55 : 0.0833;
    float y1 = lower ? 0.75 : 0.4167;
    float3 sum = 0;
    [loop] for (int y = 0; y < 8; y++)
    [loop] for (int x = 0; x < 16; x++)
    {
        float2 uv = float2((x + 0.5) / 16.0, y0 + (y1 - y0) * (y + 0.5) / 8.0);
        sum += lin(scene.SampleLevel(samp_clamp, uv, 0).rgb);
    }
    float3 cur = sum / 128.0;
    float4 p = prev.Load(int3(lower ? 1 : 0, 0, 0));
    // first frame (alpha 0): take the measurement as is; then ease (~0.25 s at 60 fps)
    float3 outc = p.a > 0.5 ? lerp(p.rgb, cur, 0.065) : cur;
    return float4(outc, 1.0);
}

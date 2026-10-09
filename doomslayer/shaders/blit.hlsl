// DOOM RING MSAA path: copies the game's frame (t5, the scene copy) into the multisampled target
// so the gun and its effects blend over it exactly as they do without MSAA, before the resolve.
// Compile: dxc -T vs_6_0 -E VSMain blit.hlsl -Fo blit_vs.cso ; dxc -T ps_6_0 -E PSMain blit.hlsl -Fo blit_ps.cso

Texture2D scene : register(t5);

float4 VSMain(uint id : SV_VertexID) : SV_Position
{
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float4 PSMain(float4 p : SV_Position) : SV_Target
{
    return scene.Load(int3(p.xy, 0));
}

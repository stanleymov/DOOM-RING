// DOOM RING gun anti-aliasing (FXAA-style edge smoothing), run only where the viewmodel drew.
// t0 = the gun pass depth (1.0 = no gun), t5 = copy of the frame after the gun was drawn.
// Compile: dxc -T vs_6_0 -E VSMain fxaa.hlsl -Fo fxaa_vs.cso ; dxc -T ps_6_0 -E PSMain fxaa.hlsl -Fo fxaa_ps.cso

Texture2D<float> gun_depth : register(t0);
Texture2D frame_tex : register(t5);
SamplerState samp_clamp : register(s1);

float4 VSMain(uint id : SV_VertexID) : SV_Position
{
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

float luma(float3 c) { return dot(c, float3(0.299, 0.587, 0.114)); }

float3 tap(float2 uv) { return frame_tex.SampleLevel(samp_clamp, uv, 0).rgb; }

float4 PSMain(float4 pos : SV_Position) : SV_Target
{
    int2 p = int2(pos.xy);
    // Only the gun and the pixels right around it (its outline against the world).
    float dmin = min(min(gun_depth.Load(int3(p, 0)), gun_depth.Load(int3(p + int2(1, 0), 0))),
                     min(gun_depth.Load(int3(p - int2(1, 0), 0)),
                         min(gun_depth.Load(int3(p + int2(0, 1), 0)), gun_depth.Load(int3(p - int2(0, 1), 0)))));
    if (dmin >= 1.0) discard;

    uint w, h;
    frame_tex.GetDimensions(w, h);
    float2 px = 1.0 / float2(w, h);
    float2 uv = (pos.xy) * px;

    float3 cM = tap(uv);
    float lM = luma(cM);
    float lN = luma(tap(uv + float2(0, -px.y)));
    float lS = luma(tap(uv + float2(0, px.y)));
    float lE = luma(tap(uv + float2(px.x, 0)));
    float lW = luma(tap(uv + float2(-px.x, 0)));
    float lMin = min(lM, min(min(lN, lS), min(lE, lW)));
    float lMax = max(lM, max(max(lN, lS), max(lE, lW)));
    float range = lMax - lMin;
    if (range < max(0.0312, lMax * 0.125)) return float4(cM, 1.0);

    float lNW = luma(tap(uv + float2(-px.x, -px.y)));
    float lNE = luma(tap(uv + float2(px.x, -px.y)));
    float lSW = luma(tap(uv + float2(-px.x, px.y)));
    float lSE = luma(tap(uv + float2(px.x, px.y)));

    // Sub-pixel aliasing amount.
    float lAvg = (2.0 * (lN + lS + lE + lW) + lNW + lNE + lSW + lSE) / 12.0;
    float sub = saturate(abs(lAvg - lM) / range);
    sub = smoothstep(0.0, 1.0, sub);
    sub = sub * sub * 0.75;

    // Edge orientation.
    float edgeH = abs(lNW + lNE - 2.0 * lN) + 2.0 * abs(lW + lE - 2.0 * lM) + abs(lSW + lSE - 2.0 * lS);
    float edgeV = abs(lNW + lSW - 2.0 * lW) + 2.0 * abs(lN + lS - 2.0 * lM) + abs(lNE + lSE - 2.0 * lE);
    bool horz = edgeH >= edgeV;

    float l1 = horz ? lN : lW;
    float l2 = horz ? lS : lE;
    float g1 = abs(l1 - lM), g2 = abs(l2 - lM);
    float stepLen = horz ? px.y : px.x;
    float lLocal;
    float grad;
    if (g1 >= g2) { stepLen = -stepLen; lLocal = 0.5 * (l1 + lM); grad = g1; }
    else          { lLocal = 0.5 * (l2 + lM); grad = g2; }

    float2 uvE = uv;
    if (horz) uvE.y += stepLen * 0.5; else uvE.x += stepLen * 0.5;
    float2 dir = horz ? float2(px.x, 0) : float2(0, px.y);
    float scaled = grad * 0.25;

    // Walk along the edge both ways until its end.
    static const float STEPS[8] = { 1.0, 1.0, 1.0, 1.5, 2.0, 2.0, 4.0, 8.0 };
    float2 uv1 = uvE - dir, uv2 = uvE + dir;
    float e1 = luma(tap(uv1)) - lLocal, e2 = luma(tap(uv2)) - lLocal;
    bool d1 = abs(e1) >= scaled, d2 = abs(e2) >= scaled;
    [unroll] for (int i = 0; i < 8; i++)
    {
        if (d1 && d2) break;
        if (!d1) { uv1 -= dir * STEPS[i]; e1 = luma(tap(uv1)) - lLocal; d1 = abs(e1) >= scaled; }
        if (!d2) { uv2 += dir * STEPS[i]; e2 = luma(tap(uv2)) - lLocal; d2 = abs(e2) >= scaled; }
    }
    float dist1 = horz ? (uv.x - uv1.x) : (uv.y - uv1.y);
    float dist2 = horz ? (uv2.x - uv.x) : (uv2.y - uv.y);
    bool near1 = dist1 < dist2;
    float dmin2 = min(dist1, dist2);
    float edgeLen = dist1 + dist2;
    float offset = -dmin2 / edgeLen + 0.5;
    bool mLess = (lM - lLocal) < 0.0;
    bool correct = ((near1 ? e1 : e2) < 0.0) != mLess;
    float edgeOff = correct ? offset : 0.0;
    float finalOff = max(edgeOff, sub);

    float2 uvF = uv;
    if (horz) uvF.y += finalOff * stepLen; else uvF.x += finalOff * stepLen;
    return float4(tap(uvF), 1.0);
}

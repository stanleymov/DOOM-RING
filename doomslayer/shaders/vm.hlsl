// DOOM RING viewmodel: skinned Doom Eternal arms + weapon, drawn over the frame like Doom does.
// Compile: dxc -T vs_6_0 -E VSMain vm.hlsl -Fo vm_vs.cso ; dxc -T ps_6_0 -E PSMain vm.hlsl -Fo vm_ps.cso

cbuffer Frame : register(b0)
{
    row_major float4x4 proj;
    float4 light_dir;   // view space, towards the light
    float4 light_col;   // rgb, a = specular strength
    float4 fill_col;    // rgb fill from below/behind
    float4 ambient;     // rgb ambient, a = emissive strength
    float4 params;      // x = flash (muzzle light), y = gamma, z = has_normal, w = has_spec
    float4 flash_pos;   // view-space muzzle flash position
};

cbuffer Bones : register(b1)
{
    row_major float3x4 bones[96];
};

Texture2D tex_albedo : register(t0);
Texture2D tex_normal : register(t1);
Texture2D tex_spec   : register(t2);
Texture2D tex_emis   : register(t3);
Texture2D tex_gloss  : register(t4);
Texture2D scene      : register(t5);   // Elden Ring's finished frame (copied before we draw)
Texture2D probe_tex  : register(t6);   // smoothed scene light: (0,0) upper, (1,0) lower (probe.hlsl)
SamplerState samp    : register(s0);
SamplerState samp_clamp : register(s1);

struct VSIn
{
    float3 pos : POSITION;
    float3 nrm : NORMAL;
    float2 uv  : TEXCOORD0;
    uint4  bi  : BLENDINDICES;
    float4 bw  : BLENDWEIGHT;
};

struct PSIn
{
    float4 sv  : SV_Position;
    float3 vp  : TEXCOORD1;
    float3 nrm : TEXCOORD2;
    float2 uv  : TEXCOORD0;
    float  gun : TEXCOORD3;   // 1 for the weapon (palette slot 95 carries a marker), 0 for the arms
};

PSIn VSMain(VSIn v)
{
    float3x4 m = bones[v.bi.x] * v.bw.x + bones[v.bi.y] * v.bw.y
               + bones[v.bi.z] * v.bw.z + bones[v.bi.w] * v.bw.w;
    float3 p = mul(m, float4(v.pos, 1.0));
    float3 n = mul((float3x3)m, v.nrm);
    PSIn o;
    o.vp = p;
    o.nrm = normalize(n);
    o.uv = v.uv;
    o.gun = bones[95][0][0] > 7000.0 ? 1.0 : 0.0;
    o.sv = mul(proj, float4(p, 1.0));
    return o;
}

// Normal mapping without tangents (cotangent frame from derivatives, C. Schueler).
float3 perturb(float3 n, float3 p, float2 uv, float3 tn)
{
    float3 dp1 = ddx(p), dp2 = ddy(p);
    float2 du1 = ddx(uv), du2 = ddy(uv);
    float3 dp2perp = cross(dp2, n);
    float3 dp1perp = cross(n, dp1);
    float3 t = dp2perp * du1.x + dp1perp * du2.x;
    float3 b = dp2perp * du1.y + dp1perp * du2.y;
    float inv = rsqrt(max(dot(t, t), dot(b, b)) + 1e-12);
    return normalize(mul(tn, float3x3(t * inv, b * inv, n)));
}

float3 lin(float3 c) { return pow(saturate(c), 2.2); }

// Scene lighting probe: average of the frame's upper (sky / walls) and lower (ground) halves.
void scene_probe(out float3 upper, out float3 lower)
{
    // Smoothed once per frame by probe.hlsl (a 16x8 grid, eased over ~0.25 s): reading 12 raw
    // pixels here made the gun's light jump as they hit different grass / rocks every frame.
    upper = probe_tex.Load(int3(0, 0, 0)).rgb;
    lower = probe_tex.Load(int3(1, 0, 0)).rgb;
}

// (the old per-pixel probe, kept for reference)
void scene_probe_raw(out float3 upper, out float3 lower)
{
    upper = 0; lower = 0;
    [unroll] for (int y = 0; y < 3; y++)
    [unroll] for (int x = 0; x < 4; x++)
    {
        float2 uv = float2((x + 0.5) / 4.0, (y + 0.5) / 6.0);
        upper += lin(scene.SampleLevel(samp_clamp, uv, 0).rgb);
        lower += lin(scene.SampleLevel(samp_clamp, float2(uv.x, 0.5 + uv.y * 0.6), 0).rgb);
    }
    upper /= 12.0; lower /= 12.0;
}

// Screen-space reflection of the real world behind the gun (blurred by roughness).
float3 scene_reflect(float3 r, float rough)
{
    float2 uv = float2(0.5 + r.x * 0.45, 0.5 - r.y * 0.45);
    float b = 0.01 + rough * 0.08;
    float3 c = lin(scene.SampleLevel(samp_clamp, uv, 0).rgb) * 0.4;
    c += lin(scene.SampleLevel(samp_clamp, uv + float2(b, 0), 0).rgb) * 0.15;
    c += lin(scene.SampleLevel(samp_clamp, uv - float2(b, 0), 0).rgb) * 0.15;
    c += lin(scene.SampleLevel(samp_clamp, uv + float2(0, b), 0).rgb) * 0.15;
    c += lin(scene.SampleLevel(samp_clamp, uv - float2(0, b), 0).rgb) * 0.15;
    return c;
}

// Albedo of the glass marker texture (viewmodel.rs fallbacks): Doom's glass ships no albedo map.
static const float3 GLASS_KEY = float3(12.0, 34.0, 56.0) / 255.0;

float4 PSMain(PSIn i, bool front : SV_IsFrontFace) : SV_Target
{
    float3 alb0 = tex_albedo.Sample(samp, i.uv).rgb;
    bool glass = all(abs(alb0 - GLASS_KEY) < 0.004);
    // Glass (the Heat Blast tubes): painted the casing's tan so the whole section reads as one
    // part, and it heats up with it below.
    float3 base = glass ? pow(float3(101.0, 85.0, 63.0) / 255.0, 2.2) : pow(saturate(alb0), 2.2);
    float3 f0 = pow(saturate(tex_spec.Sample(samp, i.uv).rgb), 2.2);
    // Pinpoint dark specks in Doom's maps (painted screw holes) turned into hard black dots under
    // our sharp filtering: never let a texel go darker than half its blurred surroundings.
    float3 f0_soft = pow(saturate(tex_spec.SampleBias(samp, i.uv, 3.0).rgb), 2.2);
    f0 = max(f0, f0_soft * 0.5);
    if (!glass) base = max(base, pow(saturate(tex_albedo.SampleBias(samp, i.uv, 3.0).rgb), 2.2) * 0.5);
    float gloss = tex_gloss.Sample(samp, i.uv).r;
    float rough = clamp(1.0 - gloss, 0.06, 1.0);
    float a = rough * rough;

    float3 n = normalize(i.nrm);
    float3 v = normalize(-i.vp);
    // Two-sided lighting: inner faces seen through the hairline gaps between panels were lit with
    // normals pointing away and came out black (the "black lines"); light them like front faces.
    if (dot(n, v) < -0.05) n = -n;
    float3 n_geo = n;
    if (params.z > 0.5)
    {
        float2 xy = tex_normal.Sample(samp, i.uv).rg * 2.0 - 1.0;
        float3 tn = float3(xy, sqrt(saturate(1.0 - dot(xy, xy))));
        n = perturb(n, i.vp, i.uv, tn);
    }
    // Degenerate UV derivatives (tiny / stretched triangles) gave NaN normals: black dots.
    if (any(isnan(n)) || dot(n, n) < 0.5) n = n_geo;

    // Light from the actual scene: the sun/key is scaled by how bright the world is right now,
    // ambient is a sky/ground blend of the frame, reflections are the frame itself.
    float3 upper, lower;
    scene_probe(upper, lower);
    float scene_lum = dot(upper * 0.6 + lower * 0.4, float3(0.2126, 0.7152, 0.0722));
    float key_scale = saturate(scene_lum * 4.0) * 0.85 + 0.15;
    float3 key_col = light_col.rgb * key_scale * lerp(float3(1, 1, 1), normalize(upper + 1e-3) * 1.7, 0.35);

    float3 l = normalize(light_dir.xyz);
    float3 h = normalize(l + v);
    float ndl = saturate(dot(n, l));
    float ndv = saturate(dot(n, v)) + 1e-4;
    float ndh = saturate(dot(n, h));
    float vdh = saturate(dot(v, h));

    float a2 = a * a;
    float d = a2 / (3.14159 * pow(ndh * ndh * (a2 - 1.0) + 1.0, 2.0));
    float k = a * 0.5;
    float vis = 1.0 / ((ndl * (1.0 - k) + k) * (ndv * (1.0 - k) + k));
    float3 F = f0 + (1.0 - f0) * pow(1.0 - vdh, 5.0);
    float3 spec_key = d * vis * 0.25 * F * ndl * key_col;

    float3 amb = lerp(lower, upper, saturate(n.y * 0.5 + 0.5)) * ambient.rgb * 3.0;
    float3 diffuse = base * (amb + key_col * ndl) * (1.0 - f0);

    float3 r = reflect(-v, n);
    // Doom lights viewmodels with neutral probes: metal reads as silver/steel, not as a mirror of
    // the world. Keep a hint of the real scene (desaturated, blurred) over a neutral studio gradient.
    float rough_env = max(rough, 0.35);
    float3 Fe = f0 + (max(1.0 - rough_env, f0) - f0) * pow(1.0 - ndv, 5.0);
    // Reflections from the smoothed sky/ground probe (like Doom's light probes). The screen-space
    // taps (scene_reflect) flickered on grass and leaves as the camera moved.
    float3 world = lerp(lower, upper, saturate(r.y * 0.5 + 0.5));
    world = lerp(world, dot(world, float3(0.2126, 0.7152, 0.0722)).xxx, 0.75);
    float3 studio = lerp(0.06, 0.32, saturate(r.y * 0.5 + 0.5)).xxx * key_scale;
    float3 envc = (world * 0.55 + studio * 0.45) * light_col.a * 1.8;
    float3 col = diffuse + spec_key + envc * Fe;

    if (params.x > 0.0)
    {
        float3 fl = flash_pos.xyz - i.vp;
        float dd = length(fl);
        col += (base + f0) * float3(1.0, 0.55, 0.2) * params.x * saturate(dot(n, fl / dd)) / (1.0 + dd * dd * 8.0);
    }
    // Emissive. flash_pos.w >= 0: Plasma Rifle heat - its glow is plasma blue and heats up to
    // orange, then white-hot (gun only; the suit's lights keep their colour).
    float3 emis = pow(saturate(tex_emis.Sample(samp, i.uv).rgb), 2.2);
    if (flash_pos.w >= 0.0 && i.gun > 0.5)
    {
        float heat = saturate(flash_pos.w);
        float glow = dot(emis, float3(0.333, 0.333, 0.333));
        float3 cold = float3(0.25, 0.6, 1.6) * 4.5;
        float3 warm = float3(3.2, 0.9, 0.15) * 5.5;
        float3 white = float3(4.0, 3.0, 2.0) * 6.5;
        float3 tint = heat < 0.7 ? lerp(cold, warm, heat / 0.7) : lerp(warm, white, (heat - 0.7) / 0.3);
        emis = glow * tint;
        // The yellow/beige casing heats up too: picked out by its warm albedo, it starts to
        // glow amber as soon as the rifle fires and goes orange, then white-hot with the heat.
        float3 alb = saturate(tex_albedo.Sample(samp, i.uv).rgb);
        float beige = glass ? 1.0 : smoothstep(0.06, 0.16, alb.r - alb.b) * smoothstep(0.03, 0.09, alb.g - alb.b) * smoothstep(0.12, 0.25, alb.r);
        float on = smoothstep(0.0, 0.08, heat);
        float3 amber = float3(1.6, 0.95, 0.3);
        float3 hotc = heat < 0.7 ? lerp(amber, float3(2.2, 0.6, 0.08), heat / 0.7) : lerp(float3(2.2, 0.6, 0.08), float3(2.4, 1.6, 0.8), (heat - 0.7) / 0.3);
        emis += beige * on * (0.25 + 0.6 * heat) * hotc * (0.3 + base * 2.5);
    }
    // ambient.a above 2 is the Ballista arbalest glow: only the gun heats up, not the suit lights.
    col += emis * (i.gun > 0.5 ? ambient.a : min(ambient.a, 2.0));
    if (any(isnan(col))) col = base * amb;
    col = col / (1.0 + col * 0.15);
    return float4(pow(saturate(col), 1.0 / params.y), 1.0);
}

// The Crucible's energy blade (viewmodel.rs: meshes of the crucible_blade* materials, drawn after
// the solid parts with additive blending, no depth write). Doom's blade masks drive it: the
// albedo slot holds the overall mask (outline bright, inside dimmer), the emissive slot Doom's red
// flow texture that breaks the colour up. Red-orange, hotter where the mask is brightest.
// Compile: dxc -T ps_6_0 -E GlowMain vm.hlsl -Fo vm_glow_ps.cso
float4 GlowMain(PSIn i) : SV_Target
{
    float m = tex_albedo.Sample(samp, i.uv).r;
    float3 flow = tex_emis.Sample(samp, i.uv).rgb;
    float f = saturate(max(flow.r, dot(flow, float3(0.333, 0.333, 0.333))));
    // red, not yellow (user): green stays tiny, the gamma step below lifts it a lot
    float3 red = float3(1.0, 0.012, 0.004);
    float3 hot = float3(1.0, 0.06, 0.03);
    float3 col = red * m * (0.55 + 0.9 * f) * 2.2 + hot * pow(m, 4.0) * 1.4;
    // flash_pos.w < 0 on other guns; the Crucible passes its brightness in ambient.a's range
    col = col / (1.0 + col * 0.15);
    return float4(pow(saturate(col), 1.0 / params.y), 1.0);
}

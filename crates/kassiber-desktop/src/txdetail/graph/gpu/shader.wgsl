// The transaction graph's glass scene: three.js MeshPhysicalMaterial (glass, satin) and
// MeshBasicMaterial (highlight sleeve) approximated for an orthographic, statically lit scene
// (kb/glass3d/stage.ts: RoomEnvironment PMREM, hemisphere + key light, Neutral tone mapping).

struct Mat {
    color: vec4<f32>,        // linear base colour, w = roughness
    attenuation: vec4<f32>,  // linear attenuation colour, w = attenuation distance
    emissive: vec4<f32>,     // linear emissive × intensity, w = transmission
    sheen: vec4<f32>,        // linear sheen colour, w = sheen roughness
    params: vec4<f32>,       // metalness, clearcoat roughness, thickness, kind (0 glass, 1 satin, 2 basic)
};

struct Frame {
    rot: mat4x4<f32>,        // model → view rotation (pitch about X, then yaw about Y)
    half: vec4<f32>,         // frame half width, half height (scene units), target px w, h
    bg: vec4<f32>,           // linear background (the surface colour)
    sky: vec4<f32>,          // hemisphere sky colour × intensity, w = highlighted part id
    ground: vec4<f32>,       // hemisphere ground colour × intensity, w = environment intensity
    key: vec4<f32>,          // key light direction (view space), w = intensity
    env: vec4<f32>,          // wall low, wall high, light gain, lobe width
    env2: vec4<f32>,         // refraction strength, blur mix, sheen gain, reflection light gain
    mats: array<Mat, 12>,
};

@group(0) @binding(0) var<uniform> f: Frame;
@group(0) @binding(1) var behind: texture_2d<f32>;
@group(0) @binding(2) var behind_blur: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) mat: u32,
    @location(3) part: u32,
};

struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) n: vec3<f32>,
    @location(1) @interpolate(flat) mat: u32,
    @location(2) @interpolate(flat) part: u32,
};

@vertex
fn vs(v: VIn) -> VOut {
    let p = (f.rot * vec4<f32>(v.pos, 1.0)).xyz;
    var o: VOut;
    // Orthographic: x, y over the frame; nearer (larger z) is smaller depth.
    o.clip = vec4<f32>(p.x / f.half.x, p.y / f.half.y, 0.5 - p.z / 40.0, 1.0);
    o.n = (f.rot * vec4<f32>(v.normal, 0.0)).xyz;
    o.mat = v.mat;
    o.part = v.part;
    return o;
}

const PI: f32 = 3.14159265;

// ---- RoomEnvironment stand-in: a grey room brighter toward the ceiling, plus the room's soft
// area lights as Gaussian lobes (positions from RoomEnvironment.js, x mirrored as three's cube
// lookup mirrors it). Lobes widen with roughness, keeping their energy.
const LIGHTS = array<vec4<f32>, 6>(
    vec4<f32>(0.0, 1.0, 0.0, 100.0),
    vec4<f32>(0.697, 0.622, 0.355, 50.0),
    vec4<f32>(0.632, 0.707, -0.322, 50.0),
    vec4<f32>(-0.770, 0.631, -0.095, 17.0),
    vec4<f32>(0.027, 0.523, 0.852, 43.0),
    vec4<f32>(-0.187, 0.664, -0.725, 20.0),
);

fn env_radiance(d: vec3<f32>, rough: f32) -> vec3<f32> {
    let wall = mix(f.env.x, f.env.y, clamp(d.y * 0.5 + 0.5, 0.0, 1.0));
    var light = 0.0;
    let s0 = f.env.w;
    let s = s0 + rough * rough * 1.6;
    for (var i = 0u; i < 6u; i++) {
        let l = LIGHTS[i];
        let c = dot(d, l.xyz);
        light += l.w * exp(-2.0 * (1.0 - c) / (s * s)) * (s0 * s0) / (s * s);
    }
    // Reflections use their own light gain: the room's real lights are small, so mirrored
    // directions rarely see them, while the wide lobes here stand in for their irradiance.
    return vec3<f32>(wall + light * f.env2.w * 0.01) * f.ground.w;
}

// Irradiance (π included, as three's getIBLIrradiance).
fn env_irradiance(n: vec3<f32>) -> vec3<f32> {
    let wall = mix(f.env.x, f.env.y, clamp(n.y * 0.25 + 0.5, 0.0, 1.0));
    var light = 0.0;
    let s0 = f.env.w;
    for (var i = 0u; i < 6u; i++) {
        let l = LIGHTS[i];
        light += l.w * max(dot(n, l.xyz), 0.0) * PI * s0 * s0 * 0.5;
    }
    return vec3<f32>(PI * wall + light * f.env.z * 0.01) * f.ground.w;
}

// Karis' analytic split-sum environment BRDF.
fn env_brdf(f0: vec3<f32>, rough: f32, nv: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = rough * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * nv)) * r.x + r.y;
    let ab = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return f0 * ab.x + ab.y;
}

fn ggx(nh: f32, nv: f32, nl: f32, rough: f32) -> f32 {
    let a = max(rough * rough, 0.002);
    let a2 = a * a;
    let d = nh * nh * (a2 - 1.0) + 1.0;
    let dist = a2 / (PI * d * d);
    let k = a * 0.5;
    let vis = 0.25 / ((nv * (1.0 - k) + k) * (nl * (1.0 - k) + k));
    return dist * vis;
}

fn sheen_dfg(nv: f32, r: f32) -> f32 {
    let r2 = r * r;
    var a = -8.48 * r2 + 14.3 * r - 9.95;
    var b = 1.97 * r2 - 3.27 * r + 0.72;
    if (r < 0.25) {
        a = -339.2 * r2 + 161.4 * r - 25.9;
        b = 44.0 * r2 - 23.7 * r + 3.26;
    }
    let dg = exp(a * nv + b) + select(0.1 * (r - 0.25), 0.0, r < 0.25);
    return clamp(dg / PI, 0.0, 1.0);
}

// Khronos PBR Neutral (three's NeutralToneMapping).
fn neutral(c_in: vec3<f32>) -> vec3<f32> {
    var c = c_in;
    let start = 0.8 - 0.04;
    let desat = 0.15;
    let x = min(c.r, min(c.g, c.b));
    let offset = select(0.04, x - 6.25 * x * x, x < 0.08);
    c -= offset;
    let peak = max(c.r, max(c.g, c.b));
    if (peak < start) { return c; }
    let d = 1.0 - start;
    let new_peak = 1.0 - d * d / (peak + d - start);
    c *= new_peak / peak;
    let g = 1.0 - 1.0 / (desat * (peak - new_peak) + 1.0);
    return mix(c, vec3<f32>(new_peak), g);
}

fn shade(mi: u32, n_in: vec3<f32>, frag: vec2<f32>, front: bool) -> vec3<f32> {
    let m = f.mats[mi];
    var n = normalize(n_in);
    let v = vec3<f32>(0.0, 0.0, 1.0);
    // Orthographic and depth-tested: a face we see points at us (winding varies by mesh).
    if (dot(n, v) < 0.0 && !front) { n = -n; }
    if (dot(n, v) < 0.0) { n = -n; }
    let nv = max(dot(n, v), 1e-3);
    let rough = clamp(m.color.w, 0.02, 1.0);
    let metal = m.params.x;
    let transmission = m.emissive.w;
    let base = m.color.rgb;
    let diffuse_color = base * (1.0 - metal);
    let f0 = mix(vec3<f32>(0.04), base, metal);
    let r = reflect(-v, n);

    // Key light.
    let l = normalize(f.key.xyz);
    let nl = max(dot(n, l), 0.0);
    let h = normalize(l + v);
    let nh = max(dot(n, h), 0.0);
    let e_key = nl * f.key.w;
    let fr_key = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);

    // Diffuse: environment (π included), hemisphere and key light, Lambert (c / π).
    let hemi = mix(f.ground.rgb, f.sky.rgb, n.y * 0.5 + 0.5);
    let e_env = env_irradiance(n);
    var diffuse = diffuse_color * (e_env + hemi + vec3<f32>(e_key)) / PI;
    let spec_env = env_radiance(r, rough) * env_brdf(f0, rough, nv);
    let spec_key = fr_key * ggx(nh, nv, nl, rough) * e_key;

    if (m.params.w < 0.5) {
        // Glass: refracted light from behind, attenuated through the thickness, tinted by the
        // base colour, replaces the diffuse share (transmission_fragment).
        let size = f.half.zw;
        let offset = -n.xy * vec2<f32>(1.0, -1.0) * m.params.z * f.env2.x * size.y * 0.05;
        let uv = clamp((frag + offset) / size, vec2<f32>(0.0), vec2<f32>(1.0));
        let sharp = textureSampleLevel(behind, samp, uv, 0.0).rgb;
        let soft = textureSampleLevel(behind_blur, samp, uv, 0.0).rgb;
        let seen = mix(sharp, soft, clamp(rough * f.env2.y, 0.0, 1.0));
        let t = pow(max(m.attenuation.rgb, vec3<f32>(1e-4)), vec3<f32>(m.params.z / max(m.attenuation.w, 1e-3)));
        let refracted = (1.0 - env_brdf(f0, rough, nv)) * t * seen * base;
        diffuse = mix(diffuse, refracted, transmission);
    }

    var color = diffuse + spec_env + spec_key;
    // Clearcoat lobe over everything (both glass and satin have clearcoat 1).
    let cc_rough = m.params.y;
    let fc = 0.04 + 0.96 * pow(1.0 - nv, 5.0);
    let cc = env_radiance(r, cc_rough) * env_brdf(vec3<f32>(0.04), cc_rough, nv) + vec3<f32>(0.04 + 0.96 * pow(1.0 - max(dot(h, v), 0.0), 5.0)) * ggx(nh, nv, nl, cc_rough) * e_key;
    color = color * (1.0 - fc) + cc;
    // Sheen rim (glass only: sheen colour is zero on satin).
    color += m.sheen.rgb * e_env * sheen_dfg(nv, m.sheen.w) * f.env2.z;
    color += m.emissive.rgb;
    return color;
}

// Opaque pass (satin): tone-mapped, as shown; its resolve is also what the glass refracts.
@fragment
fn fs_opaque(o: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var mi = o.mat;
    if (o.part != 0u && f32(o.part) == f.sky.w && (mi == 6u || mi == 7u)) {
        mi = mi + 2u;
    }
    return vec4<f32>(neutral(shade(mi, o.n, o.clip.xy, front)), 1.0);
}

@fragment
fn fs_glass(o: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let c = shade(o.mat, o.n, o.clip.xy, front);
    return vec4<f32>(neutral(c), 1.0);
}

// Highlight sleeve: MeshBasicMaterial, opacity 0.42, only on the lit leg.
@fragment
fn fs_sleeve(o: VOut) -> @location(0) vec4<f32> {
    if (f32(o.part) != f.sky.w) { discard; }
    let c = f.mats[o.mat].color.rgb;
    return vec4<f32>(c * 0.42, 0.42);
}

// ---- Fullscreen helpers: separable blur of what lies behind the glass.

struct Blur { dir: vec4<f32> };
@group(0) @binding(0) var blur_src: texture_2d<f32>;
@group(0) @binding(1) var blur_samp: sampler;
@group(0) @binding(2) var<uniform> blur: Blur;

struct FsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_full(@builtin(vertex_index) vi: u32) -> FsOut {
    let p = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u));
    var o: FsOut;
    o.pos = vec4<f32>(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = vec2<f32>(p.x, 1.0 - p.y);
    return o;
}

@fragment
fn fs_blur(i: FsOut) -> @location(0) vec4<f32> {
    let w = array<f32, 5>(0.227027, 0.1945946, 0.1216216, 0.054054, 0.016216);
    var c = textureSampleLevel(blur_src, blur_samp, i.uv, 0.0) * w[0];
    for (var k = 1; k < 5; k++) {
        let o = blur.dir.xy * f32(k);
        c += textureSampleLevel(blur_src, blur_samp, i.uv + o, 0.0) * w[k];
        c += textureSampleLevel(blur_src, blur_samp, i.uv - o, 0.0) * w[k];
    }
    return c;
}

// Draws a rendered graph into Rune's pass at its effect slot, cut by the slot's clip.

struct Blit {
    rect: vec4<f32>,   // slot rect in window device pixels: x, y, w, h
    clip: vec4<f32>,   // slot clip rect x0, y0, x1, y1 (window device pixels)
    fade: vec4<u32>,   // slot fade words (CLIP_WGSL)
    misc: vec4<f32>,   // origin x, origin y, srgb-encode flag, opacity
    dest: vec4<f32>, // pass target size w, h
};

@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> b: Blit;

struct O { @builtin(position) pos: vec4<f32> };

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> O {
    let corner = vec2<f32>(f32(vi & 1u), f32((vi >> 1u) & 1u));
    let px = b.rect.xy - b.misc.xy + corner * b.rect.zw;
    let ndc = vec2<f32>(px.x / b.dest.x * 2.0 - 1.0, 1.0 - px.y / b.dest.y * 2.0);
    var o: O;
    o.pos = vec4<f32>(ndc, 0.0, 1.0);
    return o;
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs(o: O) -> @location(0) vec4<f32> {
    let window = o.pos.xy + b.misc.xy;
    let uv = (window - b.rect.xy) / b.rect.zw;
    let c = textureSampleLevel(src, src_samp, uv, 0.0);
    var rgb = clamp(c.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    if (b.misc.z > 0.5) { rgb = to_srgb(rgb); }
    let a = clip_alpha(window, b.clip, b.fade) * b.misc.w * c.a;
    return vec4<f32>(rgb * a, a);
}

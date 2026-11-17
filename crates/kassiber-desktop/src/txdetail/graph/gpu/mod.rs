//! The glass scene on the GPU: the web's three.js view (graph3d/glassScene.ts, kb/glass3d/*)
//! rendered with real triangles, depth, 4× MSAA and a physically based glass and satin shader,
//! through Rune's compositor hook.
//!
//! How it plugs in: the graph view puts an effect slot (`El::effect(mark)`) where the drawing
//! goes and registers a `Job` (scene, pose, highlight, theme) in `GraphUi::gpu`. The app is
//! built with `compositor()`, which Rune hands each frame: it renders every registered job into
//! its own offscreen targets (only when its pose, size, highlight or theme changed), then draws
//! the UI with `Renderer::draw_with`, blitting each job's image at its slot in painter order,
//! cut by the slot's clip (rounded card corners) like any other element.
//!
//! Passes per job: (1) satin meshes (coins, edge lines) into a shown target (tone-mapped) and a
//! linear target the glass refracts; (2) a half-resolution blur of the linear target for rough
//! glass; (3) glass meshes (ribbons, collar) sampling what lies behind them, then the highlight
//! sleeve blended on top. If no compositor was installed, the view falls back to the 2.5D
//! drawing (the web falls back to its 2D graph without WebGL).

pub mod mesh;

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};

use bytemuck::{Pod, Zeroable};
use rune::Color;
use rune::compose::{Compositor, DeviceKey, Frame};
use rune::render::EffectSite;

use super::glass::{Rot, Scene};
use crate::app::App;
use mesh::SceneMeshes;

/// Effect-slot marks: one per graph view (compact, expanded).
pub const MARK_BASE: u32 = 0x4b47_0000;

static INSTALLED: AtomicBool = AtomicBool::new(false);
static FAILED: AtomicBool = AtomicBool::new(false);

/// Whether the GPU view draws (a compositor is installed and its pipelines built).
pub fn available() -> bool {
    INSTALLED.load(Ordering::Relaxed) && !FAILED.load(Ordering::Relaxed)
}

/// What one view asks the compositor to draw.
#[derive(Clone)]
pub struct Job {
    /// The slot element's key (its drawn rect sizes the render).
    pub key: String,
    pub meshes: Rc<SceneMeshes>,
    pub yaw: f32,
    pub pitch: f32,
    /// Aspect-adjusted frame half extents (scene units), as the picking camera uses.
    pub half: (f64, f64),
    pub highlight: Option<String>,
    pub dark: bool,
    /// The surface colour behind the glass (opaque, as the web's scene background).
    pub bg: Color,
    /// Keys of the scroll viewports the slot sits in. Rune hands an effect slot only the clip of
    /// its own `clip_rounded` box (an ancestor scroll viewport's rectangle is not intersected),
    /// so the blit cuts itself to these rectangles, and a slot scrolled out of them is not drawn.
    pub clip_keys: &'static [&'static str],
}

/// The compositor to build the app with (`rune::App::compositor`).
pub fn compositor() -> GlassCompositor {
    // `KB_GLASS_OFF=1` draws the fallback, to compare or debug it.
    INSTALLED.store(
        std::env::var_os("KB_GLASS_OFF").is_none(),
        Ordering::Relaxed,
    );
    GlassCompositor::default()
}

thread_local! {
    static MESH_CACHE: std::cell::RefCell<Option<(usize, Rc<SceneMeshes>)>> = const { std::cell::RefCell::new(None) };
}

/// The meshes for a laid-out scene, kept while the same scene is drawn.
pub fn meshes_for(scene: &Rc<Scene>) -> Rc<SceneMeshes> {
    let id = Rc::as_ptr(scene) as usize;
    MESH_CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((i, m)) = c.as_ref()
            && *i == id
        {
            return m.clone();
        }
        let m = Rc::new(mesh::build(scene));
        *c = Some((id, m.clone()));
        m
    })
}

// ---------------------------------------------------------------------------------------------
// Materials (kb/glass3d/materials.ts), as the shader's table.

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Default)]
struct Mat {
    color: [f32; 4],
    attenuation: [f32; 4],
    emissive: [f32; 4],
    sheen: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct FrameUniform {
    rot: [[f32; 4]; 4],
    half: [f32; 4],
    bg: [f32; 4],
    sky: [f32; 4],
    ground: [f32; 4],
    key: [f32; 4],
    env: [f32; 4],
    env2: [f32; 4],
    mats: [Mat; 12],
}

/// A CSS hex colour in linear light (`rune::Color` already holds linear sRGB).
fn lin(hex: &str) -> [f32; 3] {
    let c = rune::color::hex(hex).expect("colour");
    [c.r, c.g, c.b]
}

fn color_lin(c: Color) -> [f32; 4] {
    [c.r, c.g, c.b, 1.0]
}

/// (colour, attenuation, roughness, distance, transmission, glow, glow intensity, rim)
type Tone = (
    &'static str,
    &'static str,
    f32,
    f32,
    f32,
    &'static str,
    f32,
    &'static str,
);

const LIGHT_TONES: [Tone; 5] = [
    (
        "#d6e6ff", "#2f7cf6", 0.04, 1.1, 0.92, "#000000", 0.0, "#ffffff",
    ),
    (
        "#f5f7fa", "#b8c2d0", 0.5, 3.0, 0.85, "#000000", 0.0, "#ffffff",
    ),
    (
        "#ffe9a8", "#f59e0b", 0.06, 0.8, 0.9, "#000000", 0.0, "#fff7d6",
    ),
    (
        "#fbf3dc", "#e7c77e", 0.5, 2.5, 0.85, "#000000", 0.0, "#fffaf0",
    ),
    (
        "#f3f7fc", "#8fb8f5", 0.02, 3.5, 1.0, "#000000", 0.0, "#ffffff",
    ),
];
const DARK_TONES: [Tone; 5] = [
    (
        "#9cc4ff", "#3b82f6", 0.05, 1.4, 0.55, "#1d4ed8", 0.45, "#cfe1ff",
    ),
    (
        "#cbd3de", "#94a3b8", 0.5, 3.0, 0.45, "#334155", 0.4, "#e2e8f0",
    ),
    (
        "#ffd466", "#f59e0b", 0.06, 1.0, 0.5, "#b45309", 0.6, "#fff1c2",
    ),
    (
        "#e9dcb8", "#c9a860", 0.5, 2.5, 0.45, "#57451f", 0.4, "#f6ecd2",
    ),
    (
        "#dbe5f3", "#60a5fa", 0.03, 4.0, 0.62, "#1e293b", 0.5, "#e0ecff",
    ),
];

fn glass_mat(t: Tone, thickness: f32) -> Mat {
    let (c, a, r, d, tr, g, gi, rim) = t;
    let (c, a, g, rim) = (lin(c), lin(a), lin(g), lin(rim));
    Mat {
        color: [c[0], c[1], c[2], r],
        attenuation: [a[0], a[1], a[2], d],
        emissive: [g[0] * gi, g[1] * gi, g[2] * gi, tr],
        sheen: [rim[0], rim[1], rim[2], 0.25],
        params: [0.0, 0.04, thickness, 0.0],
    }
}

fn satin_mat(hex: &str) -> Mat {
    let c = lin(hex);
    Mat {
        color: [c[0], c[1], c[2], 0.32],
        params: [0.15, 0.12, 0.0, 1.0],
        ..Default::default()
    }
}

fn basic_mat(hex: &str) -> Mat {
    let c = lin(hex);
    Mat {
        color: [c[0], c[1], c[2], 1.0],
        params: [0.0, 0.0, 0.0, 2.0],
        ..Default::default()
    }
}

fn materials(dark: bool) -> [Mat; 12] {
    let tones = if dark { DARK_TONES } else { LIGHT_TONES };
    let mut m = [Mat::default(); 12];
    for i in 0..4 {
        m[i] = glass_mat(tones[i], 0.6);
    }
    m[4] = glass_mat(tones[4], 1.4);
    m[5] = satin_mat(if dark { "#0b1220" } else { "#3f4a5c" });
    m[6] = satin_mat("#2563eb");
    m[7] = satin_mat(if dark { "#64748b" } else { "#7b8798" });
    m[8] = satin_mat(if dark { "#60a5fa" } else { "#3b82f6" });
    m[9] = satin_mat(if dark { "#94a3b8" } else { "#a3adbb" });
    m[10] = basic_mat(if dark { "#67e8f9" } else { "#06b6d4" });
    m[11] = basic_mat(if dark { "#fcd34d" } else { "#f59e0b" });
    m
}

/// Environment stand-in parameters: wall low, wall high, light gain, lobe width, refraction,
/// blur mix, sheen gain. `KB_GLASS_ENV` overrides them (comma separated) for calibration.
fn env_params() -> ([f32; 4], [f32; 4]) {
    // Fitted to the web capture (p3-graph/tools/calib.py): ribbon face, coin front and side.
    let mut v = [0.144f32, 0.216, 52.0, 0.30, 1.0, 2.0, 1.0, 6.0];
    if let Ok(s) = std::env::var("KB_GLASS_ENV") {
        for (i, x) in s.split(',').enumerate().take(8) {
            if let Ok(x) = x.trim().parse() {
                v[i] = x;
            }
        }
    }
    ([v[0], v[1], v[2], v[3]], [v[4], v[5], v[6], v[7]])
}

fn frame_uniform(job: &Job, size: (u32, u32)) -> FrameUniform {
    let r = Rot::new(job.yaw, job.pitch);
    let col = |v: [f64; 3]| {
        let q = r.apply(v);
        [q[0] as f32, q[1] as f32, q[2] as f32, 0.0]
    };
    let (env, env2) = env_params();
    let (sky_c, ground_c, hemi, env_i) = if job.dark {
        ([1.0f32, 1.0, 1.0], lin("#334155"), 0.7f32, 0.9f32)
    } else {
        ([1.0, 1.0, 1.0], lin("#8a94a6"), 0.9, 1.15)
    };
    let hl = job
        .highlight
        .as_deref()
        .map(|p| job.meshes.part_id(p))
        .unwrap_or(0);
    let kl = (16.0f32 + 49.0 + 64.0).sqrt();
    FrameUniform {
        rot: [
            col([1.0, 0.0, 0.0]),
            col([0.0, 1.0, 0.0]),
            col([0.0, 0.0, 1.0]),
            [0.0, 0.0, 0.0, 1.0],
        ],
        half: [
            job.half.0 as f32,
            job.half.1 as f32,
            size.0 as f32,
            size.1 as f32,
        ],
        bg: color_lin(job.bg),
        sky: [sky_c[0] * hemi, sky_c[1] * hemi, sky_c[2] * hemi, hl as f32],
        ground: [
            ground_c[0] * hemi,
            ground_c[1] * hemi,
            ground_c[2] * hemi,
            env_i,
        ],
        key: [-4.0 / kl, 7.0 / kl, 8.0 / kl, 2.2],
        env,
        env2,
        mats: materials(job.dark),
    }
}

// ---------------------------------------------------------------------------------------------
// GPU resources.

/// The scene's colour targets: 8-bit sRGB storage of linear values (a quarter of an HDR MSAA
/// target's memory; the shader writes tone-mapped colour, so nothing exceeds 1).
const COLOR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// Rune's isolated-layer target format (an effect slot inside a layer draws into it).
const LAYER: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const MSAA: u32 = 4;

struct Pipes {
    key: DeviceKey,
    scene_bgl: wgpu::BindGroupLayout,
    opaque: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    sleeve: wgpu::RenderPipeline,
    blur_bgl: wgpu::BindGroupLayout,
    blur: wgpu::RenderPipeline,
    blit_bgl: wgpu::BindGroupLayout,
    blit_module: wgpu::ShaderModule,
    blits: HashMap<wgpu::TextureFormat, wgpu::RenderPipeline>,
    sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,
}

fn shader_sources() -> (String, String) {
    let all = include_str!("shader.wgsl");
    let split = all
        .find("// ---- Fullscreen helpers")
        .expect("shader split marker");
    let scene = all[..split].to_owned();
    let blur = format!("{}{}", "", &all[split..]);
    (scene, blur)
}

impl Pipes {
    fn new(device: &wgpu::Device, key: DeviceKey) -> Pipes {
        let (scene_src, blur_src) = shader_sources();
        let scene_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("graph scene"),
            source: wgpu::ShaderSource::Wgsl(scene_src.into()),
        });
        let blur_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("graph blur"),
            source: wgpu::ShaderSource::Wgsl(blur_src.into()),
        });
        let blit_src = format!("{}\n{}", rune::render::CLIP_WGSL, include_str!("blit.wgsl"));
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("graph blit"),
            source: wgpu::ShaderSource::Wgsl(blit_src.into()),
        });

        let tex_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let ubo_entry = |binding, vis| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: vis,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let samp_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let scene_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("graph scene"),
            entries: &[
                ubo_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT),
                tex_entry(1),
                tex_entry(2),
                samp_entry(3),
            ],
        });
        let blur_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("graph blur"),
            entries: &[
                tex_entry(0),
                samp_entry(1),
                ubo_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let blit_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("graph blit"),
            entries: &[
                tex_entry(0),
                samp_entry(1),
                ubo_entry(2, wgpu::ShaderStages::VERTEX_FRAGMENT),
            ],
        });
        let scene_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("graph scene"),
            bind_group_layouts: &[Some(&scene_bgl)],
            immediate_size: 0,
        });
        let vattrs =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Uint32, 3 => Uint32];
        let vlayout = Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<mesh::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &vattrs,
        });
        let premul = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let make = |entry: &str,
                    targets: &[Option<wgpu::ColorTargetState>],
                    write_depth: bool,
                    compare: wgpu::CompareFunction| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&scene_layout),
                vertex: wgpu::VertexState {
                    module: &scene_mod,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vlayout),
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write_depth),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: MSAA,
                    ..Default::default()
                },
                fragment: Some(wgpu::FragmentState {
                    module: &scene_mod,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets,
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let hdr = |blend| {
            Some(wgpu::ColorTargetState {
                format: COLOR,
                blend,
                write_mask: wgpu::ColorWrites::ALL,
            })
        };
        let opaque = make("fs_opaque", &[hdr(None)], true, wgpu::CompareFunction::Less);
        let glass = make("fs_glass", &[hdr(None)], true, wgpu::CompareFunction::Less);
        let sleeve = make(
            "fs_sleeve",
            &[hdr(Some(premul))],
            false,
            wgpu::CompareFunction::LessEqual,
        );

        let blur_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("graph blur"),
            bind_group_layouts: &[Some(&blur_bgl)],
            immediate_size: 0,
        });
        let blur = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("graph blur"),
            layout: Some(&blur_layout),
            vertex: wgpu::VertexState {
                module: &blur_mod,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blur_mod,
                entry_point: Some("fs_blur"),
                compilation_options: Default::default(),
                targets: &[hdr(None)],
            }),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("graph"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("graph dummy"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: COLOR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());
        Pipes {
            key,
            scene_bgl,
            opaque,
            glass,
            sleeve,
            blur_bgl,
            blur,
            blit_bgl,
            blit_module,
            blits: HashMap::new(),
            sampler,
            dummy,
        }
    }

    fn blit(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
    ) -> &wgpu::RenderPipeline {
        let (bgl, module) = (&self.blit_bgl, &self.blit_module);
        self.blits.entry(format).or_insert_with(|| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("graph blit"),
                bind_group_layouts: &[Some(bgl)],
                immediate_size: 0,
            });
            let premul = wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                    operation: wgpu::BlendOperation::Add,
                },
            };
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("graph blit"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleStrip,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(premul),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        })
    }
}

struct Buffers {
    vertex: wgpu::Buffer,
    index: wgpu::Buffer,
    count: u32,
}

fn upload(device: &wgpu::Device, queue: &wgpu::Queue, m: &mesh::Mesh) -> Option<Buffers> {
    if m.indices.is_empty() {
        return None;
    }
    let vb = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("graph vertices"),
        size: (m.vertices.len() * std::mem::size_of::<mesh::Vertex>()) as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&vb, 0, bytemuck::cast_slice(&m.vertices));
    let ib = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("graph indices"),
        size: (m.indices.len() * 4) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    queue.write_buffer(&ib, 0, bytemuck::cast_slice(&m.indices));
    Some(Buffers {
        vertex: vb,
        index: ib,
        count: m.indices.len() as u32,
    })
}

struct Targets {
    size: (u32, u32),
    ms_shown: wgpu::TextureView,
    depth: wgpu::TextureView,
    shown: wgpu::TextureView,
    /// The satin pass resolved: what the glass refracts.
    linear: wgpu::TextureView,
    blur_a: wgpu::TextureView,
    blur_b: wgpu::TextureView,
    scene_bg_opaque: wgpu::BindGroup,
    scene_bg_glass: wgpu::BindGroup,
    blur_h: wgpu::BindGroup,
    blur_v: wgpu::BindGroup,
    blit_bg: wgpu::BindGroup,
}

fn texture(
    device: &wgpu::Device,
    label: &str,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    samples: u32,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: samples,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
        .create_view(&Default::default())
}

/// What a target holds: (pose, highlight, theme, background, size, meshes).
type Drawn = (u32, u32, u32, bool, [u32; 4], (u32, u32), usize, (u32, u32));

struct ViewGpu {
    meshes: usize,
    opaque: Option<Buffers>,
    glass: Option<Buffers>,
    sleeves: Option<Buffers>,
    ubo: wgpu::Buffer,
    blur_ubo_h: wgpu::Buffer,
    blur_ubo_v: wgpu::Buffer,
    blit_ubo: wgpu::Buffer,
    targets: Option<Targets>,
    /// What the shown target holds.
    drawn: Option<Drawn>,
    seen: bool,
}

impl ViewGpu {
    fn new(device: &wgpu::Device) -> ViewGpu {
        let ubo = |label, size| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        ViewGpu {
            meshes: 0,
            opaque: None,
            glass: None,
            sleeves: None,
            ubo: ubo("graph frame", std::mem::size_of::<FrameUniform>() as u64),
            blur_ubo_h: ubo("graph blur h", 16),
            blur_ubo_v: ubo("graph blur v", 16),
            blit_ubo: ubo("graph blit", 80),
            targets: None,
            drawn: None,
            seen: false,
        }
    }

    /// Makes the render targets for `size`; false when the device is out of memory.
    fn ensure_targets(&mut self, device: &wgpu::Device, p: &Pipes, size: (u32, u32)) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return true;
        }
        self.targets = None;
        let oom = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let ra = wgpu::TextureUsages::RENDER_ATTACHMENT;
        let rt = ra | wgpu::TextureUsages::TEXTURE_BINDING;
        let half = ((size.0 / 2).max(1), (size.1 / 2).max(1));
        let ms_shown = texture(device, "graph ms shown", size, COLOR, MSAA, ra);
        let depth = texture(device, "graph depth", size, DEPTH, MSAA, ra);
        let shown = texture(device, "graph shown", size, COLOR, 1, rt);
        let linear = texture(device, "graph behind", size, COLOR, 1, rt);
        let blur_a = texture(device, "graph blur a", half, COLOR, 1, rt);
        let blur_b = texture(device, "graph blur b", half, COLOR, 1, rt);
        if block_on(oom.pop()).is_some() {
            return false;
        }
        let scene_bg = |a: &wgpu::TextureView, b: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("graph scene"),
                layout: &p.scene_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.ubo.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(a),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(b),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&p.sampler),
                    },
                ],
            })
        };
        let blur_bg = |src: &wgpu::TextureView, ubo: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("graph blur"),
                layout: &p.blur_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(src),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&p.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: ubo.as_entire_binding(),
                    },
                ],
            })
        };
        let blit_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("graph blit"),
            layout: &p.blit_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&shown),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&p.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.blit_ubo.as_entire_binding(),
                },
            ],
        });
        let t = Targets {
            size,
            scene_bg_opaque: scene_bg(&p.dummy, &p.dummy),
            scene_bg_glass: scene_bg(&linear, &blur_b),
            blur_h: blur_bg(&linear, &self.blur_ubo_h),
            blur_v: blur_bg(&blur_a, &self.blur_ubo_v),
            blit_bg,
            ms_shown,
            depth,
            shown,
            linear,
            blur_a,
            blur_b,
        };
        self.targets = Some(t);
        self.drawn = None;
        true
    }

    fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        p: &Pipes,
        job: &Job,
        size: (u32, u32),
    ) {
        let mid = Rc::as_ptr(&job.meshes) as usize;
        if self.meshes != mid {
            self.opaque = upload(device, queue, &job.meshes.opaque);
            self.glass = upload(device, queue, &job.meshes.glass);
            self.sleeves = upload(device, queue, &job.meshes.sleeves);
            self.meshes = mid;
            self.drawn = None;
        }
        if !self.ensure_targets(device, p, size) {
            eprintln!("graph: out of GPU memory for the 3D view, drawing the fallback");
            FAILED.store(true, Ordering::Relaxed);
            return;
        }
        let hl = job
            .highlight
            .as_deref()
            .map(|x| job.meshes.part_id(x))
            .unwrap_or(0);
        let bg = [job.bg.r, job.bg.g, job.bg.b, job.bg.a].map(|v| (v * 1000.0) as u32);
        let half = ((job.half.0 * 1e4) as u32, (job.half.1 * 1e4) as u32);
        let key = (
            job.yaw.to_bits(),
            job.pitch.to_bits(),
            hl,
            job.dark,
            bg,
            size,
            mid,
            half,
        );
        if self.drawn == Some(key) {
            return;
        }
        let t = self.targets.as_ref().expect("targets");
        let u = frame_uniform(job, size);
        queue.write_buffer(&self.ubo, 0, bytemuck::bytes_of(&u));
        let hw = ((size.0 / 2).max(1) as f32, (size.1 / 2).max(1) as f32);
        queue.write_buffer(
            &self.blur_ubo_h,
            0,
            bytemuck::cast_slice(&[1.6 / hw.0, 0.0, 0.0, 0.0f32]),
        );
        queue.write_buffer(
            &self.blur_ubo_v,
            0,
            bytemuck::cast_slice(&[0.0, 1.6 / hw.1, 0.0, 0.0f32]),
        );
        let bg = wgpu::Color {
            r: u.bg[0] as f64,
            g: u.bg[1] as f64,
            b: u.bg[2] as f64,
            a: 1.0,
        };
        let draw = |pass: &mut wgpu::RenderPass<'_>, b: &Option<Buffers>| {
            if let Some(b) = b {
                pass.set_vertex_buffer(0, b.vertex.slice(..));
                pass.set_index_buffer(b.index.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..b.count, 0, 0..1);
            }
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("graph opaque"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.ms_shown,
                    depth_slice: None,
                    resolve_target: Some(&t.linear),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(bg),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.opaque);
            pass.set_bind_group(0, &t.scene_bg_opaque, &[]);
            draw(&mut pass, &self.opaque);
        }
        for (src_bg, dst) in [(&t.blur_h, &t.blur_a), (&t.blur_v, &t.blur_b)] {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("graph blur"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: dst,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(bg),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&p.blur);
            pass.set_bind_group(0, src_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("graph glass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &t.ms_shown,
                    depth_slice: None,
                    resolve_target: Some(&t.shown),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &t.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &t.scene_bg_glass, &[]);
            pass.set_pipeline(&p.glass);
            draw(&mut pass, &self.glass);
            if hl != 0 {
                pass.set_pipeline(&p.sleeve);
                draw(&mut pass, &self.sleeves);
            }
        }
        self.drawn = Some(key);
    }
}

// ---------------------------------------------------------------------------------------------
// The compositor.

#[derive(Default)]
pub struct GlassCompositor {
    pipes: Option<Pipes>,
    views: HashMap<u32, ViewGpu>,
    /// A job's rendered size lagged its slot this frame: draw another.
    pending: bool,
    drew: bool,
}

impl Compositor<App> for GlassCompositor {
    fn render(&mut self, f: &mut Frame<'_, App>) {
        let key = f.device_key();
        if !FAILED.load(Ordering::Relaxed) && self.pipes.as_ref().is_none_or(|p| p.key != key) {
            self.views.clear();
            // A device that can't build the pipelines (validation, out of memory) turns the GPU
            // view off; the views then draw the 2.5D fallback, as the web falls back to 2D.
            let scope = f.device.push_error_scope(wgpu::ErrorFilter::Validation);
            let oom = f.device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
            let pipes = Pipes::new(f.device, key);
            let failed = block_on(oom.pop()).or_else(|| block_on(scope.pop()));
            match failed {
                Some(e) => {
                    eprintln!("graph: 3D view unavailable, drawing the fallback ({e})");
                    FAILED.store(true, Ordering::Relaxed);
                    self.pipes = None;
                    f.rt.invalidate();
                }
                None => self.pipes = Some(pipes),
            }
        }
        if self.pipes.is_none() {
            f.draw_ui();
            self.drew = false;
            return;
        }
        let (frame_size, frame_format) = (f.size, f.format);
        let Frame {
            device,
            queue,
            encoder,
            target,
            rt,
            ui,
            ..
        } = f;
        let scale = rt.env.scale;
        let jobs: Vec<(u32, Job)> = rt
            .state
            .graph
            .gpu_jobs
            .borrow()
            .iter()
            .map(|(k, j)| (*k, j.clone()))
            .collect();
        let pipes = self.pipes.as_mut().expect("pipes");
        for v in self.views.values_mut() {
            v.seen = false;
        }
        self.pending = false;
        // The part of each slot its scroll viewports show (window device pixels, x0 y0 x1 y1).
        let mut visible: HashMap<u32, [f32; 4]> = HashMap::new();
        for (mark, job) in &jobs {
            let Some((rect, _)) = rt.presented(&job.key) else {
                continue;
            };
            let size = (
                (rect[2] * scale).round().max(0.0) as u32,
                (rect[3] * scale).round().max(0.0) as u32,
            );
            if size.0 < 2 || size.1 < 2 {
                continue;
            }
            let mut vis = [rect[0], rect[1], rect[0] + rect[2], rect[1] + rect[3]];
            for k in job.clip_keys {
                if let Some((c, _)) = rt.presented(k) {
                    vis = intersect(vis, [c[0], c[1], c[0] + c[2], c[1] + c[3]]);
                }
            }
            if vis[2] - vis[0] < 1.0 || vis[3] - vis[1] < 1.0 {
                // Scrolled out of view: neither rendered nor drawn (its targets are kept).
                if let Some(v) = self.views.get_mut(mark) {
                    v.seen = true;
                }
                visible.insert(*mark, [0.0; 4]);
                continue;
            }
            visible.insert(*mark, vis.map(|x| x * scale));
            let v = self
                .views
                .entry(*mark)
                .or_insert_with(|| ViewGpu::new(device));
            v.seen = true;
            v.render(device, queue, encoder, pipes, job, size);
        }
        self.views.retain(|_, v| v.seen);

        let clear = rt.clear_color();
        ui.prepare(device, queue, &mut rt.text.atlas, &rt.instances, frame_size);
        let views = &self.views;
        let mut drew = false;
        let mut pending = false;
        // Blit pipelines are made per pass format on first use.
        let formats: Vec<wgpu::TextureFormat> = vec![frame_format, LAYER];
        for fmt in formats {
            pipes.blit(device, fmt);
        }
        let pipes = &*pipes;
        let queue = &**queue;
        let mut hook = |pass: &mut wgpu::RenderPass<'_>, site: &EffectSite<'_>| {
            let Some(v) = views.get(&site.mark) else {
                return;
            };
            let Some(t) = v.targets.as_ref() else { return };
            let Some(pipe) = pipes.blits.get(&site.format) else {
                return;
            };
            let i = site.instance;
            // The slot's own clip cut by its scroll viewports, as a scissor in pass pixels.
            let vis = intersect(
                [i.clip[0], i.clip[1], i.clip[2], i.clip[3]],
                visible
                    .get(&site.mark)
                    .copied()
                    .unwrap_or(rune::render::NO_CLIP),
            );
            let x0 = (vis[0] - site.origin[0])
                .floor()
                .clamp(0.0, site.size[0] as f32);
            let y0 = (vis[1] - site.origin[1])
                .floor()
                .clamp(0.0, site.size[1] as f32);
            let x1 = (vis[2] - site.origin[0])
                .ceil()
                .clamp(0.0, site.size[0] as f32);
            let y1 = (vis[3] - site.origin[1])
                .ceil()
                .clamp(0.0, site.size[1] as f32);
            if x1 - x0 < 1.0 || y1 - y0 < 1.0 {
                return;
            }
            pass.set_scissor_rect(x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
            let (w, h) = (i.rect[2].round() as u32, i.rect[3].round() as u32);
            if (w as i64 - t.size.0 as i64).abs() > 1 || (h as i64 - t.size.1 as i64).abs() > 1 {
                pending = true;
            }
            let fade: [f32; 4] = bytemuck::cast(i.fade);
            let data: [f32; 20] = [
                i.rect[0],
                i.rect[1],
                i.rect[2],
                i.rect[3],
                i.clip[0],
                i.clip[1],
                i.clip[2],
                i.clip[3],
                fade[0],
                fade[1],
                fade[2],
                fade[3],
                site.origin[0],
                site.origin[1],
                if site.srgb_blend { 1.0 } else { 0.0 },
                1.0,
                site.size[0] as f32,
                site.size[1] as f32,
                0.0,
                0.0,
            ];
            if std::env::var_os("KB_GLASS_DEBUG").is_some() {
                eprintln!(
                    "glass slot: in_layer {} rect {:?} clip {:?} scissor {:?} tex {:?}",
                    site.in_layer,
                    i.rect,
                    i.clip,
                    [x0, y0, x1, y1],
                    t.size
                );
            }
            queue.write_buffer(&v.blit_ubo, 0, bytemuck::cast_slice(&data));
            pass.set_pipeline(pipe);
            pass.set_bind_group(0, &t.blit_bg, &[]);
            pass.draw(0..4, 0..1);
            drew = true;
        };
        ui.draw_with(encoder, target, clear, &mut hook);
        self.drew = drew;
        self.pending = pending;
    }

    fn animating(&self) -> bool {
        self.pending
    }

    fn draws_own(&self) -> bool {
        self.drew
    }
}

/// Two rectangles' overlap (x0, y0, x1, y1); empty ones have x1 <= x0 or y1 <= y0.
fn intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].min(b[2]),
        a[3].min(b[3]),
    ]
}

/// Resolves a future that is already complete or completes on the calling thread (wgpu's error
/// scopes resolve during the pop on native backends).
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    use std::task::{Context, Poll, Waker};
    let mut cx = Context::from_waker(Waker::noop());
    let mut fut = std::pin::pin!(fut);
    loop {
        if let Poll::Ready(v) = fut.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}

//! The ULX3S in 3D, the board pane left of the die: the model from src/board_model.rs
//! drawn by the GPU, its LEDs D0..D7 lit as the design drives `led[0..7]`,
//! its buttons B0..B6 pressed with the pointer (held down while the mouse
//! button is; the cap goes down), any part named on hover. Drag turns the
//! board, the right button moves it, the wheel zooms, a double click goes
//! back to the view of the board's photo; Top and Bottom look straight at a
//! side.
//!
//! The board is drawn into a picture of its own (with depth, 4x multisampled)
//! only when something in it changes — the view, an LED, a button — and that
//! picture is put in the pane every frame, so a still board costs nothing.
//!
//! With a record kept (src/trace.rs), the shader marks, for each LED, the
//! last drawing that drew it lit and the last that drew it dark; after a
//! drawing these are read back (a few frames late) and set against the LEDs
//! that drawing was asked to show: `# board …` lines in the record. The same
//! marks say whether the OLED module, its screen, a lit pixel on it and the
//! HDMI cable were drawn; the line's last three columns set those against
//! what was asked (picture serial, lit or dark) and count the wrong ones.
//!
//! `set_oled` supplies pins-up, row-major sRGB pixels for the optional SSD1331
//! module; `set_hdmi` fits the GPDI cable and names the mode beside it. Both
//! default to absent; `set_headers` fits the optional J1/J2 male headers.
//! OLED serial/size changes upload a nearest-sampled image.
//! Windowless pictures accept ECP5_BOARD_PICTURE_OLED=test and
//! ECP5_BOARD_PICTURE_HDMI="640x480 60 Hz", and ECP5_BOARD_PICTURE_HEADERS=1.
//! `set_switches(used, on)` draws SW1 from the reported ON bits (default OFF).
//! `take_switch_clicks` drains used-lever clicks in order; clicks do not move a
//! lever locally. Lever drags turn the board, and double clicks never reset it.
//! ECP5_BOARD_PICTURE_SWITCHES=1010 sets switches 1 and 3 ON in windowless pictures.
use crate::board_model::{self, BoardModel, Camera, Mat4};
use bytemuck::{Pod, Zeroable};
use eframe::{
    egui,
    egui_wgpu::{
        self, wgpu, wgpu::util::DeviceExt, CallbackResources, CallbackTrait, ScreenDescriptor,
    },
};
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};

const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const MAX_MATERIALS: usize = 64;
/// How far a pressed cap goes down, mm (a PTS645 travels 0.25 mm; shown deeper
/// to be seen).
const PRESS_DEPTH: f32 = 0.6;
/// How bright a lit LED's lens is.
const LED_GLOW: f32 = 6.0;

/// The board's own words in the window; a test checks that the window's fonts
/// draw every character of them (a missing one shows as a box).
const VIEW_HINT: &str = "drag: turn · right drag: move · wheel: zoom";
const LOADING: &str = "Loading the 3D board…";
/// The HDMI cable's label.
fn hdmi_label(mode: &str) -> String {
    format!("HDMI out: {mode}")
}

fn switch_label(index: usize, on: bool, used: bool) -> String {
    format!(
        "SW1 switch {}: sw[{index}], {}{}",
        index + 1,
        if on { "ON" } else { "OFF" },
        if used {
            "; click to toggle"
        } else {
            ", not read by the design"
        }
    )
}

fn picture_switches(value: &str) -> [bool; 4] {
    if value.len() == 4 && value.bytes().all(|b| matches!(b, b'0' | b'1')) {
        std::array::from_fn(|i| value.as_bytes()[i] == b'1')
    } else {
        [false; 4]
    }
}

fn object_offsets(
    pressed: [bool; 7],
    options: [bool; 3],
    switches: [bool; 4],
    travel: [Option<board_model::V3>; 4],
) -> [[f32; 4]; 16] {
    let mut objects = [[0.0; 4]; 16];
    for (i, visible) in options.into_iter().enumerate() {
        objects[8 + i][3] = u32::from(visible) as f32;
    }
    for (b, down) in pressed.into_iter().enumerate() {
        if down {
            objects[1 + b] = [0.0, -PRESS_DEPTH, 0.0, 0.0];
        }
    }
    for (s, on) in switches.into_iter().enumerate() {
        if let (true, Some(t)) = (on, travel[s]) {
            objects[11 + s][..3].copy_from_slice(&t);
        }
    }
    objects
}

/// A pins-up OLED picture: RGBA8 sRGB rows, top at the header, GND at left.
/// Change `serial` whenever pixels change (even if the allocation is reused).
#[derive(Clone)]
pub struct BoardScreen {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    pub serial: u64,
}

impl BoardScreen {
    fn valid(&self, limit: u32) -> bool {
        self.width > 0
            && self.height > 0
            && self.width <= limit
            && self.height <= limit
            && u64::from(self.width) * u64::from(self.height) * 4 == self.rgba.len() as u64
    }
}

/// A cable fitted to GPDI1; `mode` is shown beside it and on hover.
pub struct BoardHdmi {
    pub mode: String,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    eye: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    /// Lights, in the view's frame (right, up, toward the eye), and how
    /// strong: they turn with the view, so every side is lit alike.
    key: [f32; 4],
    fill: [f32; 4],
    /// The drawing's number (for the marks), whether to mark, whether to
    /// encode to sRGB in the shader, unused.
    info: [u32; 4],
    /// The glow's radius, mm.
    glow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMaterial {
    base: [f32; 4],
    emissive: [f32; 4],
    /// metallic, roughness, images (0 none, 1 top, 2 bottom, 3 OLED), LED + 1.
    pbr: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Glow {
    center: [f32; 3],
    strength: f32,
    color: [f32; 3],
    pad: f32,
}

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    key: vec4<f32>,
    fill: vec4<f32>,
    info: vec4<u32>,
    glow: vec4<f32>,
};
struct Material { base: vec4<f32>, emissive: vec4<f32>, pbr: vec4<f32> };
@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<uniform> materials: array<Material, 64>;
@group(0) @binding(2) var<uniform> objects: array<vec4<f32>, 16>;
//MARKS @group(0) @binding(3) var<storage, read_write> marks: array<atomic<u32>, 32>;
@group(1) @binding(0) var top_color: texture_2d<f32>;
@group(1) @binding(1) var top_data: texture_2d<f32>;
@group(1) @binding(2) var bottom_color: texture_2d<f32>;
@group(1) @binding(3) var bottom_data: texture_2d<f32>;
@group(1) @binding(4) var samp: sampler;
@group(1) @binding(5) var oled_color: texture_2d<f32>;
@group(1) @binding(6) var oled_samp: sampler;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) material: u32,
    @location(4) object: u32,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) material: u32,
    @location(4) @interpolate(flat) object: u32,
};

@vertex
fn vs(v: VIn) -> VOut {
    var o: VOut;
    let p = v.pos + objects[v.object].xyz;
    o.clip = u.view_proj * vec4<f32>(p, 1.0);
    if (v.object >= 8u && v.object <= 10u && objects[v.object].w < 0.5) {
        o.clip = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    o.world = p;
    o.normal = v.normal;
    o.uv = v.uv;
    o.material = v.material;
    o.object = v.object;
    return o;
}

fn encode(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

// Colours stay as they are up to 0.8; brighter ones roll off toward 1.
fn tonemap(c: vec3<f32>) -> vec3<f32> {
    let k = 0.8;
    let over = k + (1.0 - k) * (1.0 - exp(-(c - k) / (1.0 - k)));
    return clamp(select(c, over, c > vec3<f32>(k)), vec3<f32>(0.0), vec3<f32>(1.0));
}

fn shade(i: VOut, front: bool) -> vec3<f32> {
    let m = materials[i.material];
    // Sampled for every point (uniform control flow), used where the
    // material says.
    let tc = textureSample(top_color, samp, i.uv);
    let td = textureSample(top_data, samp, i.uv);
    let bc = textureSample(bottom_color, samp, i.uv);
    let bd = textureSample(bottom_data, samp, i.uv);
    let oled = textureSample(oled_color, oled_samp, i.uv);
    var base = m.base.rgb;
    var metal = m.pbr.x;
    var rough = m.pbr.y;
    if (m.pbr.z > 0.5 && m.pbr.z < 1.5) {
        base = base * tc.rgb;
        rough = td.g;
        metal = td.b;
    }
    if (m.pbr.z > 1.5 && m.pbr.z < 2.5) {
        base = base * bc.rgb;
        rough = bd.g;
        metal = bd.b;
    }
    var n = normalize(i.normal);
    if (!front) {
        n = -n;
    }
    let v = normalize(u.eye.xyz - i.world);
    rough = clamp(rough, 0.05, 1.0);
    let f0 = mix(vec3<f32>(0.04), base, metal);
    let diff = base * (1.0 - metal);
    let right = u.right.xyz;
    let up = u.up.xyz;
    let back = cross(right, up);
    // Even light all round, a little more on what faces the eye.
    var c = diff * (0.26 + 0.06 * max(dot(n, back), 0.0));
    let shin = clamp(2.0 / (rough * rough * rough * rough) - 2.0, 2.0, 2048.0);
    for (var k = 0; k < 2; k = k + 1) {
        let light = select(u.fill, u.key, k == 0);
        let l = normalize(right * light.x + up * light.y + back * light.z);
        let ndl = max(dot(n, l), 0.0);
        let h = normalize(l + v);
        let fres = f0 + (1.0 - f0) * pow(1.0 - max(dot(h, v), 0.0), 5.0);
        let spec = fres * pow(max(dot(n, h), 0.0), shin) * (shin + 8.0) / 25.0;
        c = c + (diff + spec) * ndl * light.w;
    }
    // Gloss and metal mirror a room that is bright above the view, darker below.
    let r = reflect(-v, n);
    let env = mix(vec3<f32>(0.3), vec3<f32>(1.0), smoothstep(-0.4, 0.6, dot(r, up)));
    let fv = f0 + (1.0 - f0) * pow(1.0 - max(dot(n, v), 0.0), 5.0);
    c = c + env * fv * (1.0 - rough * 0.85);
    c = c + m.emissive.rgb * m.emissive.w;
    if (m.pbr.z > 2.5) {
        // Black glass contributes its specular room/lamp reflections above.
        // 2x linear white reaches display white through the highlight rolloff.
        c = c + oled.rgb * 2.0;
    }
//MARKS     let led = u32(m.pbr.w + 0.5);
//MARKS     if (led > 0u && u.info.y == 1u) {
//MARKS         let slot = select(led + 7u, led - 1u, m.emissive.w > 0.0);
//MARKS         atomicMax(&marks[slot], u.info.x);
//MARKS     }
//MARKS     // The options: 16 the OLED's screen drawn, 17 a lit pixel of it drawn,
//MARKS     // 18 the HDMI cable drawn, 19 the OLED module drawn.
//MARKS     if (u.info.y == 1u) {
//MARKS         if (m.pbr.z > 2.5) {
//MARKS             atomicMax(&marks[16], u.info.x);
//MARKS             if (max(oled.r, max(oled.g, oled.b)) > 0.02) {
//MARKS                 atomicMax(&marks[17], u.info.x);
//MARKS             }
//MARKS         }
//MARKS         if (i.object == 9u) {
//MARKS             atomicMax(&marks[18], u.info.x);
//MARKS         }
//MARKS         if (i.object == 8u) {
//MARKS             atomicMax(&marks[19], u.info.x);
//MARKS         }
//MARKS     }
    var out = tonemap(c);
    if (u.info.z == 1u) {
        out = encode(out);
    }
    return out;
}

@fragment
fn fs(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    return vec4<f32>(shade(i, front), 1.0);
}

@fragment
fn fs_blend(i: VOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let a = materials[i.material].base.a;
    return vec4<f32>(shade(i, front) * a, a);
}

struct GIn {
    @location(0) center: vec3<f32>,
    @location(1) strength: f32,
    @location(2) color: vec3<f32>,
    @location(3) pad: f32,
};
struct GOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) q: vec2<f32>,
    @location(1) color: vec3<f32>,
    @location(2) strength: f32,
};

@vertex
fn vs_glow(@builtin(vertex_index) k: u32, g: GIn) -> GOut {
    var o: GOut;
    let q = vec2<f32>(f32(k & 1u) * 2.0 - 1.0, f32((k >> 1u) & 1u) * 2.0 - 1.0);
    let p = g.center + (u.right.xyz * q.x + u.up.xyz * q.y) * u.glow.x;
    o.clip = u.view_proj * vec4<f32>(p, 1.0);
    o.q = q;
    o.color = g.color;
    // The LEDs are on the top: no glow seen from below the board.
    o.strength = select(0.0, g.strength, u.eye.y > 0.8);
    return o;
}

@fragment
fn fs_glow(i: GOut) -> @location(0) vec4<f32> {
    let a = exp(-dot(i.q, i.q) * 4.0) * i.strength;
    var c = min(i.color * a, vec3<f32>(1.0));
    if (u.info.z == 1u) {
        c = encode(c);
    }
    return vec4<f32>(c, 0.0);
}
"#;

const BLIT: &str = r#"
@group(0) @binding(0) var picture: texture_2d<f32>;
@group(0) @binding(1) var picture_sampler: sampler;
struct BOut { @builtin(position) clip: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex
fn vs_blit(@builtin(vertex_index) i: u32) -> BOut {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: BOut;
    o.clip = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    return o;
}
@fragment
fn fs_blit(i: BOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(picture, picture_sampler, i.uv, 0.0);
}
"#;

/// What the LED marks said after drawing `render`: for each LED the last
/// drawing that drew it lit (0..8) and dark (8..16).
#[derive(Clone, Copy, Debug)]
pub struct BoardRead {
    pub render: u32,
    pub marks: [u32; 32],
}

impl BoardRead {
    fn lit(&self) -> [bool; 8] {
        std::array::from_fn(|i| self.marks[i] == self.render)
    }
}

enum Reading {
    Idle,
    Copied(u32),
    Mapping(u32, Arc<AtomicBool>),
}

struct Uploaded {
    id: usize,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    opaque: std::ops::Range<u32>,
    blended: std::ops::Range<u32>,
    textures: wgpu::BindGroup,
    image_views: Vec<wgpu::TextureView>,
    oled_texture: wgpu::Texture,
    oled_key: Option<(u64, u32, u32)>,
    materials: Vec<GpuMaterial>,
    led_material: [Option<usize>; 8],
    led_color: [[f32; 3]; 8],
    leds: [Option<[f32; 3]>; 8],
    switch_travel: [Option<board_model::V3>; 4],
}

struct Target {
    size: [u32; 2],
    msaa_view: Option<wgpu::TextureView>,
    depth_view: wgpu::TextureView,
    picture_view: wgpu::TextureView,
    blit_group: wgpu::BindGroup,
}

/// The GPU's side, kept among egui's paint callback resources.
pub struct BoardGpu {
    format: wgpu::TextureFormat,
    samples: u32,
    encode: bool,
    marks_ok: bool,
    opaque: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    glow: wgpu::RenderPipeline,
    blit: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    blit_layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    materials: wgpu::Buffer,
    objects: wgpu::Buffer,
    marks: wgpu::Buffer,
    staging: wgpu::Buffer,
    glows: wgpu::Buffer,
    group: wgpu::BindGroup,
    sampler: wgpu::Sampler,
    oled_sampler: wgpu::Sampler,
    blit_sampler: wgpu::Sampler,
    model: Option<Uploaded>,
    target: Option<Target>,
    reading: Reading,
    drawn: Option<u32>,
}

impl BoardGpu {
    pub fn install(state: &egui_wgpu::RenderState) {
        let gpu = Self::new(&state.device, &state.adapter, state.target_format);
        state.renderer.write().callback_resources.insert(gpu);
    }

    fn new(
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        let marks_ok = adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::FRAGMENT_WRITABLE_STORAGE);
        let features = adapter.get_texture_format_features(target_format).flags;
        let samples = if features.contains(
            wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4
                | wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE,
        ) {
            4
        } else {
            1
        };
        let encode = !target_format.is_srgb();
        let source = if marks_ok {
            SHADER.replace("//MARKS", "")
        } else {
            SHADER.to_string()
        };
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("board"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let blit_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("board blit"),
            source: wgpu::ShaderSource::Wgsl(BLIT.into()),
        });
        let buffer_entry = |binding, ty, visibility| wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let vf = wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT;
        let mut entries = vec![
            buffer_entry(0, wgpu::BufferBindingType::Uniform, vf),
            buffer_entry(1, wgpu::BufferBindingType::Uniform, vf),
            buffer_entry(2, wgpu::BufferBindingType::Uniform, vf),
        ];
        if marks_ok {
            entries.push(buffer_entry(
                3,
                wgpu::BufferBindingType::Storage { read_only: false },
                wgpu::ShaderStages::FRAGMENT,
            ));
        }
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("board"),
            entries: &entries,
        });
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("board images"),
            entries: &[
                texture_entry(0),
                texture_entry(1),
                texture_entry(2),
                texture_entry(3),
                sampler_entry(4),
                texture_entry(5),
                sampler_entry(6),
            ],
        });
        let blit_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("board picture"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("board"),
            bind_group_layouts: &[Some(&layout), Some(&texture_layout)],
            immediate_size: 0,
        });
        let vertex = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<board_model::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x2, 3 => Uint32, 4 => Uint32],
        };
        let multisample = wgpu::MultisampleState {
            count: samples,
            ..Default::default()
        };
        let surface = |entry: &'static str, blend: Option<wgpu::BlendState>, depth_write: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("board"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: std::slice::from_ref(&vertex),
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(depth_write),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample,
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let opaque = surface("fs", None, true);
        let blended = surface(
            "fs_blend",
            Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            false,
        );
        let glow_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Glow>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32, 2 => Float32x3, 3 => Float32],
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let glow = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("board glow"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_glow"),
                compilation_options: Default::default(),
                buffers: &[glow_layout],
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_glow"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(additive),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("board picture"),
            bind_group_layouts: &[Some(&blit_layout)],
            immediate_size: 0,
        });
        let blit = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("board picture"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_module,
                entry_point: Some("vs_blit"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &blit_module,
                entry_point: Some("fs_blit"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let buffer = |label, size, usage| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };
        let uniforms = buffer(
            "board uniforms",
            std::mem::size_of::<Uniforms>() as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let materials = buffer(
            "board materials",
            (MAX_MATERIALS * std::mem::size_of::<GpuMaterial>()) as u64,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let objects = buffer(
            "board objects",
            16 * 16,
            wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        );
        let marks = buffer(
            "board marks",
            128,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
        );
        let staging = buffer(
            "board marks read",
            128,
            wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        );
        let glows = buffer(
            "board glows",
            (8 * std::mem::size_of::<Glow>()) as u64,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let mut group_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: materials.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: objects.as_entire_binding(),
            },
        ];
        if marks_ok {
            group_entries.push(wgpu::BindGroupEntry {
                binding: 3,
                resource: marks.as_entire_binding(),
            });
        }
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("board"),
            layout: &layout,
            entries: &group_entries,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("board images"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });
        let blit_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("board picture"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let oled_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("OLED pixels"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        BoardGpu {
            format: target_format,
            samples,
            encode,
            marks_ok,
            opaque,
            blended,
            glow,
            blit,
            texture_layout,
            blit_layout,
            uniforms,
            materials,
            objects,
            marks,
            staging,
            glows,
            group,
            sampler,
            oled_sampler,
            blit_sampler,
            model: None,
            target: None,
            reading: Reading::Idle,
            drawn: None,
        }
    }

    fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        id: usize,
        model: &BoardModel,
    ) {
        let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("board vertices"),
            contents: bytemuck::cast_slice(&model.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let indices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("board indices"),
            contents: bytemuck::cast_slice(&model.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        let views: Vec<wgpu::TextureView> = model
            .images
            .iter()
            .map(|image| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("board image"),
                    size: wgpu::Extent3d {
                        width: image.width,
                        height: image.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: image.levels.len() as u32,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: if image.srgb {
                        wgpu::TextureFormat::Rgba8UnormSrgb
                    } else {
                        wgpu::TextureFormat::Rgba8Unorm
                    },
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let (mut w, mut h) = (image.width, image.height);
                for (level, data) in image.levels.iter().enumerate() {
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: &texture,
                            mip_level: level as u32,
                            origin: wgpu::Origin3d::ZERO,
                            aspect: wgpu::TextureAspect::All,
                        },
                        data,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(4 * w),
                            rows_per_image: Some(h),
                        },
                        wgpu::Extent3d {
                            width: w,
                            height: h,
                            depth_or_array_layers: 1,
                        },
                    );
                    w = (w / 2).max(1);
                    h = (h / 2).max(1);
                }
                texture.create_view(&Default::default())
            })
            .collect();
        // WebGPU initializes new textures to zero: black before any picture.
        let oled_texture = Self::oled_texture(device, 1, 1);
        let textures = self.image_group(device, &views, &oled_texture);
        let mut led_material = [None; 8];
        let materials: Vec<GpuMaterial> = model
            .materials
            .iter()
            .take(MAX_MATERIALS)
            .enumerate()
            .map(|(k, m)| {
                if let Some(l) = m.led {
                    led_material[l] = Some(k);
                }
                let mut base = m.base;
                if m.blended {
                    base[3] = 0.6;
                }
                GpuMaterial {
                    base,
                    emissive: [0.0; 4],
                    pbr: [
                        m.metallic,
                        m.roughness,
                        m.textures as f32,
                        m.led.map_or(0.0, |l| l as f32 + 1.0),
                    ],
                }
            })
            .collect();
        let mut led_color = [[1.0; 3]; 8];
        for p in &model.parts {
            if let (Some(l), true) = (p.led, p.name.starts_with('D')) {
                led_color[l] = board_model::led_light(&p.value);
            }
        }
        self.model = Some(Uploaded {
            id,
            vertices,
            indices,
            opaque: model.opaque.clone(),
            blended: model.blended.clone(),
            textures,
            image_views: views,
            oled_texture,
            oled_key: None,
            materials,
            led_material,
            led_color,
            leds: model.leds,
            switch_travel: model.switch_travel,
        });
        self.drawn = None;
    }

    fn oled_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("OLED picture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn image_group(
        &self,
        device: &wgpu::Device,
        views: &[wgpu::TextureView],
        oled: &wgpu::Texture,
    ) -> wgpu::BindGroup {
        let oled_view = oled.create_view(&Default::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("board images"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&oled_view),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&self.oled_sampler),
                },
            ],
        })
    }

    fn update_oled(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        oled: Option<&BoardScreen>,
    ) {
        let Some(screen) = oled else { return };
        let limit = device.limits().max_texture_dimension_2d;
        if !screen.valid(limit) {
            return;
        }
        let key = (screen.serial, screen.width, screen.height);
        let Some(model) = &self.model else { return };
        if model.oled_key == Some(key) {
            return;
        }
        if model.oled_texture.width() != screen.width
            || model.oled_texture.height() != screen.height
        {
            let texture = Self::oled_texture(device, screen.width, screen.height);
            let group = self.image_group(device, &model.image_views, &texture);
            let model = self.model.as_mut().unwrap();
            model.oled_texture = texture;
            model.textures = group;
        }
        let model = self.model.as_mut().unwrap();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &model.oled_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &screen.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * screen.width),
                rows_per_image: Some(screen.height),
            },
            wgpu::Extent3d {
                width: screen.width,
                height: screen.height,
                depth_or_array_layers: 1,
            },
        );
        model.oled_key = Some(key);
    }

    fn size(&mut self, device: &wgpu::Device, size: [u32; 2]) {
        if self.target.as_ref().is_some_and(|t| t.size == size) {
            return;
        }
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let texture = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let picture_view = texture(
            "board picture",
            self.format,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
        );
        let msaa_view = (self.samples > 1).then(|| {
            texture(
                "board picture, multisampled",
                self.format,
                self.samples,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        });
        let depth_view = texture(
            "board depth",
            DEPTH,
            self.samples,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let blit_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("board picture"),
            layout: &self.blit_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&picture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.blit_sampler),
                },
            ],
        });
        self.target = Some(Target {
            size,
            msaa_view,
            depth_view,
            picture_view,
            blit_group,
        });
        self.drawn = None;
    }

    /// Draws the board into its picture as drawing number `render`.
    #[allow(clippy::too_many_arguments)]
    fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &Uniforms,
        lit: [bool; 8],
        pressed: [bool; 7],
        options: [bool; 3],
        switches: [bool; 4],
        render: u32,
        readback: bool,
    ) {
        let gpu = self;
        let (Some(m), Some(target)) = (&gpu.model, &gpu.target) else {
            return;
        };
        let mut materials = m.materials.clone();
        let mut glows = Vec::new();
        for (l, &on) in lit.iter().enumerate() {
            if let Some(k) = m.led_material[l] {
                let c = m.led_color[l];
                materials[k].emissive = if on {
                    [c[0], c[1], c[2], LED_GLOW]
                } else {
                    [0.0; 4]
                };
                if on {
                    if let Some(at) = m.leds[l] {
                        glows.push(Glow {
                            center: [at[0], at[1] + 0.3, at[2]],
                            strength: 1.0,
                            color: c,
                            pad: 0.0,
                        });
                    }
                }
            }
        }
        let objects = object_offsets(pressed, options, switches, m.switch_travel);
        let mut u = *uniforms;
        u.info = [
            render,
            u32::from(readback && gpu.marks_ok),
            u32::from(gpu.encode),
            0,
        ];
        queue.write_buffer(&gpu.uniforms, 0, bytemuck::bytes_of(&u));
        queue.write_buffer(&gpu.materials, 0, bytemuck::cast_slice(&materials));
        queue.write_buffer(&gpu.objects, 0, bytemuck::cast_slice(&objects));
        if !glows.is_empty() {
            queue.write_buffer(&gpu.glows, 0, bytemuck::cast_slice(&glows));
        }
        {
            let (view, resolve) = match &target.msaa_view {
                Some(msaa) => (msaa, Some(&target.picture_view)),
                None => (&target.picture_view, None),
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("board"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: resolve,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: if resolve.is_some() {
                            wgpu::StoreOp::Discard
                        } else {
                            wgpu::StoreOp::Store
                        },
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &gpu.group, &[]);
            pass.set_bind_group(1, &m.textures, &[]);
            pass.set_vertex_buffer(0, m.vertices.slice(..));
            pass.set_index_buffer(m.indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.set_pipeline(&gpu.opaque);
            pass.draw_indexed(m.opaque.clone(), 0, 0..1);
            if !m.blended.is_empty() {
                pass.set_pipeline(&gpu.blended);
                pass.draw_indexed(m.blended.clone(), 0, 0..1);
            }
            if !glows.is_empty() {
                pass.set_pipeline(&gpu.glow);
                pass.set_vertex_buffer(0, gpu.glows.slice(..));
                pass.draw(0..4, 0..glows.len() as u32);
            }
        }
        if readback && gpu.marks_ok && matches!(gpu.reading, Reading::Idle) && render > 0 {
            encoder.copy_buffer_to_buffer(&gpu.marks, 0, &gpu.staging, 0, 128);
            gpu.reading = Reading::Copied(render);
        }
        gpu.drawn = Some(render);
    }

    /// Reads the LED marks back, a step each frame: copy, map, read.
    fn read_back(&mut self, device: &wgpu::Device) -> Option<BoardRead> {
        match &self.reading {
            Reading::Idle => None,
            Reading::Copied(render) => {
                let render = *render;
                let done = Arc::new(AtomicBool::new(false));
                let flag = Arc::clone(&done);
                self.staging
                    .slice(..)
                    .map_async(wgpu::MapMode::Read, move |r| {
                        if r.is_ok() {
                            flag.store(true, Ordering::Release);
                        }
                    });
                self.reading = Reading::Mapping(render, done);
                None
            }
            Reading::Mapping(render, done) => {
                let _ = device.poll(wgpu::PollType::Poll);
                if !done.load(Ordering::Acquire) {
                    return None;
                }
                let render = *render;
                let marks = {
                    let view = self.staging.slice(..).get_mapped_range();
                    let words: &[u32] = bytemuck::cast_slice(&view);
                    std::array::from_fn(|i| words[i])
                };
                self.staging.unmap();
                self.reading = Reading::Idle;
                Some(BoardRead { render, marks })
            }
        }
    }
}

/// A frame's board: what to draw, sent to the GPU's side.
#[derive(Clone)]
pub struct BoardCallback {
    model: Option<(usize, Arc<BoardModel>)>,
    rect: egui::Rect,
    uniforms: Uniforms,
    lit: [bool; 8],
    pressed: [bool; 7],
    oled: Option<BoardScreen>,
    hdmi: bool,
    headers: bool,
    switches: [bool; 4],
    /// Stays wanted until the readback confirms the LEDs.
    render: Option<Arc<WantedRender>>,
    completed: Arc<Mutex<VecDeque<CompletedRender>>>,
    frame: u64,
    came: Option<u64>,
    read: Arc<Mutex<VecDeque<BoardRead>>>,
    recording: bool,
}

impl BoardCallback {
    fn render_number(&self, drawn: Option<u32>) -> Option<u32> {
        match (self.render.as_ref(), drawn) {
            (Some(wanted), previous) if previous != Some(wanted.number) => Some(wanted.number),
            // A new picture (size, model) needs a drawing too.
            (None, None) => Some(0),
            _ => None,
        }
    }

    /// Called only after the board commands were submitted and `drawn` advanced.
    fn did_draw(&self, readback_supported: bool) {
        let Some(wanted) = &self.render else {
            return;
        };
        self.completed.lock().unwrap().push_back(CompletedRender {
            wanted: Arc::clone(wanted),
            record: self.recording,
            readback_supported,
            drawing: Drawing {
                number: wanted.number,
                at: Instant::now(),
                frame: self.frame,
                lit: self.lit,
                came: self.came,
                interval: None,
                oled: self.oled.as_ref().map(|o| {
                    (
                        o.serial,
                        o.rgba
                            .chunks_exact(4)
                            .any(|px| px[..3].iter().any(|&c| c > 5)),
                    )
                }),
                hdmi: self.hdmi,
            },
        });
    }
}

impl CallbackTrait for BoardCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<BoardGpu>() else {
            return Vec::new();
        };
        let Some((id, model)) = &self.model else {
            return Vec::new();
        };
        if gpu.model.as_ref().is_none_or(|m| m.id != *id) {
            gpu.upload(device, queue, *id, model);
        }
        gpu.update_oled(device, queue, self.oled.as_ref());
        if let Some(read) = gpu.read_back(device) {
            self.read.lock().unwrap().push_back(read);
        }
        let px = egui::epaint::ViewportInPixels::from_points(
            &self.rect,
            screen.pixels_per_point,
            screen.size_in_pixels,
        );
        let size = [px.width_px.max(1) as u32, px.height_px.max(1) as u32];
        gpu.size(device, size);
        let Some(render) = self.render_number(gpu.drawn) else {
            return Vec::new();
        };
        // Sent now, not with egui's frame: egui calls this before it gets the
        // window's surface, and when it cannot (the window hidden) it drops
        // the frame's commands — the board's picture and the marks' copy with
        // them, while this side would count them as done.
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("board"),
        });
        gpu.draw(
            queue,
            &mut encoder,
            &self.uniforms,
            self.lit,
            self.pressed,
            [self.oled.is_some(), self.hdmi, self.headers],
            self.switches,
            render,
            true,
        );
        queue.submit([encoder.finish()]);
        if gpu.drawn == Some(render) {
            self.did_draw(gpu.marks_ok);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        let Some(gpu) = resources.get::<BoardGpu>() else {
            return;
        };
        let (Some(target), Some(_)) = (&gpu.target, gpu.drawn) else {
            return;
        };
        pass.set_pipeline(&gpu.blit);
        pass.set_bind_group(0, &target.blit_group, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// The shader's uniforms for a camera at `distance`.
fn uniforms(camera: &Camera, distance: f32, aspect: f32) -> Uniforms {
    let vp = camera.view_proj(distance, aspect);
    let (_, right, up) = camera.basis();
    let eye = camera.eye(distance);
    // From above left of the eye, and a weak one from the right.
    let key_light = board_model::normalize([-0.5, 0.8, 1.0]);
    let fill_light = board_model::normalize([0.8, -0.1, 0.6]);
    Uniforms {
        view_proj: vp.0,
        eye: [eye[0], eye[1], eye[2], 0.0],
        right: [right[0], right[1], right[2], 0.0],
        up: [up[0], up[1], up[2], 0.0],
        key: [key_light[0], key_light[1], key_light[2], 0.55],
        fill: [fill_light[0], fill_light[1], fill_light[2], 0.2],
        info: [0; 4],
        glow: [2.6, 0.0, 0.0, 0.0],
    }
}

/// Draws the board without a window, as the panel would (`ecp5_gui
/// --board-picture`): `width` x `height` pixels on the panel's dark grey, from
/// the view `view` (3d, top, bottom), the LEDs in `lit` lit, the buttons in
/// `pressed` down; written as PNG.
/// ECP5_BOARD_PICTURE_OLED=test fits the module with a 96x64 test pattern;
/// ECP5_BOARD_PICTURE_HDMI fits a cable (its value supplies the mode label in UI).
/// ECP5_BOARD_PICTURE_HEADERS=1 fits the two GPIO headers.
/// ECP5_BOARD_PICTURE_SWITCHES=1010 sets switch 1/3 ON and switch 2/4 OFF.
pub fn picture(
    out: &std::path::Path,
    view: &str,
    lit: [bool; 8],
    pressed: [bool; 7],
    width: u32,
    height: u32,
) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let model = board_model::load(&board_model::default_path())?;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
            .context("no GPU adapter")?;
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))?;
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut gpu = BoardGpu::new(&device, &adapter, format);
    gpu.upload(&device, &queue, 1, &model);
    gpu.size(&device, [width, height]);
    let mut board = BoardView::with_load(Load::NoGpu);
    board.set_oled(
        (std::env::var("ECP5_BOARD_PICTURE_OLED").as_deref() == Ok("test")).then(oled_test_picture),
    );
    board.set_hdmi(
        std::env::var("ECP5_BOARD_PICTURE_HDMI")
            .ok()
            .map(|mode| BoardHdmi { mode }),
    );
    board.set_headers(std::env::var("ECP5_BOARD_PICTURE_HEADERS").as_deref() == Ok("1"));
    let switches = std::env::var("ECP5_BOARD_PICTURE_SWITCHES")
        .map(|value| picture_switches(&value))
        .unwrap_or([false; 4]);
    board.set_switches([false; 4], switches);
    gpu.update_oled(&device, &queue, board.oled.as_ref());
    let view_of = BoardView::camera_for(view);
    let aspect = width as f32 / height as f32;
    let distance = view_of.fit(&model.outline, aspect) * view_of.zoom;
    let mut encoder = device.create_command_encoder(&Default::default());
    gpu.draw(
        &queue,
        &mut encoder,
        &uniforms(&view_of, distance, aspect),
        lit,
        pressed,
        [board.oled.is_some(), board.hdmi.is_some(), board.headers],
        board.switches_on,
        1,
        false,
    );
    let row = (4 * width).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("board picture read"),
        size: (row * height) as u64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let target = gpu.target.as_ref().context("no picture")?;
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target.picture_view.texture(),
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    let data = buffer.slice(..).get_mapped_range();
    // Premultiplied over the panel's grey (egui's dark panel).
    let bg = 27.0f32;
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for y in 0..height {
        let line = &data[(y * row) as usize..(y * row + 4 * width) as usize];
        for px in line.chunks_exact(4) {
            let a = px[3] as f32 / 255.0;
            for &v in &px[..3] {
                rgb.push((v as f32 + bg * (1.0 - a)).min(255.0) as u8);
            }
        }
    }
    image::RgbImage::from_raw(width, height, rgb)
        .context("picture size")?
        .save(out)
        .with_context(|| format!("writing {}", out.display()))?;
    Ok(())
}

// --- the panel's side ----------------------------------------------------------

fn oled_test_picture() -> BoardScreen {
    let mut rgba = vec![0u8; 96 * 64 * 4];
    let bars = [
        [255, 255, 255],
        [255, 255, 0],
        [0, 255, 255],
        [0, 255, 0],
        [255, 0, 255],
        [255, 0, 0],
        [0, 0, 255],
        [0, 0, 0],
    ];
    for y in 0..64 {
        for x in 0..96 {
            let color = if x == 0 || x == 95 || y == 0 || y == 63 {
                [255; 3]
            } else if y < 32 {
                bars[x / 12]
            } else {
                [0; 3]
            };
            let at = (y * 96 + x) * 4;
            rgba[at..at + 3].copy_from_slice(&color);
            rgba[at + 3] = 255;
        }
    }
    // Five columns by seven rows per glyph, doubled to keep the text legible.
    let glyphs = [
        [17, 17, 17, 17, 17, 17, 14],
        [16, 16, 16, 16, 16, 16, 31],
        [17, 17, 10, 4, 10, 17, 17],
        [30, 1, 1, 14, 1, 1, 30],
        [15, 16, 16, 14, 1, 1, 30],
    ];
    for (g, rows) in glyphs.iter().enumerate() {
        for (y, row) in rows.iter().enumerate() {
            for x in 0..5 {
                if row & (1 << (4 - x)) != 0 {
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let at = ((42 + y * 2 + dy) * 96 + 19 + g * 12 + x * 2 + dx) * 4;
                            rgba[at..at + 3].fill(255);
                        }
                    }
                }
            }
        }
    }
    BoardScreen {
        width: 96,
        height: 64,
        rgba: rgba.into(),
        serial: 1,
    }
}

enum Load {
    NoGpu,
    /// ECP5_GUI_NO_BOARD: the board's room kept, nothing drawn in it.
    Measuring,
    Loading(mpsc::Receiver<anyhow::Result<BoardModel>>),
    Ready(Arc<BoardModel>),
    Failed(String),
}

fn model_load_error(error: &anyhow::Error) -> String {
    let message = format!("{error:#}");
    match message.as_str() {
        "3D board model not found; showing the flat board. Run `git lfs pull` or see tools/ulx3s_3d."
        | "3D board model is a git LFS pointer; run `git lfs pull`." => message,
        _ => format!("3D board model: {message}"),
    }
}

/// What the board is to show this frame.
pub struct BoardInputs {
    /// `led[n]` lit (used by the design and driven high).
    pub lit: [bool; 8],
    /// `led[n]`'s value (none: unknown).
    pub values: [Option<bool>; 8],
    pub leds_used: [bool; 8],
    pub buttons_used: [bool; 7],
    /// Each button held down, by whatever means (shown pressed).
    pub held: [bool; 7],
}

/// How the pointer started on the board: turning it, on a button, or on a lever.
#[derive(Clone, Copy, PartialEq)]
enum Grab {
    Turn,
    Button(usize),
    Switch(usize),
}

/// One submitted drawing of the board, for the record: when it was drawn (and
/// in which of the window's frames), the LEDs it was to show lit, and the frame
/// the newest of those LED values came in.
struct Drawing {
    number: u32,
    at: Instant,
    frame: u64,
    lit: [bool; 8],
    came: Option<u64>,
    /// With new LED values: since the drawing of the LED values before, ms.
    interval: Option<f64>,
    /// The OLED module asked for: its picture's serial and whether any of its
    /// pixels is lit; the HDMI cable asked for.
    oled: Option<(u64, bool)>,
    hdmi: bool,
}

struct WantedRender {
    number: u32,
    /// A retry has a fresh number but still belongs to this wanted picture.
    first: u32,
    key: Vec<u32>,
    lit: [bool; 8],
}

struct CompletedRender {
    wanted: Arc<WantedRender>,
    drawing: Drawing,
    record: bool,
    readback_supported: bool,
}

pub struct BoardView {
    load: Load,
    id: usize,
    pub camera: Camera,
    grab: Option<Grab>,
    /// The button held with the pointer on the board.
    pub pressed: Option<usize>,
    /// The last submitted picture, not the last layout pass.
    last_key: Option<Vec<u32>>,
    pending: Option<Arc<WantedRender>>,
    completed: Arc<Mutex<VecDeque<CompletedRender>>>,
    submitted: u32,
    readback_supported: Option<bool>,
    last_read: Option<BoardRead>,
    attempt_frame: Option<u64>,
    #[cfg(test)]
    callback: Option<BoardCallback>,
    oled: Option<BoardScreen>,
    hdmi: Option<BoardHdmi>,
    headers: bool,
    switches_used: [bool; 4],
    switches_on: [bool; 4],
    switch_clicks: Vec<usize>,
    number: u32,
    /// The record (src/trace.rs): kept while true.
    pub recording: bool,
    /// Where the board was drawn last frame, in points.
    pub rect: Option<egui::Rect>,
    read: Arc<Mutex<VecDeque<BoardRead>>>,
    drawings: VecDeque<Drawing>,
    shown: [bool; 8],
    came: Option<u64>,
    notes: Vec<String>,
    /// The record's clock: when the design was ready.
    pub ready: Option<Instant>,
    frame: u64,
    checked: u64,
    wrong: u64,
    unseen: u64,
    late: u64,
    /// Drawings whose OLED module or HDMI cable the GPU drew against what was asked.
    options_wrong: u64,
    /// When the newest LED values were drawn (how often they change).
    changed_at: Option<Instant>,
}

impl BoardView {
    /// Sets the GPU's side up and starts reading the model in the background.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // ECP5_GUI_NO_BOARD=1: the window as it is with the board, its room in
        // the pane kept empty, nothing drawn (to measure what the board costs).
        let wanted = std::env::var_os("ECP5_GUI_NO_BOARD").is_none();
        let load = match cc.wgpu_render_state.as_ref() {
            Some(_) if !wanted => Load::Measuring,
            None => Load::NoGpu,
            Some(state) => {
                BoardGpu::install(state);
                let path = board_model::default_path();
                let (send, receive) = mpsc::channel();
                let ctx = cc.egui_ctx.clone();
                std::thread::Builder::new()
                    .name("board model".into())
                    .spawn(move || {
                        let model = if path.exists() {
                            board_model::load(&path)
                        } else {
                            Err(anyhow::anyhow!(
                                "3D board model not found; showing the flat board. Run `git lfs pull` or see tools/ulx3s_3d."
                            ))
                        };
                        let _ = send.send(model);
                        ctx.request_repaint();
                    })
                    .ok();
                Load::Loading(receive)
            }
        };
        Self::with_load(load)
    }

    fn with_load(load: Load) -> Self {
        Self {
            load,
            id: 1,
            camera: Camera::PHOTO,
            grab: None,
            pressed: None,
            last_key: None,
            pending: None,
            completed: Arc::new(Mutex::new(VecDeque::new())),
            submitted: 0,
            readback_supported: None,
            last_read: None,
            attempt_frame: None,
            #[cfg(test)]
            callback: None,
            oled: None,
            hdmi: None,
            headers: false,
            switches_used: [false; 4],
            switches_on: [false; 4],
            switch_clicks: Vec::new(),
            number: 0,
            recording: false,
            rect: None,
            read: Arc::new(Mutex::new(VecDeque::new())),
            drawings: VecDeque::new(),
            shown: [false; 8],
            came: None,
            notes: Vec::new(),
            ready: None,
            frame: 0,
            checked: 0,
            wrong: 0,
            unseen: 0,
            late: 0,
            options_wrong: 0,
            changed_at: None,
        }
    }

    /// Fits/removes the SSD1331 module. Pixel changes require a new serial.
    pub fn set_oled(&mut self, oled: Option<BoardScreen>) {
        self.oled = oled;
    }

    /// Fits/removes the HDMI cable and updates its mode label.
    pub fn set_hdmi(&mut self, hdmi: Option<BoardHdmi>) {
        self.hdmi = hdmi;
    }

    /// Fits/removes the straight male headers on J1 and J2 (absent by default).
    pub fn set_headers(&mut self, on: bool) {
        self.headers = on;
    }

    /// Updates SW1 from the design's usage and reported state; default unused/OFF.
    pub fn set_switches(&mut self, used: [bool; 4], on: [bool; 4]) {
        self.switches_used = used;
        self.switches_on = on;
    }

    /// Drains used-lever clicks (sw[0..3]) in order, without changing their state.
    pub fn take_switch_clicks(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.switch_clicks)
    }

    /// The view's preset by name: `3d` (the photo's), `top` or `bottom`.
    pub fn view(&mut self, name: &str) {
        self.camera = Self::camera_for(name);
    }

    fn camera_for(name: &str) -> Camera {
        match name {
            "top" => Camera::TOP,
            "bottom" => Camera::BOTTOM,
            _ => Camera::PHOTO,
        }
    }

    /// Whether the 3D board is there to show (its model read, a GPU).
    pub fn available(&self) -> bool {
        matches!(self.load, Load::Ready(_) | Load::Measuring)
    }

    /// The views' buttons and how to move the board, on one line.
    pub fn toolbar(&mut self, ui: &mut egui::Ui) {
        for (name, label, hint) in [
            (
                "3d",
                "3D",
                "As on the board's photo (double click on the board)",
            ),
            ("top", "Top", "Straight from above"),
            (
                "bottom",
                "Bottom",
                "From below, turned as on the board's bottom photo",
            ),
        ] {
            if ui.small_button(label).on_hover_text(hint).clicked() {
                self.view(name);
            }
        }
        ui.label(egui::RichText::new(VIEW_HINT).small().weak());
    }

    /// The record's lines since the last call.
    pub fn take_notes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.notes)
    }

    /// The board, as wide as the panel; returns the button the pointer holds.
    pub fn show(&mut self, ui: &mut egui::Ui, inputs: &BoardInputs) -> Option<usize> {
        self.frame += 1;
        self.finish_drawings();
        self.judge();
        if let Load::Loading(receive) = &self.load {
            if let Ok(result) = receive.try_recv() {
                self.load = match result {
                    Ok(model) => Load::Ready(Arc::new(model)),
                    Err(e) => Load::Failed(model_load_error(&e)),
                };
            }
        }
        let model = match &self.load {
            Load::Ready(model) => Arc::clone(model),
            Load::Loading(_) => {
                ui.label(egui::RichText::new(LOADING).small().weak());
                return None;
            }
            Load::Failed(e) => {
                ui.label(egui::RichText::new(e).small().weak());
                return None;
            }
            Load::NoGpu => return None,
            Load::Measuring => {
                let size = ui.available_size().max(egui::vec2(120.0, 100.0));
                self.rect = Some(ui.allocate_exact_size(size, egui::Sense::hover()).0);
                return None;
            }
        };
        // All the room left in the pane.
        let size = ui.available_size().max(egui::vec2(120.0, 100.0));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        self.rect = Some(rect);
        let aspect = rect.width() / rect.height().max(1.0);
        let fit = self.camera.fit(&model.outline, aspect);
        let distance = fit * self.camera.zoom;

        // What the pointer is on.
        let pick = |cam: &Camera, pos: egui::Pos2| {
            let at = [
                (pos.x - rect.center().x) / (rect.width() / 2.0),
                -(pos.y - rect.center().y) / (rect.height() / 2.0),
            ];
            let (o, d) = cam.ray(distance, aspect, at);
            model
                .pick(
                    o,
                    d,
                    [self.oled.is_some(), self.hdmi.is_some(), self.headers],
                )
                .map(|(k, _)| k)
        };
        let hovered = response.hover_pos().and_then(|p| pick(&self.camera, p));
        let primary_down = ui.input(|i| i.pointer.primary_down());
        let clicked = response.clicked_by(egui::PointerButton::Primary);
        if (response.is_pointer_button_down_on() && primary_down || clicked) && self.grab.is_none()
        {
            // Use the press position even when press/release arrive in one frame.
            let press = ui.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            let started = press.map_or(hovered, |p| pick(&self.camera, p));
            let on_button = started
                .and_then(|k| model.parts[k].btn)
                .filter(|&b| inputs.buttons_used[b]);
            let on_switch = started.and_then(|k| model.parts[k].sw);
            self.grab = Some(match (on_button, on_switch) {
                (Some(b), _) => Grab::Button(b),
                (_, Some(s)) => Grab::Switch(s),
                _ => Grab::Turn,
            });
        }
        if matches!(self.grab, Some(Grab::Switch(_)))
            && response.dragged_by(egui::PointerButton::Primary)
        {
            self.grab = Some(Grab::Turn);
        }
        if let Some(Grab::Switch(s)) = self.grab {
            if clicked
                && self.switches_used[s]
                && hovered.and_then(|k| model.parts[k].sw) == Some(s)
            {
                self.switch_clicks.push(s);
            }
        }
        if !primary_down {
            self.grab = None;
        }
        self.pressed = match self.grab {
            Some(Grab::Button(b)) => Some(b),
            _ => None,
        };
        if self.grab == Some(Grab::Turn) && response.dragged_by(egui::PointerButton::Primary) {
            let d = response.drag_delta();
            self.camera.yaw -= d.x * 0.008;
            self.camera.pitch = (self.camera.pitch + d.y * 0.008).clamp(-1.5698, 1.5698);
        }
        if response.dragged_by(egui::PointerButton::Secondary) {
            let d = response.drag_delta();
            let (_, right, up) = self.camera.basis();
            let k = distance * (self.camera.fov_y / 2.0).tan() * 2.0 / rect.height();
            self.camera.target = board_model::add(
                self.camera.target,
                board_model::add(
                    board_model::scale(right, -d.x * k),
                    board_model::scale(up, d.y * k),
                ),
            );
        }
        if response.hovered() {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            if scroll != 0.0 || zoom != 1.0 {
                self.camera.zoom =
                    (self.camera.zoom * (-scroll * 0.0015).exp() / zoom).clamp(0.12, 4.0);
            }
        }
        if response.double_clicked()
            && hovered.is_none_or(|k| model.parts[k].btn.is_none() && model.parts[k].sw.is_none())
        {
            self.view("3d");
        }
        if let Some(k) = hovered.filter(|_| self.grab != Some(Grab::Turn)) {
            let p = &model.parts[k];
            let text = if p.object == 8 {
                "OLED1: SSD1331 96x64, as the design drives it".to_string()
            } else if p.object == 9 {
                format!(
                    "HDMI cable: the design drives GPDI ({})",
                    self.hdmi.as_ref().map_or("", |h| h.mode.as_str())
                )
            } else if let Some(s) = p.sw {
                if self.switches_used[s] {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                switch_label(s, self.switches_on[s], self.switches_used[s])
            } else if let Some(l) = p.led {
                let state = if !inputs.leds_used[l] {
                    ", not used by the design".to_string()
                } else {
                    match inputs.values[l] {
                        Some(true) => ": 1, lit".to_string(),
                        Some(false) => ": 0, dark".to_string(),
                        None => ": unknown (x), dark".to_string(),
                    }
                };
                format!(
                    "{} ({}): led[{l}] on {}{state}",
                    p.reference,
                    p.value,
                    ecp5_emu::board::LEDS[l]
                )
            } else if let Some(b) = p.btn {
                let button = &ecp5_emu::board::BUTTONS[b];
                if inputs.buttons_used[b] {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                }
                format!(
                    "{} {}: btn[{b}] on {}{}",
                    p.reference,
                    button.name,
                    button.ball,
                    if inputs.buttons_used[b] {
                        "; hold to press"
                    } else {
                        ", not used by the design"
                    }
                )
            } else {
                format!(
                    "{} {}{}",
                    p.reference,
                    p.value,
                    if p.side == "bottom" {
                        " (bottom side)"
                    } else {
                        ""
                    }
                )
            };
            response.clone().on_hover_text_at_pointer(text);
        }

        let distance = fit * self.camera.zoom;
        let uniforms = uniforms(&self.camera, distance, aspect);
        let vp = Mat4(uniforms.view_proj);

        // Draw again only when what is shown changed.
        let ppp = ui.ctx().pixels_per_point();
        if inputs.lit == self.shown {
            self.came = None;
        } else if self.came.is_none() {
            self.came = Some(self.frame);
        }
        let mut key: Vec<u32> = vp.0.iter().flatten().map(|v| v.to_bits()).collect();
        key.extend([
            rect.width().to_bits(),
            rect.height().to_bits(),
            ppp.to_bits(),
            inputs
                .lit
                .iter()
                .enumerate()
                .map(|(i, &l)| u32::from(l) << i)
                .sum(),
            inputs
                .held
                .iter()
                .enumerate()
                .map(|(i, &h)| u32::from(h) << i)
                .sum(),
            self.id as u32,
            u32::from(self.oled.is_some()),
            u32::from(self.hdmi.is_some()),
            u32::from(self.headers),
        ]);
        key.extend(self.switches_on.map(u32::from));
        key.extend(self.switches_used.map(u32::from));
        if let Some(oled) = &self.oled {
            key.extend([
                oled.serial as u32,
                (oled.serial >> 32) as u32,
                oled.width,
                oled.height,
            ]);
        }
        if self.pending.as_ref().is_none_or(|p| p.key != key)
            && (self.last_key.as_ref() != Some(&key) || self.pending.is_some())
        {
            self.number += 1;
            self.pending = Some(Arc::new(WantedRender {
                number: self.number,
                first: self.number,
                key,
                lit: inputs.lit,
            }));
            self.attempt_frame = Some(ui.ctx().cumulative_frame_nr());
        }
        // A submitted callback is not proof that the wanted LEDs reached the
        // picture. Give each retry its own drawing number, once per frame;
        // discarded passes reuse that number until a callback actually runs.
        if let Some(pending) = &self.pending {
            let frame = ui.ctx().cumulative_frame_nr();
            if self.submitted >= pending.number && self.attempt_frame != Some(frame) {
                self.number += 1;
                self.pending = Some(Arc::new(WantedRender {
                    number: self.number,
                    first: pending.first,
                    key: pending.key.clone(),
                    lit: pending.lit,
                }));
                self.attempt_frame = Some(frame);
            }
        }
        if self.pending.is_some()
            || (self.readback_supported == Some(true)
                && self.last_read.is_none_or(|r| r.render < self.submitted))
        {
            // Also poll the final readback while paused. No spin loop, and no
            // repaint request once the latest completed readback matches.
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
        let callback = BoardCallback {
            model: Some((self.id, Arc::clone(&model))),
            rect,
            uniforms,
            lit: inputs.lit,
            pressed: inputs.held,
            oled: self.oled.clone(),
            hdmi: self.hdmi.is_some(),
            headers: self.headers,
            switches: self.switches_on,
            render: self.pending.clone(),
            completed: Arc::clone(&self.completed),
            frame: self.frame,
            came: self.came,
            read: Arc::clone(&self.read),
            recording: self.recording,
        };
        #[cfg(test)]
        {
            self.callback = Some(callback.clone());
        }
        ui.painter()
            .add(egui_wgpu::Callback::new_paint_callback(rect, callback));
        if let Some(hdmi) = &self.hdmi {
            let project = |p| {
                let p = vp.point(p);
                (p[0].abs() < 0.95 && p[1].abs() < 0.9 && (0.0..1.0).contains(&p[2])).then(|| {
                    egui::pos2(
                        rect.center().x + p[0] * rect.width() / 2.0,
                        rect.center().y - p[1] * rect.height() / 2.0,
                    )
                })
            };
            let at = model.hdmi_end.and_then(project).or_else(|| {
                model
                    .parts
                    .iter()
                    .find(|p| p.name == "GPDI1")
                    .and_then(|p| p.inverse.inverse())
                    .and_then(|m| project(m.point([0.0; 3])))
            });
            if let Some(at) = at {
                let text = hdmi_label(&hdmi.mode);
                let galley = ui.painter().layout_no_wrap(
                    text,
                    egui::FontId::proportional(11.0),
                    egui::Color32::WHITE,
                );
                let pos = egui::pos2(
                    (at.x + 8.0)
                        .min(rect.right() - galley.size().x - 4.0)
                        .max(rect.left() + 4.0),
                    (at.y - galley.size().y - 6.0).max(rect.top() + 4.0),
                );
                ui.painter().rect_filled(
                    egui::Rect::from_min_size(pos, galley.size()).expand(3.0),
                    3.0,
                    egui::Color32::from_black_alpha(190),
                );
                ui.painter().galley(pos, galley, egui::Color32::WHITE);
            }
        }
        self.pressed
    }

    /// Layout may be discarded. Only a callback's acknowledgment commits its picture.
    fn finish_drawings(&mut self) {
        let completed = std::mem::take(&mut *self.completed.lock().unwrap());
        for CompletedRender {
            wanted,
            mut drawing,
            record,
            readback_supported,
        } in completed
        {
            self.last_key = Some(wanted.key.clone());
            self.shown = drawing.lit;
            self.submitted = self.submitted.max(wanted.number);
            self.readback_supported = Some(readback_supported);
            if self
                .pending
                .as_ref()
                .is_some_and(|p| p.number == wanted.number)
            {
                // Adapters without fragment storage cannot provide marks;
                // retain the callback acknowledgment fallback for those.
                if !readback_supported {
                    self.pending = None;
                }
                self.came = None;
            }
            if record {
                drawing.interval = drawing
                    .came
                    .and(self.changed_at)
                    .map(|b| drawing.at.saturating_duration_since(b).as_secs_f64() * 1e3);
                if drawing.came.is_some() {
                    self.changed_at = Some(drawing.at);
                }
                self.drawings.push_back(drawing);
                while self.drawings.len() > 512 {
                    self.drawings.pop_front();
                }
            }
        }
    }

    /// Sets what the GPU drew (read back) against what each drawing was to
    /// show, and writes it down.
    fn judge(&mut self) {
        let reads = std::mem::take(&mut *self.read.lock().unwrap());
        for read in reads {
            if self.last_read.is_none_or(|last| read.render > last.render) {
                self.last_read = Some(read);
            }
            self.judge_read(read);
        }
        if self.pending.as_ref().is_some_and(|p| {
            self.last_read
                .is_some_and(|r| r.render >= p.first && r.lit() == p.lit)
        }) {
            // OLED serials and switches are not fully read back. Only the LED
            // lit pattern closes this loop; the rest of the key still redraws
            // normally when changed.
            self.pending = None;
            self.came = None;
        }
    }

    fn judge_read(&mut self, read: BoardRead) {
        let Some(d) = self.drawings.iter().find(|d| d.number == read.render) else {
            return;
        };
        let bits = |f: &dyn Fn(usize) -> bool| -> String {
            (0..8).map(|i| if f(i) { '1' } else { '0' }).collect()
        };
        let drawn_lit = |i: usize| read.marks[i] == read.render;
        let drawn_dark = |i: usize| read.marks[8 + i] == read.render;
        let wrong = (0..8)
            .filter(|&i| (d.lit[i] && drawn_dark(i)) || (!d.lit[i] && drawn_lit(i)))
            .count();
        let unseen = (0..8).filter(|&i| !drawn_lit(i) && !drawn_dark(i)).count();
        self.checked += 1;
        self.wrong += wrong as u64;
        self.unseen += unseen as u64;
        // The frames between the window getting new LED values and drawing them.
        let gap = d.came.map(|c| d.frame - c);
        if gap.is_some_and(|g| g > 0) {
            self.late += 1;
        }
        let since = d.interval;
        // The options: asked for, and what the GPU drew of them in this drawing.
        let drawn = |slot: usize| read.marks[slot] == read.render;
        let (screen, screen_lit, cable, module) = (drawn(16), drawn(17), drawn(18), drawn(19));
        let oled_note = match d.oled {
            None => {
                if screen || module {
                    self.options_wrong += 1;
                    "OLED module not asked for, drawn: WRONG".to_string()
                } else {
                    "OLED module not asked for, not drawn".to_string()
                }
            }
            Some((serial, any_lit)) => {
                let lit_ok = !screen || screen_lit == any_lit;
                if !lit_ok {
                    self.options_wrong += 1;
                }
                format!(
                    "OLED module asked for (picture {serial}, {}): module {}, screen {}, lit pixels {}{}",
                    if any_lit { "with lit pixels" } else { "all dark" },
                    if module { "drawn" } else { "not in sight" },
                    if screen { "drawn" } else { "not in sight" },
                    if screen_lit { "drawn" } else { "none drawn" },
                    if lit_ok { "" } else { ": WRONG" },
                )
            }
        };
        let hdmi_note = match (d.hdmi, cable) {
            (true, true) => "HDMI cable asked for, drawn".to_string(),
            (true, false) => "HDMI cable asked for, not in sight".to_string(),
            (false, false) => "HDMI cable not asked for, not drawn".to_string(),
            (false, true) => {
                self.options_wrong += 1;
                "HDMI cable not asked for, drawn: WRONG".to_string()
            }
        };
        let t = self
            .ready
            .map_or(0.0, |r| d.at.saturating_duration_since(r).as_secs_f64());
        self.notes.push(format!(
            "board\t{t:.3}\tdrawing {}\tLEDs D0..D7 to show lit {}\tdrawn lit {}\tdrawn dark {}\t{}\t{}\ttotals: drawings checked {}, LEDs drawn wrong {}, not in sight {}, drawn frames after their values came {}",
            read.render,
            bits(&|i| d.lit[i]),
            bits(&drawn_lit),
            bits(&drawn_dark),
            if wrong == 0 { "ok".to_string() } else { format!("WRONG {wrong}") },
            gap.map_or("no LED change".to_string(), |g| match since {
                Some(ms) => format!("new LED values, drawn {g} frames after they came, {ms:.1} ms after the drawn values before"),
                None => format!("new LED values, drawn {g} frames after they came"),
            }),
            self.checked,
            self.wrong,
            self.unseen,
            self.late,
        ));
        if let Some(line) = self.notes.last_mut() {
            line.push_str(&format!(
                "\t{oled_note}\t{hdmi_note}\toptions drawn wrong {}",
                self.options_wrong
            ));
        }
        let keep = read.render;
        while self.drawings.front().is_some_and(|d| d.number <= keep) {
            self.drawings.pop_front();
        }
    }
}

#[allow(dead_code)]
fn _assert_layouts() {
    // The shader's structs are these sizes.
    const _: () = assert!(std::mem::size_of::<Uniforms>() == 64 + 16 * 7);
    const _: () = assert!(std::mem::size_of::<GpuMaterial>() == 48);
    const _: () = assert!(std::mem::size_of::<Glow>() == 32);
    let _ = Mat4::IDENTITY;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The layout/callback protocol needs no GPU or board asset.
    fn led_board() -> BoardView {
        let mut board = BoardView::with_load(Load::Ready(Arc::new(BoardModel {
            vertices: Vec::new(),
            indices: Vec::new(),
            opaque: 0..0,
            blended: 0..0,
            materials: Vec::new(),
            images: Vec::new(),
            leds: [None; 8],
            hdmi_end: None,
            switch_travel: [None; 4],
            outline: vec![[-7.0, 0.0, -4.0], [7.0, 2.0, 4.0]],
            parts: Vec::new(),
        })));
        board.camera = Camera::TOP;
        board.recording = true;
        board
    }

    fn led_frame(
        ctx: &egui::Context,
        board: &mut BoardView,
        led0: bool,
        discard: bool,
    ) -> Vec<BoardCallback> {
        let mut callbacks = Vec::new();
        let output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 500.0),
                )),
                ..Default::default()
            },
            |ui| {
                let lit = [led0, false, false, false, false, false, false, false];
                board.show(
                    ui,
                    &BoardInputs {
                        lit,
                        values: lit.map(Some),
                        leds_used: [true; 8],
                        buttons_used: [false; 7],
                        held: [false; 7],
                    },
                );
                callbacks.push(board.callback.take().unwrap());
                if discard && ctx.current_pass_index() == 0 {
                    ctx.request_discard("discard the only LED transition's layout pass");
                }
            },
        );
        assert_eq!(output.platform_output.num_completed_passes, callbacks.len());
        assert_eq!(callbacks.len(), if discard { 2 } else { 1 });
        callbacks
    }

    /// Stand in for prepare's command submission and `gpu.drawn` advancement.
    /// Only the surviving pass's callback is executed, as in egui's renderer.
    fn submit_picture(callback: &BoardCallback, drawn: &mut Option<u32>) -> bool {
        let Some(number) = callback.render_number(*drawn) else {
            return false;
        };
        *drawn = Some(number);
        callback.did_draw(true);
        true
    }

    fn read_picture(board: &mut BoardView, number: u32, lit: [bool; 8]) {
        let mut marks = [0; 32];
        for (i, on) in lit.into_iter().enumerate() {
            marks[if on { i } else { 8 + i }] = number;
        }
        board.read.lock().unwrap().push_back(BoardRead {
            render: number,
            marks,
        });
    }

    fn judge_picture(board: &mut BoardView, number: u32, lit: [bool; 8]) -> String {
        read_picture(board, number, lit);
        board.judge();
        let notes = board.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(!notes[0].contains("WRONG"), "{}", notes[0]);
        notes.into_iter().next().unwrap()
    }

    #[test]
    fn discarded_led_change_stays_pending_until_surviving_callback_draws() {
        let ctx = egui::Context::default();
        let mut board = led_board();
        let mut drawn = None;
        let initial = led_frame(&ctx, &mut board, false, false).pop().unwrap();
        assert!(
            board.last_key.is_none(),
            "layout is not a completed drawing"
        );
        assert!(board.drawings.is_empty());
        assert!(submit_picture(&initial, &mut drawn));
        board.finish_drawings();
        judge_picture(&mut board, drawn.unwrap(), [false; 8]);
        let dark_key = board.last_key.clone();

        let passes = led_frame(&ctx, &mut board, true, true);
        let wanted = passes[0].render_number(drawn).unwrap();
        assert_eq!(passes[1].render_number(drawn), Some(wanted));
        assert_eq!(board.last_key, dark_key);
        assert_eq!(board.shown, [false; 8]);
        assert!(
            board.drawings.is_empty(),
            "discarded layouts are not recorded"
        );
        assert_eq!(board.pending.as_ref().unwrap().number, wanted);

        assert!(submit_picture(&passes[1], &mut drawn));
        // A duplicate callback for this number cannot draw or record it twice.
        assert!(!submit_picture(&passes[1], &mut drawn));
        board.finish_drawings();
        assert!(board.pending.is_some(), "submission still needs a readback");
        assert_ne!(board.last_key, dark_key);
        assert!(board.shown[0]);
        assert_eq!(board.drawings.len(), 1);
        let drawing = &board.drawings[0];
        assert_eq!(drawing.frame, passes[1].frame);
        assert_eq!(drawing.came, Some(passes[0].frame));
        let shown = board.shown;
        let note = judge_picture(&mut board, wanted, shown);
        assert!(board.pending.is_none());
        assert!(note.contains("to show lit 10000000\tdrawn lit 10000000"));
        assert!(note.contains("drawn 1 frames after they came"));

        for _ in 0..4 {
            let callback = led_frame(&ctx, &mut board, true, false).pop().unwrap();
            assert!(callback.render.is_none());
            assert!(!submit_picture(&callback, &mut drawn));
        }
        assert!(board.drawings.is_empty());
        assert!(board.take_notes().is_empty());
    }

    #[test]
    fn constant_led_after_reset_survives_frames_without_a_paint_callback() {
        let ctx = egui::Context::default();
        let mut board = led_board();
        let mut drawn = None;
        // Reset's dark picture is requested but never painted. Then the design
        // drives `assign led[0] = !reset` and stays high forever.
        let reset = led_frame(&ctx, &mut board, false, false).pop().unwrap();
        let reset_number = reset.render_number(drawn).unwrap();
        let mut constant = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        let wanted = constant.render_number(drawn).unwrap();
        assert_ne!(wanted, reset_number);
        for _ in 0..3 {
            constant = led_frame(&ctx, &mut board, true, false).pop().unwrap();
            assert_eq!(constant.render_number(drawn), Some(wanted));
            assert!(board.last_key.is_none());
            assert!(!board.shown[0]);
            assert!(board.drawings.is_empty());
        }
        assert!(submit_picture(&constant, &mut drawn));
        board.finish_drawings();
        assert!(board.pending.is_some(), "submission still needs a readback");
        assert!(board.shown[0]);
        assert_eq!(
            board.drawings.len(),
            1,
            "only the painted picture is recorded"
        );
        assert_eq!(board.drawings[0].number, wanted);
        let shown = board.shown;
        let note = judge_picture(&mut board, wanted, shown);
        assert!(board.pending.is_none());
        assert!(note.contains("to show lit 10000000\tdrawn lit 10000000"));
        for _ in 0..8 {
            let callback = led_frame(&ctx, &mut board, true, false).pop().unwrap();
            assert!(callback.render.is_none());
            assert!(!submit_picture(&callback, &mut drawn));
        }
        assert_eq!(drawn, Some(wanted));
        assert!(board.drawings.is_empty());
    }

    #[test]
    fn lost_callback_and_stale_readbacks_retry_until_leds_match() {
        let ctx = egui::Context::default();
        let mut board = led_board();
        let mut drawn = None;
        let dark = led_frame(&ctx, &mut board, false, false).pop().unwrap();
        assert!(submit_picture(&dark, &mut drawn));
        board.finish_drawings();
        let dark_number = drawn.unwrap();
        judge_picture(&mut board, dark_number, [false; 8]);

        // Lose the sole rising edge's callback. Stable inputs still request it
        // on every following frame, without inventing completed drawings.
        let lost = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        let wanted = lost.render_number(drawn).unwrap();
        for _ in 0..3 {
            let callback = led_frame(&ctx, &mut board, true, false).pop().unwrap();
            assert_eq!(callback.render_number(drawn), Some(wanted));
            assert!(board.drawings.is_empty());
        }
        let passes = led_frame(&ctx, &mut board, true, true);
        assert_eq!(passes[0].render_number(drawn), Some(wanted));
        assert_eq!(passes[1].render_number(drawn), Some(wanted));
        assert!(submit_picture(&passes[1], &mut drawn));
        board.finish_drawings();

        // An old dark readback does not acknowledge this wanted picture or
        // write a duplicate # board line for the already completed drawing.
        read_picture(&mut board, dark_number, [false; 8]);
        board.judge();
        assert!(board.pending.is_some());
        assert!(board.take_notes().is_empty());
        let retry = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        let retry_number = retry.render_number(drawn).unwrap();
        assert!(
            retry_number > wanted,
            "retry bypasses GPU submission caching"
        );
        assert!(submit_picture(&retry, &mut drawn));
        assert!(!submit_picture(&retry, &mut drawn), "one paint per frame");
        board.finish_drawings();

        // Even a callback claiming completion is insufficient: its actual
        // readback can still say dark. Record that mismatch honestly and retry.
        read_picture(&mut board, retry_number, [false; 8]);
        board.judge();
        let notes = board.take_notes();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].contains("to show lit 10000000\tdrawn lit 00000000"));
        assert!(notes[0].contains("WRONG 1"));
        assert!(board.pending.is_some());
        let repaired = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        let repaired_number = repaired.render_number(drawn).unwrap();
        assert!(repaired_number > retry_number);
        assert!(submit_picture(&repaired, &mut drawn));
        board.finish_drawings();
        let shown = board.shown;
        judge_picture(&mut board, repaired_number, shown);
        assert!(board.pending.is_none());
        // A delayed older readback cannot overwrite the newer confirmation.
        read_picture(&mut board, wanted, [false; 8]);
        board.judge();
        for _ in 0..5 {
            let matched = led_frame(&ctx, &mut board, true, false).pop().unwrap();
            assert!(matched.render.is_none());
            assert!(!submit_picture(&matched, &mut drawn));
        }
        assert!(board.take_notes().is_empty());
    }

    #[test]
    fn led_feedback_also_runs_without_recording() {
        let ctx = egui::Context::default();
        let mut board = led_board();
        board.recording = false;
        let mut drawn = None;
        let requested = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        assert!(submit_picture(&requested, &mut drawn));
        board.finish_drawings();
        assert!(board.pending.is_some());
        read_picture(&mut board, drawn.unwrap(), [false; 8]);
        board.judge();
        let retry = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        assert!(submit_picture(&retry, &mut drawn));
        board.finish_drawings();
        let shown = board.shown;
        read_picture(&mut board, drawn.unwrap(), shown);
        board.judge();
        let matched = led_frame(&ctx, &mut board, true, false).pop().unwrap();
        assert!(matched.render.is_none());
        assert!(!submit_picture(&matched, &mut drawn));
        assert!(board.drawings.is_empty());
        assert!(board.take_notes().is_empty());
    }

    #[test]
    fn board_model_load_errors_have_context() {
        for message in [
            "3D board model not found; showing the flat board. Run `git lfs pull` or see tools/ulx3s_3d.",
            "3D board model is a git LFS pointer; run `git lfs pull`.",
        ] {
            assert_eq!(model_load_error(&anyhow::anyhow!(message)), message);
        }
        assert_eq!(
            model_load_error(&anyhow::anyhow!("not a glTF 2 binary")),
            "3D board model: not a glTF 2 binary"
        );
        let unreadable = anyhow::anyhow!("permission denied").context("reading board.glb");
        assert_eq!(
            model_load_error(&unreadable),
            "3D board model: reading board.glb: permission denied"
        );
    }

    #[test]
    fn board_shaders_with_and_without_led_marks() {
        // Validate sampling uniformity and resource types without a GPU adapter.
        for source in [
            SHADER.replace("//MARKS", ""),
            SHADER.to_string(),
            BLIT.to_string(),
        ] {
            let module = wgpu::naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
            wgpu::naga::valid::Validator::new(
                wgpu::naga::valid::ValidationFlags::all(),
                wgpu::naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap();
        }
    }

    /// Every character the board writes in the window is drawn with a glyph
    /// of its own in the window's fonts, the HDMI mode's "×" included.
    #[test]
    fn board_words_have_glyphs() {
        let ctx = egui::Context::default();
        // The window's own theme and fonts, not egui's defaults.
        crate::theme::install(&ctx, egui::ThemePreference::Dark);
        let _ = ctx.run_ui(egui::RawInput::default(), |_| {});
        let fonts = [
            egui::TextStyle::Small.resolve(&ctx.global_style()),
            egui::TextStyle::Body.resolve(&ctx.global_style()),
            egui::FontId::proportional(11.0),
        ];
        let words = [
            VIEW_HINT.to_string(),
            LOADING.to_string(),
            hdmi_label("640×480 59.52 Hz"),
            switch_label(0, true, true),
            switch_label(1, false, false),
            switch_label(2, false, true),
            switch_label(3, true, false),
        ];
        let missing: Vec<(String, char)> = words
            .iter()
            .flat_map(|w| w.chars().map(move |c| (w.clone(), c)))
            .filter(|(_, c)| !fonts.iter().all(|font| crate::theme::draws(&ctx, font, *c)))
            .collect();
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    #[test]
    fn board_switch_picture_offsets_and_words() {
        assert_eq!(picture_switches("1010"), [true, false, true, false]);
        assert_eq!(picture_switches("0101"), [false, true, false, true]);
        for invalid in ["", "101", "10101", "10x0", "é10"] {
            assert_eq!(picture_switches(invalid), [false; 4]);
        }
        let travel = [Some([0.0, 0.0, -1.7]); 4];
        let offsets = object_offsets([false; 7], [false; 3], picture_switches("1010"), travel);
        assert_eq!(offsets[11], [0.0, 0.0, -1.7, 0.0]);
        assert_eq!(offsets[12], [0.0; 4]);
        assert_eq!(offsets[13], [0.0, 0.0, -1.7, 0.0]);
        assert_eq!(offsets[14], [0.0; 4]);
        assert_eq!(
            switch_label(0, true, true),
            "SW1 switch 1: sw[0], ON; click to toggle"
        );
        assert_eq!(
            switch_label(3, false, false),
            "SW1 switch 4: sw[3], OFF, not read by the design"
        );
    }

    #[test]
    fn board_switch_clicks_usage_drags_and_double_clicks() {
        let ctx = egui::Context::default();
        let mut board = BoardView::with_load(Load::Ready(Arc::new(BoardModel {
            vertices: Vec::new(),
            indices: Vec::new(),
            opaque: 0..0,
            blended: 0..0,
            materials: Vec::new(),
            images: Vec::new(),
            leds: [None; 8],
            hdmi_end: None,
            switch_travel: [Some([0.0, 0.0, -1.7]); 4],
            outline: (0..8)
                .map(|k| {
                    [
                        if k & 1 == 0 { -7.0 } else { 7.0 },
                        if k & 2 == 0 { 0.0 } else { 2.0 },
                        if k & 4 == 0 { -4.0 } else { 4.0 },
                    ]
                })
                .collect(),
            parts: (0..4)
                .map(|s| board_model::Part {
                    name: format!("SW1_slider{}", s + 1),
                    reference: "SW1".into(),
                    value: String::new(),
                    side: "top".into(),
                    btn: None,
                    sw: Some(s),
                    led: None,
                    object: 11 + s as u32,
                    inverse: Mat4::IDENTITY,
                    min: [(s as f32 - 1.5) * 3.0 - 0.5, 0.0, -1.3],
                    max: [(s as f32 - 1.5) * 3.0 + 0.5, 1.0, 1.3],
                })
                .collect(),
        })));
        board.camera = Camera::TOP;
        assert_eq!(board.switches_used, [false; 4]);
        assert_eq!(board.switches_on, [false; 4]);
        assert!(board.take_switch_clicks().is_empty());
        board.set_switches([true, false, true, false], [false, true, false, true]);
        let frame = |board: &mut BoardView, time, events| {
            ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(600.0, 400.0),
                    )),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let inputs = BoardInputs {
                        lit: [false; 8],
                        values: [None; 8],
                        leds_used: [false; 8],
                        buttons_used: [false; 7],
                        held: [false; 7],
                    };
                    board.show(ui, &inputs);
                },
            )
        };
        frame(&mut board, 0.0, Vec::new());
        let rect = board.rect.unwrap();
        let model = match &board.load {
            Load::Ready(m) => m,
            _ => unreachable!(),
        };
        let aspect = rect.width() / rect.height();
        let distance = board.camera.fit(&model.outline, aspect) * board.camera.zoom;
        let matrix = Mat4(uniforms(&board.camera, distance, aspect).view_proj);
        let positions: [egui::Pos2; 4] = std::array::from_fn(|s| {
            let p = matrix.point([(s as f32 - 1.5) * 3.0, 1.0, 0.0]);
            egui::pos2(
                rect.center().x + p[0] * rect.width() / 2.0,
                rect.center().y - p[1] * rect.height() / 2.0,
            )
        });
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        for (n, s) in [2, 0, 1].into_iter().enumerate() {
            let pos = positions[s];
            let time = 1.0 + n as f64;
            frame(
                &mut board,
                time,
                vec![egui::Event::PointerMoved(pos), button(pos, true)],
            );
            assert!(board.pressed.is_none(), "a lever never holds a button");
            let out = frame(&mut board, time + 0.01, vec![button(pos, false)]);
            if s == 1 {
                assert_ne!(
                    out.platform_output.cursor_icon,
                    egui::CursorIcon::PointingHand
                );
            }
        }
        assert_eq!(board.take_switch_clicks(), vec![2, 0]);
        assert!(board.take_switch_clicks().is_empty());
        assert_eq!(
            board.switches_on,
            [false, true, false, true],
            "clicks wait for reported state"
        );

        // A fast press/release in one frame still counts, and a double click never resets TOP.
        let pos = positions[0];
        for time in [4.0, 4.1] {
            frame(
                &mut board,
                time,
                vec![
                    egui::Event::PointerMoved(pos),
                    button(pos, true),
                    button(pos, false),
                ],
            );
        }
        assert_eq!(board.take_switch_clicks(), vec![0, 0]);
        assert_eq!(board.camera.yaw, Camera::TOP.yaw);
        assert_eq!(board.camera.pitch, Camera::TOP.pitch);

        let key = board.pending.as_ref().map(|p| p.key.clone());
        board.set_switches([true; 4], board.switches_on);
        frame(&mut board, 5.0, Vec::new());
        assert_ne!(
            board.pending.as_ref().map(|p| p.key.clone()),
            key,
            "usage changes request a redraw too"
        );
        let pos = positions[2];
        let yaw = board.camera.yaw;
        frame(
            &mut board,
            6.0,
            vec![egui::Event::PointerMoved(pos), button(pos, true)],
        );
        let moved = pos + egui::vec2(45.0, 20.0);
        frame(&mut board, 6.1, vec![egui::Event::PointerMoved(moved)]);
        frame(&mut board, 6.2, vec![button(moved, false)]);
        assert_ne!(board.camera.yaw, yaw, "a lever drag turns the board");
        assert!(
            board.take_switch_clicks().is_empty(),
            "a drag never toggles"
        );
    }

    #[test]
    fn board_oled_test_pattern_is_pins_up() {
        let image = oled_test_picture();
        assert!(image.valid(4096));
        let pixel = |x: usize, y: usize| &image.rgba[(y * 96 + x) * 4..(y * 96 + x) * 4 + 4];
        for x in 0..96 {
            assert_eq!(pixel(x, 0), [255; 4]);
            assert_eq!(pixel(x, 63), [255; 4]);
        }
        for y in 0..64 {
            assert_eq!(pixel(0, y), [255; 4]);
            assert_eq!(pixel(95, y), [255; 4]);
        }
        assert_eq!(pixel(15, 8), [255, 255, 0, 255]);
        assert_eq!(pixel(80, 8), [0, 0, 255, 255]);
        // U: open at the top, rounded bottom, with black space around it.
        assert_eq!(pixel(23, 42), [0, 0, 0, 255]);
        assert_eq!(pixel(23, 54), [255; 4]);
        assert_eq!(pixel(18, 42), [0, 0, 0, 255]);
    }
}

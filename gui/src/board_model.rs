//! The ULX3S in 3D, on the CPU: the board model that `tools/ulx3s_3d` builds
//! from the board's KiCad design and exports from Blender as glTF binary
//! (`out/ulx3s_3d/ulx3s_v3.0.8.glb`), read into one vertex list in board
//! millimetres, with the parts the panel can point at, and the camera that
//! looks at it.
//!
//! The file's frame is glTF's: x to the right along the board, y up from it
//! (the top side faces +y), z toward the board's bottom edge (the SD slot).
//! Each part is a node named by its reference (U1, D0, B3, ...) whose glTF
//! `extras` carry `ref`, `value`, `side` and, for the board's I/O,
//! `led_index` (D0..D7 = `led[0..7]`) or `btn_index` (B0..B6 = `btn[0..6]`).
//! A button's cap is its child node `B<n>_plunger`; each LED's lens has a
//! material of its own, `LED_D<n>`.
//! Optional OLED/HDMI/header nodes inherit `extras.option` through their children:
//! objects 8/9/10 are hidden and excluded from picking until the view shows them.
//! `OLED_screen` uses image slot 3, with glTF UV (0,0) at the header's GND end.
//! SW1 levers carry `sw_index` (0..3, from the row's SW1..SW4 net) and
//! `sw_travel` (OFF -> ON, already in board glTF millimetres). Objects 11..14
//! move the levers and their children; their pick boxes cover the full slot.
use anyhow::{bail, Context, Result};
use bytemuck::{Pod, Zeroable};
use serde_json::Value;
use std::path::{Path, PathBuf};

pub type V3 = [f32; 3];

/// Look first at `$ECP5_BOARD_MODEL` (also set by package launchers), then a
/// local rebuild in the repository's `out/ulx3s_3d/ulx3s_v3.0.8.glb`, then the
/// shipped git LFS model in `assets/ulx3s_3d/ulx3s_v3.0.8.glb`.
pub fn default_path() -> PathBuf {
    default_path_in(
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap(),
        std::env::var_os("ECP5_BOARD_MODEL").map(PathBuf::from),
    )
}

fn default_path_in(root: &Path, override_path: Option<PathBuf>) -> PathBuf {
    override_path.unwrap_or_else(|| {
        let rebuilt = root.join("out/ulx3s_3d/ulx3s_v3.0.8.glb");
        if rebuilt.is_file() {
            rebuilt
        } else {
            root.join("assets/ulx3s_3d/ulx3s_v3.0.8.glb")
        }
    })
}

/// A vertex in board millimetres. `object` 0 never moves; `1 + n` is the cap
/// of button `n`, moved down while it is pressed; 8/9/10 are the OLED/HDMI/header options.
/// Objects 11..14 are SW1 levers, translated from OFF to ON by `switch_travel`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: V3,
    pub normal: V3,
    pub uv: [f32; 2],
    pub material: u32,
    pub object: u32,
}

pub struct Material {
    /// Linear RGBA.
    pub base: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Images: 0 none, 1 board top, 2 board bottom, 3 live OLED screen.
    pub textures: u32,
    /// Drawn over the rest, see-through (the DIP switch's tape).
    pub blended: bool,
    /// `led[n]`'s lens.
    pub led: Option<usize>,
}

/// An image with its mip levels, RGBA8; the colour ones are sRGB.
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub levels: Vec<Vec<u8>>,
    pub srgb: bool,
}

/// A part the pointer can be on: a node with a reference, its box in its own
/// frame and the frame.
pub struct Part {
    pub name: String,
    pub reference: String,
    pub value: String,
    pub side: String,
    pub btn: Option<usize>,
    pub sw: Option<usize>,
    pub led: Option<usize>,
    pub object: u32,
    pub inverse: Mat4,
    pub min: V3,
    pub max: V3,
}

pub struct BoardModel {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// The index ranges of the opaque triangles and of the see-through ones.
    pub opaque: std::ops::Range<u32>,
    pub blended: std::ops::Range<u32>,
    pub materials: Vec<Material>,
    /// The board's images: top colour, top roughness/metal, bottom colour,
    /// bottom roughness/metal (glTF's metallicRoughness: G rough, B metal).
    pub images: Vec<Image>,
    pub parts: Vec<Part>,
    /// The top of each of `led[0..7]`'s lenses, for its glow.
    pub leds: [Option<V3>; 8],
    /// The fitted board's box corners, excluding options: what the view fits
    /// to, so fitting/removing a cable does not change the board's framing.
    pub outline: Vec<V3>,
    /// Cable's far end, in board millimetres, for the HDMI label.
    pub hdmi_end: Option<V3>,
    /// OFF -> ON vectors in board glTF millimetres, indexed by sw[0..3].
    pub switch_travel: [Option<V3>; 4],
}

// --- small linear algebra (column-major, as WGSL's mat4x4) -------------------

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub const IDENTITY: Mat4 = Mat4([
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ]);

    pub fn mul(&self, o: &Mat4) -> Mat4 {
        let mut m = [[0.0; 4]; 4];
        for (c, col) in m.iter_mut().enumerate() {
            for (r, v) in col.iter_mut().enumerate() {
                *v = (0..4).map(|k| self.0[k][r] * o.0[c][k]).sum();
            }
        }
        Mat4(m)
    }

    pub fn point(&self, p: V3) -> V3 {
        let m = &self.0;
        let w = m[0][3] * p[0] + m[1][3] * p[1] + m[2][3] * p[2] + m[3][3];
        let w = if w.abs() < 1e-12 { 1.0 } else { w };
        [
            (m[0][0] * p[0] + m[1][0] * p[1] + m[2][0] * p[2] + m[3][0]) / w,
            (m[0][1] * p[0] + m[1][1] * p[1] + m[2][1] * p[2] + m[3][1]) / w,
            (m[0][2] * p[0] + m[1][2] * p[1] + m[2][2] * p[2] + m[3][2]) / w,
        ]
    }

    pub fn dir(&self, d: V3) -> V3 {
        let m = &self.0;
        [
            m[0][0] * d[0] + m[1][0] * d[1] + m[2][0] * d[2],
            m[0][1] * d[0] + m[1][1] * d[1] + m[2][1] * d[2],
            m[0][2] * d[0] + m[1][2] * d[1] + m[2][2] * d[2],
        ]
    }

    /// From glTF's translation, rotation (x, y, z, w) and scale.
    pub fn trs(t: V3, q: [f32; 4], s: V3) -> Mat4 {
        let [x, y, z, w] = q;
        let r = [
            [
                1.0 - 2.0 * (y * y + z * z),
                2.0 * (x * y + z * w),
                2.0 * (x * z - y * w),
            ],
            [
                2.0 * (x * y - z * w),
                1.0 - 2.0 * (x * x + z * z),
                2.0 * (y * z + x * w),
            ],
            [
                2.0 * (x * z + y * w),
                2.0 * (y * z - x * w),
                1.0 - 2.0 * (x * x + y * y),
            ],
        ];
        Mat4([
            [r[0][0] * s[0], r[0][1] * s[0], r[0][2] * s[0], 0.0],
            [r[1][0] * s[1], r[1][1] * s[1], r[1][2] * s[1], 0.0],
            [r[2][0] * s[2], r[2][1] * s[2], r[2][2] * s[2], 0.0],
            [t[0], t[1], t[2], 1.0],
        ])
    }

    pub fn inverse(&self) -> Option<Mat4> {
        let m: Vec<f32> = self.0.iter().flatten().copied().collect();
        let mut inv = [0.0f32; 16];
        inv[0] = m[5] * m[10] * m[15] - m[5] * m[11] * m[14] - m[9] * m[6] * m[15]
            + m[9] * m[7] * m[14]
            + m[13] * m[6] * m[11]
            - m[13] * m[7] * m[10];
        inv[4] = -m[4] * m[10] * m[15] + m[4] * m[11] * m[14] + m[8] * m[6] * m[15]
            - m[8] * m[7] * m[14]
            - m[12] * m[6] * m[11]
            + m[12] * m[7] * m[10];
        inv[8] = m[4] * m[9] * m[15] - m[4] * m[11] * m[13] - m[8] * m[5] * m[15]
            + m[8] * m[7] * m[13]
            + m[12] * m[5] * m[11]
            - m[12] * m[7] * m[9];
        inv[12] = -m[4] * m[9] * m[14] + m[4] * m[10] * m[13] + m[8] * m[5] * m[14]
            - m[8] * m[6] * m[13]
            - m[12] * m[5] * m[10]
            + m[12] * m[6] * m[9];
        inv[1] = -m[1] * m[10] * m[15] + m[1] * m[11] * m[14] + m[9] * m[2] * m[15]
            - m[9] * m[3] * m[14]
            - m[13] * m[2] * m[11]
            + m[13] * m[3] * m[10];
        inv[5] = m[0] * m[10] * m[15] - m[0] * m[11] * m[14] - m[8] * m[2] * m[15]
            + m[8] * m[3] * m[14]
            + m[12] * m[2] * m[11]
            - m[12] * m[3] * m[10];
        inv[9] = -m[0] * m[9] * m[15] + m[0] * m[11] * m[13] + m[8] * m[1] * m[15]
            - m[8] * m[3] * m[13]
            - m[12] * m[1] * m[11]
            + m[12] * m[3] * m[9];
        inv[13] = m[0] * m[9] * m[14] - m[0] * m[10] * m[13] - m[8] * m[1] * m[14]
            + m[8] * m[2] * m[13]
            + m[12] * m[1] * m[10]
            - m[12] * m[2] * m[9];
        inv[2] = m[1] * m[6] * m[15] - m[1] * m[7] * m[14] - m[5] * m[2] * m[15]
            + m[5] * m[3] * m[14]
            + m[13] * m[2] * m[7]
            - m[13] * m[3] * m[6];
        inv[6] = -m[0] * m[6] * m[15] + m[0] * m[7] * m[14] + m[4] * m[2] * m[15]
            - m[4] * m[3] * m[14]
            - m[12] * m[2] * m[7]
            + m[12] * m[3] * m[6];
        inv[10] = m[0] * m[5] * m[15] - m[0] * m[7] * m[13] - m[4] * m[1] * m[15]
            + m[4] * m[3] * m[13]
            + m[12] * m[1] * m[7]
            - m[12] * m[3] * m[5];
        inv[14] = -m[0] * m[5] * m[14] + m[0] * m[6] * m[13] + m[4] * m[1] * m[14]
            - m[4] * m[2] * m[13]
            - m[12] * m[1] * m[6]
            + m[12] * m[2] * m[5];
        inv[3] = -m[1] * m[6] * m[11] + m[1] * m[7] * m[10] + m[5] * m[2] * m[11]
            - m[5] * m[3] * m[10]
            - m[9] * m[2] * m[7]
            + m[9] * m[3] * m[6];
        inv[7] = m[0] * m[6] * m[11] - m[0] * m[7] * m[10] - m[4] * m[2] * m[11]
            + m[4] * m[3] * m[10]
            + m[8] * m[2] * m[7]
            - m[8] * m[3] * m[6];
        inv[11] = -m[0] * m[5] * m[11] + m[0] * m[7] * m[9] + m[4] * m[1] * m[11]
            - m[4] * m[3] * m[9]
            - m[8] * m[1] * m[7]
            + m[8] * m[3] * m[5];
        inv[15] = m[0] * m[5] * m[10] - m[0] * m[6] * m[9] - m[4] * m[1] * m[10]
            + m[4] * m[2] * m[9]
            + m[8] * m[1] * m[6]
            - m[8] * m[2] * m[5];
        let det = m[0] * inv[0] + m[1] * inv[4] + m[2] * inv[8] + m[3] * inv[12];
        if det.abs() < 1e-12 {
            return None;
        }
        let mut out = [[0.0; 4]; 4];
        for (i, v) in inv.iter().enumerate() {
            out[i / 4][i % 4] = v / det;
        }
        Some(Mat4(out))
    }
}

pub fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, k: f32) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
pub fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn normalize(a: V3) -> V3 {
    let l = dot(a, a).sqrt();
    if l < 1e-12 {
        a
    } else {
        scale(a, 1.0 / l)
    }
}

// --- camera ------------------------------------------------------------------

/// An orbit around the board: yaw about +y from +z toward +x, pitch above the
/// board (negative: from below), the distance a share of the one that fits
/// the board in the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub zoom: f32,
    pub target: V3,
    pub fov_y: f32,
}

impl Camera {
    /// About where the radiona.org photo of the board was taken from.
    pub const PHOTO: Camera = Camera {
        yaw: -0.496,
        pitch: 0.977,
        zoom: 1.0,
        target: [0.0, 0.0, 0.0],
        fov_y: 0.42,
    };
    pub const TOP: Camera = Camera {
        yaw: 0.0,
        pitch: 1.5698,
        zoom: 1.0,
        target: [0.0, 0.0, 0.0],
        fov_y: 0.42,
    };
    /// From below, turned so the board reads as on its bottom photo.
    pub const BOTTOM: Camera = Camera {
        yaw: std::f32::consts::PI,
        pitch: -1.5698,
        zoom: 1.0,
        target: [0.0, 0.0, 0.0],
        fov_y: 0.42,
    };

    /// The way from the target to the eye, and the view's right and up.
    pub fn basis(&self) -> (V3, V3, V3) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let back = [cp * sy, sp, cp * cy];
        let right = [cy, 0.0, -sy];
        let up = cross(right, scale(back, -1.0));
        (back, right, up)
    }

    pub fn eye(&self, distance: f32) -> V3 {
        add(self.target, scale(self.basis().0, distance))
    }

    pub fn view_proj(&self, distance: f32, aspect: f32) -> Mat4 {
        let (back, right, up) = self.basis();
        let eye = self.eye(distance);
        let view = Mat4([
            [right[0], up[0], back[0], 0.0],
            [right[1], up[1], back[1], 0.0],
            [right[2], up[2], back[2], 0.0],
            [-dot(right, eye), -dot(up, eye), -dot(back, eye), 1.0],
        ]);
        let near = (distance * 0.2).max(1.0);
        let far = distance * 4.0 + 200.0;
        let f = 1.0 / (self.fov_y / 2.0).tan();
        let proj = Mat4([
            [f / aspect, 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, near * far / (near - far), 0.0],
        ]);
        proj.mul(&view)
    }

    /// The ray through a point of the view, as shares from its middle
    /// (x right, y up, each -1..1).
    pub fn ray(&self, distance: f32, aspect: f32, at: [f32; 2]) -> (V3, V3) {
        let (back, right, up) = self.basis();
        let t = (self.fov_y / 2.0).tan();
        let d = add(
            add(scale(back, -1.0), scale(right, at[0] * t * aspect)),
            scale(up, at[1] * t),
        );
        (self.eye(distance), normalize(d))
    }

    /// The distance at which `points` fill the view at `aspect`, with a margin.
    pub fn fit(&self, points: &[V3], aspect: f32) -> f32 {
        let corners = points;
        let fits = |d: f32| {
            let m = self.view_proj(d, aspect);
            corners.iter().all(|&c| {
                let p = m.point(c);
                p[0].abs() <= 0.96 && p[1].abs() <= 0.94
            })
        };
        let (mut lo, mut hi) = (10.0f32, 5000.0f32);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if fits(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        hi
    }
}

impl BoardModel {
    /// The part a ray meets first, and how far along it. `options` shows OLED/HDMI/headers.
    pub fn pick(&self, origin: V3, dir: V3, options: [bool; 3]) -> Option<(usize, f32)> {
        let mut hits = Vec::new();
        for (k, part) in self.parts.iter().enumerate() {
            if (8..=10).contains(&part.object) && !options[(part.object - 8) as usize] {
                continue;
            }
            let o = part.inverse.point(origin);
            let d = part.inverse.dir(dir);
            // Small parts get a little room around them.
            let grow = 0.2;
            let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
            let mut hit = true;
            for a in 0..3 {
                let (lo, hi) = (part.min[a] - grow, part.max[a] + grow);
                if d[a].abs() < 1e-9 {
                    if o[a] < lo || o[a] > hi {
                        hit = false;
                        break;
                    }
                } else {
                    let (mut ta, mut tb) = ((lo - o[a]) / d[a], (hi - o[a]) / d[a]);
                    if ta > tb {
                        std::mem::swap(&mut ta, &mut tb);
                    }
                    t0 = t0.max(ta);
                    t1 = t1.min(tb);
                    if t0 > t1 {
                        hit = false;
                        break;
                    }
                }
            }
            if hit {
                hits.push((k, t0));
            }
        }
        hits.iter()
            .copied()
            .filter(|&(k, _)| {
                let p = &self.parts[k];
                // SW1's Kapton/body box encloses the lever slots. A slot hit selects
                // its lever through that assembly's cover, but never through another part.
                p.sw.is_some()
                    || !hits.iter().any(|&(other, _)| {
                        self.parts[other].sw.is_some() && self.parts[other].reference == p.reference
                    })
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
    }
}

// --- glTF binary -------------------------------------------------------------

pub fn load(path: &Path) -> Result<BoardModel> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    parse(&bytes, 4096)
}

struct Glb<'a> {
    json: Value,
    bin: &'a [u8],
}

fn glb(bytes: &[u8]) -> Result<Glb<'_>> {
    if is_lfs_pointer(bytes) {
        bail!("3D board model is a git LFS pointer; run `git lfs pull`.");
    }
    let word = |at: usize| -> Result<u32> {
        Ok(u32::from_le_bytes(
            bytes.get(at..at + 4).context("glb cut short")?.try_into()?,
        ))
    };
    if bytes.get(0..4) != Some(b"glTF") || word(4)? != 2 {
        bail!("not a glTF 2 binary");
    }
    let json_len = word(12)? as usize;
    if word(16)? != 0x4E4F534A {
        bail!("glb: the first chunk is not JSON");
    }
    let json: Value =
        serde_json::from_slice(bytes.get(20..20 + json_len).context("glb JSON cut short")?)?;
    let at = 20 + json_len;
    let bin = if at + 8 <= bytes.len() {
        let len = word(at)? as usize;
        bytes
            .get(at + 8..at + 8 + len)
            .context("glb BIN cut short")?
    } else {
        &[]
    };
    Ok(Glb { json, bin })
}

fn is_lfs_pointer(bytes: &[u8]) -> bool {
    bytes.starts_with(b"version https://git-lfs.github.com/spec/v1")
}

impl Glb<'_> {
    fn view(&self, index: usize) -> Result<&[u8]> {
        let v = &self.json["bufferViews"][index];
        let at = v["byteOffset"].as_u64().unwrap_or(0) as usize;
        let len = v["byteLength"]
            .as_u64()
            .context("bufferView without byteLength")? as usize;
        self.bin
            .get(at..at + len)
            .context("bufferView out of the BIN chunk")
    }

    /// An accessor's values as floats (or integers as floats), `n` a element.
    fn floats(&self, index: usize) -> Result<(Vec<f32>, usize)> {
        let a = &self.json["accessors"][index];
        let n = match a["type"].as_str().unwrap_or("") {
            "SCALAR" => 1,
            "VEC2" => 2,
            "VEC3" => 3,
            "VEC4" => 4,
            t => bail!("accessor type {t} not handled"),
        };
        let count = a["count"].as_u64().context("accessor without count")? as usize;
        let kind = a["componentType"]
            .as_u64()
            .context("accessor without componentType")?;
        let size = match kind {
            5126 | 5125 => 4,
            5123 | 5122 => 2,
            5121 | 5120 => 1,
            k => bail!("component type {k} not handled"),
        };
        let view_index = a["bufferView"]
            .as_u64()
            .context("sparse or empty accessor")? as usize;
        let view = self.view(view_index)?;
        let offset = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let stride = self.json["bufferViews"][view_index]["byteStride"]
            .as_u64()
            .map_or(n * size, |s| s as usize);
        let mut out = Vec::with_capacity(count * n);
        for i in 0..count {
            for c in 0..n {
                let at = offset + i * stride + c * size;
                let b = view
                    .get(at..at + size)
                    .context("accessor out of its bufferView")?;
                out.push(match kind {
                    5126 => f32::from_le_bytes(b.try_into()?),
                    5125 => u32::from_le_bytes(b.try_into()?) as f32,
                    5123 => u16::from_le_bytes(b.try_into()?) as f32,
                    5122 => i16::from_le_bytes(b.try_into()?) as f32,
                    5121 => b[0] as f32,
                    _ => b[0] as i8 as f32,
                });
            }
        }
        Ok((out, n))
    }

    fn indices(&self, index: usize) -> Result<Vec<u32>> {
        let a = &self.json["accessors"][index];
        let count = a["count"].as_u64().context("accessor without count")? as usize;
        let kind = a["componentType"].as_u64().unwrap_or(0);
        let view = self.view(
            a["bufferView"]
                .as_u64()
                .context("indices without bufferView")? as usize,
        )?;
        let offset = a["byteOffset"].as_u64().unwrap_or(0) as usize;
        (0..count)
            .map(|i| {
                Ok(match kind {
                    5125 => u32::from_le_bytes(
                        view.get(offset + 4 * i..offset + 4 * i + 4)
                            .context("indices cut")?
                            .try_into()?,
                    ),
                    5123 => u16::from_le_bytes(
                        view.get(offset + 2 * i..offset + 2 * i + 2)
                            .context("indices cut")?
                            .try_into()?,
                    ) as u32,
                    5121 => *view.get(offset + i).context("indices cut")? as u32,
                    k => bail!("index type {k} not handled"),
                })
            })
            .collect()
    }
}

fn v3(v: &Value, default: V3) -> V3 {
    match v.as_array() {
        Some(a) if a.len() == 3 => {
            [0, 1, 2].map(|i| a[i].as_f64().unwrap_or(default[i] as f64) as f32)
        }
        _ => default,
    }
}

fn node_matrix(node: &Value) -> Mat4 {
    if let Some(m) = node["matrix"].as_array().filter(|m| m.len() == 16) {
        let f: Vec<f32> = m.iter().map(|v| v.as_f64().unwrap_or(0.0) as f32).collect();
        return Mat4([0, 1, 2, 3].map(|c| [0, 1, 2, 3].map(|r| f[c * 4 + r])));
    }
    let t = v3(&node["translation"], [0.0; 3]);
    let s = v3(&node["scale"], [1.0; 3]);
    let q = match node["rotation"].as_array() {
        Some(a) if a.len() == 4 => [0, 1, 2, 3].map(|i| a[i].as_f64().unwrap_or(0.0) as f32),
        _ => [0.0, 0.0, 0.0, 1.0],
    };
    Mat4::trs(t, q, s)
}

/// Halves an RGBA8 image (a 2x2 box; an odd edge keeps its last row/column).
fn half(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            for c in 0..4 {
                let mut s = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (2 * x + dx).min(w - 1);
                    let sy = (2 * y + dy).min(h - 1);
                    s += src[((sy * w + sx) * 4 + c) as usize] as u32;
                }
                out[((y * nw + x) * 4 + c) as usize] = ((s + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// An image with all its mip levels, first halved until it is at most
/// `max_side` wide and high.
pub fn with_mips(mut rgba: Vec<u8>, mut w: u32, mut h: u32, max_side: u32, srgb: bool) -> Image {
    while w > max_side || h > max_side {
        let (r, nw, nh) = half(&rgba, w, h);
        rgba = r;
        w = nw;
        h = nh;
    }
    let (width, height) = (w, h);
    let mut levels = vec![rgba];
    while w > 1 || h > 1 {
        let (r, nw, nh) = half(levels.last().unwrap(), w, h);
        levels.push(r);
        w = nw;
        h = nh;
    }
    Image {
        width,
        height,
        levels,
        srgb,
    }
}

/// Reads a glTF binary of the board. Images wider or higher than
/// `max_side` are halved until they fit.
pub fn parse(bytes: &[u8], max_side: u32) -> Result<BoardModel> {
    let g = glb(bytes)?;
    let j = &g.json;
    let empty = Vec::new();

    // The images: the board's top and bottom, colour and roughness/metal.
    let image_names: Vec<String> = j["images"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|i| i["name"].as_str().unwrap_or("").to_string())
        .collect();
    let slot_of = |name: &str| -> Option<usize> {
        match name {
            n if n.starts_with("top_color") => Some(0),
            n if n.starts_with("top_data") => Some(1),
            n if n.starts_with("bottom_color") => Some(2),
            n if n.starts_with("bottom_data") => Some(3),
            _ => None,
        }
    };
    let mut images: Vec<Option<Image>> = (0..4).map(|_| None).collect();
    for (k, im) in j["images"].as_array().unwrap_or(&empty).iter().enumerate() {
        let Some(slot) = slot_of(&image_names[k]) else {
            continue;
        };
        let Some(view) = im["bufferView"].as_u64() else {
            continue;
        };
        let data = g.view(view as usize)?;
        let decoded = image::load_from_memory(data)
            .with_context(|| format!("decoding image {}", image_names[k]))?;
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        // Roughness/metal need less detail than the colour.
        let side = if slot % 2 == 1 {
            max_side / 2
        } else {
            max_side
        };
        images[slot] = Some(with_mips(rgba.into_raw(), w, h, side.max(1), slot % 2 == 0));
    }
    let images: Vec<Image> = images
        .into_iter()
        .enumerate()
        .map(|(k, i)| {
            i.unwrap_or_else(|| {
                // No image there: a single texel, white for colour, the data
                // map's "smooth, not metal".
                let px = if k % 2 == 0 {
                    vec![255, 255, 255, 255]
                } else {
                    vec![0, 102, 0, 255]
                };
                Image {
                    width: 1,
                    height: 1,
                    levels: vec![px],
                    srgb: k % 2 == 0,
                }
            })
        })
        .collect();

    let texture_image = |t: &Value| -> Option<usize> {
        let tex = t["index"].as_u64()? as usize;
        let src = j["textures"][tex]["source"].as_u64()? as usize;
        slot_of(image_names.get(src)?)
    };
    let materials: Vec<Material> = j["materials"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .map(|m| {
            let name = m["name"].as_str().unwrap_or("").to_string();
            let pbr = &m["pbrMetallicRoughness"];
            let base = match pbr["baseColorFactor"].as_array() {
                Some(a) if a.len() == 4 => {
                    [0, 1, 2, 3].map(|i| a[i].as_f64().unwrap_or(1.0) as f32)
                }
                _ => [1.0; 4],
            };
            let textures = if name == "OLED_screen" {
                3
            } else {
                match texture_image(&pbr["baseColorTexture"]) {
                    Some(0) => 1,
                    Some(2) => 2,
                    _ => 0,
                }
            };
            let transmission = m["extensions"]["KHR_materials_transmission"]["transmissionFactor"]
                .as_f64()
                .unwrap_or(0.0) as f32;
            let led = name
                .strip_prefix("LED_D")
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|&n| n < 8);
            Material {
                blended: (transmission > 0.0 && led.is_none() && !name.starts_with("LED_"))
                    || m["alphaMode"].as_str() == Some("BLEND"),
                metallic: pbr["metallicFactor"].as_f64().unwrap_or(1.0) as f32,
                roughness: pbr["roughnessFactor"].as_f64().unwrap_or(1.0) as f32,
                base,
                textures,
                led,
            }
        })
        .collect();

    let mut vertices = Vec::new();
    let mut tris: Vec<(bool, [u32; 3])> = Vec::new();
    let mut parts = Vec::new();
    let mut leds = [None; 8];
    let mut outline = Vec::new();
    let mut hdmi_end = None;
    let mut switch_travel = [None; 4];

    // Walk the scene's nodes, each with its frame in the board's.
    let scene = j["scene"].as_u64().unwrap_or(0) as usize;
    type WalkNode = (usize, Mat4, Option<usize>, Option<u32>, Option<usize>);
    let mut stack: Vec<WalkNode> = j["scenes"][scene]["nodes"]
        .as_array()
        .unwrap_or(&empty)
        .iter()
        .filter_map(|n| n.as_u64())
        .map(|n| (n as usize, Mat4::IDENTITY, None, None, None))
        .collect();
    while let Some((index, parent, parent_btn, parent_option, parent_sw)) = stack.pop() {
        let node = &j["nodes"][index];
        let world = parent.mul(&node_matrix(node));
        let name = node["name"].as_str().unwrap_or("").to_string();
        let extras = &node["extras"];
        let btn = extras["btn_index"]
            .as_u64()
            .map(|b| b as usize)
            .filter(|&b| b < 7);
        let led = extras["led_index"]
            .as_u64()
            .map(|b| b as usize)
            .filter(|&b| b < 8);
        let sw = extras["sw_index"]
            .as_u64()
            .map(|s| s as usize)
            .filter(|&s| s < 4)
            .or(parent_sw);
        if let Some(s) = sw {
            if extras["sw_travel"].is_array() {
                // Exported extras already use the board frame; do not turn them again.
                switch_travel[s] = Some(v3(&extras["sw_travel"], [0.0; 3]));
            }
        }
        // A cap is the child of its button (`B3_plunger` under `B3`).
        let cap = if name.ends_with("_plunger") {
            parent_btn
        } else {
            None
        };
        let option = match extras["option"].as_str() {
            Some("oled") => Some(8),
            Some("hdmi") => Some(9),
            Some("headers") => Some(10),
            _ => parent_option,
        };
        let object = option.unwrap_or_else(|| match sw {
            Some(s) => 11 + s as u32,
            None => cap.map_or(0, |b| b as u32 + 1),
        });
        if option == Some(9) && extras["cable_end"].is_array() {
            // Custom property stays in Blender's local (x,y,z) frame.
            let [x, y, z] = v3(&extras["cable_end"], [0.0; 3]);
            hdmi_end = Some(world.point([x, z, -y]));
        }
        for child in node["children"]
            .as_array()
            .unwrap_or(&empty)
            .iter()
            .filter_map(|c| c.as_u64())
        {
            stack.push((child as usize, world, btn.or(parent_btn), option, sw));
        }
        let Some(mesh) = node["mesh"].as_u64() else {
            continue;
        };
        let normal_m = world.inverse().map(|i| {
            // The normals turn by the inverse transpose.
            let t = i.0;
            Mat4([0, 1, 2, 3].map(|c| [0, 1, 2, 3].map(|r| t[r][c])))
        });
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for prim in j["meshes"][mesh as usize]["primitives"]
            .as_array()
            .unwrap_or(&empty)
        {
            if prim["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let attrs = &prim["attributes"];
            let (pos, _) = g.floats(
                attrs["POSITION"]
                    .as_u64()
                    .context("primitive without POSITION")? as usize,
            )?;
            let normals = match attrs["NORMAL"].as_u64() {
                Some(a) => Some(g.floats(a as usize)?.0),
                None => None,
            };
            let uvs = match attrs["TEXCOORD_0"].as_u64() {
                Some(a) => Some(g.floats(a as usize)?.0),
                None => None,
            };
            let count = pos.len() / 3;
            let material = prim["material"].as_u64().unwrap_or(0) as u32;
            let blended = materials.get(material as usize).is_some_and(|m| m.blended);
            let first = vertices.len() as u32;
            for i in 0..count {
                let local = [pos[3 * i], pos[3 * i + 1], pos[3 * i + 2]];
                for a in 0..3 {
                    lo[a] = lo[a].min(local[a]);
                    hi[a] = hi[a].max(local[a]);
                }
                let p = world.point(local);
                let n = normals
                    .as_ref()
                    .map(|n| [n[3 * i], n[3 * i + 1], n[3 * i + 2]])
                    .unwrap_or([0.0, 1.0, 0.0]);
                let n = normalize(normal_m.map_or(n, |m| m.dir(n)));
                let uv = uvs
                    .as_ref()
                    .map(|u| [u[2 * i], u[2 * i + 1]])
                    .unwrap_or([0.0, 0.0]);
                vertices.push(Vertex {
                    pos: p,
                    normal: n,
                    uv,
                    material,
                    object,
                });
            }
            let idx = match prim["indices"].as_u64() {
                Some(a) => g.indices(a as usize)?,
                None => (0..count as u32).collect(),
            };
            for t in idx.chunks_exact(3) {
                tris.push((blended, [first + t[0], first + t[1], first + t[2]]));
            }
        }
        if lo[0].is_finite() && option.is_none() {
            for k in 0..8 {
                outline.push(world.point([
                    if k & 1 == 0 { lo[0] } else { hi[0] },
                    if k & 2 == 0 { lo[1] } else { hi[1] },
                    if k & 4 == 0 { lo[2] } else { hi[2] },
                ]));
            }
        }
        if let Some(reference) = extras["ref"].as_str() {
            if lo[0].is_finite() {
                let inverse = world.inverse().unwrap_or(Mat4::IDENTITY);
                if let Some(l) = led {
                    let c = world.point([(lo[0] + hi[0]) / 2.0, hi[1], (lo[2] + hi[2]) / 2.0]);
                    leds[l] = Some(c);
                }
                if let Some(travel) = sw.and_then(|s| switch_travel[s]) {
                    let local = inverse.dir(travel);
                    for a in 0..3 {
                        lo[a] += local[a].min(0.0);
                        hi[a] += local[a].max(0.0);
                    }
                }
                parts.push(Part {
                    name: name.clone(),
                    reference: reference.to_string(),
                    value: extras["value"].as_str().unwrap_or("").to_string(),
                    side: extras["side"].as_str().unwrap_or("").to_string(),
                    btn: btn.or(cap),
                    sw,
                    led,
                    object,
                    inverse,
                    min: lo,
                    max: hi,
                });
            }
        }
    }
    if vertices.is_empty() {
        bail!("the model has no triangles");
    }
    // The opaque triangles first, then the see-through ones.
    tris.sort_by_key(|t| t.0);
    let n_opaque = tris.iter().filter(|t| !t.0).count() as u32 * 3;
    let indices: Vec<u32> = tris.iter().flat_map(|t| t.1).collect();
    let total = indices.len() as u32;
    Ok(BoardModel {
        vertices,
        indices,
        opaque: 0..n_opaque,
        blended: n_opaque..total,
        materials,
        images,
        parts,
        leds,
        outline,
        hdmi_end,
        switch_travel,
    })
}

/// A linear colour for an LED's light, from its KiCad value.
pub fn led_light(value: &str) -> [f32; 3] {
    match value {
        "RED" => [1.0, 0.06, 0.03],
        "ORANGE" => [1.0, 0.32, 0.02],
        "GREEN" => [0.08, 1.0, 0.12],
        "BLUE" => [0.05, 0.25, 1.0],
        _ => [1.0, 1.0, 0.9],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Pass the environment value into the resolver so these tests do not
    // mutate the process environment while other board tests read it.
    #[test]
    fn model_path_lookup_order() {
        let root = crate::test_temp::unique_dir("ecp5-board-model-");
        let out = root.join("out/ulx3s_3d/ulx3s_v3.0.8.glb");
        let assets = root.join("assets/ulx3s_3d/ulx3s_v3.0.8.glb");
        for path in [&out, &assets] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, tiny_glb()).unwrap();
        }
        let override_path = root.join("override.glb");
        std::fs::write(&override_path, tiny_glb()).unwrap();
        assert_eq!(
            default_path_in(&root, Some(override_path.clone())),
            override_path
        );
        assert_eq!(default_path_in(&root, None), out);
        std::fs::remove_file(&out).unwrap();
        assert_eq!(default_path_in(&root, None), assets);
        // An explicit override must not silently fall back when missing.
        std::fs::remove_file(&override_path).unwrap();
        assert_eq!(
            default_path_in(&root, Some(override_path.clone())),
            override_path
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn model_path_environment_override() {
        const CHILD_PATH: &str = "ECP5_TEST_BOARD_MODEL_PATH";
        if let Some(expected) = std::env::var_os(CHILD_PATH) {
            assert_eq!(default_path(), PathBuf::from(expected));
            return;
        }
        let root = crate::test_temp::unique_dir("ecp5-board-env-");
        let path = root.join("override.glb");
        std::fs::write(&path, tiny_glb()).unwrap();
        // Exercise the real environment in a child, isolated from parallel tests.
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "board_model::tests::model_path_environment_override",
                "--nocapture",
            ])
            .env("ECP5_BOARD_MODEL", &path)
            .env(CHILD_PATH, &path)
            .status()
            .unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert!(status.success());
    }

    #[test]
    fn model_path_lfs_pointer_error() {
        let root = crate::test_temp::unique_dir("ecp5-board-pointer-");
        let path = root.join("model.glb");
        let pointer = b"version https://git-lfs.github.com/spec/v1\noid sha256:b1d25e5b8640c365e0d53d6773bd7dbd409b6aeb6c14e63c06285c94206f5c9d\nsize 11507464\n";
        std::fs::write(&path, pointer).unwrap();
        let err = load(&path).err().expect("an LFS pointer is not a model");
        assert_eq!(
            err.to_string(),
            "3D board model is a git LFS pointer; run `git lfs pull`."
        );
        assert!(!is_lfs_pointer(&tiny_glb()));
        assert!(!is_lfs_pointer(b"not a model"));
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A glTF binary with one node: a triangle, its extras, a translation.
    fn tiny_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        for p in [[0.0f32, 0.0, 0.0], [2.0, 0.0, 0.0], [0.0, 0.0, -2.0]] {
            for v in p {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        for _ in 0..3 {
            for v in [0.0f32, 1.0, 0.0] {
                bin.extend_from_slice(&v.to_le_bytes());
            }
        }
        for i in [0u16, 1, 2, 0] {
            bin.extend_from_slice(&i.to_le_bytes());
        }
        let json = serde_json::json!({
            "asset": {"version": "2.0"},
            "scene": 0,
            "scenes": [{"nodes": [0]}],
            "nodes": [{"name": "B3", "mesh": 0, "translation": [10.0, 1.0, 0.0],
                       "extras": {"ref": "B3", "value": "PTS645", "side": "top", "btn_index": 3},
                       "children": [1]},
                      {"name": "B3_plunger", "mesh": 0, "extras": {"ref": "B3"}}],
            "meshes": [{"primitives": [{"attributes": {"POSITION": 0, "NORMAL": 1}, "indices": 2, "material": 0}]}],
            "materials": [{"name": "LED_D2", "pbrMetallicRoughness": {"baseColorFactor": [1.0, 0.5, 0.5, 1.0], "metallicFactor": 0.0}}],
            "accessors": [
                {"bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 1, "componentType": 5126, "count": 3, "type": "VEC3"},
                {"bufferView": 2, "componentType": 5123, "count": 3, "type": "SCALAR"}
            ],
            "bufferViews": [
                {"buffer": 0, "byteOffset": 0, "byteLength": 36},
                {"buffer": 0, "byteOffset": 36, "byteLength": 36},
                {"buffer": 0, "byteOffset": 72, "byteLength": 6}
            ],
            "buffers": [{"byteLength": bin.len()}]
        });
        encode_glb(&json, &bin)
    }

    fn encode_glb(json: &Value, bin: &[u8]) -> Vec<u8> {
        let mut js = serde_json::to_vec(json).unwrap();
        while !js.len().is_multiple_of(4) {
            js.push(b' ');
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&((12 + 8 + js.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend_from_slice(&(js.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x4E4F534Au32.to_le_bytes());
        out.extend_from_slice(&js);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E4942u32.to_le_bytes());
        out.extend_from_slice(bin);
        out
    }

    /// Nodes land where their frames put them; a button's cap moves with
    /// its button; LED materials know their LED; picking finds the part.
    #[test]
    fn reads_nodes_parts_and_picks() {
        let m = parse(&tiny_glb(), 64).unwrap();
        assert_eq!(m.vertices.len(), 6);
        assert_eq!(m.vertices[0].pos, [10.0, 1.0, 0.0]);
        assert_eq!(m.vertices[1].pos, [12.0, 1.0, 0.0]);
        assert_eq!(m.parts.len(), 2);
        let cap = m.parts.iter().find(|p| p.name == "B3_plunger").unwrap();
        assert_eq!(cap.btn, Some(3));
        assert_eq!(m.vertices[3].object, 4, "the cap moves with button 3");
        assert_eq!(m.vertices[0].object, 0);
        assert_eq!(m.materials[0].led, Some(2));
        assert!(!m.materials[0].blended);
        assert_eq!(m.opaque, 0..6);
        // Straight down onto the triangle's corner region.
        let (k, t) = m
            .pick([10.5, 50.0, -0.5], [0.0, -1.0, 0.0], [false; 3])
            .unwrap();
        assert_eq!(m.parts[k].reference, "B3");
        assert!((t - 48.8).abs() < 0.3, "{t}");
        assert!(m
            .pick([50.0, 50.0, 50.0], [0.0, -1.0, 0.0], [false; 3])
            .is_none());
    }

    #[test]
    fn option_children_screen_and_visible_picking() {
        let bytes = tiny_glb();
        let mut g = glb(&bytes).unwrap();
        g.json["scenes"][0]["nodes"] = serde_json::json!([0, 2, 4]);
        g.json["nodes"] = serde_json::json!([
            {"name":"OLED_module", "mesh":0, "translation":[10,2,0],
             "extras":{"ref":"OLED1", "option":"oled"}, "children":[1]},
            {"name":"screen", "mesh":0, "translation":[0,1,0], "extras":{"ref":"OLED1"}},
            {"name":"HDMI_plug", "translation":[30,0,0],
             "extras":{"option":"hdmi", "cable_end":[1,2,3]}, "children":[3]},
            {"name":"cable", "mesh":0, "extras":{"ref":"GPDI1"}},
            {"name":"J1_header", "mesh":0, "translation":[50,0,0],
             "extras":{"ref":"J1", "option":"headers"}, "children":[5]},
            {"name":"pin", "mesh":0, "translation":[0,1,0], "extras":{"ref":"J1"}}
        ]);
        g.json["materials"][0]["name"] = Value::from("OLED_screen");
        let m = parse(&encode_glb(&g.json, g.bin), 64).unwrap();
        assert_eq!(m.materials[0].textures, 3);
        assert_eq!(m.vertices.iter().filter(|v| v.object == 8).count(), 6);
        assert_eq!(m.vertices.iter().filter(|v| v.object == 9).count(), 3);
        assert_eq!(m.vertices.iter().filter(|v| v.object == 10).count(), 6);
        assert!(m.parts.iter().all(|p| p.object
            == match p.reference.as_str() {
                "OLED1" => 8,
                "GPDI1" => 9,
                "J1" => 10,
                _ => unreachable!(),
            }));
        assert_eq!(m.hdmi_end, Some([31.0, 3.0, -2.0]));
        assert!(
            m.outline.is_empty(),
            "options do not change the board's camera fit"
        );
        for (x, shown, object) in [
            (10.5, [true, false, false], 8),
            (30.5, [false, true, false], 9),
            (50.5, [false, false, true], 10),
        ] {
            let origin = [x, 50.0, -0.5];
            let dir = [0.0, -1.0, 0.0];
            assert!(m.pick(origin, dir, [false; 3]).is_none());
            let (k, _) = m.pick(origin, dir, shown).unwrap();
            assert_eq!(m.parts[k].object, object);
        }
    }

    #[test]
    fn switch_nodes_children_travel_and_slot_picking() {
        let bytes = tiny_glb();
        let mut g = glb(&bytes).unwrap();
        g.json["scenes"][0]["nodes"] = serde_json::json!([0, 2]);
        g.json["nodes"] = serde_json::json!([
            {"name":"SW1_slider3", "mesh":0, "translation":[10,2,0],
             "rotation":[0, std::f64::consts::FRAC_1_SQRT_2, 0, std::f64::consts::FRAC_1_SQRT_2],
             "scale":[0.2,1,0.2],
             "extras":{"ref":"SW1", "sw_index":2, "sw_travel":[0,0,-1.7]}, "children":[1]},
            {"name":"lever_tip", "mesh":0, "translation":[0,2,0], "extras":{"ref":"SW1"}},
            {"name":"SW1", "mesh":0, "translation":[9.4,5,0.4], "scale":[1,1,2],
             "extras":{"ref":"SW1"}}
        ]);
        let m = parse(&encode_glb(&g.json, g.bin), 64).unwrap();
        assert_eq!(m.parts.len(), 3);
        assert!(m
            .parts
            .iter()
            .filter(|p| p.name != "SW1")
            .all(|p| p.sw == Some(2) && p.object == 13));
        assert_eq!(m.vertices.iter().filter(|v| v.object == 13).count(), 6);
        assert_eq!(m.switch_travel, [None, None, Some([0.0, 0.0, -1.7]), None]);
        // The rotated/scaled node needs travel converted back to its local pick box.
        // Picking is independent of ON/OFF and of optional OLED/HDMI/header visibility.
        for z in [-0.2, -1.9] {
            let (k, _) = m
                .pick([9.8, 50.0, z], [0.0, -1.0, 0.0], [false; 3])
                .unwrap();
            assert_eq!(m.parts[k].sw, Some(2));
        }
        let (k, _) = m
            .pick([9.8, 50.0, -3.0], [0.0, -1.0, 0.0], [false; 3])
            .unwrap();
        assert_eq!(
            m.parts[k].name, "SW1",
            "outside the slots still picks the body"
        );
        g.json["scenes"][0]["nodes"] = serde_json::json!([0, 2, 3]);
        g.json["nodes"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "name":"other_part", "mesh":0, "translation":[9.4,4.5,0.4], "scale":[1,1,2],
                "extras":{"ref":"OTHER"}
            }));
        let m = parse(&encode_glb(&g.json, g.bin), 64).unwrap();
        let (k, _) = m
            .pick([9.8, 50.0, -0.2], [0.0, -1.0, 0.0], [false; 3])
            .unwrap();
        assert_eq!(
            m.parts[k].reference, "OTHER",
            "other parts still occlude a lever"
        );
    }

    /// The camera: the photo view looks at the board from above, the top view
    /// straight down with the board's top edge up, the bottom view from
    /// below with the board turned like its bottom photo; a ray through the
    /// middle hits the target; fitting puts the box inside the view.
    #[test]
    fn camera_views_rays_and_fit() {
        let (back, right, up) = Camera::TOP.basis();
        assert!(back[1] > 0.999);
        assert!((right[0] - 1.0).abs() < 1e-3);
        assert!(up[2] < -0.999, "top view: the board's top edge (-z) is up");
        let (back, right, up) = Camera::BOTTOM.basis();
        assert!(back[1] < -0.999);
        assert!(
            right[0] < -0.999,
            "bottom view: mirrored left-right like the photo"
        );
        assert!(up[2] < -0.999);
        let photo = Camera::PHOTO;
        let eye = photo.eye(100.0);
        assert!(eye[0] < 0.0 && eye[1] > 0.0 && eye[2] > 0.0, "{eye:?}");
        let (o, d) = photo.ray(100.0, 1.6, [0.0, 0.0]);
        let t = -o[1] / d[1];
        let hit = add(o, scale(d, t));
        assert!(hit[0].abs() < 1e-3 && hit[2].abs() < 1e-3, "{hit:?}");
        let (min, max) = ([-47.0, -4.0, -25.4], [47.0, 6.0, 31.7]);
        let corners: Vec<V3> = (0..8)
            .map(|k| {
                [
                    if k & 1 == 0 { min[0] } else { max[0] },
                    if k & 2 == 0 { min[1] } else { max[1] },
                    if k & 4 == 0 { min[2] } else { max[2] },
                ]
            })
            .collect();
        for cam in [Camera::PHOTO, Camera::TOP, Camera::BOTTOM] {
            for aspect in [0.6, 0.8, 1.6, 2.5] {
                let d = cam.fit(&corners, aspect);
                let m = cam.view_proj(d, aspect);
                for &c in &corners {
                    let p = m.point(c);
                    assert!(
                        p[0].abs() <= 0.97 && p[1].abs() <= 0.95 && (0.0..1.0).contains(&p[2]),
                        "{p:?}"
                    );
                }
            }
        }
    }

    /// Mip levels halve down to one texel; a big image is halved to fit first.
    #[test]
    fn mips() {
        let img = with_mips(vec![200; 13 * 6 * 4], 13, 6, 8, false);
        assert_eq!((img.width, img.height), (6, 3));
        let sizes: Vec<usize> = img.levels.iter().map(|l| l.len() / 4).collect();
        assert_eq!(sizes, vec![18, 3, 1]);
        assert!(img.levels.iter().all(|l| l.iter().all(|&v| v == 200)));
    }

    /// The shipped or rebuilt model: the eight user LEDs
    /// and the seven buttons with their caps are there, and the board fits
    /// its true size.
    #[test]
    fn the_built_model() {
        let path = default_path();
        if !path.exists() {
            eprintln!("skipped: {} not built (tools/ulx3s_3d)", path.display());
            return;
        }
        if is_lfs_pointer(&std::fs::read(&path).unwrap()) {
            eprintln!("skipped: git LFS model not downloaded (git lfs pull)");
            return;
        }
        let m = load(&path).unwrap();
        assert!(m.leds.iter().all(Option::is_some), "{:?}", m.leds);
        for b in 0..7 {
            assert!(m
                .parts
                .iter()
                .any(|p| p.btn == Some(b) && p.name == format!("B{b}")));
            assert!(
                m.vertices.iter().any(|v| v.object == b as u32 + 1),
                "B{b}'s cap moves"
            );
        }
        for l in 0..8 {
            assert!(m.materials.iter().any(|mat| mat.led == Some(l)), "LED_D{l}");
        }
        let (lo, hi) = m
            .vertices
            .iter()
            .filter(|v| v.object < 8)
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
                (lo.min(v.pos[0]), hi.max(v.pos[0]))
            });
        assert!((hi - lo - 93.98).abs() < 0.6, "{lo} {hi}");
        assert!(
            m.materials.iter().any(|mat| mat.textures == 1)
                && m.materials.iter().any(|mat| mat.textures == 2)
        );
        assert!(!m.blended.is_empty(), "the DIP switch's tape");
        assert!(m.images.iter().all(|i| i.width > 1));
        for (name, object) in [
            ("OLED_module", 8),
            ("HDMI_plug", 9),
            ("J1_header", 10),
            ("J2_header", 10),
        ] {
            assert!(
                m.parts.iter().any(|p| p.name == name && p.object == object),
                "{name}"
            );
            assert!(m.vertices.iter().any(|v| v.object == object));
        }
        let sliders: Vec<_> = m
            .parts
            .iter()
            .filter(|p| p.name.starts_with("SW1_slider"))
            .collect();
        assert_eq!(sliders.len(), 4, "four SW1 levers");
        let mut switch_x = [0.0; 4];
        for (s, x) in switch_x.iter_mut().enumerate() {
            let lever = sliders.iter().find(|p| p.sw == Some(s)).unwrap_or_else(|| {
                panic!("SW1 lever sw[{s}] needs sw_index extras; rebuild the GLB")
            });
            assert_eq!(lever.object, 11 + s as u32);
            assert!(m.vertices.iter().any(|v| v.object == 11 + s as u32));
            let travel = m.switch_travel[s].expect("SW1 lever needs sw_travel");
            assert!((dot(travel, travel).sqrt() - 1.7).abs() < 0.1, "{travel:?}");
            assert!(
                travel[2] < -1.6,
                "ON faces the board top, negative glTF z: {travel:?}"
            );
            *x = lever.inverse.inverse().unwrap().point([0.0; 3])[0];
        }
        assert!(
            switch_x[1..].iter().all(|&x| switch_x[0] < x),
            "sw[0] must be leftmost: {switch_x:?}"
        );
        let screen: Vec<_> = m
            .vertices
            .iter()
            .filter(|v| m.materials[v.material as usize].textures == 3)
            .collect();
        assert!(
            !screen.is_empty() && screen.iter().all(|v| v.object == 8),
            "OLED screen belongs to the module"
        );
        let corner = |uv: [f32; 2]| {
            screen
                .iter()
                .find(|v| (v.uv[0] - uv[0]).abs() < 1e-5 && (v.uv[1] - uv[1]).abs() < 1e-5)
                .unwrap()
                .pos
        };
        let (a, b, c) = (corner([0.0, 0.0]), corner([1.0, 0.0]), corner([0.0, 1.0]));
        let dist = |p, q| dot(add(p, scale(q, -1.0)), add(p, scale(q, -1.0)));
        // board.json is a local rebuild byproduct, not a shipped runtime input.
        let board_json = path.with_file_name("board.json");
        if board_json.is_file() {
            let data: Value = serde_json::from_slice(&std::fs::read(board_json).unwrap()).unwrap();
            let fp = data["footprints"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["ref"] == "OLED1")
                .unwrap();
            let gnd = fp["pads"]
                .as_array()
                .unwrap()
                .iter()
                .find(|p| p["net"] == "GND")
                .unwrap();
            let f = |v: &Value| v.as_f64().unwrap() as f32;
            let rot = f(&fp["rot"]).to_radians();
            let (s, co) = rot.sin_cos();
            let x = f(&fp["x"]) + f(&gnd["x"]) * co + f(&gnd["y"]) * s;
            let y = f(&fp["y"]) - f(&gnd["x"]) * s + f(&gnd["y"]) * co;
            let board = &data["board"];
            let gnd_world = [
                x - (f(&board["x0"]) + f(&board["x1"])) / 2.0,
                f(&board["thickness"]) / 2.0,
                y - (f(&board["y0"]) + f(&board["y1"])) / 2.0,
            ];
            assert!(
                dist(a, gnd_world) < dist(b, gnd_world),
                "UV (0,0) must be at GND: {a:?} {b:?} {gnd_world:?}"
            );
        } else {
            eprintln!("skipped: OLED GND comparison needs tools/ulx3s_3d board.json");
        }
        let header = m
            .parts
            .iter()
            .find(|p| p.name == "OLED1")
            .unwrap()
            .inverse
            .inverse()
            .unwrap()
            .point([0.0; 3]);
        assert!(
            dist(a, header) < dist(c, header),
            "V grows away from header: {a:?} {c:?}"
        );
        assert!((dist(a, b).sqrt() - 20.14).abs() < 0.01);
        assert!((dist(a, c).sqrt() - 13.42).abs() < 0.01);
    }
}

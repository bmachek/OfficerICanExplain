//! Procedural surface textures.
//!
//! No image files ship with this game, so every texture is painted into an
//! `Image` at startup by evaluating a function per pixel. The noise underneath
//! it is a hash of the lattice coordinate rather than a permutation table, and
//! the lattice wraps at a fixed period — which is what lets one 512-pixel
//! square tile across two kilometres of road without a seam.
//!
//! Two details cost more thought than they look:
//!
//! The noise, painting and mip helpers at the top are the shared kit: anything
//! in the project that needs to paint a texture uses them rather than growing
//! its own. The generators below them are the city's own surfaces.
//!
//! * **Mip chains are built here.** Bevy has no runtime mip generator, and a
//!   texture tiled a few hundred times across the ground plane without mips
//!   aliases into a shimmering mess the moment the camera moves. Levels are
//!   averaged in linear space for sRGB images, because averaging sRGB bytes
//!   directly darkens every level.
//! * **Facades carry three maps.** Base colour, an emissive mask saying which
//!   windows are lit, and a packed roughness/metallic map so glass catches the
//!   sun and the wall beside it does not. The emissive *strength* is not baked
//!   in: the day/night cycle drives it, so the city lights up at dusk. See
//!   `timeofday::light_windows`.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// Facades are the only textures read close up and at a glancing angle.
const FACADE_SIZE: u32 = 512;
/// Emissive and roughness maps only ever modulate the base, so half is plenty.
const MASK_SIZE: u32 = 256;
const GROUND_SIZE: u32 = 256;

// ---------------------------------------------------------------- noise ----

fn hash(x: u32, y: u32, seed: u32) -> u32 {
    let mut h =
        x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h
}

/// A stable pseudo-random number in 0..1 for a lattice cell.
pub fn hash01(x: u32, y: u32, seed: u32) -> f32 {
    hash(x, y, seed) as f32 / u32::MAX as f32
}

pub fn smoothstep(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// Value noise on a lattice that wraps every `period` cells.
///
/// The wrap is the whole point: it makes the texture tileable, so a road can be
/// one quad with the sampler repeating rather than thousands of unique polys.
fn value_noise(u: f32, v: f32, period: u32, seed: u32) -> f32 {
    let period = period.max(1);
    let x = u * period as f32;
    let y = v * period as f32;
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = (smoothstep(x - x0), smoothstep(y - y0));

    let xi = (x0 as i64).rem_euclid(period as i64) as u32;
    let yi = (y0 as i64).rem_euclid(period as i64) as u32;
    let xj = (xi + 1) % period;
    let yj = (yi + 1) % period;

    let bottom = hash01(xi, yi, seed).lerp(hash01(xj, yi, seed), fx);
    let top = hash01(xi, yj, seed).lerp(hash01(xj, yj, seed), fx);
    bottom.lerp(top, fy)
}

/// Summed octaves of [`value_noise`], each one wrapping at twice the rate of
/// the last so the whole stack still tiles.
pub fn fbm(u: f32, v: f32, period: u32, octaves: u32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amplitude = 1.0;
    let mut total = 0.0;
    let mut p = period;
    for octave in 0..octaves {
        sum += value_noise(u, v, p, seed.wrapping_add(octave * 9781)) * amplitude;
        total += amplitude;
        amplitude *= 0.5;
        p = p.saturating_mul(2);
    }
    sum / total
}

/// Ridged noise: peaks where the underlying field crosses its midpoint, which
/// draws thin wandering lines rather than blobs. Used for cracks.
pub fn ridge(u: f32, v: f32, period: u32, octaves: u32, seed: u32) -> f32 {
    1.0 - (fbm(u, v, period, octaves, seed) * 2.0 - 1.0).abs()
}

// -------------------------------------------------------------- painting ----

fn srgb_to_linear(byte: u8) -> f32 {
    let c = byte as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

pub fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// One box-filter step of a mip chain.
pub fn downsample(src: &[u8], width: u32, height: u32, srgb: bool) -> (Vec<u8>, u32, u32) {
    let (dw, dh) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = vec![0u8; (dw * dh * 4) as usize];

    for y in 0..dh {
        for x in 0..dw {
            for channel in 0..4u32 {
                // Alpha is already linear; colour channels are only linear in
                // a linear format.
                let encoded = srgb && channel < 3;
                let mut sum = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(width - 1);
                    let sy = (y * 2 + dy).min(height - 1);
                    let raw = src[((sy * width + sx) * 4 + channel) as usize];
                    sum += if encoded {
                        srgb_to_linear(raw)
                    } else {
                        raw as f32 / 255.0
                    };
                }
                let average = sum / 4.0;
                out[((y * dw + x) * 4 + channel) as usize] = byte(if encoded {
                    linear_to_srgb(average)
                } else {
                    average
                });
            }
        }
    }
    (out, dw, dh)
}

/// Paints an image by evaluating `paint` at the centre of every texel, then
/// builds its mip chain and a repeating, anisotropic sampler.
///
/// Built with `new_uninit` rather than `Image::new` because the latter asserts
/// that the data is exactly one mip level.
pub fn painted(size: u32, format: TextureFormat, paint: impl Fn(f32, f32) -> [u8; 4]) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let u = (x as f32 + 0.5) / size as f32;
            let v = (y as f32 + 0.5) / size as f32;
            data.extend_from_slice(&paint(u, v));
        }
    }

    let srgb = format == TextureFormat::Rgba8UnormSrgb;
    let mut level = data.clone();
    let (mut width, mut height) = (size, size);
    let mut levels = 1;
    while width > 1 || height > 1 {
        let (next, nw, nh) = downsample(&level, width, height, srgb);
        data.extend_from_slice(&next);
        level = next;
        width = nw;
        height = nh;
        levels += 1;
    }

    let mut image = Image::new_uninit(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        format,
        // Nothing ever reads these back on the CPU.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        // The road is seen almost edge-on almost all the time, which is exactly
        // the case trilinear filtering blurs into mud.
        anisotropy_clamp: 8,
        ..default()
    });
    image
}

/// A tangent-space normal map sampled from a wrapping height field.
///
/// Central differences rather than a Sobel kernel: these height fields are
/// interpolated noise and already smooth, so the extra taps buy blur and
/// nothing else. `relief` is how tall the bumps are as a fraction of the tile —
/// small numbers only. Push it past a few percent and a flat surface starts to
/// look like it was moulded out of putty.
pub fn normal_map(size: u32, relief: f32, height: impl Fn(f32, f32) -> f32) -> Image {
    let step = 1.0 / size as f32;
    // Slope per texel, converted to slope per unit of UV.
    let scale = relief * size as f32 * 0.5;
    painted(size, TextureFormat::Rgba8Unorm, |u, v| {
        let dx = height(u + step, v) - height(u - step, v);
        let dy = height(u, v + step) - height(u, v - step);
        let normal = Vec3::new(-dx * scale, -dy * scale, 1.0).normalize();
        [
            byte(normal.x * 0.5 + 0.5),
            byte(normal.y * 0.5 + 0.5),
            byte(normal.z * 0.5 + 0.5),
            255,
        ]
    })
}

// ---------------------------------------------------------------- ground ----

/// Height field the asphalt's colour and its normal map are both built from,
/// so a crack that reads dark also reads deep.
fn asphalt_height(u: f32, v: f32) -> f32 {
    let grain = fbm(u, v, 48, 4, 11) - 0.5;
    let patches = fbm(u, v, 6, 3, 23) - 0.5;
    let mut height = 0.5 + grain * 0.5 + patches * 0.2;

    // Cracks are drawn from a high-frequency ridge and cut off hard. A gentler
    // threshold reads as tarmac rivers rather than as a crack.
    let crack = ridge(u, v, 14, 3, 67);
    if crack > 0.986 {
        height -= ((crack - 0.986) * 40.0).min(0.55);
    }
    height.clamp(0.0, 1.0)
}

/// Road surface: aggregate grain, patched repairs, and a few cracks.
pub fn asphalt() -> Image {
    painted(GROUND_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let mut value = 0.90 + (asphalt_height(u, v) - 0.5) * 0.32;

        // Aggregate: individual chips of stone catching the light.
        let speck = hash01(
            (u * GROUND_SIZE as f32) as u32,
            (v * GROUND_SIZE as f32) as u32,
            41,
        );
        if speck > 0.988 {
            value += 0.22;
        }

        let c = byte(value);
        [c, c, byte(value * 0.995), 255]
    })
}

pub fn asphalt_normal() -> Image {
    normal_map(GROUND_SIZE, 0.007, asphalt_height)
}

/// Slabs per tile, on each axis.
const SLABS: f32 = 4.0;

/// Height field for the pavement: flat slabs, recessed joints.
fn paving_height(u: f32, v: f32) -> f32 {
    let (su, sv) = (u * SLABS, v * SLABS);
    let (fu, fv) = (su.fract(), sv.fract());
    let joint = fu.min(1.0 - fu).min(fv).min(1.0 - fv);
    let bevel = smoothstep01(joint / 0.035);
    0.25 + bevel * 0.7 + (fbm(u, v, 64, 3, 13) - 0.5) * 0.10
}

/// Pavement slabs, jointed on a grid.
pub fn paving() -> Image {
    painted(GROUND_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let (cu, cv) = ((u * SLABS).floor(), (v * SLABS).floor());
        // Per-slab tone, so the pavement does not read as one flat sheet.
        let tone = hash01(cu as u32, cv as u32, 7) * 0.08 - 0.04;
        let value = 0.80 + tone + paving_height(u, v) * 0.17;

        let c = byte(value);
        [c, c, byte(value * 0.98), 255]
    })
}

pub fn paving_normal() -> Image {
    normal_map(GROUND_SIZE, 0.020, paving_height)
}

/// Park grass: two scales of mottling plus a fine speckle.
pub fn grass() -> Image {
    painted(GROUND_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let broad = fbm(u, v, 4, 3, 31) - 0.5;
        let fine = fbm(u, v, 40, 4, 53) - 0.5;
        let value = 0.94 + broad * 0.22 + fine * 0.20;
        // Greener where it is darker: shaded grass reads more saturated.
        [
            byte(value * 0.88),
            byte(value * 1.04),
            byte(value * 0.78),
            255,
        ]
    })
}

fn roof_height(u: f32, v: f32) -> f32 {
    (fbm(u, v, 56, 4, 71) * 0.8 + fbm(u, v, 4, 3, 83) * 0.2).clamp(0.0, 1.0)
}

/// Flat roof: tar and gravel, with damp patches.
pub fn roof() -> Image {
    painted(GROUND_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let value = 0.72 + (roof_height(u, v) - 0.5) * 0.55;
        [byte(value), byte(value * 0.99), byte(value * 1.02), 255]
    })
}

pub fn roof_normal() -> Image {
    normal_map(GROUND_SIZE, 0.018, roof_height)
}

// --------------------------------------------------------------- facades ----

/// How a building is glazed. Picked from height, because that is what actually
/// separates a house from a tower: floor spacing barely changes, so a taller
/// building simply has more, smaller-looking windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FacadeClass {
    House,
    Lowrise,
    Midrise,
    Tower,
}

impl FacadeClass {
    pub const ALL: [FacadeClass; 4] = [
        FacadeClass::House,
        FacadeClass::Lowrise,
        FacadeClass::Midrise,
        FacadeClass::Tower,
    ];

    pub fn for_height(height: f32) -> Self {
        match height {
            h if h < 11.0 => FacadeClass::House,
            h if h < 26.0 => FacadeClass::Lowrise,
            h if h < 55.0 => FacadeClass::Midrise,
            _ => FacadeClass::Tower,
        }
    }

    pub fn index(self) -> usize {
        match self {
            FacadeClass::House => 0,
            FacadeClass::Lowrise => 1,
            FacadeClass::Midrise => 2,
            FacadeClass::Tower => 3,
        }
    }

    /// Windows across, floors up.
    fn grid(self) -> (f32, f32) {
        match self {
            FacadeClass::House => (3.0, 2.0),
            FacadeClass::Lowrise => (5.0, 4.0),
            FacadeClass::Midrise => (7.0, 9.0),
            FacadeClass::Tower => (9.0, 17.0),
        }
    }

    /// Fraction of each cell the glass fills, horizontally and vertically.
    fn glazing(self) -> (f32, f32) {
        match self {
            FacadeClass::House => (0.46, 0.50),
            FacadeClass::Lowrise => (0.56, 0.52),
            FacadeClass::Midrise => (0.70, 0.56),
            // Curtain wall: glass edge to edge, hairline mullions.
            FacadeClass::Tower => (0.88, 0.72),
        }
    }

    /// How likely any one window is lit after dark.
    fn occupancy(self) -> f32 {
        match self {
            FacadeClass::House => 0.55,
            FacadeClass::Lowrise => 0.45,
            FacadeClass::Midrise => 0.38,
            FacadeClass::Tower => 0.30,
        }
    }
}

/// The four maps that make up one facade.
pub struct FacadeMaps {
    pub base: Image,
    /// White where a window is lit; the material's `emissive` scales it.
    pub emissive: Image,
    /// Packed the way glTF packs it: green is roughness, blue is metallic.
    pub surface: Image,
    /// Tangent-space relief: recessed panes, grooved floor lines.
    pub normal: Image,
}

pub fn smoothstep01(t: f32) -> f32 {
    smoothstep(t.clamp(0.0, 1.0))
}

/// Where a texel falls within its window cell.
struct Cell {
    column: u32,
    row: u32,
    /// 1 well inside the glass, 0 well outside, ramped across the reveal.
    ///
    /// Soft rather than boolean because the same value drives the height field
    /// the normal map is built from, and a one-texel cliff there produces a
    /// bevel with a staircase in it.
    pane: f32,
    /// True inside the glass.
    glass: bool,
    /// 0 at the bottom of the pane, 1 at the top. Meaningless off the glass.
    up_pane: f32,
    /// Distance below the pane above, in cell heights; `None` above it.
    below_pane: Option<f32>,
}

/// Resolves a UV into the window grid. `v` runs up the building.
fn cell_at(class: FacadeClass, u: f32, v: f32) -> Cell {
    let (columns, rows) = class.grid();
    let (glass_w, glass_h) = class.glazing();

    let su = u * columns;
    let sv = v * rows;
    let (column, row) = (su.floor(), sv.floor());
    let (fu, fv) = (su - column, sv - row);

    // Panes sit slightly above centre in their cell, leaving a spandrel below.
    let (u0, u1) = (0.5 - glass_w * 0.5, 0.5 + glass_w * 0.5);
    let (v0, v1) = (0.62 - glass_h * 0.5, 0.62 + glass_h * 0.5);

    // Softness of the reveal, in cell widths.
    const REVEAL: f32 = 0.02;
    let pane = smoothstep01((fu - u0) / REVEAL)
        * smoothstep01((u1 - fu) / REVEAL)
        * smoothstep01((fv - v0) / REVEAL)
        * smoothstep01((v1 - fv) / REVEAL);

    Cell {
        column: column as u32,
        row: row as u32,
        pane,
        glass: pane > 0.5,
        up_pane: ((fv - v0) / (v1 - v0)).clamp(0.0, 1.0),
        // Grime runs down from the sill, so only the strip under a pane cares.
        below_pane: (fv < v0 && fu > u0 && fu < u1).then(|| (v0 - fv) / v0.max(1e-3)),
    }
}

pub fn facade(class: FacadeClass) -> FacadeMaps {
    let seed = 500 + class.index() as u32 * 131;

    let base = painted(FACADE_SIZE, TextureFormat::Rgba8UnormSrgb, move |u, v| {
        let cell = cell_at(class, u, v);

        if cell.glass {
            // Glass is dark, and lighter towards the top of the pane where it
            // is reflecting sky rather than the street opposite.
            let tint = hash01(cell.column, cell.row, seed) * 0.10;
            let sky = cell.up_pane * 0.16;
            let blind = if hash01(cell.column, cell.row, seed + 3) > 0.82 {
                // Some panes have a blind pulled down.
                0.22
            } else {
                0.0
            };
            return [
                byte(0.17 + sky * 0.8 + tint + blind),
                byte(0.20 + sky * 0.95 + tint + blind),
                byte(0.25 + sky + tint + blind * 0.9),
                255,
            ];
        }

        // Wall. Kept close to white so the district palette on the material,
        // not the texture, decides what colour the building is.
        let mut value = 0.94 + (fbm(u, v, 40, 4, seed + 11) - 0.5) * 0.09;
        // Floor lines: a shadow where each storey meets the next.
        let (_, rows) = class.grid();
        let storey = (v * rows).fract();
        if storey < 0.04 {
            value -= 0.10 * (1.0 - storey / 0.04);
        }
        // Rain shadow under every sill, fading out as it runs down the wall.
        if let Some(depth) = cell.below_pane {
            let streak = fbm(u * 6.0, v, 24, 3, seed + 29);
            value -= (1.0 - depth).max(0.0) * 0.09 * (0.4 + streak * 0.9);
        }
        // Weathering: the base of a building is always dirtier than its top.
        value -= (1.0 - v).powi(4) * 0.06;

        let c = byte(value);
        [c, c, byte(value * 0.995), 255]
    });

    let emissive = painted(MASK_SIZE, TextureFormat::Rgba8UnormSrgb, move |u, v| {
        let cell = cell_at(class, u, v);
        if !cell.glass {
            return [0, 0, 0, 255];
        }
        // The ground floor is shops and lobbies: nearly always lit.
        let occupancy = if cell.row == 0 {
            0.85
        } else {
            class.occupancy()
        };
        if hash01(cell.column, cell.row, seed + 7) > occupancy {
            return [0, 0, 0, 255];
        }

        let brightness = 0.55 + hash01(cell.column, cell.row, seed + 13) * 0.45;
        // A minority of interiors are fluorescent rather than tungsten, which
        // is what stops a night skyline reading as a single orange wash.
        let cool = hash01(cell.column, cell.row, seed + 17) > 0.72;
        let (r, g, b) = if cool {
            (0.80, 0.92, 1.00)
        } else {
            (1.00, 0.80, 0.52)
        };
        [
            byte(r * brightness),
            byte(g * brightness),
            byte(b * brightness),
            255,
        ]
    });

    // Panes sit back behind the wall, and each storey meets the next in a
    // shadow line. Both are what stop a facade reading as a decal on a box.
    let height = move |u: f32, v: f32| {
        let cell = cell_at(class, u, v);
        let (_, rows) = class.grid();
        let storey = (v * rows).fract();
        // Only the two features that are genuinely three-dimensional. Adding
        // the wall's colour noise here as well made a flat facade look like
        // poured concrete that had gone off badly.
        let reveal = 0.75 - cell.pane * 0.55;
        reveal - smoothstep01(1.0 - storey / 0.04) * 0.22
    };
    let normal = normal_map(MASK_SIZE, 0.018, height);

    let surface = painted(MASK_SIZE, TextureFormat::Rgba8Unorm, move |u, v| {
        let cell = cell_at(class, u, v);
        let (roughness, metallic) = if cell.glass {
            (0.10, 0.55)
        } else {
            (0.88 + (fbm(u, v, 32, 2, seed + 23) - 0.5) * 0.12, 0.0)
        };
        [255, byte(roughness), byte(metallic), 255]
    });

    FacadeMaps {
        base,
        emissive,
        surface,
        normal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_wraps_at_the_period() {
        // The left and right edges of a tile have to agree, or every tiled
        // surface in the game grows a grid of seams.
        for i in 0..64 {
            let v = i as f32 / 64.0;
            let left = fbm(0.0, v, 8, 4, 5);
            let right = fbm(1.0, v, 8, 4, 5);
            assert!((left - right).abs() < 1e-4, "seam at v={v}: {left} {right}");
            let bottom = fbm(v, 0.0, 8, 4, 5);
            let top = fbm(v, 1.0, 8, 4, 5);
            assert!((bottom - top).abs() < 1e-4, "seam at u={v}");
        }
    }

    #[test]
    fn noise_stays_in_range() {
        for i in 0..200 {
            let u = i as f32 / 200.0;
            let value = fbm(u, u * 0.37, 6, 5, 99);
            assert!((0.0..=1.0).contains(&value), "fbm out of range: {value}");
        }
    }

    #[test]
    fn a_painted_image_carries_a_full_mip_chain() {
        let image = paving();
        let levels = image.texture_descriptor.mip_level_count;
        assert_eq!(levels, GROUND_SIZE.ilog2() + 1, "chain must reach 1x1");

        // Every level must be present in the buffer, or wgpu rejects the upload.
        let mut expected = 0usize;
        let mut size = GROUND_SIZE;
        for _ in 0..levels {
            expected += (size * size * 4) as usize;
            size = (size / 2).max(1);
        }
        assert_eq!(image.data.as_ref().map(Vec::len), Some(expected));
    }

    #[test]
    fn facade_classes_follow_height() {
        assert_eq!(FacadeClass::for_height(7.0), FacadeClass::House);
        assert_eq!(FacadeClass::for_height(40.0), FacadeClass::Midrise);
        assert_eq!(FacadeClass::for_height(130.0), FacadeClass::Tower);
        // Indices address the material table, so they must be dense and unique.
        for (i, class) in FacadeClass::ALL.iter().enumerate() {
            assert_eq!(class.index(), i);
        }
    }

    #[test]
    fn windows_are_lit_only_where_there_is_glass() {
        let maps = facade(FacadeClass::Tower);
        let data = maps.emissive.data.as_ref().unwrap();
        let lit = data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 8 || p[2] > 8)
            .count();
        let total = (MASK_SIZE * MASK_SIZE) as usize;
        assert!(lit > 0, "a tower at night should have lit windows");
        assert!(
            lit < total / 2,
            "more than half the facade glowing is not a building, it is a lamp"
        );
    }
}

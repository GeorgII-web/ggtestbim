//! Procedural tileable textures (grayscale detail, tinted by the material color) with a full mip chain.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const SIZE: usize = 256;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tex {
    Plaster,
    Brick,
    Tiles,
    Asphalt,
    Grass,
    Wood,
    Concrete,
    Stone,
    Leaves,
    Water,
    Bark,
}

pub fn image(tex: Tex) -> Image {
    let mut level: Vec<[f32; 3]> = (0..SIZE * SIZE)
        .map(|i| pixel(tex, (i % SIZE) as i32, (i / SIZE) as i32))
        .collect();
    let mut data = Vec::new();
    let mut size = SIZE;
    let mut mips = 0;
    loop {
        for p in &level {
            data.extend(p.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8));
            data.push(255);
        }
        mips += 1;
        if size == 1 {
            break;
        }
        let s = size / 2;
        level = (0..s * s)
            .map(|i| {
                let (x, y) = (i % s * 2, i / s * 2);
                let at = |dx: usize, dy: usize| level[(y + dy) * size + x + dx];
                let q = [at(0, 0), at(1, 0), at(0, 1), at(1, 1)];
                [0, 1, 2].map(|c| q.iter().map(|p| p[c]).sum::<f32>() / 4.0)
            })
            .collect();
        size = s;
    }
    let mut image = Image::new_uninit(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.data = Some(data);
    image.texture_descriptor.mip_level_count = mips;
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..Default::default()
    });
    image
}

fn pixel(tex: Tex, x: i32, y: i32) -> [f32; 3] {
    let n = SIZE as i32;
    let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
    let grain = hash(x, y, 1) - 0.5;
    let g = |l: f32| [l, l, l];
    match tex {
        Tex::Plaster => g(0.86 + 0.1 * fbm(u, v, 4, 2) + 0.05 * grain),
        Tex::Brick => {
            // 4 x 8 bricks per tile, every other row shifted by half a brick.
            let row = y / 32;
            let xs = (x + if row % 2 == 1 { 32 } else { 0 }) % n;
            if xs % 64 < 3 || y % 32 < 3 {
                g(0.97 + 0.03 * fbm(u, v, 8, 3))
            } else {
                g(0.7 + 0.18 * hash(xs / 64, row, 4) + 0.08 * fbm(u, v, 16, 5) + 0.06 * grain)
            }
        }
        Tex::Tiles => {
            if x % 64 < 2 || y % 64 < 2 {
                g(0.55)
            } else {
                g(0.8 + 0.12 * hash(x / 64, y / 64, 6) + 0.06 * fbm(u, v, 8, 7) + 0.04 * grain)
            }
        }
        Tex::Stone => {
            let row = y / 64;
            let xs = (x + if row % 2 == 1 { 64 } else { 0 }) % n;
            if xs % 128 < 3 || y % 64 < 3 {
                g(0.6)
            } else {
                g(0.74 + 0.16 * hash(xs / 128, row, 8) + 0.08 * fbm(u, v, 8, 9) + 0.05 * grain)
            }
        }
        Tex::Asphalt => {
            let speck = if hash(x, y, 10) > 0.93 { 0.2 } else { 0.0 };
            g(0.72 + 0.1 * fbm(u, v, 8, 11) + speck + 0.08 * grain)
        }
        Tex::Grass => {
            let l = 0.6 + 0.25 * fbm(u, v, 8, 12) + 0.25 * (hash(x, y / 3, 13) - 0.5);
            [l * 0.95, l, l * 0.85]
        }
        Tex::Wood => {
            // 8 planks along u, boards of half a tile with shifted joints.
            let plank = y / 32;
            let xs = (x + (hash(plank, 0, 14) * 128.0) as i32) % n;
            if y % 32 < 1 || xs % 128 < 1 {
                g(0.4)
            } else {
                g(0.68
                    + 0.14 * hash(xs / 128, plank, 15)
                    + 0.12 * noise(u, v, 4, 64, 16)
                    + 0.04 * grain)
            }
        }
        Tex::Concrete => g(0.82 + 0.12 * fbm(u, v, 4, 17) + 0.06 * grain),
        Tex::Leaves => g(0.45 + 0.5 * fbm(u, v, 16, 18) + 0.15 * grain),
        Tex::Water => g(0.85 + 0.1 * noise(u, v, 4, 16, 19) + 0.05 * fbm(u, v, 8, 20)),
        Tex::Bark => g(0.55 + 0.35 * noise(u, v, 16, 2, 21) + 0.1 * grain),
    }
}

fn hash(x: i32, y: i32, s: u32) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ s.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xff_ffff) as f32 / 16_777_216.0
}

/// Tileable value noise with `pu` x `pv` cells over the unit square.
fn noise(u: f32, v: f32, pu: i32, pv: i32, s: u32) -> f32 {
    let (x, y) = (u * pu as f32, v * pv as f32);
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
    let (fx, fy) = (smooth(x - xi as f32), smooth(y - yi as f32));
    let h = |a: i32, b: i32| hash(a.rem_euclid(pu), b.rem_euclid(pv), s);
    let top = h(xi, yi) * (1.0 - fx) + h(xi + 1, yi) * fx;
    let bottom = h(xi, yi + 1) * (1.0 - fx) + h(xi + 1, yi + 1) * fx;
    top * (1.0 - fy) + bottom * fy
}

/// Four octaves of tileable noise, roughly 0..1.
fn fbm(u: f32, v: f32, period: i32, s: u32) -> f32 {
    (0..4)
        .map(|k| noise(u, v, period << k, period << k, s + k) / (1 << k) as f32)
        .sum::<f32>()
        / 1.875
}

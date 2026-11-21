//! Procedural city on several levels: a street grid of blocks raised on terraces (stairs up from the street),
//! each block a building (rooms, doors, windows, stairs up to a walkable roof; a rectangular core, often
//! with wings that make an L, T, U or cross shape, sometimes a tower of 10-18 floors), an open plaza, a park
//! or a deep pit (stairs down along its walls, anomaly artifacts at the bottom).
//! Neighbouring buildings are joined over the street by covered corridors between upper floors or open
//! bridges between roofs, with stairs where the heights differ.
//! Everything is an axis-aligned box: the list is the collision world of the player, and the boxes of each
//! material are merged into one mesh with world-space UVs, so textures tile at a fixed size in meters.
//! Faces inside a building or covered corridor get an indoor copy of their material that lets in only
//! `INDOOR_AMBIENT` of the sky's ambient light: rooms are lit by the sun through windows and doors.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use std::collections::HashMap;

use crate::player::Player;
use crate::textures::{self, Tex};

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        let seed = world_seed();
        app.insert_resource(Level {
            chunks: HashMap::new(),
            rng: Rng(seed),
            new_chunks: Vec::new(),
            new_spots: Vec::new(),
            new_artifacts: Vec::new(),
        })
        .insert_resource(PropBoxes(Vec::new()))
        .add_systems(Startup, (setup_palette, initial_load).chain())
        .add_systems(
            Update,
            ((regenerate, stream_chunks).chain(), prop_boxes, haunt),
        );
    }
}

#[derive(Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

/// The loaded chunks: their collision boxes and render entities. A chunk that is unloaded and later
/// needed again is generated anew with a fresh random seed.
#[derive(Resource)]
pub struct Level {
    chunks: HashMap<IVec2, Chunk>,
    pub(crate) rng: Rng,
    /// Chunks generated since the cars module last took them, to put cars on their streets.
    pub(crate) new_chunks: Vec<IVec2>,
    /// Watcher spots of the chunks generated since the watchers module last took them.
    pub(crate) new_spots: Vec<Vec<Spot>>,
    /// Artifact places (pit bottoms) of the chunks generated since the artifacts module last took them.
    pub(crate) new_artifacts: Vec<Vec3>,
}

struct Chunk {
    boxes: Vec<Aabb>,
    entities: Vec<Entity>,
}

impl Level {
    /// Collision boxes of the chunk under `p` and its eight neighbours.
    pub fn near(&self, p: Vec3) -> Vec<&[Aabb]> {
        let c = chunk_of(p);
        let mut v = Vec::new();
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(ch) = self.chunks.get(&(c + IVec2::new(dx, dz))) {
                    v.push(&ch.boxes[..]);
                }
            }
        }
        v
    }

    /// Whether a level box cuts the segment from `a` to `b` (line of sight). Only the chunks around `b` are
    /// tested, enough for sight lines up to about 60 m beyond their edge.
    pub fn blocked(&self, a: Vec3, b: Vec3) -> bool {
        let (lo, hi, d) = (a.min(b), a.max(b), b - a);
        self.near(b).iter().any(|boxes| {
            boxes.iter().any(|bx| {
                !(bx.max.cmplt(lo).any() || bx.min.cmpgt(hi).any()) && segment_hits(a, d, bx)
            })
        })
    }

    /// Nearest level box the ray `o + t * dir` hits within `max` (the chunks around `o`): distance and face normal.
    pub fn raycast(&self, o: Vec3, dir: Vec3, max: f32) -> Option<(f32, Vec3)> {
        self.near(o)
            .iter()
            .flat_map(|boxes| boxes.iter())
            .filter_map(|bx| ray_box(o, dir, max, bx))
            .min_by(|a, b| a.0.total_cmp(&b.0))
    }

    /// Copies of the level boxes overlapping the region from `min` to `max` (the chunks around its center),
    /// for searches that test many short segments.
    pub fn boxes_in(&self, min: Vec3, max: Vec3) -> Vec<Aabb> {
        self.near((min + max) / 2.0)
            .iter()
            .flat_map(|boxes| boxes.iter())
            .filter(|b| b.max.cmpgt(min).all() && b.min.cmplt(max).all())
            .copied()
            .collect()
    }

    /// Whether the ground under `p` exists yet (the player waits for it, cars outside it are removed).
    pub fn loaded(&self, p: Vec3) -> bool {
        self.chunks.contains_key(&chunk_of(p))
    }
}

/// First hit of the ray `o + t * dir` (dir normalized, 0 < t < max) on the box: distance and the outward
/// normal of the face it enters through. A ray starting inside the box doesn't hit it.
pub(crate) fn ray_box(o: Vec3, dir: Vec3, max: f32, bx: &Aabb) -> Option<(f32, Vec3)> {
    let (mut t0, mut t1) = (0.0f32, max);
    let mut normal = Vec3::ZERO;
    for i in 0..3 {
        if dir[i].abs() < 1e-6 {
            if o[i] <= bx.min[i] || o[i] >= bx.max[i] {
                return None;
            }
            continue;
        }
        let (u, v) = ((bx.min[i] - o[i]) / dir[i], (bx.max[i] - o[i]) / dir[i]);
        let (near, far) = (u.min(v), u.max(v));
        if near > t0 {
            t0 = near;
            normal = Vec3::ZERO;
            normal[i] = -dir[i].signum();
        }
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    (normal != Vec3::ZERO).then_some((t0, normal))
}

/// Whether one of `boxes` cuts the segment from `a` to `b`.
pub(crate) fn segment_blocked(boxes: &[Aabb], a: Vec3, b: Vec3) -> bool {
    let (lo, hi, d) = (a.min(b), a.max(b), b - a);
    boxes
        .iter()
        .any(|bx| !(bx.max.cmplt(lo).any() || bx.min.cmpgt(hi).any()) && segment_hits(a, d, bx))
}

/// Slab test: does `a + t * d` for t in 0..1 pass through the box?
fn segment_hits(a: Vec3, d: Vec3, bx: &Aabb) -> bool {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for i in 0..3 {
        if d[i].abs() < 1e-6 {
            if a[i] <= bx.min[i] || a[i] >= bx.max[i] {
                return false;
            }
        } else {
            let (u, v) = ((bx.min[i] - a[i]) / d[i], (bx.max[i] - a[i]) / d[i]);
            t0 = t0.max(u.min(v));
            t1 = t1.min(u.max(v));
            if t0 >= t1 {
                return false;
            }
        }
    }
    true
}

/// Where a watcher stands (feet), and its escapes: each a list of waypoints to a place out of sight from
/// some side; the watcher picks the one that ends hidden from the player.
#[derive(Clone)]
pub(crate) struct Spot {
    pub(crate) pos: Vec3,
    pub(crate) escapes: Vec<Vec<Vec3>>,
    pub(crate) kind: SpotKind,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum SpotKind {
    /// Behind a window; escapes: a step aside behind the wall next to the window, or deeper into the room.
    Window,
    /// Outside, pressed to a wall right at a building corner; escapes: around the corner, or back along
    /// its wall (hidden from the other side).
    Corner,
    /// On the roof at a parapet corner; escapes back from the edge.
    Roof,
}

/// Blocks per chunk side, chunk size in meters, chunks kept loaded around the player (square radius),
/// chunks generated per frame while walking (the first load at startup has no limit).
pub(crate) const CHUNK: i32 = 2;
pub(crate) const CHUNK_SIZE: f32 = CHUNK as f32 * PITCH;
const LOAD_RADIUS: i32 = 2;
const LOADS_PER_FRAME: usize = 1;

/// Random seed from the clock: every start is a different world.
fn world_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}

fn chunk_of(p: Vec3) -> IVec2 {
    IVec2::new(
        (p.x / CHUNK_SIZE).floor() as i32,
        (p.z / CHUNK_SIZE).floor() as i32,
    )
}

#[derive(Component)]
struct LevelPart;

/// Block size and pitch (block + street); streets run along x, z = k * PITCH, so (0, 0) is a crossing.
const BLOCK: f32 = 22.0;
pub(crate) const PITCH: f32 = 30.0;
const STREET: f32 = PITCH - BLOCK;
/// Story height, outer wall thickness, stair step rise and run.
pub(crate) const FH: f32 = 3.5;
pub(crate) const T: f32 = 0.3;
const RISE: f32 = 0.25;
const RUN: f32 = 0.3;
const TERRACE: f32 = 1.5;
/// Width of one stair lane (a stairwell has two).
pub(crate) const LANE: f32 = 1.4;
/// Largest height difference a passage can bridge with stairs over one street (steps of at least 0.28 m).
const MAX_PASSAGE_RISE: f32 = 7.0;

#[derive(Clone, Copy)]
enum Mat {
    Asphalt,
    Paving,
    Grass,
    Stair,
    Trunk,
    Leaves,
    Water,
    Stone,
    Interior,
    Metal,
    /// Random per world: floor and outer wall styles (texture kind, pattern seed, color, tile size).
    Floor(u8),
    Wall(u8),
}

const FIXED: usize = 10;
const FLOORS: u8 = 4;
const WALLS: u8 = 16;
const MATS: usize = FIXED + FLOORS as usize + WALLS as usize;

impl Mat {
    fn index(self) -> usize {
        match self {
            Mat::Asphalt => 0,
            Mat::Paving => 1,
            Mat::Grass => 2,
            Mat::Stair => 3,
            Mat::Trunk => 4,
            Mat::Leaves => 5,
            Mat::Water => 6,
            Mat::Stone => 7,
            Mat::Interior => 8,
            Mat::Metal => 9,
            Mat::Floor(i) => FIXED + i as usize,
            Mat::Wall(i) => FIXED + FLOORS as usize + i as usize,
        }
    }
}

/// Fixed materials by index: color tint, texture, texture tile size in meters, roughness.
fn fixed_look(i: usize) -> ([f32; 3], Tex, f32, f32) {
    [
        ([0.34, 0.34, 0.36], Tex::Asphalt, 4.0, 0.95),
        ([0.78, 0.74, 0.68], Tex::Tiles, 4.0, 0.85),
        ([0.38, 0.62, 0.3], Tex::Grass, 3.0, 0.95),
        ([0.72, 0.72, 0.72], Tex::Concrete, 2.0, 0.9),
        ([0.45, 0.3, 0.18], Tex::Bark, 1.0, 0.95),
        ([0.32, 0.56, 0.26], Tex::Leaves, 2.0, 0.9),
        ([0.3, 0.52, 0.82], Tex::Water, 3.0, 0.08),
        ([0.78, 0.75, 0.7], Tex::Stone, 4.0, 0.9),
        ([0.93, 0.9, 0.84], Tex::Plaster, 4.0, 0.9),
        ([0.3, 0.3, 0.33], Tex::Concrete, 2.0, 0.4),
    ][i]
}

#[derive(Resource)]
struct Palette {
    mats: Vec<Handle<StandardMaterial>>,
    /// The same materials for faces inside buildings: ambient light cut to `INDOOR_AMBIENT`.
    indoor: Vec<Handle<StandardMaterial>>,
    /// Texture tile size in meters per material index.
    scales: Vec<f32>,
    /// Unit cube for props, scaled per part.
    cube: Handle<Mesh>,
    /// Odd colors a prop can turn into behind the player's back.
    crazy: Vec<Handle<StandardMaterial>>,
}

fn setup_palette(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut textures: HashMap<(Tex, u32), Handle<Image>> = HashMap::new();
    let mut texture = |tex: Tex, seed: u32| {
        textures
            .entry((tex, seed))
            .or_insert_with(|| images.add(textures::image(tex, seed)))
            .clone()
    };
    let mut rng = Rng(world_seed() ^ 0xA5A5);
    let mut mats = Vec::with_capacity(MATS);
    let mut scales = Vec::with_capacity(MATS);
    for i in 0..MATS {
        let (color, tex, seed, scale, roughness) = if i < FIXED {
            let (c, tex, scale, roughness) = fixed_look(i);
            (Color::srgb(c[0], c[1], c[2]), tex, 0, scale, roughness)
        } else {
            let floor = i < FIXED + FLOORS as usize;
            let kinds: &[Tex] = if floor {
                &textures::FLOOR_KINDS
            } else {
                &textures::WALL_KINDS
            };
            let tex = kinds[rng.below(kinds.len() as u32) as usize];
            let color = Color::hsl(
                rng.range(0.0, 360.0),
                rng.range(0.1, 0.6),
                rng.range(0.45, 0.8),
            );
            let scale = if floor {
                rng.range(2.0, 4.0)
            } else {
                rng.range(2.0, 6.0)
            };
            (
                color,
                tex,
                1 + rng.below(1 << 20),
                scale,
                rng.range(0.5, 0.95),
            )
        };
        mats.push(materials.add(StandardMaterial {
            base_color: color,
            base_color_texture: Some(texture(tex, seed)),
            perceptual_roughness: roughness,
            ..default()
        }));
        scales.push(scale);
    }
    let leaves = texture(Tex::Leaves, 0);
    // A 1x1 occlusion map: the shader multiplies only ambient (indirect) light by it, so the sun through a
    // window still lights a room fully while the rest of it stays in shade.
    let shade = images.add(Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[(INDOOR_AMBIENT * 255.0) as u8],
        TextureFormat::R8Unorm,
        RenderAssetUsages::default(),
    ));
    let indoor = mats
        .iter()
        .map(|m| {
            let mut mat = materials.get(m).expect("material just added").clone();
            mat.occlusion_texture = Some(shade.clone());
            // A faint glow of its own (textured like the surface), so rooms are never fully dark. Scaled by
            // the camera exposure like the lights (Bevy's default weight 0 shows it as raw HDR, far too bright).
            mat.emissive = mat.base_color.to_linear() * INDOOR_GLOW;
            mat.emissive_texture = mat.base_color_texture.clone();
            mat.emissive_exposure_weight = 1.0;
            materials.add(mat)
        })
        .collect();
    let crazy = [
        (0.85, 0.2, 0.15),
        (0.55, 0.25, 0.75),
        (0.95, 0.75, 0.15),
        (0.2, 0.8, 0.85),
        (0.95, 0.45, 0.7),
        (0.95, 0.95, 0.95),
    ]
    .iter()
    .map(|&(r, g, b)| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            base_color_texture: Some(leaves.clone()),
            perceptual_roughness: 0.9,
            ..default()
        })
    })
    .collect();
    commands.insert_resource(Palette {
        mats,
        indoor,
        scales,
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        crazy,
    });
}

fn initial_load(
    mut commands: Commands,
    palette: Res<Palette>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut level: ResMut<Level>,
) {
    stream(
        &mut level,
        &mut commands,
        &mut meshes,
        &palette,
        Vec3::ZERO,
        usize::MAX,
    );
}

fn stream_chunks(
    mut commands: Commands,
    palette: Res<Palette>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut level: ResMut<Level>,
    player: Single<&Transform, With<Player>>,
) {
    stream(
        &mut level,
        &mut commands,
        &mut meshes,
        &palette,
        player.translation,
        LOADS_PER_FRAME,
    );
}

/// Unloads chunks more than one ring beyond the radius (the margin stops flicker on a chunk border), then
/// generates up to `max_loads` missing chunks inside the radius, nearest first.
fn stream(
    level: &mut Level,
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    palette: &Palette,
    pos: Vec3,
    max_loads: usize,
) {
    let center = chunk_of(pos);
    let far: Vec<IVec2> = level
        .chunks
        .keys()
        .filter(|&&c| (c - center).abs().max_element() > LOAD_RADIUS + 1)
        .copied()
        .collect();
    for c in far {
        if let Some(chunk) = level.chunks.remove(&c) {
            for e in chunk.entities {
                commands.entity(e).despawn();
            }
        }
    }
    let mut missing: Vec<IVec2> = (-LOAD_RADIUS..=LOAD_RADIUS)
        .flat_map(|dx| (-LOAD_RADIUS..=LOAD_RADIUS).map(move |dz| center + IVec2::new(dx, dz)))
        .filter(|c| !level.chunks.contains_key(c))
        .collect();
    missing.sort_by_key(|&c| (c - center).length_squared());
    for c in missing.into_iter().take(max_loads) {
        let seed = level.rng.next();
        let (parts, props, spots, rooms, artifacts) = generate(seed, c);
        let mut groups: Vec<Vec<Aabb>> = vec![Vec::new(); MATS];
        for (b, mat) in &parts {
            groups[mat.index()].push(*b);
        }
        let mut entities: Vec<Entity> = Vec::new();
        for (i, boxes) in groups.iter().enumerate().filter(|(_, b)| !b.is_empty()) {
            let scale = palette.scales[i];
            for (inside, mats) in [(false, &palette.mats), (true, &palette.indoor)] {
                let Some(mesh) = box_mesh(boxes, scale, |face| indoor(&rooms, face) == inside)
                else {
                    continue;
                };
                entities.push(
                    commands
                        .spawn((
                            Mesh3d(meshes.add(mesh)),
                            MeshMaterial3d(mats[i].clone()),
                            LevelPart,
                        ))
                        .id(),
                );
            }
        }
        for prop in props {
            let children: Vec<Entity> = prop
                .parts
                .iter()
                .map(|(b, mat)| {
                    commands
                        .spawn((
                            Mesh3d(palette.cube.clone()),
                            MeshMaterial3d(palette.mats[mat.index()].clone()),
                            Transform::from_translation((b.min + b.max) / 2.0)
                                .with_scale(b.max - b.min),
                        ))
                        .id()
                })
                .collect();
            let boxes = prop.parts.iter().map(|(b, _)| *b).collect();
            let root = commands
                .spawn((
                    Transform::from_translation(prop.base),
                    Visibility::default(),
                    PropState {
                        boxes,
                        children: children.clone(),
                    },
                    LevelPart,
                ))
                .add_children(&children)
                .id();
            entities.push(root);
        }
        let boxes = parts.into_iter().map(|(b, _)| b).collect();
        level.chunks.insert(c, Chunk { boxes, entities });
        level.new_chunks.push(c);
        level.new_spots.push(spots);
        level.new_artifacts.extend(artifacts);
    }
}

/// N drops the whole world; it is generated anew around the spawn point, where the player module puts
/// the player back.
fn regenerate(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    parts: Query<Entity, With<LevelPart>>,
    palette: Res<Palette>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut level: ResMut<Level>,
) {
    if !keys.just_pressed(KeyCode::KeyN) {
        return;
    }
    for e in &parts {
        commands.entity(e).despawn();
    }
    level.chunks.clear();
    stream(
        &mut level,
        &mut commands,
        &mut meshes,
        &palette,
        Vec3::ZERO,
        usize::MAX,
    );
}

#[derive(Component)]
pub(crate) struct PropState {
    /// Part boxes relative to the prop origin, before its transform.
    boxes: Vec<Aabb>,
    children: Vec<Entity>,
}

/// World boxes of the props near the player, rebuilt every frame (props move and change size).
#[derive(Resource)]
pub struct PropBoxes(pub Vec<Aabb>);

pub(crate) fn prop_boxes(
    props: Query<(&PropState, &Transform)>,
    player: Single<&Transform, With<Player>>,
    mut out: ResMut<PropBoxes>,
) {
    out.0.clear();
    for (prop, tf) in &props {
        if tf.translation.distance_squared(player.translation) > 15.0 * 15.0 {
            continue;
        }
        for b in &prop.boxes {
            let corners = (0..8).map(|i| {
                let c = Vec3::new(
                    if i & 1 == 0 { b.min.x } else { b.max.x },
                    if i & 2 == 0 { b.min.y } else { b.max.y },
                    if i & 4 == 0 { b.min.z } else { b.max.z },
                );
                tf.transform_point(c)
            });
            let (min, max) =
                corners.fold((Vec3::MAX, Vec3::MIN), |(lo, hi), c| (lo.min(c), hi.max(c)));
            out.0.push(Aabb { min, max });
        }
    }
}

/// Every 1.5 s each prop behind the player (more than 100 degrees from the view direction, 5 to 70 m away) has
/// a 4 % chance to change: grow or shrink, stretch, turn, or take an odd color.
fn haunt(
    time: Res<Time>,
    mut clock: Local<f32>,
    camera: Single<&Transform, With<Player>>,
    mut props: Query<(&mut Transform, &PropState), Without<Player>>,
    mut materials: Query<&mut MeshMaterial3d<StandardMaterial>>,
    palette: Res<Palette>,
    mut level: ResMut<Level>,
) {
    *clock += time.delta_secs();
    if *clock < 1.5 {
        return;
    }
    *clock = 0.0;
    let forward = camera.forward().as_vec3();
    for (mut tf, prop) in &mut props {
        let to = tf.translation - camera.translation;
        let dist = to.length();
        if !(5.0..70.0).contains(&dist)
            || to.normalize().dot(forward) > -0.17
            || !level.rng.chance(0.04)
        {
            continue;
        }
        let rng = &mut level.rng;
        match rng.below(4) {
            0 => {
                tf.scale =
                    (tf.scale * rng.range(0.5, 1.6)).clamp(Vec3::splat(0.3), Vec3::splat(3.0))
            }
            1 => tf.scale.y = (tf.scale.y * rng.range(0.5, 1.8)).clamp(0.3, 3.0),
            2 => tf.rotate_y(rng.range(0.5, 2.5)),
            _ => {
                let child = prop.children[rng.below(prop.children.len() as u32) as usize];
                if let Ok(mut m) = materials.get_mut(child) {
                    m.0 = palette.crazy[rng.below(palette.crazy.len() as u32) as usize].clone();
                }
            }
        }
    }
}

/// Share of the sky's ambient light that reaches faces inside buildings.
const INDOOR_AMBIENT: f32 = 0.15;
/// Indoor faces also glow by themselves at this many nits times their color (about the light a room gets
/// from an ambient of 15): at night a very dark room with the walls barely visible, never black.
const INDOOR_GLOW: f32 = 15.0;

/// True when a face (a point just in front of its center) lies in one of the indoor volumes.
fn indoor(rooms: &[Aabb], p: Vec3) -> bool {
    rooms
        .iter()
        .any(|r| p.cmpgt(r.min).all() && p.cmplt(r.max).all())
}

/// The faces of all boxes of one material that `keep` accepts (it gets a point 5 cm in front of the face
/// center) as a single mesh, or None when no face is kept; UVs are world coordinates divided by the texture
/// tile size.
fn box_mesh(boxes: &[Aabb], scale: f32, keep: impl Fn(Vec3) -> bool) -> Option<Mesh> {
    let mut positions = Vec::with_capacity(boxes.len() * 24);
    let mut normals = Vec::with_capacity(boxes.len() * 24);
    let mut uvs = Vec::with_capacity(boxes.len() * 24);
    let mut indices: Vec<u32> = Vec::with_capacity(boxes.len() * 36);
    for b in boxes {
        for a in 0..3 {
            // (b, c) are the face axes with b x c = a, so the corner order below is counter-clockwise from +a.
            let (ab, ac) = ((a + 1) % 3, (a + 2) % 3);
            for positive in [false, true] {
                let mut n = [0.0; 3];
                n[a] = if positive { 1.0 } else { -1.0 };
                if !keep(
                    (b.min + b.max) / 2.0
                        + Vec3::from_array(n) * ((b.max[a] - b.min[a]) / 2.0 + 0.05),
                ) {
                    continue;
                }
                let base = positions.len() as u32;
                for (hb, hc) in [(false, false), (true, false), (true, true), (false, true)] {
                    let mut p = Vec3::ZERO;
                    p[a] = if positive { b.max[a] } else { b.min[a] };
                    p[ab] = if hb { b.max[ab] } else { b.min[ab] };
                    p[ac] = if hc { b.max[ac] } else { b.min[ac] };
                    positions.push(p.to_array());
                    normals.push(n);
                    let t = match a {
                        0 => [p.z, -p.y],
                        1 => [p.x, p.z],
                        _ => [p.x, -p.y],
                    };
                    uvs.push([t[0] / scale, t[1] / scale]);
                }
                let quad = if positive {
                    [0, 1, 2, 0, 2, 3]
                } else {
                    [0, 2, 1, 0, 3, 2]
                };
                indices.extend(quad.map(|q| base + q));
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    Some(
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
        .with_inserted_indices(Indices::U32(indices)),
    )
}

/// SplitMix64: small, seedable, no dependency.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub(crate) fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub(crate) fn range(&mut self, a: f32, b: f32) -> f32 {
        if b <= a { a } else { a + (b - a) * self.unit() }
    }
    pub(crate) fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
    pub(crate) fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// A gap in a wall: center and width along the wall, bottom and top above the wall base.
#[derive(Clone, Copy)]
struct Opening {
    c: f32,
    w: f32,
    bottom: f32,
    top: f32,
}

fn door_at(c: f32) -> Opening {
    Opening {
        c,
        w: 1.4,
        bottom: 0.0,
        top: 2.4,
    }
}

/// A gap a passage needs in a building wall. `floor == floors` is the roof parapet.
/// Sides: 0 south (z0), 1 north (z1), 2 west (x0), 3 east (x1).
struct Gap {
    floor: u32,
    side: u32,
    opening: Opening,
}

struct Building {
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    floors: u32,
    wall: Mat,
    floor: Mat,
    /// Interior walls: along z at `xm`, along x at `zm`.
    xm: f32,
    zm: f32,
    gaps: Vec<Gap>,
    /// Lower annexes on the sides without passages.
    wings: Vec<Wing>,
}

/// An annex built against one side of a building's core (`side` as in `Gap`): one open room per floor,
/// windows, a street door on its far wall, a door into the core on every floor and one from the core out
/// onto its roof when the core is higher.
struct Wing {
    side: u32,
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    floors: u32,
}

enum Kind {
    Building(Building),
    Plaza,
    Park,
    /// A pit dug from the street.
    Pit(PitPlan),
}

/// A pit's hole (world x / z), depth, and how its stair layout is turned: built in a local frame with the
/// stairs starting at the local south-west corner, then `swap` exchanges the axes and `flip_x` / `flip_z`
/// mirror them, so the stairs start in any corner and turn either way.
#[derive(Clone, Copy)]
struct PitPlan {
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    depth: f32,
    swap: bool,
    flip_x: bool,
    flip_z: bool,
}

impl PitPlan {
    /// A point of the pit's local frame in the world.
    fn world(self, l: Vec3) -> Vec3 {
        let (a, c) = if self.swap { (l.z, l.x) } else { (l.x, l.z) };
        let x = if self.flip_x {
            self.x1 - a
        } else {
            self.x0 + a
        };
        let z = if self.flip_z {
            self.z1 - c
        } else {
            self.z0 + c
        };
        Vec3::new(x, l.y, z)
    }
}

/// What stands in one quarter of a pit floor.
#[derive(Clone, Copy)]
enum Feature {
    Empty,
    Pillars,
    Ruin,
    Platform,
    Mound,
    Pool,
    Boulder,
}

struct Block {
    x0: f32,
    z0: f32,
    /// Terrace height of the block.
    e: f32,
    kind: Kind,
}

/// A walkway over a street between two buildings, along x or z: flat over the block margins at each
/// building's height, stairs across the street. `covered` is a corridor with walls and a roof, else a bridge.
struct Passage {
    along_x: bool,
    /// Building A face, A block edge, B block edge, building B face.
    u: [f32; 4],
    v: f32,
    ya: f32,
    yb: f32,
    covered: bool,
}

struct Builder {
    rng: Rng,
    parts: Vec<(Aabb, Mat)>,
    props: Vec<Prop>,
    /// Places where a watcher can stand, with their escapes.
    spots: Vec<Spot>,
    /// Indoor air volumes (inside the outer walls of buildings and covered corridors): faces in them get
    /// the indoor materials.
    rooms: Vec<Aabb>,
    /// Where anomaly artifacts float (pit bottoms).
    artifacts: Vec<Vec3>,
}

/// A movable object (tree, bench): its own entity, so it can change while the player looks away.
/// Part boxes are relative to `base`.
struct Prop {
    base: Vec3,
    parts: Vec<(Aabb, Mat)>,
}

/// Random character of a chunk: share of buildings and plazas (the rest are parks), floor count range,
/// highest hill in terrace levels, number of hills, share of building pairs joined by a passage, chance of
/// a building being a tower, chance of one block being a pit.
struct Theme {
    building: f32,
    plaza: f32,
    floors: (u32, u32),
    peak: f32,
    hills: u32,
    connect: f32,
    tower: f32,
    pit: f32,
}

/// Tower floor range; pit margin to the block edge, depth range and stair width; wing depth range.
const TOWER: (u32, u32) = (10, 18);
const PIT_MARGIN: f32 = 1.0;
const PIT_STAIR: f32 = 1.4;
/// Pit kinds: (chance, side range, depth range): a narrow deep shaft, a wide shallow crater, a pit between.
type Range = (f32, f32);
const PIT_KINDS: [(f32, Range, Range); 3] = [
    (0.35, (9.0, 13.0), (16.0, 32.0)),
    (0.3, (16.0, 20.0), (4.0, 8.0)),
    (0.35, (12.0, 20.0), (8.0, 22.0)),
];
const WING: (f32, f32) = (3.5, 7.0);

/// One chunk: ground, road marks, then blocks and passages from a theme picked at random.
#[allow(clippy::type_complexity)]
fn generate(
    seed: u64,
    chunk: IVec2,
) -> (Vec<(Aabb, Mat)>, Vec<Prop>, Vec<Spot>, Vec<Aabb>, Vec<Vec3>) {
    let mut b = Builder {
        rng: Rng(seed),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
        rooms: Vec::new(),
        artifacts: Vec::new(),
    };
    let origin = chunk.as_vec2() * CHUNK_SIZE;
    b.road_marks(origin);
    let theme = match b.rng.below(4) {
        // Downtown: towers close together, many passages.
        0 => Theme {
            building: 0.85,
            plaza: 0.1,
            floors: (3, 6),
            peak: 1.0,
            hills: 1,
            connect: 0.9,
            tower: 0.3,
            pit: 0.15,
        },
        // Hills: high terraces with plazas.
        1 => Theme {
            building: 0.45,
            plaza: 0.3,
            floors: (1, 3),
            peak: 5.0,
            hills: 2,
            connect: 0.6,
            tower: 0.05,
            pit: 0.3,
        },
        // Parkland: low houses among parks.
        2 => Theme {
            building: 0.3,
            plaza: 0.2,
            floors: (1, 2),
            peak: 2.0,
            hills: 2,
            connect: 0.5,
            tower: 0.03,
            pit: 0.4,
        },
        _ => Theme {
            building: 0.55,
            plaza: 0.25,
            floors: (1, 4),
            peak: 3.0,
            hills: 2,
            connect: 0.7,
            tower: 0.1,
            pit: 0.25,
        },
    };
    let mut blocks = b.plan(chunk, &theme);
    b.ground(origin, &blocks);
    let passages = b.connect(&mut blocks, &theme);
    b.wings(&mut blocks);
    for block in &blocks {
        b.terrace(block);
        let (x0, z0, e) = (block.x0, block.z0, block.e);
        match &block.kind {
            Kind::Building(bl) => b.building(bl, e),
            Kind::Plaza => b.plaza(x0, z0, x0 + BLOCK, z0 + BLOCK, e),
            Kind::Park => b.park(x0, z0, x0 + BLOCK, z0 + BLOCK, e),
            Kind::Pit(plan) => b.pit(plan),
        }
    }
    for p in &passages {
        b.passage(p);
    }
    // Spots inside a box (a corner covered by a wing) are dropped; escapes that run into a box (a window next
    // to an interior wall) too, and spots left without one.
    let spots = b
        .spots
        .into_iter()
        .filter_map(|mut spot| {
            let pos = spot.pos;
            let chest = pos + Vec3::Y * 1.2;
            if b.parts
                .iter()
                .any(|(bx, _)| chest.cmpgt(bx.min).all() && chest.cmplt(bx.max).all())
            {
                return None;
            }
            spot.escapes.retain(|e| escape_clear(&b.parts, pos, e));
            (!spot.escapes.is_empty()).then_some(spot)
        })
        .collect();
    (b.parts, b.props, spots, b.rooms, b.artifacts)
}

/// From `pos` through the waypoints, at knee, waist and head height, no box is in the way.
fn escape_clear(parts: &[(Aabb, Mat)], pos: Vec3, escape: &[Vec3]) -> bool {
    let mut from = pos;
    escape.iter().all(|&to| {
        let clear = [0.5, 1.2, 2.3].iter().all(|&h| {
            let (a, b) = (from + Vec3::Y * h, to + Vec3::Y * h);
            !parts.iter().any(|(bx, _)| segment_hits(a, b - a, bx))
        });
        from = to;
        clear
    })
}

/// Index of a block inside its chunk.
fn block_index(i: i32, j: i32) -> Option<usize> {
    let r = 0..CHUNK;
    (r.contains(&i) && r.contains(&j)).then(|| (i * CHUNK + j) as usize)
}

impl Builder {
    fn add(&mut self, min: Vec3, max: Vec3, mat: Mat) {
        let s = max - min;
        if s.x > 0.01 && s.y > 0.01 && s.z > 0.01 {
            self.parts.push((Aabb { min, max }, mat));
        }
    }

    /// Box given along/across a direction: `u` along x when `along_x`, else along z; `v` the other one.
    #[allow(clippy::too_many_arguments)]
    fn add_uv(
        &mut self,
        along_x: bool,
        u0: f32,
        u1: f32,
        v0: f32,
        v1: f32,
        y0: f32,
        y1: f32,
        mat: Mat,
    ) {
        if along_x {
            self.add(Vec3::new(u0, y0, v0), Vec3::new(u1, y1, v1), mat);
        } else {
            self.add(Vec3::new(v0, y0, u0), Vec3::new(v1, y1, u1), mat);
        }
    }

    /// Dashed center lines on the streets at the low x and z edge of every cell of the chunk (the high edges
    /// belong to the neighbour chunk).
    fn road_marks(&mut self, origin: Vec2) {
        for k in 0..CHUNK {
            for along_x in [true, false] {
                let (u0, v) = if along_x {
                    (origin.x, origin.y + k as f32 * PITCH)
                } else {
                    (origin.y, origin.x + k as f32 * PITCH)
                };
                let mut u = u0 + 1.0;
                while u < u0 + CHUNK_SIZE - 2.0 {
                    self.add_uv(
                        along_x,
                        u,
                        u + 2.0,
                        v - 0.08,
                        v + 0.08,
                        0.0,
                        0.02,
                        Mat::Paving,
                    );
                    u += 5.0;
                }
            }
        }
    }

    /// Terrace heights from a few random hills, then what stands on each block.
    fn plan(&mut self, chunk: IVec2, theme: &Theme) -> Vec<Block> {
        let c = CHUNK as f32;
        let hills: Vec<(f32, f32, f32, f32)> = (0..theme.hills)
            .map(|_| {
                (
                    self.rng.range(0.0, c),
                    self.rng.range(0.0, c),
                    self.rng.range(0.0, theme.peak + 0.99).floor(),
                    self.rng.range(1.0, 3.0),
                )
            })
            .collect();
        let mut blocks = Vec::new();
        for i in 0..CHUNK {
            for j in 0..CHUNK {
                let x0 = (chunk.x * CHUNK + i) as f32 * PITCH + STREET / 2.0;
                let z0 = (chunk.y * CHUNK + j) as f32 * PITCH + STREET / 2.0;
                let (ci, cj) = (i as f32 + 0.5, j as f32 + 0.5);
                let level = hills
                    .iter()
                    .map(|&(hx, hz, peak, r)| {
                        peak * (1.0 - ((ci - hx).powi(2) + (cj - hz).powi(2)).sqrt() / r)
                    })
                    .fold(0.0, f32::max)
                    .round();
                let roll = self.rng.unit();
                let pit = !blocks
                    .iter()
                    .any(|b: &Block| matches!(b.kind, Kind::Pit(_)))
                    && self.rng.chance(theme.pit);
                let kind = if pit {
                    Kind::Pit(self.pit_plan(x0, z0))
                } else if roll < theme.building {
                    // A core with room for wings, or one filling most of the block.
                    let (w, d) = if self.rng.chance(0.65) {
                        (self.rng.range(12.0, 14.0), self.rng.range(10.5, 13.0))
                    } else {
                        (
                            self.rng.range(12.0, BLOCK - 3.0),
                            self.rng.range(12.0, BLOCK - 3.0),
                        )
                    };
                    let bx = x0 + self.rng.range(1.5, BLOCK - w - 1.5);
                    let bz = z0 + self.rng.range(1.5, BLOCK - d - 1.5);
                    let (bx1, bz1) = (bx + w, bz + d);
                    Kind::Building(Building {
                        x0: bx,
                        z0: bz,
                        x1: bx1,
                        z1: bz1,
                        floors: if self.rng.chance(theme.tower) {
                            TOWER.0 + self.rng.below(TOWER.1 - TOWER.0 + 1)
                        } else {
                            theme.floors.0 + self.rng.below(theme.floors.1 - theme.floors.0 + 1)
                        },
                        wall: Mat::Wall(self.rng.below(WALLS as u32) as u8),
                        floor: Mat::Floor(self.rng.below(FLOORS as u32) as u8),
                        xm: self.rng.range(stair_x1(bx) + 2.0, bx1 - 3.5),
                        zm: self.rng.range(stair_z1(bz) + 3.0, bz1 - 3.5),
                        gaps: Vec::new(),
                        wings: Vec::new(),
                    })
                } else if roll < theme.building + theme.plaza {
                    Kind::Plaza
                } else {
                    Kind::Park
                };
                // A pit is dug from the street level.
                let e = if matches!(kind, Kind::Pit(_)) {
                    0.0
                } else {
                    level * TERRACE
                };
                blocks.push(Block { x0, z0, e, kind });
            }
        }
        blocks
    }

    /// Joins neighbouring buildings of the chunk: half of `theme.connect` a corridor between upper floors,
    /// the other half a bridge between roofs.
    /// Records the wall gaps on both buildings.
    fn connect(&mut self, blocks: &mut [Block], theme: &Theme) -> Vec<Passage> {
        let mut passages = Vec::new();
        for i in 0..CHUNK {
            for j in 0..CHUNK {
                for along_x in [true, false] {
                    let (ia, ib) = match (
                        block_index(i, j),
                        if along_x {
                            block_index(i + 1, j)
                        } else {
                            block_index(i, j + 1)
                        },
                    ) {
                        (Some(a), Some(b)) => (a, b),
                        _ => continue,
                    };
                    let (Kind::Building(a), Kind::Building(b)) =
                        (&blocks[ia].kind, &blocks[ib].kind)
                    else {
                        continue;
                    };
                    let roll = self.rng.unit();
                    if roll >= theme.connect {
                        continue;
                    }
                    let (ea, eb) = (blocks[ia].e, blocks[ib].e);
                    // Across range on both faces, clear of B's stairwell (south-west corner) and both
                    // interior walls, which meet the outer walls at xm / zm.
                    let (lo, hi, ma, mb) = if along_x {
                        (
                            a.z0.max(stair_z1(b.z0)) + 1.5,
                            a.z1.min(b.z1) - 1.5,
                            a.zm,
                            b.zm,
                        )
                    } else {
                        (
                            a.x0.max(stair_x1(b.x0) + 1.5) + 0.5,
                            a.x1.min(b.x1) - 1.5,
                            a.xm,
                            b.xm,
                        )
                    };
                    let Some(v) = (0..8)
                        .map(|_| self.rng.range(lo, hi))
                        .find(|&v| hi > lo && (v - ma).abs() > 1.6 && (v - mb).abs() > 1.6)
                    else {
                        continue;
                    };
                    // Corridor: the pair of upper floors (not ground, not roof) closest in height.
                    let corridor = (roll < theme.connect / 2.0 && a.floors >= 2 && b.floors >= 2)
                        .then(|| {
                            (1..a.floors)
                                .flat_map(|ka| (1..b.floors).map(move |kb| (ka, kb)))
                                .min_by(|x, y| {
                                    let d = |(ka, kb): (u32, u32)| {
                                        (ea + ka as f32 * FH - eb - kb as f32 * FH).abs()
                                    };
                                    d(*x).total_cmp(&d(*y))
                                })
                        })
                        .flatten();
                    let (ka, kb, covered) = match corridor {
                        Some((ka, kb)) => (ka, kb, true),
                        None => (a.floors, b.floors, false),
                    };
                    let (ya, yb) = (ea + ka as f32 * FH, eb + kb as f32 * FH);
                    if (ya - yb).abs() > MAX_PASSAGE_RISE {
                        continue;
                    }
                    let u = if along_x {
                        [a.x1, blocks[ia].x0 + BLOCK, blocks[ib].x0, b.x0]
                    } else {
                        [a.z1, blocks[ia].z0 + BLOCK, blocks[ib].z0, b.z0]
                    };
                    let opening = if covered {
                        Opening {
                            c: v,
                            w: 1.6,
                            bottom: 0.0,
                            top: 2.4,
                        }
                    } else {
                        Opening {
                            c: v,
                            w: 2.0,
                            bottom: 0.0,
                            top: 1.0,
                        }
                    };
                    let (side_a, side_b) = if along_x { (3, 2) } else { (1, 0) };
                    if let Kind::Building(a) = &mut blocks[ia].kind {
                        a.gaps.push(Gap {
                            floor: ka,
                            side: side_a,
                            opening,
                        });
                    }
                    if let Kind::Building(b) = &mut blocks[ib].kind {
                        b.gaps.push(Gap {
                            floor: kb,
                            side: side_b,
                            opening,
                        });
                    }
                    passages.push(Passage {
                        along_x,
                        u,
                        v,
                        ya,
                        yb,
                        covered,
                    });
                }
            }
        }
        passages
    }

    /// The street-level ground of the chunk (1 m of asphalt), with holes over the pits: strips between the
    /// x edges of the holes, each split along z around the holes it crosses.
    fn ground(&mut self, origin: Vec2, blocks: &[Block]) {
        let holes: Vec<(f32, f32, f32, f32)> = blocks
            .iter()
            .filter_map(|b| match &b.kind {
                Kind::Pit(p) => Some((p.x0, p.z0, p.x1, p.z1)),
                _ => None,
            })
            .collect();
        let (ox, oz, s) = (origin.x, origin.y, CHUNK_SIZE);
        let mut xs = vec![ox, ox + s];
        for h in &holes {
            xs.extend([h.0, h.2]);
        }
        xs.sort_by(f32::total_cmp);
        xs.dedup();
        for pair in xs.windows(2) {
            let (xa, xb) = (pair[0], pair[1]);
            let mut cuts: Vec<(f32, f32)> = holes
                .iter()
                .filter(|h| h.0 <= xa && h.2 >= xb)
                .map(|h| (h.1, h.3))
                .collect();
            cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut z = oz;
            for (c0, c1) in cuts.into_iter().chain([(oz + s, oz + s)]) {
                self.add(Vec3::new(xa, -1.0, z), Vec3::new(xb, 0.0, c0), Mat::Asphalt);
                z = c1;
            }
        }
    }

    /// Wings for the buildings: up to two, on sides without passage gaps where the block leaves room (at
    /// least `WING.0` deep plus a 1 m margin), each over part of the side, with a door into the core on each
    /// of its floors (clear of the stairwell and the interior walls).
    fn wings(&mut self, blocks: &mut [Block]) {
        for block in blocks.iter_mut() {
            let (bx0, bz0) = (block.x0, block.z0);
            let Kind::Building(bl) = &mut block.kind else {
                continue;
            };
            let mut sides: Vec<u32> = (0..4)
                .filter(|&s| !bl.gaps.iter().any(|g| g.side == s))
                .collect();
            // Shuffle.
            for i in (1..sides.len()).rev() {
                let j = self.rng.below(i as u32 + 1) as usize;
                sides.swap(i, j);
            }
            let want = self.rng.below(3) as usize;
            let mut made = 0;
            for side in sides {
                if made >= want {
                    break;
                }
                let room = match side {
                    0 => bl.z0 - bz0,
                    1 => bz0 + BLOCK - bl.z1,
                    2 => bl.x0 - bx0,
                    _ => bx0 + BLOCK - bl.x1,
                } - 1.0;
                if room < WING.0 {
                    continue;
                }
                let depth = self.rng.range(WING.0, room.min(WING.1));
                let along_x = side < 2;
                let (c0, c1) = if along_x {
                    (bl.x0, bl.x1)
                } else {
                    (bl.z0, bl.z1)
                };
                let span = self.rng.range(5.0, c1 - c0);
                let u0 = self.rng.range(c0, c1 - span);
                let u1 = u0 + span;
                // Door into the core: inside the wing, clear of the stairwell (south-west corner) and of the
                // interior wall that meets this side.
                let (stair_end, m) = if along_x {
                    (stair_x1(bl.x0) + 1.0, bl.xm)
                } else {
                    (stair_z1(bl.z0) + 1.0, bl.zm)
                };
                let lo = (u0 + T + 0.9).max(if side == 0 || side == 2 {
                    stair_end
                } else {
                    f32::MIN
                });
                let hi = u1 - T - 0.9;
                let Some(c) = (0..8)
                    .map(|_| self.rng.range(lo, hi))
                    .find(|&c| hi > lo && (c - m).abs() > 1.6)
                else {
                    continue;
                };
                let floors = 1 + self.rng.below(bl.floors.min(6));
                let (x0, z0, x1, z1) = match side {
                    0 => (u0, bl.z0 - depth, u1, bl.z0),
                    1 => (u0, bl.z1, u1, bl.z1 + depth),
                    2 => (bl.x0 - depth, u0, bl.x0, u1),
                    _ => (bl.x1, u0, bl.x1 + depth, u1),
                };
                for k in 0..floors {
                    bl.gaps.push(Gap {
                        floor: k,
                        side,
                        opening: door_at(c),
                    });
                }
                if bl.floors > floors {
                    bl.gaps.push(Gap {
                        floor: floors,
                        side,
                        opening: door_at(c),
                    });
                }
                bl.wings.push(Wing {
                    side,
                    x0,
                    z0,
                    x1,
                    z1,
                    floors,
                });
                made += 1;
            }
        }
    }

    /// A wing: outer walls with windows on the three free edges (the x walls run full length, the z walls
    /// between them or up to the core), a street door in the far wall, a slab per floor (the last one is its
    /// roof, with a parapet).
    fn wing(&mut self, w: &Wing, e: f32, wall: Mat, floor: Mat) {
        // The edge against the core: 0 south, 1 north, 2 west, 3 east of the wing.
        let attached = w.side ^ 1;
        let far = w.side;
        let (x0, z0, x1, z1) = (w.x0, w.z0, w.x1, w.z1);
        let inset = |edge: u32| if edge == attached { 0.0 } else { T };
        let (ix0, ix1) = (x0 + inset(2), x1 - inset(3));
        let (iz0, iz1) = (z0 + inset(0), z1 - inset(1));
        self.rooms.push(Aabb {
            min: Vec3::new(ix0, e, iz0),
            max: Vec3::new(ix1, e + w.floors as f32 * FH, iz1),
        });
        self.add(Vec3::new(ix0, e, iz0), Vec3::new(ix1, e + 0.05, iz1), floor);
        let door_side = far;
        for k in 0..=w.floors {
            let y = e + k as f32 * FH;
            let roof = k == w.floors;
            let h = if roof { 1.0 } else { FH };
            for edge in (0..4).filter(|&s| s != attached) {
                let along_x = edge < 2;
                let (u0, u1) = if along_x { (x0, x1) } else { (iz0, iz1) };
                let v = [z0 + T / 2.0, z1 - T / 2.0, x0 + T / 2.0, x1 - T / 2.0][edge as usize];
                let mut doors = Vec::new();
                if k == 0 && edge == door_side && u1 - u0 > 4.0 {
                    doors.push(door_at(self.rng.range(u0 + 1.5, u1 - 1.5)));
                }
                let ops = if roof {
                    Vec::new()
                } else {
                    self.openings(u0, u1, doors)
                };
                self.wall(along_x, u0, u1, v, y, h, T, &ops, wall);
            }
            if !roof {
                let top = y + FH;
                self.add(
                    Vec3::new(ix0, top - 0.2, iz0),
                    Vec3::new(ix1, top, iz1),
                    floor,
                );
            }
        }
    }

    /// A random pit for the block at (`bx`, `bz`): a kind (shaft, crater or pit) for its size and depth, a
    /// place in the block, a stair layout; the depth is cut to what the stairs along the walls can reach.
    fn pit_plan(&mut self, bx: f32, bz: f32) -> PitPlan {
        let roll = self.rng.unit();
        let mut acc = 0.0;
        let &(_, size, depth) = PIT_KINDS
            .iter()
            .find(|k| {
                acc += k.0;
                roll < acc
            })
            .unwrap_or(&PIT_KINDS[2]);
        let max = BLOCK - 2.0 * PIT_MARGIN;
        let (sx, sz) = (
            self.rng.range(size.0, size.1).min(max),
            self.rng.range(size.0, size.1).min(max),
        );
        let x0 = bx + self.rng.range(PIT_MARGIN, BLOCK - PIT_MARGIN - sx);
        let z0 = bz + self.rng.range(PIT_MARGIN, BLOCK - PIT_MARGIN - sz);
        let swap = self.rng.chance(0.5);
        // Steps that fit along the walls: the first side from its corner, the others between landings.
        let (lx, lz) = if swap { (sz, sx) } else { (sx, sz) };
        let w = PIT_STAIR;
        let fit = ((lx - w) / RUN).floor()
            + ((lz - 2.0 * w) / RUN).floor()
            + ((lx - 2.0 * w) / RUN).floor()
            + ((lz - 2.0 * w) / RUN).floor()
            - 2.0;
        let d = (self.rng.range(depth.0, depth.1) / RISE).round().min(fit) * RISE;
        PitPlan {
            x0,
            z0,
            x1: x0 + sx,
            z1: z0 + sz,
            depth: d,
            swap,
            flip_x: self.rng.chance(0.5),
            flip_z: self.rng.chance(0.5),
        }
    }

    /// A pit dug into the block from the street, built in its local frame (see `PitPlan`): retaining walls
    /// under the asphalt, a floor `depth` down, a flight of steps along the walls (from the local south-west
    /// corner along the south wall, a landing in each corner, on along the next wall) with a railing on the
    /// open side, a railing round the rim (open at the stairs' top), and on the floor a feature in each
    /// quarter (pillars, a ruined corner, a platform, a two-tier mound, a pool, a boulder or nothing) and one
    /// to four artifacts over the middle of some quarters (on top of a platform or mound).
    fn pit(&mut self, p: &PitPlan) {
        let (lx, lz) = if p.swap {
            (p.z1 - p.z0, p.x1 - p.x0)
        } else {
            (p.x1 - p.x0, p.z1 - p.z0)
        };
        let to_world = |a: f32, b: f32, lo: f32, hi: f32, flip: bool| {
            if flip {
                (hi - b, hi - a)
            } else {
                (lo + a, lo + b)
            }
        };
        // A local box to the world.
        let put = |b: &mut Builder, min: Vec3, max: Vec3, mat: Mat| {
            let (ax0, ax1, az0, az1) = if p.swap {
                (min.z, max.z, min.x, max.x)
            } else {
                (min.x, max.x, min.z, max.z)
            };
            let (x0, x1) = to_world(ax0, ax1, p.x0, p.x1, p.flip_x);
            let (z0, z1) = to_world(az0, az1, p.z0, p.z1, p.flip_z);
            b.add(Vec3::new(x0, min.y, z0), Vec3::new(x1, max.y, z1), mat);
        };
        let (w, bottom) = (PIT_STAIR, -p.depth);
        let v = Vec3::new;
        // Walls under the 1 m asphalt (the south and north ones over the corners) and the floor.
        let (wy0, wy1) = (bottom - 0.5, -1.0);
        put(self, v(-1.0, wy0, -1.0), v(lx + 1.0, wy1, 0.0), Mat::Stone);
        put(
            self,
            v(-1.0, wy0, lz),
            v(lx + 1.0, wy1, lz + 1.0),
            Mat::Stone,
        );
        put(self, v(-1.0, wy0, 0.0), v(0.0, wy1, lz), Mat::Stone);
        put(self, v(lx, wy0, 0.0), v(lx + 1.0, wy1, lz), Mat::Stone);
        put(
            self,
            v(0.0, bottom - 0.5, 0.0),
            v(lx, bottom, lz),
            Mat::Grass,
        );
        // Rim railing, open where the stairs start.
        let (r, rh) = (0.08, 1.0);
        put(self, v(w, 0.0, -r), v(lx + r, rh, 0.0), Mat::Metal);
        put(self, v(-r, 0.0, lz), v(lx + r, rh, lz + r), Mat::Metal);
        put(self, v(-r, 0.0, w), v(0.0, rh, lz), Mat::Metal);
        put(self, v(lx, 0.0, 0.0), v(lx + r, rh, lz), Mat::Metal);

        // A piece of the stair strip along side `s` (0 south, 1 east, 2 north, 3 west, counterclockwise
        // from the south-west corner), from `ta` to `tb` m from the side's start corner, `a` to `c` m from
        // the wall.
        let piece = |b: &mut Builder,
                     s: u32,
                     (ta, tb): (f32, f32),
                     (a, c): (f32, f32),
                     (y0, y1): (f32, f32),
                     mat: Mat| {
            let (min, max) = match s {
                0 => (v(ta, y0, a), v(tb, y1, c)),
                1 => (v(lx - c, y0, ta), v(lx - a, y1, tb)),
                2 => (v(lx - tb, y0, lz - c), v(lx - ta, y1, lz - a)),
                _ => (v(a, y0, lz - tb), v(c, y1, lz - ta)),
            };
            put(b, min, max, mat);
        };
        let steps = (p.depth / RISE).round() as i32;
        let (mut i, mut s, mut t) = (0, 0u32, 0.0f32);
        while i < steps && s < 4 {
            let len = if s % 2 == 0 { lx } else { lz };
            let end = len - w;
            while i < steps && t + RUN <= end + 1e-3 {
                let top = -((i + 1) as f32) * RISE;
                piece(self, s, (t, t + RUN), (0.0, w), (bottom, top), Mat::Stair);
                // No railing on the last steps (under 0.75 m): the way off the stairs onto the floor.
                if steps - i > 3 {
                    piece(
                        self,
                        s,
                        (t, t + RUN),
                        (w - r, w),
                        (top, top + rh),
                        Mat::Metal,
                    );
                }
                t += RUN;
                i += 1;
            }
            if i >= steps {
                break;
            }
            // Landing in the next corner, railed on the part beside the pit.
            let top = -(i as f32) * RISE;
            piece(self, s, (t, len), (0.0, w), (bottom, top), Mat::Stair);
            piece(self, s, (t, end), (w - r, w), (top, top + rh), Mat::Metal);
            s += 1;
            t = w;
        }

        // The floor inside the stair strips in four quarters, each with a feature that keeps its middle free
        // (a platform or mound fills it, the artifact floats on top).
        let (fx0, fx1, fz0, fz1) = (w + 0.6, lx - w - 0.6, w + 0.6, lz - w - 0.6);
        let (mx, mz) = ((fx0 + fx1) / 2.0, (fz0 + fz1) / 2.0);
        let quarters = [
            (fx0, mx, fz0, mz),
            (mx, fx1, fz0, mz),
            (fx0, mx, mz, fz1),
            (mx, fx1, mz, fz1),
        ];
        let mut tops = [bottom; 4];
        for (q, &(qx0, qx1, qz0, qz1)) in quarters.iter().enumerate() {
            // 0.6 m clear on each side: walkways of 1.2 m between quarters and along the stairs.
            let (qx0, qx1, qz0, qz1) = (qx0 + 0.6, qx1 - 0.6, qz0 + 0.6, qz1 - 0.6);
            let (cx, cz) = ((qx0 + qx1) / 2.0, (qz0 + qz1) / 2.0);
            let room = (qx1 - qx0).min(qz1 - qz0);
            let feature = [
                Feature::Empty,
                Feature::Pillars,
                Feature::Ruin,
                Feature::Platform,
                Feature::Mound,
                Feature::Pool,
                Feature::Boulder,
            ][self.rng.below(7) as usize];
            match feature {
                Feature::Empty => {}
                Feature::Pillars => {
                    for (px, pz) in [(qx0, qz0), (qx1, qz0), (qx0, qz1), (qx1, qz1)] {
                        if !self.rng.chance(0.6) {
                            continue;
                        }
                        let half = self.rng.range(0.3, 0.45);
                        let h = self.rng.range(2.0, 7.0);
                        let (x, z) = (
                            px + (cx - px).signum() * half,
                            pz + (cz - pz).signum() * half,
                        );
                        put(
                            self,
                            v(x - half, bottom, z - half),
                            v(x + half, bottom + h, z + half),
                            Mat::Stone,
                        );
                    }
                }
                Feature::Ruin => {
                    let (ha, hb) = (self.rng.range(1.2, 1.8), self.rng.range(2.0, 3.0));
                    put(
                        self,
                        v(qx0, bottom, qz0),
                        v(qx1, bottom + ha, qz0 + 0.4),
                        Mat::Stone,
                    );
                    put(
                        self,
                        v(qx0, bottom, qz0 + 0.4),
                        v(qx0 + 0.4, bottom + hb, qz1),
                        Mat::Stone,
                    );
                }
                Feature::Platform | Feature::Mound => {
                    let half = (room / 2.0 - 0.2).min(1.6);
                    put(
                        self,
                        v(cx - half, bottom, cz - half),
                        v(cx + half, bottom + 0.5, cz + half),
                        Mat::Stair,
                    );
                    tops[q] = bottom + 0.5;
                    if matches!(feature, Feature::Mound) && half > 1.0 {
                        let h2 = half * 0.55;
                        put(
                            self,
                            v(cx - h2, bottom + 0.5, cz - h2),
                            v(cx + h2, bottom + 1.0, cz + h2),
                            Mat::Stone,
                        );
                        tops[q] = bottom + 1.0;
                    }
                }
                Feature::Pool => {
                    let half = (room / 2.0 - 0.2).min(1.8);
                    put(
                        self,
                        v(cx - half, bottom, cz - half),
                        v(cx + half, bottom + 0.06, cz + half),
                        Mat::Water,
                    );
                    tops[q] = bottom + 0.06;
                }
                Feature::Boulder => {
                    let size = self.rng.range(0.8, 1.4).min(room / 2.0);
                    let h = self.rng.range(0.6, 1.5);
                    let (x, z) = if self.rng.chance(0.5) {
                        (qx0, qz1 - size)
                    } else {
                        (qx1 - size, qz0)
                    };
                    put(
                        self,
                        v(x, bottom, z),
                        v(x + size, bottom + h, z + size),
                        Mat::Stone,
                    );
                }
            }
        }
        // One to four artifacts, each over the middle of its own quarter.
        let mut order = [0usize, 1, 2, 3];
        for k in (1..4).rev() {
            let j = self.rng.below(k as u32 + 1) as usize;
            order.swap(k, j);
        }
        let n = 1 + self.rng.below(4) as usize;
        for &q in &order[..n] {
            let (qx0, qx1, qz0, qz1) = quarters[q];
            let c = v((qx0 + qx1) / 2.0, tops[q] + 1.1, (qz0 + qz1) / 2.0);
            self.artifacts.push(p.world(c));
        }
    }

    /// Raised block: a stone terrace with a flight of steps from the street on its south and north sides.
    fn terrace(&mut self, block: &Block) {
        let (x0, z0, e) = (block.x0, block.z0, block.e);
        if e <= 0.0 {
            return;
        }
        self.add(
            Vec3::new(x0, 0.0, z0),
            Vec3::new(x0 + BLOCK, e, z0 + BLOCK),
            Mat::Stone,
        );
        let n = (e / RISE).round() as i32;
        let len = n as f32 * RUN;
        for (za, zb) in [(z0 - 1.6, z0), (z0 + BLOCK, z0 + BLOCK + 1.6)] {
            let start = self.rng.range(x0 + 0.5, x0 + BLOCK - len - 0.5);
            let up_x = self.rng.chance(0.5);
            for i in 0..n {
                let x = if up_x {
                    start + i as f32 * RUN
                } else {
                    start + len - (i + 1) as f32 * RUN
                };
                self.add(
                    Vec3::new(x, 0.0, za),
                    Vec3::new(x + RUN, (i + 1) as f32 * RISE, zb),
                    Mat::Stair,
                );
            }
        }
    }

    /// A wall of thickness `t` centered on `v`, running from `u0` to `u1` along x (or z), with sorted openings.
    #[allow(clippy::too_many_arguments)]
    fn wall(
        &mut self,
        along_x: bool,
        u0: f32,
        u1: f32,
        v: f32,
        y0: f32,
        h: f32,
        t: f32,
        openings: &[Opening],
        mat: Mat,
    ) {
        let (v0, v1) = (v - t / 2.0, v + t / 2.0);
        let mut cursor = u0;
        for o in openings {
            let a = (o.c - o.w / 2.0).max(cursor);
            let c = (o.c + o.w / 2.0).min(u1);
            if c <= a {
                continue;
            }
            self.add_uv(along_x, cursor, a, v0, v1, y0, y0 + h, mat);
            self.add_uv(along_x, a, c, v0, v1, y0, y0 + o.bottom, mat);
            self.add_uv(along_x, a, c, v0, v1, y0 + o.top, y0 + h, mat);
            cursor = c;
        }
        self.add_uv(along_x, cursor, u1, v0, v1, y0, y0 + h, mat);
    }

    /// The given doors plus windows every 3 m (70 % chance each) clear of them; sorted by position.
    fn openings(&mut self, u0: f32, u1: f32, doors: Vec<Opening>) -> Vec<Opening> {
        let mut v = doors.clone();
        let mut u = u0 + 2.0;
        while u < u1 - 1.5 {
            let clear = doors.iter().all(|d| (u - d.c).abs() > d.w / 2.0 + 1.0);
            if clear && self.rng.chance(0.7) {
                v.push(Opening {
                    c: u,
                    w: 1.2,
                    bottom: 1.0,
                    top: 2.3,
                });
            }
            u += 3.0;
        }
        v.sort_by(|a, b| a.c.total_cmp(&b.c));
        v
    }

    /// Building on a terrace at height `e`: outer walls with windows, a street door and the passage gaps,
    /// four rooms per floor joined by doors, a switchback stairwell in the south-west corner (the top flight
    /// leads to the roof), railings at each stair hole and a parapet on the roof.
    /// Stairwell: two lanes side by side along x with a landing at each end. Flight k climbs in lane k % 2
    /// (+x in lane 0, -x in lane 1), so the next flight starts where the last one ends, and the slab above
    /// flight k has its hole over that lane only; flights never stack inside one another's head room.
    /// Floor slabs stop at the inner face of the outer walls, so no faces overlap.
    fn building(&mut self, bl: &Building, e: f32) {
        for w in &bl.wings {
            self.wing(w, e, bl.wall, bl.floor);
        }
        let (x0, z0, x1, z1) = (bl.x0, bl.z0, bl.x1, bl.z1);
        let steps = (FH / RISE) as i32;
        let (sx0, sx1) = (stair_x0(x0), stair_x1(x0));
        let sz0 = z0 + T;
        let sz1 = stair_z1(z0);
        let (ix0, iz0, ix1, iz1) = (x0 + T, z0 + T, x1 - T, z1 - T);
        let door_side = self.rng.below(4);
        // Indoors from the ground floor up to the roof slab's top (the roof's upper face is outdoors).
        self.rooms.push(Aabb {
            min: Vec3::new(ix0, e, iz0),
            max: Vec3::new(ix1, e + bl.floors as f32 * FH, iz1),
        });
        // Watcher spots outside: at each corner on the terrace, pressed to one of the two walls right at the
        // edge (peeking around it; escapes: around the corner 1.5 m along the other wall, or 1.8 m back along
        // its own), and on one random roof corner inside the parapet (not the stairwell's; escape: 3 m back
        // from the edge).
        let top = e + bl.floors as f32 * FH;
        for (cx, cz, sx, sz) in [
            (x0, z0, 1.0, 1.0),
            (x1, z0, -1.0, 1.0),
            (x0, z1, 1.0, -1.0),
            (x1, z1, -1.0, -1.0),
        ] {
            let out = Vec3::new(cx - sx * 0.35, e, cz - sz * 0.35);
            let (pos, around, back) = if self.rng.chance(0.5) {
                (
                    Vec3::new(cx - sx * 0.35, e, cz + sz * 0.25),
                    Vec3::new(cx + sx * 1.5, e, cz - sz * 0.35),
                    Vec3::new(cx - sx * 0.35, e, cz + sz * 1.8),
                )
            } else {
                (
                    Vec3::new(cx + sx * 0.25, e, cz - sz * 0.35),
                    Vec3::new(cx - sx * 0.35, e, cz + sz * 1.5),
                    Vec3::new(cx + sx * 1.8, e, cz - sz * 0.35),
                )
            };
            self.spots.push(Spot {
                pos,
                escapes: vec![vec![out, around], vec![back]],
                kind: SpotKind::Corner,
            });
        }
        let corner = 1 + self.rng.below(3) as usize;
        let (cx, cz, sx, sz) = [
            (x0, z0, 1.0, 1.0),
            (x1, z0, -1.0, 1.0),
            (x0, z1, 1.0, -1.0),
            (x1, z1, -1.0, -1.0),
        ][corner];
        let pos = Vec3::new(cx + sx * (T + 0.5), top, cz + sz * (T + 0.5));
        self.spots.push(Spot {
            pos,
            escapes: vec![vec![pos + Vec3::new(sx, 0.0, sz) * 3.0]],
            kind: SpotKind::Roof,
        });

        self.add(
            Vec3::new(ix0, e, iz0),
            Vec3::new(ix1, e + 0.05, iz1),
            bl.floor,
        );
        for k in 0..=bl.floors {
            let y = e + k as f32 * FH;
            let roof = k == bl.floors;
            let h = if roof { 1.0 } else { FH };
            for side in 0..4 {
                let along_x = side < 2;
                let (u0, u1) = if along_x { (x0, x1) } else { (iz0, iz1) };
                let v = [z0 + T / 2.0, z1 - T / 2.0, x0 + T / 2.0, x1 - T / 2.0][side as usize];
                let mut doors: Vec<Opening> = bl
                    .gaps
                    .iter()
                    .filter(|g| g.floor == k && g.side == side)
                    .map(|g| g.opening)
                    .collect();
                if k == 0 && side == door_side {
                    // Clear of the stairwell on the south side and of the interior wall.
                    let lo = if side == 0 { sx1 + 1.5 } else { u0 + 1.5 };
                    let m = if along_x { bl.xm } else { bl.zm };
                    let c = (0..8)
                        .map(|_| self.rng.range(lo, u1 - 1.5))
                        .find(|c| (c - m).abs() > 1.5);
                    doors.extend(c.map(door_at));
                }
                let ops = if roof {
                    doors.sort_by(|a, b| a.c.total_cmp(&b.c));
                    doors
                } else {
                    self.openings(u0, u1, doors)
                };
                // Watcher spots: 0.35 m inside the windows of every floor (close to the glass, in the light
                // from the window), away from the stairwell; escapes: a step to either side behind the wall,
                // or 1.8 m deeper into the room.
                if !roof {
                    for o in ops.iter().filter(|o| o.bottom > 0.0) {
                        let inside = T + 0.35;
                        let (spot, inward) = match side {
                            0 => (Vec3::new(o.c, y, z0 + inside), Vec3::Z),
                            1 => (Vec3::new(o.c, y, z1 - inside), -Vec3::Z),
                            2 => (Vec3::new(x0 + inside, y, o.c), Vec3::X),
                            _ => (Vec3::new(x1 - inside, y, o.c), -Vec3::X),
                        };
                        let along = if along_x { Vec3::X } else { Vec3::Z };
                        let aside = along * (o.w / 2.0 + 0.45);
                        let in_stairwell = spot.x < sx1 + 1.0 && spot.z < sz1 + 1.0;
                        if !in_stairwell {
                            self.spots.push(Spot {
                                pos: spot,
                                escapes: vec![
                                    vec![spot + aside],
                                    vec![spot - aside],
                                    vec![spot + inward * 1.8],
                                ],
                                kind: SpotKind::Window,
                            });
                        }
                    }
                }
                self.wall(along_x, u0, u1, v, y, h, T, &ops, bl.wall);
            }
            if roof {
                break;
            }
            // Interior walls, two doors each, kept away from the crossing; they stop under the slab above, so
            // their tops don't share the floor plane of the next story.
            let (xm, zm) = (bl.xm, bl.zm);
            let d1 = self.rng.range(ix0 + 1.2, xm - 1.5);
            let d2 = self.rng.range(xm + 1.5, ix1 - 1.2);
            self.wall(
                true,
                ix0,
                ix1,
                zm,
                y,
                FH - 0.2,
                0.2,
                &[door_at(d1), door_at(d2)],
                Mat::Interior,
            );
            let d1 = self.rng.range(sz1 + 0.8, zm - 1.5);
            let d2 = self.rng.range(zm + 1.5, iz1 - 1.2);
            self.wall(
                false,
                iz0,
                iz1,
                xm,
                y,
                FH - 0.2,
                0.2,
                &[door_at(d1), door_at(d2)],
                Mat::Interior,
            );

            let lane = k % 2;
            let lz0 = sz0 + lane as f32 * LANE;
            let lz1 = lz0 + LANE;
            for i in 0..steps {
                let a = if lane == 0 {
                    sx0 + i as f32 * RUN
                } else {
                    sx1 - (i + 1) as f32 * RUN
                };
                self.add(
                    Vec3::new(a, y, lz0),
                    Vec3::new(a + RUN, y + (i + 1) as f32 * RISE, lz1),
                    Mat::Stair,
                );
            }
            // Slab above with the hole over this flight's lane, as boxes around it; railings on the open
            // sides of the hole (lane 0 lies against the south wall), inside the hole's lane: the next flight
            // starts in the other lane at the slab's height, and a railing there would cut into its steps.
            let top = y + FH;
            let (y0, y1) = (top - 0.2, top);
            self.add(Vec3::new(ix0, y0, iz0), Vec3::new(sx0, y1, iz1), bl.floor);
            self.add(Vec3::new(sx1, y0, iz0), Vec3::new(ix1, y1, iz1), bl.floor);
            self.add(Vec3::new(sx0, y0, iz0), Vec3::new(sx1, y1, lz0), bl.floor);
            self.add(Vec3::new(sx0, y0, lz1), Vec3::new(sx1, y1, iz1), bl.floor);
            self.add(
                Vec3::new(sx0, top, lz1 - 0.08),
                Vec3::new(sx1, top + 1.0, lz1),
                Mat::Metal,
            );
            if lane == 1 {
                self.add(
                    Vec3::new(sx0, top, lz0),
                    Vec3::new(sx1, top + 1.0, lz0 + 0.08),
                    Mat::Metal,
                );
            }
        }
    }

    /// Flat over the block margins, stairs across the street; covered: side walls with a window band and a
    /// roof, else railings.
    fn passage(&mut self, p: &Passage) {
        let [ua, ua_blk, ub_blk, ub] = p.u;
        let d = p.yb - p.ya;
        let n = (d.abs() / RISE).ceil() as i32;
        let mut segs = vec![(ua, ua_blk, p.ya)];
        if n == 0 {
            segs.push((ua_blk, ub_blk, p.ya));
        } else {
            let run = (ub_blk - ua_blk) / n as f32;
            for i in 0..n {
                let u = ua_blk + i as f32 * run;
                segs.push((u, u + run, p.ya + d * (i + 1) as f32 / n as f32));
            }
        }
        segs.push((ub_blk, ub, p.yb));
        let half = if p.covered { 1.2 } else { 1.0 };
        let (v0, v1) = (p.v - half, p.v + half);
        for (u0, u1, y) in segs {
            self.add_uv(p.along_x, u0, u1, v0, v1, y - 0.3, y, Mat::Stair);
            if p.covered {
                for (w0, w1) in [(v0, v0 + 0.2), (v1 - 0.2, v1)] {
                    self.add_uv(p.along_x, u0, u1, w0, w1, y, y + 0.9, Mat::Stone);
                    self.add_uv(p.along_x, u0, u1, w0, w1, y + 2.1, y + 2.8, Mat::Stone);
                }
                self.add_uv(p.along_x, u0, u1, v0, v1, y + 2.8, y + 3.0, Mat::Metal);
                let (a, b) = (v0 + 0.2, v1 - 0.2);
                self.rooms.push(if p.along_x {
                    Aabb {
                        min: Vec3::new(u0, y - 0.3, a),
                        max: Vec3::new(u1, y + 2.8, b),
                    }
                } else {
                    Aabb {
                        min: Vec3::new(a, y - 0.3, u0),
                        max: Vec3::new(b, y + 2.8, u1),
                    }
                });
            } else {
                for (w0, w1) in [(v0, v0 + 0.08), (v1 - 0.08, v1)] {
                    self.add_uv(p.along_x, u0, u1, w0, w1, y, y + 1.0, Mat::Metal);
                }
            }
        }
    }

    /// Paved open place: a fountain, a stepped pyramid or a pillar hall, plus low walls and benches.
    fn plaza(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, e: f32) {
        let p = e + 0.12;
        self.add(
            Vec3::new(x0 + 0.02, e, z0 + 0.02),
            Vec3::new(x1 - 0.02, p, z1 - 0.02),
            Mat::Paving,
        );
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        match self.rng.below(3) {
            0 => {
                let r = 3.0;
                for v in [cz - r, cz + r] {
                    self.wall(true, cx - r, cx + r, v, p, 0.6, 0.4, &[], Mat::Stone);
                }
                for v in [cx - r, cx + r] {
                    self.wall(
                        false,
                        cz - r + 0.2,
                        cz + r - 0.2,
                        v,
                        p,
                        0.6,
                        0.4,
                        &[],
                        Mat::Stone,
                    );
                }
                self.add(
                    Vec3::new(cx - r + 0.2, p, cz - r + 0.2),
                    Vec3::new(cx + r - 0.2, p + 0.3, cz + r - 0.2),
                    Mat::Water,
                );
                self.add(
                    Vec3::new(cx - 0.4, p, cz - 0.4),
                    Vec3::new(cx + 0.4, p + 2.1, cz + 0.4),
                    Mat::Stone,
                );
            }
            1 => {
                for tier in 0..5 {
                    let s = 7.0 - tier as f32 * 1.4;
                    let y = p + tier as f32 * 0.45;
                    self.add(
                        Vec3::new(cx - s, y, cz - s),
                        Vec3::new(cx + s, y + 0.45, cz + s),
                        Mat::Stone,
                    );
                }
            }
            _ => {
                // Pillar hall: pillars on a grid under a roof with an open middle.
                let s = 8.0;
                for gx in 0..5 {
                    for gz in 0..5 {
                        let (x, z) = (cx - s + gx as f32 * 4.0, cz - s + gz as f32 * 4.0);
                        self.add(
                            Vec3::new(x - 0.35, p, z - 0.35),
                            Vec3::new(x + 0.35, p + 3.9, z + 0.35),
                            Mat::Stone,
                        );
                    }
                }
                let (y0, y1) = (p + 3.9, p + 4.3);
                let (a, b) = (s + 1.0, 3.0);
                self.add(
                    Vec3::new(cx - a, y0, cz - a),
                    Vec3::new(cx + a, y1, cz - b),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx - a, y0, cz + b),
                    Vec3::new(cx + a, y1, cz + a),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx - a, y0, cz - b),
                    Vec3::new(cx - b, y1, cz + b),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx + b, y0, cz - b),
                    Vec3::new(cx + a, y1, cz + b),
                    Mat::Stone,
                );
            }
        }
        // Low walls of different heights: two that cross never share a top face.
        for h in [0.75, 0.9, 1.05] {
            let len = self.rng.range(3.0, 6.0);
            let along_x = self.rng.chance(0.5);
            let u = if along_x {
                self.rng.range(x0, x1 - len)
            } else {
                self.rng.range(z0, z1 - len)
            };
            let v = if along_x {
                self.rng.range(z0 + 0.5, z0 + 3.0)
            } else {
                self.rng.range(x1 - 3.0, x1 - 0.5)
            };
            self.wall(along_x, u, u + len, v, p, h, 0.4, &[], Mat::Stone);
        }
    }

    /// Grass with trees and benches.
    fn park(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, e: f32) {
        let g = e + 0.08;
        self.add(
            Vec3::new(x0 + 0.02, e, z0 + 0.02),
            Vec3::new(x1 - 0.02, g, z1 - 0.02),
            Mat::Grass,
        );
        for _ in 0..(6 + self.rng.below(7)) {
            let x = self.rng.range(x0 + 1.5, x1 - 1.5);
            let z = self.rng.range(z0 + 1.5, z1 - 1.5);
            let h = self.rng.range(2.0, 3.0);
            let r = self.rng.range(1.0, 1.8);
            let trunk = Aabb {
                min: Vec3::new(-0.2, 0.0, -0.2),
                max: Vec3::new(0.2, h, 0.2),
            };
            let crown = Aabb {
                min: Vec3::new(-r, h, -r),
                max: Vec3::new(r, h + r * 1.6, r),
            };
            self.props.push(Prop {
                base: Vec3::new(x, g, z),
                parts: vec![(trunk, Mat::Trunk), (crown, Mat::Leaves)],
            });
        }
        // One bench per third of the block along x, so two never overlap.
        let w = (x1 - x0 - 4.0) / 3.0;
        for i in 0..3 {
            let a = x0 + 2.0 + i as f32 * w;
            let x = self.rng.range(a + 0.9, a + w - 0.9);
            let z = self.rng.range(z0 + 1.0, z1 - 1.0);
            let seat = Aabb {
                min: Vec3::new(-0.9, 0.0, -0.25),
                max: Vec3::new(0.9, 0.45, 0.25),
            };
            self.props.push(Prop {
                base: Vec3::new(x, g, z),
                parts: vec![(seat, Mat::Stone)],
            });
        }
    }
}

/// Stairwell of a building with its west wall at `x0`: the flights run from `stair_x0` to `stair_x1`, with a
/// 1 m landing between the west wall and `stair_x0` and the open floor east of `stair_x1` as the other landing.
pub(crate) fn stair_x0(x0: f32) -> f32 {
    x0 + T + 1.0
}

pub(crate) fn stair_x1(x0: f32) -> f32 {
    stair_x0(x0) + (FH / RISE) * RUN
}

/// North edge of the two stair lanes of a building with its south wall at `z0`.
pub(crate) fn stair_z1(z0: f32) -> f32 {
    z0 + T + 2.0 * LANE
}

/// Boxes of a single building with `floors` stories at the origin, for movement tests.
#[cfg(test)]
pub(crate) fn test_building(floors: u32) -> Vec<Aabb> {
    let mut b = Builder {
        rng: Rng(7),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
        rooms: Vec::new(),
        artifacts: Vec::new(),
    };
    b.add(
        Vec3::new(-20.0, -1.0, -20.0),
        Vec3::new(40.0, 0.0, 40.0),
        Mat::Asphalt,
    );
    let bl = Building {
        x0: 0.0,
        z0: 0.0,
        x1: 18.0,
        z1: 16.0,
        floors,
        wall: Mat::Wall(0),
        floor: Mat::Floor(0),
        xm: stair_x1(0.0) + 2.5,
        zm: stair_z1(0.0) + 3.5,
        gaps: Vec::new(),
        wings: Vec::new(),
    };
    b.building(&bl, 0.0);
    b.parts.into_iter().map(|(a, _)| a).collect()
}

/// Boxes of a chunk ground with one pit of `depth` in the block at the origin, and the hole (x0, z0, x1, z1).
#[cfg(test)]
pub(crate) fn test_pit(depth: f32) -> (Vec<Aabb>, (f32, f32, f32, f32)) {
    let mut b = Builder {
        rng: Rng(5),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
        rooms: Vec::new(),
        artifacts: Vec::new(),
    };
    let w = BLOCK - PIT_MARGIN;
    let plan = PitPlan {
        x0: PIT_MARGIN,
        z0: PIT_MARGIN,
        x1: w,
        z1: w,
        depth,
        swap: false,
        flip_x: false,
        flip_z: false,
    };
    let block = Block {
        x0: 0.0,
        z0: 0.0,
        e: 0.0,
        kind: Kind::Pit(plan),
    };
    b.ground(Vec2::splat(-STREET / 2.0), std::slice::from_ref(&block));
    b.pit(&plan);
    (
        b.parts.into_iter().map(|(a, _)| a).collect(),
        (PIT_MARGIN, PIT_MARGIN, w, w),
    )
}

/// A random pit (seed) in the block at the origin: its boxes, a point on the street in front of its stairs'
/// top, and its artifacts.
#[cfg(test)]
pub(crate) fn test_random_pit(seed: u64) -> (Vec<Aabb>, Vec3, Vec<Vec3>) {
    let mut b = Builder {
        rng: Rng(seed),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
        rooms: Vec::new(),
        artifacts: Vec::new(),
    };
    let plan = b.pit_plan(0.0, 0.0);
    let block = Block {
        x0: 0.0,
        z0: 0.0,
        e: 0.0,
        kind: Kind::Pit(plan),
    };
    b.ground(Vec2::splat(-STREET / 2.0), std::slice::from_ref(&block));
    b.pit(&plan);
    let start = plan.world(Vec3::new(PIT_STAIR / 2.0, 0.0, -0.6));
    (
        b.parts.into_iter().map(|(a, _)| a).collect(),
        start,
        b.artifacts,
    )
}

/// Boxes of one terrace block at height `e` (no content) and its south flight of steps, lowest step first.
#[cfg(test)]
pub(crate) fn test_terrace(e: f32) -> (Vec<Aabb>, Vec<Aabb>) {
    let mut b = Builder {
        rng: Rng(3),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
        rooms: Vec::new(),
        artifacts: Vec::new(),
    };
    b.add(
        Vec3::new(-20.0, -1.0, -20.0),
        Vec3::new(40.0, 0.0, 40.0),
        Mat::Asphalt,
    );
    b.terrace(&Block {
        x0: 0.0,
        z0: 0.0,
        e,
        kind: Kind::Park,
    });
    let mut south: Vec<Aabb> = b
        .parts
        .iter()
        .filter(|(a, m)| matches!(m, Mat::Stair) && a.max.z <= 0.0)
        .map(|(a, _)| *a)
        .collect();
    south.sort_by(|a, b| a.max.y.total_cmp(&b.max.y));
    (b.parts.into_iter().map(|(a, _)| a).collect(), south)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shot hits the face it enters through, at the right distance, with that face's normal.
    #[test]
    fn ray_hits_box_face() {
        let bx = Aabb {
            min: Vec3::new(2.0, 0.0, -1.0),
            max: Vec3::new(3.0, 2.0, 1.0),
        };
        let (t, n) = ray_box(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 10.0, &bx).unwrap();
        assert!((t - 2.0).abs() < 1e-4 && n == Vec3::NEG_X);
        let down = Vec3::new(0.0, -1.0, 0.0);
        let (t, n) = ray_box(Vec3::new(2.5, 5.0, 0.0), down, 10.0, &bx).unwrap();
        assert!((t - 3.0).abs() < 1e-4 && n == Vec3::Y);
        assert!(ray_box(Vec3::new(0.0, 1.0, 0.0), Vec3::NEG_X, 10.0, &bx).is_none());
        assert!(ray_box(Vec3::new(0.0, 1.0, 0.0), Vec3::X, 1.5, &bx).is_none());
    }

    /// Every watcher escape runs through free space, and most spots survive the check.
    #[test]
    fn watcher_escapes_are_clear() {
        let mut kept = 0;
        for s in 0..20u64 {
            let (parts, _, spots, _, _) = generate(s * 104_729 + 3, IVec2::new(s as i32, 1));
            assert!(spots.iter().all(|spot| {
                !spot.escapes.is_empty()
                    && spot
                        .escapes
                        .iter()
                        .all(|e| escape_clear(&parts, spot.pos, e))
            }));
            kept += spots.len();
        }
        assert!(kept > 20 * 10, "only {kept} spots in 20 chunks");
    }

    /// Over many chunks there are pits with artifacts and buildings with wings.
    #[test]
    fn pits_and_wings_appear() {
        let (mut artifacts, mut wings) = (0, 0);
        for s in 0..40u64 {
            let mut b = Builder {
                rng: Rng(s * 31 + 7),
                parts: Vec::new(),
                props: Vec::new(),
                spots: Vec::new(),
                rooms: Vec::new(),
                artifacts: Vec::new(),
            };
            let theme = Theme {
                building: 0.6,
                plaza: 0.2,
                floors: (1, 4),
                peak: 2.0,
                hills: 1,
                connect: 0.5,
                tower: 0.1,
                pit: 0.3,
            };
            let mut blocks = b.plan(IVec2::new(s as i32, 0), &theme);
            b.connect(&mut blocks, &theme);
            b.wings(&mut blocks);
            wings += blocks
                .iter()
                .map(|bl| match &bl.kind {
                    Kind::Building(x) => x.wings.len(),
                    _ => 0,
                })
                .sum::<usize>();
            artifacts += generate(s * 13 + 1, IVec2::new(0, s as i32)).4.len();
        }
        assert!(artifacts > 5, "only {artifacts} artifacts in 40 chunks");
        assert!(wings > 10, "only {wings} wings in 40 chunks");
    }

    #[test]
    fn no_coplanar_visible_faces() {
        for s in 0..60u64 {
            let (parts, _, _, _, _) = generate(s * 7919 + 1, IVec2::new(0, s as i32));
            let boxes: Vec<Aabb> = parts.iter().map(|(a, _)| *a).collect();
            let covered = |c: Vec3| {
                boxes
                    .iter()
                    .any(|o| c.cmpgt(o.min).all() && c.cmplt(o.max).all())
            };
            for (i, a) in boxes.iter().enumerate() {
                for b in &boxes[i + 1..] {
                    for ax in 0..3 {
                        let (u, v) = ((ax + 1) % 3, (ax + 2) % 3);
                        let lo = a.min.max(b.min);
                        let hi = a.max.min(b.max);
                        if hi[u] - lo[u] <= 0.01 || hi[v] - lo[v] <= 0.01 {
                            continue;
                        }
                        for (fa, fb, d) in
                            [(a.min[ax], b.min[ax], -0.01), (a.max[ax], b.max[ax], 0.01)]
                        {
                            let mut c = (lo + hi) / 2.0;
                            c[ax] = fa + d;
                            assert!(
                                (fa - fb).abs() >= 1e-3 || covered(c),
                                "seed {s}: faces fight at {c:?}: {:?}-{:?} vs {:?}-{:?}",
                                a.min,
                                a.max,
                                b.min,
                                b.max
                            );
                        }
                    }
                }
            }
        }
    }
}

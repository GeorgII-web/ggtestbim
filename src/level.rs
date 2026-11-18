//! Procedural city on several levels: a street grid of blocks raised on terraces (stairs up from the street),
//! each block a building (rooms, doors, windows, stairs up to a walkable roof), an open plaza or a park.
//! Neighbouring buildings are joined over the street by covered corridors between upper floors or open
//! bridges between roofs, with stairs where the heights differ.
//! Everything is an axis-aligned box: the list is the collision world of the player, and the boxes of each
//! material are merged into one mesh with world-space UVs, so textures tile at a fixed size in meters.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
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
    pub(crate) new_spots: Vec<Vec<Vec3>>,
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

    /// Whether the ground under `p` exists yet (the player waits for it, cars outside it are removed).
    pub fn loaded(&self, p: Vec3) -> bool {
        self.chunks.contains_key(&chunk_of(p))
    }
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
        let (parts, props, spots) = generate(seed, c);
        let mut groups: Vec<Vec<Aabb>> = vec![Vec::new(); MATS];
        for (b, mat) in &parts {
            groups[mat.index()].push(*b);
        }
        let mut entities: Vec<Entity> = groups
            .iter()
            .enumerate()
            .filter(|(_, boxes)| !boxes.is_empty())
            .map(|(i, boxes)| {
                let scale = palette.scales[i];
                commands
                    .spawn((
                        Mesh3d(meshes.add(box_mesh(boxes, scale))),
                        MeshMaterial3d(palette.mats[i].clone()),
                        LevelPart,
                    ))
                    .id()
            })
            .collect();
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
    }
}

/// R drops the whole world; it is generated anew around the spawn point, where the player module puts
/// the player back.
fn regenerate(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    parts: Query<Entity, With<LevelPart>>,
    palette: Res<Palette>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut level: ResMut<Level>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
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

/// All boxes of one material as a single mesh; UVs are world coordinates divided by the texture tile size.
fn box_mesh(boxes: &[Aabb], scale: f32) -> Mesh {
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
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
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
}

enum Kind {
    Building(Building),
    Plaza,
    Park,
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
    /// Places behind upper-floor windows where a watcher can stand (feet position).
    spots: Vec<Vec3>,
}

/// A movable object (tree, bench): its own entity, so it can change while the player looks away.
/// Part boxes are relative to `base`.
struct Prop {
    base: Vec3,
    parts: Vec<(Aabb, Mat)>,
}

/// Random character of a chunk: share of buildings and plazas (the rest are parks), floor count range,
/// highest hill in terrace levels, number of hills, share of building pairs joined by a passage.
struct Theme {
    building: f32,
    plaza: f32,
    floors: (u32, u32),
    peak: f32,
    hills: u32,
    connect: f32,
}

/// One chunk: ground, road marks, then blocks and passages from a theme picked at random.
fn generate(seed: u64, chunk: IVec2) -> (Vec<(Aabb, Mat)>, Vec<Prop>, Vec<Vec3>) {
    let mut b = Builder {
        rng: Rng(seed),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
    };
    let origin = chunk.as_vec2() * CHUNK_SIZE;
    b.add(
        Vec3::new(origin.x, -1.0, origin.y),
        Vec3::new(origin.x + CHUNK_SIZE, 0.0, origin.y + CHUNK_SIZE),
        Mat::Asphalt,
    );
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
        },
        // Hills: high terraces with plazas.
        1 => Theme {
            building: 0.45,
            plaza: 0.3,
            floors: (1, 3),
            peak: 5.0,
            hills: 2,
            connect: 0.6,
        },
        // Parkland: low houses among parks.
        2 => Theme {
            building: 0.3,
            plaza: 0.2,
            floors: (1, 2),
            peak: 2.0,
            hills: 2,
            connect: 0.5,
        },
        _ => Theme {
            building: 0.55,
            plaza: 0.25,
            floors: (1, 4),
            peak: 3.0,
            hills: 2,
            connect: 0.7,
        },
    };
    let mut blocks = b.plan(chunk, &theme);
    let passages = b.connect(&mut blocks, &theme);
    for block in &blocks {
        b.terrace(block);
        let (x0, z0, e) = (block.x0, block.z0, block.e);
        match &block.kind {
            Kind::Building(bl) => b.building(bl, e),
            Kind::Plaza => b.plaza(x0, z0, x0 + BLOCK, z0 + BLOCK, e),
            Kind::Park => b.park(x0, z0, x0 + BLOCK, z0 + BLOCK, e),
        }
    }
    for p in &passages {
        b.passage(p);
    }
    (b.parts, b.props, b.spots)
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
                let kind = if roll < theme.building {
                    let w = self.rng.range(12.0, BLOCK - 3.0);
                    let d = self.rng.range(12.0, BLOCK - 3.0);
                    let bx = x0 + self.rng.range(1.5, BLOCK - w - 1.5);
                    let bz = z0 + self.rng.range(1.5, BLOCK - d - 1.5);
                    let (bx1, bz1) = (bx + w, bz + d);
                    Kind::Building(Building {
                        x0: bx,
                        z0: bz,
                        x1: bx1,
                        z1: bz1,
                        floors: theme.floors.0
                            + self.rng.below(theme.floors.1 - theme.floors.0 + 1),
                        wall: Mat::Wall(self.rng.below(WALLS as u32) as u8),
                        floor: Mat::Floor(self.rng.below(FLOORS as u32) as u8),
                        xm: self.rng.range(stair_x1(bx) + 2.0, bx1 - 3.5),
                        zm: self.rng.range(stair_z1(bz) + 3.0, bz1 - 3.5),
                        gaps: Vec::new(),
                    })
                } else if roll < theme.building + theme.plaza {
                    Kind::Plaza
                } else {
                    Kind::Park
                };
                blocks.push(Block {
                    x0,
                    z0,
                    e: level * TERRACE,
                    kind,
                });
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
        let (x0, z0, x1, z1) = (bl.x0, bl.z0, bl.x1, bl.z1);
        let steps = (FH / RISE) as i32;
        let (sx0, sx1) = (stair_x0(x0), stair_x1(x0));
        let sz0 = z0 + T;
        let sz1 = stair_z1(z0);
        let (ix0, iz0, ix1, iz1) = (x0 + T, z0 + T, x1 - T, z1 - T);
        let door_side = self.rng.below(4);

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
                // Watcher spots: 0.6 m inside upper-floor windows, away from the stairwell.
                if k >= 1 && !roof {
                    for o in ops.iter().filter(|o| o.bottom > 0.0) {
                        let inside = T + 0.6;
                        let spot = match side {
                            0 => Vec3::new(o.c, y, z0 + inside),
                            1 => Vec3::new(o.c, y, z1 - inside),
                            2 => Vec3::new(x0 + inside, y, o.c),
                            _ => Vec3::new(x1 - inside, y, o.c),
                        };
                        let in_stairwell = spot.x < sx1 + 1.0 && spot.z < sz1 + 1.0;
                        if !in_stairwell {
                            self.spots.push(spot);
                        }
                    }
                }
                self.wall(along_x, u0, u1, v, y, h, T, &ops, bl.wall);
            }
            if roof {
                break;
            }
            // Interior walls, two doors each, kept away from the crossing.
            let (xm, zm) = (bl.xm, bl.zm);
            let d1 = self.rng.range(ix0 + 1.2, xm - 1.5);
            let d2 = self.rng.range(xm + 1.5, ix1 - 1.2);
            self.wall(
                true,
                ix0,
                ix1,
                zm,
                y,
                FH,
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
                FH,
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
            // sides of the hole (lane 0 lies against the south wall).
            let top = y + FH;
            let (y0, y1) = (top - 0.2, top);
            self.add(Vec3::new(ix0, y0, iz0), Vec3::new(sx0, y1, iz1), bl.floor);
            self.add(Vec3::new(sx1, y0, iz0), Vec3::new(ix1, y1, iz1), bl.floor);
            self.add(Vec3::new(sx0, y0, iz0), Vec3::new(sx1, y1, lz0), bl.floor);
            self.add(Vec3::new(sx0, y0, lz1), Vec3::new(sx1, y1, iz1), bl.floor);
            self.add(
                Vec3::new(sx0, top, lz1),
                Vec3::new(sx1, top + 1.0, lz1 + 0.08),
                Mat::Metal,
            );
            if lane == 1 {
                self.add(
                    Vec3::new(sx0, top, lz0 - 0.08),
                    Vec3::new(sx1, top + 1.0, lz0),
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
        for _ in 0..3 {
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
            self.wall(along_x, u, u + len, v, p, 0.9, 0.4, &[], Mat::Stone);
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
        for _ in 0..3 {
            let x = self.rng.range(x0 + 2.0, x1 - 2.0);
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
    };
    b.building(&bl, 0.0);
    b.parts.into_iter().map(|(a, _)| a).collect()
}

/// Boxes of one terrace block at height `e` (no content) and its south flight of steps, lowest step first.
#[cfg(test)]
pub(crate) fn test_terrace(e: f32) -> (Vec<Aabb>, Vec<Aabb>) {
    let mut b = Builder {
        rng: Rng(3),
        parts: Vec::new(),
        props: Vec::new(),
        spots: Vec::new(),
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

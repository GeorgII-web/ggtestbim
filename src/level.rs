//! Procedural city: a street grid of blocks; each block is a building (rooms, doors, windows, stairs up to a
//! walkable roof), an open plaza or a park. Everything is an axis-aligned box: the same list renders the scene
//! and is the collision world of the player.

use bevy::prelude::*;

pub struct LevelPlugin;

impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Level {
            seed: 1,
            boxes: Vec::new(),
        })
        .add_systems(Startup, (setup_palette, spawn_level).chain())
        .add_systems(Update, regenerate);
    }
}

#[derive(Clone, Copy)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

/// Collision world, rebuilt with the scene.
#[derive(Resource)]
pub struct Level {
    pub seed: u64,
    pub boxes: Vec<Aabb>,
}

#[derive(Component)]
struct LevelPart;

/// Blocks per side from the center; block pitch = block + street.
const BLOCKS: i32 = 4;
const BLOCK: f32 = 22.0;
const PITCH: f32 = 30.0;
const WALL_COLORS: u8 = 5;

#[derive(Clone, Copy)]
enum Mat {
    Asphalt,
    Paving,
    Grass,
    Floor,
    Stair,
    Trunk,
    Leaves,
    Water,
    Stone,
    Wall(u8),
}

impl Mat {
    fn index(self) -> usize {
        match self {
            Mat::Asphalt => 0,
            Mat::Paving => 1,
            Mat::Grass => 2,
            Mat::Floor => 3,
            Mat::Stair => 4,
            Mat::Trunk => 5,
            Mat::Leaves => 6,
            Mat::Water => 7,
            Mat::Stone => 8,
            Mat::Wall(i) => 9 + i as usize,
        }
    }
}

#[derive(Resource)]
struct Palette {
    cube: Handle<Mesh>,
    mats: Vec<Handle<StandardMaterial>>,
}

fn setup_palette(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let colors = [
        (0.18, 0.18, 0.2),  // asphalt
        (0.62, 0.6, 0.56),  // paving
        (0.3, 0.55, 0.25),  // grass
        (0.55, 0.42, 0.3),  // floor
        (0.5, 0.5, 0.52),   // stair
        (0.4, 0.27, 0.15),  // trunk
        (0.2, 0.45, 0.18),  // leaves
        (0.25, 0.45, 0.75), // water
        (0.7, 0.68, 0.64),  // stone
        (0.85, 0.8, 0.68),  // walls...
        (0.65, 0.3, 0.25),
        (0.9, 0.9, 0.88),
        (0.5, 0.58, 0.68),
        (0.78, 0.62, 0.42),
    ];
    let mats = colors
        .iter()
        .map(|&(r, g, b)| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(r, g, b),
                perceptual_roughness: 0.9,
                ..default()
            })
        })
        .collect();
    commands.insert_resource(Palette {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        mats,
    });
}

fn spawn_level(mut commands: Commands, palette: Res<Palette>, mut level: ResMut<Level>) {
    let parts = generate(level.seed);
    level.boxes = parts.iter().map(|(b, _)| *b).collect();
    for (b, mat) in parts {
        commands.spawn((
            Mesh3d(palette.cube.clone()),
            MeshMaterial3d(palette.mats[mat.index()].clone()),
            Transform::from_translation((b.min + b.max) / 2.0).with_scale(b.max - b.min),
            LevelPart,
        ));
    }
}

/// R builds a new city with the next seed; the player module puts the player back on the spawn point.
fn regenerate(
    keys: Res<ButtonInput<KeyCode>>,
    mut commands: Commands,
    parts: Query<Entity, With<LevelPart>>,
    palette: Res<Palette>,
    mut level: ResMut<Level>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    for e in &parts {
        commands.entity(e).despawn();
    }
    level.seed += 1;
    spawn_level(commands, palette, level);
}

/// SplitMix64: small, seedable, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        if b <= a { a } else { a + (b - a) * self.unit() }
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() % n as u64) as u32
    }
    fn chance(&mut self, p: f32) -> bool {
        self.unit() < p
    }
}

/// A gap in a wall: center and width along the wall, bottom and top above the wall base.
struct Opening {
    c: f32,
    w: f32,
    bottom: f32,
    top: f32,
}

struct Builder {
    rng: Rng,
    parts: Vec<(Aabb, Mat)>,
}

fn generate(seed: u64) -> Vec<(Aabb, Mat)> {
    let mut b = Builder {
        rng: Rng(seed),
        parts: Vec::new(),
    };
    let half = BLOCKS as f32 * PITCH + 10.0;
    b.add(
        Vec3::new(-half, -1.0, -half),
        Vec3::new(half, 0.0, half),
        Mat::Asphalt,
    );
    for i in -BLOCKS..BLOCKS {
        for j in -BLOCKS..BLOCKS {
            // Streets run along x = k * PITCH, so the spawn point (0, 0) is a crossing.
            let x0 = i as f32 * PITCH + (PITCH - BLOCK) / 2.0;
            let z0 = j as f32 * PITCH + (PITCH - BLOCK) / 2.0;
            let roll = b.rng.unit();
            if roll < 0.55 {
                let w = b.rng.range(12.0, BLOCK - 2.0);
                let d = b.rng.range(12.0, BLOCK - 2.0);
                let bx = x0 + b.rng.range(0.0, BLOCK - w);
                let bz = z0 + b.rng.range(0.0, BLOCK - d);
                let floors = 1 + b.rng.below(3);
                b.building(bx, bz, bx + w, bz + d, floors);
            } else if roll < 0.8 {
                b.plaza(x0, z0, x0 + BLOCK, z0 + BLOCK);
            } else {
                b.park(x0, z0, x0 + BLOCK, z0 + BLOCK);
            }
        }
    }
    b.parts
}

impl Builder {
    fn add(&mut self, min: Vec3, max: Vec3, mat: Mat) {
        let s = max - min;
        if s.x > 0.01 && s.y > 0.01 && s.z > 0.01 {
            self.parts.push((Aabb { min, max }, mat));
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
        let piece = |b: &mut Builder, a: f32, c: f32, ya: f32, yb: f32| {
            if along_x {
                b.add(
                    Vec3::new(a, ya, v - t / 2.0),
                    Vec3::new(c, yb, v + t / 2.0),
                    mat,
                );
            } else {
                b.add(
                    Vec3::new(v - t / 2.0, ya, a),
                    Vec3::new(v + t / 2.0, yb, c),
                    mat,
                );
            }
        };
        let mut cursor = u0;
        for o in openings {
            let a = (o.c - o.w / 2.0).max(cursor);
            let c = (o.c + o.w / 2.0).min(u1);
            if c <= a {
                continue;
            }
            piece(self, cursor, a, y0, y0 + h);
            piece(self, a, c, y0, y0 + o.bottom);
            piece(self, a, c, y0 + o.top, y0 + h);
            cursor = c;
        }
        piece(self, cursor, u1, y0, y0 + h);
    }

    /// Windows every 3 m (70 % chance each) and an optional door; sorted by position.
    fn openings(&mut self, u0: f32, u1: f32, door: Option<f32>, windows: bool) -> Vec<Opening> {
        let mut v = Vec::new();
        let mut u = u0 + 2.0;
        while u < u1 - 1.5 {
            let near_door = door.is_some_and(|d| (u - d).abs() < 1.8);
            if windows && !near_door && self.rng.chance(0.7) {
                v.push(Opening {
                    c: u,
                    w: 1.2,
                    bottom: 1.0,
                    top: 2.3,
                });
            }
            u += 3.0;
        }
        if let Some(d) = door {
            v.push(door_at(d));
        }
        v.sort_by(|a, b| a.c.total_cmp(&b.c));
        v
    }

    /// Building with `floors` stories: outer walls with windows and one street door, four rooms per floor
    /// connected by doors, a staircase in the south-west corner on every floor (the top one leads to the roof),
    /// a railing around each stair hole and a parapet on the roof.
    fn building(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, floors: u32) {
        let t = 0.3;
        let fh = 3.5;
        let wall = Mat::Wall(self.rng.below(WALL_COLORS as u32) as u8);
        let steps = 14;
        let run = 0.3;
        let rise = fh / steps as f32;
        let sx0 = x0 + t + 0.2;
        let sx1 = sx0 + steps as f32 * run;
        let sz0 = z0 + t;
        let sz1 = sz0 + 1.6;
        let xm = self.rng.range(sx1 + 2.0, x1 - 3.5);
        let zm = self.rng.range(sz1 + 3.0, z1 - 3.5);
        let door_side = self.rng.below(4);

        for k in 0..floors {
            let y = k as f32 * fh;
            let ground = k == 0;
            for side in 0..4 {
                let along_x = side < 2;
                let (u0, u1) = if along_x { (x0, x1) } else { (z0 + t, z1 - t) };
                let v = match side {
                    0 => z0 + t / 2.0,
                    1 => z1 - t / 2.0,
                    2 => x0 + t / 2.0,
                    _ => x1 - t / 2.0,
                };
                // The south door must not open onto the staircase.
                let lo = if side == 0 { sx1 + 1.5 } else { u0 + 1.5 };
                let door = (ground && side == door_side).then(|| self.rng.range(lo, u1 - 1.5));
                let ops = self.openings(u0, u1, door, true);
                self.wall(along_x, u0, u1, v, y, fh, t, &ops, wall);
            }
            // Interior walls, two doors each, kept away from the crossing.
            let d1 = self.rng.range(x0 + 1.5, xm - 1.5);
            let d2 = self.rng.range(xm + 1.5, x1 - 1.5);
            self.wall(
                true,
                x0 + t,
                x1 - t,
                zm,
                y,
                fh,
                0.2,
                &[door_at(d1), door_at(d2)],
                wall,
            );
            let d1 = self.rng.range(z0 + t + 1.0, zm - 1.5);
            let d2 = self.rng.range(zm + 1.5, z1 - 1.5);
            self.wall(
                false,
                z0 + t,
                z1 - t,
                xm,
                y,
                fh,
                0.2,
                &[door_at(d1), door_at(d2)],
                wall,
            );

            for i in 0..steps {
                let a = sx0 + i as f32 * run;
                self.add(
                    Vec3::new(a, y, sz0),
                    Vec3::new(a + run, y + (i + 1) as f32 * rise, sz1),
                    Mat::Stair,
                );
            }
            // Slab above with the stair hole, as four boxes around it.
            let top = y + fh;
            let (sy0, sy1) = (top - 0.2, top);
            self.add(Vec3::new(x0, sy0, z0), Vec3::new(sx0, sy1, z1), Mat::Floor);
            self.add(Vec3::new(sx1, sy0, z0), Vec3::new(x1, sy1, z1), Mat::Floor);
            self.add(
                Vec3::new(sx0, sy0, z0),
                Vec3::new(sx1, sy1, sz0),
                Mat::Floor,
            );
            self.add(
                Vec3::new(sx0, sy0, sz1),
                Vec3::new(sx1, sy1, z1),
                Mat::Floor,
            );
            self.add(
                Vec3::new(sx0, top, sz1),
                Vec3::new(sx1, top + 1.0, sz1 + 0.1),
                Mat::Stone,
            );
        }
        let roof = floors as f32 * fh;
        self.wall(true, x0, x1, z0 + t / 2.0, roof, 1.0, t, &[], wall);
        self.wall(true, x0, x1, z1 - t / 2.0, roof, 1.0, t, &[], wall);
        self.wall(false, z0 + t, z1 - t, x0 + t / 2.0, roof, 1.0, t, &[], wall);
        self.wall(false, z0 + t, z1 - t, x1 - t / 2.0, roof, 1.0, t, &[], wall);
    }

    /// Paved open place: a fountain, a stepped pyramid or a pillar hall, plus low walls and benches.
    fn plaza(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) {
        self.add(Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.12, z1), Mat::Paving);
        let (cx, cz) = ((x0 + x1) / 2.0, (z0 + z1) / 2.0);
        match self.rng.below(3) {
            0 => {
                let r = 3.0;
                for (along_x, v) in [(true, cz - r), (true, cz + r)] {
                    self.wall(along_x, cx - r, cx + r, v, 0.12, 0.6, 0.4, &[], Mat::Stone);
                }
                for (along_x, v) in [(false, cx - r), (false, cx + r)] {
                    self.wall(
                        along_x,
                        cz - r + 0.2,
                        cz + r - 0.2,
                        v,
                        0.12,
                        0.6,
                        0.4,
                        &[],
                        Mat::Stone,
                    );
                }
                self.add(
                    Vec3::new(cx - r + 0.2, 0.12, cz - r + 0.2),
                    Vec3::new(cx + r - 0.2, 0.4, cz + r - 0.2),
                    Mat::Water,
                );
                self.add(
                    Vec3::new(cx - 0.4, 0.12, cz - 0.4),
                    Vec3::new(cx + 0.4, 2.2, cz + 0.4),
                    Mat::Stone,
                );
            }
            1 => {
                for tier in 0..5 {
                    let s = 7.0 - tier as f32 * 1.4;
                    let y = 0.12 + tier as f32 * 0.45;
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
                let mut x = cx - s;
                while x <= cx + s + 0.1 {
                    let mut z = cz - s;
                    while z <= cz + s + 0.1 {
                        self.add(
                            Vec3::new(x - 0.35, 0.12, z - 0.35),
                            Vec3::new(x + 0.35, 4.0, z + 0.35),
                            Mat::Stone,
                        );
                        z += 4.0;
                    }
                    x += 4.0;
                }
                let (y0, y1) = (4.0, 4.4);
                self.add(
                    Vec3::new(cx - s - 1.0, y0, cz - s - 1.0),
                    Vec3::new(cx + s + 1.0, y1, cz - 3.0),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx - s - 1.0, y0, cz + 3.0),
                    Vec3::new(cx + s + 1.0, y1, cz + s + 1.0),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx - s - 1.0, y0, cz - 3.0),
                    Vec3::new(cx - 3.0, y1, cz + 3.0),
                    Mat::Stone,
                );
                self.add(
                    Vec3::new(cx + 3.0, y0, cz - 3.0),
                    Vec3::new(cx + s + 1.0, y1, cz + 3.0),
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
            self.wall(along_x, u, u + len, v, 0.12, 0.9, 0.4, &[], Mat::Stone);
        }
    }

    /// Grass with trees and benches.
    fn park(&mut self, x0: f32, z0: f32, x1: f32, z1: f32) {
        self.add(Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.08, z1), Mat::Grass);
        for _ in 0..(6 + self.rng.below(7)) {
            let x = self.rng.range(x0 + 1.5, x1 - 1.5);
            let z = self.rng.range(z0 + 1.5, z1 - 1.5);
            let h = self.rng.range(2.0, 3.0);
            let r = self.rng.range(1.0, 1.8);
            self.add(
                Vec3::new(x - 0.2, 0.08, z - 0.2),
                Vec3::new(x + 0.2, h, z + 0.2),
                Mat::Trunk,
            );
            self.add(
                Vec3::new(x - r, h, z - r),
                Vec3::new(x + r, h + r * 1.6, z + r),
                Mat::Leaves,
            );
        }
        for _ in 0..3 {
            let x = self.rng.range(x0 + 1.0, x1 - 3.0);
            let z = self.rng.range(z0 + 1.0, z1 - 1.0);
            self.add(
                Vec3::new(x, 0.08, z),
                Vec3::new(x + 1.8, 0.5, z + 0.5),
                Mat::Stone,
            );
        }
    }
}

fn door_at(c: f32) -> Opening {
    Opening {
        c,
        w: 1.4,
        bottom: 0.0,
        top: 2.4,
    }
}

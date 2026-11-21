//! Dogs: stray dogs in packs (`PACK` dogs each, up to `MAX_PACKS` packs) roam the streets around the player;
//! a pack is spawned out of sight on a street and removed when far behind. None come during the first
//! `GRACE` seconds after a (re)spawn or new world.
//! A pack wanders together (each dog steers toward its pack's center and keeps a little apart). When one dog
//! sees the player (line of sight within `Exposure`), hears a noise the player made (`Noises`: steps,
//! landings, shots) or a stranger calls (`DogCall`), the whole pack barks and gives chase. A stranger that
//! sees the player guides every dog within `GUIDE_RANGE`: for `GUIDE_TIME` seconds after its last sight the
//! dogs know the player's exact place and search a path to it at once, again every `GUIDE_REPLAN` seconds
//! or when the player moved `GUIDE_MOVE` m.
//! Chasing: a dog that sees the player runs straight at it; otherwise it sniffs the player's trail (the
//! `Trail` of breadcrumbs the player left, fresh for `SCENT` seconds), from the nearest crumb (one it can
//! see first, else one it smells behind walls, searching a path to it) toward the newest, and so takes the way the player came, also into buildings and up stairs. Without a
//! sight, a sound or a fresh crumb for `LOSE` seconds it gives up; at the end of a cold trail it sniffs
//! around (wanders).
//! A chasing dog that is blocked, makes no progress for `STUCK_CHECK` seconds, or has its target above or
//! below it (a roof, an upper floor, a pit) searches a path (`find_path`: A* over a walkable grid within
//! `SEARCH_RADIUS`, through doors, around buildings, up and down stairs) and follows it. A search that
//! does not reach the target is taken only when it gets the dog `DEAD_END_GAIN` m closer and does not end
//! at a known dead end; otherwise the dog remembers its place as a dead end, runs to a random place
//! `EXPLORE` m away and searches again from there. Dogs never stay stuck: a dog that wants to move but
//! stays within `IDLE_MOVE` for `IDLE_MAX` seconds, or finds no way at all twice, is put on a random street
//! out of the player's sight and searches again from there.
//! On a rope the player is out of reach: chasing dogs search a path to the rope end the player is nearer
//! to (the ground under where it was thrown, or under / on the surface it is tied to), wait there, and bite
//! by jumping up to `JUMP_BITE` above them.
//! Next to the player it bites (`BITE_DAMAGE` every `BITE_EVERY` s). Dogs walk on the level boxes: they
//! follow the ground, climb steps up to `STEP`, steer around walls and drop down ledges up to `DROP`.
//! Their eyes glow red. A bullet kills one: it falls on its side, lies and sinks away.
//! Every dog is different (`Traits`): fur color, size and strength (bite damage), speed, and smartness
//! (how soon it notices it is stuck, how far and hard it searches a path, how often a stranger's guidance
//! makes it search anew).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet, VecDeque};

use bevy::prelude::*;

use crate::flashlight::Exposure;
use crate::level::{Aabb, Level, PITCH, Rng, ray_box, segment_blocked};
use crate::player::{HANG, Player, Ride};
use crate::sound::{Noises, Sfx, SoundQueue};
use crate::targets::{Blip, Killed, Kind, Shootable};
use crate::watchers::DogCall;

pub struct DogsPlugin;

impl Plugin for DogsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DogRng(Rng(0xD065)))
            .insert_resource(Trail::default())
            .add_systems(Startup, setup_dogs)
            .add_systems(
                Update,
                (record_trail, spawn_dogs, run_dogs, dog_death)
                    .chain()
                    .after(crate::player::PlayerSet),
            );
    }
}

/// No dogs this long after a (re)spawn or a new world.
const GRACE: f32 = 30.0;
const MAX_PACKS: usize = 2;
/// Dogs per pack (random in the range).
const PACK: (u32, u32) = (3, 4);
const SPAWN_EVERY: f32 = 4.0;
const SPAWN_DIST: (f32, f32) = (45.0, 80.0);
const DESPAWN_DIST: f32 = 120.0;
/// Speeds of an average dog (times its `Traits::speed`).
const WANDER_SPEED: f32 = 2.2;
const CHASE_SPEED: f32 = 9.5;
/// Trait ranges: strength (bite damage factor, also the size), speed factor, smartness 0..1.
const STRENGTH: (f32, f32) = (0.5, 1.8);
const SPEED: (f32, f32) = (0.75, 1.3);
/// Fur colors to pick from (each varied a little).
const FURS: [(f32, f32, f32); 7] = [
    (0.22, 0.16, 0.11),
    (0.06, 0.06, 0.06),
    (0.42, 0.42, 0.42),
    (0.62, 0.5, 0.32),
    (0.8, 0.78, 0.72),
    (0.45, 0.22, 0.1),
    (0.3, 0.27, 0.24),
];
/// Without seeing or hearing the player or smelling a fresh crumb this long, a chasing dog gives up.
const LOSE: f32 = 15.0;
/// A trail crumb can be smelled this long after the player left it.
const SCENT: f32 = 60.0;
/// Path search: grid step, reach around the dog (and up and down), node budget, searches per frame (all
/// dogs together); a path is dropped after `PATH_MAX` seconds and searched anew if still needed.
const GRID: f32 = 0.7;
const SEARCH_RADIUS: f32 = 32.0;
/// Up and down: the deepest pit (32 m) and the roof of the highest tower (18 floors, 63 m).
const SEARCH_HEIGHT: f32 = 70.0;
const SEARCH_NODES: usize = 8000;
/// Weight on the remaining-distance estimate (weighted A*: faster, paths a little longer).
const GREED: f32 = 1.5;
const SEARCHES_PER_FRAME: usize = 1;
const PATH_MAX: f32 = 10.0;
/// Never stuck: wanting to move but staying within `IDLE_MOVE` m for `IDLE_MAX` s, or `FAILS` failed
/// searches in a row, puts the dog on a street `RELOCATE_DIST` from the player, out of sight.
const IDLE_MOVE: f32 = 1.0;
const IDLE_MAX: f32 = 6.0;
const FAILS: u32 = 2;
const RELOCATE_DIST: (f32, f32) = (30.0, 70.0);
/// A search that does not reach the target must get the dog this much closer (else it is a dead end);
/// dead ends remembered per dog; how far a random run from a dead end goes (and at least `EXPLORE_MIN`).
const DEAD_END_GAIN: f32 = 3.0;
const DEAD_ENDS: usize = 4;
const EXPLORE: (f32, f32) = (15.0, 30.0);
const EXPLORE_MIN: f32 = 6.0;
/// How high above itself a dog bites a player hanging on a rope (it jumps).
const JUMP_BITE: f32 = 2.4;
/// A stranger's call about a noise reaches dogs within this distance of it; its guidance (it sees the
/// player) reaches dogs within `GUIDE_RANGE`, lasts `GUIDE_TIME` s after the last sight, and the path is
/// searched anew every `GUIDE_REPLAN` s or when the player moved `GUIDE_MOVE` m.
const CALL_RANGE: f32 = 90.0;
const GUIDE_RANGE: f32 = 120.0;
const GUIDE_TIME: f32 = 4.0;
const GUIDE_REPLAN: f32 = 2.0;
const GUIDE_MOVE: f32 = 3.0;
const BITE_RANGE: f32 = 1.3;
const BITE_DAMAGE: f32 = 12.0;
const BITE_EVERY: f32 = 0.9;
/// Highest step up and deepest drop a dog takes.
const STEP: f32 = 0.6;
const DROP: f32 = 3.0;
/// Eye height of a dog, for its line of sight.
const EYE: f32 = 0.6;
const SIGHT_EVERY: f32 = 0.2;
/// A paw sound every this many meters, walking and running.
const PAW_STRIDE_WALK: f32 = 0.45;
const PAW_STRIDE_RUN: f32 = 0.8;
/// Pack: wander back toward the center beyond `COHESION`, keep `SPACING` from each other.
const COHESION: f32 = 6.0;
const SPACING: f32 = 1.2;
/// Trail: a crumb every `CRUMB` meters the player walks on the ground, at most `CRUMBS` kept; a dog picks
/// up the trail at a visible crumb within `PICKUP`.
const CRUMB: f32 = 1.5;
const CRUMBS: usize = 600;
const PICKUP: f32 = 30.0;
/// Stuck: less than `PROGRESS` m closer to the target in `STUCK_CHECK` s.
const STUCK_CHECK: f32 = 1.5;
const PROGRESS: f32 = 0.7;

#[derive(Resource)]
struct DogRng(Rng);

/// Breadcrumbs of the player's path on the ground, oldest first; each has a running id and the time it
/// was left.
#[derive(Resource, Default)]
struct Trail {
    crumbs: VecDeque<(u64, Vec3, f32)>,
    next: u64,
}

impl Trail {
    /// A crumb that still smells to a dog that smells crumbs `scent` seconds old.
    fn get(&self, id: u64, now: f32, scent: f32) -> Option<Vec3> {
        let first = self.crumbs.front()?.0;
        self.crumbs
            .get(id.checked_sub(first)? as usize)
            .filter(|c| now - c.2 < scent)
            .map(|c| c.1)
    }
}

#[derive(Resource)]
struct DogAssets {
    cube: Handle<Mesh>,
    dark: Handle<StandardMaterial>,
    eyes: Handle<StandardMaterial>,
}

enum Mode {
    Wander {
        heading: f32,
        turn_in: f32,
    },
    Chase {
        last_seen: Vec3,
        lost: f32,
        bark_in: f32,
    },
}

/// What makes a dog different: bite damage factor (and size), speed factor, smartness 0..1.
#[derive(Clone, Copy)]
struct Traits {
    strength: f32,
    speed: f32,
    smart: f32,
}

impl Traits {
    /// How long without progress before it searches a path.
    fn stuck_check(&self) -> f32 {
        STUCK_CHECK * (1.8 - self.smart)
    }
    /// Reach and node budget of its path searches.
    fn search(&self) -> (f32, usize) {
        (
            SEARCH_RADIUS * (0.5 + 0.6 * self.smart),
            (SEARCH_NODES as f32 * (0.3 + 0.7 * self.smart)) as usize,
        )
    }
    fn guide_replan(&self) -> f32 {
        GUIDE_REPLAN * (2.0 - self.smart)
    }
}

#[derive(Component)]
struct Dog {
    pack: u32,
    traits: Traits,
    mode: Mode,
    bite_in: f32,
    sight_in: f32,
    sees: bool,
    /// Gait phase for the legs.
    gait: f32,
    /// Meters since the last paw sound.
    walked: f32,
    /// The trail crumb it heads for, when following the trail.
    crumb: Option<u64>,
    /// Stuck check: time since the last check and the distance to the target then.
    check_t: f32,
    check_dist: f32,
    /// Turn bias around small obstacles (+1 left first, -1 right first), flipped on each search.
    bias: f32,
    /// A searched path (waypoints, next first), its age, and a search wanted toward this target.
    path: VecDeque<Vec3>,
    path_age: f32,
    search: Option<Vec3>,
    /// Failed searches in a row.
    fails: u32,
    /// Idle watch: where it was when it last moved `IDLE_MOVE` m, and the time since.
    idle_from: Vec3,
    idle_t: f32,
    /// Guided by a stranger (or the player on a rope): seconds left, and the target its path was searched to.
    guided: f32,
    guide_to: Vec3,
    /// Places where searches led nowhere, newest last; on a random run away from one.
    dead_ends: VecDeque<Vec3>,
    exploring: bool,
}

#[derive(Component)]
struct Leg {
    front: bool,
    left: bool,
}

fn setup_dogs(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(DogAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        dark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.06, 0.05, 0.04),
            perceptual_roughness: 1.0,
            ..default()
        }),
        eyes: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.1, 0.05),
            emissive: LinearRgba::rgb(40.0, 2.0, 1.0),
            unlit: true,
            ..default()
        }),
    });
}

/// Ground height under `p` (the top of the highest box below `p + up`), if any within `DROP + up`.
fn ground(level: &Level, p: Vec3, up: f32) -> Option<f32> {
    level
        .raycast(p + Vec3::Y * up, Vec3::NEG_Y, up + DROP)
        .map(|(t, _)| p.y + up - t)
}

/// A crumb every `CRUMB` meters of the player's walk on the ground; cleared on a new spawn.
fn record_trail(time: Res<Time>, player: Single<&Player>, mut trail: ResMut<Trail>) {
    if player.since_spawn < 0.1 {
        trail.crumbs.clear();
    }
    if player.motion() == crate::player::Motion::Air || player.dead {
        return;
    }
    let feet = player.feet();
    let far = trail
        .crumbs
        .back()
        .is_none_or(|c| c.1.distance(feet) >= CRUMB);
    if far {
        let id = trail.next;
        trail.next += 1;
        trail.crumbs.push_back((id, feet, time.elapsed_secs()));
        if trail.crumbs.len() > CRUMBS {
            trail.crumbs.pop_front();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_dogs(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
    time: Res<Time>,
    assets: Res<DogAssets>,
    level: Res<Level>,
    keys: Res<ButtonInput<KeyCode>>,
    camera: Single<(&Transform, &Player)>,
    dogs: Query<(Entity, &Transform, &Dog)>,
    mut rng: ResMut<DogRng>,
    mut timer: Local<f32>,
    mut next_pack: Local<u32>,
) {
    let (camera, player) = *camera;
    let eye = camera.translation;
    let grace = player.since_spawn < GRACE || player.dead;
    // Spawn grace, new world or far away: gone.
    for (e, tf, _) in &dogs {
        if grace
            || keys.just_pressed(KeyCode::KeyN)
            || tf.translation.distance(eye) > DESPAWN_DIST
            || !level.loaded(tf.translation)
        {
            commands.entity(e).despawn();
        }
    }
    *timer -= time.delta_secs();
    if grace || *timer > 0.0 {
        return;
    }
    *timer = SPAWN_EVERY;
    let mut packs: Vec<u32> = dogs.iter().map(|(_, _, d)| d.pack).collect();
    packs.sort_unstable();
    packs.dedup();
    if packs.len() >= MAX_PACKS {
        return;
    }
    let rng = &mut rng.0;
    if let Some((p, along_z)) = street_spot(&level, eye, rng, SPAWN_DIST) {
        // The pack: dogs a meter or two apart along the street.
        let pack = *next_pack;
        *next_pack += 1;
        let n = PACK.0 + rng.below(PACK.1 - PACK.0 + 1);
        let heading = rng.range(0.0, std::f32::consts::TAU);
        for i in 0..n {
            let off = (i as f32 - n as f32 / 2.0) * 1.6;
            let mut q = p + if along_z {
                Vec3::Z * off
            } else {
                Vec3::X * off
            };
            if let Some(g) = ground(&level, q, 0.5) {
                q.y = g;
                spawn_dog(
                    &mut commands,
                    &assets,
                    &mut materials,
                    rng,
                    q,
                    heading,
                    pack,
                );
            }
        }
    }
}

/// A place on the middle of a street `dist` from `eye`, loaded, with ground, out of the player's sight (up to
/// 8 tries); and whether the street runs along z.
fn street_spot(level: &Level, eye: Vec3, rng: &mut Rng, dist: (f32, f32)) -> Option<(Vec3, bool)> {
    for _ in 0..8 {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let d = rng.range(dist.0, dist.1);
        let mut p = eye + Vec3::new(angle.cos(), 0.0, angle.sin()) * d;
        // Onto the middle of the nearest street (streets run along x and z at multiples of PITCH).
        let (sx, sz) = ((p.x / PITCH).round() * PITCH, (p.z / PITCH).round() * PITCH);
        let along_z = (p.x - sx).abs() < (p.z - sz).abs();
        if along_z {
            p.x = sx;
        } else {
            p.z = sz;
        }
        p.y = 0.0;
        if !level.loaded(p) || !level.blocked(eye, p + Vec3::Y * EYE) {
            continue;
        }
        if let Some(g) = ground(level, p, 0.5) {
            return Some((Vec3::new(p.x, g, p.z), along_z));
        }
    }
    None
}

fn spawn_dog(
    commands: &mut Commands,
    assets: &DogAssets,
    materials: &mut Assets<StandardMaterial>,
    rng: &mut Rng,
    at: Vec3,
    heading: f32,
    pack: u32,
) {
    let traits = Traits {
        strength: rng.range(STRENGTH.0, STRENGTH.1),
        speed: rng.range(SPEED.0, SPEED.1),
        smart: rng.unit(),
    };
    let (r, g, b) = FURS[rng.below(FURS.len() as u32) as usize];
    let k = rng.range(0.8, 1.2);
    let fur = materials.add(StandardMaterial {
        base_color: Color::srgb(r * k, g * k, b * k),
        perceptual_roughness: 1.0,
        ..default()
    });
    // Stronger dogs are bigger.
    let size = 0.75 + 0.4 * (traits.strength - STRENGTH.0) / (STRENGTH.1 - STRENGTH.0);
    let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
        (
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(center).with_scale(size),
        )
    };
    // Modeled facing local -z: body, head, snout, ears, red eyes, tail, four legs.
    commands
        .spawn((
            Transform::from_translation(at)
                .with_rotation(Quat::from_rotation_y(heading))
                .with_scale(Vec3::splat(size)),
            Visibility::default(),
            Dog {
                pack,
                traits,
                mode: Mode::Wander {
                    heading,
                    turn_in: 3.0,
                },
                bite_in: 0.0,
                sight_in: 0.0,
                sees: false,
                gait: 0.0,
                walked: 0.0,
                crumb: None,
                check_t: 0.0,
                check_dist: f32::MAX,
                bias: 1.0,
                path: VecDeque::new(),
                path_age: 0.0,
                search: None,
                fails: 0,
                idle_from: at,
                idle_t: 0.0,
                guided: 0.0,
                guide_to: at,
                dead_ends: VecDeque::new(),
                exploring: false,
            },
            Shootable {
                kind: Kind::Dog,
                half: 0.4 * size,
                height: 0.9 * size,
            },
            Blip(Kind::Dog),
        ))
        .with_children(|c| {
            c.spawn(part(
                Vec3::new(0.0, 0.55, 0.0),
                Vec3::new(0.3, 0.3, 0.8),
                &fur,
            ));
            c.spawn(part(
                Vec3::new(0.0, 0.78, -0.42),
                Vec3::new(0.22, 0.24, 0.26),
                &fur,
            ));
            c.spawn(part(
                Vec3::new(0.0, 0.72, -0.6),
                Vec3::new(0.12, 0.1, 0.16),
                &assets.dark,
            ));
            for x in [-0.07, 0.07] {
                c.spawn(part(
                    Vec3::new(x, 0.95, -0.38),
                    Vec3::new(0.05, 0.1, 0.04),
                    &assets.dark,
                ));
                // Eyes on the front of the head, glowing red.
                c.spawn(part(
                    Vec3::new(x * 0.9, 0.84, -0.556),
                    Vec3::new(0.045, 0.03, 0.01),
                    &assets.eyes,
                ));
            }
            // Tail, raised.
            c.spawn((
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(fur.clone()),
                Transform::from_xyz(0.0, 0.72, 0.5)
                    .with_rotation(Quat::from_rotation_x(0.6))
                    .with_scale(Vec3::new(0.05, 0.05, 0.3)),
            ));
            for (front, z) in [(true, -0.3), (false, 0.3)] {
                for (left, x) in [(true, -0.1), (false, 0.1)] {
                    c.spawn((
                        part(
                            Vec3::new(x, 0.2, z),
                            Vec3::new(0.08, 0.4, 0.08),
                            &assets.dark,
                        ),
                        Leg { front, left },
                    ));
                }
            }
        });
}

/// Tries to move `step` meters along `dir`, turning up to 110 degrees around obstacles (first to the side
/// of `bias`); the new position (on the ground) and the direction taken, or None when every way is blocked.
fn step_toward(level: &Level, pos: Vec3, dir: Vec3, step: f32, bias: f32) -> Option<(Vec3, Vec3)> {
    [0.0f32, 35.0, -35.0, 70.0, -70.0, 110.0, -110.0]
        .into_iter()
        .map(|turn| Quat::from_rotation_y((turn * bias).to_radians()) * dir)
        .find_map(|d| try_step(level, pos, d, step).map(|p| (p, d)))
}

/// One step straight along `d`: body height clear ahead (above a step) and ground there not too high or
/// too deep; the new position on the ground.
fn try_step(level: &Level, pos: Vec3, d: Vec3, step: f32) -> Option<Vec3> {
    let look = pos + d * (step + 0.35);
    if level.blocked(pos + Vec3::Y * (STEP + 0.1), look + Vec3::Y * (STEP + 0.1)) {
        return None;
    }
    let next = pos + d * step;
    let g = ground(level, next, STEP + 0.05)?;
    if g - pos.y > STEP || pos.y - g > DROP {
        return None;
    }
    Some(Vec3::new(next.x, g, next.z))
}

/// A straight walk to `target` looks possible: nothing in the way at body height and the first step works.
fn clear_way(level: &Level, pos: Vec3, target: Vec3) -> bool {
    let to = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z);
    let h = Vec3::Y * (STEP + 0.1);
    !level.blocked(pos + h, target + h)
        && (to.length() < 0.5 || try_step(level, pos, to.normalize(), 0.6).is_some())
}

/// The nearest fresh trail crumb within `PICKUP` (the newest among equals): one the dog can see, else the
/// nearest it smells (behind a wall, in a building; it searches a path there).
fn pick_crumb(level: &Level, trail: &Trail, pos: Vec3, now: f32, scent: f32) -> Option<u64> {
    let mut near: Vec<(f32, u64, Vec3)> = trail
        .crumbs
        .iter()
        .filter(|c| now - c.2 < scent)
        .map(|&(id, p, _)| (p.distance(pos), id, p))
        .filter(|c| c.0 < PICKUP)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
    near.iter()
        .take(10)
        .find(|c| !level.blocked(pos + Vec3::Y * EYE, c.2 + Vec3::Y * EYE))
        .or(near.first())
        .map(|c| c.1)
}

/// Where dogs wait for a player on a rope: the ground under the end the player is nearer to (under where it
/// was thrown, or in front of the wall it is tied to), or the floor or roof it is tied to.
fn rope_end(level: &Level, r: &Ride) -> Vec3 {
    let under = |p: Vec3| {
        level
            .raycast(p, Vec3::NEG_Y, 80.0)
            .map_or(p, |h| p - Vec3::Y * h.0)
    };
    if r.t < 0.5 {
        under(r.a - Vec3::Y * (HANG - 0.2))
    } else if r.nb.y > 0.5 {
        r.b
    } else {
        under(r.b + Vec3::new(r.nb.x, 0.0, r.nb.z).normalize_or_zero() * 0.6)
    }
}

/// A random run from a dead end: a path toward a random place `EXPLORE` m away (up to four directions),
/// taken when it leads at least `EXPLORE_MIN` m away.
fn explore(
    level: &Level,
    from: Vec3,
    rng: &mut Rng,
    (radius, nodes): (f32, usize),
) -> Option<Vec<Vec3>> {
    for _ in 0..4 {
        let a = rng.range(0.0, std::f32::consts::TAU);
        let to =
            from + Vec3::new(a.cos(), 0.0, a.sin()) * rng.range(EXPLORE.0, EXPLORE.1).min(radius);
        if let Some(path) = find_path(level, from, to, (radius, nodes / 2))
            && path
                .last()
                .is_some_and(|e| Vec2::new(e.x - from.x, e.z - from.z).length() >= EXPLORE_MIN)
        {
            return Some(path);
        }
    }
    None
}

/// One move straight along `d` over the given boxes (for path searches), in sub-steps of at most 0.35 m so
/// each climbs at most one stair step: body height clear just ahead, ground not too high or too deep.
fn step_in(boxes: &[Aabb], pos: Vec3, d: Vec3, step: f32) -> Option<Vec3> {
    let n = (step / 0.35).ceil().max(1.0);
    let sub = step / n;
    let h = Vec3::Y * (STEP + 0.1);
    let up = STEP + 0.05;
    let mut p = pos;
    for _ in 0..n as usize {
        let next = p + d * sub;
        // The highest ground under the paws (three rays along the body, so a step edge is never missed).
        let g = [-0.15, 0.0, 0.15]
            .iter()
            .filter_map(|&o| {
                let top = next + d * o + Vec3::Y * up;
                boxes
                    .iter()
                    .filter_map(|b| ray_box(top, Vec3::NEG_Y, up + DROP, b))
                    .map(|h| top.y - h.0)
                    .max_by(f32::total_cmp)
            })
            .max_by(f32::total_cmp)?;
        if g - p.y > STEP || p.y - g > DROP {
            return None;
        }
        let at = Vec3::new(next.x, g, next.z);
        if segment_blocked(boxes, p + h, at + d * 0.3 + h) {
            return None;
        }
        p = at;
    }
    Some(p)
}

/// Grid cell of a walkable place (x, z by `GRID`, height by a quarter meter).
fn cell(p: Vec3) -> (i32, i32, i32) {
    (
        (p.x / GRID).round() as i32,
        (p.z / GRID).round() as i32,
        (p.y / 0.25).round() as i32,
    )
}

/// Weighted A* from `from` toward `to` over walkable grid steps (8 directions, steps up to `STEP`, drops up to
/// `DROP`) within `radius` (at most `SEARCH_RADIUS` for the smartest dog) and `nodes` nodes. The waypoints to the target, or to the
/// reachable place nearest it; None when no step at all is possible.
fn find_path(
    level: &Level,
    from: Vec3,
    to: Vec3,
    (radius, nodes): (f32, usize),
) -> Option<Vec<Vec3>> {
    let reach = Vec3::new(radius, SEARCH_HEIGHT, radius);
    path_over(
        &level.boxes_in(from - reach, from + reach),
        from,
        to,
        reach,
        nodes,
    )
}

/// `find_path` over the given boxes.
fn path_over(
    boxes: &[Aabb],
    from: Vec3,
    to: Vec3,
    reach: Vec3,
    max_nodes: usize,
) -> Option<Vec<Vec3>> {
    let h = |p: Vec3| Vec2::new(to.x - p.x, to.z - p.z).length() + 1.5 * (to.y - p.y).abs();
    let goal =
        |p: Vec3| Vec2::new(to.x - p.x, to.z - p.z).length() < 1.0 && (to.y - p.y).abs() < 1.0;
    // Nodes: position, parent, cost so far.
    let mut nodes: Vec<(Vec3, usize, f32)> = vec![(from, usize::MAX, 0.0)];
    let mut best_of: HashMap<(i32, i32, i32), usize> = HashMap::from([(cell(from), 0)]);
    let mut closed: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut open = BinaryHeap::from([Reverse(((h(from) * 100.0) as u32, 0usize))]);
    let (mut best, mut best_h) = (0, h(from));
    let dirs: Vec<(Vec3, f32)> = (0..8)
        .map(|i| {
            let a = i as f32 * std::f32::consts::FRAC_PI_4;
            let len = if i % 2 == 0 {
                GRID
            } else {
                GRID * std::f32::consts::SQRT_2
            };
            (Vec3::new(a.cos(), 0.0, a.sin()), len)
        })
        .collect();
    while let Some(Reverse((_, i))) = open.pop() {
        let (p, _, g) = nodes[i];
        let k = cell(p);
        if best_of.get(&k) != Some(&i) || !closed.insert(k) {
            continue;
        }
        let hp = h(p);
        if hp < best_h {
            (best, best_h) = (i, hp);
        }
        if goal(p) {
            best = i;
            break;
        }
        if closed.len() > max_nodes {
            break;
        }
        // Only the boxes around this node for its steps.
        let (lo, hi) = (
            p - Vec3::new(2.0, DROP + 0.5, 2.0),
            p + Vec3::new(2.0, STEP + 0.5, 2.0),
        );
        let local: Vec<Aabb> = boxes
            .iter()
            .filter(|b| b.max.cmpgt(lo).all() && b.min.cmplt(hi).all())
            .copied()
            .collect();
        for &(d, len) in &dirs {
            let Some(n) = step_in(&local, p, d, len) else {
                continue;
            };
            if (n - from).abs().cmpgt(reach).any() {
                continue;
            }
            let kn = cell(n);
            if closed.contains(&kn) {
                continue;
            }
            let ng = g + len + 0.5 * (n.y - p.y).abs();
            if best_of.get(&kn).is_some_and(|&j| nodes[j].2 <= ng) {
                continue;
            }
            nodes.push((n, i, ng));
            let j = nodes.len() - 1;
            best_of.insert(kn, j);
            open.push(Reverse((((ng + GREED * h(n)) * 100.0) as u32, j)));
        }
    }
    if best == 0 {
        return None;
    }
    let mut path = Vec::new();
    let mut i = best;
    while i != 0 {
        path.push(nodes[i].0);
        i = nodes[i].1;
    }
    path.reverse();
    Some(path)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_dogs(
    time: Res<Time>,
    level: Res<Level>,
    exposure: Res<Exposure>,
    trail: Res<Trail>,
    noises: Res<Noises>,
    mut call: ResMut<DogCall>,
    mut player: Single<(&mut Player, &Transform)>,
    mut dogs: Query<(&mut Dog, &mut Transform, &Children), (Without<Killed>, Without<Player>)>,
    mut legs: Query<(&Leg, &mut Transform), (Without<Dog>, Without<Player>)>,
    mut rng: ResMut<DogRng>,
    mut sound: ResMut<SoundQueue>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let (player, camera) = &mut *player;
    let eye = camera.translation;
    let feet = player.feet();
    let called = call.0.take();
    let on_rope = player.rope.map(|r| rope_end(&level, &r));

    // Pass 1: who sees, hears or is called; the pack follows its first alerted dog.
    let mut pack_alert: HashMap<u32, (Vec3, bool)> = HashMap::new();
    let mut pack_pos: HashMap<u32, (Vec3, f32)> = HashMap::new();
    let mut positions: Vec<Vec3> = Vec::new();
    for (mut dog, tf, _) in &mut dogs {
        let pos = tf.translation;
        positions.push(pos);
        let c = pack_pos.entry(dog.pack).or_insert((Vec3::ZERO, 0.0));
        c.0 += pos;
        c.1 += 1.0;
        dog.sight_in -= dt;
        if dog.sight_in <= 0.0 {
            dog.sight_in = SIGHT_EVERY;
            dog.sees = !player.dead
                && pos.distance(eye) < exposure.0
                && !level.blocked(pos + Vec3::Y * EYE, eye);
        }
        let heard = noises
            .0
            .iter()
            .filter(|n| n.0.distance(pos) < n.1)
            .map(|n| n.0)
            .next();
        let called_here = called
            .filter(|&(_, from, seen)| {
                from.distance(pos) < if seen { GUIDE_RANGE } else { CALL_RANGE }
            })
            .map(|c| c.0);
        dog.guided -= dt;
        if called.is_some_and(|(_, from, seen)| seen && from.distance(pos) < GUIDE_RANGE) {
            dog.guided = GUIDE_TIME;
        }
        let alert = if dog.sees {
            Some((feet, true))
        } else {
            heard.or(called_here).map(|p| (p, false))
        };
        if let Some(a) = alert {
            let e = pack_alert.entry(dog.pack).or_insert(a);
            // A sight beats a sound.
            if a.1 && !e.1 {
                *e = a;
            }
        }
    }

    let mut searches = 0;
    for (mut dog, mut tf, children) in &mut dogs {
        let dog = &mut *dog;
        let pos = tf.translation;
        let traits = dog.traits;
        let scent = SCENT;
        let (wander_speed, chase_speed) = (WANDER_SPEED * traits.speed, CHASE_SPEED * traits.speed);

        // Alerts: start or refresh the chase.
        if let Some(&(target, _)) = pack_alert.get(&dog.pack) {
            let target = if dog.sees { feet } else { target };
            match &mut dog.mode {
                Mode::Chase {
                    last_seen, lost, ..
                } => {
                    *last_seen = target;
                    *lost = 0.0;
                }
                Mode::Wander { .. } => {
                    sound.play(Sfx::Bark, Some(pos), 1.0);
                    dog.mode = Mode::Chase {
                        last_seen: target,
                        lost: 0.0,
                        bark_in: rng.0.range(1.5, 3.5),
                    };
                    dog.crumb = None;
                    dog.path.clear();
                    dog.check_t = 0.0;
                    dog.check_dist = f32::MAX;
                }
            }
        }

        let (want, speed) = match &mut dog.mode {
            Mode::Wander { heading, turn_in } => {
                *turn_in -= dt;
                if *turn_in <= 0.0 {
                    *turn_in = rng.0.range(2.0, 6.0);
                    *heading += rng.0.range(-1.5, 1.5);
                }
                let mut want = Vec3::new(-heading.sin(), 0.0, -heading.cos());
                // Back toward the pack when straying.
                if let Some((sum, n)) = pack_pos.get(&dog.pack) {
                    let to = *sum / *n - pos;
                    let flat = Vec3::new(to.x, 0.0, to.z);
                    if flat.length() > COHESION {
                        want = (want + flat.normalize() * 1.5).normalize();
                    }
                }
                (want, wander_speed)
            }
            Mode::Chase {
                last_seen,
                lost,
                bark_in,
            } => {
                *bark_in -= dt;
                if *bark_in <= 0.0 {
                    *bark_in = rng.0.range(1.5, 3.5);
                    sound.play(Sfx::Bark, Some(pos), 1.0);
                }
                // Where to run: straight at a visible player; else along the player's trail toward the
                // newest crumb; else to where the player was last seen or heard.
                let guided = dog.guided > 0.0 || on_rope.is_some();
                let target = if let Some(end) = on_rope {
                    dog.crumb = None;
                    end
                } else if dog.sees || guided {
                    dog.crumb = None;
                    feet
                } else {
                    if dog.crumb.and_then(|id| trail.get(id, now, scent)).is_none() {
                        dog.crumb = pick_crumb(&level, &trail, pos, now, scent);
                    }
                    match dog
                        .crumb
                        .and_then(|id| trail.get(id, now, scent).map(|p| (id, p)))
                    {
                        Some((id, p)) => {
                            let flat = Vec2::new(p.x - pos.x, p.z - pos.z).length();
                            if flat < 1.2 && (p.y - pos.y).abs() < 1.5 {
                                dog.crumb = trail.get(id + 1, now, scent).map(|_| id + 1);
                            }
                            dog.crumb
                                .and_then(|id| trail.get(id, now, scent))
                                .unwrap_or(*last_seen)
                        }
                        None => *last_seen,
                    }
                };
                // The scent keeps the chase going.
                if !dog.sees && dog.crumb.is_none() && on_rope.is_none() {
                    *lost += dt;
                }
                let to = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z);
                let d = to.length();
                let level_diff = (target.y - pos.y).abs();

                // Guided (or the player on a rope): a fresh path to the exact target, unless it can run
                // straight there, is already there, or is on a random run from a dead end.
                if guided
                    && dog.search.is_none()
                    && !dog.exploring
                    && !(d < 2.0 && level_diff < 1.2)
                {
                    let due = dog.path.is_empty()
                        || dog.path_age > traits.guide_replan()
                        || target.distance(dog.guide_to) > GUIDE_MOVE;
                    let straight = level_diff < 1.0 && clear_way(&level, pos, target);
                    if due && !straight {
                        dog.search = Some(target);
                        dog.guide_to = target;
                    }
                }

                // On a path: drop the waypoints reached; leave it when it is old or the way is clear.
                if !dog.path.is_empty() {
                    dog.path_age += dt;
                    while dog.path.front().is_some_and(|w| {
                        Vec2::new(w.x - pos.x, w.z - pos.z).length() < 0.45
                            && (w.y - pos.y).abs() < 0.8
                    }) {
                        dog.path.pop_front();
                    }
                    let clear =
                        dog.path_age > 1.0 && level_diff < 1.0 && clear_way(&level, pos, target);
                    if dog.path_age > PATH_MAX || clear {
                        dog.path.clear();
                        dog.check_t = 0.0;
                        dog.check_dist = f32::MAX;
                    }
                }
                if let Some(w) = dog.path.front() {
                    let to = Vec3::new(w.x - pos.x, 0.0, w.z - pos.z);
                    (to.normalize_or_zero(), chase_speed)
                } else if dog.exploring {
                    // At the end of a random run: search the way to the target again from here.
                    dog.exploring = false;
                    dog.search = Some(target);
                    (Vec3::ZERO, 0.0)
                } else {
                    // Stuck check: not getting closer.
                    dog.check_t += dt;
                    let stuck = dog.check_t > traits.stuck_check() && {
                        let s = d > BITE_RANGE * 2.0 && dog.check_dist - d < PROGRESS;
                        dog.check_t = 0.0;
                        dog.check_dist = d;
                        s
                    };
                    // Above or below: the way there must be searched.
                    let other_level = d < 2.0 && level_diff >= 1.2;
                    if stuck || other_level {
                        dog.search = Some(target);
                    }
                    if *lost > LOSE {
                        let heading = rng.0.range(0.0, std::f32::consts::TAU);
                        dog.mode = Mode::Wander {
                            heading,
                            turn_in: 3.0,
                        };
                        (Vec3::ZERO, 0.0)
                    } else if d < 0.8 && level_diff < 1.2 {
                        if !dog.sees && dog.crumb.is_none() && on_rope.is_none() {
                            // End of a cold trail: sniff around.
                            let heading = rng.0.range(0.0, std::f32::consts::TAU);
                            dog.mode = Mode::Wander {
                                heading,
                                turn_in: 2.0,
                            };
                        }
                        (Vec3::ZERO, 0.0)
                    } else if d < 0.8 {
                        // Right above or below: circle while the way is searched.
                        let a = now * 2.0 + dog.pack as f32;
                        (Vec3::new(a.cos(), 0.0, a.sin()), wander_speed)
                    } else {
                        (to.normalize(), chase_speed)
                    }
                }
            }
        };

        // A wanted search, within the frame's budget; two failures in a row put the dog elsewhere.
        let mut relocate = false;
        if let Some(target) = dog.search
            && searches < SEARCHES_PER_FRAME
            && matches!(dog.mode, Mode::Chase { .. })
        {
            searches += 1;
            dog.search = None;
            dog.bias = -dog.bias;
            // Remaining way (vertical counts more); a path is taken when it reaches the target or gets
            // clearly closer without ending at a known dead end.
            let left = |p: Vec3| {
                Vec2::new(target.x - p.x, target.z - p.z).length() + 1.5 * (target.y - p.y).abs()
            };
            let useful = find_path(&level, pos, target, traits.search()).filter(|path| {
                let end = *path.last().unwrap_or(&pos);
                left(end) < 2.5
                    || (left(pos) - left(end) > DEAD_END_GAIN
                        && !dog.dead_ends.iter().any(|d| d.distance(end) < 3.0))
            });
            match useful {
                Some(path) => {
                    if path.last().is_some_and(|&e| left(e) < 2.5) {
                        dog.dead_ends.clear();
                    }
                    dog.path = path.into();
                    dog.path_age = 0.0;
                    dog.fails = 0;
                    dog.exploring = false;
                }
                None => {
                    // A dead end: remember it and run to a random place, then search again.
                    dog.dead_ends.push_back(pos);
                    if dog.dead_ends.len() > DEAD_ENDS {
                        dog.dead_ends.pop_front();
                    }
                    match explore(&level, pos, &mut rng.0, traits.search()) {
                        Some(path) => {
                            dog.path = path.into();
                            dog.path_age = 0.0;
                            dog.fails = 0;
                            dog.exploring = true;
                        }
                        None => {
                            dog.fails += 1;
                            relocate = dog.fails >= FAILS;
                        }
                    }
                }
            }
            dog.check_t = 0.0;
            dog.check_dist = f32::MAX;
        }

        // Keep a little apart from the others.
        let mut want = want;
        if want != Vec3::ZERO {
            for p in &positions {
                let d = Vec3::new(pos.x - p.x, 0.0, pos.z - p.z);
                let l = d.length();
                if l > 0.01 && l < SPACING {
                    want = (want + d / l * (SPACING - l)).normalize_or_zero();
                }
            }
        }

        // Bite when next to the player.
        dog.bite_in -= dt;
        let flat = Vec2::new(feet.x - pos.x, feet.z - pos.z).length();
        // A player on a rope is bitten by a jump.
        let up = if player.rope.is_some() {
            JUMP_BITE
        } else {
            1.2
        };
        let rise = feet.y - pos.y;
        if !player.dead && flat < BITE_RANGE && rise < up && rise > -1.2 && dog.bite_in <= 0.0 {
            dog.bite_in = BITE_EVERY;
            player.hurt(BITE_DAMAGE * traits.strength);
            sound.play(Sfx::Bite, Some(pos), 1.0);
        }
        let mut moving = false;
        if speed > 0.0 && want != Vec3::ZERO && flat > BITE_RANGE * 0.8 {
            match step_toward(&level, pos, want, speed * dt, dog.bias) {
                Some((next, d)) => {
                    tf.translation = next;
                    tf.rotation = Quat::from_rotation_y((-d.x).atan2(-d.z));
                    moving = true;
                    if let Mode::Wander { heading, .. } = &mut dog.mode {
                        *heading = (-d.x).atan2(-d.z);
                    }
                }
                None => match &mut dog.mode {
                    Mode::Wander { heading, .. } => *heading += std::f32::consts::PI * 0.75,
                    Mode::Chase { last_seen, .. } => {
                        // Blocked, even on a path: search (again) from here.
                        let target = dog.path.back().copied().unwrap_or(*last_seen);
                        dog.path.clear();
                        if dog.search.is_none() {
                            dog.search = Some(target);
                        }
                    }
                },
            }
        }
        // Never stuck: wanting to move but staying in place too long puts the dog elsewhere.
        if speed > 0.0 && want != Vec3::ZERO {
            if tf.translation.distance(dog.idle_from) > IDLE_MOVE {
                dog.idle_from = tf.translation;
                dog.idle_t = 0.0;
            } else {
                dog.idle_t += dt;
            }
        } else {
            dog.idle_from = tf.translation;
            dog.idle_t = 0.0;
        }
        if (relocate || dog.idle_t > IDLE_MAX)
            && let Some((p, _)) = street_spot(&level, eye, &mut rng.0, RELOCATE_DIST)
        {
            tf.translation = p;
            dog.idle_from = p;
            dog.idle_t = 0.0;
            dog.fails = 0;
            dog.path.clear();
            dog.search = None;
            dog.crumb = None;
            dog.check_t = 0.0;
            dog.check_dist = f32::MAX;
            moving = false;
        }
        if !moving && flat < 3.0 {
            // Face the player when close.
            let to = feet - pos;
            tf.rotation = Quat::from_rotation_y((-to.x).atan2(-to.z));
        }
        // Legs swing in a trot, faster when running; a paw pat every stride (longer when running).
        if moving {
            dog.gait += dt * speed * 4.0;
            dog.walked += speed * dt;
            let (stride, volume) = if speed > wander_speed {
                (PAW_STRIDE_RUN, 0.5)
            } else {
                (PAW_STRIDE_WALK, 0.25)
            };
            if dog.walked >= stride {
                dog.walked -= stride;
                sound.play(Sfx::Paw, Some(tf.translation), volume);
            }
        }
        for c in children.iter() {
            if let Ok((leg, mut ltf)) = legs.get_mut(c) {
                let phase = if leg.front == leg.left {
                    0.0
                } else {
                    std::f32::consts::PI
                };
                let swing = if moving {
                    (dog.gait + phase).sin() * 0.6
                } else {
                    0.0
                };
                ltf.rotation = Quat::from_rotation_x(swing);
            }
        }
    }
}

/// Falls on its side, lies, sinks, removed.
fn dog_death(
    time: Res<Time>,
    mut commands: Commands,
    mut dead: Query<(Entity, &mut Killed, &mut Transform), With<Dog>>,
) {
    for (e, mut k, mut tf) in &mut dead {
        if k.from.is_none() {
            k.from = Some(tf.rotation);
        }
        k.t += time.delta_secs();
        if k.t > 4.0 {
            commands.entity(e).despawn();
            continue;
        }
        let fall = (k.t / 0.4).min(1.0);
        tf.rotation = k.from.unwrap_or(tf.rotation)
            * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2 * fall);
        if k.t > 2.0 {
            tf.translation.y -= 0.2 * time.delta_secs();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Crumbs are found by id after old ones are dropped from the front.
    #[test]
    fn trail_lookup_by_id() {
        let mut t = Trail::default();
        for i in 0..5u64 {
            t.crumbs
                .push_back((i + 10, Vec3::X * i as f32, i as f32 * 10.0));
        }
        t.crumbs.pop_front();
        assert_eq!(t.get(10, 0.0, SCENT), None);
        assert_eq!(t.get(11, 0.0, SCENT), Some(Vec3::X));
        assert_eq!(t.get(14, 0.0, SCENT), Some(Vec3::X * 4.0));
        assert_eq!(t.get(15, 0.0, SCENT), None);
        // Scent gone: crumb 11 was left at 10 s.
        assert_eq!(t.get(11, 10.0 + SCENT, SCENT), None);
        assert_eq!(t.get(14, 10.0 + SCENT, SCENT), Some(Vec3::X * 4.0));
    }

    /// The path ends at the target, and each waypoint is one walkable step from the one before.
    fn assert_walkable(boxes: &[Aabb], from: Vec3, to: Vec3) {
        let reach = Vec3::new(SEARCH_RADIUS, SEARCH_HEIGHT, SEARCH_RADIUS);
        let path = path_over(boxes, from, to, reach, SEARCH_NODES).expect("a path");
        let end = *path.last().unwrap();
        assert!(
            Vec2::new(end.x - to.x, end.z - to.z).length() < 1.0 && (end.y - to.y).abs() < 1.0,
            "path ends at {end:?}, target {to:?}"
        );
        let mut p = from;
        for &w in &path {
            let d = w - p;
            let flat = Vec3::new(d.x, 0.0, d.z);
            let n = step_in(boxes, p, flat.normalize(), flat.length()).expect("walkable step");
            assert!((n - w).length() < 0.05, "step to {w:?} lands at {n:?}");
            p = w;
        }
    }

    /// From the street in front of a 3-story building, in through its door, up the stairwell to the roof.
    #[test]
    fn path_up_to_a_roof() {
        let boxes = crate::level::test_building(3);
        let roof = 3.0 * crate::level::FH;
        assert_walkable(
            &boxes,
            Vec3::new(-6.0, 0.0, 8.0),
            Vec3::new(12.0, roof, 10.0),
        );
    }

    /// In random pits (all sizes, depths, stair layouts and floor features) a dog finds its way from the
    /// street to every artifact (to the floor or the platform under it).
    #[test]
    fn paths_to_artifacts_in_random_pits() {
        for seed in 0..60u64 {
            let (boxes, start, artifacts) = crate::level::test_random_pit(seed * 7 + 1);
            assert!(!artifacts.is_empty());
            for a in artifacts {
                let t = a - Vec3::Y * 1.1;
                assert_walkable(&boxes, start, t);
            }
        }
    }

    /// From the street down the stairs to the floor of a 20 m pit.
    #[test]
    fn path_down_into_a_pit() {
        let (boxes, (x0, z0, x1, z1)) = crate::level::test_pit(20.0);
        let floor = Vec3::new((x0 + x1) / 2.0, -20.0, (z0 + z1) / 2.0);
        assert_walkable(&boxes, Vec3::new(x0 + 2.0, 0.0, z0 - 2.0), floor);
    }
}

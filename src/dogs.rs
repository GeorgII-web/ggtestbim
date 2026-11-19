//! Dogs: stray dogs in packs (`PACK` dogs each, up to `MAX_PACKS` packs) roam the streets around the player;
//! a pack is spawned out of sight on a street and removed when far behind. None come during the first
//! `GRACE` seconds after a (re)spawn or new world.
//! A pack wanders together (each dog steers toward its pack's center and keeps a little apart). When one dog
//! sees the player (line of sight within `Exposure`), hears a noise the player made (`Noises`: steps,
//! landings, shots) or a stranger calls (`DogCall`), the whole pack barks and gives chase.
//! Chasing: a dog that sees the player runs straight at it; otherwise it follows the player's trail (the
//! `Trail` of breadcrumbs the player left), from the nearest crumb it can see toward the newest, and so
//! takes the way the player came. Without a sight or sound for `LOSE` seconds it gives up.
//! A chasing dog that makes no progress for `STUCK_CHECK` seconds backs off along its own recent path and
//! then tries again by the trail with a turning bias to the other side.
//! Next to the player it bites (`BITE_DAMAGE` every `BITE_EVERY` s). Dogs walk on the level boxes: they
//! follow the ground, climb steps up to `STEP`, steer around walls and drop down ledges up to `DROP`.
//! Their eyes glow red. A bullet kills one: it falls on its side, lies and sinks away.

use std::collections::{HashMap, VecDeque};

use bevy::prelude::*;

use crate::flashlight::Exposure;
use crate::level::{Level, PITCH, Rng};
use crate::player::Player;
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
const WANDER_SPEED: f32 = 2.2;
const CHASE_SPEED: f32 = 9.5;
const BACK_SPEED: f32 = 5.0;
/// Without seeing or hearing the player this long, a chasing dog gives up.
const LOSE: f32 = 15.0;
/// A stranger's call reaches dogs within this distance of it.
const CALL_RANGE: f32 = 90.0;
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
/// Stuck: less than `PROGRESS` m closer to the target in `STUCK_CHECK` s; back off for up to `BACK_TIME` s
/// to where it was `BACK_STEPS` history points ago, then follow the trail with a turn bias for `BIAS_TIME`.
const STUCK_CHECK: f32 = 1.5;
const PROGRESS: f32 = 0.7;
const BACK_TIME: f32 = 3.0;
const BACK_STEPS: usize = 6;
const BIAS_TIME: f32 = 5.0;
/// A history point every this many meters the dog moves, at most `HISTORY` kept.
const HISTORY_STEP: f32 = 1.0;
const HISTORY: usize = 24;

#[derive(Resource)]
struct DogRng(Rng);

/// Breadcrumbs of the player's path on the ground, oldest first; each has a running id.
#[derive(Resource, Default)]
struct Trail {
    crumbs: VecDeque<(u64, Vec3)>,
    next: u64,
}

impl Trail {
    fn get(&self, id: u64) -> Option<Vec3> {
        let first = self.crumbs.front()?.0;
        self.crumbs
            .get(id.checked_sub(first)? as usize)
            .map(|c| c.1)
    }
}

#[derive(Resource)]
struct DogAssets {
    cube: Handle<Mesh>,
    fur: Handle<StandardMaterial>,
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
    /// Backing off after getting stuck, then back to the chase.
    Back {
        to: Vec3,
        t: f32,
        last_seen: Vec3,
        lost: f32,
    },
}

#[derive(Component)]
struct Dog {
    pack: u32,
    mode: Mode,
    bite_in: f32,
    sight_in: f32,
    sees: bool,
    /// Gait phase for the legs.
    gait: f32,
    /// Meters since the last paw sound and the last history point.
    walked: f32,
    moved: f32,
    /// Its own recent positions, newest last (where to back off to).
    history: VecDeque<Vec3>,
    /// The trail crumb it heads for, when following the trail.
    crumb: Option<u64>,
    /// Stuck check: time since the last check and the distance to the target then.
    check_t: f32,
    check_dist: f32,
    /// Turn bias (+1 left first, -1 right first) and how long it lasts; while it lasts the dog keeps to
    /// the trail even when it sees the player.
    bias: f32,
    bias_t: f32,
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
        fur: materials.add(StandardMaterial {
            base_color: Color::srgb(0.22, 0.16, 0.11),
            perceptual_roughness: 1.0,
            ..default()
        }),
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
fn record_trail(player: Single<&Player>, mut trail: ResMut<Trail>) {
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
        trail.crumbs.push_back((id, feet));
        if trail.crumbs.len() > CRUMBS {
            trail.crumbs.pop_front();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_dogs(
    mut commands: Commands,
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
    for _ in 0..8 {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let dist = rng.range(SPAWN_DIST.0, SPAWN_DIST.1);
        let mut p = eye + Vec3::new(angle.cos(), 0.0, angle.sin()) * dist;
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
        if ground(&level, p, 0.5).is_none() {
            continue;
        }
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
                spawn_dog(&mut commands, &assets, q, heading, pack);
            }
        }
        return;
    }
}

fn spawn_dog(commands: &mut Commands, assets: &DogAssets, at: Vec3, heading: f32, pack: u32) {
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
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(heading)),
            Visibility::default(),
            Dog {
                pack,
                mode: Mode::Wander {
                    heading,
                    turn_in: 3.0,
                },
                bite_in: 0.0,
                sight_in: 0.0,
                sees: false,
                gait: 0.0,
                walked: 0.0,
                moved: 0.0,
                history: VecDeque::new(),
                crumb: None,
                check_t: 0.0,
                check_dist: f32::MAX,
                bias: 1.0,
                bias_t: 0.0,
            },
            Shootable {
                kind: Kind::Dog,
                half: 0.4,
                height: 0.9,
            },
            Blip(Kind::Dog),
        ))
        .with_children(|c| {
            c.spawn(part(
                Vec3::new(0.0, 0.55, 0.0),
                Vec3::new(0.3, 0.3, 0.8),
                &assets.fur,
            ));
            c.spawn(part(
                Vec3::new(0.0, 0.78, -0.42),
                Vec3::new(0.22, 0.24, 0.26),
                &assets.fur,
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
                MeshMaterial3d(assets.fur.clone()),
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
    for turn in [0.0f32, 35.0, -35.0, 70.0, -70.0, 110.0, -110.0] {
        let d = Quat::from_rotation_y((turn * bias).to_radians()) * dir;
        let look = pos + d * (step + 0.35);
        // Body height clear ahead (above a step), and ground there not too high or too deep.
        if level.blocked(pos + Vec3::Y * (STEP + 0.1), look + Vec3::Y * (STEP + 0.1)) {
            continue;
        }
        let next = pos + d * step;
        let Some(g) = ground(level, next, STEP + 0.05) else {
            continue;
        };
        if g - pos.y > STEP || pos.y - g > DROP {
            continue;
        }
        return Some((Vec3::new(next.x, g, next.z), d));
    }
    None
}

/// The nearest trail crumb the dog can see (within `PICKUP`), the newest among equals.
fn pick_crumb(level: &Level, trail: &Trail, pos: Vec3) -> Option<u64> {
    let mut near: Vec<(f32, u64, Vec3)> = trail
        .crumbs
        .iter()
        .map(|&(id, p)| (p.distance(pos), id, p))
        .filter(|c| c.0 < PICKUP)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.1.cmp(&a.1)));
    near.iter()
        .take(10)
        .find(|c| !level.blocked(pos + Vec3::Y * EYE, c.2 + Vec3::Y * EYE))
        .map(|c| c.1)
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
    let (player, camera) = &mut *player;
    let eye = camera.translation;
    let feet = player.feet();
    let called = call.0.take();

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
            .filter(|(_, from)| from.distance(pos) < CALL_RANGE)
            .map(|c| c.0);
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

    for (mut dog, mut tf, children) in &mut dogs {
        let dog = &mut *dog;
        let pos = tf.translation;
        dog.bias_t -= dt;

        // Alerts: start or refresh the chase.
        if let Some(&(target, _)) = pack_alert.get(&dog.pack) {
            let target = if dog.sees { feet } else { target };
            match &mut dog.mode {
                Mode::Chase {
                    last_seen, lost, ..
                }
                | Mode::Back {
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
                (want, WANDER_SPEED)
            }
            Mode::Back {
                to,
                t,
                last_seen,
                lost,
            } => {
                *t += dt;
                let d = Vec3::new(to.x - pos.x, 0.0, to.z - pos.z);
                if d.length() < 0.8 || *t > BACK_TIME {
                    let (last_seen, lost) = (*last_seen, *lost);
                    dog.mode = Mode::Chase {
                        last_seen,
                        lost,
                        bark_in: 1.0,
                    };
                    dog.check_t = 0.0;
                    dog.check_dist = f32::MAX;
                    (Vec3::ZERO, 0.0)
                } else {
                    (d.normalize(), BACK_SPEED)
                }
            }
            Mode::Chase {
                last_seen,
                lost,
                bark_in,
            } => {
                if !dog.sees {
                    *lost += dt;
                }
                *bark_in -= dt;
                if *bark_in <= 0.0 {
                    *bark_in = rng.0.range(1.5, 3.5);
                    sound.play(Sfx::Bark, Some(pos), 1.0);
                }
                // Where to run: straight at a visible player; else along the player's trail toward the
                // newest crumb; else to where the player was last seen or heard.
                let target = if dog.sees && dog.bias_t <= 0.0 {
                    dog.crumb = None;
                    feet
                } else {
                    if dog.crumb.and_then(|id| trail.get(id)).is_none() {
                        dog.crumb = pick_crumb(&level, &trail, pos);
                    }
                    match dog.crumb.and_then(|id| trail.get(id).map(|p| (id, p))) {
                        Some((id, p)) => {
                            if p.distance(pos) < 1.2 {
                                dog.crumb = trail.get(id + 1).map(|_| id + 1);
                            }
                            dog.crumb.and_then(|id| trail.get(id)).unwrap_or(*last_seen)
                        }
                        None => *last_seen,
                    }
                };
                let to = Vec3::new(target.x - pos.x, 0.0, target.z - pos.z);
                // Stuck check: not getting closer.
                dog.check_t += dt;
                if dog.check_t > STUCK_CHECK {
                    let d = to.length();
                    let stuck = d > BITE_RANGE * 2.0 && dog.check_dist - d < PROGRESS;
                    dog.check_t = 0.0;
                    dog.check_dist = d;
                    if stuck {
                        let back = dog
                            .history
                            .len()
                            .checked_sub(BACK_STEPS)
                            .and_then(|i| dog.history.get(i))
                            .or(dog.history.front())
                            .copied()
                            .unwrap_or(pos);
                        let (last_seen, lost) = (*last_seen, *lost);
                        dog.mode = Mode::Back {
                            to: back,
                            t: 0.0,
                            last_seen,
                            lost,
                        };
                        dog.bias = -dog.bias;
                        dog.bias_t = BIAS_TIME;
                        dog.crumb = None;
                        (Vec3::ZERO, 0.0)
                    } else if *lost > LOSE {
                        let heading = rng.0.range(0.0, std::f32::consts::TAU);
                        dog.mode = Mode::Wander {
                            heading,
                            turn_in: 3.0,
                        };
                        (Vec3::ZERO, 0.0)
                    } else if to.length() < 0.8 {
                        (Vec3::ZERO, 0.0)
                    } else {
                        (to.normalize(), CHASE_SPEED)
                    }
                } else if to.length() < 0.8 {
                    (Vec3::ZERO, 0.0)
                } else {
                    (to.normalize(), CHASE_SPEED)
                }
            }
        };

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
        if !player.dead && flat < BITE_RANGE && (feet.y - pos.y).abs() < 1.2 && dog.bite_in <= 0.0 {
            dog.bite_in = BITE_EVERY;
            player.hurt(BITE_DAMAGE);
            sound.play(Sfx::Bite, Some(pos), 1.0);
        }
        let mut moving = false;
        if speed > 0.0 && want != Vec3::ZERO && flat > BITE_RANGE * 0.8 {
            match step_toward(&level, pos, want, speed * dt, dog.bias) {
                Some((next, d)) => {
                    tf.translation = next;
                    tf.rotation = Quat::from_rotation_y((-d.x).atan2(-d.z));
                    moving = true;
                    let step = next.distance(pos);
                    dog.moved += step;
                    if dog.moved >= HISTORY_STEP {
                        dog.moved = 0.0;
                        dog.history.push_back(next);
                        if dog.history.len() > HISTORY {
                            dog.history.pop_front();
                        }
                    }
                    if let Mode::Wander { heading, .. } = &mut dog.mode {
                        *heading = (-d.x).atan2(-d.z);
                    }
                }
                None => {
                    if let Mode::Wander { heading, .. } = &mut dog.mode {
                        *heading += std::f32::consts::PI * 0.75;
                    }
                }
            }
        } else if flat < 3.0 {
            // Face the player when close.
            let to = feet - pos;
            tf.rotation = Quat::from_rotation_y((-to.x).atan2(-to.z));
        }
        // Legs swing in a trot, faster when running; a paw pat every stride (longer when running).
        if moving {
            dog.gait += dt * speed * 4.0;
            dog.walked += speed * dt;
            let (stride, volume) = if speed > WANDER_SPEED {
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
            t.crumbs.push_back((i + 10, Vec3::X * i as f32));
        }
        t.crumbs.pop_front();
        assert_eq!(t.get(10), None);
        assert_eq!(t.get(11), Some(Vec3::X));
        assert_eq!(t.get(14), Some(Vec3::X * 4.0));
        assert_eq!(t.get(15), None);
    }
}

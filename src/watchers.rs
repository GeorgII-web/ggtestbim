//! Strangers (watchers): tall dark figures with a pale face and glowing eyes. A director keeps a few of them
//! around the player, appearing at any time: each starts hidden at the end of one of its spot's escapes
//! (the cover, chosen out of the player's line of sight), creeps out to the spot (to a window, around a
//! corner, to a roof edge) and watches the player. Seen for a moment, approached within `FLEE`, or after
//! watching long enough, it goes back the way it came (or along another escape that ends out of sight) and
//! is gone once something blocks the view. If its cover turns out to be in view, it ducks down out of sight
//! instead of standing there. Closer than `CATCH` it ducks at once (never caught).
//! While a stranger sees the player (in its line of sight within `Exposure`), it calls the dogs near it.
//! A bullet kills it (`Killed`): it falls backwards, its eyes go out, it sinks into the ground.

use bevy::prelude::*;

use crate::flashlight::Exposure;
use crate::level::{Level, Spot, SpotKind};
use crate::player::Player;
use crate::targets::{Blip, Killed, Kind, Shootable};

pub struct WatchersPlugin;

impl Plugin for WatchersPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Spots(Vec::new()))
            .insert_resource(DogCall(None))
            .add_systems(Startup, setup_watchers)
            .add_systems(Update, (collect_spots, direct, watch, die).chain());
    }
}

/// Strangers around the player at most, and how often the director tries to bring a new one.
const MAX_ACTIVE: usize = 5;
const SPAWN_EVERY: (f32, f32) = (1.5, 4.0);
/// Distance range from the player for a new stranger's spot.
const SPAWN_DIST: (f32, f32) = (18.0, 80.0);
/// Spot kind weights: mostly windows and corners, roofs now and then.
const WEIGHTS: [(SpotKind, f32); 3] = [
    (SpotKind::Window, 0.5),
    (SpotKind::Corner, 0.4),
    (SpotKind::Roof, 0.1),
];
/// Closer than this it goes back into cover; closer than `CATCH` it ducks at once.
const FLEE: f32 = 16.0;
const CATCH: f32 = 3.0;
/// Seen this long (random in the range) and it goes back.
const REACT: (f32, f32) = (0.4, 1.2);
/// Watches at most this long (random) before going back on its own.
const LINGER: (f32, f32) = (5.0, 14.0);
/// Creeping out, going back.
const CREEP_SPEED: f32 = 1.6;
const RUN_SPEED: f32 = 8.0;
/// In cover but still in view this long: it ducks.
const COVER_GRACE: f32 = 0.4;
const DUCK_TIME: f32 = 0.35;
/// Beyond this distance it is too small to count as seen by the player.
const SEE_DIST: f32 = 120.0;
/// Head height above the feet, the target of lines of sight.
const HEAD: f32 = 2.3;
/// Death: falling over takes `FALL`, then it lies until `LIE`, then sinks into the ground until `SINK`.
const FALL: f32 = 0.6;
const LIE: f32 = 2.0;
const SINK: f32 = 4.0;

/// Spots of the loaded chunks.
#[derive(Resource)]
struct Spots(Vec<Spot>);

/// The latest call for dogs: where the player was seen, and from where (dogs near the caller come).
#[derive(Resource)]
pub struct DogCall(pub Option<(Vec3, Vec3)>);

#[derive(Component)]
struct Eye;

enum State {
    /// Moving along `path` (toward the spot when creeping out, toward cover when going back).
    Move {
        path: Vec<Vec3>,
        next: usize,
        speed: f32,
        out: bool,
    },
    /// At the spot, watching; `seen` counts how long the player has seen it.
    Watch { seen: f32 },
    /// In cover: gone as soon as the view is blocked, ducks if not.
    Cover,
    /// Ducking (shrinking down), then gone.
    Duck,
}

#[derive(Component)]
struct Watcher {
    spot: Spot,
    /// The escape it came out of (and goes back along unless another one hides it better).
    escape: Vec<Vec3>,
    state: State,
    clock: f32,
    react: f32,
    linger: f32,
}

#[derive(Resource)]
struct WatcherAssets {
    cube: Handle<Mesh>,
    body: Handle<StandardMaterial>,
    /// Pale, faintly glowing head: it shows in the dark rooms where the black body doesn't.
    face: Handle<StandardMaterial>,
    eyes: Handle<StandardMaterial>,
}

fn setup_watchers(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(WatcherAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        body: materials.add(StandardMaterial {
            base_color: Color::srgb(0.03, 0.03, 0.04),
            perceptual_roughness: 1.0,
            ..default()
        }),
        face: materials.add(StandardMaterial {
            base_color: Color::srgb(0.78, 0.76, 0.72),
            emissive: LinearRgba::rgb(0.25, 0.24, 0.22),
            perceptual_roughness: 1.0,
            ..default()
        }),
        eyes: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.6),
            emissive: LinearRgba::rgb(30.0, 20.0, 6.0),
            unlit: true,
            ..default()
        }),
    });
}

/// New chunks' spots join the pool, unloaded ones leave it; a new world (N) clears everything.
fn collect_spots(
    mut commands: Commands,
    mut level: ResMut<Level>,
    mut spots: ResMut<Spots>,
    keys: Res<ButtonInput<KeyCode>>,
    old: Query<Entity, With<Watcher>>,
) {
    if keys.just_pressed(KeyCode::KeyN) {
        for e in &old {
            commands.entity(e).despawn();
        }
        spots.0.clear();
    }
    for chunk in std::mem::take(&mut level.new_spots) {
        spots.0.extend(chunk);
    }
    let level = &*level;
    spots.0.retain(|s| level.loaded(s.pos));
}

/// The player sees a point: on screen (well inside the view), near enough and nothing in between.
fn player_sees(level: &Level, eye: Vec3, forward: Vec3, p: Vec3) -> bool {
    let to = p - eye;
    to.length() < SEE_DIST && to.normalize().dot(forward) > 0.75 && !level.blocked(eye, p)
}

fn end(e: &[Vec3]) -> Vec3 {
    e[e.len() - 1]
}

/// The escape of the spot that ends out of the player's line of sight (the shortest such), if any.
fn hidden_escape(level: &Level, eye: Vec3, spot: &Spot) -> Option<Vec<Vec3>> {
    let length = |e: &Vec<Vec3>| {
        let mut from = spot.pos;
        e.iter()
            .map(|&p| {
                let d = p.distance(from);
                from = p;
                d
            })
            .sum::<f32>()
    };
    spot.escapes
        .iter()
        .filter(|e| level.blocked(eye, end(e) + Vec3::Y * HEAD))
        .min_by(|a, b| length(a).total_cmp(&length(b)))
        .cloned()
}

/// Every few seconds, with fewer than `MAX_ACTIVE` strangers, one appears hidden in the cover of a spot
/// at a fitting distance and creeps out to it.
#[allow(clippy::too_many_arguments)]
fn direct(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<WatcherAssets>,
    spots: Res<Spots>,
    camera: Single<&Transform, With<Player>>,
    active: Query<(), (With<Watcher>, Without<Killed>)>,
    mut level: ResMut<Level>,
    mut timer: Local<f32>,
) {
    *timer -= time.delta_secs();
    if *timer > 0.0 {
        return;
    }
    *timer = level.rng.range(SPAWN_EVERY.0, SPAWN_EVERY.1);
    if active.iter().count() >= MAX_ACTIVE || spots.0.is_empty() {
        return;
    }
    let eye = camera.translation;
    // A few random tries: a spot at the right distance whose cover the player can't see.
    for _ in 0..12 {
        let rng = &mut level.rng;
        let r = rng.unit() * WEIGHTS.iter().map(|w| w.1).sum::<f32>();
        let mut acc = 0.0;
        let kind = WEIGHTS
            .iter()
            .find(|w| {
                acc += w.1;
                r <= acc
            })
            .map_or(SpotKind::Window, |w| w.0);
        let spot = &spots.0[rng.below(spots.0.len() as u32) as usize];
        let dist = spot.pos.distance(eye);
        if spot.kind != kind || dist < SPAWN_DIST.0 || dist > SPAWN_DIST.1 {
            continue;
        }
        let Some(escape) = hidden_escape(&level, eye, spot) else {
            continue;
        };
        let react = level.rng.range(REACT.0, REACT.1);
        let linger = level.rng.range(LINGER.0, LINGER.1);
        // From the cover back through the waypoints to the spot.
        let mut path: Vec<Vec3> = escape.iter().rev().skip(1).copied().collect();
        path.push(spot.pos);
        let watcher = Watcher {
            spot: spot.clone(),
            escape: escape.clone(),
            state: State::Move {
                path,
                next: 0,
                speed: CREEP_SPEED,
                out: true,
            },
            clock: 0.0,
            react,
            linger,
        };
        spawn_watcher(&mut commands, &assets, end(&escape), watcher);
        return;
    }
}

fn spawn_watcher(commands: &mut Commands, assets: &WatcherAssets, at: Vec3, watcher: Watcher) {
    let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
        (
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(center).with_scale(size),
        )
    };
    // Modeled facing local -z: thin body, long neck, pale head, two eyes on the front.
    commands
        .spawn((
            Transform::from_translation(at),
            Visibility::default(),
            watcher,
            Shootable {
                kind: Kind::Stranger,
                half: 0.25,
                height: 2.6,
            },
            Blip(Kind::Stranger),
        ))
        .with_children(|c| {
            c.spawn(part(
                Vec3::new(0.0, 1.05, 0.0),
                Vec3::new(0.45, 2.1, 0.3),
                &assets.body,
            ));
            c.spawn(part(
                Vec3::new(0.0, 2.35, 0.0),
                Vec3::new(0.32, 0.4, 0.32),
                &assets.face,
            ));
            for x in [-0.08, 0.08] {
                c.spawn((
                    part(
                        Vec3::new(x, 2.4, -0.17),
                        Vec3::new(0.11, 0.08, 0.02),
                        &assets.eyes,
                    ),
                    Eye,
                ));
            }
        });
}

/// Back into cover: the escape that ends hidden now (its own if that still hides it), else its own.
fn go_back(w: &mut Watcher, level: &Level, eye: Vec3) {
    let path = if level.blocked(eye, end(&w.escape) + Vec3::Y * HEAD) {
        w.escape.clone()
    } else {
        hidden_escape(level, eye, &w.spot).unwrap_or_else(|| w.escape.clone())
    };
    w.escape = path.clone();
    w.state = State::Move {
        path,
        next: 0,
        speed: RUN_SPEED,
        out: false,
    };
}

#[allow(clippy::too_many_arguments)]
fn watch(
    time: Res<Time>,
    mut commands: Commands,
    camera: Single<(&Transform, &Player), Without<Watcher>>,
    mut watchers: Query<(Entity, &mut Watcher, &mut Transform), Without<Killed>>,
    level: Res<Level>,
    exposure: Res<Exposure>,
    mut call: ResMut<DogCall>,
) {
    let dt = time.delta_secs();
    let (camera, player) = *camera;
    let eye = camera.translation;
    let forward = camera.forward().as_vec3();
    for (e, mut w, mut tf) in &mut watchers {
        if !level.loaded(w.spot.pos) {
            commands.entity(e).despawn();
            continue;
        }
        let w = &mut *w;
        w.clock += dt;
        let head = tf.translation + Vec3::Y * HEAD;
        let dist = tf.translation.distance(eye);
        if dist < CATCH && !matches!(w.state, State::Duck) {
            w.state = State::Duck;
            w.clock = 0.0;
        }
        let mut facing = eye - tf.translation;
        match &mut w.state {
            State::Move {
                path,
                next,
                speed,
                out,
            } => {
                let is_out = *out;
                let to = path[*next] - tf.translation;
                let step = *speed * dt;
                let mut arrived = false;
                if to.length() <= step {
                    tf.translation = path[*next];
                    *next += 1;
                    arrived = *next == path.len();
                } else {
                    tf.translation += to.normalize() * step;
                    if !is_out {
                        facing = to;
                    }
                }
                if is_out && dist < FLEE {
                    // Too close while still creeping out: back at once.
                    go_back(w, &level, eye);
                } else if arrived {
                    w.state = if is_out {
                        State::Watch { seen: 0.0 }
                    } else {
                        State::Cover
                    };
                    w.clock = 0.0;
                }
            }
            State::Watch { seen } => {
                let visible = player_sees(&level, eye, forward, head);
                *seen = if visible { *seen + dt } else { 0.0 };
                // It sees the player: the dogs near it come.
                if dist < exposure.0 && !level.blocked(head, eye) {
                    call.0 = Some((player.feet(), tf.translation));
                }
                if *seen > w.react || dist < FLEE || w.clock > w.linger {
                    go_back(w, &level, eye);
                }
            }
            State::Cover => {
                if level.blocked(eye, head) {
                    commands.entity(e).despawn();
                    continue;
                }
                if w.clock > COVER_GRACE {
                    w.state = State::Duck;
                    w.clock = 0.0;
                }
            }
            State::Duck => {
                let k = 1.0 - (w.clock / DUCK_TIME).min(1.0);
                tf.scale = Vec3::new(1.0, k.max(0.01), 1.0);
                if w.clock > DUCK_TIME {
                    commands.entity(e).despawn();
                    continue;
                }
            }
        }
        if facing.x != 0.0 || facing.z != 0.0 {
            tf.rotation = Quat::from_rotation_y((-facing.x).atan2(-facing.z));
        }
    }
}

/// Falls backwards (about its feet, away from where it faced), eyes out, lies, sinks, removed.
fn die(
    time: Res<Time>,
    mut commands: Commands,
    mut dead: Query<(Entity, &mut Killed, &mut Transform, &Children), With<Watcher>>,
    mut eyes: Query<&mut Visibility, With<Eye>>,
) {
    for (e, mut k, mut tf, children) in &mut dead {
        if k.from.is_none() {
            k.from = Some(tf.rotation);
            for c in children.iter() {
                if let Ok(mut v) = eyes.get_mut(c) {
                    *v = Visibility::Hidden;
                }
            }
        }
        k.t += time.delta_secs();
        if k.t > SINK {
            commands.entity(e).despawn();
            continue;
        }
        // Local -z is its front: tipping about local x by +90 degrees lays it on its back.
        let fall = (k.t / FALL).min(1.0);
        let angle = std::f32::consts::FRAC_PI_2 * fall * fall;
        tf.rotation = k.from.unwrap_or(tf.rotation) * Quat::from_rotation_x(angle);
        if k.t > LIE {
            tf.translation.y -= 0.25 * time.delta_secs();
        }
    }
}

//! Watchers: tall dark figures with a pale face and glowing eyes, standing behind windows, at building
//! corners or (rarely) on roofs, turned toward the player. When the player really sees one (on screen and
//! in line of sight) for a moment, or comes within `FLEE`, it runs along its spot's escape (deeper into the
//! room, around the corner, back from the roof edge) and hides there. Once the player can't see it, it is
//! gone: it turns up again at another spot of its chunk, far from the player and out of sight.
//! It can't be caught: closer than `CATCH` it vanishes on the spot. While unseen, it may wander.

use bevy::prelude::*;

use crate::level::{Level, Rng, Spot, SpotKind};
use crate::player::Player;

pub struct WatchersPlugin;

impl Plugin for WatchersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_watchers)
            .add_systems(Update, (spawn_watchers, watch).chain());
    }
}

/// Chance of each of the (up to) `PER_CHUNK` watchers of a new chunk with spots.
const CHANCE: f32 = 0.9;
const PER_CHUNK: usize = 4;
/// Spot kind weights for placing a watcher: mostly windows and corners, roofs now and then.
const WEIGHTS: [(SpotKind, f32); 3] = [
    (SpotKind::Window, 0.5),
    (SpotKind::Corner, 0.4),
    (SpotKind::Roof, 0.1),
];
/// Closer than this and the watcher runs; a new spot must be at least `FAR` from the player.
const FLEE: f32 = 20.0;
const FAR: f32 = 30.0;
/// Closer than this it vanishes on the spot, even in plain view.
const CATCH: f32 = 5.0;
/// Seen this long (random in the range) and it runs.
const REACT: (f32, f32) = (0.3, 0.9);
const RUN_SPEED: f32 = 6.0;
/// Hidden at the end of its escape, it stays at least this long before it can be gone.
const HIDE_MIN: f32 = 0.6;
/// Beyond this distance it is too small to count as seen.
const SEE_DIST: f32 = 120.0;
/// While unseen, every `WANDER_EVERY` seconds a watcher may move to another spot of its chunk.
const WANDER_EVERY: f32 = 6.0;
/// Head height above the feet, the target of the line of sight.
const HEAD: f32 = 2.3;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);

enum State {
    /// Standing at its spot, watching the player; `seen` counts how long the player has seen it.
    Watch { seen: f32 },
    /// Running along the escape waypoints, `next` is the one it heads for.
    Run { path: Vec<Vec3>, next: usize },
    /// At the end of its escape, waiting to be out of sight.
    Hide,
    /// Nowhere (at `HIDDEN`), waiting for a free spot.
    Gone,
}

#[derive(Component)]
struct Watcher {
    spots: Vec<Spot>,
    /// Index of the spot it stands at (or ran from).
    spot: usize,
    state: State,
    clock: f32,
    react: f32,
}

#[derive(Resource)]
struct WatcherAssets {
    cube: Handle<Mesh>,
    body: Handle<StandardMaterial>,
    /// Pale, faintly glowing head: it shows in the dark rooms (15 % ambient) where the black body doesn't.
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

/// A random spot index by the kind weights, among the indices `ok` accepts (kinds with none are skipped).
fn pick(rng: &mut Rng, spots: &[Spot], ok: impl Fn(usize) -> bool) -> Option<usize> {
    let kinds: Vec<(SpotKind, f32)> = WEIGHTS
        .iter()
        .copied()
        .filter(|(k, _)| (0..spots.len()).any(|i| spots[i].kind == *k && ok(i)))
        .collect();
    let total: f32 = kinds.iter().map(|(_, w)| w).sum();
    let mut r = rng.unit() * total;
    let kind = kinds
        .iter()
        .find(|(_, w)| {
            r -= w;
            r <= 0.0
        })
        .or(kinds.last())?
        .0;
    let of_kind: Vec<usize> = (0..spots.len())
        .filter(|&i| spots[i].kind == kind && ok(i))
        .collect();
    Some(of_kind[rng.below(of_kind.len() as u32) as usize])
}

fn spawn_watchers(
    mut commands: Commands,
    assets: Res<WatcherAssets>,
    mut level: ResMut<Level>,
    keys: Res<ButtonInput<KeyCode>>,
    old: Query<Entity, With<Watcher>>,
) {
    // A new world (N) brings new buildings: the old watchers leave with them.
    if keys.just_pressed(KeyCode::KeyN) {
        for e in &old {
            commands.entity(e).despawn();
        }
    }
    let all = std::mem::take(&mut level.new_spots);
    for spots in all {
        let mut taken: Vec<usize> = Vec::new();
        for _ in 0..PER_CHUNK {
            if !level.rng.chance(CHANCE) {
                continue;
            }
            // Different spots, or two figures would overlap.
            let Some(i) = pick(&mut level.rng, &spots, |i| {
                !taken.iter().any(|&t| spots[t].pos == spots[i].pos)
            }) else {
                break;
            };
            taken.push(i);
            let react = level.rng.range(REACT.0, REACT.1);
            spawn_watcher(&mut commands, &assets, spots.clone(), i, react);
        }
    }
}

fn spawn_watcher(
    commands: &mut Commands,
    assets: &WatcherAssets,
    spots: Vec<Spot>,
    spot: usize,
    react: f32,
) {
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
            Transform::from_translation(spots[spot].pos),
            Visibility::default(),
            Watcher {
                spots,
                spot,
                state: State::Watch { seen: 0.0 },
                clock: 0.0,
                react,
            },
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
                c.spawn(part(
                    Vec3::new(x, 2.4, -0.17),
                    Vec3::new(0.11, 0.08, 0.02),
                    &assets.eyes,
                ));
            }
        });
}

/// The player sees a point: on screen (well inside the view), near enough and nothing in between.
fn sees(level: &Level, eye: Vec3, forward: Vec3, p: Vec3) -> bool {
    let to = p - eye;
    to.length() < SEE_DIST && to.normalize().dot(forward) > 0.75 && !level.blocked(eye, p)
}

/// Puts the watcher at a spot of its chunk far from the player and out of sight, or nowhere if none is free.
fn relocate(w: &mut Watcher, level: &mut Level, tf: &mut Transform, eye: Vec3, forward: Vec3) {
    let free: Vec<bool> = w
        .spots
        .iter()
        .map(|s| s.pos.distance(eye) > FAR && !sees(level, eye, forward, s.pos + Vec3::Y * HEAD))
        .collect();
    let found = pick(&mut level.rng, &w.spots, |i| free[i]);
    w.clock = 0.0;
    match found {
        Some(i) => {
            w.spot = i;
            w.state = State::Watch { seen: 0.0 };
            w.react = level.rng.range(REACT.0, REACT.1);
            tf.translation = w.spots[i].pos;
        }
        None => {
            w.state = State::Gone;
            tf.translation = HIDDEN;
        }
    }
}

fn watch(
    time: Res<Time>,
    mut commands: Commands,
    camera: Single<&Transform, (With<Player>, Without<Watcher>)>,
    mut watchers: Query<(Entity, &mut Watcher, &mut Transform)>,
    mut level: ResMut<Level>,
) {
    let dt = time.delta_secs();
    let eye = camera.translation;
    let forward = camera.forward().as_vec3();
    for (e, mut w, mut tf) in &mut watchers {
        // Its spots are gone with their chunk.
        if !level.loaded(w.spots[0].pos) {
            commands.entity(e).despawn();
            continue;
        }
        let w = &mut *w;
        w.clock += dt;
        let dist = tf.translation.distance(eye);
        if !matches!(w.state, State::Gone) && dist < CATCH {
            w.state = State::Gone;
            w.clock = 0.0;
            tf.translation = HIDDEN;
            continue;
        }
        let seen = !matches!(w.state, State::Gone)
            && sees(&level, eye, forward, tf.translation + Vec3::Y * HEAD);
        // Faces the player, or the way it runs (the model looks along local -z).
        let mut facing = eye - tf.translation;
        match &mut w.state {
            State::Watch { seen: s } => {
                *s = if seen { *s + dt } else { 0.0 };
                if *s > w.react || dist < FLEE {
                    w.state = State::Run {
                        path: w.spots[w.spot].escape.clone(),
                        next: 0,
                    };
                } else if !seen && w.clock > WANDER_EVERY && level.rng.chance(0.3) {
                    relocate(w, &mut level, &mut tf, eye, forward);
                }
            }
            State::Run { path, next } => {
                let to = path[*next] - tf.translation;
                let step = RUN_SPEED * dt;
                if to.length() <= step {
                    tf.translation = path[*next];
                    *next += 1;
                    if *next == path.len() {
                        w.state = State::Hide;
                        w.clock = 0.0;
                    }
                } else {
                    tf.translation += to.normalize() * step;
                    facing = to;
                }
            }
            State::Hide => {
                if !seen && w.clock > HIDE_MIN {
                    relocate(w, &mut level, &mut tf, eye, forward);
                }
            }
            State::Gone => {
                if w.clock > WANDER_EVERY {
                    relocate(w, &mut level, &mut tf, eye, forward);
                }
            }
        }
        if facing.x != 0.0 || facing.z != 0.0 {
            tf.rotation = Quat::from_rotation_y((-facing.x).atan2(-facing.z));
        }
    }
}

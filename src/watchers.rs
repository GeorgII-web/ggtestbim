//! Watchers: rare tall dark figures with glowing eyes standing behind upper-floor windows, always turned
//! toward the player. They can't be found: come within `FLEE` and the watcher is gone, standing at a
//! window far away (or nowhere); while the player isn't looking it may move to another window.

use bevy::prelude::*;

use crate::level::Level;
use crate::player::Player;

pub struct WatchersPlugin;

impl Plugin for WatchersPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_watchers)
            .add_systems(Update, (spawn_watchers, watch).chain());
    }
}

/// Share of new chunks with window spots that get a watcher.
const CHANCE: f32 = 0.35;
/// Closer than this and the watcher moves away; a new spot must be at least `FAR` from the player.
const FLEE: f32 = 16.0;
const FAR: f32 = 30.0;
/// While unseen, every `WANDER_EVERY` seconds a watcher may move to another window of its chunk.
const WANDER_EVERY: f32 = 6.0;
const HIDDEN: Vec3 = Vec3::new(0.0, -500.0, 0.0);

#[derive(Component)]
struct Watcher {
    spots: Vec<Vec3>,
    clock: f32,
}

#[derive(Resource)]
struct WatcherAssets {
    cube: Handle<Mesh>,
    body: Handle<StandardMaterial>,
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
        eyes: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.6),
            emissive: LinearRgba::rgb(30.0, 20.0, 6.0),
            unlit: true,
            ..default()
        }),
    });
}

fn spawn_watchers(
    mut commands: Commands,
    assets: Res<WatcherAssets>,
    mut level: ResMut<Level>,
    keys: Res<ButtonInput<KeyCode>>,
    old: Query<Entity, With<Watcher>>,
) {
    // A new world (R) brings new buildings: the old watchers leave with them.
    if keys.just_pressed(KeyCode::KeyR) {
        for e in &old {
            commands.entity(e).despawn();
        }
    }
    let all = std::mem::take(&mut level.new_spots);
    for spots in all {
        if spots.is_empty() || !level.rng.chance(CHANCE) {
            continue;
        }
        let at = spots[level.rng.below(spots.len() as u32) as usize];
        let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
            (
                Mesh3d(assets.cube.clone()),
                MeshMaterial3d(mat.clone()),
                Transform::from_translation(center).with_scale(size),
            )
        };
        // Modeled facing local -z: thin body, long neck, head, two eyes on the front.
        commands
            .spawn((
                Transform::from_translation(at),
                Visibility::default(),
                Watcher { spots, clock: 0.0 },
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
                    &assets.body,
                ));
                for x in [-0.08, 0.08] {
                    c.spawn(part(
                        Vec3::new(x, 2.4, -0.17),
                        Vec3::new(0.1, 0.07, 0.02),
                        &assets.eyes,
                    ));
                }
            });
    }
}

fn watch(
    time: Res<Time>,
    mut commands: Commands,
    camera: Single<&Transform, (With<Player>, Without<Watcher>)>,
    mut watchers: Query<(Entity, &mut Watcher, &mut Transform)>,
    mut level: ResMut<Level>,
) {
    let eye = camera.translation;
    let forward = camera.forward().as_vec3();
    for (e, mut w, mut tf) in &mut watchers {
        // Its windows are gone with their chunk.
        if !level.loaded(w.spots[0]) {
            commands.entity(e).despawn();
            continue;
        }
        w.clock += time.delta_secs();
        let to = eye - tf.translation;
        let dist = to.length();
        let seen = tf.translation != HIDDEN && (-to).normalize().dot(forward) > 0.8;
        let flee = dist < FLEE;
        if flee || (!seen && w.clock > WANDER_EVERY && level.rng.chance(0.3)) {
            w.clock = 0.0;
            let far: Vec<Vec3> = w
                .spots
                .iter()
                .copied()
                .filter(|s| s.distance(eye) > FAR)
                .collect();
            tf.translation = if far.is_empty() {
                HIDDEN
            } else {
                far[level.rng.below(far.len() as u32) as usize]
            };
        } else if tf.translation == HIDDEN && w.clock > WANDER_EVERY {
            w.clock = 0.0;
            if let Some(&s) = w.spots.iter().find(|s| s.distance(eye) > FAR) {
                tf.translation = s;
            }
        }
        let to = eye - tf.translation;
        tf.rotation = Quat::from_rotation_y((-to.x).atan2(-to.z));
    }
}

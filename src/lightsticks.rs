//! Light sticks: 4 throws a glowing stick along the view (a little upward); it flies under gravity, bounces
//! off walls, comes to rest on the first floor it hits and lights the area around it in green for
//! `LIFE` seconds (fading over the last `FADE`). At most `MAX` lie around; the oldest goes first.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::level::Level;
use crate::player::Player;
use crate::sound::{Sfx, SoundQueue};

pub struct LightSticksPlugin;

impl Plugin for LightSticksPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_sticks)
            .add_systems(Update, (throw, fly).chain().after(crate::player::PlayerSet));
    }
}

const SPEED: f32 = 13.0;
const UP: f32 = 3.0;
const GRAVITY: f32 = 18.0;
const LIFE: f32 = 300.0;
const FADE: f32 = 10.0;
const MAX: usize = 10;
const LUMENS: f32 = 120_000.0;
const RANGE: f32 = 18.0;
const LENGTH: f32 = 0.16;

#[derive(Resource)]
struct StickAssets {
    mesh: Handle<Mesh>,
    glow: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Stick {
    vel: Vec3,
    resting: bool,
    age: f32,
}

fn setup_sticks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(StickAssets {
        mesh: meshes.add(Cuboid::new(0.025, 0.025, LENGTH)),
        glow: materials.add(StandardMaterial {
            base_color: Color::srgb(0.5, 1.0, 0.5),
            emissive: LinearRgba::rgb(4.0, 30.0, 6.0),
            unlit: true,
            ..default()
        }),
    });
}

fn throw(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    assets: Res<StickAssets>,
    camera: Single<(&Transform, &Player)>,
    sticks: Query<(Entity, &Stick)>,
    mut sound: ResMut<SoundQueue>,
) {
    let (camera, player) = *camera;
    if !keys.just_pressed(KeyCode::Digit4) || player.dead {
        return;
    }
    // Over the limit: the oldest goes.
    let mut all: Vec<(f32, Entity)> = sticks.iter().map(|(e, s)| (s.age, e)).collect();
    if all.len() >= MAX {
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        commands.entity(all[0].1).despawn();
    }
    let dir = camera.forward().as_vec3();
    let from = camera.translation + dir * 0.4 - Vec3::Y * 0.2;
    sound.play(Sfx::Switch, None, 0.4);
    commands.spawn((
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.glow.clone()),
        Transform::from_translation(from).looking_to(dir, Vec3::Y),
        NotShadowCaster,
        Stick {
            vel: dir * SPEED + Vec3::Y * UP,
            resting: false,
            age: 0.0,
        },
        children![(
            PointLight {
                color: Color::srgb(0.45, 1.0, 0.5),
                intensity: LUMENS,
                range: RANGE,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(0.0, 0.15, 0.0),
        )],
    ));
}

/// Flight with bounces until a floor stops it, then the glow fades out at the end of its life.
fn fly(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<Level>,
    mut sticks: Query<(Entity, &mut Stick, &mut Transform, &Children)>,
    mut lights: Query<&mut PointLight>,
) {
    let dt = time.delta_secs();
    for (e, mut s, mut tf, children) in &mut sticks {
        s.age += dt;
        if s.age > LIFE || !level.loaded(tf.translation) {
            commands.entity(e).despawn();
            continue;
        }
        let fade = ((LIFE - s.age) / FADE).min(1.0);
        for c in children.iter() {
            if let Ok(mut l) = lights.get_mut(c) {
                l.intensity = LUMENS * fade;
            }
        }
        if s.resting {
            continue;
        }
        s.vel.y -= GRAVITY * dt;
        let step = s.vel * dt;
        let len = step.length();
        if len < 1e-5 {
            continue;
        }
        let dir = step / len;
        match level.raycast(tf.translation, dir, len + 0.02) {
            Some((t, n)) => {
                tf.translation += dir * (t - 0.02).max(0.0);
                if n.y > 0.5 {
                    // On a floor: lie flat along its last direction.
                    s.resting = true;
                    tf.translation += n * 0.013;
                    let flat = Vec3::new(s.vel.x, 0.0, s.vel.z);
                    let along = if flat.length() > 0.01 { flat } else { Vec3::Z };
                    tf.look_to(along, Vec3::Y);
                } else {
                    // Off a wall or ceiling: reflect, losing most of the speed.
                    s.vel = (s.vel - 2.0 * s.vel.dot(n) * n) * 0.35;
                }
            }
            None => {
                tf.translation += step;
                // Tumbles in flight.
                tf.rotate_local_x(dt * 12.0);
            }
        }
    }
}

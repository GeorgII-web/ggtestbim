//! Anomaly artifacts at the bottom of the pits: glowing crystals that float, turn and pulse, each with a
//! violet light (on only within `LIGHT_RANGE` of the player). Walking into one (within `PICK`) takes it;
//! the count is shown under the clock. Removed with their chunk and on a new world (N).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::level::Level;
use crate::player::{Player, PlayerSet};
use crate::sound::{Sfx, SoundQueue};

pub struct ArtifactsPlugin;

impl Plugin for ArtifactsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Found(0))
            .add_systems(Startup, setup_artifacts)
            .add_systems(
                Update,
                (cleanup, spawn_artifacts, float_and_take)
                    .chain()
                    .after(PlayerSet),
            );
    }
}

const PICK: f32 = 1.4;
const LIGHT_RANGE: f32 = 45.0;
const LUMENS: f32 = 60_000.0;

/// Artifacts taken so far.
#[derive(Resource)]
struct Found(u32);

#[derive(Component)]
struct Artifact {
    base: Vec3,
    phase: f32,
}

#[derive(Component)]
struct FoundText;

#[derive(Resource)]
struct ArtifactAssets {
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

fn setup_artifacts(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(ArtifactAssets {
        // An octahedron: a sphere with no subdivision.
        mesh: meshes.add(Sphere::new(0.25).mesh().ico(0).expect("ico sphere")),
        mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.75, 0.5, 1.0),
            emissive: LinearRgba::rgb(6.0, 2.0, 14.0),
            unlit: true,
            ..default()
        }),
    });
    commands.spawn((
        Text::new("artifacts 0"),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        TextColor(Color::srgb(0.8, 0.65, 1.0)),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(56.0),
            right: Val::Px(8.0),
            ..default()
        },
        FoundText,
    ));
}

fn cleanup(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<Level>,
    artifacts: Query<(Entity, &Artifact)>,
) {
    for (e, a) in &artifacts {
        if keys.just_pressed(KeyCode::KeyN) || !level.loaded(a.base) {
            commands.entity(e).despawn();
        }
    }
}

fn spawn_artifacts(mut commands: Commands, mut level: ResMut<Level>, assets: Res<ArtifactAssets>) {
    for base in std::mem::take(&mut level.new_artifacts) {
        let phase = (base.x * 1.7 + base.z * 0.9).rem_euclid(std::f32::consts::TAU);
        commands.spawn((
            Mesh3d(assets.mesh.clone()),
            MeshMaterial3d(assets.mat.clone()),
            Transform::from_translation(base),
            NotShadowCaster,
            Artifact { base, phase },
            children![(
                PointLight {
                    color: Color::srgb(0.7, 0.45, 1.0),
                    intensity: LUMENS,
                    range: 12.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Visibility::Hidden,
            )],
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn float_and_take(
    mut commands: Commands,
    time: Res<Time>,
    player: Single<&Player>,
    mut artifacts: Query<(Entity, &Artifact, &mut Transform, &Children)>,
    mut lights: Query<(&mut PointLight, &mut Visibility)>,
    mut found: ResMut<Found>,
    mut text: Single<&mut Text, With<FoundText>>,
    mut sound: ResMut<SoundQueue>,
) {
    let t = time.elapsed_secs();
    let body = player.feet() + Vec3::Y * 0.9;
    for (e, a, mut tf, children) in &mut artifacts {
        if !player.dead && a.base.distance(body) < PICK {
            commands.entity(e).despawn();
            found.0 += 1;
            text.0 = format!("artifacts {}", found.0);
            sound.play(Sfx::Switch, None, 0.8);
            continue;
        }
        tf.translation = a.base + Vec3::Y * (0.15 * (t * 1.8 + a.phase).sin());
        tf.rotation = Quat::from_rotation_y(t * 1.2 + a.phase) * Quat::from_rotation_x(0.4);
        let near = a.base.distance(body) < LIGHT_RANGE;
        for c in children.iter() {
            if let Ok((mut l, mut vis)) = lights.get_mut(c) {
                l.intensity = LUMENS * (0.75 + 0.25 * (t * 3.1 + a.phase).sin());
                *vis = if near {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
            }
        }
    }
}

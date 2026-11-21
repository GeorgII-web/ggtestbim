//! Rope: E throws a rope at the surface in the view (up to `RANGE`); it is tied there and, at the other
//! end, hangs in the air above the player's head where it was thrown from, and the player takes hold of it
//! at once (the player module moves along it). E again lets go. The rope disappears as soon as the player
//! is off it (let go, climbed off at its end, died, new world).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::level::Level;
use crate::player::{HANG, Player, PlayerSet, Ride};
use crate::sound::{Sfx, SoundQueue};

pub struct RopesPlugin;

impl Plugin for RopesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_ropes)
            .add_systems(Update, use_rope.before(PlayerSet))
            .add_systems(Update, remove_used.after(PlayerSet));
    }
}

const RANGE: f32 = 60.0;
/// Shortest rope worth throwing.
const MIN_LENGTH: f32 = 3.0;

#[derive(Component)]
struct Rope;

#[derive(Resource)]
struct RopeAssets {
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

fn setup_ropes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(RopeAssets {
        mesh: meshes.add(Cuboid::new(0.04, 0.04, 1.0)),
        mat: materials.add(StandardMaterial {
            base_color: Color::srgb(0.55, 0.42, 0.25),
            perceptual_roughness: 1.0,
            ..default()
        }),
    });
}

/// The rope goes once the player is off it.
fn remove_used(mut commands: Commands, player: Single<&Player>, ropes: Query<Entity, With<Rope>>) {
    if player.rope.is_none() {
        for e in &ropes {
            commands.entity(e).despawn();
        }
    }
}

fn use_rope(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<Level>,
    assets: Res<RopeAssets>,
    mut camera: Single<(&Transform, &mut Player)>,
    mut sound: ResMut<SoundQueue>,
) {
    let (camera, player) = &mut *camera;
    if !keys.just_pressed(KeyCode::KeyE) || player.dead {
        return;
    }
    if player.rope.take().is_some() {
        return;
    }
    let hands = player.feet() + Vec3::Y * HANG;
    // Throw: tied where the view meets a surface, if the line from the hands to there is clear.
    let eye = camera.translation;
    let dir = camera.forward().as_vec3();
    let tied = level.raycast(eye, dir, RANGE).and_then(|(d, n)| {
        let b = eye + dir * d;
        let along = (b - hands).normalize_or_zero();
        (hands.distance(b) >= MIN_LENGTH && !level.blocked(hands, b - along * 0.15))
            .then_some((b, n))
    });
    let Some((b, nb)) = tied else {
        sound.play(Sfx::DryFire, None, 0.5);
        return;
    };
    let a = hands;
    let up = if (b - a).normalize().y.abs() > 0.99 {
        Vec3::X
    } else {
        Vec3::Y
    };
    commands.spawn((
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.mat.clone()),
        Transform::from_translation((a + b) / 2.0)
            .looking_to(b - a, up)
            .with_scale(Vec3::new(1.0, 1.0, a.distance(b))),
        NotShadowCaster,
        Rope,
    ));
    player.rope = Some(Ride { a, b, t: 0.0, nb });
    sound.play(Sfx::Switch, None, 0.6);
}

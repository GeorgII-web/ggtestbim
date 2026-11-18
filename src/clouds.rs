//! Clouds: random clumps of flattened spheres high above the city, drifting with the wind and casting
//! moving shadows. A cloud that drifts or is left too far from the player comes back on the opposite
//! side with a new random shape.

use bevy::prelude::*;

use crate::level::{Level, Rng};
use crate::player::Player;

pub struct CloudsPlugin;

impl Plugin for CloudsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_clouds)
            .add_systems(Update, drift);
    }
}

const COUNT: usize = 45;
/// Horizontal distance from the player within which clouds live.
const RANGE: f32 = 320.0;
const WIND: Vec3 = Vec3::new(3.0, 0.0, 1.2);

#[derive(Component)]
struct Cloud;

#[derive(Resource)]
struct CloudAssets {
    sphere: Handle<Mesh>,
    white: Handle<StandardMaterial>,
}

fn setup_clouds(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut level: ResMut<Level>,
) {
    let assets = CloudAssets {
        sphere: meshes.add(Sphere::new(1.0).mesh().ico(2).unwrap()),
        white: materials.add(StandardMaterial {
            base_color: Color::srgb(0.97, 0.97, 1.0),
            unlit: true,
            fog_enabled: false,
            ..default()
        }),
    };
    for _ in 0..COUNT {
        let rng = &mut level.rng;
        let at = Vec3::new(
            rng.range(-RANGE, RANGE),
            rng.range(90.0, 140.0),
            rng.range(-RANGE, RANGE),
        );
        let root = commands
            .spawn((
                Transform::from_translation(at),
                Visibility::default(),
                Cloud,
            ))
            .id();
        shape(&mut commands, &assets, root, rng);
    }
    commands.insert_resource(assets);
}

/// 4 to 9 flattened spheres around the cloud center.
fn shape(commands: &mut Commands, a: &CloudAssets, root: Entity, rng: &mut Rng) {
    commands.entity(root).despawn_related::<Children>();
    let size = rng.range(0.6, 1.6);
    for _ in 0..(4 + rng.below(6)) {
        let r = rng.range(6.0, 14.0) * size;
        let offset = Vec3::new(
            rng.range(-14.0, 14.0),
            rng.range(-2.0, 3.0),
            rng.range(-9.0, 9.0),
        ) * size;
        let puff = commands
            .spawn((
                Mesh3d(a.sphere.clone()),
                MeshMaterial3d(a.white.clone()),
                Transform::from_translation(offset).with_scale(Vec3::new(r, r * 0.45, r * 0.8)),
            ))
            .id();
        commands.entity(root).add_child(puff);
    }
}

fn drift(
    time: Res<Time>,
    mut commands: Commands,
    assets: Res<CloudAssets>,
    player: Single<&Transform, (With<Player>, Without<Cloud>)>,
    mut clouds: Query<(Entity, &mut Transform), With<Cloud>>,
    mut level: ResMut<Level>,
) {
    let center = Vec3::new(player.translation.x, 0.0, player.translation.z);
    for (e, mut tf) in &mut clouds {
        tf.translation += WIND * time.delta_secs();
        let rel = Vec3::new(tf.translation.x, 0.0, tf.translation.z) - center;
        if rel.length() > RANGE + 40.0 {
            let back = -rel.normalize() * RANGE * 0.95;
            let rng = &mut level.rng;
            let side = Vec3::new(-back.z, 0.0, back.x).normalize() * rng.range(-RANGE, RANGE) * 0.5;
            tf.translation = Vec3::new(
                center.x + back.x + side.x,
                rng.range(90.0, 140.0),
                center.z + back.z + side.z,
            );
            shape(&mut commands, &assets, e, rng);
        }
    }
}

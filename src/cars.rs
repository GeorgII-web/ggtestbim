//! Cars on the streets: a few per newly generated chunk, driving straight along a street lane through the
//! world, braking for the player in front of them, removed when they reach an unloaded chunk. Their boxes
//! are colliders for the player (bump into them, jump on the roof).

use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI};

use crate::level::{Aabb, CHUNK, CHUNK_SIZE, Level, PITCH};
use crate::player::Player;

pub struct CarsPlugin;

impl Plugin for CarsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CarBoxes(Vec::new()))
            .add_systems(Startup, setup_cars)
            .add_systems(Update, (spawn_cars, drive).chain());
    }
}

const PER_CHUNK: u32 = 3;
/// Lane center distance from the street center line; a car is 1.8 m wide, so it stays clear of the
/// terrace stairs that reach 1.6 m into the street from the block edge.
const LANE: f32 = 1.2;
const LENGTH: f32 = 4.2;
const WIDTH: f32 = 1.8;
const HEIGHT: f32 = 1.6;
const ACCEL: f32 = 5.0;
const BRAKE: f32 = 18.0;
/// Distance ahead in which the player makes a car stop.
const LOOK_AHEAD: f32 = 10.0;

#[derive(Component)]
pub(crate) struct Car {
    along_x: bool,
    dir: f32,
    speed: f32,
    cruise: f32,
}

/// World boxes of all cars, rebuilt every frame.
#[derive(Resource)]
pub struct CarBoxes(pub Vec<Aabb>);

#[derive(Resource)]
struct CarAssets {
    cube: Handle<Mesh>,
    paints: Vec<Handle<StandardMaterial>>,
    glass: Handle<StandardMaterial>,
    tire: Handle<StandardMaterial>,
}

fn setup_cars(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut paint = |r: f32, g: f32, b: f32, rough: f32| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            perceptual_roughness: rough,
            metallic: 0.3,
            ..default()
        })
    };
    let paints = [
        (0.75, 0.1, 0.1),
        (0.1, 0.25, 0.7),
        (0.9, 0.9, 0.9),
        (0.1, 0.1, 0.1),
        (0.95, 0.75, 0.1),
        (0.2, 0.55, 0.3),
    ]
    .iter()
    .map(|&(r, g, b)| paint(r, g, b, 0.35))
    .collect();
    let glass = paint(0.1, 0.15, 0.2, 0.1);
    let tire = paint(0.05, 0.05, 0.05, 0.9);
    commands.insert_resource(CarAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        paints,
        glass,
        tire,
    });
}

/// Puts cars on the streets of every newly generated chunk: a random street line of the chunk, a random
/// direction (the lane on that side), a random spot and cruise speed.
fn spawn_cars(mut commands: Commands, assets: Res<CarAssets>, mut level: ResMut<Level>) {
    let chunks = std::mem::take(&mut level.new_chunks);
    for c in chunks {
        let origin = c.as_vec2() * CHUNK_SIZE;
        for _ in 0..PER_CHUNK {
            let rng = &mut level.rng;
            let along_x = rng.chance(0.5);
            let dir = if rng.chance(0.5) { 1.0 } else { -1.0 };
            let line = rng.below(CHUNK as u32) as f32 * PITCH;
            let along = rng.range(0.0, CHUNK_SIZE);
            let pos = if along_x {
                Vec3::new(origin.x + along, 0.0, origin.y + line + dir * LANE)
            } else {
                Vec3::new(origin.x + line - dir * LANE, 0.0, origin.y + along)
            };
            let cruise = rng.range(6.0, 16.0);
            let paint = assets.paints[rng.below(assets.paints.len() as u32) as usize].clone();
            spawn_car(&mut commands, &assets, paint, pos, along_x, dir, cruise);
        }
    }
}

/// A car is modeled along its local +x: body, cabin, four wheels.
fn spawn_car(
    commands: &mut Commands,
    a: &CarAssets,
    paint: Handle<StandardMaterial>,
    pos: Vec3,
    along_x: bool,
    dir: f32,
    cruise: f32,
) {
    let yaw = match (along_x, dir > 0.0) {
        (true, true) => 0.0,
        (true, false) => PI,
        (false, true) => -FRAC_PI_2,
        (false, false) => FRAC_PI_2,
    };
    let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
        (
            Mesh3d(a.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(center).with_scale(size),
        )
    };
    commands
        .spawn((
            Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::default(),
            Car {
                along_x,
                dir,
                speed: cruise,
                cruise,
            },
        ))
        .with_children(|c| {
            c.spawn(part(
                Vec3::new(0.0, 0.7, 0.0),
                Vec3::new(LENGTH, 0.7, WIDTH),
                &paint,
            ));
            c.spawn(part(
                Vec3::new(-0.3, 1.3, 0.0),
                Vec3::new(2.2, 0.6, WIDTH - 0.2),
                &a.glass,
            ));
            for (x, z) in [(1.3, 0.8), (1.3, -0.8), (-1.3, 0.8), (-1.3, -0.8)] {
                c.spawn(part(
                    Vec3::new(x, 0.35, z),
                    Vec3::new(0.7, 0.7, 0.3),
                    &a.tire,
                ));
            }
        });
}

pub(crate) fn drive(
    time: Res<Time>,
    mut commands: Commands,
    mut cars: Query<(Entity, &mut Car, &mut Transform), Without<Player>>,
    player: Single<&Transform, With<Player>>,
    level: Res<Level>,
    mut boxes: ResMut<CarBoxes>,
) {
    let dt = time.delta_secs().min(1.0 / 30.0);
    boxes.0.clear();
    for (e, mut car, mut tf) in &mut cars {
        if !level.loaded(tf.translation) {
            commands.entity(e).despawn();
            continue;
        }
        let (axis, side) = if car.along_x { (0, 2) } else { (2, 0) };
        let rel = player.translation - tf.translation;
        let ahead = rel[axis] * car.dir;
        let blocked = (0.0..LOOK_AHEAD + LENGTH / 2.0).contains(&ahead)
            && rel[side].abs() < WIDTH
            && rel.y < 3.5;
        let target = if blocked { 0.0 } else { car.cruise };
        let rate = if target < car.speed { BRAKE } else { ACCEL };
        car.speed += (target - car.speed).clamp(-rate * dt, rate * dt);
        let step = car.speed * car.dir * dt;
        tf.translation[axis] += step;

        let mut half = Vec3::new(WIDTH / 2.0, 0.0, WIDTH / 2.0);
        half[axis] = LENGTH / 2.0;
        boxes.0.push(Aabb {
            min: tf.translation - Vec3::new(half.x, 0.0, half.z),
            max: tf.translation + Vec3::new(half.x, HEIGHT, half.z),
        });
    }
}

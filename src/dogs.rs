//! Dogs: a few stray dogs roam the streets around the player (up to `MAX_DOGS`, spawned out of sight on a
//! street, removed when far behind). A dog wanders; when it sees the player (line of sight within the
//! player's `Exposure`) or a stranger calls it (`DogCall`, within `CALL_RANGE` of the caller), it barks and
//! runs to where the player was last seen, keeps following while it sees the player and gives up after
//! `LOSE` seconds without a sight. Next to the player it bites (`BITE_DAMAGE` every `BITE_EVERY` s).
//! Dogs walk on the level boxes: they follow the ground, climb steps up to `STEP`, steer around walls
//! by trying turns left and right, and drop down ledges up to `DROP`. A bullet kills one: it falls on its
//! side, lies and sinks away.

use bevy::prelude::*;

use crate::flashlight::Exposure;
use crate::level::{Level, PITCH, Rng};
use crate::player::Player;
use crate::sound::{Sfx, SoundQueue};
use crate::targets::{Blip, Killed, Kind, Shootable};
use crate::watchers::DogCall;

pub struct DogsPlugin;

impl Plugin for DogsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DogRng(Rng(0xD065)))
            .add_systems(Startup, setup_dogs)
            .add_systems(Update, (spawn_dogs, run_dogs, dog_death).chain());
    }
}

const MAX_DOGS: usize = 5;
const SPAWN_EVERY: f32 = 3.0;
const SPAWN_DIST: (f32, f32) = (40.0, 75.0);
const DESPAWN_DIST: f32 = 110.0;
const WANDER_SPEED: f32 = 2.2;
const CHASE_SPEED: f32 = 9.5;
/// Without seeing the player this long, a chasing dog gives up.
const LOSE: f32 = 7.0;
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

#[derive(Resource)]
struct DogRng(Rng);

#[derive(Resource)]
struct DogAssets {
    cube: Handle<Mesh>,
    fur: Handle<StandardMaterial>,
    dark: Handle<StandardMaterial>,
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

#[derive(Component)]
struct Dog {
    mode: Mode,
    bite_in: f32,
    sight_in: f32,
    sees: bool,
    /// Gait phase for the legs.
    gait: f32,
    /// Meters since the last paw sound.
    walked: f32,
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
    });
}

/// Ground height under `p` (the top of the highest box below `p + up`), if any within `DROP + up`.
fn ground(level: &Level, p: Vec3, up: f32) -> Option<f32> {
    level
        .raycast(p + Vec3::Y * up, Vec3::NEG_Y, up + DROP)
        .map(|(t, _)| p.y + up - t)
}

#[allow(clippy::too_many_arguments)]
fn spawn_dogs(
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<DogAssets>,
    level: Res<Level>,
    keys: Res<ButtonInput<KeyCode>>,
    camera: Single<&Transform, With<Player>>,
    dogs: Query<(Entity, &Transform), With<Dog>>,
    mut rng: ResMut<DogRng>,
    mut timer: Local<f32>,
) {
    let eye = camera.translation;
    // New world or far away: gone.
    for (e, tf) in &dogs {
        if keys.just_pressed(KeyCode::KeyN)
            || tf.translation.distance(eye) > DESPAWN_DIST
            || !level.loaded(tf.translation)
        {
            commands.entity(e).despawn();
        }
    }
    *timer -= time.delta_secs();
    if *timer > 0.0 || dogs.iter().count() >= MAX_DOGS {
        return;
    }
    *timer = SPAWN_EVERY;
    let rng = &mut rng.0;
    for _ in 0..8 {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let dist = rng.range(SPAWN_DIST.0, SPAWN_DIST.1);
        let mut p = eye + Vec3::new(angle.cos(), 0.0, angle.sin()) * dist;
        // Onto the middle of the nearest street (streets run along x and z at multiples of PITCH).
        let (sx, sz) = ((p.x / PITCH).round() * PITCH, (p.z / PITCH).round() * PITCH);
        if (p.x - sx).abs() < (p.z - sz).abs() {
            p.x = sx;
        } else {
            p.z = sz;
        }
        p.y = 0.0;
        if !level.loaded(p) || !level.blocked(eye, p + Vec3::Y * EYE) {
            continue;
        }
        let Some(g) = ground(&level, p, 0.5) else {
            continue;
        };
        p.y = g;
        spawn_dog(
            &mut commands,
            &assets,
            p,
            rng.range(0.0, std::f32::consts::TAU),
        );
        return;
    }
}

fn spawn_dog(commands: &mut Commands, assets: &DogAssets, at: Vec3, heading: f32) {
    let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
        (
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(center).with_scale(size),
        )
    };
    // Modeled facing local -z: body, neck and head, ears, snout, tail, four legs.
    commands
        .spawn((
            Transform::from_translation(at).with_rotation(Quat::from_rotation_y(heading)),
            Visibility::default(),
            Dog {
                mode: Mode::Wander {
                    heading,
                    turn_in: 3.0,
                },
                bite_in: 0.0,
                sight_in: 0.0,
                sees: false,
                gait: 0.0,
                walked: 0.0,
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

/// Tries to move `step` meters along `dir`, turning up to 110 degrees either way around obstacles; the
/// new position (on the ground) and the direction taken, or None when every way is blocked.
fn step_toward(level: &Level, pos: Vec3, dir: Vec3, step: f32) -> Option<(Vec3, Vec3)> {
    for turn in [0.0f32, 35.0, -35.0, 70.0, -70.0, 110.0, -110.0] {
        let d = Quat::from_rotation_y(turn.to_radians()) * dir;
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

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn run_dogs(
    time: Res<Time>,
    level: Res<Level>,
    exposure: Res<Exposure>,
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
    for (mut dog, mut tf, children) in &mut dogs {
        let dog = &mut *dog;
        let pos = tf.translation;
        // Sight is checked a few times a second.
        dog.sight_in -= dt;
        if dog.sight_in <= 0.0 {
            dog.sight_in = SIGHT_EVERY;
            dog.sees = pos.distance(eye) < exposure.0 && !level.blocked(pos + Vec3::Y * EYE, eye);
        }
        let called_here = called.filter(|(_, from)| from.distance(pos) < CALL_RANGE);
        if dog.sees || called_here.is_some() {
            let target = if dog.sees {
                feet
            } else {
                called_here.map_or(feet, |c| c.0)
            };
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
                (Vec3::new(-heading.sin(), 0.0, -heading.cos()), WANDER_SPEED)
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
                let to = Vec3::new(last_seen.x - pos.x, 0.0, last_seen.z - pos.z);
                let lost_it = *lost > LOSE;
                if lost_it {
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
            }
        };
        // Bite when next to the player.
        dog.bite_in -= dt;
        let flat = Vec2::new(feet.x - pos.x, feet.z - pos.z).length();
        if flat < BITE_RANGE && (feet.y - pos.y).abs() < 1.2 && dog.bite_in <= 0.0 {
            dog.bite_in = BITE_EVERY;
            player.hurt(BITE_DAMAGE);
            sound.play(Sfx::Bite, Some(pos), 1.0);
        }
        let mut moving = false;
        if speed > 0.0 && flat > BITE_RANGE * 0.8 {
            match step_toward(&level, pos, want, speed * dt) {
                Some((next, d)) => {
                    tf.translation = next;
                    tf.rotation = Quat::from_rotation_y((-d.x).atan2(-d.z));
                    moving = true;
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

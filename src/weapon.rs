//! Pistol: a first-person model at the bottom right of the view (a child of the camera), a crosshair in the
//! middle of the screen (its gap is the next shot's spread cone) and the ammo at the bottom right. Left click (with the mouse grabbed) fires one shot
//! along the view: a short muzzle flash with a flash of light, recoil, and at the hit a bullet mark that
//! stays `MARK_LIFE` seconds plus a burst of sparks. Hits on cars and props only spark (they move or change).
//! Each shot leaves the center at a random angle inside a cone: its radius grows with movement (still,
//! walk, run, in the air) and with each quick follow-up shot (the streak cools down between shots).
//! Shots hit strangers and dogs too (`Shootable`): a hit one is `Killed` (its own module plays its
//! death) and dark red bits spray instead of sparks.
//! Empty, the slide locks back and a click only twitches the pistol (dry fire).
//! `MAGAZINE` rounds; R reloads with an animation (the pistol cants, the magazine drops out, a new one
//! slides in, the slide racks), an empty magazine only clicks.
//! While the optic is up the pistol and crosshair are hidden (the optic has its own reticle); shots still
//! go to the center.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::cars::CarBoxes;
use crate::level::{Level, PropBoxes, Rng, ray_box};
use crate::optic::Optic;
use crate::player::{Motion, Player};
use crate::sound::{HEAR_SHOT, Noises, Sfx, SoundQueue};
use crate::targets::{Killed, Kind, Shootable};

pub struct WeaponPlugin;

impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Gun {
            ammo: MAGAZINE,
            cooldown: 0.0,
            recoil: 0.0,
            reload: None,
            flash: 0.0,
            streak: 0.0,
            started_empty: false,
            rng: Rng(0x5EED_F1E1),
        })
        .add_systems(PostStartup, setup_weapon)
        .add_systems(
            Update,
            (fire, animate, sparks, marks, ui)
                .chain()
                .after(crate::player::PlayerSet),
        );
    }
}

const MAGAZINE: u32 = 12;
/// Shortest time between two shots, and the time a reload takes.
const FIRE_GAP: f32 = 0.12;
const RELOAD_TIME: f32 = 1.3;
/// Spread: cone radius in degrees by movement, times a factor for the shot's place in a quick series
/// (1st, 2nd, 3rd, later). The series count drops by one every `STREAK_COOL` seconds without a shot.
const SPREAD_STILL: f32 = 0.3;
const SPREAD_WALK: f32 = 1.2;
const SPREAD_RUN: f32 = 3.5;
const SPREAD_AIR: f32 = 5.0;
const STREAK_FACTOR: [f32; 4] = [1.0, 1.6, 2.3, 3.0];
const STREAK_COOL: f32 = 0.35;
/// Longest shot.
const RANGE: f32 = 150.0;
/// Bullet marks last this long (the last `MARK_FADE` seconds they shrink away); at most `MAX_MARKS` exist.
const MARK_LIFE: f32 = 60.0;
const MARK_FADE: f32 = 3.0;
const MAX_MARKS: usize = 300;
const MARK_SIZE: f32 = 0.07;
/// Sparks per hit and their life.
const SPARKS: usize = 9;
const SPARK_LIFE: f32 = 0.35;
/// Muzzle flash time.
const FLASH: f32 = 0.05;
/// Pistol rest position in camera space (x right, y up, -z forward) and its muzzle, local to the pistol.
const REST: Vec3 = Vec3::new(0.17, -0.16, -0.33);
const MUZZLE: Vec3 = Vec3::new(0.0, 0.03, -0.11);

#[derive(Resource)]
struct Gun {
    ammo: u32,
    cooldown: f32,
    /// Recoil kick, 1 right after a shot, decaying to 0.
    recoil: f32,
    /// Seconds into a reload, while one runs.
    reload: Option<f32>,
    /// Seconds of muzzle flash left.
    flash: f32,
    /// The running reload began with an empty magazine (slide locked back until the release).
    started_empty: bool,
    /// Shots in the current quick series (0 = the next one is a first shot), cooling down over time.
    streak: f32,
    rng: Rng,
}

#[derive(Resource)]
struct WeaponAssets {
    cube: Handle<Mesh>,
    mark: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
    blood: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Pistol;

#[derive(Component)]
struct MuzzleFlash;

/// A part of the slide (it racks back during a reload), with its rest position.
#[derive(Component)]
struct Slide(Vec3);

/// The magazine in the grip, with its rest transform.
#[derive(Component)]
struct Magazine(Transform);

#[derive(Component)]
struct FlashLight;

#[derive(Component)]
struct Mark {
    age: f32,
    size: Vec3,
}

#[derive(Component)]
struct Spark {
    vel: Vec3,
    age: f32,
    size: f32,
}

#[derive(Component)]
struct Crosshair;

/// One piece of the crosshair.
#[derive(Component, Clone, Copy)]
enum Tick {
    Dot,
    Left,
    Right,
    Up,
    Down,
}

/// Crosshair tick length and thickness, and the smallest gap, in logical pixels.
const TICK_LEN: f32 = 9.0;
const TICK_W: f32 = 2.0;
const MIN_GAP: f32 = 3.0;

#[derive(Component)]
struct AmmoText;

fn setup_weapon(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    camera: Single<Entity, With<Player>>,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.13),
        metallic: 0.6,
        perceptual_roughness: 0.45,
        ..default()
    });
    let grip = materials.add(StandardMaterial {
        base_color: Color::srgb(0.07, 0.06, 0.05),
        perceptual_roughness: 0.9,
        ..default()
    });
    let flame = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.75, 0.3),
        emissive: LinearRgba::rgb(40.0, 18.0, 4.0),
        unlit: true,
        ..default()
    });
    commands.insert_resource(WeaponAssets {
        cube: cube.clone(),
        mark: materials.add(StandardMaterial {
            base_color: Color::srgb(0.04, 0.035, 0.03),
            perceptual_roughness: 1.0,
            ..default()
        }),
        spark: materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.4),
            emissive: LinearRgba::rgb(30.0, 14.0, 3.0),
            unlit: true,
            ..default()
        }),
        blood: materials.add(StandardMaterial {
            base_color: Color::srgb(0.35, 0.02, 0.02),
            unlit: true,
            ..default()
        }),
    });
    let mag_steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.2, 0.21),
        metallic: 0.5,
        perceptual_roughness: 0.5,
        ..default()
    });
    let part = |center: Vec3, size: Vec3, mat: &Handle<StandardMaterial>| {
        (
            Mesh3d(cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(center).with_scale(size),
            NotShadowCaster,
        )
    };
    // Slide on top, frame under it, grip tilted back, trigger guard, front sight; muzzle at -z.
    let pistol = commands
        .spawn((
            Transform::from_translation(REST),
            Visibility::default(),
            Pistol,
        ))
        .with_children(|c| {
            let slide = Vec3::new(0.0, 0.03, -0.02);
            c.spawn((
                part(slide, Vec3::new(0.032, 0.032, 0.18), &steel),
                Slide(slide),
            ));
            c.spawn(part(
                Vec3::new(0.0, 0.006, -0.03),
                Vec3::new(0.028, 0.02, 0.15),
                &steel,
            ));
            c.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(grip.clone()),
                Transform::from_xyz(0.0, -0.045, 0.045)
                    .with_rotation(Quat::from_rotation_x(0.3))
                    .with_scale(Vec3::new(0.03, 0.1, 0.045)),
                NotShadowCaster,
            ));
            c.spawn(part(
                Vec3::new(0.0, -0.012, -0.01),
                Vec3::new(0.006, 0.02, 0.04),
                &steel,
            ));
            let sight = Vec3::new(0.0, 0.05, -0.105);
            c.spawn((
                part(sight, Vec3::new(0.005, 0.008, 0.006), &steel),
                Slide(sight),
            ));
            // Magazine: in the grip, its base plate showing under it.
            let mag = Transform::from_xyz(0.0, -0.06, 0.05)
                .with_rotation(Quat::from_rotation_x(0.3))
                .with_scale(Vec3::new(0.026, 0.1, 0.038));
            c.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(mag_steel),
                mag,
                NotShadowCaster,
                Magazine(mag),
            ));
            c.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(flame),
                Transform::from_translation(MUZZLE + Vec3::new(0.0, 0.0, -0.03))
                    .with_scale(Vec3::new(0.05, 0.05, 0.08)),
                NotShadowCaster,
                Visibility::Hidden,
                MuzzleFlash,
            ));
            c.spawn((
                PointLight {
                    color: Color::srgb(1.0, 0.75, 0.4),
                    intensity: 0.0,
                    range: 8.0,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_translation(MUZZLE + Vec3::new(0.0, 0.0, -0.1)),
                FlashLight,
            ));
        })
        .id();
    commands.entity(*camera).add_child(pistol);

    let font = TextFont {
        font_size: FontSize::Px(22.0),
        ..default()
    };
    // Crosshair: a dot and four ticks, placed in pixels by `ui` at the spread radius of the next shot.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            Crosshair,
        ))
        .with_children(|c| {
            for tick in [Tick::Dot, Tick::Left, Tick::Right, Tick::Up, Tick::Down] {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                    tick,
                ));
            }
        });
    commands.spawn((
        Text::new(""),
        font,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(16.0),
            bottom: Val::Px(12.0),
            ..default()
        },
        AmmoText,
    ));
}

/// Spread cone radius (radians) of the next shot for this movement and series count, and the shot's place
/// in the series (0 = first). The shot and the crosshair both use it.
fn spread(motion: Motion, streak: f32) -> (f32, usize) {
    let base = match motion {
        Motion::Still => SPREAD_STILL,
        Motion::Walk => SPREAD_WALK,
        Motion::Run => SPREAD_RUN,
        Motion::Air => SPREAD_AIR,
    };
    let nth = (streak.ceil() as usize).min(STREAK_FACTOR.len() - 1);
    ((base * STREAK_FACTOR[nth]).to_radians(), nth)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn fire(
    mut commands: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    camera: Single<(&Transform, &Player)>,
    level: Res<Level>,
    cars: Res<CarBoxes>,
    props: Res<PropBoxes>,
    figures: Query<(Entity, &Transform, &Shootable), (Without<Killed>, Without<Player>)>,
    assets: Res<WeaponAssets>,
    mut gun: ResMut<Gun>,
    mut sound: ResMut<SoundQueue>,
    mut noises: ResMut<Noises>,
) {
    let dt = time.delta_secs();
    gun.cooldown -= dt;
    gun.streak = (gun.streak - dt / STREAK_COOL).max(0.0);
    if let Some(t) = gun.reload.as_mut() {
        *t += dt;
        if *t >= RELOAD_TIME {
            gun.reload = None;
            gun.ammo = MAGAZINE;
        }
    }
    // Dead: no reload, no shots.
    if camera.1.dead {
        return;
    }
    if keys.just_pressed(KeyCode::KeyR) && gun.reload.is_none() && gun.ammo < MAGAZINE {
        gun.reload = Some(0.0);
        gun.started_empty = gun.ammo == 0;
        sound.play(Sfx::Reload, None, 0.6);
    }
    // The click that grabs the mouse doesn't shoot.
    let grabbed = cursor.grab_mode != CursorGrabMode::None;
    if !(grabbed && mouse.just_pressed(MouseButton::Left))
        || gun.reload.is_some()
        || gun.cooldown > 0.0
    {
        return;
    }
    if gun.ammo == 0 {
        // Dry fire: the trigger clicks, the pistol twitches.
        gun.recoil = 0.3;
        gun.cooldown = FIRE_GAP;
        sound.play(Sfx::DryFire, None, 0.7);
        return;
    }
    sound.play(Sfx::Shot, None, 0.8);
    noises.make(camera.1.feet(), HEAR_SHOT);
    gun.ammo -= 1;
    gun.cooldown = FIRE_GAP;
    gun.recoil = 1.0;
    gun.flash = FLASH;

    let (camera, player) = *camera;
    let (radius, nth) = spread(player.motion(), gun.streak);
    gun.streak = (nth + 1) as f32;
    // Uniform over the cone's disk: angle off center ~ sqrt(u), any direction around it.
    let off = radius * gun.rng.unit().sqrt();
    let around = gun.rng.range(0.0, std::f32::consts::TAU);
    let o = camera.translation;
    let dir = (camera.forward().as_vec3()
        + (camera.right().as_vec3() * around.cos() + camera.up().as_vec3() * around.sin())
            * off.tan())
    .normalize();
    let world = level.raycast(o, dir, RANGE);
    let moving = cars
        .0
        .iter()
        .chain(props.0.iter())
        .filter_map(|bx| ray_box(o, dir, RANGE, bx))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    let (t, normal, mark) = match (world, moving) {
        (Some(w), Some(m)) if m.0 < w.0 => (m.0, m.1, false),
        (Some(w), _) => (w.0, w.1, true),
        (None, Some(m)) => (m.0, m.1, false),
        (None, None) => (RANGE, Vec3::ZERO, false),
    };
    // A figure in front of whatever else the shot hits is killed.
    let figure = figures
        .iter()
        .filter_map(|(e, tf, s)| {
            let bx = s.aabb(tf.translation);
            ray_box(o, dir, t, &bx).map(|h| (e, s.kind, h))
        })
        .min_by(|a, b| a.2.0.total_cmp(&b.2.0));
    if let Some((e, kind, (ft, fnormal))) = figure {
        commands.entity(e).insert(Killed::default());
        let hit = o + dir * ft;
        sound.play(Sfx::Hit, Some(hit), 1.0);
        let cry = match kind {
            Kind::Stranger => Sfx::Kill,
            Kind::Dog => Sfx::Yelp,
        };
        sound.play(cry, Some(hit), 1.0);
        burst(
            &mut commands,
            &assets.cube,
            &assets.blood,
            &mut gun.rng,
            hit,
            fnormal,
            14,
            0.025,
        );
        return;
    }
    if normal == Vec3::ZERO {
        return;
    }
    let hit = o + dir * t;
    sound.play(Sfx::Hit, Some(hit), 1.0);
    if mark {
        // A thin dark square on the face, 2 mm off it; the size jitters a little.
        let s = MARK_SIZE * gun.rng.range(0.8, 1.2);
        let size = Vec3::splat(s) * (Vec3::ONE - normal.abs()) + normal.abs() * 0.004;
        commands.spawn((
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(assets.mark.clone()),
            Transform::from_translation(hit + normal * 0.002).with_scale(size),
            NotShadowCaster,
            Mark { age: 0.0, size },
        ));
    }
    burst(
        &mut commands,
        &assets.cube,
        &assets.spark,
        &mut gun.rng,
        hit,
        normal,
        SPARKS,
        0.015,
    );
}

/// `n` small bits flying off a hit point, mostly along the face normal, falling under gravity.
#[allow(clippy::too_many_arguments)]
fn burst(
    commands: &mut Commands,
    cube: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
    rng: &mut Rng,
    hit: Vec3,
    normal: Vec3,
    n: usize,
    size: f32,
) {
    for _ in 0..n {
        let r = Vec3::new(
            rng.range(-1.0, 1.0),
            rng.range(-0.3, 1.0),
            rng.range(-1.0, 1.0),
        );
        let vel = (normal * 1.2 + r).normalize_or_zero() * rng.range(2.0, 5.0);
        commands.spawn((
            Mesh3d(cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_translation(hit + normal * 0.02).with_scale(Vec3::splat(size)),
            NotShadowCaster,
            Spark {
                vel,
                age: 0.0,
                size,
            },
        ));
    }
}

/// Smooth 0..1 over [a, b] of x.
fn ease(x: f32, a: f32, b: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Recoil, the reload, muzzle flash and its light; hidden behind the optic.
/// Reload over `RELOAD_TIME` (phase 0..1): cant the pistol (0-0.15), magazine out down the grip (0.15-0.35,
/// then hidden), new magazine up into the grip (0.45-0.7), slide racked back and released (0.75-0.88),
/// pistol back to rest (0.88-1).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn animate(
    time: Res<Time>,
    optic: Res<Optic>,
    mut gun: ResMut<Gun>,
    mut pistol: Single<(&mut Transform, &mut Visibility), With<Pistol>>,
    mut flash: Single<&mut Visibility, (With<MuzzleFlash>, Without<Pistol>)>,
    mut light: Single<&mut PointLight, With<FlashLight>>,
    mut slides: Query<(&Slide, &mut Transform), Without<Pistol>>,
    mut mag: Single<
        (&Magazine, &mut Transform, &mut Visibility),
        (Without<Pistol>, Without<Slide>, Without<MuzzleFlash>),
    >,
) {
    let dt = time.delta_secs();
    gun.recoil = (gun.recoil - dt * 8.0).max(0.0);
    gun.flash = (gun.flash - dt).max(0.0);
    let r = gun.recoil * gun.recoil;
    let p = gun.reload.map(|t| t / RELOAD_TIME);
    let cant = p
        .map(|p| ease(p, 0.0, 0.15) - ease(p, 0.88, 1.0))
        .unwrap_or(0.0);
    let (tf, vis) = &mut *pistol;
    tf.translation = REST
        + Vec3::new(
            -0.04 * cant,
            0.012 * r + 0.03 * cant,
            0.05 * r + 0.04 * cant,
        );
    tf.rotation = Quat::from_rotation_x(0.25 * r + 0.25 * cant) * Quat::from_rotation_z(0.7 * cant);
    **vis = if optic.on {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };

    // Magazine: out along the grip's axis, gone, then a new one in.
    let (rest, mtf, mvis) = &mut *mag;
    let out = p
        .map(|p| {
            if p < 0.4 {
                ease(p, 0.15, 0.35)
            } else {
                1.0 - ease(p, 0.45, 0.7)
            }
        })
        .unwrap_or(0.0);
    let axis = rest.0.rotation * Vec3::NEG_Y;
    mtf.translation = rest.0.translation + axis * 0.16 * out;
    **mvis = if p.is_some_and(|p| (0.35..0.45).contains(&p)) {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };

    // Slide: racked back by the hand, snapping forward; also kicks back a little with each shot.
    // Empty: locked back. A reload from empty keeps it back until the release snaps it forward.
    let rack = match p {
        Some(p) if gun.started_empty => 1.0 - ease(p, 0.84, 0.88),
        Some(p) => ease(p, 0.75, 0.82) - ease(p, 0.84, 0.88),
        None if gun.ammo == 0 => 1.0,
        None => 0.0,
    };
    for (slide, mut stf) in &mut slides {
        stf.translation = slide.0 + Vec3::Z * (0.045 * rack + 0.02 * r);
    }

    let on = gun.flash > 0.0 && !optic.on;
    **flash = if on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    light.intensity = if gun.flash > 0.0 { 60_000.0 } else { 0.0 };
}

fn sparks(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Spark, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (e, mut s, mut tf) in &mut q {
        s.age += dt;
        if s.age > SPARK_LIFE {
            commands.entity(e).despawn();
            continue;
        }
        s.vel.y -= 9.8 * dt;
        tf.translation += s.vel * dt;
        tf.scale = Vec3::splat(s.size * (1.0 - s.age / SPARK_LIFE));
    }
}

/// Marks age, shrink away at the end, and go with their chunk; the oldest go first past `MAX_MARKS`.
fn marks(
    mut commands: Commands,
    time: Res<Time>,
    level: Res<Level>,
    mut q: Query<(Entity, &mut Mark, &mut Transform)>,
) {
    let dt = time.delta_secs();
    let mut all: Vec<(f32, Entity)> = Vec::new();
    for (e, mut m, mut tf) in &mut q {
        m.age += dt;
        if m.age > MARK_LIFE || !level.loaded(tf.translation) {
            commands.entity(e).despawn();
            continue;
        }
        let fade = ((MARK_LIFE - m.age) / MARK_FADE).min(1.0);
        tf.scale = m.size * fade.max(0.05);
        all.push((m.age, e));
    }
    if all.len() > MAX_MARKS {
        all.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (_, e) in &all[..all.len() - MAX_MARKS] {
            commands.entity(*e).despawn();
        }
    }
}

/// Crosshair gap = the next shot's spread cone on screen: tan(radius) / tan(fov / 2) of half the window
/// height (vertical fov), eased so it opens on a shot and closes as the series cools; ammo text.
#[allow(clippy::too_many_arguments)]
fn ui(
    time: Res<Time>,
    gun: Res<Gun>,
    optic: Res<Optic>,
    window: Single<&Window>,
    camera: Single<(&Player, &Projection)>,
    mut gap: Local<f32>,
    mut cross: Single<&mut Visibility, With<Crosshair>>,
    mut ticks: Query<(&Tick, &mut Node)>,
    mut ammo: Single<&mut Text, With<AmmoText>>,
) {
    **cross = if optic.on {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let (player, projection) = *camera;
    let fov = match projection {
        Projection::Perspective(p) => p.fov,
        _ => crate::player::FOV,
    };
    let (w, h) = (window.width(), window.height());
    let (radius, _) = spread(player.motion(), gun.streak);
    let target = (radius.tan() / (fov / 2.0).tan() * h / 2.0).max(MIN_GAP);
    *gap += (target - *gap) * (time.delta_secs() * 20.0).min(1.0);
    let (cx, cy, g) = (w / 2.0, h / 2.0, *gap);
    for (tick, mut node) in &mut ticks {
        let (left, top, width, height) = match tick {
            Tick::Dot => (cx - TICK_W / 2.0, cy - TICK_W / 2.0, TICK_W, TICK_W),
            Tick::Left => (cx - g - TICK_LEN, cy - TICK_W / 2.0, TICK_LEN, TICK_W),
            Tick::Right => (cx + g, cy - TICK_W / 2.0, TICK_LEN, TICK_W),
            Tick::Up => (cx - TICK_W / 2.0, cy - g - TICK_LEN, TICK_W, TICK_LEN),
            Tick::Down => (cx - TICK_W / 2.0, cy + g, TICK_W, TICK_LEN),
        };
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        node.width = Val::Px(width);
        node.height = Val::Px(height);
    }
    ammo.0 = match gun.reload {
        Some(_) => "reloading".into(),
        None if gun.ammo == 0 => format!("0 / {MAGAZINE}  R reload"),
        None => format!("{} / {MAGAZINE}", gun.ammo),
    };
}

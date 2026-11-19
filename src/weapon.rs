//! Pistol: a first-person model at the bottom right of the view (a child of the camera), a crosshair in the
//! middle of the screen and the ammo at the bottom right. Left click (with the mouse grabbed) fires one shot
//! along the view: a short muzzle flash with a flash of light, recoil, and at the hit a bullet mark that
//! stays `MARK_LIFE` seconds plus a burst of sparks. Hits on cars and props only spark (they move or change).
//! Each shot leaves the center at a random angle inside a cone: its radius grows with movement (still,
//! walk, run, in the air) and with each quick follow-up shot (the streak cools down between shots).
//! `MAGAZINE` rounds; R reloads (the pistol dips out of view and back), an empty magazine only clicks.
//! While the optic is up the pistol and crosshair are hidden (the optic has its own reticle); shots still
//! go to the center.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::cars::CarBoxes;
use crate::level::{Level, PropBoxes, Rng, ray_box};
use crate::optic::Optic;
use crate::player::{Motion, Player};

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
    /// Shots in the current quick series (0 = the next one is a first shot), cooling down over time.
    streak: f32,
    rng: Rng,
}

#[derive(Resource)]
struct WeaponAssets {
    cube: Handle<Mesh>,
    mark: Handle<StandardMaterial>,
    spark: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Pistol;

#[derive(Component)]
struct MuzzleFlash;

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
}

#[derive(Component)]
struct Crosshair;

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
            c.spawn(part(
                Vec3::new(0.0, 0.03, -0.02),
                Vec3::new(0.032, 0.032, 0.18),
                &steel,
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
            c.spawn(part(
                Vec3::new(0.0, 0.05, -0.105),
                Vec3::new(0.005, 0.008, 0.006),
                &steel,
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
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|c| {
            c.spawn((
                Text::new("+"),
                font.clone(),
                TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
                Crosshair,
            ));
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

#[allow(clippy::too_many_arguments)]
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
    assets: Res<WeaponAssets>,
    mut gun: ResMut<Gun>,
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
    if keys.just_pressed(KeyCode::KeyR) && gun.reload.is_none() && gun.ammo < MAGAZINE {
        gun.reload = Some(0.0);
    }
    // The click that grabs the mouse doesn't shoot.
    let grabbed = cursor.grab_mode != CursorGrabMode::None;
    if !(grabbed && mouse.just_pressed(MouseButton::Left))
        || gun.reload.is_some()
        || gun.cooldown > 0.0
        || gun.ammo == 0
    {
        return;
    }
    gun.ammo -= 1;
    gun.cooldown = FIRE_GAP;
    gun.recoil = 1.0;
    gun.flash = FLASH;

    let (camera, player) = *camera;
    let base = match player.motion() {
        Motion::Still => SPREAD_STILL,
        Motion::Walk => SPREAD_WALK,
        Motion::Run => SPREAD_RUN,
        Motion::Air => SPREAD_AIR,
    };
    let nth = (gun.streak.ceil() as usize).min(STREAK_FACTOR.len() - 1);
    let radius = (base * STREAK_FACTOR[nth]).to_radians();
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
        (None, None) => return,
    };
    let hit = o + dir * t;
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
    for _ in 0..SPARKS {
        let r = Vec3::new(
            gun.rng.range(-1.0, 1.0),
            gun.rng.range(-0.3, 1.0),
            gun.rng.range(-1.0, 1.0),
        );
        let vel = (normal * 1.2 + r).normalize_or_zero() * gun.rng.range(2.0, 5.0);
        commands.spawn((
            Mesh3d(assets.cube.clone()),
            MeshMaterial3d(assets.spark.clone()),
            Transform::from_translation(hit + normal * 0.02).with_scale(Vec3::splat(0.015)),
            NotShadowCaster,
            Spark { vel, age: 0.0 },
        ));
    }
}

/// Recoil, reload dip, muzzle flash and its light; hidden behind the optic.
fn animate(
    time: Res<Time>,
    optic: Res<Optic>,
    mut gun: ResMut<Gun>,
    mut pistol: Single<(&mut Transform, &mut Visibility), With<Pistol>>,
    mut flash: Single<&mut Visibility, (With<MuzzleFlash>, Without<Pistol>)>,
    mut light: Single<&mut PointLight, With<FlashLight>>,
) {
    let dt = time.delta_secs();
    gun.recoil = (gun.recoil - dt * 8.0).max(0.0);
    gun.flash = (gun.flash - dt).max(0.0);
    let r = gun.recoil * gun.recoil;
    // Reload: down and tilted out of view, then back.
    let dip = gun
        .reload
        .map(|t| (t / RELOAD_TIME * std::f32::consts::PI).sin())
        .unwrap_or(0.0);
    let (tf, vis) = &mut *pistol;
    tf.translation = REST + Vec3::new(0.0, 0.012 * r - 0.25 * dip, 0.05 * r);
    tf.rotation = Quat::from_rotation_x(0.25 * r - 0.9 * dip) * Quat::from_rotation_z(0.5 * dip);
    **vis = if optic.on {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
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
        tf.scale = Vec3::splat(0.015 * (1.0 - s.age / SPARK_LIFE));
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

fn ui(
    gun: Res<Gun>,
    optic: Res<Optic>,
    mut cross: Single<&mut Visibility, With<Crosshair>>,
    mut ammo: Single<&mut Text, With<AmmoText>>,
) {
    **cross = if optic.on {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    ammo.0 = match gun.reload {
        Some(_) => "reloading".into(),
        None if gun.ammo == 0 => format!("0 / {MAGAZINE}  R reload"),
        None => format!("{} / {MAGAZINE}", gun.ammo),
    };
}

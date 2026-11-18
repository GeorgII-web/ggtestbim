//! First-person walker: mouse look, WASD, Shift to run, Space to jump, gravity, box collision with step-up
//! (stairs, curbs). Click grabs the mouse, Esc releases it.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::level::{Aabb, Level};

const RADIUS: f32 = 0.35;
const HEIGHT: f32 = 1.8;
const EYE: f32 = 1.65;
const WALK: f32 = 5.0;
const RUN: f32 = 9.0;
const JUMP: f32 = 7.0;
const GRAVITY: f32 = 20.0;
const MAX_FALL: f32 = 50.0;
/// Highest ledge walked up without jumping (a stair step is 0.25).
const STEP: f32 = 0.55;
const SENSITIVITY: f32 = 0.0025;
/// Longest move per collision substep, below the thinnest slab (0.2) to avoid tunneling.
const MAX_SUBSTEP: f32 = 0.15;
const SPAWN: Vec3 = Vec3::new(0.0, 0.5, 0.0);

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_player, grab))
            .add_systems(Update, (cursor_toggle, look, walk).chain());
    }
}

#[derive(Component)]
pub struct Player {
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    pitch: f32,
    grounded: bool,
}

fn spawn_player(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 75f32.to_radians(),
            ..default()
        }),
        Transform::from_translation(SPAWN + Vec3::Y * EYE),
        Player {
            pos: SPAWN,
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            grounded: false,
        },
    ));
}

fn set_grab(cursor: &mut CursorOptions, on: bool) {
    cursor.grab_mode = if on {
        CursorGrabMode::Locked
    } else {
        CursorGrabMode::None
    };
    cursor.visible = !on;
}

fn grab(mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    set_grab(&mut cursor, true);
}

fn cursor_toggle(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if mouse.just_pressed(MouseButton::Left) {
        set_grab(&mut cursor, true);
    }
    if keys.just_pressed(KeyCode::Escape) {
        set_grab(&mut cursor, false);
    }
}

fn look(
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    mut player: Single<&mut Player>,
) {
    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    player.yaw -= motion.delta.x * SENSITIVITY;
    player.pitch = (player.pitch - motion.delta.y * SENSITIVITY).clamp(-1.54, 1.54);
}

fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<Level>,
    player: Single<(&mut Player, &mut Transform)>,
) {
    let (mut p, mut tf) = player.into_inner();
    let dt = time.delta_secs().min(1.0 / 30.0);

    if keys.just_pressed(KeyCode::KeyR) || p.pos.y < -30.0 {
        p.pos = SPAWN;
        p.vel = Vec3::ZERO;
    }

    let forward = Vec3::new(-p.yaw.sin(), 0.0, -p.yaw.cos());
    let right = Vec3::new(p.yaw.cos(), 0.0, -p.yaw.sin());
    let mut wish = Vec3::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        wish += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        wish -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        wish += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        wish -= right;
    }
    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        RUN
    } else {
        WALK
    };
    let wish = wish.normalize_or_zero() * speed;
    p.vel.x = wish.x;
    p.vel.z = wish.z;
    if p.grounded && keys.pressed(KeyCode::Space) {
        p.vel.y = JUMP;
        p.grounded = false;
    }
    p.vel.y = (p.vel.y - GRAVITY * dt).max(-MAX_FALL);

    let n = ((p.vel * dt).abs().max_element() / MAX_SUBSTEP)
        .ceil()
        .max(1.0) as usize;
    for _ in 0..n {
        substep(&mut p, &level.boxes, dt / n as f32);
    }

    tf.translation = p.pos + Vec3::Y * EYE;
    tf.rotation = Quat::from_euler(EulerRot::YXZ, p.yaw, p.pitch, 0.0);
}

/// Moves along x, then z (each blocked or stepped up on its own, so the player slides along walls), then y.
fn substep(p: &mut Player, boxes: &[Aabb], h: f32) {
    let was_grounded = p.grounded;
    for axis in [0, 2] {
        let d = p.vel[axis] * h;
        if d == 0.0 {
            continue;
        }
        let mut next = p.pos;
        next[axis] += d;
        if !hits(boxes, next) {
            p.pos = next;
        } else if was_grounded && let Some(y) = step_up(boxes, next, p.pos.y) {
            p.pos = Vec3::new(next.x, y, next.z);
        }
    }

    let mut next = p.pos;
    next.y += p.vel.y * h;
    let body = body(next);
    let mut touched = boxes.iter().filter(|b| overlap(&body, b)).peekable();
    if touched.peek().is_none() {
        p.pos = next;
        p.grounded = false;
    } else if p.vel.y < 0.0 {
        p.pos.y = touched.map(|b| b.max.y).fold(f32::MIN, f32::max);
        p.vel.y = 0.0;
        p.grounded = true;
    } else {
        p.pos.y = touched.map(|b| b.min.y).fold(f32::MAX, f32::min) - HEIGHT;
        p.vel.y = 0.0;
    }
}

/// Feet height on top of the boxes blocking `next`, when all are low enough to step on and the space is free.
fn step_up(boxes: &[Aabb], next: Vec3, y: f32) -> Option<f32> {
    let body = body(next);
    let top = boxes
        .iter()
        .filter(|b| overlap(&body, b))
        .map(|b| b.max.y)
        .fold(f32::MIN, f32::max);
    (top <= y + STEP && !hits(boxes, Vec3::new(next.x, top, next.z))).then_some(top)
}

fn body(feet: Vec3) -> Aabb {
    Aabb {
        min: feet - Vec3::new(RADIUS, 0.0, RADIUS),
        max: feet + Vec3::new(RADIUS, HEIGHT, RADIUS),
    }
}

fn hits(boxes: &[Aabb], feet: Vec3) -> bool {
    let body = body(feet);
    boxes.iter().any(|b| overlap(&body, b))
}

/// Strict overlap with a small margin, so touching faces (standing on a floor, leaning on a wall) don't count.
fn overlap(a: &Aabb, b: &Aabb) -> bool {
    const E: f32 = 1e-4;
    a.min.x < b.max.x - E
        && a.max.x > b.min.x + E
        && a.min.y < b.max.y - E
        && a.max.y > b.min.y + E
        && a.min.z < b.max.z - E
        && a.max.z > b.min.z + E
}

//! First-person walker: mouse look, WASD (instant on the ground, a short glide after release), running by default, Shift to walk, Ctrl to
//! crouch (fits through windows), Space to jump, gravity, box collision with step-up on stairs and ledges
//! (also in the air, to climb onto a window sill), snap-down on the way down and a landing dip. Cars and
//! props (trees, benches) collide too.
//! Click grabs the mouse, Esc releases it.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

use crate::cars::CarBoxes;
use crate::level::{Aabb, Level, PropBoxes};
use crate::optic::Optic;

const RADIUS: f32 = 0.35;
const STAND: f32 = 1.8;
/// Crouched body height; a window opening is 1.3 m high.
const CROUCH: f32 = 0.9;
/// Eye below the top of the body.
const EYE_DROP: f32 = 0.15;
const RUN: f32 = 12.0;
const WALK: f32 = 3.0;
const CRAWL: f32 = 2.0;
/// On the ground a held direction sets the speed at once (a new direction cancels the old motion, so quick
/// left/right taps react instantly); with no key held the body glides to a stop at this braking (full run:
/// about 0.25 s and 1.5 m).
const BRAKE: f32 = 48.0;
/// Little air control: a jump keeps the speed it took off with.
const AIR_ACCEL: f32 = 4.0;
/// Short, heavy jump: apex 0.94 m, 0.5 s in the air. Enough for a crouched body to reach a 1.0 m
/// window sill with the air step-up (feet above sill - STEP).
const JUMP: f32 = 7.5;
/// Crouched (Ctrl held) the jump is higher: apex 1.12 m, so with the air step-up the feet reach a 1.5 m
/// terrace ledge (1.5 - STEP = 0.95 m), and the crouched body still fits a window (sill 1.0 + 1.12 + 0.9 < 2.3).
const CROUCH_JUMP: f32 = 8.2;
const GRAVITY: f32 = 30.0;
/// A jump still works this long after leaving the ground (coyote time), and a press this long before
/// landing is kept until the landing (jump buffer); jumping while running at full speed stays reliable.
const JUMP_GRACE: f32 = 0.12;
const MAX_FALL: f32 = 50.0;
/// Highest ledge stepped onto without jumping (a stair step is 0.25).
const STEP: f32 = 0.55;
const SENSITIVITY: f32 = 0.0006;
/// Vertical field of view without the optic (it narrows it by its zoom).
pub(crate) const FOV: f32 = 75.0 * std::f32::consts::PI / 180.0;
/// Longest move per collision substep, below the thinnest slab (0.2) to avoid tunneling.
const MAX_SUBSTEP: f32 = 0.15;
const SPAWN: Vec3 = Vec3::new(0.0, 0.5, 0.0);

pub struct PlayerPlugin;

/// The player's input, look and movement; systems that use the camera's pose for this frame run after it.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct PlayerSet;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_player, grab)).add_systems(
            Update,
            (cursor_toggle, look, walk)
                .chain()
                .in_set(PlayerSet)
                .after(crate::cars::drive)
                .after(crate::level::prop_boxes),
        );
    }
}

#[derive(Component)]
pub struct Player {
    pos: Vec3,
    vel: Vec3,
    yaw: f32,
    pitch: f32,
    grounded: bool,
    height: f32,
    /// Smoothed eye height above the feet.
    eye: f32,
    /// Camera dip after a landing, decays back to 0.
    dip: f32,
    /// Seconds since the feet last touched the ground, and since Space was last pressed.
    air_time: f32,
    jump_pressed: f32,
    /// 0..=MAX_HEALTH; 0 is death (the health module respawns the player).
    pub(crate) health: f32,
    /// Seconds since the last damage (health regenerates after a while).
    pub(crate) since_hurt: f32,
    /// Dead: no movement, look or shooting until the health module respawns the player.
    pub(crate) dead: bool,
    /// Seconds since the last (re)spawn or new world (dogs keep away for a while).
    pub(crate) since_spawn: f32,
}

/// How the player moves right now, for the gun's spread.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) enum Motion {
    Still,
    Walk,
    Run,
    Air,
}

pub(crate) const MAX_HEALTH: f32 = 100.0;
/// Landing faster than this hurts, `FALL_DAMAGE` per m/s above it (a 1-story drop is ~14.5 m/s).
const SAFE_FALL: f32 = 13.0;
const FALL_DAMAGE: f32 = 7.0;

impl Player {
    pub(crate) fn hurt(&mut self, amount: f32) {
        self.health = (self.health - amount).max(0.0);
        self.since_hurt = 0.0;
    }

    /// Back to the spawn point with full health.
    pub(crate) fn respawn(&mut self) {
        self.pos = SPAWN;
        self.vel = Vec3::ZERO;
        self.health = MAX_HEALTH;
        self.since_hurt = f32::MAX;
        self.dead = false;
        self.since_spawn = 0.0;
    }

    /// Feet position.
    pub(crate) fn feet(&self) -> Vec3 {
        self.pos
    }

    /// Horizontal speed in m/s.
    pub(crate) fn ground_speed(&self) -> f32 {
        Vec2::new(self.vel.x, self.vel.z).length()
    }

    pub(crate) fn crouched(&self) -> bool {
        self.height < STAND
    }

    pub(crate) fn motion(&self) -> Motion {
        let speed = Vec2::new(self.vel.x, self.vel.z).length();
        if !self.grounded {
            Motion::Air
        } else if speed < 0.5 {
            Motion::Still
        } else if speed <= WALK + 0.5 {
            Motion::Walk
        } else {
            Motion::Run
        }
    }
}

fn spawn_player(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        // No MSAA and the cheapest shadow filter: the target is an integrated laptop GPU (Iris Xe).
        Msaa::Off,
        bevy::light::ShadowFilteringMethod::Hardware2x2,
        Projection::Perspective(PerspectiveProjection {
            fov: FOV,
            ..default()
        }),
        DistanceFog {
            color: Color::srgb(0.62, 0.77, 0.93),
            falloff: FogFalloff::Linear {
                start: 45.0,
                end: 115.0,
            },
            ..default()
        },
        Transform::from_translation(SPAWN + Vec3::Y * (STAND - EYE_DROP)),
        Player {
            pos: SPAWN,
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            grounded: false,
            height: STAND,
            eye: STAND - EYE_DROP,
            dip: 0.0,
            air_time: 0.0,
            jump_pressed: f32::MAX,
            health: MAX_HEALTH,
            since_hurt: f32::MAX,
            dead: false,
            since_spawn: 0.0,
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
    optic: Res<Optic>,
    mut player: Single<&mut Player>,
) {
    if cursor.grab_mode == CursorGrabMode::None || player.dead {
        return;
    }
    // Through the optic the view turns slower by its zoom, so aiming stays steady.
    let s = SENSITIVITY / optic.zoom;
    player.yaw -= motion.delta.x * s;
    player.pitch = (player.pitch - motion.delta.y * s).clamp(-1.54, 1.54);
}

fn walk(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    level: Res<Level>,
    cars: Res<CarBoxes>,
    props: Res<PropBoxes>,
    player: Single<(&mut Player, &mut Transform)>,
) {
    let (mut p, mut tf) = player.into_inner();
    let dt = time.delta_secs().min(1.0 / 30.0);
    // Dead: the body stays where it fell until the respawn.
    if p.dead {
        return;
    }
    p.since_spawn += dt;
    if keys.just_pressed(KeyCode::KeyN) || p.pos.y < -30.0 {
        p.pos = SPAWN;
        p.vel = Vec3::ZERO;
        p.since_spawn = 0.0;
    }
    // Wait until the chunk under the player is generated.
    if !level.loaded(p.pos) {
        return;
    }
    // One pass over the nearby chunks, cars and props keeps only the boxes this frame's move can touch;
    // the substeps then test a few dozen boxes instead of tens of thousands.
    let reach = {
        let span = p.vel.abs() * dt + Vec3::splat(2.0);
        Aabb {
            min: p.pos - Vec3::new(RADIUS, 0.0, RADIUS) - span,
            max: p.pos + Vec3::new(RADIUS, STAND, RADIUS) + span,
        }
    };
    let mut sources = level.near(p.pos);
    sources.push(&cars.0);
    sources.push(&props.0);
    let local: Vec<Aabb> = boxes(&sources)
        .filter(|b| overlap(&reach, b))
        .copied()
        .collect();
    let near = [&local[..]];

    let crouch_key = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    // Crouching starts only on the ground (Ctrl in the air does nothing); a crouch begun on the ground holds
    // through a jump, for climbing into windows.
    if crouch_key && (p.grounded || p.height < STAND) {
        p.height = CROUCH;
    } else if p.height < STAND && !hits(&near, p.pos, STAND) {
        p.height = STAND;
    }
    let crouched = p.height < STAND;

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
    let speed = if crouched {
        CRAWL
    } else if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        WALK
    } else {
        RUN
    };
    let wish = wish.normalize_or_zero() * speed;
    let horizontal = Vec2::new(p.vel.x, p.vel.z);
    let target = Vec2::new(wish.x, wish.z);
    let horizontal = match (p.grounded, wish != Vec3::ZERO) {
        (true, true) => target,
        (true, false) => horizontal + (target - horizontal).clamp_length_max(BRAKE * dt),
        (false, _) => horizontal + (target - horizontal).clamp_length_max(AIR_ACCEL * dt),
    };
    p.vel.x = horizontal.x;
    p.vel.z = horizontal.y;

    p.air_time = if p.grounded { 0.0 } else { p.air_time + dt };
    p.jump_pressed = if keys.just_pressed(KeyCode::Space) {
        0.0
    } else {
        p.jump_pressed + dt
    };
    let held = keys.pressed(KeyCode::Space) && p.grounded;
    if (p.jump_pressed <= JUMP_GRACE || held) && p.air_time <= JUMP_GRACE && p.vel.y <= 0.0 {
        p.vel.y = if crouched { CROUCH_JUMP } else { JUMP };
        p.grounded = false;
        p.air_time = JUMP_GRACE + 1.0;
        p.jump_pressed = f32::MAX;
    }
    p.vel.y = (p.vel.y - GRAVITY * dt).max(-MAX_FALL);

    let fall_speed = -p.vel.y;
    let was_grounded = p.grounded;
    let n = ((p.vel * dt).abs().max_element() / MAX_SUBSTEP)
        .ceil()
        .max(1.0) as usize;
    for _ in 0..n {
        substep(&mut p, &near, dt / n as f32);
    }
    if p.grounded && !was_grounded && fall_speed > 4.0 {
        p.dip = (fall_speed * 0.02).min(0.3);
        if fall_speed > SAFE_FALL {
            p.hurt((fall_speed - SAFE_FALL) * FALL_DAMAGE);
        }
    }

    // Eye height (crouching) and the landing dip ease toward their targets.
    let target_eye = p.height - EYE_DROP;
    p.eye += (target_eye - p.eye) * (1.0 - (-12.0 * dt).exp());
    p.dip *= (-6.0 * dt).exp();

    tf.translation = p.pos + Vec3::Y * (p.eye - p.dip);
    tf.rotation = Quat::from_euler(EulerRot::YXZ, p.yaw, p.pitch, 0.0);
}

/// Moves along x, then z (each blocked or stepped up on its own, so the player slides along walls), then y.
fn substep(p: &mut Player, near: &[&[Aabb]], h: f32) {
    let was_grounded = p.grounded;
    for axis in [0, 2] {
        let d = p.vel[axis] * h;
        if d == 0.0 {
            continue;
        }
        let mut next = p.pos;
        next[axis] += d;
        if !hits(near, next, p.height) {
            p.pos = next;
        } else if let Some(y) = step_up(near, next, p.pos.y, p.height) {
            p.pos = Vec3::new(next.x, y, next.z);
            p.vel.y = p.vel.y.max(0.0);
        } else {
            p.vel[axis] = 0.0;
        }
    }

    let mut next = p.pos;
    next.y += p.vel.y * h;
    let body = body(next, p.height);
    let touched: Vec<&Aabb> = boxes(near).filter(|b| overlap(&body, b)).collect();
    if touched.is_empty() {
        // Walking down stairs or off a low ledge: stay on the ground instead of a small fall.
        match (was_grounded && p.vel.y <= 0.0)
            .then(|| ground_below(near, p.pos))
            .flatten()
        {
            Some(y) => {
                p.pos.y = y;
                p.vel.y = 0.0;
            }
            None => {
                p.pos = next;
                p.grounded = false;
            }
        }
    } else if p.vel.y < 0.0 {
        p.pos.y = touched.iter().map(|b| b.max.y).fold(f32::MIN, f32::max);
        p.vel.y = 0.0;
        p.grounded = true;
    } else {
        p.pos.y = touched.iter().map(|b| b.min.y).fold(f32::MAX, f32::min) - p.height;
        p.vel.y = 0.0;
    }
}

fn boxes<'a>(near: &'a [&'a [Aabb]]) -> impl Iterator<Item = &'a Aabb> {
    near.iter().flat_map(|s| s.iter())
}

/// Feet height on top of the boxes blocking `next`, when all are low enough to step on and the body fits there.
fn step_up(near: &[&[Aabb]], next: Vec3, y: f32, height: f32) -> Option<f32> {
    let body = body(next, height);
    let top = boxes(near)
        .filter(|b| overlap(&body, b))
        .map(|b| b.max.y)
        .fold(f32::MIN, f32::max);
    (top <= y + STEP && !hits(near, Vec3::new(next.x, top, next.z), height)).then_some(top)
}

/// Highest floor within a step below the feet.
fn ground_below(near: &[&[Aabb]], feet: Vec3) -> Option<f32> {
    let probe = Aabb {
        min: feet - Vec3::new(RADIUS, STEP, RADIUS),
        max: feet + Vec3::new(RADIUS, 0.0, RADIUS),
    };
    let top = boxes(near)
        .filter(|b| overlap(&probe, b))
        .map(|b| b.max.y)
        .fold(f32::MIN, f32::max);
    (top > f32::MIN && top <= feet.y).then_some(top)
}

fn body(feet: Vec3, height: f32) -> Aabb {
    Aabb {
        min: feet - Vec3::new(RADIUS, 0.0, RADIUS),
        max: feet + Vec3::new(RADIUS, height, RADIUS),
    }
}

fn hits(near: &[&[Aabb]], feet: Vec3, height: f32) -> bool {
    let body = body(feet, height);
    boxes(near).any(|b| overlap(&body, b))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::{FH, LANE, T, stair_x0, stair_x1, test_building, test_terrace};

    fn player_at(pos: Vec3) -> Player {
        Player {
            pos,
            vel: Vec3::ZERO,
            yaw: 0.0,
            pitch: 0.0,
            grounded: true,
            height: STAND,
            eye: 0.0,
            dip: 0.0,
            air_time: 0.0,
            jump_pressed: f32::MAX,
            health: MAX_HEALTH,
            since_hurt: f32::MAX,
            dead: false,
            since_spawn: 0.0,
        }
    }

    /// Walks straight toward `target` (x, z) at walking speed, with gravity, for at most `secs`.
    fn walk_to(p: &mut Player, near: &[&[Aabb]], target: Vec2, secs: f32) {
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while t < secs {
            let to = target - Vec2::new(p.pos.x, p.pos.z);
            if to.length() < 0.05 {
                return;
            }
            let v = to.normalize() * WALK.min(to.length() / dt);
            p.vel.x = v.x;
            p.vel.z = v.y;
            p.vel.y = (p.vel.y - GRAVITY * dt).max(-MAX_FALL);
            let n = ((p.vel * dt).abs().max_element() / MAX_SUBSTEP)
                .ceil()
                .max(1.0) as usize;
            for _ in 0..n {
                substep(p, near, dt / n as f32);
            }
            t += dt;
        }
    }

    /// Up a 6-story building flight by flight: along each lane, turn on the landing, to the roof.
    #[test]
    fn climbs_switchback_stairs_to_the_roof() {
        let floors = 6;
        let boxes = test_building(floors);
        let near = [&boxes[..]];
        let (sx0, sx1) = (stair_x0(0.0), stair_x1(0.0));
        let lane_z = |lane: u32| T + (lane as f32 + 0.5) * LANE;
        let (west, east) = ((T + sx0) / 2.0, sx1 + 1.0);
        let mut p = player_at(Vec3::new(west, 0.05, lane_z(0)));
        for k in 0..floors {
            let lane = k % 2;
            let (from, to) = if lane == 0 {
                (west, east)
            } else {
                (east, west)
            };
            walk_to(&mut p, &near, Vec2::new(from, lane_z(lane)), 3.0);
            walk_to(&mut p, &near, Vec2::new(to, lane_z(lane)), 6.0);
            let expected = (k + 1) as f32 * FH;
            assert!(
                (p.pos.y - expected).abs() < 0.1,
                "flight {k}: expected y {expected}, at {:?}",
                p.pos
            );
        }
    }

    /// Up the flight beside a 6 m terrace from the street, then sideways onto the terrace.
    #[test]
    fn climbs_terrace_stairs() {
        let (boxes, steps) = test_terrace(6.0);
        let near = [&boxes[..]];
        let (low, high) = (steps[0], steps[steps.len() - 1]);
        let dir = (high.min.x - low.min.x).signum();
        let z = (low.min.z + low.max.z) / 2.0;
        let start = (low.min.x + low.max.x) / 2.0 - dir * 1.5;
        let end = (high.min.x + high.max.x) / 2.0;
        let mut p = player_at(Vec3::new(start, 0.0, z));
        walk_to(&mut p, &near, Vec2::new(end, z), 8.0);
        walk_to(&mut p, &near, Vec2::new(end, 2.0), 2.0);
        assert!(
            (p.pos.y - 6.0).abs() < 0.1 && p.pos.z > 1.5,
            "at {:?}",
            p.pos
        );
    }

    /// Crouched, jump at a window (sill 1.0 m, top 2.3 m, wall 0.3 m) and walk on: through it.
    #[test]
    fn crouch_jumps_through_a_window() {
        let b = |min: Vec3, max: Vec3| Aabb { min, max };
        let boxes = [
            b(Vec3::new(-10.0, -1.0, -10.0), Vec3::new(10.0, 0.0, 10.0)),
            b(Vec3::new(0.0, 0.0, -5.0), Vec3::new(0.3, 1.0, 5.0)),
            b(Vec3::new(0.0, 2.3, -5.0), Vec3::new(0.3, 3.5, 5.0)),
            b(Vec3::new(0.0, 1.0, -5.0), Vec3::new(0.3, 2.3, -0.6)),
            b(Vec3::new(0.0, 1.0, 0.6), Vec3::new(0.3, 2.3, 5.0)),
        ];
        let near = [&boxes[..]];
        let mut p = player_at(Vec3::new(-0.6, 0.0, 0.0));
        p.height = CROUCH;
        p.vel.y = CROUCH_JUMP;
        p.grounded = false;
        walk_to(&mut p, &near, Vec2::new(2.0, 0.0), 3.0);
        assert!(p.pos.x > 1.5, "stuck at {:?}", p.pos);
    }

    /// Crouched, jump at a 1.5 m terrace ledge and walk on: up onto it.
    #[test]
    fn crouch_jumps_onto_a_terrace_ledge() {
        let b = |min: Vec3, max: Vec3| Aabb { min, max };
        let boxes = [
            b(Vec3::new(-10.0, -1.0, -10.0), Vec3::new(10.0, 0.0, 10.0)),
            b(Vec3::new(0.0, 0.0, -10.0), Vec3::new(10.0, 1.5, 10.0)),
        ];
        let near = [&boxes[..]];
        let mut p = player_at(Vec3::new(-0.4, 0.0, 0.0));
        p.height = CROUCH;
        p.vel.y = CROUCH_JUMP;
        p.grounded = false;
        walk_to(&mut p, &near, Vec2::new(2.0, 0.0), 3.0);
        assert!(p.pos.x > 1.5 && p.pos.y > 1.4, "stuck at {:?}", p.pos);
    }
}

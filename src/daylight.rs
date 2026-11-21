//! Day and night: the clock runs `DAY_SECONDS` real seconds per 24 h and starts at a random hour on launch
//! and on every new world (N). The one directional light is the sun by day (rising in the east at 6:00,
//! highest at noon, warm near the horizon) and the moon by night (dim, bluish, from the opposite side);
//! sky, fog and ambient light follow: blue day, orange dusk and dawn, dark navy night. A sun disc and a moon
//! disc sit far away in the sky along the light directions (unlit, no fog, behind clouds and buildings). The
//! hour is shown under the fps.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;

use crate::level::Rng;
use crate::player::Player;

pub struct DaylightPlugin;

impl Plugin for DaylightPlugin {
    fn build(&self, app: &mut App) {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        let mut rng = Rng(seed ^ 0xDA7);
        let hour = rng.range(0.0, 24.0);
        app.insert_resource(DayTime { hour, rng })
            .add_systems(Startup, (setup_clock, setup_discs))
            .add_systems(
                Update,
                (advance, apply, place_discs)
                    .chain()
                    .after(crate::player::PlayerSet),
            );
    }
}

/// Real seconds for a full day.
const DAY_SECONDS: f32 = 20.0 * 60.0;
const SUN_LUX: f32 = 8000.0;
const MOON_LUX: f32 = 350.0;
const DAY_AMBIENT: f32 = 300.0;
const NIGHT_AMBIENT: f32 = 25.0;
const DAY_SKY: Vec3 = Vec3::new(0.62, 0.77, 0.93);
const DUSK_SKY: Vec3 = Vec3::new(0.86, 0.55, 0.38);
const NIGHT_SKY: Vec3 = Vec3::new(0.02, 0.03, 0.07);

/// How far the sun and moon discs sit from the camera (inside the 1000 m far plane, beyond the clouds).
const DISC_DISTANCE: f32 = 400.0;
const SUN_RADIUS: f32 = 9.0;
const MOON_RADIUS: f32 = 7.0;

#[derive(Component)]
enum Disc {
    Sun,
    Moon,
}

#[derive(Resource)]
pub struct DayTime {
    /// Hour of the day, 0..24.
    pub hour: f32,
    rng: Rng,
}

#[derive(Component)]
struct Clock;

fn setup_clock(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(18.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(32.0),
            right: Val::Px(8.0),
            ..default()
        },
        Clock,
    ));
}

fn setup_discs(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (disc, radius, color) in [
        (Disc::Sun, SUN_RADIUS, Color::srgb(1.0, 0.95, 0.8)),
        (Disc::Moon, MOON_RADIUS, Color::srgb(0.8, 0.84, 0.92)),
    ] {
        commands.spawn((
            Mesh3d(meshes.add(Sphere::new(radius).mesh().ico(3).expect("ico sphere"))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: color,
                unlit: true,
                fog_enabled: false,
                ..default()
            })),
            Transform::default(),
            NotShadowCaster,
            disc,
        ));
    }
}

fn advance(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut day: ResMut<DayTime>) {
    if keys.just_pressed(KeyCode::KeyN) {
        day.hour = day.rng.range(0.0, 24.0);
    }
    day.hour = (day.hour + time.delta_secs() * 24.0 / DAY_SECONDS).rem_euclid(24.0);
}

/// Sun height for an hour: -1 at midnight, 0 at 6:00 and 18:00, 1 at noon.
pub fn sun_height(hour: f32) -> f32 {
    ((hour - 6.0) / 24.0 * std::f32::consts::TAU).sin()
}

pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Unit directions to the sun and to the moon for an hour, and the sun height (`sun_height`).
fn directions(hour: f32) -> (f32, Vec3, Vec3) {
    let h = sun_height(hour);
    // Compass angle of the sun: east (+x) at 6:00, south (+z) at noon, west at 18:00.
    let az = (hour - 6.0) / 24.0 * std::f32::consts::TAU;
    let sun = Vec3::new(az.cos(), 0.0, az.sin()) * (1.0 - h * h).max(0.0).sqrt() + Vec3::Y * h;
    let moon = Vec3::new(-sun.x, -sun.y * 0.8 + 0.2, -sun.z).normalize_or(Vec3::Y);
    (h, sun, moon)
}

/// Low sun is warm (1), high sun white (0).
fn warmth(h: f32) -> f32 {
    1.0 - smooth(0.05, 0.4, h)
}

#[allow(clippy::type_complexity)]
fn apply(
    day: Res<DayTime>,
    mut light: Single<(&mut DirectionalLight, &mut Transform), Without<Player>>,
    mut fog: Single<&mut DistanceFog, With<Player>>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clock: Single<&mut Text, With<Clock>>,
) {
    let (h, sun_dir, moon_dir) = directions(day.hour);
    let day_k = smooth(-0.05, 0.25, h);
    let night_k = smooth(0.0, -0.2, h);
    let (lit, dir) = if h > -0.02 {
        (day_k, sun_dir)
    } else {
        (night_k, moon_dir)
    };
    let (dl, tf) = &mut *light;
    // Low sun is warm, high sun white, the moon bluish.
    let warm = warmth(h);
    dl.illuminance = if h > -0.02 {
        SUN_LUX * lit
    } else {
        MOON_LUX * lit
    };
    dl.color = if h > -0.02 {
        Color::srgb(1.0, 1.0 - 0.25 * warm, 1.0 - 0.5 * warm)
    } else {
        Color::srgb(0.6, 0.7, 1.0)
    };
    let pos = dir * 60.0;
    **tf = Transform::from_translation(pos).looking_at(Vec3::ZERO, Vec3::Y);

    // Sky: night -> dusk around the horizon -> day.
    let dusk = smooth(-0.2, 0.0, h) * (1.0 - smooth(0.05, 0.3, h));
    let base = NIGHT_SKY.lerp(DAY_SKY, smooth(-0.1, 0.25, h));
    let sky = base.lerp(DUSK_SKY, dusk * 0.6);
    let color = Color::srgb(sky.x, sky.y, sky.z);
    clear.0 = color;
    fog.color = color;
    ambient.brightness = NIGHT_AMBIENT + (DAY_AMBIENT - NIGHT_AMBIENT) * smooth(-0.15, 0.2, h);
    ambient.color = if h > -0.05 {
        Color::WHITE
    } else {
        Color::srgb(0.7, 0.75, 1.0)
    };

    let minutes = (day.hour * 60.0) as u32;
    clock.0 = format!("{:02}:{:02}", minutes / 60, minutes % 60);
}

/// The discs follow the camera, so they stay at one spot in the sky; hidden below the horizon (the moon also
/// by full day). The sun is orange near the horizon, pale yellow-white high up.
#[allow(clippy::type_complexity)]
fn place_discs(
    day: Res<DayTime>,
    camera: Single<&Transform, With<Player>>,
    mut discs: Query<
        (
            &Disc,
            &mut Transform,
            &mut Visibility,
            &MeshMaterial3d<StandardMaterial>,
        ),
        Without<Player>,
    >,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (h, sun_dir, moon_dir) = directions(day.hour);
    let eye = camera.translation;
    for (disc, mut tf, mut vis, mat) in &mut discs {
        let (d, shown) = match disc {
            Disc::Sun => (sun_dir, sun_dir.y > -0.05),
            Disc::Moon => (moon_dir, moon_dir.y > -0.05 && h < 0.15),
        };
        tf.translation = eye + d * DISC_DISTANCE;
        *vis = if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if let (Disc::Sun, Some(mut m)) = (disc, materials.get_mut(&mat.0)) {
            let warm = warmth(h);
            m.base_color = Color::srgb(1.0, 0.95 - 0.4 * warm, 0.8 - 0.55 * warm);
        }
    }
}

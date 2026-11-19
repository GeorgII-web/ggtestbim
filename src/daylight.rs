//! Day and night: the clock runs `DAY_SECONDS` real seconds per 24 h and starts at a random hour on launch
//! and on every new world (N). The one directional light is the sun by day (rising in the east at 6:00,
//! highest at noon, warm near the horizon) and the moon by night (dim, bluish, from the opposite side);
//! sky, fog and ambient light follow: blue day, orange dusk and dawn, dark navy night. The hour is shown
//! under the fps.

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
            .add_systems(Startup, setup_clock)
            .add_systems(Update, (advance, apply).chain());
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

#[allow(clippy::type_complexity)]
fn apply(
    day: Res<DayTime>,
    mut light: Single<(&mut DirectionalLight, &mut Transform), Without<Player>>,
    mut fog: Single<&mut DistanceFog, With<Player>>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut clock: Single<&mut Text, With<Clock>>,
) {
    let tau = std::f32::consts::TAU;
    let h = sun_height(day.hour);
    // Compass angle of the sun: east (+x) at 6:00, south (+z) at noon, west at 18:00.
    let az = (day.hour - 6.0) / 24.0 * tau;
    let sun_dir = Vec3::new(az.cos(), 0.0, az.sin()) * (1.0 - h * h).max(0.0).sqrt() + Vec3::Y * h;
    let day_k = smooth(-0.05, 0.25, h);
    let night_k = smooth(0.0, -0.2, h);
    let (lit, dir) = if h > -0.02 {
        (day_k, sun_dir)
    } else {
        (
            night_k,
            Vec3::new(-sun_dir.x, -sun_dir.y * 0.8 + 0.2, -sun_dir.z),
        )
    };
    let (dl, tf) = &mut *light;
    // Low sun is warm, high sun white, the moon bluish.
    let warm = 1.0 - smooth(0.05, 0.4, h);
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
    let pos = dir.normalize_or(Vec3::Y) * 60.0;
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

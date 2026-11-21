//! Flashlight on F: a spot light from the camera, for the night. It makes the player easier to spot:
//! `Exposure` is how far strangers and dogs can see the player (daylight 50 m, night 22 m, flashlight on at
//! least 70 m; crouching shortens it to 70 %).

use bevy::prelude::*;

use crate::daylight::{DayTime, sun_height};
use crate::player::Player;
use crate::sound::{Sfx, SoundQueue};

pub struct FlashlightPlugin;

impl Plugin for FlashlightPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Exposure(DAY_RANGE))
            .insert_resource(Flashlight(false))
            .add_systems(PostStartup, setup_flashlight)
            .add_systems(Update, (toggle, expose));
    }
}

const DAY_RANGE: f32 = 50.0;
const NIGHT_RANGE: f32 = 22.0;
const LIGHT_RANGE: f32 = 70.0;
const LUMENS: f32 = 4_000_000.0;

/// How far (m) strangers and dogs can see the player right now.
#[derive(Resource)]
pub struct Exposure(pub f32);

/// Whether the flashlight is on.
#[derive(Resource)]
pub struct Flashlight(pub bool);

#[derive(Component)]
struct Lamp;

fn setup_flashlight(mut commands: Commands, camera: Single<Entity, With<Player>>) {
    let lamp = commands
        .spawn((
            SpotLight {
                intensity: 0.0,
                range: 90.0,
                color: Color::srgb(1.0, 0.95, 0.85),
                inner_angle: 0.25,
                outer_angle: 0.45,
                shadow_maps_enabled: false,
                ..default()
            },
            // Slightly right and below the eye, pointing along the view (-z).
            Transform::from_xyz(0.15, -0.15, 0.0),
            Lamp,
        ))
        .id();
    commands.entity(*camera).add_child(lamp);
}

fn toggle(
    keys: Res<ButtonInput<KeyCode>>,
    mut on: ResMut<Flashlight>,
    mut lamp: Single<&mut SpotLight, With<Lamp>>,
    mut sound: ResMut<SoundQueue>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        on.0 = !on.0;
        lamp.intensity = if on.0 { LUMENS } else { 0.0 };
        sound.play(Sfx::Switch, None, 0.5);
    }
}

fn expose(
    day: Res<DayTime>,
    on: Res<Flashlight>,
    player: Single<&Player>,
    mut exposure: ResMut<Exposure>,
) {
    let h = sun_height(day.hour);
    let light = ((h + 0.1) / 0.35).clamp(0.0, 1.0);
    let mut range = NIGHT_RANGE + (DAY_RANGE - NIGHT_RANGE) * light;
    if on.0 {
        range = range.max(LIGHT_RANGE);
    }
    if player.crouched() {
        range *= 0.7;
    }
    exposure.0 = range;
}

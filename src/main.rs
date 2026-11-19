// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cars;
mod clouds;
mod daylight;
mod dogs;
mod flashlight;
mod health;
mod hud;
mod level;
mod optic;
mod player;
mod radar;
mod render_scale;
mod sound;
mod targets;
mod textures;
mod watchers;
mod weapon;

use bevy::light::{CascadeShadowConfigBuilder, DirectionalLightShadowMap};
use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "ggtestbim".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.62, 0.77, 0.93)))
        .insert_resource(GlobalAmbientLight {
            brightness: 300.0,
            ..default()
        })
        .add_plugins((
            level::LevelPlugin,
            player::PlayerPlugin,
            cars::CarsPlugin,
            watchers::WatchersPlugin,
            clouds::CloudsPlugin,
            hud::HudPlugin,
            optic::OpticPlugin,
            weapon::WeaponPlugin,
            render_scale::RenderScalePlugin,
            sound::SoundPlugin,
            daylight::DaylightPlugin,
            flashlight::FlashlightPlugin,
            dogs::DogsPlugin,
            health::HealthPlugin,
            radar::RadarPlugin,
        ))
        // Shadows only near the player (2 cascades to 50 m, 1024 map): cheap enough for an integrated GPU.
        .insert_resource(DirectionalLightShadowMap { size: 1024 })
        .add_systems(Startup, setup)
        .add_systems(Update, toggle_shadows)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn((
        DirectionalLight {
            illuminance: 8000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            // A short first cascade: up close (stairwells, rooms) one shadow texel covered many screen pixels
            // and shadow edges showed as large saw teeth; 6 m gives about 2.5x the texel density there.
            first_cascade_far_bound: 6.0,
            maximum_distance: 50.0,
            ..default()
        }
        .build(),
    ));
}

/// G switches the sun's shadows off and on (the biggest GPU cost).
fn toggle_shadows(keys: Res<ButtonInput<KeyCode>>, mut sun: Single<&mut DirectionalLight>) {
    if keys.just_pressed(KeyCode::KeyG) {
        sun.shadow_maps_enabled = !sun.shadow_maps_enabled;
    }
}

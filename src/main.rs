// Release builds on Windows run without a console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cars;
mod clouds;
mod hud;
mod level;
mod player;
mod textures;
mod watchers;

use bevy::prelude::*;

const HELP: &str = "WASD move (runs) · Shift walk · Ctrl crouch · Space jump · mouse look
Crouch + jump into windows · Click grab mouse · Esc release · R new world";

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
        ))
        .add_systems(Startup, setup)
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
    ));
    commands.spawn((
        Text::new(HELP),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
    ));
}

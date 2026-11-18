//! Compass at the top center: a strip of 15° ticks with N/E/S/W around the view direction, a marker
//! under the middle and the heading in degrees. North is -z, headings grow clockwise.

use bevy::prelude::*;

use crate::player::Player;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_compass)
            .add_systems(Update, compass);
    }
}

/// Ticks shown on each side of the heading.
const HALF: i32 = 8;
const TICK_DEG: i32 = 15;

#[derive(Component)]
struct Compass;

fn setup_compass(mut commands: Commands) {
    commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|c| {
            c.spawn((
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(18.0),
                    ..default()
                },
                TextLayout::justify(Justify::Center),
                Compass,
            ));
        });
}

fn compass(camera: Single<&Transform, With<Player>>, mut text: Single<&mut Text, With<Compass>>) {
    let f = camera.forward();
    let heading = f.x.atan2(-f.z).to_degrees().rem_euclid(360.0);
    let base = (heading / TICK_DEG as f32).round() as i32 * TICK_DEG;
    let strip: Vec<&str> = (-HALF..=HALF)
        .map(|i| match (base + i * TICK_DEG).rem_euclid(360) {
            0 => "N",
            90 => "E",
            180 => "S",
            270 => "W",
            45 | 135 | 225 | 315 => "+",
            _ => "·",
        })
        .collect();
    let marker = format!(
        "{}^{}",
        "  ".repeat(HALF as usize),
        "  ".repeat(HALF as usize)
    );
    text.0 = format!("{}\n{}\n{:03.0}°", strip.join(" "), marker, heading);
}

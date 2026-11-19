//! Top line of the screen: "~ help" on the left (~ unfolds the controls under it), the compass in the center
//! (a strip of 15 degree ticks with N/E/S/W around the view direction, a marker under the middle, the heading
//! in degrees; north is -z, headings grow clockwise) and the frame rate on the right.

use bevy::prelude::*;

use crate::player::Player;

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (setup_compass, setup_corners))
            .add_systems(Update, (compass, toggle_help));
    }
}

/// Ticks shown on each side of the heading.
const HALF: i32 = 8;
const TICK_DEG: i32 = 15;

const HELP_HINT: &str = "~ help";
const HELP: &str = "~ help
WASD move (runs) | Shift walk | Ctrl crouch | Space jump | mouse look
Crouch + jump into windows | Click grab mouse | Esc release | R new world | G shadows";

#[derive(Component)]
struct Compass;

#[derive(Component)]
struct Help;

#[derive(Component)]
struct Fps;

/// The help hint at the top left and the frame rate at the top right.
fn setup_corners(mut commands: Commands) {
    let font = TextFont {
        font_size: FontSize::Px(18.0),
        ..default()
    };
    commands.spawn((
        Text::new(HELP_HINT),
        font.clone(),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        Help,
    ));
    commands.spawn((
        Text::new(""),
        font,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            right: Val::Px(8.0),
            ..default()
        },
        Fps,
    ));
}

/// ~ (the key left of 1) shows or hides the controls under the hint.
fn toggle_help(keys: Res<ButtonInput<KeyCode>>, mut text: Single<&mut Text, With<Help>>) {
    if keys.just_pressed(KeyCode::Backquote) {
        text.0 = if text.0 == HELP_HINT { HELP } else { HELP_HINT }.into();
    }
}

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

fn compass(
    time: Res<Time>,
    mut fps: Local<f32>,
    camera: Single<&Transform, With<Player>>,
    mut text: Single<&mut Text, With<Compass>>,
    mut fps_text: Single<&mut Text, (With<Fps>, Without<Compass>)>,
) {
    // Frames per second, smoothed over about half a second.
    let dt = time.delta_secs().max(1e-4);
    *fps += (1.0 / dt - *fps) * (dt * 2.0).min(1.0);
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
            _ => ".",
        })
        .collect();
    let marker = format!(
        "{}^{}",
        "  ".repeat(HALF as usize),
        "  ".repeat(HALF as usize)
    );
    text.0 = format!("{}\n{}\n{:03.0}", strip.join(" "), marker, heading);
    fps_text.0 = format!("{:.0} fps", *fps);
}

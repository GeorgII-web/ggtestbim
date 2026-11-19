//! Top line of the screen: "~ help" on the left (~ unfolds the controls under it), the compass in the center
//! (a strip of 15 degree ticks with N/E/S/W around the view direction, a marker under the middle, the heading
//! in degrees right of the strip; north is -z, headings grow clockwise) and the frame rate on the right.

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
Ctrl + Space high jump (ledges, windows) | Right click optic, wheel zoom | Click grab mouse | Esc release
Left click shoot | R reload | N new world | G shadows | F render scale";

#[derive(Component)]
struct Compass;

/// The compass text after "S": the root `Text` holds the strip up to S, this span the red S, the next one
/// the rest. Without an S in view the root holds everything and the spans are empty.
#[derive(Component)]
struct CompassSouth;

#[derive(Component)]
struct CompassRest;

const SOUTH_COLOR: Color = Color::srgb(0.95, 0.2, 0.15);

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
                // Left-justified so the marker line lines up with the strip above it.
                TextLayout::justify(Justify::Left),
                Compass,
                children![
                    (TextSpan::new(""), TextColor(SOUTH_COLOR), CompassSouth),
                    (TextSpan::new(""), CompassRest),
                ],
            ));
        });
}

#[allow(clippy::too_many_arguments)]
fn compass(
    time: Res<Time>,
    mut fps: Local<f32>,
    camera: Single<&Transform, With<Player>>,
    mut text: Single<&mut Text, With<Compass>>,
    mut south: Single<&mut TextSpan, (With<CompassSouth>, Without<CompassRest>)>,
    mut rest: Single<&mut TextSpan, (With<CompassRest>, Without<CompassSouth>)>,
    mut fps_text: Single<&mut Text, (With<Fps>, Without<Compass>)>,
    scale: Option<Res<crate::render_scale::RenderScale>>,
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
    // Five spaces in front balance the "  297" behind, so the strip stays centered on the screen.
    let all = format!(
        "     {}  {:03.0}\n     {}",
        strip.join(" "),
        heading,
        marker
    );
    // The strip has at most one S (17 ticks of 15 degrees cover 240).
    let (head, s, tail) = match all.find('S') {
        Some(i) => (&all[..i], "S", &all[i + 1..]),
        None => (all.as_str(), "", ""),
    };
    text.0 = head.into();
    south.0 = s.into();
    rest.0 = tail.into();
    // The render scale (F) next to the frame rate.
    let pct = scale.map_or(100.0, |s| s.scale() * 100.0);
    fps_text.0 = format!("{pct:.0}%  {:.0} fps", *fps);
}

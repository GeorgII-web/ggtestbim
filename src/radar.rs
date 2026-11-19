//! Radar at the bottom left, above the health bar: a round screen turning with the view (up is where the
//! player looks), the player in the middle, dogs as red dots and strangers as pale dots within `RANGE`.

use bevy::prelude::*;

use crate::player::Player;
use crate::targets::{Blip, Killed, Kind};

pub struct RadarPlugin;

impl Plugin for RadarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_radar)
            .add_systems(Update, radar.after(crate::player::PlayerSet));
    }
}

const RANGE: f32 = 60.0;
const SIZE: f32 = 150.0;
const DOT: f32 = 7.0;
/// Dot nodes kept for blips; more blips than this are not shown.
const DOTS: usize = 32;

#[derive(Component)]
struct Dot(usize);

fn setup_radar(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(36.0),
                width: Val::Px(SIZE),
                height: Val::Px(SIZE),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::MAX,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.08, 0.03, 0.55)),
            BorderColor::all(Color::srgba(0.4, 0.9, 0.5, 0.6)),
        ))
        .with_children(|c| {
            // Range ring at half distance and the player marker.
            c.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(SIZE / 4.0 - 1.0),
                    top: Val::Px(SIZE / 4.0 - 1.0),
                    width: Val::Px(SIZE / 2.0),
                    height: Val::Px(SIZE / 2.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::MAX,
                    ..default()
                },
                BorderColor::all(Color::srgba(0.4, 0.9, 0.5, 0.25)),
            ));
            c.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(SIZE / 2.0 - 4.0),
                    top: Val::Px(SIZE / 2.0 - 4.0),
                    width: Val::Px(6.0),
                    height: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.5, 1.0, 0.6)),
            ));
            for i in 0..DOTS {
                c.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Px(DOT),
                        height: Val::Px(DOT),
                        border_radius: BorderRadius::MAX,
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                    Visibility::Hidden,
                    Dot(i),
                ));
            }
        });
}

fn radar(
    camera: Single<&Transform, With<Player>>,
    blips: Query<(&Blip, &GlobalTransform), Without<Killed>>,
    mut dots: Query<(&Dot, &mut Node, &mut BackgroundColor, &mut Visibility)>,
) {
    let eye = camera.translation;
    let f = camera.forward();
    // Screen up = the view direction, screen right = its right.
    let (fwd, right) = (
        Vec2::new(f.x, f.z).normalize_or_zero(),
        Vec2::new(-f.z, f.x).normalize_or_zero(),
    );
    let mut shown: Vec<(Vec2, Kind)> = blips
        .iter()
        .filter_map(|(b, tf)| {
            let d = tf.translation() - eye;
            let flat = Vec2::new(d.x, d.z);
            (flat.length() < RANGE).then(|| (Vec2::new(flat.dot(right), -flat.dot(fwd)), b.0))
        })
        .collect();
    shown.truncate(DOTS);
    let scale = (SIZE / 2.0 - DOT) / RANGE;
    for (dot, mut node, mut color, mut vis) in &mut dots {
        match shown.get(dot.0) {
            Some((p, kind)) => {
                node.left = Val::Px(SIZE / 2.0 + p.x * scale - DOT / 2.0 - 1.0);
                node.top = Val::Px(SIZE / 2.0 + p.y * scale - DOT / 2.0 - 1.0);
                color.0 = match kind {
                    Kind::Dog => Color::srgb(1.0, 0.25, 0.2),
                    Kind::Stranger => Color::srgb(0.9, 0.9, 0.8),
                };
                *vis = Visibility::Inherited;
            }
            None => *vis = Visibility::Hidden,
        }
    }
}

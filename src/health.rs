//! Health: a bar at the bottom left (green, yellow, red), a red flash on every hit, regeneration
//! (`REGEN` per second after `REGEN_AFTER` seconds without damage) and death: at 0 the screen goes dark red
//! with "You died" for `DEAD_TIME` seconds (no control meanwhile), then the player respawns at the spawn point with full health.

use bevy::prelude::*;

use crate::player::{MAX_HEALTH, Player};
use crate::sound::{Sfx, SoundQueue};

pub struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_health)
            .add_systems(Update, health.after(crate::player::PlayerSet));
    }
}

const REGEN: f32 = 2.0;
const REGEN_AFTER: f32 = 6.0;
const DEAD_TIME: f32 = 3.0;
const BAR_W: f32 = 180.0;

#[derive(Component)]
struct Fill;

#[derive(Component)]
struct HealthText;

/// Full-screen red: a flash on damage, solid while dead.
#[derive(Component)]
struct Overlay;

#[derive(Component)]
struct DeadText;

fn setup_health(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(12.0),
                bottom: Val::Px(12.0),
                width: Val::Px(BAR_W),
                height: Val::Px(14.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
        ))
        .with_children(|c| {
            c.spawn((
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.3, 0.8, 0.3)),
                Fill,
            ));
        });
    commands.spawn((
        Text::new(""),
        TextFont {
            font_size: FontSize::Px(16.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(BAR_W + 22.0),
            bottom: Val::Px(9.0),
            ..default()
        },
        HealthText,
    ));
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::NONE),
        GlobalZIndex(5),
        Overlay,
        children![(
            Text::new(""),
            TextFont {
                font_size: FontSize::Px(48.0),
                ..default()
            },
            TextColor(Color::srgb(0.9, 0.85, 0.8)),
            DeadText,
        )],
    ));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn health(
    time: Res<Time>,
    mut player: Single<&mut Player>,
    mut fill: Single<(&mut Node, &mut BackgroundColor), With<Fill>>,
    mut text: Single<&mut Text, (With<HealthText>, Without<DeadText>)>,
    mut overlay: Single<&mut BackgroundColor, (With<Overlay>, Without<Fill>)>,
    mut dead_text: Single<&mut Text, With<DeadText>>,
    mut sound: ResMut<SoundQueue>,
    mut state: Local<(f32, f32, Option<f32>)>,
) {
    let dt = time.delta_secs();
    // (last health, flash, seconds dead)
    let (last, flash, dead) = &mut *state;
    if *last == 0.0 && dead.is_none() {
        *last = player.health;
    }
    if let Some(t) = dead {
        *t += dt;
        overlay.0 = Color::srgba(0.25, 0.0, 0.0, 0.85);
        dead_text.0 = "You died".into();
        if *t > DEAD_TIME {
            player.respawn();
            *dead = None;
            *last = player.health;
            dead_text.0 = String::new();
            overlay.0 = Color::NONE;
        }
        return;
    }
    if player.health < *last {
        *flash = ((*last - player.health) / 30.0).clamp(0.25, 0.7);
        sound.play(Sfx::Hurt, None, 0.7);
    }
    if player.health <= 0.0 {
        *dead = Some(0.0);
        player.dead = true;
        return;
    }
    player.since_hurt += dt;
    if player.since_hurt > REGEN_AFTER {
        player.health = (player.health + REGEN * dt).min(MAX_HEALTH);
    }
    *last = player.health;
    *flash = (*flash - dt * 1.5).max(0.0);
    overlay.0 = Color::srgba(0.7, 0.0, 0.0, *flash * 0.5);

    let k = player.health / MAX_HEALTH;
    let (node, color) = &mut *fill;
    node.width = Val::Percent(k * 100.0);
    color.0 = if k > 0.6 {
        Color::srgb(0.3, 0.8, 0.3)
    } else if k > 0.3 {
        Color::srgb(0.9, 0.75, 0.2)
    } else {
        Color::srgb(0.9, 0.2, 0.15)
    };
    text.0 = format!("{:.0}", player.health);
}

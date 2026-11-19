//! Rifle-scope optic: the right mouse button puts it up (10x) and takes it down again; the mouse wheel zooms
//! between `MIN_ZOOM` and `MAX_ZOOM` while it is up. Zoom narrows the camera's field of view and slows the
//! mouse look by the same factor. The view is a full-screen overlay: black around a round lens with a dark
//! rim, a thin crosshair with thick posts from the edge and mil dots, and the zoom under the lens.

use bevy::asset::RenderAssetUsages;
use bevy::input::mouse::{AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::player::{FOV, Player};

pub struct OpticPlugin;

impl Plugin for OpticPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Optic {
            on: false,
            zoom: 1.0,
        })
        .add_systems(Startup, setup_optic)
        .add_systems(Update, optic);
    }
}

const START_ZOOM: f32 = 10.0;
const MIN_ZOOM: f32 = 2.0;
const MAX_ZOOM: f32 = 30.0;
/// Zoom factor per wheel notch (a touchpad's pixels count as `PIXELS_PER_NOTCH` per notch).
const ZOOM_STEP: f32 = 1.15;
const PIXELS_PER_NOTCH: f32 = 40.0;
/// Size of the generated reticle texture in pixels; the lens radius as a share of it.
const SIZE: usize = 1024;
const LENS: f32 = 0.48;

/// Whether the optic is up and its zoom (1 when down); the player's look divides the mouse by `zoom`.
#[derive(Resource)]
pub struct Optic {
    pub on: bool,
    pub zoom: f32,
}

#[derive(Component)]
struct OpticView;

#[derive(Component)]
struct ZoomLabel;

fn setup_optic(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let reticle = images.add(reticle());
    let black = Color::BLACK;
    // Under the HUD (compass, help, fps): black side bars, the square reticle in the middle.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            GlobalZIndex(-1),
            Visibility::Hidden,
            OpticView,
        ))
        .with_children(|c| {
            c.spawn((
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                BackgroundColor(black),
            ));
            c.spawn((
                Node {
                    height: Val::Percent(100.0),
                    aspect_ratio: Some(1.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::End,
                    padding: UiRect::bottom(Val::Percent(6.0)),
                    ..default()
                },
                ImageNode::new(reticle),
            ))
            .with_children(|c| {
                c.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: FontSize::Px(18.0),
                        ..default()
                    },
                    TextColor(Color::srgb(0.8, 0.8, 0.75)),
                    ZoomLabel,
                ));
            });
            c.spawn((
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                BackgroundColor(black),
            ));
        });
}

fn optic(
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut state: ResMut<Optic>,
    mut projection: Single<&mut Projection, With<Player>>,
    mut view: Single<&mut Visibility, With<OpticView>>,
    mut label: Single<&mut Text, With<ZoomLabel>>,
) {
    if mouse.just_pressed(MouseButton::Right) {
        state.on = !state.on;
        state.zoom = if state.on { START_ZOOM } else { 1.0 };
    }
    if state.on && scroll.delta.y != 0.0 {
        let notches = match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / PIXELS_PER_NOTCH,
        };
        state.zoom = (state.zoom * ZOOM_STEP.powf(notches)).clamp(MIN_ZOOM, MAX_ZOOM);
    }
    if !state.is_changed() {
        return;
    }
    if let Projection::Perspective(p) = projection.as_mut() {
        p.fov = 2.0 * ((FOV / 2.0).tan() / state.zoom).atan();
    }
    **view = if state.on {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    label.0 = format!("{:.1}x", state.zoom);
}

/// Square RGBA reticle: opaque black outside the lens, a dark rim fading inward, a thin crosshair with a gap
/// in the middle, thick posts from the rim to a quarter of the radius, and a dot every tenth of the radius.
fn reticle() -> Image {
    let r = LENS * SIZE as f32;
    let c = SIZE as f32 / 2.0;
    let mut data = Vec::with_capacity(SIZE * SIZE * 4);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let (ax, ay) = (dx.abs(), dy.abs());
            // Outside the lens, with 1.5 px of antialiasing at the edge.
            let mut alpha = ((d - r) / 1.5 + 0.5).clamp(0.0, 1.0);
            // Rim shade over the last 12 % of the radius.
            let rim = ((d / r - 0.88) / 0.12).clamp(0.0, 1.0);
            alpha = alpha.max(rim * rim * 0.85);
            let line = |across: f32, along: f32| {
                let thin = across < 1.0 && along > 0.02 * r;
                let post = across < 5.0 && along > 0.25 * r;
                let dot = across < 3.0 && {
                    let k = along / (0.1 * r);
                    (k - k.round()).abs() * 0.1 * r < 3.0 && k.round() >= 1.0 && k.round() <= 2.0
                };
                thin || post || dot
            };
            if d < r && (line(ay, ax) || line(ax, ay)) {
                alpha = 1.0;
            }
            data.extend_from_slice(&[0, 0, 0, (alpha * 255.0) as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: SIZE as u32,
            height: SIZE as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

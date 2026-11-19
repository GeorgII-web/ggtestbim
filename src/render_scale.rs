//! Render resolution: the 3D camera draws into an image of a fixed height (720 by default, width from the
//! window's aspect), and a 2D camera shows it stretched over the whole window (linear filtering) under the
//! UI. The integrated GPU pays per pixel, so a big or full-screen window costs no more than HD; the HUD and
//! the optic stay sharp because the UI renders at full size on the 2D camera. F cycles `HEIGHTS` (0 is the
//! window's own resolution); never above the window's.

use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureFormat};

use crate::player::Player;

pub struct RenderScalePlugin;

impl Plugin for RenderScalePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, setup_scale)
            .add_systems(Update, rescale);
    }
}

const HEIGHTS: [u32; 4] = [720, 900, 1080, 0];
const START: usize = 0;

/// The current resolution (index into `HEIGHTS`), the scene image and its size in pixels.
#[derive(Resource)]
pub struct RenderScale {
    index: usize,
    image: Handle<Image>,
    size: UVec2,
}

impl RenderScale {
    /// Render height in pixels.
    pub fn height(&self) -> u32 {
        self.size.y
    }
}

/// Image size for render height `height` (0: the window's): the window's aspect, never larger than it.
fn target_size(window: &Window, height: u32) -> UVec2 {
    let full = UVec2::new(window.physical_width(), window.physical_height()).max(UVec2::ONE);
    if height == 0 || height >= full.y {
        return full;
    }
    let width = (full.x as f32 * height as f32 / full.y as f32).round() as u32;
    UVec2::new(width.max(1), height)
}

fn setup_scale(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window>,
    camera: Single<Entity, With<Player>>,
) {
    let size = target_size(&window, HEIGHTS[START]);
    let image = images.add(Image::new_target_texture(
        size.x,
        size.y,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands
        .entity(*camera)
        .insert(RenderTarget::Image(ImageRenderTarget {
            handle: image.clone(),
            scale_factor: 1.0,
        }));
    // The window camera: draws the scene image and all UI on top of it.
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            ..default()
        },
        Msaa::Off,
        IsDefaultUiCamera,
    ));
    commands.spawn((
        ImageNode::new(image.clone()),
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            ..default()
        },
        GlobalZIndex(i32::MIN),
    ));
    commands.insert_resource(RenderScale {
        index: START,
        image,
        size,
    });
}

/// F cycles the resolution; the image follows the window size.
fn rescale(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    mut state: ResMut<RenderScale>,
    mut images: ResMut<Assets<Image>>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        state.index = (state.index + 1) % HEIGHTS.len();
    }
    let size = target_size(&window, HEIGHTS[state.index]);
    if size == state.size {
        return;
    }
    state.size = size;
    if let Some(mut image) = images.get_mut(&state.image) {
        image.resize(Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        });
    }
}

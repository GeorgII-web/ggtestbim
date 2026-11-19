//! Render scale: the 3D camera draws into an image of `scale` x the window's physical size, and a 2D camera
//! shows it stretched over the whole window (linear filtering) under the UI. The integrated GPU pays per
//! pixel, so 70 % (about half the pixels) nearly doubles the frame budget; the HUD and the optic stay sharp
//! because the UI renders at full size on the 2D camera. F cycles the scale through `SCALES`.

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

const SCALES: [f32; 4] = [1.0, 0.85, 0.7, 0.55];
const START: usize = 2;

/// The current scale (index into `SCALES`), the scene image and its size in pixels.
#[derive(Resource)]
pub struct RenderScale {
    index: usize,
    image: Handle<Image>,
    size: UVec2,
}

impl RenderScale {
    pub fn scale(&self) -> f32 {
        SCALES[self.index]
    }
}

fn target_size(window: &Window, scale: f32) -> UVec2 {
    (Vec2::new(
        window.physical_width() as f32,
        window.physical_height() as f32,
    ) * scale)
        .round()
        .as_uvec2()
        .max(UVec2::ONE)
}

fn setup_scale(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    window: Single<&Window>,
    camera: Single<Entity, With<Player>>,
) {
    let size = target_size(&window, SCALES[START]);
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

/// F cycles the scale; the image follows the window size.
fn rescale(
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window>,
    mut state: ResMut<RenderScale>,
    mut images: ResMut<Assets<Image>>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        state.index = (state.index + 1) % SCALES.len();
    }
    let size = target_size(&window, state.scale());
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

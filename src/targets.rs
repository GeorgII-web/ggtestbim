//! What the weapon can hit and the radar shows: strangers and dogs carry a `Shootable` body box and a
//! `Blip`; a hit inserts `Killed`, and the owning module plays the death.

use bevy::prelude::*;

use crate::level::Aabb;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Stranger,
    Dog,
}

/// A body bullets can hit: a box `half` wide (x and z) and `height` tall above the feet position.
#[derive(Component)]
pub struct Shootable {
    pub kind: Kind,
    pub half: f32,
    pub height: f32,
}

impl Shootable {
    pub fn aabb(&self, feet: Vec3) -> Aabb {
        Aabb {
            min: feet - Vec3::new(self.half, 0.0, self.half),
            max: feet + Vec3::new(self.half, self.height, self.half),
        }
    }
}

/// Shot: seconds since the hit and the rotation when hit; the owner's behavior stops and its death plays.
#[derive(Component, Default)]
pub struct Killed {
    pub t: f32,
    pub from: Option<Quat>,
}

/// Shown on the radar as this kind.
#[derive(Component)]
pub struct Blip(pub Kind);

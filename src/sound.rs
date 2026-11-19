//! Sound, all synthesized at startup (no audio files): footsteps, pistol shot, dry fire, reload, bullet
//! hits, a figure's death cry and car engines. Other modules queue one-shot sounds in `SoundQueue` (with a
//! world position for spatial ones); this module plays them, steps the player's feet by distance walked,
//! and gives the cars near the player a looping engine whose pitch follows the car's speed.
//! Spatial sounds use `SPATIAL_SCALE`: rodio's gain falls as 1 / (scaled distance)^2, so at 0.12 a sound is
//! full up to about 8 m and a ninth as loud at 25 m.

use std::num::NonZero;
use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{
    AddAudioSource, ChannelCount, Decodable, PlaybackMode, Sample, SampleRate, Source,
    SpatialAudioSink, SpatialListener, SpatialScale, Volume,
};
use bevy::prelude::*;

use crate::cars::Car;
use crate::level::Rng;
use crate::player::{Motion, Player};

pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<Synth>()
            .insert_resource(SoundQueue(Vec::new()))
            .add_systems(Startup, setup_sounds)
            .add_systems(PostStartup, add_listener)
            .add_systems(Update, (footsteps, engines))
            .add_systems(PostUpdate, play_queue);
    }
}

const RATE: u32 = 44_100;
const SPATIAL_SCALE: f32 = 0.12;
/// Stride length by gait: a step sound every this many meters walked.
const STRIDE_WALK: f32 = 0.8;
const STRIDE_RUN: f32 = 1.9;
/// Cars within `ENGINE_ON` get an engine sound, beyond `ENGINE_OFF` it stops.
const ENGINE_ON: f32 = 40.0;
const ENGINE_OFF: f32 = 50.0;

/// Mono samples at `RATE`, played by rodio through `SynthDecoder`.
#[derive(Asset, TypePath, Clone)]
pub struct Synth {
    samples: Arc<[f32]>,
}

pub struct SynthDecoder {
    samples: Arc<[f32]>,
    at: usize,
}

impl Iterator for SynthDecoder {
    type Item = Sample;
    fn next(&mut self) -> Option<Sample> {
        let s = self.samples.get(self.at).copied();
        self.at += 1;
        s
    }
}

impl Source for SynthDecoder {
    fn current_span_len(&self) -> Option<usize> {
        Some(self.samples.len().saturating_sub(self.at))
    }
    fn channels(&self) -> ChannelCount {
        NonZero::new(1).expect("one channel")
    }
    fn sample_rate(&self) -> SampleRate {
        NonZero::new(RATE).expect("rate")
    }
    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(
            self.samples.len() as f32 / RATE as f32,
        ))
    }
}

impl Decodable for Synth {
    type Decoder = SynthDecoder;
    fn decoder(&self) -> SynthDecoder {
        SynthDecoder {
            samples: self.samples.clone(),
            at: 0,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Sfx {
    Step,
    Shot,
    DryFire,
    Reload,
    Hit,
    Kill,
    Bark,
    Paw,
    Bite,
    Yelp,
    Hurt,
    Switch,
}

/// One-shot sounds to play this frame: what, where (None: at the listener, not spatial), volume.
#[derive(Resource)]
pub struct SoundQueue(pub Vec<(Sfx, Option<Vec3>, f32)>);

impl SoundQueue {
    pub fn play(&mut self, sfx: Sfx, at: Option<Vec3>, volume: f32) {
        self.0.push((sfx, at, volume));
    }
}

#[derive(Resource)]
struct Sounds {
    steps: Vec<Handle<Synth>>,
    hits: Vec<Handle<Synth>>,
    shot: Handle<Synth>,
    dry: Handle<Synth>,
    reload: Handle<Synth>,
    kill: Handle<Synth>,
    barks: Vec<Handle<Synth>>,
    paws: Vec<Handle<Synth>>,
    bite: Handle<Synth>,
    yelp: Handle<Synth>,
    hurt: Handle<Synth>,
    switch: Handle<Synth>,
    engine: Handle<Synth>,
    rng: Rng,
}

/// The engine sound child of a car.
#[derive(Component)]
struct Engine;

fn setup_sounds(mut commands: Commands, mut synths: ResMut<Assets<Synth>>) {
    let mut rng = Rng(0xA0D1_0F00);
    let mut add = |s: Vec<f32>| {
        synths.add(Synth {
            samples: normalize(s, 0.9).into(),
        })
    };
    let steps = (0..4).map(|i| add(step(i))).collect();
    let hits = (0..3).map(|i| add(hit(i))).collect();
    let sounds = Sounds {
        steps,
        hits,
        shot: add(shot()),
        dry: add(dry_fire()),
        reload: add(reload()),
        kill: add(kill()),
        barks: (0..3).map(|i| add(bark(i))).collect(),
        paws: (0..3).map(|i| add(paw(i))).collect(),
        bite: add(bite()),
        yelp: add(yelp()),
        hurt: add(hurt()),
        switch: add(switch()),
        engine: add(engine()),
        rng: Rng(rng.next()),
    };
    commands.insert_resource(sounds);
}

fn add_listener(mut commands: Commands, camera: Single<Entity, With<Player>>) {
    commands.entity(*camera).insert(SpatialListener::new(0.3));
}

fn play_queue(mut commands: Commands, mut queue: ResMut<SoundQueue>, mut sounds: ResMut<Sounds>) {
    let sounds = &mut *sounds;
    for (sfx, at, volume) in std::mem::take(&mut queue.0) {
        let rng = &mut sounds.rng;
        let pick =
            |v: &Vec<Handle<Synth>>, rng: &mut Rng| v[rng.below(v.len() as u32) as usize].clone();
        let handle = match sfx {
            Sfx::Step => pick(&sounds.steps, rng),
            Sfx::Hit => pick(&sounds.hits, rng),
            Sfx::Shot => sounds.shot.clone(),
            Sfx::DryFire => sounds.dry.clone(),
            Sfx::Reload => sounds.reload.clone(),
            Sfx::Kill => sounds.kill.clone(),
            Sfx::Bark => pick(&sounds.barks, rng),
            Sfx::Paw => pick(&sounds.paws, rng),
            Sfx::Bite => sounds.bite.clone(),
            Sfx::Yelp => sounds.yelp.clone(),
            Sfx::Hurt => sounds.hurt.clone(),
            Sfx::Switch => sounds.switch.clone(),
        };
        // A little pitch variety keeps repeats from sounding mechanical.
        let speed = sounds.rng.range(0.94, 1.06);
        let settings = PlaybackSettings::DESPAWN
            .with_volume(Volume::Linear(volume))
            .with_speed(speed)
            .with_spatial(at.is_some())
            .with_spatial_scale(SpatialScale::new(SPATIAL_SCALE));
        commands.spawn((
            AudioPlayer(handle),
            settings,
            Transform::from_translation(at.unwrap_or(Vec3::ZERO)),
        ));
    }
}

/// A step sound every stride of ground travel (longer strides when running, quiet when crouched), and a
/// heavier one on landing.
fn footsteps(
    time: Res<Time>,
    player: Single<&Player>,
    mut queue: ResMut<SoundQueue>,
    mut walked: Local<f32>,
    mut last: Local<Option<Motion>>,
) {
    let motion = player.motion();
    if *last == Some(Motion::Air) && motion != Motion::Air {
        queue.play(Sfx::Step, None, 0.3);
        *walked = 0.0;
    }
    *last = Some(motion);
    let (stride, volume) = match motion {
        Motion::Walk if player.crouched() => (STRIDE_WALK, 0.04),
        Motion::Walk => (STRIDE_WALK, 0.1),
        Motion::Run => (STRIDE_RUN, 0.18),
        Motion::Still | Motion::Air => return,
    };
    *walked += player.ground_speed() * time.delta_secs();
    if *walked >= stride {
        *walked -= stride;
        queue.play(Sfx::Step, None, volume);
    }
}

/// Engines on for cars near the player, off for those far away; pitch by speed.
fn engines(
    mut commands: Commands,
    sounds: Res<Sounds>,
    camera: Single<&Transform, With<Player>>,
    cars: Query<(Entity, &Car, &Transform, Option<&Children>)>,
    mut engines: Query<Option<&mut SpatialAudioSink>, With<Engine>>,
) {
    let eye = camera.translation;
    for (e, car, tf, children) in &cars {
        let dist = tf.translation.distance(eye);
        let engine = children.and_then(|c| c.iter().find(|c| engines.contains(*c)));
        match engine {
            None if dist < ENGINE_ON => {
                let child = commands
                    .spawn((
                        AudioPlayer(sounds.engine.clone()),
                        PlaybackSettings {
                            mode: PlaybackMode::Loop,
                            volume: Volume::Linear(0.35),
                            spatial: true,
                            spatial_scale: Some(SpatialScale::new(SPATIAL_SCALE)),
                            ..default()
                        },
                        Transform::default(),
                        Engine,
                    ))
                    .id();
                commands.entity(e).add_child(child);
            }
            Some(c) if dist > ENGINE_OFF => commands.entity(c).despawn(),
            Some(c) => {
                if let Ok(Some(sink)) = engines.get_mut(c) {
                    sink.set_speed(0.7 + car.speed / 14.0);
                }
            }
            None => {}
        }
    }
}

// ---- synthesis -------------------------------------------------------------------------------------

fn len(secs: f32) -> usize {
    (secs * RATE as f32) as usize
}

/// One-pole low-pass coefficient for cutoff `hz`.
fn lp(hz: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * hz / RATE as f32).exp()
}

fn noise(rng: &mut Rng) -> f32 {
    rng.range(-1.0, 1.0)
}

fn normalize(mut s: Vec<f32>, peak: f32) -> Vec<f32> {
    let max = s.iter().fold(0.0f32, |m, x| m.max(x.abs())).max(1e-6);
    for x in &mut s {
        *x *= peak / max;
    }
    s
}

/// Footstep: a soft low-passed scuff and a short low thump; variants differ in tone.
fn step(variant: u64) -> Vec<f32> {
    let mut rng = Rng(0x57E9 + variant);
    let n = len(0.14);
    let a = lp(500.0 + variant as f32 * 120.0);
    let mut y = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            y += a * (noise(&mut rng) - y);
            let thump = (std::f32::consts::TAU * (70.0 + variant as f32 * 8.0) * t).sin();
            y * (-t * 35.0).exp() * 2.5 + thump * (-t * 60.0).exp() * 0.6
        })
        .collect()
}

/// Pistol shot: a sharp crack, a noisy blast and a falling low boom.
fn shot() -> Vec<f32> {
    let mut rng = Rng(0x5A07);
    let n = len(0.6);
    let a = lp(3000.0);
    let (mut y, mut phase) = (0.0, 0.0f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let w = noise(&mut rng);
            y += a * (w - y);
            let crack = if t < 0.004 { w } else { 0.0 };
            phase += std::f32::consts::TAU * (90.0 - 50.0 * (t / 0.6)) / RATE as f32;
            crack * 1.5 + y * (-t * 18.0).exp() * 1.2 + phase.sin() * (-t * 9.0).exp() * 0.8
        })
        .collect()
}

/// Empty pistol: a dry metal click.
fn dry_fire() -> Vec<f32> {
    let mut rng = Rng(0xD7F1);
    click(&mut rng, len(0.06), 0.0, 2600.0)
}

/// A click at `start` seconds in a buffer of `n` samples: a noise tick and a ringing tone.
fn click(rng: &mut Rng, n: usize, start: f32, hz: f32) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32 - start;
            if t < 0.0 {
                return 0.0;
            }
            let tick = if t < 0.002 { noise(rng) } else { 0.0 };
            tick + (std::f32::consts::TAU * hz * t).sin() * (-t * 120.0).exp() * 0.5
        })
        .collect()
}

/// Reload, timed to the animation (1.3 s): magazine out, magazine in, slide back, slide forward.
fn reload() -> Vec<f32> {
    let mut rng = Rng(0x2E10);
    let n = len(1.3);
    let mut out = vec![0.0; n];
    for (at, hz, gain) in [
        (0.25, 1500.0, 0.6),
        (0.9, 1200.0, 0.9),
        (1.0, 1900.0, 0.5),
        (1.13, 2300.0, 1.0),
    ] {
        for (o, c) in out.iter_mut().zip(click(&mut rng, n, at, hz)) {
            *o += c * gain;
        }
    }
    out
}

/// Bullet hit: a dull thwack; one variant adds a ricochet whine.
fn hit(variant: u64) -> Vec<f32> {
    let mut rng = Rng(0x417 + variant);
    let n = len(if variant == 2 { 0.45 } else { 0.2 });
    let (a, b) = (lp(2500.0), lp(400.0));
    let (mut y1, mut y2, mut phase) = (0.0, 0.0, 0.0f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let w = noise(&mut rng);
            y1 += a * (w - y1);
            y2 += b * (y1 - y2);
            // Band-passed noise: the low-pass minus a lower one.
            let thwack = (y1 - y2) * (-t * 40.0).exp() * 3.0;
            let whine = if variant == 2 {
                phase += std::f32::consts::TAU * (3200.0 - 2400.0 * t / 0.45) / RATE as f32;
                phase.sin() * (-t * 6.0).exp() * 0.25 * (t * 40.0).min(1.0)
            } else {
                0.0
            };
            thwack + whine
        })
        .collect()
}

/// A figure's death: two dissonant tones sliding down with a wavering vibrato over a breath of noise.
fn kill() -> Vec<f32> {
    let mut rng = Rng(0xC1DE);
    let n = len(1.6);
    let a = lp(900.0);
    let (mut y, mut p1, mut p2) = (0.0, 0.0f32, 0.0f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let slide = 1.0 - 0.5 * (t / 1.6);
            let vib = 1.0 + 0.03 * (std::f32::consts::TAU * 6.0 * t).sin();
            p1 += std::f32::consts::TAU * 220.0 * slide * vib / RATE as f32;
            p2 += std::f32::consts::TAU * 233.0 * slide * vib / RATE as f32;
            y += a * (noise(&mut rng) - y);
            let env = (t / 0.05).min(1.0) * (-t * 1.8).exp();
            (p1.sin() + p2.sin() * 0.8 + y * 1.5) * env
        })
        .collect()
}

/// A dog's bark: a short harsh tone falling in pitch, rough with noise; variants differ in pitch.
fn bark(variant: u64) -> Vec<f32> {
    let mut rng = Rng(0xBA4C + variant);
    let n = len(0.22);
    let base = 330.0 + variant as f32 * 60.0;
    let mut phase = 0.0f32;
    let a = lp(2000.0);
    let mut y = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            phase += std::f32::consts::TAU * base * (1.0 - 0.45 * t / 0.22) / RATE as f32;
            // A clipped (square-ish) tone is harsher than a sine.
            let tone = (phase.sin() * 3.0).clamp(-1.0, 1.0) + (phase * 2.0).sin() * 0.4;
            y += a * (noise(&mut rng) - y);
            let env = (t / 0.01).min(1.0) * (-t * 12.0).exp();
            (tone * 0.7 + y * 1.2) * env
        })
        .collect()
}

/// A dog's paw on the ground: a short soft pat with a tiny scratch of claws; variants differ in tone.
fn paw(variant: u64) -> Vec<f32> {
    let mut rng = Rng(0x9A3 + variant);
    let n = len(0.06);
    let a = lp(900.0 + variant as f32 * 250.0);
    let mut y = 0.0;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            let w = noise(&mut rng);
            y += a * (w - y);
            let claw = if t < 0.004 { w * 0.3 } else { 0.0 };
            y * (-t * 70.0).exp() * 2.0 + claw
        })
        .collect()
}

/// A bite: a low snarl and a snap of teeth.
fn bite() -> Vec<f32> {
    let mut rng = Rng(0xB17E);
    let n = len(0.35);
    let a = lp(600.0);
    let (mut y, mut phase) = (0.0, 0.0f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            phase += std::f32::consts::TAU * (120.0 + 30.0 * (t * 40.0).sin()) / RATE as f32;
            y += a * (noise(&mut rng) - y);
            let snarl = (phase.sin() * 0.6 + y * 2.0) * (-t * 6.0).exp();
            let snap = if (0.12..0.125).contains(&t) {
                noise(&mut rng) * 2.0
            } else {
                0.0
            };
            snarl + snap
        })
        .collect()
}

/// A dog's death yelp: a high whine sliding down.
fn yelp() -> Vec<f32> {
    let n = len(0.5);
    let mut phase = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            phase += std::f32::consts::TAU * (1100.0 - 700.0 * t / 0.5) / RATE as f32;
            let env = (t / 0.01).min(1.0) * (-t * 5.0).exp();
            ((phase.sin() * 2.0).clamp(-1.0, 1.0) * 0.7 + (phase * 0.5).sin() * 0.3) * env
        })
        .collect()
}

/// The player hurt: a short low grunt.
fn hurt() -> Vec<f32> {
    let mut rng = Rng(0x4C27);
    let n = len(0.25);
    let a = lp(700.0);
    let (mut y, mut phase) = (0.0, 0.0f32);
    (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            phase += std::f32::consts::TAU * (150.0 - 50.0 * t / 0.25) / RATE as f32;
            y += a * (noise(&mut rng) - y);
            let env = (t / 0.02).min(1.0) * (-t * 9.0).exp();
            ((phase.sin() * 1.5).clamp(-1.0, 1.0) * 0.7 + y * 1.0) * env
        })
        .collect()
}

/// The flashlight switch: a small plastic click.
fn switch() -> Vec<f32> {
    let mut rng = Rng(0x5117);
    click(&mut rng, len(0.04), 0.0, 3500.0)
}

/// Engine loop (1 s, seamless): a 50 Hz hum with harmonics (whole cycles per loop) and a low rumble,
/// cross-faded at the seam.
fn engine() -> Vec<f32> {
    let mut rng = Rng(0xE061);
    let n = len(1.0);
    let a = lp(180.0);
    let mut y = 0.0;
    let mut s: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            y += a * (noise(&mut rng) - y);
            let tau = std::f32::consts::TAU;
            (tau * 50.0 * t).sin() * 0.6
                + (tau * 100.0 * t).sin() * 0.3
                + (tau * 150.0 * t).sin() * 0.15
                + y * 2.0
        })
        .collect();
    let fade = len(0.05);
    for i in 0..fade {
        let k = i as f32 / fade as f32;
        let tail = s[n - fade + i];
        s[i] = s[i] * k + tail * (1.0 - k);
    }
    s.truncate(n - fade);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every synthesized sound has samples, no NaN or infinity, and peaks at 0.9 after normalizing.
    #[test]
    fn synth_sounds_are_sane() {
        let all = [
            step(0),
            step(3),
            shot(),
            dry_fire(),
            reload(),
            hit(0),
            hit(2),
            kill(),
            bark(1),
            paw(2),
            bite(),
            yelp(),
            hurt(),
            switch(),
            engine(),
        ];
        for s in all {
            assert!(!s.is_empty());
            assert!(s.iter().all(|x| x.is_finite()));
            let n = normalize(s, 0.9);
            let peak = n.iter().fold(0.0f32, |m, x| m.max(x.abs()));
            assert!((peak - 0.9).abs() < 1e-4);
        }
    }
}

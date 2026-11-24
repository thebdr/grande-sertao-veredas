//! The scene file, `veredas-cena` version 1: what a scene holds and every refusal by its place
//! (`docs/cena-schema.md`). Metres, kilograms, seconds; z up; a part's pose is its centre and three
//! angles in degrees, yaw about z, then pitch about the new y, then roll about the new x.

use serde::Deserialize;
use veredas_core::schema::HeaderError;

use super::FisicaError;

/// The scene file's `format`.
pub const FORMAT: &str = "veredas-cena";
/// The version of the scene file this reader knows.
pub const VERSION: u32 = 1;

/// The longest side a box may have, and the farthest a centre may be from the origin, in metres.
pub const MAX_LENGTH: f64 = 10_000.0;
/// The shortest side a box may have, and the smallest ball, in metres: a millimetre (the engine
/// rests a smaller part sunk into what it lies on).
pub const MIN_LENGTH: f64 = 0.001;
/// The heaviest body, in kilograms.
pub const MAX_MASS: f64 = 100_000.0;
/// The lightest body, in kilograms: a milligram (the engine leaves a body of 1e-38 kg unmoved).
pub const MIN_MASS: f64 = 0.000_001;
/// The fastest belt, either way, in metres per second.
pub const MAX_SPEED: f64 = 100.0;
/// The highest friction coefficient a part may state.
pub const MAX_FRICTION: f64 = 10.0;

fn default_step_ms() -> u32 {
    4
}
fn default_gravity() -> f64 {
    9.81
}
fn default_friction() -> f64 {
    0.6
}
fn default_mass() -> f64 {
    1.0
}

/// A whole scene: its step, its gravity and its parts, in the file's order within each kind.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    /// The fixed step, in milliseconds (1 to 100).
    pub step_ms: u32,
    /// Gravity's pull along -z, in m/s² (0 to 100).
    pub gravity: f64,
    /// Fixed shapes: a floor, guides, a chute's sides.
    pub fixed: Vec<Fixed>,
    /// Belts: fixed boxes whose top surface moves along their own x.
    pub belts: Vec<Belt>,
    /// Bodies: the cartons and parts that move under gravity and contact.
    pub bodies: Vec<Body>,
    /// Movers: kinematic boxes the plant side places, a pusher's blade or a lift's table.
    pub movers: Vec<Mover>,
    /// Sensors: a box or a ball that reports the bodies and movers inside it.
    pub sensors: Vec<Sensor>,
}

impl Default for Scene {
    fn default() -> Self {
        Scene { step_ms: default_step_ms(), gravity: default_gravity(), fixed: Vec::new(), belts: Vec::new(), bodies: Vec::new(), movers: Vec::new(), sensors: Vec::new() }
    }
}

/// Where a part stands: its centre and its angles in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// The centre, in metres.
    pub at: [f64; 3],
    /// About z, in degrees.
    #[serde(default)]
    pub yaw: f64,
    /// About the y the yaw left, in degrees.
    #[serde(default)]
    pub pitch: f64,
    /// About the x the pitch left, in degrees.
    #[serde(default)]
    pub roll: f64,
}

impl Placement {
    /// A centre with no rotation.
    pub fn at(at: [f64; 3]) -> Placement {
        Placement { at, ..Placement::default() }
    }
}

/// A fixed box.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixed {
    pub name: String,
    /// Its full lengths along its own x, y and z, in metres.
    pub size: [f64; 3],
    pub at: [f64; 3],
    #[serde(default)]
    pub yaw: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub roll: f64,
    #[serde(default = "default_friction")]
    pub friction: f64,
}

/// A belt: a fixed box whose top surface (its own +z face) moves along its own x.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Belt {
    pub name: String,
    pub size: [f64; 3],
    pub at: [f64; 3],
    #[serde(default)]
    pub yaw: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub roll: f64,
    /// The surface's speed along the belt's own x, in m/s; negative runs it backwards.
    #[serde(default)]
    pub speed: f64,
    #[serde(default = "default_friction")]
    pub friction: f64,
}

/// A body: a box that moves under gravity and contact.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Body {
    pub name: String,
    pub size: [f64; 3],
    pub at: [f64; 3],
    #[serde(default)]
    pub yaw: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub roll: f64,
    /// In kilograms, spread evenly through the box.
    #[serde(default = "default_mass")]
    pub mass: f64,
    #[serde(default = "default_friction")]
    pub friction: f64,
}

impl Body {
    /// A body of 1 kg and friction 0.6, unrotated.
    pub fn new(name: &str, size: [f64; 3], at: [f64; 3]) -> Body {
        Body { name: name.to_string(), size, at, yaw: 0.0, pitch: 0.0, roll: 0.0, mass: default_mass(), friction: default_friction() }
    }
}

/// A mover: a kinematic box the plant side places; it pushes bodies and nothing pushes it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Mover {
    pub name: String,
    pub size: [f64; 3],
    pub at: [f64; 3],
    #[serde(default)]
    pub yaw: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub roll: f64,
    #[serde(default = "default_friction")]
    pub friction: f64,
}

/// A sensor: a box (`size`) or a ball (`radius`) that reports what is inside it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sensor {
    pub name: String,
    #[serde(default)]
    pub size: Option<[f64; 3]>,
    #[serde(default)]
    pub radius: Option<f64>,
    pub at: [f64; 3],
    #[serde(default)]
    pub yaw: f64,
    #[serde(default)]
    pub pitch: f64,
    #[serde(default)]
    pub roll: f64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileV1 {
    #[allow(dead_code)]
    format: String,
    #[allow(dead_code)]
    version: u32,
    #[serde(default = "default_step_ms")]
    step_ms: u32,
    #[serde(default = "default_gravity")]
    gravity: f64,
    #[serde(default)]
    fixed: Vec<Fixed>,
    #[serde(default)]
    belt: Vec<Belt>,
    #[serde(default)]
    body: Vec<Body>,
    #[serde(default)]
    mover: Vec<Mover>,
    #[serde(default)]
    sensor: Vec<Sensor>,
}

/// Reads a scene file's text: its header first (core's reader), then its body, then every rule.
pub fn load(text: &str) -> Result<Scene, FisicaError> {
    veredas_core::schema::check(text, "a scene file", FORMAT, VERSION).map_err(|e| match e {
        HeaderError::NotToml { reason, .. } => FisicaError::NotToml(reason),
        HeaderError::WrongFormat { found, .. } => FisicaError::WrongFormat(found),
        HeaderError::UnknownVersion { found, .. } => FisicaError::UnknownVersion(found),
        other => FisicaError::Invalid(other.to_string()),
    })?;
    let file: FileV1 = toml::from_str(text).map_err(|e| FisicaError::NotToml(e.to_string()))?;
    let scene = Scene { step_ms: file.step_ms, gravity: file.gravity, fixed: file.fixed, belts: file.belt, bodies: file.body, movers: file.mover, sensors: file.sensor };
    validate(&scene)?;
    Ok(scene)
}

fn invalid(m: String) -> FisicaError {
    FisicaError::Invalid(m)
}

/// Every rule a scene keeps, the first broken one named by its place.
pub fn validate(scene: &Scene) -> Result<(), FisicaError> {
    if !(1..=100).contains(&scene.step_ms) {
        return Err(invalid(format!("step_ms: {}, not 1 to 100", scene.step_ms)));
    }
    if !scene.gravity.is_finite() || !(0.0..=100.0).contains(&scene.gravity) {
        return Err(invalid(format!("gravity: {}, not 0 to 100 m/s²", scene.gravity)));
    }
    let mut names: Vec<(String, &'static str)> = Vec::new();
    for (i, f) in scene.fixed.iter().enumerate() {
        let place = named(&mut names, "fixed", i, &f.name)?;
        check_box(&place, f.size, f.at, [f.yaw, f.pitch, f.roll])?;
        check_friction(&place, f.friction)?;
    }
    for (i, b) in scene.belts.iter().enumerate() {
        let place = named(&mut names, "belt", i, &b.name)?;
        check_box(&place, b.size, b.at, [b.yaw, b.pitch, b.roll])?;
        check_speed(&place, b.speed)?;
        check_friction(&place, b.friction)?;
    }
    for (i, b) in scene.bodies.iter().enumerate() {
        let place = named(&mut names, "body", i, &b.name)?;
        check_body(&place, b)?;
    }
    for (i, m) in scene.movers.iter().enumerate() {
        let place = named(&mut names, "mover", i, &m.name)?;
        check_box(&place, m.size, m.at, [m.yaw, m.pitch, m.roll])?;
        check_friction(&place, m.friction)?;
    }
    for (i, s) in scene.sensors.iter().enumerate() {
        let place = named(&mut names, "sensor", i, &s.name)?;
        match (s.size, s.radius) {
            (Some(size), None) => check_box(&place, size, s.at, [s.yaw, s.pitch, s.roll])?,
            (None, Some(r)) => {
                if !r.is_finite() || r < MIN_LENGTH || r > MAX_LENGTH {
                    return Err(invalid(format!("{place}.radius: {r} m, not from {MIN_LENGTH} to {MAX_LENGTH}")));
                }
                check_pose(&place, s.at, [s.yaw, s.pitch, s.roll])?;
            }
            (Some(_), Some(_)) => return Err(invalid(format!("{place}: a sensor is a box (`size`) or a ball (`radius`), not both"))),
            (None, None) => return Err(invalid(format!("{place}: a sensor is a box (`size`) or a ball (`radius`); it has neither"))),
        }
    }
    Ok(())
}

/// The place of the `i`-th part of a kind, once its name is checked and taken.
fn named(names: &mut Vec<(String, &'static str)>, kind: &'static str, i: usize, name: &str) -> Result<String, FisicaError> {
    let place = format!("{kind}[{i}]");
    check_name(names, kind, &place, name)?;
    names.push((name.to_string(), kind));
    Ok(format!("{place} (`{name}`)"))
}

pub(super) fn check_name(names: &[(String, &'static str)], kind: &str, place: &str, name: &str) -> Result<(), FisicaError> {
    if name.trim().is_empty() {
        return Err(invalid(format!("{place}: the {kind} has no name")));
    }
    if name != name.trim() {
        return Err(invalid(format!("{place}: the name `{name}` has surrounding whitespace")));
    }
    if let Some((_, other)) = names.iter().find(|(n, _)| n == name) {
        return Err(invalid(format!("{place}: `{name}` is already the name of a {other}")));
    }
    Ok(())
}

pub(super) fn check_body(place: &str, b: &Body) -> Result<(), FisicaError> {
    check_box(place, b.size, b.at, [b.yaw, b.pitch, b.roll])?;
    if !b.mass.is_finite() || b.mass < MIN_MASS || b.mass > MAX_MASS {
        return Err(invalid(format!("{place}.mass: {} kg, not from {MIN_MASS} to {MAX_MASS}", b.mass)));
    }
    check_friction(place, b.friction)
}

fn check_box(place: &str, size: [f64; 3], at: [f64; 3], angles: [f64; 3]) -> Result<(), FisicaError> {
    if size.iter().any(|s| !s.is_finite() || *s < MIN_LENGTH || *s > MAX_LENGTH) {
        return Err(invalid(format!("{place}.size: {size:?} m, each side from {MIN_LENGTH} to {MAX_LENGTH}")));
    }
    check_pose(place, at, angles)
}

pub(crate) fn check_pose(place: &str, at: [f64; 3], angles: [f64; 3]) -> Result<(), FisicaError> {
    if at.iter().any(|a| !a.is_finite() || a.abs() > MAX_LENGTH) {
        return Err(invalid(format!("{place}.at: {at:?} m, each coordinate within {MAX_LENGTH} of the origin")));
    }
    for (key, a) in ["yaw", "pitch", "roll"].iter().zip(angles) {
        if !a.is_finite() {
            return Err(invalid(format!("{place}.{key}: {a}, not a finite angle")));
        }
    }
    Ok(())
}

pub(super) fn check_speed(place: &str, speed: f64) -> Result<(), FisicaError> {
    if !speed.is_finite() || speed.abs() > MAX_SPEED {
        return Err(invalid(format!("{place}.speed: {speed} m/s, not within {MAX_SPEED} either way")));
    }
    Ok(())
}

fn check_friction(place: &str, friction: f64) -> Result<(), FisicaError> {
    if !friction.is_finite() || !(0.0..=MAX_FRICTION).contains(&friction) {
        return Err(invalid(format!("{place}.friction: {friction}, not 0 to {MAX_FRICTION}")));
    }
    Ok(())
}

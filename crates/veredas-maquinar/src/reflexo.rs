//! `reflexo`, its first half: ready-made models that make a bench device behave as the machine
//! behind it (`docs/maquinar-plan.md`, "`reflexo`'s first half, as designed before its code").
//!
//! A model reads what a controller wrote to a device's points and writes what the controller
//! reads, and moves a `fisica` scene's parts: a drive ramps its speed, a conveyor's drive moves a
//! belt, a sensor reports a scene sensor on a point, an actuator moves a mover between two poses
//! and reports its ends. Models step together at a fixed step: on the scene's thread before each
//! of its steps, or on a thread of their own when they move no scene (drives alone).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::fisica::{self, FisicaHandle, Placement, World};
use crate::{ethernetip, modbus};

/// What a point carries, as the models see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointKind {
    Bool,
    Number,
}

impl PointKind {
    fn word(self) -> &'static str {
        match self {
            PointKind::Bool => "a bool",
            PointKind::Number => "a number",
        }
    }
}

/// A device's points as the models read and write them: a bool as 0 or 1, a number converted to
/// the point's type (an integer rounded and held to its range; NaN written as 0 whatever the type).
pub trait Points: Send + Sync {
    fn kind(&self, point: &str) -> Option<PointKind>;
    fn read(&self, point: &str) -> Result<f64, String>;
    fn write(&self, point: &str, value: f64) -> Result<(), String>;
}

/// `v` rounded and held to `[lo, hi]`; NaN is 0.
fn held(v: f64, lo: f64, hi: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v.round().clamp(lo, hi) }
}

impl Points for modbus::ModbusPoints {
    fn kind(&self, point: &str) -> Option<PointKind> {
        self.ty(point).ok().map(|t| if t == modbus::PointType::Bool { PointKind::Bool } else { PointKind::Number })
    }

    fn read(&self, point: &str) -> Result<f64, String> {
        use modbus::Value::*;
        Ok(match self.get(point).map_err(|e| e.to_string())? {
            Bool(b) => f64::from(u8::from(b)),
            U16(v) => f64::from(v),
            I16(v) => f64::from(v),
            U32(v) => f64::from(v),
            I32(v) => f64::from(v),
            F32(v) => f64::from(v),
        })
    }

    fn write(&self, point: &str, v: f64) -> Result<(), String> {
        use modbus::{PointType as T, Value};
        let value = match self.ty(point).map_err(|e| e.to_string())? {
            T::Bool => Value::Bool(v != 0.0 && !v.is_nan()),
            T::U16 => Value::U16(held(v, 0.0, f64::from(u16::MAX)) as u16),
            T::I16 => Value::I16(held(v, f64::from(i16::MIN), f64::from(i16::MAX)) as i16),
            T::U32 => Value::U32(held(v, 0.0, f64::from(u32::MAX)) as u32),
            T::I32 => Value::I32(held(v, f64::from(i32::MIN), f64::from(i32::MAX)) as i32),
            T::F32 => Value::F32(if v.is_nan() { 0.0 } else { v as f32 }),
        };
        self.set_value(point, value).map_err(|e| e.to_string())
    }
}

impl Points for ethernetip::EnipPoints {
    fn kind(&self, point: &str) -> Option<PointKind> {
        self.ty(point).ok().map(|t| if t == ethernetip::PointType::Bool { PointKind::Bool } else { PointKind::Number })
    }

    fn read(&self, point: &str) -> Result<f64, String> {
        use ethernetip::Value::*;
        Ok(match self.get(point).map_err(|e| e.to_string())? {
            Bool(b) => f64::from(u8::from(b)),
            U8(v) => f64::from(v),
            I8(v) => f64::from(v),
            U16(v) => f64::from(v),
            I16(v) => f64::from(v),
            U32(v) => f64::from(v),
            I32(v) => f64::from(v),
            F32(v) => f64::from(v),
        })
    }

    fn write(&self, point: &str, v: f64) -> Result<(), String> {
        use ethernetip::{PointType as T, Value};
        let value = match self.ty(point).map_err(|e| e.to_string())? {
            T::Bool => Value::Bool(v != 0.0 && !v.is_nan()),
            T::U8 => Value::U8(held(v, 0.0, f64::from(u8::MAX)) as u8),
            T::I8 => Value::I8(held(v, f64::from(i8::MIN), f64::from(i8::MAX)) as i8),
            T::U16 => Value::U16(held(v, 0.0, f64::from(u16::MAX)) as u16),
            T::I16 => Value::I16(held(v, f64::from(i16::MIN), f64::from(i16::MAX)) as i16),
            T::U32 => Value::U32(held(v, 0.0, f64::from(u32::MAX)) as u32),
            T::I32 => Value::I32(held(v, f64::from(i32::MIN), f64::from(i32::MAX)) as i32),
            T::F32 => Value::F32(if v.is_nan() { 0.0 } else { v as f32 }),
        };
        self.set_value(point, value).map_err(|e| e.to_string())
    }
}

/// The longest delay or stroke, in milliseconds.
pub const MAX_MS: u32 = 600_000;

/// A part's models, as the bench file declares them (its `name` apart).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReflexoConfig {
    /// The `[[fisica]]` part whose scene the models move, if any.
    #[serde(default)]
    pub scene: Option<String>,
    #[serde(default)]
    pub drive: Vec<Drive>,
    #[serde(default)]
    pub conveyor: Vec<Conveyor>,
    #[serde(default)]
    pub sensor: Vec<SensorModel>,
    #[serde(default)]
    pub actuator: Vec<Actuator>,
}

/// A drive: a speed ramped toward its setpoint while it runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drive {
    pub name: String,
    /// Read: a bool point, run while 1.
    pub run: String,
    /// Read: a number point, the speed wanted in the drive's unit.
    pub setpoint: String,
    /// Read: a bool point, the setpoint negated while 1.
    #[serde(default)]
    pub reverse: Option<String>,
    /// Units a second the speed gains away from 0.
    pub accel: f64,
    /// Units a second the speed loses toward 0 or a smaller setpoint.
    pub decel: f64,
    /// How near the setpoint the speed counts as at it.
    #[serde(default)]
    pub tolerance: f64,
    /// Written: a number point, the speed.
    #[serde(default)]
    pub speed: Option<String>,
    /// Written: a bool point, 1 while the speed is not 0.
    #[serde(default)]
    pub running: Option<String>,
    /// Written: a bool point, 1 while running within the tolerance of the setpoint.
    #[serde(default)]
    pub at_speed: Option<String>,
}

/// A conveyor: a drive whose speed moves a belt of the scene.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Conveyor {
    pub name: String,
    /// The scene's belt.
    pub belt: String,
    /// The belt's surface speed, m/s, for one unit of the drive's speed.
    pub metres_per_unit: f64,
    pub run: String,
    pub setpoint: String,
    #[serde(default)]
    pub reverse: Option<String>,
    pub accel: f64,
    pub decel: f64,
    #[serde(default)]
    pub tolerance: f64,
    #[serde(default)]
    pub speed: Option<String>,
    #[serde(default)]
    pub running: Option<String>,
    #[serde(default)]
    pub at_speed: Option<String>,
}

impl Conveyor {
    fn drive(&self) -> Drive {
        Drive {
            name: self.name.clone(),
            run: self.run.clone(),
            setpoint: self.setpoint.clone(),
            reverse: self.reverse.clone(),
            accel: self.accel,
            decel: self.decel,
            tolerance: self.tolerance,
            speed: self.speed.clone(),
            running: self.running.clone(),
            at_speed: self.at_speed.clone(),
        }
    }
}

/// A sensor: a scene sensor reported on a bool point.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SensorModel {
    pub name: String,
    /// The scene's sensor.
    pub sensor: String,
    /// Written: a bool point, 1 while something is inside (0 when normally closed).
    pub point: String,
    #[serde(default)]
    pub normally_closed: bool,
    /// How long something stays inside before the point says so, in milliseconds.
    #[serde(default)]
    pub on_delay_ms: u32,
    /// How long the sensor stays empty before the point says so, in milliseconds.
    #[serde(default)]
    pub off_delay_ms: u32,
}

/// An actuator: a cylinder moving a mover of the scene between two poses.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Actuator {
    pub name: String,
    /// The scene's mover.
    pub mover: String,
    /// Read: a bool point, extend while 1, retract while 0.
    pub extend: String,
    pub retracted: Placement,
    pub extended: Placement,
    /// The time from one end to the other, in milliseconds.
    pub stroke_ms: u32,
    /// Written: bool points, 1 at that end.
    #[serde(default)]
    pub retracted_switch: Option<String>,
    #[serde(default)]
    pub extended_switch: Option<String>,
}

/// Why a part's models were refused, or did not start.
#[derive(Debug, thiserror::Error)]
pub enum ReflexoError {
    #[error("{0}")]
    Invalid(String),
    #[error("the models' thread did not start: {0}")]
    Thread(String),
}

/// Every rule a part's models keep, the first broken one named by its place. `scene` is the scene
/// its `scene` key names, already read; `point` says what a `<part>.<point>` carries, or why it
/// names nothing.
pub fn validate(cfg: &ReflexoConfig, scene: Option<&fisica::Scene>, point: &dyn Fn(&str, &str) -> Result<PointKind, String>) -> Result<(), String> {
    validate_beside(cfg, scene, point, &[]).map(|_| ())
}

/// What one model of the bench takes for itself: a point it writes, or a scene's belt or mover it
/// moves. A belt or a mover is known by its kind, its scene part and its name apart, never by a
/// text joining them (round 7: two belts whose joined names read alike were taken for one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// A point written, by its `<part>.<point>` reference.
    Point(String),
    /// A belt or a mover moved: `kind` is "belt" or "mover", `scene` the `[[fisica]]` part's name.
    Moved { kind: &'static str, scene: String, name: String },
}

/// [`validate`], beside the points other parts' models already write (each with the place that
/// writes it, named in full): a point is written by one model of the whole bench at most, and a
/// scene's belt or mover moved by one at most (round 6: two loaded, the last in the order won, and
/// the first said what did not happen). Returns what this part's models write and move, each with
/// its place in the part.
pub fn validate_beside(cfg: &ReflexoConfig, scene: Option<&fisica::Scene>, point: &dyn Fn(&str, &str) -> Result<PointKind, String>, others: &[(Claim, String)]) -> Result<Vec<(Claim, String)>, String> {
    let mut names: Vec<(String, &'static str)> = Vec::new();
    let mut written: Vec<(Claim, String)> = Vec::new();
    let mut check_name = |kind: &'static str, i: usize, name: &str| -> Result<String, String> {
        let place = format!("{kind}[{i}]");
        if name.trim().is_empty() {
            return Err(format!("{place}: the {kind} has no name"));
        }
        if name != name.trim() {
            return Err(format!("{place}: the name `{name}` has surrounding whitespace"));
        }
        if let Some((_, other)) = names.iter().find(|(n, _)| n == name) {
            return Err(format!("{place}: `{name}` is already the name of a {other}"));
        }
        names.push((name.to_string(), kind));
        Ok(format!("{place} (`{name}`)"))
    };
    let resolve = |place: &str, key: &str, reference: &str, want: PointKind| -> Result<(), String> {
        let Some((part, name)) = reference.split_once('.').filter(|(p, n)| !p.is_empty() && !n.is_empty()) else {
            return Err(format!("{place}.{key}: `{reference}` is not `<part>.<point>`"));
        };
        let kind = point(part, name).map_err(|e| format!("{place}.{key}: {e}"))?;
        if kind != want {
            return Err(format!("{place}.{key}: `{reference}` is {}, {} point is wanted", kind.word(), want.word()));
        }
        Ok(())
    };
    // A point written, or a scene's belt or mover moved: by one model of the whole bench at most.
    let mut claim = |place: &str, key: &str, id: Claim| -> Result<(), String> {
        if let Some((_, by)) = written.iter().chain(others).find(|(c, _)| *c == id) {
            return Err(match &id {
                Claim::Moved { kind, name, .. } => format!("{place}.{key}: the {kind} `{name}` is moved by {by} already"),
                Claim::Point(r) => format!("{place}.{key}: `{r}` is written by {by} already"),
            });
        }
        written.push((id, place.to_string()));
        Ok(())
    };
    let scene_name = cfg.scene.clone().unwrap_or_default();
    let moved = |kind: &'static str, name: &str| Claim::Moved { kind, scene: scene_name.clone(), name: name.to_string() };
    let needs_scene = |place: &str, what: &str| -> Result<&fisica::Scene, String> {
        scene.ok_or_else(|| format!("{place}: {what}; this part names no `scene`"))
    };
    let mut drives: Vec<(String, Drive)> = Vec::new();
    for (i, d) in cfg.drive.iter().enumerate() {
        drives.push((check_name("drive", i, &d.name)?, d.clone()));
    }
    for (i, c) in cfg.conveyor.iter().enumerate() {
        let place = check_name("conveyor", i, &c.name)?;
        let s = needs_scene(&place, "a conveyor moves a belt of a scene")?;
        if !s.belts.iter().any(|b| b.name == c.belt) {
            return Err(format!("{place}.belt: the scene has no belt `{}`", c.belt));
        }
        claim(&place, "belt", moved("belt", &c.belt))?;
        if !c.metres_per_unit.is_finite() || c.metres_per_unit == 0.0 {
            return Err(format!("{place}.metres_per_unit: {}, not a finite number other than 0", c.metres_per_unit));
        }
        drives.push((place, c.drive()));
    }
    for (place, d) in &drives {
        resolve(place, "run", &d.run, PointKind::Bool)?;
        resolve(place, "setpoint", &d.setpoint, PointKind::Number)?;
        if let Some(r) = &d.reverse {
            resolve(place, "reverse", r, PointKind::Bool)?;
        }
        for (key, v) in [("accel", d.accel), ("decel", d.decel)] {
            if !v.is_finite() || v <= 0.0 {
                return Err(format!("{place}.{key}: {v}, not a finite number over 0"));
            }
        }
        if !d.tolerance.is_finite() || d.tolerance < 0.0 {
            return Err(format!("{place}.tolerance: {}, not a finite number from 0", d.tolerance));
        }
        for (key, reference, want) in [("speed", &d.speed, PointKind::Number), ("running", &d.running, PointKind::Bool), ("at_speed", &d.at_speed, PointKind::Bool)] {
            if let Some(r) = reference {
                resolve(place, key, r, want)?;
                claim(place, key, Claim::Point(r.clone()))?;
            }
        }
    }
    for (i, a) in cfg.actuator.iter().enumerate() {
        let place = check_name("actuator", i, &a.name)?;
        let s = needs_scene(&place, "an actuator moves a mover of a scene")?;
        if !s.movers.iter().any(|m| m.name == a.mover) {
            return Err(format!("{place}.mover: the scene has no mover `{}`", a.mover));
        }
        claim(&place, "mover", moved("mover", &a.mover))?;
        resolve(&place, "extend", &a.extend, PointKind::Bool)?;
        for (key, p) in [("retracted", a.retracted), ("extended", a.extended)] {
            fisica::scene::check_pose(&format!("{place}.{key}"), p.at, [p.yaw, p.pitch, p.roll]).map_err(|e| e.to_string())?;
        }
        if !(1..=MAX_MS).contains(&a.stroke_ms) {
            return Err(format!("{place}.stroke_ms: {}, not 1 to {MAX_MS}", a.stroke_ms));
        }
        for (key, reference) in [("retracted_switch", &a.retracted_switch), ("extended_switch", &a.extended_switch)] {
            if let Some(r) = reference {
                resolve(&place, key, r, PointKind::Bool)?;
                claim(&place, key, Claim::Point(r.clone()))?;
            }
        }
    }
    for (i, m) in cfg.sensor.iter().enumerate() {
        let place = check_name("sensor", i, &m.name)?;
        let s = needs_scene(&place, "a sensor reports a sensor of a scene")?;
        if !s.sensors.iter().any(|x| x.name == m.sensor) {
            return Err(format!("{place}.sensor: the scene has no sensor `{}`", m.sensor));
        }
        resolve(&place, "point", &m.point, PointKind::Bool)?;
        claim(&place, "point", Claim::Point(m.point.clone()))?;
        for (key, v) in [("on_delay_ms", m.on_delay_ms), ("off_delay_ms", m.off_delay_ms)] {
            if v > MAX_MS {
                return Err(format!("{place}.{key}: {v}, over {MAX_MS}"));
            }
        }
    }
    Ok(written)
}

/// A point bound to the device that holds it.
struct Bound {
    points: Arc<dyn Points>,
    point: String,
    reference: String,
}

impl Bound {
    fn read(&self) -> Result<f64, String> {
        self.points.read(&self.point).map_err(|e| format!("{}: {e}", self.reference))
    }

    fn write(&self, v: f64) -> Result<(), String> {
        self.points.write(&self.point, v).map_err(|e| format!("{}: {e}", self.reference))
    }
}

struct DriveState {
    name: String,
    run: Bound,
    setpoint: Bound,
    reverse: Option<Bound>,
    accel: f64,
    decel: f64,
    tolerance: f64,
    speed_out: Option<Bound>,
    running_out: Option<Bound>,
    at_speed_out: Option<Bound>,
    speed: f64,
    ramp: Option<Segment>,
}

/// A ramp toward one target: the speed at its `k`-th step is `from` plus `k` steps' change, the
/// target itself from its `steps`-th on. Taken from a count, not summed, so no rounding gathers
/// however long the ramp: it lands at the first step at or after its time. A ramp through 0 is one
/// ramp of two rates: down to 0 at the deceleration in `to_zero` steps (a fraction of a step as
/// often as not), then away at the acceleration from that moment, so the step that passes 0 carries
/// what is left of it past 0 and the whole lands at the first step at or after the sum of the two.
struct Segment {
    from: f64,
    target: f64,
    per_step: f64,
    /// Through 0: the steps to 0, and the change a step after it.
    to_zero: Option<(f64, f64)>,
    steps: u64,
    k: u64,
}

/// The first step at or after `x` steps. The division's rounding grows with the count (past 2^24
/// steps it passes any fixed slack), so the slack is relative: sixteen units in the last place of
/// the count, and a billionth of a step besides. A count too large for 64 bits (an infinite setpoint
/// a client wrote, a rate of 1e-300) is held at the largest: the ramp goes on at its rate.
fn first_step_at_or_after(x: f64) -> u64 {
    (x * (1.0 - 16.0 * f64::EPSILON) - 1e-9).ceil().max(1.0) as u64
}

/// The steps a change of `distance` takes at `rate` units a second in steps of `dt` seconds: the
/// first step at or after its time.
pub fn ramp_steps(distance: f64, rate: f64, dt: f64) -> u64 {
    first_step_at_or_after(distance / (rate * dt))
}

impl Segment {
    fn new(from: f64, target: f64, accel: f64, decel: f64, dt: f64) -> Segment {
        if from != 0.0 && target != 0.0 && from.signum() != target.signum() {
            let to_zero = from.abs() / (decel * dt);
            let steps = first_step_at_or_after(to_zero + target.abs() / (accel * dt));
            let after = target.signum() * accel * dt;
            return Segment { from, target, per_step: -from.signum() * decel * dt, to_zero: Some((to_zero, after)), steps, k: 0 };
        }
        // Away from 0 (from 0, or past `from` on its side) at the acceleration; toward 0 or a
        // smaller target of the same sign at the deceleration.
        let rate = if target.abs() > from.abs() { accel } else { decel };
        let per_step = if target > from { rate * dt } else { -rate * dt };
        Segment { from, target, per_step, to_zero: None, steps: ramp_steps((target - from).abs(), rate, dt), k: 0 }
    }

    /// The speed at its `k`-th step.
    fn speed(&self) -> f64 {
        if self.k >= self.steps {
            return self.target;
        }
        let k = self.k as f64;
        let v = match self.to_zero {
            // On 0 at a step within the slack of its time (the division's rounding put 9/(18 x
            // 0.004) at 124.99999999999999): 0, not a speck of either sign with the drive running.
            // A zero past f64 (a deceleration of 1e-306) is never reached: the ramp goes on at its rate.
            Some((to_zero, _)) if to_zero.is_finite() && (k - to_zero).abs() <= to_zero * 16.0 * f64::EPSILON + 1e-9 => 0.0,
            Some((to_zero, after)) if k >= to_zero => after * (k - to_zero),
            // Before 0 the speed keeps its sign, whatever the rounding.
            Some(_) if self.from > 0.0 => (self.from + self.per_step * k).max(0.0),
            Some(_) => (self.from + self.per_step * k).min(0.0),
            None => self.from + self.per_step * k,
        };
        if v == 0.0 { 0.0 } else { v }
    }
}

impl DriveState {
    /// One step from the speed toward `target`: away from 0 at the acceleration, toward 0 (or a
    /// smaller target of the same sign) at the deceleration, through 0 when the sign changes.
    fn ramp(&mut self, target: f64, dt: f64) -> f64 {
        // A NaN target ramps to 0; a reversed setpoint of 0 is 0, not -0.
        let target = if target.is_nan() || target == 0.0 { 0.0 } else { target };
        if self.speed == target {
            self.ramp = None;
            return target;
        }
        let mut s = match self.ramp.take() {
            Some(s) if s.target == target => s,
            _ => Segment::new(self.speed, target, self.accel, self.decel, dt),
        };
        s.k += 1;
        let speed = s.speed();
        if s.k < s.steps {
            self.ramp = Some(s);
        }
        speed
    }

    fn step(&mut self, dt: f64) -> Result<(), String> {
        let run = self.run.read()? != 0.0;
        let reverse = match &self.reverse {
            Some(r) => r.read()? != 0.0,
            None => false,
        };
        let setpoint = self.setpoint.read()?;
        let target = if run { if reverse { -setpoint } else { setpoint } } else { 0.0 };
        self.speed = self.ramp(target, dt);
        if let Some(o) = &self.speed_out {
            o.write(self.speed)?;
        }
        if let Some(o) = &self.running_out {
            o.write(f64::from(u8::from(self.speed != 0.0)))?;
        }
        if let Some(o) = &self.at_speed_out {
            o.write(f64::from(u8::from(run && (self.speed - target).abs() <= self.tolerance)))?;
        }
        Ok(())
    }
}

struct ConveyorState {
    drive: DriveState,
    belt: String,
    metres_per_unit: f64,
}

struct SensorState {
    name: String,
    sensor: String,
    out: Bound,
    normally_closed: bool,
    on_delay: u64,
    off_delay: u64,
    /// What the point says: something inside.
    occupied: bool,
    /// How long the scene has disagreed with the point, in nanoseconds: counted, never rounded.
    pending: u64,
}

struct ActuatorState {
    name: String,
    mover: String,
    extend: Bound,
    retracted: Placement,
    extended: Placement,
    /// The stroke's time, in nanoseconds.
    stroke: u64,
    retracted_switch: Option<Bound>,
    extended_switch: Option<Bound>,
    /// How far out, as the time from the retracted end in nanoseconds, 0 to `stroke`: counted in
    /// whole steps, so an end is reached at the first step at or after its time.
    out: u64,
}

impl ActuatorState {
    /// 0 retracted, 1 extended.
    fn position(&self) -> f64 {
        self.out as f64 / self.stroke as f64
    }
}

fn lerp(a: Placement, b: Placement, p: f64) -> Placement {
    // Each end exactly (a pose the scene took stays one it takes: -9999.9 to 10000 at p = 1 gave
    // 10000.000000000002), every pose between them held between them, and a difference past f64
    // (yaws of -1e308 and 1e308) taken in two halves.
    let l = |x: f64, y: f64| {
        if p >= 1.0 {
            y
        } else if p <= 0.0 {
            x
        } else if (y - x).is_finite() {
            (x + (y - x) * p).clamp(x.min(y), x.max(y))
        } else {
            (x * (1.0 - p) + y * p).clamp(x.min(y), x.max(y))
        }
    };
    Placement { at: [l(a.at[0], b.at[0]), l(a.at[1], b.at[1]), l(a.at[2], b.at[2])], yaw: l(a.yaw, b.yaw), pitch: l(a.pitch, b.pitch), roll: l(a.roll, b.roll) }
}

/// What a part's models have done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReflexoStatus {
    pub steps: u64,
    /// Steps in which a model could not read or write a point or move its part, each counted once.
    pub errors: u64,
    pub last_error: Option<String>,
}

/// A part's models, bound to their points, stepped by their caller.
pub struct Models {
    dt: f64,
    /// The step in nanoseconds, what the actuators and the sensors' delays count.
    dt_ns: u64,
    drives: Vec<DriveState>,
    conveyors: Vec<ConveyorState>,
    actuators: Vec<ActuatorState>,
    sensors: Vec<SensorState>,
    status: ReflexoStatus,
}

impl Models {
    /// The models of a checked configuration ([`validate`]), each point bound through `part`
    /// (a part's name to its points), stepping every `step`.
    pub fn new(cfg: &ReflexoConfig, step: Duration, part: &dyn Fn(&str) -> Option<Arc<dyn Points>>) -> Result<Models, ReflexoError> {
        let bind = |reference: &str| -> Result<Bound, ReflexoError> {
            let (p, name) = reference.split_once('.').ok_or_else(|| ReflexoError::Invalid(format!("`{reference}` is not `<part>.<point>`")))?;
            let points = part(p).ok_or_else(|| ReflexoError::Invalid(format!("no part `{p}` holds points")))?;
            Ok(Bound { points, point: name.to_string(), reference: reference.to_string() })
        };
        let bind_opt = |r: &Option<String>| r.as_deref().map(bind).transpose();
        let drive = |d: &Drive| -> Result<DriveState, ReflexoError> {
            Ok(DriveState {
                name: d.name.clone(),
                run: bind(&d.run)?,
                setpoint: bind(&d.setpoint)?,
                reverse: bind_opt(&d.reverse)?,
                accel: d.accel,
                decel: d.decel,
                tolerance: d.tolerance,
                speed_out: bind_opt(&d.speed)?,
                running_out: bind_opt(&d.running)?,
                at_speed_out: bind_opt(&d.at_speed)?,
                speed: 0.0,
                ramp: None,
            })
        };
        let ms = |v: u32| u64::from(v) * 1_000_000;
        Ok(Models {
            dt: step.as_secs_f64(),
            dt_ns: u64::try_from(step.as_nanos()).unwrap_or(u64::MAX),
            drives: cfg.drive.iter().map(drive).collect::<Result<_, _>>()?,
            conveyors: cfg.conveyor.iter().map(|c| Ok(ConveyorState { drive: drive(&c.drive())?, belt: c.belt.clone(), metres_per_unit: c.metres_per_unit })).collect::<Result<_, ReflexoError>>()?,
            actuators: cfg
                .actuator
                .iter()
                .map(|a| {
                    Ok(ActuatorState {
                        name: a.name.clone(),
                        mover: a.mover.clone(),
                        extend: bind(&a.extend)?,
                        retracted: a.retracted,
                        extended: a.extended,
                        stroke: ms(a.stroke_ms),
                        retracted_switch: bind_opt(&a.retracted_switch)?,
                        extended_switch: bind_opt(&a.extended_switch)?,
                        out: 0,
                    })
                })
                .collect::<Result<_, ReflexoError>>()?,
            sensors: cfg
                .sensor
                .iter()
                .map(|m| {
                    Ok(SensorState {
                        name: m.name.clone(),
                        sensor: m.sensor.clone(),
                        out: bind(&m.point)?,
                        normally_closed: m.normally_closed,
                        on_delay: ms(m.on_delay_ms),
                        off_delay: ms(m.off_delay_ms),
                        occupied: false,
                        pending: 0,
                    })
                })
                .collect::<Result<_, ReflexoError>>()?,
            status: ReflexoStatus::default(),
        })
    }

    /// One step of every model, in this order: drives, conveyors, actuators, sensors, each kind
    /// in the file's order. A model that cannot read or write a point is skipped for the rest of
    /// the step; a step in which any model failed is counted once, its last error named.
    pub fn step(&mut self, mut world: Option<&mut World>) {
        let (dt, dt_ns) = (self.dt, self.dt_ns);
        let mut errors: Vec<String> = Vec::new();
        for d in &mut self.drives {
            if let Err(e) = d.step(dt) {
                errors.push(format!("drive `{}`: {e}", d.name));
            }
        }
        for c in &mut self.conveyors {
            if let Err(e) = c.drive.step(dt) {
                errors.push(format!("conveyor `{}`: {e}", c.drive.name));
                continue;
            }
            if let Some(w) = world.as_deref_mut() {
                let speed = (c.drive.speed * c.metres_per_unit).clamp(-fisica::scene::MAX_SPEED, fisica::scene::MAX_SPEED);
                if w.speed(&c.belt) != Some(speed)
                    && let Err(e) = w.set_speed(&c.belt, speed)
                {
                    errors.push(format!("conveyor `{}`: {e}", c.drive.name));
                }
            }
        }
        'actuators: for a in &mut self.actuators {
            let extend = match a.extend.read() {
                Ok(v) => v != 0.0,
                Err(e) => {
                    errors.push(format!("actuator `{}`: {e}", a.name));
                    continue;
                }
            };
            a.out = if extend { a.out.saturating_add(dt_ns).min(a.stroke) } else { a.out.saturating_sub(dt_ns) };
            if let Some(w) = world.as_deref_mut()
                && let Err(e) = w.place(&a.mover, lerp(a.retracted, a.extended, a.position()))
            {
                // Its mover not where it says: no switch written for the rest of its step.
                errors.push(format!("actuator `{}`: {e}", a.name));
                continue 'actuators;
            }
            for (switch, at) in [(&a.retracted_switch, a.out == 0), (&a.extended_switch, a.out == a.stroke)] {
                if let Some(s) = switch
                    && let Err(e) = s.write(f64::from(u8::from(at)))
                {
                    // Skipped for the rest of its step, as a drive is at its first failed write.
                    errors.push(format!("actuator `{}`: {e}", a.name));
                    continue 'actuators;
                }
            }
        }
        for s in &mut self.sensors {
            let inside = world.as_deref().and_then(|w| w.sensor(&s.sensor)).is_some_and(|x| x.occupied());
            if inside == s.occupied {
                s.pending = 0;
            } else {
                s.pending = s.pending.saturating_add(dt_ns);
                // The first step at or after the delay, the scene disagreeing with the point at
                // every step till then.
                if s.pending >= if inside { s.on_delay } else { s.off_delay } {
                    s.occupied = inside;
                    s.pending = 0;
                }
            }
            if let Err(e) = s.out.write(f64::from(u8::from(s.occupied != s.normally_closed))) {
                errors.push(format!("sensor `{}`: {e}", s.name));
            }
        }
        self.status.steps += 1;
        if let Some(last) = errors.pop() {
            self.status.errors += 1;
            self.status.last_error = Some(last);
        }
    }

    pub fn status(&self) -> ReflexoStatus {
        self.status.clone()
    }

    /// A drive's or a conveyor's speed, in its unit.
    pub fn speed(&self, name: &str) -> Option<f64> {
        self.drives.iter().chain(self.conveyors.iter().map(|c| &c.drive)).find(|d| d.name == name).map(|d| d.speed)
    }

    /// An actuator's position, 0 retracted to 1 extended.
    pub fn position(&self, name: &str) -> Option<f64> {
        self.actuators.iter().find(|a| a.name == name).map(ActuatorState::position)
    }

    /// What a sensor model's point says: something inside (before `normally_closed`).
    pub fn occupied(&self, name: &str) -> Option<bool> {
        self.sensors.iter().find(|s| s.name == name).map(|s| s.occupied)
    }

    /// How many models.
    pub fn len(&self) -> usize {
        self.drives.len() + self.conveyors.len() + self.actuators.len() + self.sensors.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

struct Shared {
    models: Mutex<Models>,
    stop: AtomicBool,
}

impl Shared {
    fn models(&self) -> MutexGuard<'_, Models> {
        self.models.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A part's models, running. Dropping it stops them.
pub struct ReflexoHandle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// The step of models that move no scene.
pub const STEP_WITHOUT_SCENE: Duration = Duration::from_millis(4);

/// Runs the models: on `scene`'s thread before each of its steps, else on a thread of their own
/// at their step, one step per step of real time.
pub fn start(models: Models, scene: Option<&FisicaHandle>) -> Result<ReflexoHandle, ReflexoError> {
    let step = Duration::from_secs_f64(models.dt);
    let shared = Arc::new(Shared { models: Mutex::new(models), stop: AtomicBool::new(false) });
    let run = Arc::clone(&shared);
    if let Some(scene) = scene {
        scene.add_hook(Box::new(move |world: &mut World| {
            // The flag read under the models' lock, so a stop that took the lock after it is kept.
            let mut models = run.models();
            if !run.stop.load(Ordering::SeqCst) {
                models.step(Some(world));
            }
        }));
        return Ok(ReflexoHandle { shared, thread: None });
    }
    let thread = std::thread::Builder::new()
        .name("reflexo".into())
        .spawn(move || {
            let _timer = crate::timer::FineTimer::hold();
            let begun = Instant::now();
            let mut k: u64 = 0;
            while !run.stop.load(Ordering::SeqCst) {
                k += 1;
                let due = begun + Duration::from_nanos((step.as_nanos() as u64).saturating_mul(k));
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                }
                let mut models = run.models();
                if run.stop.load(Ordering::SeqCst) {
                    break;
                }
                models.step(None);
            }
        })
        .map_err(|e| ReflexoError::Thread(e.to_string()))?;
    Ok(ReflexoHandle { shared, thread: Some(thread) })
}

impl ReflexoHandle {
    pub fn status(&self) -> ReflexoStatus {
        self.shared.models().status()
    }

    pub fn speed(&self, name: &str) -> Option<f64> {
        self.shared.models().speed(name)
    }

    pub fn position(&self, name: &str) -> Option<f64> {
        self.shared.models().position(name)
    }

    pub fn occupied(&self, name: &str) -> Option<bool> {
        self.shared.models().occupied(name)
    }

    pub fn models(&self) -> usize {
        self.shared.models().len()
    }

    /// Stops the models: a step under way ends first, and none begins after this returns.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        drop(self.shared.models());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for ReflexoHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

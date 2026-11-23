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
/// the point's type (an integer rounded and held to its range, NaN as 0).
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
            T::F32 => Value::F32(v as f32),
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
            T::F32 => Value::F32(v as f32),
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
    let mut names: Vec<(String, &'static str)> = Vec::new();
    let mut written: Vec<(String, String)> = Vec::new();
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
    let mut write = |place: &str, key: &str, reference: &str| -> Result<(), String> {
        if let Some((_, by)) = written.iter().find(|(r, _)| r == reference) {
            return Err(format!("{place}.{key}: `{reference}` is written by {by} already"));
        }
        written.push((reference.to_string(), place.to_string()));
        Ok(())
    };
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
                write(place, key, r)?;
            }
        }
    }
    for (i, a) in cfg.actuator.iter().enumerate() {
        let place = check_name("actuator", i, &a.name)?;
        let s = needs_scene(&place, "an actuator moves a mover of a scene")?;
        if !s.movers.iter().any(|m| m.name == a.mover) {
            return Err(format!("{place}.mover: the scene has no mover `{}`", a.mover));
        }
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
                write(&place, key, r)?;
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
        write(&place, "point", &m.point)?;
        for (key, v) in [("on_delay_ms", m.on_delay_ms), ("off_delay_ms", m.off_delay_ms)] {
            if v > MAX_MS {
                return Err(format!("{place}.{key}: {v}, over {MAX_MS}"));
            }
        }
    }
    Ok(())
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
}

/// One step of a ramp from `speed` toward `target`: away from 0 at `accel`, toward 0 (or a smaller
/// target of the same sign) at `decel`, through 0 when the sign changes; landing on the target
/// exactly.
pub fn ramp(speed: f64, target: f64, accel: f64, decel: f64, dt: f64) -> f64 {
    if speed == target {
        return speed;
    }
    let away = target != 0.0 && (speed == 0.0 || speed.signum() == target.signum()) && target.abs() > speed.abs();
    // A step's change summed in floating point drifts by a few units in the last place: within a
    // billionth of a step's change of the goal is the goal, so a ramp lands when its time is up.
    if away {
        let next = speed + target.signum() * accel * dt;
        if (target - next) * target.signum() <= accel * dt * 1e-9 { target } else { next }
    } else {
        let goal = if target == 0.0 || target.signum() != speed.signum() { 0.0 } else { target };
        let next = speed - speed.signum() * decel * dt;
        if (next - goal) * speed.signum() <= decel * dt * 1e-9 { goal } else { next }
    }
}

impl DriveState {
    fn step(&mut self, dt: f64) -> Result<(), String> {
        let run = self.run.read()? != 0.0;
        let reverse = match &self.reverse {
            Some(r) => r.read()? != 0.0,
            None => false,
        };
        let setpoint = self.setpoint.read()?;
        let target = if run { if reverse { -setpoint } else { setpoint } } else { 0.0 };
        self.speed = ramp(self.speed, target, self.accel, self.decel, dt);
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
    on_delay: f64,
    off_delay: f64,
    /// What the point says: something inside.
    occupied: bool,
    /// How long the scene has disagreed with the point.
    pending: f64,
}

struct ActuatorState {
    name: String,
    mover: String,
    extend: Bound,
    retracted: Placement,
    extended: Placement,
    stroke: f64,
    retracted_switch: Option<Bound>,
    extended_switch: Option<Bound>,
    /// 0 retracted, 1 extended.
    position: f64,
}

fn lerp(a: Placement, b: Placement, p: f64) -> Placement {
    let l = |x: f64, y: f64| x + (y - x) * p;
    Placement { at: [l(a.at[0], b.at[0]), l(a.at[1], b.at[1]), l(a.at[2], b.at[2])], yaw: l(a.yaw, b.yaw), pitch: l(a.pitch, b.pitch), roll: l(a.roll, b.roll) }
}

/// What a part's models have done.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReflexoStatus {
    pub steps: u64,
    /// Steps in which a model could not read or write a point or move its part.
    pub errors: u64,
    pub last_error: Option<String>,
}

/// A part's models, bound to their points, stepped by their caller.
pub struct Models {
    dt: f64,
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
            })
        };
        Ok(Models {
            dt: step.as_secs_f64(),
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
                        stroke: f64::from(a.stroke_ms) / 1000.0,
                        retracted_switch: bind_opt(&a.retracted_switch)?,
                        extended_switch: bind_opt(&a.extended_switch)?,
                        position: 0.0,
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
                        on_delay: f64::from(m.on_delay_ms) / 1000.0,
                        off_delay: f64::from(m.off_delay_ms) / 1000.0,
                        occupied: false,
                        pending: 0.0,
                    })
                })
                .collect::<Result<_, ReflexoError>>()?,
            status: ReflexoStatus::default(),
        })
    }

    /// One step of every model, in this order: drives, conveyors, actuators, sensors, each kind
    /// in the file's order. A model that cannot read or write a point is skipped for the step, and
    /// counted.
    pub fn step(&mut self, mut world: Option<&mut World>) {
        let dt = self.dt;
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
        for a in &mut self.actuators {
            let extend = match a.extend.read() {
                Ok(v) => v != 0.0,
                Err(e) => {
                    errors.push(format!("actuator `{}`: {e}", a.name));
                    continue;
                }
            };
            let delta = dt / a.stroke;
            a.position = if extend { (a.position + delta).min(1.0) } else { (a.position - delta).max(0.0) };
            if let Some(w) = world.as_deref_mut()
                && let Err(e) = w.place(&a.mover, lerp(a.retracted, a.extended, a.position))
            {
                errors.push(format!("actuator `{}`: {e}", a.name));
            }
            for (switch, at) in [(&a.retracted_switch, a.position == 0.0), (&a.extended_switch, a.position == 1.0)] {
                if let Some(s) = switch
                    && let Err(e) = s.write(f64::from(u8::from(at)))
                {
                    errors.push(format!("actuator `{}`: {e}", a.name));
                }
            }
        }
        for s in &mut self.sensors {
            let inside = world.as_deref().and_then(|w| w.sensor(&s.sensor)).is_some_and(|x| x.occupied());
            if inside == s.occupied {
                s.pending = 0.0;
            } else {
                s.pending += dt;
                let delay = if inside { s.on_delay } else { s.off_delay };
                // The first step at or after the delay (a millionth of a step's slack for the sum).
                if s.pending + dt * 1e-6 >= delay {
                    s.occupied = inside;
                    s.pending = 0.0;
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
        self.actuators.iter().find(|a| a.name == name).map(|a| a.position)
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
            if !run.stop.load(Ordering::SeqCst) {
                run.models().step(Some(world));
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
                if run.stop.load(Ordering::SeqCst) {
                    break;
                }
                run.models().step(None);
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

    /// Stops the models: no step begins after this returns.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
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

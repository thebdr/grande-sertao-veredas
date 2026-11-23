//! The world: a scene's parts in Rapier, stepped by its caller at the scene's fixed step. The only
//! module that names the engine (`docs/questions.md` Q160).

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

use rapier3d::glamx::{EulerRot, Quat};
use rapier3d::prelude::*;

use super::FisicaError;
use super::scene::{self, Body, Placement, Scene};

/// The most sensor events a world keeps unread; past it the oldest go, counted.
pub const MAX_EVENTS: usize = 65_536;

/// The penetration a contact is allowed before the solver pushes back, in metres: a millimetre,
/// for parts measured in centimetres (Rapier's own default is 5 mm).
const ALLOWED_PENETRATION: f32 = 0.001;

/// What a part of a scene is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Fixed,
    Belt,
    Body,
    Mover,
    Sensor,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Fixed => "fixed shape",
            Kind::Belt => "belt",
            Kind::Body => "body",
            Kind::Mover => "mover",
            Kind::Sensor => "sensor",
        }
    }
}

/// A body's or a mover's state after the last step.
#[derive(Debug, Clone, PartialEq)]
pub struct BodyState {
    pub name: String,
    /// `Kind::Body` or `Kind::Mover`.
    pub kind: Kind,
    /// Its centre, in metres.
    pub at: [f32; 3],
    /// Its rotation, a unit quaternion (x, y, z, w).
    pub rotation: [f32; 4],
    /// Its centre's velocity, in m/s.
    pub velocity: [f32; 3],
}

impl BodyState {
    /// Its rotation as the scene file states one: yaw, pitch and roll in degrees.
    pub fn angles(&self) -> [f32; 3] {
        let [x, y, z, w] = self.rotation;
        let (yaw, pitch, roll) = Quat::from_xyzw(x, y, z, w).to_euler(EulerRot::ZYX);
        [yaw.to_degrees(), pitch.to_degrees(), roll.to_degrees()]
    }
}

/// A sensor's state after the last step: the bodies and movers inside it, by name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorState {
    pub name: String,
    pub inside: Vec<String>,
}

impl SensorState {
    /// Something is inside it.
    pub fn occupied(&self) -> bool {
        !self.inside.is_empty()
    }
}

/// A body or a mover entering or leaving a sensor, at the step that saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorEvent {
    /// The step after which it was inside (entered) or no longer inside (left).
    pub step: u64,
    pub sensor: String,
    pub body: String,
    pub entered: bool,
}

struct BeltPart {
    collider: ColliderHandle,
    /// The belt's own x and z in the world.
    axis: Vector,
    up: Vector,
    speed: f64,
}

struct SensorPart {
    inside: BTreeSet<String>,
}

/// The belts' surfaces: each contact on a belt's top face gets the belt's speed as the tangent
/// velocity the solver aims for; its sides stay still.
struct BeltHooks {
    belts: HashMap<ColliderHandle, (Vector, Vector)>,
}

impl PhysicsHooks for BeltHooks {
    fn modify_solver_contacts(&self, ctx: &mut ContactModificationContext) {
        let (belt, first) = match (self.belts.get(&ctx.collider1), self.belts.get(&ctx.collider2)) {
            (Some(b), _) => (*b, true),
            (None, Some(b)) => (*b, false),
            (None, None) => return,
        };
        let (velocity, up) = belt;
        let Some(m) = ctx.rigid_mut() else { return };
        // The normal points from the pair's first collider to its second: turned to point out of
        // the belt, it is the belt's up on its top face alone (level on a side, down underneath).
        let out = if first { *m.normal } else { -*m.normal };
        if out.dot(up) < 0.7 {
            return;
        }
        // The tangent velocity is the second collider's, relative to the first: the belt's speed
        // when the belt is first, the opposite when it is second.
        let v = if first { velocity } else { -velocity };
        for c in m.solver_contacts.iter_mut() {
            c.tangent_velocity = v;
        }
    }
}

/// A scene in Rapier, stepped by its caller.
pub struct World {
    w: PhysicsWorld,
    step_ms: u32,
    steps: u64,
    kinds: BTreeMap<String, Kind>,
    belts: BTreeMap<String, BeltPart>,
    moving: BTreeMap<String, (RigidBodyHandle, Kind)>,
    owners: HashMap<ColliderHandle, String>,
    sensors: BTreeMap<String, SensorPart>,
    sensor_of: HashMap<ColliderHandle, String>,
    events: VecDeque<SensorEvent>,
    dropped_events: u64,
    collisions: Receiver<CollisionEvent>,
    collector: ChannelEventCollector,
    _forces: Receiver<ContactForceEvent>,
    _tears: Receiver<SoftBodyTearEvent>,
}

/// An angle in degrees as the engine takes it: reduced to a turn in double precision first, so a
/// huge finite angle is the angle it names, never NaN.
fn radians(deg: f64) -> f32 {
    deg.rem_euclid(360.0).to_radians() as f32
}

fn rotation(yaw: f64, pitch: f64, roll: f64) -> Quat {
    Quat::from_euler(EulerRot::ZYX, radians(yaw), radians(pitch), radians(roll))
}

fn pose(at: [f64; 3], yaw: f64, pitch: f64, roll: f64) -> Pose {
    Pose::from_parts(Vector::new(at[0] as f32, at[1] as f32, at[2] as f32), rotation(yaw, pitch, roll))
}

fn half(size: [f64; 3]) -> (f32, f32, f32) {
    ((size[0] / 2.0) as f32, (size[1] / 2.0) as f32, (size[2] / 2.0) as f32)
}

impl World {
    /// A world holding the scene's parts, at step 0. The scene is checked first.
    pub fn new(scene: &Scene) -> Result<World, FisicaError> {
        scene::validate(scene)?;
        let mut w = PhysicsWorld::new();
        w.gravity = Vector::new(0.0, 0.0, -(scene.gravity as f32));
        w.integration_parameters.dt = scene.step_ms as f32 / 1000.0;
        w.integration_parameters.normalized_allowed_linear_error = ALLOWED_PENETRATION;
        let ground = w.insert_body(RigidBodyBuilder::fixed());
        let (ctx, collisions) = channel();
        let (ftx, forces) = channel();
        let (ttx, tears) = channel();
        let mut world = World {
            w,
            step_ms: scene.step_ms,
            steps: 0,
            kinds: BTreeMap::new(),
            belts: BTreeMap::new(),
            moving: BTreeMap::new(),
            owners: HashMap::new(),
            sensors: BTreeMap::new(),
            sensor_of: HashMap::new(),
            events: VecDeque::new(),
            dropped_events: 0,
            collisions,
            collector: ChannelEventCollector::new(ctx, ftx, ttx),
            _forces: forces,
            _tears: tears,
        };
        // The bodies go in first, so a body the file holds meets a belt as the pair's first
        // collider, and one added later as its second: the belt's hook serves both orders.
        for b in &scene.bodies {
            world.insert_body(b);
        }
        for f in &scene.fixed {
            let (x, y, z) = half(f.size);
            world.w.insert_collider(ColliderBuilder::cuboid(x, y, z).position(pose(f.at, f.yaw, f.pitch, f.roll)).friction(f.friction as f32), Some(ground));
            world.kinds.insert(f.name.clone(), Kind::Fixed);
        }
        for b in &scene.belts {
            let (x, y, z) = half(b.size);
            let rot = rotation(b.yaw, b.pitch, b.roll);
            let collider = world.w.insert_collider(
                ColliderBuilder::cuboid(x, y, z)
                    .position(pose(b.at, b.yaw, b.pitch, b.roll))
                    .friction(b.friction as f32)
                    .active_hooks(ActiveHooks::MODIFY_SOLVER_CONTACTS),
                Some(ground),
            );
            world.belts.insert(b.name.clone(), BeltPart { collider, axis: rot * Vector::X, up: rot * Vector::Z, speed: b.speed });
            world.kinds.insert(b.name.clone(), Kind::Belt);
        }
        for m in &scene.movers {
            let (x, y, z) = half(m.size);
            let body = world.w.insert_body(RigidBodyBuilder::kinematic_position_based().pose(pose(m.at, m.yaw, m.pitch, m.roll)));
            let collider = world.w.insert_collider(ColliderBuilder::cuboid(x, y, z).friction(m.friction as f32), Some(body));
            world.owners.insert(collider, m.name.clone());
            world.moving.insert(m.name.clone(), (body, Kind::Mover));
            world.kinds.insert(m.name.clone(), Kind::Mover);
        }
        for s in &scene.sensors {
            let shape = match (s.size, s.radius) {
                (Some(size), _) => {
                    let (x, y, z) = half(size);
                    ColliderBuilder::cuboid(x, y, z)
                }
                (None, Some(r)) => ColliderBuilder::ball(r as f32),
                (None, None) => unreachable!("validated: a sensor has a size or a radius"),
            };
            let collider = world.w.insert_collider(
                shape
                    .position(pose(s.at, s.yaw, s.pitch, s.roll))
                    .sensor(true)
                    .active_events(ActiveEvents::COLLISION_EVENTS)
                    // A fixed sensor sees the movers too, which Rapier's defaults leave out.
                    .active_collision_types(ActiveCollisionTypes::default() | ActiveCollisionTypes::KINEMATIC_FIXED),
                Some(ground),
            );
            world.sensor_of.insert(collider, s.name.clone());
            world.sensors.insert(s.name.clone(), SensorPart { inside: BTreeSet::new() });
            world.kinds.insert(s.name.clone(), Kind::Sensor);
        }
        Ok(world)
    }

    fn insert_body(&mut self, b: &Body) {
        let (x, y, z) = half(b.size);
        let body = self.w.insert_body(RigidBodyBuilder::dynamic().pose(pose(b.at, b.yaw, b.pitch, b.roll)));
        let collider = self.w.insert_collider(ColliderBuilder::cuboid(x, y, z).mass(b.mass as f32).friction(b.friction as f32), Some(body));
        self.owners.insert(collider, b.name.clone());
        self.moving.insert(b.name.clone(), (body, Kind::Body));
        self.kinds.insert(b.name.clone(), Kind::Body);
    }

    /// One fixed step: the bodies move, the movers reach the poses they were given, and every
    /// sensor's change is kept as an event of this step.
    pub fn step(&mut self) {
        // A body asleep never meets its contacts' solver: one resting on a belt that runs slowly
        // (under about 0.1 m/s the engine puts it to sleep) would stop. Every body touching a
        // running belt is woken first.
        let mut wake = Vec::new();
        for b in self.belts.values().filter(|b| b.speed != 0.0) {
            for pair in self.w.contact_pairs_with(b.collider) {
                if !pair.has_any_active_contact() {
                    continue;
                }
                let other = if pair.collider1 == b.collider { pair.collider2 } else { pair.collider1 };
                if let Some(body) = self.w.colliders.get(other).and_then(|c| c.parent()) {
                    wake.push(body);
                }
            }
        }
        for body in wake {
            self.w.wake_up(body, true);
        }
        let hooks = BeltHooks { belts: self.belts.values().map(|b| (b.collider, (b.axis * b.speed as f32, b.up))).collect() };
        self.w.step_with_events(&hooks, &self.collector);
        self.steps += 1;
        let mut seen = Vec::new();
        while let Ok(e) = self.collisions.try_recv() {
            let (a, b) = (e.collider1(), e.collider2());
            let (sensor, other) = match (self.sensor_of.get(&a), self.sensor_of.get(&b)) {
                (Some(s), None) => (s, b),
                (None, Some(s)) => (s, a),
                _ => continue,
            };
            // A collider no longer owned (a body removed, its leaving already counted) is skipped.
            let Some(body) = self.owners.get(&other) else { continue };
            seen.push(SensorEvent { step: self.steps, sensor: sensor.clone(), body: body.clone(), entered: e.started() });
        }
        // Within a step, by sensor and then by body: the engine's own order is not the scene's.
        seen.sort_by(|x, y| (&x.sensor, &x.body).cmp(&(&y.sensor, &y.body)));
        for e in seen {
            let Some(part) = self.sensors.get_mut(&e.sensor) else { continue };
            let changed = if e.entered { part.inside.insert(e.body.clone()) } else { part.inside.remove(&e.body) };
            if changed {
                self.push_event(e);
            }
        }
    }

    fn push_event(&mut self, e: SensorEvent) {
        if self.events.len() == MAX_EVENTS {
            self.events.pop_front();
            self.dropped_events += 1;
        }
        self.events.push_back(e);
    }

    /// The steps taken.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The fixed step.
    pub fn step_length(&self) -> Duration {
        Duration::from_millis(self.step_ms as u64)
    }

    /// The scene's time: the steps taken times the step.
    pub fn time(&self) -> Duration {
        Duration::from_millis(u64::from(self.step_ms).saturating_mul(self.steps))
    }

    /// What a name is in this scene.
    pub fn kind(&self, name: &str) -> Option<Kind> {
        self.kinds.get(name).copied()
    }

    /// A body's or a mover's state.
    pub fn body(&self, name: &str) -> Option<BodyState> {
        let (handle, kind) = self.moving.get(name)?;
        let b = self.w.bodies.get(*handle)?;
        let t = b.translation();
        let r = b.rotation();
        let v = b.linvel();
        Some(BodyState { name: name.to_string(), kind: *kind, at: [t.x, t.y, t.z], rotation: [r.x, r.y, r.z, r.w], velocity: [v.x, v.y, v.z] })
    }

    /// Every body's and mover's state, by name.
    pub fn bodies(&self) -> Vec<BodyState> {
        self.moving.keys().filter_map(|n| self.body(n)).collect()
    }

    /// A sensor's state.
    pub fn sensor(&self, name: &str) -> Option<SensorState> {
        let s = self.sensors.get(name)?;
        Some(SensorState { name: name.to_string(), inside: s.inside.iter().cloned().collect() })
    }

    /// Every sensor's state, by name.
    pub fn sensors(&self) -> Vec<SensorState> {
        self.sensors.keys().filter_map(|n| self.sensor(n)).collect()
    }

    /// The sensor events not read yet, oldest first; reading them takes them.
    pub fn take_events(&mut self) -> Vec<SensorEvent> {
        self.events.drain(..).collect()
    }

    /// Events lost because more than [`MAX_EVENTS`] waited unread.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// A belt's surface speed.
    pub fn speed(&self, belt: &str) -> Option<f64> {
        self.belts.get(belt).map(|b| b.speed)
    }

    /// Sets a belt's surface speed, from the next step on.
    pub fn set_speed(&mut self, belt: &str, speed: f64) -> Result<(), FisicaError> {
        let Some(b) = self.belts.get_mut(belt) else { return Err(self.unknown(Kind::Belt, belt)) };
        scene::check_speed(&format!("belt `{belt}`"), speed)?;
        b.speed = speed;
        // A body asleep on a belt that starts again is woken, or it would stay where it stopped.
        self.w.wake_up_all(true);
        Ok(())
    }

    /// Places a mover: it reaches the pose at the next step, moving at the speed that takes it
    /// there in one step, and pushes what is in its way.
    pub fn place(&mut self, mover: &str, to: Placement) -> Result<(), FisicaError> {
        let handle = match self.moving.get(mover) {
            Some((h, Kind::Mover)) => *h,
            _ => return Err(self.unknown(Kind::Mover, mover)),
        };
        scene::check_pose(&format!("mover `{mover}`"), to.at, [to.yaw, to.pitch, to.roll])?;
        if let Some(b) = self.w.bodies.get_mut(handle) {
            b.set_next_kinematic_position(pose(to.at, to.yaw, to.pitch, to.roll));
        }
        Ok(())
    }

    /// Adds a body, from the next step on.
    pub fn add_body(&mut self, b: &Body) -> Result<(), FisicaError> {
        let names: Vec<(String, &'static str)> = self.kinds.iter().map(|(n, k)| (n.clone(), k.word())).collect();
        let place = format!("body `{}`", b.name);
        scene::check_name(&names, "body", &place, &b.name)?;
        scene::check_body(&place, b)?;
        self.insert_body(b);
        Ok(())
    }

    /// Removes a body: every sensor it was in sees it leave at this step.
    pub fn remove_body(&mut self, name: &str) -> Result<(), FisicaError> {
        let handle = match self.moving.get(name) {
            Some((h, Kind::Body)) => *h,
            _ => return Err(self.unknown(Kind::Body, name)),
        };
        let mut left = Vec::new();
        for (sensor, part) in self.sensors.iter_mut() {
            if part.inside.remove(name) {
                left.push(SensorEvent { step: self.steps, sensor: sensor.clone(), body: name.to_string(), entered: false });
            }
        }
        for e in left {
            self.push_event(e);
        }
        if let Some(b) = self.w.bodies.get(handle) {
            for c in b.colliders().to_vec() {
                self.owners.remove(&c);
            }
        }
        self.w.remove_body_with_colliders(handle, true);
        self.moving.remove(name);
        self.kinds.remove(name);
        Ok(())
    }

    fn unknown(&self, kind: Kind, name: &str) -> FisicaError {
        match self.kinds.get(name) {
            Some(other) => FisicaError::Invalid(format!("`{name}` is a {}, not a {}", other.word(), kind.word())),
            None => FisicaError::Unknown { kind: kind.word(), name: name.to_string() },
        }
    }
}

//! `fisica`: the bodies of a plant's scene and their motion, headless, at a fixed step
//! (`docs/maquinar-plan.md`, "`fisica`, as designed before its code"; the scene file is
//! `docs/cena-schema.md`).
//!
//! A [`World`] is pure: its caller steps it, so a test pins the physics without sleeping. [`start`]
//! runs one on a thread of its own in real time, behind a [`FisicaHandle`] that reads every pose
//! and sensor and moves what the plant side moves (a belt's speed, a mover's pose, a body added
//! or taken away). The same scene and the same inputs give the same run, bit for bit.

pub mod scene;
mod world;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub use scene::{Belt, Body, Fixed, Mover, Placement, Scene, Sensor, load};
pub use world::{BodyState, Kind, MAX_EVENTS, SensorEvent, SensorState, World};

/// Why a scene was refused, or a change to it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FisicaError {
    #[error("not a scene file: {0}")]
    NotToml(String),
    #[error("`format` is `{0}`, not `veredas-cena`")]
    WrongFormat(String),
    #[error("version {0} of `veredas-cena` is not known here (this reader knows 1)")]
    UnknownVersion(u32),
    /// The scene or the change is wrong, with its place named.
    #[error("{0}")]
    Invalid(String),
    /// No part of that kind has the name.
    #[error("no {kind} is named `{name}`")]
    Unknown { kind: &'static str, name: String },
    /// The thread that runs the scene could not start.
    #[error("the scene's thread did not start: {0}")]
    Thread(String),
}

/// What the runner has done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FisicaStatus {
    /// The steps taken.
    pub steps: u64,
    /// The steps begun more than one step after their time, the machine being busy.
    pub late: u64,
    pub bodies: usize,
    pub sensors: usize,
}

struct Shared {
    world: Mutex<World>,
    stop: AtomicBool,
    late: AtomicU64,
}

impl Shared {
    fn world(&self) -> MutexGuard<'_, World> {
        self.world.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A scene running in real time. Dropping it stops the scene.
pub struct FisicaHandle {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// Starts a scene on a thread of its own, one step per step of real time: a step late is taken at
/// once, never skipped, so the scene's time is the steps taken times the step.
pub fn start(scene: &Scene) -> Result<FisicaHandle, FisicaError> {
    let world = World::new(scene)?;
    let step = world.step_length();
    let shared = Arc::new(Shared { world: Mutex::new(world), stop: AtomicBool::new(false), late: AtomicU64::new(0) });
    let run = Arc::clone(&shared);
    let thread = std::thread::Builder::new()
        .name("fisica".into())
        .spawn(move || {
            let _timer = crate::timer::FineTimer::hold();
            let begun = Instant::now();
            let mut k: u32 = 0;
            while !run.stop.load(Ordering::SeqCst) {
                k = k.saturating_add(1);
                let due = begun + step * k;
                let now = Instant::now();
                if due > now {
                    std::thread::sleep(due - now);
                } else if now - due > step {
                    run.late.fetch_add(1, Ordering::Relaxed);
                }
                if run.stop.load(Ordering::SeqCst) {
                    break;
                }
                run.world().step();
            }
        })
        .map_err(|e| FisicaError::Thread(e.to_string()))?;
    Ok(FisicaHandle { shared, thread: Some(thread) })
}

impl FisicaHandle {
    /// Reads the world under its lock, between two steps.
    pub fn with<T>(&self, f: impl FnOnce(&World) -> T) -> T {
        f(&self.shared.world())
    }

    pub fn steps(&self) -> u64 {
        self.shared.world().steps()
    }

    pub fn time(&self) -> Duration {
        self.shared.world().time()
    }

    pub fn body(&self, name: &str) -> Option<BodyState> {
        self.shared.world().body(name)
    }

    pub fn bodies(&self) -> Vec<BodyState> {
        self.shared.world().bodies()
    }

    pub fn sensor(&self, name: &str) -> Option<SensorState> {
        self.shared.world().sensor(name)
    }

    pub fn sensors(&self) -> Vec<SensorState> {
        self.shared.world().sensors()
    }

    /// The sensor events not read yet; reading them takes them.
    pub fn take_events(&self) -> Vec<SensorEvent> {
        self.shared.world().take_events()
    }

    pub fn speed(&self, belt: &str) -> Option<f64> {
        self.shared.world().speed(belt)
    }

    pub fn set_speed(&self, belt: &str, speed: f64) -> Result<(), FisicaError> {
        self.shared.world().set_speed(belt, speed)
    }

    pub fn place(&self, mover: &str, to: Placement) -> Result<(), FisicaError> {
        self.shared.world().place(mover, to)
    }

    pub fn add_body(&self, body: &Body) -> Result<(), FisicaError> {
        self.shared.world().add_body(body)
    }

    pub fn remove_body(&self, name: &str) -> Result<(), FisicaError> {
        self.shared.world().remove_body(name)
    }

    pub fn status(&self) -> FisicaStatus {
        let w = self.shared.world();
        let bodies = w.bodies().iter().filter(|b| b.kind == Kind::Body).count();
        FisicaStatus { steps: w.steps(), late: self.shared.late.load(Ordering::Relaxed), bodies, sensors: w.sensors().len() }
    }

    /// Stops the scene and waits for its thread; the world keeps the state of its last step.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }

    /// The thread has ended.
    pub fn stopped(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
}

impl Drop for FisicaHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

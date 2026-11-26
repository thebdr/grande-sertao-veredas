//! C-114: `fisica`, a plant's scene stepped at a fixed step: the reference's worked example run as
//! its prose says, the physics held to numbers worked out by hand, every refusal by its place, the
//! same run twice bit for bit, and the runner in real time.

use std::time::{Duration, Instant};

use veredas_maquinar::fisica::{self, Body, BodyState, FisicaError, Kind, Placement, Scene, SensorEvent, World};

const G: f64 = 9.81;

/// The reference's worked example, as the test finds it in `docs/cena-schema.md`.
fn reference() -> String {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cena-schema.md")).unwrap();
    let start = doc.find("```toml\n").unwrap() + "```toml\n".len();
    let end = start + doc[start..].find("```").unwrap();
    doc[start..end].to_string()
}

fn scene(body: &str) -> Scene {
    fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap()
}

fn world(body: &str) -> World {
    World::new(&scene(body)).unwrap()
}

fn at(w: &World, name: &str) -> [f64; 3] {
    w.body(name).unwrap().at
}

fn steps(w: &mut World, n: u64) {
    for _ in 0..n {
        w.step();
    }
}

/// Steps until `done`, keeping each step's pose of `name`; at most `max` steps.
/// Records each step's start as its hook sees it (the steps taken so far, the instant), the world in
/// hand: where the runner judges a step late.
fn step_starts(h: &fisica::FisicaHandle) -> std::sync::Arc<std::sync::Mutex<Vec<(u64, Instant)>>> {
    let starts = std::sync::Arc::new(std::sync::Mutex::new(Vec::with_capacity(4096)));
    let st = std::sync::Arc::clone(&starts);
    h.add_hook(Box::new(move |w: &mut World| st.lock().unwrap().push((w.steps(), Instant::now()))));
    starts
}

/// Of the steps from the `from`-th on, how many began more than one step after their time: surely
/// (their lateness read against the earliest start, at most the real one) and perhaps (within a
/// tenth of a step of it: the earliest start is late by a twentieth of a millisecond or so). The
/// machine's load decides how many; the runner must count them, and no more.
fn begun_late(starts: &[(u64, Instant)], step: Duration, from: u64) -> (usize, usize) {
    let base = starts.iter().map(|(n, t)| *t - step * (*n as u32 + 1)).min().unwrap();
    let lateness = |(n, t): &(u64, Instant)| t.saturating_duration_since(base + step * (*n as u32 + 1));
    let window: Vec<Duration> = starts.iter().filter(|(n, _)| *n >= from).map(lateness).collect();
    (window.iter().filter(|l| **l > step).count(), window.iter().filter(|l| **l > step - step / 10).count())
}

fn run_until(w: &mut World, name: &str, max: u64, mut done: impl FnMut(&mut World, &[SensorEvent]) -> bool) -> (Vec<[f64; 3]>, Vec<SensorEvent>) {
    let mut poses = vec![at(w, name)];
    for _ in 0..max {
        w.step();
        poses.push(at(w, name));
        let events = w.take_events();
        if done(w, &events) {
            return (poses, events);
        }
    }
    panic!("not done within {max} steps");
}

#[test]
fn the_documented_scene_loads_with_its_numbers_and_runs_as_the_reference_says() {
    let s = fisica::load(&reference()).unwrap();
    assert_eq!((s.step_ms, s.gravity), (4, 9.81));
    assert_eq!((s.fixed.len(), s.belts.len(), s.bodies.len(), s.movers.len(), s.sensors.len()), (1, 1, 1, 1, 2));
    assert_eq!((s.belts[0].name.as_str(), s.belts[0].speed, s.belts[0].size, s.belts[0].friction), ("infeed", 0.5, [4.0, 0.6, 0.1], 0.6));
    assert_eq!((s.bodies[0].name.as_str(), s.bodies[0].mass, s.bodies[0].at), ("carton-1", 2.5, [0.3, 0.0, 0.95]));
    assert_eq!((s.sensors[0].size, s.sensors[0].radius), (Some([0.01, 0.8, 0.05]), None));
    assert_eq!((s.sensors[1].size, s.sensors[1].radius), (None, Some(0.05)));
    assert_eq!((s.movers[0].name.as_str(), s.movers[0].yaw, s.movers[0].friction), ("pusher", 0.0, 0.6));

    let mut w = World::new(&s).unwrap();
    assert_eq!((w.steps(), w.time()), (0, Duration::ZERO));
    assert_eq!([w.kind("floor"), w.kind("infeed"), w.kind("carton-1"), w.kind("pusher"), w.kind("B1"), w.kind("nothing")], [Some(Kind::Fixed), Some(Kind::Belt), Some(Kind::Body), Some(Kind::Mover), Some(Kind::Sensor), None]);
    // Fixed shapes and belts are never inside a sensor, though the barrier crosses the belt.
    steps(&mut w, 1);
    assert!(w.sensors().iter().all(|s| !s.occupied()), "{:?}", w.sensors());
    // The belt takes the carton to its speed within a tenth of a second, standing on it.
    steps(&mut w, 24);
    assert_eq!(w.time(), Duration::from_millis(100));
    let c = w.body("carton-1").unwrap();
    assert!((c.velocity[0] - 0.5).abs() < 0.005 && c.velocity[1].abs() < 0.005, "{c:?}");
    assert!((c.at[2] - 0.95).abs() < 0.0015, "{c:?}");
    // B1 sees it when its front edge reaches the beam, about 3.1 s in: one step after the first
    // step whose pose overlaps the beam, as the engine detects before it moves.
    let (poses, events) = run_until(&mut w, "carton-1", 1000, |_, e| !e.is_empty());
    assert_eq!(events, [SensorEvent { step: w.steps(), sensor: "B1".into(), body: "carton-1".into(), entered: true }]);
    let n = poses.len();
    assert!(poses[n - 2][0] + 0.15 >= 1.995 && poses[n - 3][0] + 0.15 < 1.995, "{:?}", &poses[n - 3..]);
    let t = w.time().as_secs_f64();
    assert!((3.0..3.25).contains(&t), "{t}");
    assert_eq!(w.sensor("B1").unwrap().inside, ["carton-1"]);
    // It leaves when its back edge has passed the beam, one step late the same way.
    let (poses, events) = run_until(&mut w, "carton-1", 1000, |_, e| !e.is_empty());
    assert_eq!(events, [SensorEvent { step: w.steps(), sensor: "B1".into(), body: "carton-1".into(), entered: false }]);
    let n = poses.len();
    assert!(poses[n - 2][0] - 0.15 > 2.005 && poses[n - 3][0] - 0.15 <= 2.005, "{:?}", &poses[n - 3..]);
    assert!(!w.sensor("B1").unwrap().occupied());
    // In front of the pusher the belt stops; the pusher sweeps across and the carton falls off
    // the belt's far side onto the floor. S1 sees the blade at the end of its stroke.
    run_until(&mut w, "carton-1", 2000, |w, _| at(w, "carton-1")[0] >= 3.5);
    w.set_speed("infeed", 0.0).unwrap();
    steps(&mut w, 25);
    for i in 1..=125 {
        let y = -0.4 + 0.65 * i as f64 / 125.0;
        w.place("pusher", Placement::at([3.5, y, 0.95])).unwrap();
        w.step();
    }
    let p = w.body("pusher").unwrap();
    assert!((p.at[1] - 0.25).abs() < 1e-5 && p.kind == Kind::Mover, "{p:?}");
    assert!(w.sensor("S1").unwrap().inside.contains(&"pusher".to_string()), "{:?}", w.sensor("S1"));
    steps(&mut w, 500);
    let c = w.body("carton-1").unwrap();
    assert!((c.at[2] - 0.15).abs() < 0.02 && c.at[1] > 0.3, "on the floor beyond the belt: {c:?}");
}

#[test]
fn a_free_fall_keeps_to_gravity_within_a_step_and_a_body_comes_to_rest_within_a_millimetre() {
    let mut w = world("[[body]]\nname = \"ball\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 100.0]\n");
    steps(&mut w, 250);
    let b = w.body("ball").unwrap();
    let t = w.time().as_secs_f64();
    assert_eq!(t, 1.0);
    // The velocity is g t exactly as the steps add it; the drop is g t²/2 to within the step's
    // share an integrator that moves after it accelerates takes (g dt t / 2).
    assert!((b.velocity[2] + G * t).abs() < 1e-3, "{b:?}");
    let exact = 100.0 - G * t * t / 2.0;
    assert!((b.at[2] - exact).abs() <= G * 0.004 * t / 2.0 + 1e-4, "{} against {exact}", b.at[2]);
    assert_eq!((b.at[0], b.at[1], b.velocity[0], b.velocity[1]), (0.0, 0.0, 0.0, 0.0));
    // No gravity: it stays.
    let mut w = world("gravity = 0\n[[body]]\nname = \"ball\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1.0]\n");
    steps(&mut w, 250);
    assert_eq!(at(&w, "ball"), [0.0, 0.0, 1.0]);
    // Dropped 5 cm onto a floor whose top is at z = 0: at rest on it within a millimetre.
    let mut w = world(
        "[[fixed]]\nname = \"floor\"\nsize = [2.0, 2.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
         [[body]]\nname = \"box\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.15]\nmass = 5\n",
    );
    steps(&mut w, 250);
    let b = w.body("box").unwrap();
    assert!((b.at[2] - 0.1).abs() <= 0.001 && b.velocity.iter().all(|v| v.abs() < 1e-3), "{b:?}");
}

#[test]
fn a_belt_carries_what_lies_on_it_either_way_round_and_leaves_what_touches_its_side() {
    let text = "[[fixed]]\nname = \"floor\"\nsize = [20.0, 4.0, 0.1]\nat = [0.0, 0.0, -0.05]\n\
                [[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 0.0, 0.25]\nspeed = 0.5\n\
                [[belt]]\nname = \"cross\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 5.0, 0.25]\nyaw = 90\nspeed = 0.4\n\
                [[body]]\nname = \"a\"\nsize = [0.2, 0.2, 0.2]\nat = [-2.0, 0.0, 0.6]\n\
                [[body]]\nname = \"side\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.401, 0.1]\n\
                [[body]]\nname = \"c\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 4.0, 0.6]\n";
    let mut w = world(text);
    // `a` is the file's, so it meets the belt as the pair's first collider; `b`, added now, as its
    // second. Both are carried along the belt's x at its speed.
    w.add_body(&Body::new("b", [0.2, 0.2, 0.2], [0.0, 0.0, 0.6])).unwrap();
    steps(&mut w, 75);
    for name in ["a", "b"] {
        let s = w.body(name).unwrap();
        assert!((s.velocity[0] - 0.5).abs() < 0.005 && s.velocity[1].abs() < 0.005, "{s:?}");
    }
    // The belt yawed 90° carries along the world's y, its own x.
    let c = w.body("c").unwrap();
    assert!((c.velocity[1] - 0.4).abs() < 0.005 && c.velocity[0].abs() < 0.005, "{c:?}");
    // A body standing on the floor against the belt's side is not carried.
    let s = w.body("side").unwrap();
    assert!(s.velocity[0].abs() < 0.01 && s.at[0].abs() < 0.001, "{s:?}");
    // Backwards, then stopped: the bodies follow, and stay where it stopped them.
    w.set_speed("belt", -0.3).unwrap();
    assert_eq!(w.speed("belt"), Some(-0.3));
    steps(&mut w, 75);
    for name in ["a", "b"] {
        let s = w.body(name).unwrap();
        assert!((s.velocity[0] + 0.3).abs() < 0.005, "{s:?}");
    }
    w.set_speed("belt", 0.0).unwrap();
    steps(&mut w, 75);
    let held = at(&w, "a");
    steps(&mut w, 250);
    let s = w.body("a").unwrap();
    assert!(s.velocity[0].abs() < 1e-3 && (s.at[0] - held[0]).abs() < 0.001, "{s:?}");
    // Started again after it slept, it carries again.
    w.set_speed("belt", 0.5).unwrap();
    steps(&mut w, 75);
    assert!((w.body("a").unwrap().velocity[0] - 0.5).abs() < 0.005);
}

#[test]
fn a_mover_reaches_each_pose_at_the_next_step_and_pushes_without_being_pushed() {
    let mut w = world(
        "[[fixed]]\nname = \"floor\"\nsize = [10.0, 10.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
         [[mover]]\nname = \"blade\"\nsize = [0.05, 1.0, 0.4]\nat = [-1.0, 0.0, 0.2]\n\
         [[body]]\nname = \"heavy\"\nsize = [0.4, 0.4, 0.4]\nat = [0.0, 0.0, 0.2]\nmass = 1000\n",
    );
    steps(&mut w, 25);
    // Gravity leaves it where it is.
    let b = w.body("blade").unwrap();
    assert_eq!((b.at, b.kind), ([-1.0, 0.0, 0.2], Kind::Mover));
    // Each pose reached at the next step, at the speed that takes it there; the tonne in its way
    // is pushed and the blade keeps to its poses.
    for i in 1..=250 {
        let x = -1.0 + 1.5 * i as f64 / 250.0;
        w.place("blade", Placement::at([x, 0.0, 0.2])).unwrap();
        w.step();
        let b = w.body("blade").unwrap();
        assert!((b.at[0] - x).abs() < 1e-5 && b.at[1] == 0.0 && b.at[2] == 0.2, "{i}: {b:?}");
    }
    // Ahead of the blade's face (0.525) at the blade's speed, 1.5 m/s, its back face into the
    // blade within 7 mm: one step's travel (6 mm) and a millimetre (2.04 mm measured).
    let h = w.body("heavy").unwrap();
    assert!(h.at[0] - 0.2 >= 0.525 - 0.001 - 0.006 && h.at[0] - 0.2 <= 0.525 + 0.001, "pushed ahead of the blade: {h:?}");
    assert!((h.velocity[0] - 1.5).abs() < 0.05, "{h:?}");
    // Turned: the pose's angles come back as given.
    w.place("blade", Placement { at: [-3.0, 0.0, 0.2], yaw: 30.0, pitch: 0.0, roll: 0.0 }).unwrap();
    w.step();
    let a = w.body("blade").unwrap().angles();
    assert!((a[0] - 30.0).abs() < 1e-3 && a[1].abs() < 1e-3 && a[2].abs() < 1e-3, "{a:?}");
}

/// A slab pitched by `angle` degrees and a body resting on its top face; how far the body moves in
/// a second.
fn slide(angle: f64) -> f64 {
    let p = angle.to_radians();
    // The slab's top normal is its own z turned by the pitch about y: (sin p, 0, cos p).
    let (nx, nz) = (p.sin(), p.cos());
    let d = 0.05 + 0.1 + 0.0005;
    let text = format!(
        "[[fixed]]\nname = \"slab\"\nsize = [4.0, 1.0, 0.1]\nat = [0.0, 0.0, 0.0]\npitch = {angle}\n\
         [[body]]\nname = \"part\"\nsize = [0.2, 0.2, 0.2]\nat = [{}, 0.0, {}]\npitch = {angle}\n",
        nx * d,
        nz * d
    );
    let mut w = world(&text);
    steps(&mut w, 25);
    let start = at(&w, "part");
    steps(&mut w, 250);
    let end = at(&w, "part");
    ((end[0] - start[0]).powi(2) + (end[2] - start[2]).powi(2)).sqrt()
}

#[test]
fn friction_holds_a_body_on_a_gentle_slope_and_lets_it_slide_on_a_steep_one() {
    // Friction 0.6 holds up to atan(0.6), 31°.
    let held = slide(20.0);
    assert!(held < 0.002, "{held}");
    let slid = slide(40.0);
    // On 40° it accelerates at g (sin 40° - 0.6 cos 40°), about 1.80 m/s². Measured from 0.1 s to
    // 1.1 s, it covers a (0.1 × 1 + 1²/2), about 1.08 m.
    let a = G * (40f64.to_radians().sin() - 0.6 * 40f64.to_radians().cos());
    let expected = a * (0.1 + 0.5);
    assert!((slid - expected).abs() < 0.03 * expected, "{slid} against {expected}");
}

#[test]
fn sensors_see_bodies_and_movers_in_order_and_a_body_taken_away_leaves_them() {
    let mut w = world(
        "gravity = 0\n\
         [[sensor]]\nname = \"zone\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n\
         [[sensor]]\nname = \"ball\"\nradius = 0.2\nat = [0.0, 0.0, 0.0]\n\
         [[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [5.0, 0.0, 0.0]\n",
    );
    // Two bodies inside both sensors from the start: seen at the first step, by sensor then body.
    w.add_body(&Body::new("y", [0.1, 0.1, 0.1], [0.0, 0.0, 0.0])).unwrap();
    w.add_body(&Body::new("x", [0.1, 0.1, 0.1], [0.05, 0.0, 0.0])).unwrap();
    w.step();
    let ev = |sensor: &str, body: &str, step: u64, entered: bool| SensorEvent { step, sensor: sensor.into(), body: body.into(), entered };
    assert_eq!(w.take_events(), [ev("ball", "x", 1, true), ev("ball", "y", 1, true), ev("zone", "x", 1, true), ev("zone", "y", 1, true)]);
    assert_eq!(w.take_events(), []);
    // The mover enters too: placed, it is there after step 2 and seen at step 3.
    w.place("m", Placement::at([0.3, 0.0, 0.0])).unwrap();
    w.step();
    assert_eq!(w.take_events(), []);
    w.step();
    assert_eq!(w.take_events(), [ev("zone", "m", 3, true)]);
    assert_eq!(w.sensor("zone").unwrap().inside, ["m", "x", "y"]);
    // A body taken away leaves every sensor at once, at the step it was taken.
    w.remove_body("x").unwrap();
    assert_eq!(w.take_events(), [ev("ball", "x", 3, false), ev("zone", "x", 3, false)]);
    steps(&mut w, 3);
    assert_eq!(w.take_events(), []);
    assert_eq!(w.sensor("zone").unwrap().inside, ["m", "y"]);
    assert_eq!(w.body("x"), None);
    assert_eq!(w.kind("x"), None);
    // Its name is free again.
    w.add_body(&Body::new("x", [0.1, 0.1, 0.1], [3.0, 0.0, 0.0])).unwrap();
    // Unread events past the cap: the oldest go, counted.
    assert_eq!(w.dropped_events(), 0);
    assert_eq!(fisica::MAX_EVENTS, 65_536);
}

/// A run of the reference with every kind of input, its states after each step and its events.
fn scripted_run() -> (Vec<Vec<BodyState>>, Vec<SensorEvent>) {
    let mut w = World::new(&fisica::load(&reference()).unwrap()).unwrap();
    let mut states = Vec::new();
    let mut events = Vec::new();
    for i in 0..1500u32 {
        match i {
            300 => w.add_body(&Body { mass: 4.0, yaw: 10.0, ..Body::new("carton-2", [0.3, 0.4, 0.2], [0.3, 0.0, 1.2]) }).unwrap(),
            600 => w.set_speed("infeed", 0.8).unwrap(),
            900 => w.remove_body("carton-1").unwrap(),
            1000..=1100 => w.place("pusher", Placement::at([3.5, -0.4 + 0.0065 * (i - 1000) as f64, 0.95])).unwrap(),
            _ => {}
        }
        w.step();
        states.push(w.bodies());
        events.extend(w.take_events());
    }
    (states, events)
}

fn bits(states: &[Vec<BodyState>]) -> Vec<(String, [u64; 10])> {
    states
        .iter()
        .flatten()
        .map(|b| {
            let v: Vec<u64> = b.at.iter().chain(b.rotation.iter()).chain(b.velocity.iter()).map(|f| f.to_bits()).collect();
            (b.name.clone(), v.try_into().unwrap())
        })
        .collect()
}

#[test]
fn the_same_scene_and_inputs_give_the_same_run_bit_for_bit() {
    let (a, ea) = scripted_run();
    let (b, eb) = scripted_run();
    assert!(!ea.is_empty());
    assert_eq!(ea, eb);
    assert_eq!(bits(&a), bits(&b));
    // Two worlds stepped in turns share nothing.
    let s = fisica::load(&reference()).unwrap();
    let (mut one, mut two) = (World::new(&s).unwrap(), World::new(&s).unwrap());
    two.set_speed("infeed", 0.2).unwrap();
    for _ in 0..500 {
        one.step();
        two.step();
    }
    let mut alone = World::new(&s).unwrap();
    steps(&mut alone, 500);
    assert_eq!(bits(&[one.bodies()]), bits(&[alone.bodies()]));
    assert_ne!(bits(&[two.bodies()]), bits(&[alone.bodies()]));
}

#[test]
fn every_refusal_is_named_by_its_place() {
    let refused = |body: &str| fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap_err();
    let invalid = |body: &str| match refused(body) {
        FisicaError::Invalid(m) => m,
        other => panic!("{body}: {other:?}"),
    };
    // The header, as core's reader states it.
    assert!(matches!(fisica::load("this is [ not toml"), Err(FisicaError::NotToml(_))));
    assert_eq!(fisica::load("format = \"veredas-bench\"\nversion = 1\n"), Err(FisicaError::WrongFormat("veredas-bench".into())));
    assert_eq!(fisica::load("format = \"veredas-cena\"\nversion = 2\n"), Err(FisicaError::UnknownVersion(2)));
    assert_eq!(FisicaError::UnknownVersion(2).to_string(), "version 2 of `veredas-cena` is not known here (this reader knows 1)");
    assert_eq!(FisicaError::WrongFormat("x".into()).to_string(), "`format` is `x`, not `veredas-cena`");
    let no_version = fisica::load("format = \"veredas-cena\"\n").unwrap_err();
    assert!(no_version.to_string().contains("version"), "{no_version}");
    // A misspelled key in every table.
    for body in [
        "step = 4\n",
        "[[fixed]]\nname = \"f\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nfriccion = 1\n",
        "[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nvelocity = 1\n",
        "[[body]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nweight = 1\n",
        "[[mover]]\nname = \"m\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nspeed = 1\n",
        "[[sensor]]\nname = \"s\"\nradius = 1.0\nat = [0.0, 0.0, 0.0]\nrange = 1\n",
    ] {
        match refused(body) {
            FisicaError::NotToml(m) => assert!(m.contains("unknown field"), "{m}"),
            other => panic!("{body}: {other:?}"),
        }
    }
    let part = |kind: &str, extra: &str| format!("[[{kind}]]\nname = \"p\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n{extra}");
    let cases: Vec<(String, &str)> = vec![
        ("step_ms = 0\n".into(), "step_ms: 0, not 1 to 100"),
        ("step_ms = 101\n".into(), "step_ms: 101, not 1 to 100"),
        ("gravity = -1.0\n".into(), "gravity: -1, not 0 to 100 m/s²"),
        ("gravity = 100.5\n".into(), "gravity: 100.5, not 0 to 100 m/s²"),
        ("gravity = nan\n".into(), "gravity: NaN, not 0 to 100 m/s²"),
        ("[[fixed]]\nname = \"\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n".into(), "fixed[0]: the fixed has no name"),
        ("[[belt]]\nname = \" b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n".into(), "belt[0]: the name ` b` has surrounding whitespace"),
        (format!("{}{}", part("body", ""), part("sensor", "").replace("size = [1.0, 1.0, 1.0]", "radius = 1.0")), "sensor[0]: `p` is already the name of a body"),
        (part("fixed", "").replace("[1.0, 1.0, 1.0]", "[1.0, 0.0, 1.0]"), "fixed[0] (`p`).size: [1.0, 0.0, 1.0] m, each side from 0.001 to 10000"),
        (part("belt", "").replace("[1.0, 1.0, 1.0]", "[-1.0, 1.0, 1.0]"), "belt[0] (`p`).size: [-1.0, 1.0, 1.0] m, each side from 0.001 to 10000"),
        (part("body", "").replace("[1.0, 1.0, 1.0]", "[10001.0, 1.0, 1.0]"), "body[0] (`p`).size: [10001.0, 1.0, 1.0] m, each side from 0.001 to 10000"),
        (part("mover", "").replace("at = [0.0, 0.0, 0.0]", "at = [0.0, 0.0, -10001.0]"), "mover[0] (`p`).at: [0.0, 0.0, -10001.0] m, each coordinate within 10000 of the origin"),
        (part("fixed", "yaw = inf\n"), "fixed[0] (`p`).yaw: inf, not a finite angle"),
        (part("fixed", "roll = nan\n"), "fixed[0] (`p`).roll: NaN, not a finite angle"),
        (part("fixed", "friction = -0.1\n"), "fixed[0] (`p`).friction: -0.1, not 0 to 10"),
        (part("mover", "friction = 11\n"), "mover[0] (`p`).friction: 11, not 0 to 10"),
        (part("body", "mass = 0\n"), "body[0] (`p`).mass: 0 kg, not from 0.000001 to 100000"),
        (part("body", "mass = 100001\n"), "body[0] (`p`).mass: 100001 kg, not from 0.000001 to 100000"),
        (part("belt", "speed = -100.5\n"), "belt[0] (`p`).speed: -100.5 m/s, not within 100 either way"),
        (part("sensor", "radius = 1.0\n"), "sensor[0] (`p`): a sensor is a box (`size`) or a ball (`radius`), not both"),
        ("[[sensor]]\nname = \"s\"\nat = [0.0, 0.0, 0.0]\n".into(), "sensor[0] (`s`): a sensor is a box (`size`) or a ball (`radius`); it has neither"),
        ("[[sensor]]\nname = \"s\"\nradius = 0.0\nat = [0.0, 0.0, 0.0]\n".into(), "sensor[0] (`s`).radius: 0 m, not from 0.001 to 10000"),
        ("[[sensor]]\nname = \"s\"\nradius = 1.0\nat = [0.0, 0.0, 0.0]\npitch = inf\n".into(), "sensor[0] (`s`).pitch: inf, not a finite angle"),
    ];
    for (body, message) in cases {
        assert_eq!(invalid(&body), message, "{body}");
    }
    // The emptiest scene is one.
    assert_eq!(fisica::load("format = \"veredas-cena\"\nversion = 1\n").unwrap(), Scene::default());
    // A change refused while the scene runs, by name and by kind.
    let mut w = World::new(&fisica::load(&reference()).unwrap()).unwrap();
    assert_eq!(w.set_speed("belt", 1.0), Err(FisicaError::Unknown { kind: "belt", name: "belt".into() }));
    assert_eq!(w.set_speed("carton-1", 1.0), Err(FisicaError::Invalid("`carton-1` is a body, not a belt".into())));
    assert_eq!(w.set_speed("infeed", 101.0), Err(FisicaError::Invalid("belt `infeed`.speed: 101 m/s, not within 100 either way".into())));
    assert_eq!(w.place("floor", Placement::at([0.0; 3])), Err(FisicaError::Invalid("`floor` is a fixed shape, not a mover".into())));
    assert_eq!(w.place("pusher", Placement::at([f64::NAN, 0.0, 0.0])), Err(FisicaError::Invalid("mover `pusher`.at: [NaN, 0.0, 0.0] m, each coordinate within 10000 of the origin".into())));
    assert_eq!(w.add_body(&Body::new("B1", [0.1; 3], [0.0; 3])), Err(FisicaError::Invalid("body `B1`: `B1` is already the name of a sensor".into())));
    assert_eq!(w.add_body(&Body { mass: -1.0, ..Body::new("n", [0.1; 3], [0.0; 3]) }), Err(FisicaError::Invalid("body `n`.mass: -1 kg, not from 0.000001 to 100000".into())));
    assert_eq!(w.remove_body("pusher"), Err(FisicaError::Invalid("`pusher` is a mover, not a body".into())));
    assert_eq!(w.remove_body("ghost"), Err(FisicaError::Unknown { kind: "body", name: "ghost".into() }));
    assert_eq!(FisicaError::Unknown { kind: "body", name: "ghost".into() }.to_string(), "no body is named `ghost`");
    // Nothing a refusal touched changed.
    assert_eq!(w.speed("infeed"), Some(0.5));
    assert_eq!(w.kind("n"), None);
    assert!(World::new(&Scene { step_ms: 0, ..Scene::default() }).is_err());
}

#[test]
fn the_runner_steps_in_real_time_and_its_handle_reads_and_moves_the_scene() {
    let scene = fisica::load(&reference()).unwrap();
    let mut h = fisica::start(&scene).unwrap();
    let starts = step_starts(&h);
    // One step per 4 ms of real time, a late step taken at once, never skipped: measured between
    // two readings while it runs, so the world's making and the thread's start count for nothing.
    std::thread::sleep(Duration::from_millis(100));
    let (first, begun, late0) = (h.steps(), Instant::now(), h.status().late);
    std::thread::sleep(Duration::from_millis(1000));
    let (steps, elapsed, late) = (h.steps() - first, begun.elapsed(), h.status().late - late0);
    let due = (elapsed.as_millis() / 4) as u64;
    assert!(steps + 3 >= due && steps <= due + 2, "{steps} steps in {elapsed:?}");
    // Read under one lock: a step between two reads would part them.
    let (time, taken) = h.with(|w| (w.time(), w.steps()));
    assert_eq!(time, Duration::from_millis(4 * taken));
    let s = h.status();
    assert_eq!((s.bodies, s.sensors), (1, 2));
    // A step counted late only past one step behind its time (refute round 2: every step counted
    // late passed before), judged against the steps that began so in this second, whatever the
    // machine's load made of it (the orchestrator's whole-suite run: 6 counted, a bound of 5).
    let (surely, perhaps) = begun_late(&starts.lock().unwrap(), Duration::from_millis(4), first);
    assert!(late as usize + 3 >= surely && late as usize <= perhaps + 3, "{late} of {steps} steps counted late, {surely} to {perhaps} began late");
    // The handle reads and moves the scene as the world does.
    let c = h.body("carton-1").unwrap();
    assert!((c.velocity[0] - 0.5).abs() < 0.01, "{c:?}");
    h.set_speed("infeed", 0.0).unwrap();
    assert_eq!(h.speed("infeed"), Some(0.0));
    h.place("pusher", Placement::at([3.5, 0.25, 0.95])).unwrap();
    h.add_body(&Body::new("dropped", [0.1; 3], [-2.0, 0.0, 0.5])).unwrap();
    std::thread::sleep(Duration::from_millis(300));
    assert!((h.body("pusher").unwrap().at[1] - 0.25).abs() < 1e-5);
    assert!(h.sensor("S1").unwrap().inside.contains(&"pusher".to_string()));
    assert!(h.take_events().iter().any(|e| e.sensor == "S1" && e.body == "pusher" && e.entered));
    assert_eq!(h.bodies().len(), 3);
    assert!(h.with(|w| w.body("dropped").unwrap().at[2]) < 0.5);
    h.remove_body("dropped").unwrap();
    assert_eq!(h.status().bodies, 1);
    assert_eq!(h.sensors().len(), 2);
    // Stopped: no more steps, the last state kept.
    h.stop();
    assert!(h.stopped());
    let after = h.steps();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(h.steps(), after);
    assert!(h.body("carton-1").is_some());
    // Dropped while running: its thread ends.
    let h = fisica::start(&scene).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    drop(h);
    // A scene refused does not start.
    assert!(fisica::start(&Scene { gravity: -1.0, ..Scene::default() }).is_err());
}

#[test]
fn a_paused_scene_takes_no_step_till_it_runs_and_its_hooks_run_from_its_first() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    let scene = fisica::load(&reference()).unwrap();
    let mut h = fisica::paused(&scene).unwrap();
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!((h.steps(), h.stopped()), (0, true), "paused: no step, no thread");
    // A hook added while paused sees every step from the first: its calls before a step are the
    // steps taken.
    let (calls, off) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicBool::new(false)));
    let (c, o) = (Arc::clone(&calls), Arc::clone(&off));
    // The threads the hook ran on: one, the scene's (a second run starting another would show).
    let threads = Arc::new(std::sync::Mutex::new(std::collections::BTreeSet::new()));
    let t = Arc::clone(&threads);
    h.add_hook(Box::new(move |w: &mut World| {
        t.lock().unwrap().insert(format!("{:?}", std::thread::current().id()));
        if c.fetch_add(1, Ordering::SeqCst) != w.steps() {
            o.store(true, Ordering::SeqCst);
        }
    }));
    h.run().unwrap();
    h.run().unwrap();
    let begun = Instant::now();
    while h.steps() < 10 {
        assert!(begun.elapsed() < Duration::from_secs(5), "running");
        std::thread::sleep(Duration::from_millis(1));
    }
    h.stop();
    assert!(!off.load(Ordering::SeqCst));
    assert_eq!(calls.load(Ordering::SeqCst), h.steps(), "one call before each step, a second run nothing");
    let threads = threads.lock().unwrap().clone();
    assert!(threads.len() == 1 && !threads.contains(&format!("{:?}", std::thread::current().id())), "{threads:?}");
    // Stopped, it never runs again; a paused scene stopped never runs.
    h.run().unwrap();
    let mut p = fisica::paused(&scene).unwrap();
    p.stop();
    p.run().unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!((calls.load(Ordering::SeqCst), h.steps(), p.steps()), (h.steps(), h.steps(), 0));
}

/// C-114's first refute round: the critic's probes, kept as pins (the event cap and its order, the
/// runner's late steps, drop, hooks, friction's mean, mass, a belt's side and underside, slow
/// belts, movers and lifts, the angle order and ranges, any finite angle, removals' order, sleep).
mod round_1 {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use veredas_maquinar::fisica::{self, Placement, World};

    use super::{begun_late, step_starts};

    fn scene(body: &str) -> fisica::Scene {
        fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap()
    }
    fn world(body: &str) -> World {
        World::new(&scene(body)).unwrap()
    }
    fn steps(w: &mut World, n: u64) {
        for _ in 0..n {
            w.step();
        }
    }

    /// P1: more than 65 536 unread events: exactly the newest 65 536 kept, the oldest dropped, counted.
    #[test]
    fn the_event_cap_drops_the_oldest_and_counts_them() {
        let mut text = String::from("gravity = 0\n");
        let n = 260;
        for i in 0..n {
            text += &format!("[[sensor]]\nname = \"s{i:03}\"\nsize = [100.0, 100.0, 100.0]\nat = [0.0, 0.0, 0.0]\n");
        }
        for i in 0..n {
            text += &format!("[[body]]\nname = \"b{i:03}\"\nsize = [0.1, 0.1, 0.1]\nat = [{}, {}, 0.0]\n", (i % 16) as f64, (i / 16) as f64);
        }
        let mut w = world(&text);
        w.step();
        let total = n * n;
        let ev = w.take_events();
        println!("P1 events kept {} dropped {} of {total}", ev.len(), w.dropped_events());
        assert_eq!(ev.len(), fisica::MAX_EVENTS);
        assert_eq!(w.dropped_events() as usize, total - fisica::MAX_EVENTS);
        // The oldest dropped: by sensor then body, the first kept is the (total - cap)-th.
        let first = total - fisica::MAX_EVENTS;
        assert_eq!((ev[0].sensor.clone(), ev[0].body.clone()), (format!("s{:03}", first / n), format!("b{:03}", first % n)));
        assert_eq!((ev.last().unwrap().sensor.as_str(), ev.last().unwrap().body.as_str()), ("s259", "b259"));
    }

    /// P2: a late step taken at once and counted, never skipped: a hook stalls the scene's thread
    /// 200 ms once; after it the runner catches up to the clock and counts the late steps.
    #[test]
    fn a_stalled_runner_catches_up_and_counts_its_late_steps() {
        let s = scene("[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n");
        let h = fisica::start(&s).unwrap();
        let starts = step_starts(&h);
        std::thread::sleep(Duration::from_millis(100));
        let (first, begun, late0) = (h.steps(), Instant::now(), h.status().late);
        let mut once = false;
        h.add_hook(Box::new(move |_w| {
            if !once {
                once = true;
                std::thread::sleep(Duration::from_millis(200));
            }
        }));
        std::thread::sleep(Duration::from_millis(1000));
        let (n, elapsed) = (h.steps() - first, begun.elapsed());
        let due = (elapsed.as_millis() / 4) as u64;
        let late = h.status().late - late0;
        println!("P2 steps {n} due {due} late {late} in {elapsed:?}");
        assert!(n + 3 >= due && n <= due + 2, "caught up: {n} steps against {due}");
        // The 50 steps the stall put behind, counted late, and no more than began late (refute
        // round 2: a count of only steps nine behind, or of every step, passed a floor alone); the
        // steps the machine's load put behind besides are judged as the runner test judges them
        // (a fixed 46 to 55 failed once beside a build, 2026-11-25).
        let (surely, perhaps) = begun_late(&starts.lock().unwrap(), Duration::from_millis(4), first);
        assert!(late >= 46, "the steps the stall put behind counted late: {late}");
        assert!(late as usize + 3 >= surely && late as usize <= perhaps + 3, "{late} of {n} steps counted late, {surely} to {perhaps} began late");
    }

    /// P3: dropped while running, its thread ends: the hooks it ran stop being called and are dropped.
    #[test]
    fn drop_ends_the_thread() {
        let s = scene("");
        let h = fisica::start(&s).unwrap();
        let calls = Arc::new(AtomicU64::new(0));
        let c = Arc::clone(&calls);
        h.add_hook(Box::new(move |_w| {
            c.fetch_add(1, Ordering::SeqCst);
        }));
        std::thread::sleep(Duration::from_millis(50));
        assert!(calls.load(Ordering::SeqCst) > 0);
        drop(h);
        let a = calls.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(100));
        let b = calls.load(Ordering::SeqCst);
        println!("P3 calls at drop {a}, 100 ms later {b}, strong count {}", Arc::strong_count(&calls));
        assert_eq!(a, b, "the hook still runs after drop");
        assert_eq!(Arc::strong_count(&calls), 1, "the thread still holds the hook");
    }

    /// P4: a hook runs before each step from the next on: its first call sees the steps taken when it
    /// was added (taken under the world's lock, so no step can come between).
    #[test]
    fn a_hook_runs_before_the_step() {
        let s = scene("");
        let h = fisica::start(&s).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        let seen = Arc::new(AtomicU64::new(u64::MAX));
        let sn = Arc::clone(&seen);
        let added_at = h.with(|w| {
            let n = w.steps();
            h.add_hook(Box::new(move |w: &mut World| {
                let _ = sn.compare_exchange(u64::MAX, w.steps(), Ordering::SeqCst, Ordering::SeqCst);
            }));
            n
        });
        std::thread::sleep(Duration::from_millis(40));
        let first = seen.load(Ordering::SeqCst);
        println!("P4 added at {added_at}, first call saw {first}");
        assert_eq!(first, added_at, "the hook ran after a step, not before it");
    }

    /// The slab-and-part slide of the repository's test, with frictions of the critic's choosing.
    fn slide(angle: f64, slab_friction: f64, part_friction: f64) -> f64 {
        let p = angle.to_radians();
        let (nx, nz) = (p.sin(), p.cos());
        let d = 0.05 + 0.1 + 0.0005;
        let mut w = world(&format!(
            "[[fixed]]\nname = \"slab\"\nsize = [4.0, 1.0, 0.1]\nat = [0.0, 0.0, 0.0]\npitch = {angle}\nfriction = {slab_friction}\n\
             [[body]]\nname = \"part\"\nsize = [0.2, 0.2, 0.2]\nat = [{}, 0.0, {}]\npitch = {angle}\nfriction = {part_friction}\n",
            nx * d,
            nz * d
        ));
        steps(&mut w, 25);
        let a = w.body("part").unwrap().at;
        steps(&mut w, 250);
        let b = w.body("part").unwrap().at;
        ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
    }

    /// P5: a contact's friction is the mean of its two: 0.2 on 1.0 is 0.6, which holds on 20°.
    #[test]
    fn friction_is_the_mean_of_the_two() {
        let held = slide(20.0, 1.0, 0.2);
        let held2 = slide(20.0, 0.2, 1.0);
        let slid = slide(40.0, 1.0, 0.2);
        let g = 9.81f64;
        let a = g * (40f64.to_radians().sin() - 0.6 * 40f64.to_radians().cos());
        let expected = a * (0.1 + 0.5);
        println!("P5 held {held} {held2} slid {slid} expected {expected}");
        assert!(held < 0.002 && held2 < 0.002, "{held} {held2}");
        assert!((slid - expected).abs() < 0.03 * expected, "{slid} against {expected}");
    }

    /// P6: a body's mass: a 1 kg body thrown at 1 m/s into a 99 kg one at rest, no gravity and no
    /// friction, nothing bounces: they go on together at 1/100 m/s.
    #[test]
    fn a_body_moves_with_its_mass() {
        let mut w = world(
            "gravity = 0\n\
             [[mover]]\nname = \"bat\"\nsize = [0.05, 0.5, 0.5]\nat = [-0.2, 0.0, 0.0]\nfriction = 0\n\
             [[body]]\nname = \"light\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.0]\nmass = 1\nfriction = 0\n\
             [[body]]\nname = \"heavy\"\nsize = [0.2, 0.2, 0.2]\nat = [1.0, 0.0, 0.0]\nmass = 99\nfriction = 0\n",
        );
        // The bat hits the light body at 1 m/s and stops after 0.2 s.
        for i in 1..=50 {
            w.place("bat", Placement::at([-0.2 + 0.004 * i as f64, 0.0, 0.0])).unwrap();
            w.step();
            if i % 10 == 0 {
                println!("P6 step {i}: bat {:?} light {:?}", w.body("bat").unwrap().at, w.body("light").unwrap());
            }
        }
        w.place("bat", Placement::at([-0.2, 0.0, 2.0])).unwrap();
        // The light body reaches the heavy one about 0.7 s on; read both 0.3 s after, before either
        // slows under Rapier's sleep threshold.
        steps(&mut w, 250);
        let (l, h) = (w.body("light").unwrap(), w.body("heavy").unwrap());
        println!("P6 light {:?} heavy {:?}", l.velocity, h.velocity);
        assert!(h.at[0] > 1.0005 && (h.velocity[0] - 0.0105).abs() < 0.003, "{h:?}");
    }

    /// P7: a part pressed against a belt's side is not carried: a frictionless floor slopes 20° down
    /// toward the belt, so the part leans on the belt's side with a share of its weight.
    #[test]
    fn a_part_pressed_on_a_belt_side_is_not_carried() {
        // The slab rolled 20° about x: its top rises along +y. Its top passes z = 0.05 at y = 0.3.
        let r = 20f64.to_radians();
        let (ny, nz) = (-r.sin(), r.cos());
        // A point on the top face at y = 0.3, z = 0.05; the slab's centre 0.05 below it along -n.
        let (cy, cz) = (0.3 - 0.05 * ny + 0.7 * r.cos(), 0.05 - 0.05 * nz + 0.7 * r.sin());
        // The part's centre 0.1 above the top along n, at y = 0.6 up the slope.
        let (py, pz) = (0.3 + 0.35 * r.cos() + 0.1005 * ny, 0.05 + 0.35 * r.sin() + 0.1005 * nz);
        let text = format!(
            "[[fixed]]\nname = \"slope\"\nsize = [8.0, 1.4, 0.1]\nat = [0.0, {cy}, {cz}]\nroll = 20\nfriction = 0\n\
             [[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 0.0, 0.25]\nspeed = 0.5\nfriction = 1.2\n\
             [[body]]\nname = \"part\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, {py}, {pz}]\nroll = 20\nfriction = 0\n"
        );
        let mut w = world(&text);
        steps(&mut w, 250);
        let a = w.body("part").unwrap();
        steps(&mut w, 250);
        let b = w.body("part").unwrap();
        println!("P7 after 1 s {a:?}\nP7 after 2 s {b:?}");
        assert!(a.at[1] < 0.45, "the part reached the belt's side: {a:?}");
        assert!(b.at[0].abs() < 0.01 && b.velocity[0].abs() < 0.01, "carried by the belt's side: {b:?}");
    }

    /// P9: the belt's own +z is its top: a belt rolled 180° has its top face down, so a part lying on
    /// its upward face lies on its bottom and is not carried.
    #[test]
    fn a_belt_carries_on_its_top_face_only() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 0.0, 0.25]\nroll = 180\nspeed = 0.5\n\
             [[body]]\nname = \"part\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.6]\n",
        );
        steps(&mut w, 250);
        let b = w.body("part").unwrap();
        println!("P9 rolled-over belt: {b:?}");
        assert!(b.velocity[0].abs() < 0.01, "carried by the belt's bottom face: {b:?}");
    }

    /// P9b: a part lifted against a belt's underside by a table (a mover) is not carried by it.
    #[test]
    fn a_part_pressed_up_against_a_belt_underside_is_not_carried() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 1.05]\nspeed = 0.5\nfriction = 1.2\n\
             [[mover]]\nname = \"table\"\nsize = [0.6, 0.6, 0.05]\nat = [0.0, 0.0, 0.5]\nfriction = 0\n\
             [[body]]\nname = \"part\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.626]\nfriction = 0\n",
        );
        steps(&mut w, 50);
        // The table rises until the part's top meets the belt's underside (z = 1.0), then 3 mm more.
        // Part top = table centre + 0.025 + 0.2.
        let top: f64 = 1.0 - 0.2 - 0.025 + 0.003;
        let mut z: f64 = 0.5;
        while z < top {
            z = (z + 0.002).min(top);
            w.place("table", Placement::at([0.0, 0.0, z])).unwrap();
            w.step();
        }
        let a = w.body("part").unwrap();
        steps(&mut w, 250);
        let b = w.body("part").unwrap();
        println!("P9b pressed: {a:?}\nP9b one second on: {b:?}");
        assert!((b.at[0] - a.at[0]).abs() < 0.01, "carried by the belt's underside: {a:?} -> {b:?}");
    }

    /// P10: a carton held by a gate on a running belt falls asleep; the gate opens and the belt,
    /// still running, carries it on.
    #[test]
    fn a_carton_held_at_a_gate_goes_on_when_the_gate_opens() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = 0.5\n\
             [[mover]]\nname = \"gate\"\nsize = [0.02, 0.8, 0.3]\nat = [0.5, 0.0, 0.95]\n\
             [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.95]\nmass = 2.5\n",
        );
        steps(&mut w, 1000);
        let held = w.body("carton").unwrap();
        // The gate goes up out of the way.
        w.place("gate", Placement::at([0.5, 0.0, 2.0])).unwrap();
        steps(&mut w, 250);
        let after = w.body("carton").unwrap();
        println!("P10 held {held:?}\nP10 a second after the gate opened {after:?}");
        assert!(held.velocity[0].abs() < 0.01, "held: {held:?}");
        assert!((after.velocity[0] - 0.5).abs() < 0.01, "not carried after the gate opened: {after:?}");
    }

    /// P10b: the same with the gate moved aside along y in many small steps.
    #[test]
    fn a_carton_held_at_a_gate_goes_on_when_the_gate_slides_away() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = 0.5\n\
             [[mover]]\nname = \"gate\"\nsize = [0.02, 0.8, 0.3]\nat = [0.5, 0.0, 0.95]\n\
             [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.95]\nmass = 2.5\n",
        );
        steps(&mut w, 1000);
        for i in 1..=100 {
            w.place("gate", Placement::at([0.5, 0.01 * i as f64, 0.95])).unwrap();
            w.step();
        }
        steps(&mut w, 250);
        let after = w.body("carton").unwrap();
        println!("P10b {after:?}");
        assert!((after.velocity[0] - 0.5).abs() < 0.01, "not carried after the gate slid away: {after:?}");
    }

    /// P13: a body asleep on another falls when the one under it is taken away.
    #[test]
    fn a_body_left_in_the_air_falls() {
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
             [[body]]\nname = \"under\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.1]\n\
             [[body]]\nname = \"over\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.3]\n",
        );
        steps(&mut w, 750);
        w.remove_body("under").unwrap();
        steps(&mut w, 250);
        let o = w.body("over").unwrap();
        println!("P13 {o:?}");
        assert!((o.at[2] - 0.1).abs() < 0.002, "left in the air: {o:?}");
    }

    /// P11: the pose's angles are intrinsic z-y-x: yaw 90 then pitch 30 about the new y puts the
    /// part's own x along (0, cos 30°, -sin 30°).
    #[test]
    fn the_angles_are_yaw_then_pitch_about_the_new_y_then_roll() {
        let mut w = world("gravity = 0\n[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\nyaw = 90\npitch = 30\n");
        w.step();
        let s = w.body("b").unwrap();
        let [x, y, z, ww] = s.rotation.map(|v| v);
        // The quaternion's rotation of (1, 0, 0): the first column of its matrix.
        let col = [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + ww * z), 2.0 * (x * z - ww * y)];
        let c = 30f64.to_radians();
        println!("P11 own x {col:?} angles {:?}", s.angles());
        assert!((col[0]).abs() < 1e-5 && (col[1] - c.cos()).abs() < 1e-5 && (col[2] + c.sin()).abs() < 1e-5, "{col:?}");
        // And a belt yawed 90 and pitched -10 runs up along +y.
        let mut w = world(
            "[[belt]]\nname = \"ramp\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.0]\nyaw = 90\npitch = -10\nspeed = 0.5\n\
             [[body]]\nname = \"p\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.151]\nyaw = 90\npitch = -10\n",
        );
        steps(&mut w, 250);
        let p = w.body("p").unwrap();
        let t = 10f64.to_radians();
        println!("P11 ramp {p:?}");
        assert!(p.velocity[0].abs() < 0.01 && (p.velocity[1] - 0.5 * t.cos()).abs() < 0.01 && (p.velocity[2] - 0.5 * t.sin()).abs() < 0.01, "{p:?}");
    }

    /// P11b: the angles come back as placed, combined, and past 180°.
    #[test]
    fn the_angles_come_back_as_placed_within_their_ranges() {
        let mut w = world("[[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n");
        w.place("m", Placement { at: [0.0; 3], yaw: 30.0, pitch: 20.0, roll: 10.0 }).unwrap();
        w.step();
        let a = w.body("m").unwrap().angles();
        println!("P11b combined {a:?}");
        assert!((a[0] - 30.0).abs() < 1e-3 && (a[1] - 20.0).abs() < 1e-3 && (a[2] - 10.0).abs() < 1e-3, "{a:?}");
        w.place("m", Placement { at: [0.0; 3], yaw: 200.0, pitch: 0.0, roll: 0.0 }).unwrap();
        w.step();
        // Read back within ±180° for yaw and roll and ±90° for pitch: 200 is -160.
        let a = w.body("m").unwrap().angles();
        assert!((a[0] + 160.0).abs() < 1e-3 && a[1].abs() < 1e-3 && a[2].abs() < 1e-3, "{a:?}");
    }

    /// P12: events within a step by sensor then body, removals between steps included.
    #[test]
    fn a_removal_s_leavings_follow_the_step_s_events() {
        let mut w = world(
            "gravity = 0\n\
             [[sensor]]\nname = \"a\"\nradius = 1.0\nat = [0.0, 0.0, 0.0]\n\
             [[sensor]]\nname = \"b\"\nradius = 1.0\nat = [0.0, 0.0, 0.0]\n\
             [[body]]\nname = \"x\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n\
             [[body]]\nname = \"y\"\nsize = [0.1, 0.1, 0.1]\nat = [0.3, 0.0, 0.0]\n",
        );
        w.step();
        w.remove_body("y").unwrap();
        w.remove_body("x").unwrap();
        let ev: Vec<(u64, String, String, bool)> = w.take_events().into_iter().map(|e| (e.step, e.sensor, e.body, e.entered)).collect();
        println!("P12 {ev:?}");
        // A step's events by sensor then body; a removal's leavings follow them, in the order of
        // removal (y, then x), each by sensor.
        let e = |s: &str, b: &str, entered: bool| (1, s.to_string(), b.to_string(), entered);
        assert_eq!(ev, [e("a", "x", true), e("a", "y", true), e("b", "x", true), e("b", "y", true), e("a", "y", false), e("b", "y", false), e("a", "x", false), e("b", "x", false)]);
    }

    /// P17: a slow belt carries what lies on it toward its speed, however slow.
    #[test]
    fn a_slow_belt_carries_what_lies_on_it() {
        let mut bad = Vec::new();
        for speed in [0.01f64, 0.03, 0.06, 0.1, 0.15, 0.2, 0.3, -0.3, 0.5] {
            let mut w = world(&format!(
                "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = {speed}\n\
                 [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.95]\nmass = 2.5\n"
            ));
            steps(&mut w, 50);
            let a = w.body("carton").unwrap();
            steps(&mut w, 1250);
            let b = w.body("carton").unwrap();
            let moved = b.at[0] - a.at[0];
            println!("P17 belt at {speed} m/s: moved {moved} m in 5 s (expected {}), velocity {:?}", speed * 5.0, b.velocity);
            if (moved as f64 - speed * 5.0).abs() >= 0.02 * speed.abs() * 5.0 + 0.002 {
                bad.push((speed, moved));
            }
        }
        assert!(bad.is_empty(), "belts that stopped carrying (speed, metres moved in 5 s): {bad:?}");
    }

    /// P18: a slow mover pushes, and is not passed through: a blade at 2 cm/s against a carton.
    #[test]
    fn a_slow_mover_pushes() {
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [10.0, 10.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
             [[mover]]\nname = \"blade\"\nsize = [0.05, 1.0, 0.4]\nat = [-0.25, 0.0, 0.2]\n\
             [[body]]\nname = \"carton\"\nsize = [0.4, 0.4, 0.4]\nat = [0.0, 0.0, 0.2]\nmass = 5\n",
        );
        steps(&mut w, 250);
        for i in 1..=1250 {
            w.place("blade", Placement::at([-0.25 + 0.02 * 0.004 * i as f64, 0.0, 0.2])).unwrap();
            w.step();
        }
        let (b, c) = (w.body("blade").unwrap(), w.body("carton").unwrap());
        println!("P18 blade {:?} carton {:?}", b.at, c);
        // The blade's face at blade + 0.025; the carton's back at carton - 0.2.
        assert!(c.at[0] - 0.2 >= b.at[0] + 0.025 - 0.002, "the blade went into the carton: {b:?} {c:?}");
    }

    /// P19: a lift's table rising at 2 cm/s carries the carton on it up.
    #[test]
    fn a_slow_lift_carries_up() {
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [10.0, 10.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
             [[mover]]\nname = \"table\"\nsize = [0.6, 0.6, 0.05]\nat = [0.0, 0.0, 0.025]\n\
             [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.2]\nmass = 5\n",
        );
        steps(&mut w, 250);
        for i in 1..=1250 {
            w.place("table", Placement::at([0.0, 0.0, 0.025 + 0.02 * 0.004 * i as f64])).unwrap();
            w.step();
        }
        let (t, c) = (w.body("table").unwrap(), w.body("carton").unwrap());
        println!("P19 table {:?} carton {:?}", t.at, c);
        assert!(c.at[2] - 0.15 >= t.at[2] + 0.025 - 0.002, "the table went into the carton: {t:?} {c:?}");
    }

    /// P22: a body yawed 1e300 degrees (finite, accepted by the reader) and one yawed 1e10 degrees,
    /// which is 280 degrees.
    #[test]
    fn any_finite_angle_is_the_angle_it_names() {
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
             [[body]]\nname = \"huge\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.3]\nyaw = 1e300\n\
             [[body]]\nname = \"big\"\nsize = [0.2, 0.2, 0.2]\nat = [1.0, 0.0, 0.3]\nyaw = 1e10\n\
             [[body]]\nname = \"plain\"\nsize = [0.2, 0.2, 0.2]\nat = [-1.0, 0.0, 0.3]\nyaw = 280\n",
        );
        w.step();
        println!("P22 after one step: huge {:?}", w.body("huge").unwrap());
        let (big, plain) = (w.body("big").unwrap().angles(), w.body("plain").unwrap().angles());
        assert!((big[0] - plain[0]).abs() < 1e-3, "1e10 degrees is 280: {big:?} against {plain:?}");
        for _ in 0..250 {
            w.step();
        }
        let huge = w.body("huge").unwrap();
        assert!(huge.rotation.iter().all(|v| v.is_finite()) && (huge.at[2] - 0.1).abs() < 0.002, "on the floor, its pose finite: {huge:?}");
        let mut w = world("[[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n");
        w.place("m", Placement { at: [0.0; 3], yaw: 1e300, pitch: 0.0, roll: 0.0 }).unwrap();
        w.step();
        println!("P22 the mover after it: {:?}", w.body("m").unwrap());
        assert!(w.body("m").unwrap().rotation.iter().all(|v| v.is_finite()), "a finite angle made a NaN pose");
    }

    /// P21: a body that falls asleep inside a sensor stays inside it, with no event of its sleep; woken
    /// by its belt starting again, it leaves the sensor once, when it is carried out of it.
    #[test]
    fn a_body_asleep_inside_a_sensor_stays_inside() {
        let mut w = World::new(&fisica::load(
            "format = \"veredas-cena\"\nversion = 1\n\
             [[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\n\
             [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.95]\n\
             [[sensor]]\nname = \"zone\"\nsize = [0.5, 0.5, 0.5]\nat = [0.0, 0.0, 0.95]\n",
        ).unwrap()).unwrap();
        for _ in 0..1000 {
            w.step();
        }
        let ev = w.take_events();
        println!("P21 after 4 s at rest: {ev:?} inside {:?} v {:?}", w.sensor("zone").unwrap().inside, w.body("carton").unwrap().velocity);
        assert_eq!(ev.len(), 1);
        assert_eq!(w.sensor("zone").unwrap().inside, ["carton"]);
        w.set_speed("belt", 0.5).unwrap();
        for _ in 0..500 {
            w.step();
        }
        let ev = w.take_events();
        println!("P21 after the belt ran 2 s: {ev:?} at {:?}", w.body("carton").unwrap().at);
        assert_eq!(ev.iter().filter(|e| !e.entered).count(), 1);
        assert_eq!(ev.len(), 1);
    }


    /// A refused change touches nothing: the pusher stays where it was after a step.
    #[test]
    fn a_refused_place_changes_nothing() {
        let mut w = world("[[mover]]\nname = \"pusher\"\nsize = [0.4, 0.05, 0.3]\nat = [3.5, -0.4, 0.95]\n");
        assert!(w.place("pusher", Placement::at([f64::NAN, 0.0, 0.0])).is_err());
        assert!(w.place("pusher", Placement { at: [0.0; 3], yaw: f64::INFINITY, pitch: 0.0, roll: 0.0 }).is_err());
        w.step();
        assert_eq!(w.body("pusher").unwrap().at, [3.5, -0.4, 0.95]);
    }

    /// The two defaults a part's file states: a body's mass 1 kg, a belt's speed 0.
    #[test]
    fn a_body_s_mass_and_a_belt_s_speed_default_as_the_reference_says() {
        let s = scene("[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n[[body]]\nname = \"c\"\nsize = [1.0, 1.0, 1.0]\nat = [5.0, 0.0, 0.0]\n");
        assert_eq!((s.bodies[0].mass, s.belts[0].speed, s.bodies[0].friction, s.belts[0].friction), (1.0, 0.0, 0.6, 0.6));
    }
}

/// C-114's second refute round: the critic's probes (`~/.cache/veredas/critic-c114-r2/probes`), kept
/// as pins: a mover's velocity, a mover's and a sensor's angles from the file, movers in and out of a
/// sensor, a stack on a slow belt, angles at the poles, a name freed and taken again in a sensor, a
/// value of another kind, a part's index in a refusal, two worlds on two threads, how deep a body
/// rests, and time in 64 bits.
mod round_2 {
    use veredas_maquinar::fisica::{self, Body, FisicaError, Placement, Scene, World};

    fn scene(body: &str) -> Scene {
        fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap()
    }
    fn world(body: &str) -> World {
        World::new(&scene(body)).unwrap()
    }
    fn steps(w: &mut World, n: u64) {
        for _ in 0..n {
            w.step();
        }
    }

    #[test]
    fn a_mover_s_velocity_is_read_while_it_moves_and_0_once_held() {
        let mut w = world("gravity = 0\n[[mover]]\nname = \"blade\"\nsize = [0.05, 1.0, 0.4]\nat = [-1.0, 0.0, 0.2]\n");
        steps(&mut w, 5);
        let mut seen = Vec::new();
        for i in 1..=50 {
            w.place("blade", Placement::at([-1.0 + 1.5 * f64::from(i) / 250.0, 0.0, 0.2])).unwrap();
            w.step();
            seen.push(w.body("blade").unwrap().velocity);
        }
        w.step();
        assert!(seen.iter().all(|v| (v[0] - 1.5).abs() < 1e-3 && v[1] == 0.0 && v[2] == 0.0), "{seen:?}");
        assert_eq!(w.body("blade").unwrap().velocity, [0.0; 3]);
    }

    #[test]
    fn a_mover_and_a_sensor_take_their_angles_from_the_file() {
        let mut w = world("gravity = 0\n[[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\nyaw = 30\npitch = 20\nroll = 10\n");
        let before = w.body("m").unwrap().angles();
        w.step();
        for a in [before, w.body("m").unwrap().angles()] {
            assert!((a[0] - 30.0).abs() < 1e-3 && (a[1] - 20.0).abs() < 1e-3 && (a[2] - 10.0).abs() < 1e-3, "{a:?}");
        }
        // A beam yawed 90°: it holds what lies along y, not along x.
        let mut w = world(
            "gravity = 0\n[[sensor]]\nname = \"beam\"\nsize = [2.0, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\nyaw = 90\n\
             [[body]]\nname = \"on_y\"\nsize = [0.05, 0.05, 0.05]\nat = [0.0, 0.8, 0.0]\n\
             [[body]]\nname = \"on_x\"\nsize = [0.05, 0.05, 0.05]\nat = [0.8, 0.0, 0.0]\n",
        );
        w.step();
        assert_eq!(w.sensor("beam").unwrap().inside, ["on_y"]);
    }

    #[test]
    fn a_mover_inside_a_sensor_is_seen_from_the_start_and_seen_leaving() {
        let mut w = world("[[sensor]]\nname = \"s\"\nradius = 0.5\nat = [0.0, 0.0, 0.0]\n[[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n");
        w.step();
        assert_eq!((w.take_events().len(), w.sensor("s").unwrap().inside), (1, vec!["m".to_string()]));
        // Still inside for 4 s: no leaving; then out: seen one step after it got there.
        steps(&mut w, 1000);
        assert!(w.take_events().is_empty());
        w.place("m", Placement::at([3.0, 0.0, 0.0])).unwrap();
        w.step();
        let first = w.take_events();
        w.step();
        let second = w.take_events();
        assert!(first.is_empty() && second.len() == 1 && !second[0].entered, "{first:?} {second:?}");
    }

    #[test]
    fn a_stack_on_a_slow_belt_rides_together() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = 0.01\n\
             [[body]]\nname = \"under\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, 0.95]\nmass = 2.5\n\
             [[body]]\nname = \"over\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 1.2005]\nmass = 1\n",
        );
        steps(&mut w, 50);
        let (u0, o0) = (w.body("under").unwrap().at, w.body("over").unwrap().at);
        steps(&mut w, 1250);
        let (u1, o1) = (w.body("under").unwrap().at, w.body("over").unwrap().at);
        assert!(((u1[0] - u0[0]) - 0.05).abs() < 0.002, "under {u0:?} {u1:?}");
        assert!(((o1[0] - o0[0]) - (u1[0] - u0[0])).abs() < 0.002, "the top one left behind: {o0:?} {o1:?}");
    }

    #[test]
    fn angles_at_the_poles_and_past_a_turn_read_back_in_range() {
        let mut w = world("[[mover]]\nname = \"m\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n");
        for (y, p, r) in [(30.0, 90.0, 0.0), (30.0, -90.0, 10.0), (0.0, 1e10, 0.0), (-1e-14, 0.0, 0.0), (0.0, 120.0, 0.0), (0.0, 0.0, 190.0), (1e300, 1e300, 1e300)] {
            w.place("m", Placement { at: [0.0; 3], yaw: y, pitch: p, roll: r }).unwrap();
            w.step();
            let a = w.body("m").unwrap().angles();
            assert!(a.iter().all(|v| v.is_finite()) && a[0].abs() <= 180.0 && a[2].abs() <= 180.0 && a[1].abs() <= 90.0, "({y}, {p}, {r}): {a:?}");
        }
    }

    #[test]
    fn a_name_freed_and_taken_again_inside_a_sensor_leaves_then_enters() {
        let mut w = world("gravity = 0\n[[sensor]]\nname = \"s\"\nradius = 0.5\nat = [0.0, 0.0, 0.0]\n[[body]]\nname = \"x\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 0.0]\n");
        w.step();
        assert_eq!(w.take_events().len(), 1);
        w.remove_body("x").unwrap();
        w.add_body(&Body::new("x", [0.1; 3], [0.1, 0.0, 0.0])).unwrap();
        assert_eq!(w.take_events().len(), 1, "the leaving at the removal");
        w.step();
        let next = w.take_events();
        steps(&mut w, 5);
        assert!(next.len() == 1 && next[0].entered && w.take_events().is_empty(), "{next:?}");
        assert_eq!(w.sensor("s").unwrap().inside, ["x"]);
    }

    #[test]
    fn a_value_of_another_kind_or_a_missing_key_is_refused_and_a_refusal_names_the_part_s_index() {
        for body in ["step_ms = -1\n", "step_ms = \"4\"\n", "gravity = \"g\"\n", "[[body]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nmass = \"heavy\"\n"] {
            let e = fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap_err();
            assert!(matches!(e, FisicaError::NotToml(_)), "{body}: {e:?}");
        }
        for body in ["[[body]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\n", "[[body]]\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n", "[[fixed]]\nname = \"f\"\nat = [0.0, 0.0, 0.0]\n"] {
            assert!(matches!(fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")), Err(FisicaError::NotToml(_))), "{body}");
        }
        let e = fisica::load("format = \"veredas-cena\"\nversion = 1\n[[body]]\nname = \"a\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n[[body]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nmass = 0\n").unwrap_err();
        assert_eq!(e.to_string(), "body[1] (`b`).mass: 0 kg, not from 0.000001 to 100000");
    }

    #[test]
    fn two_worlds_on_two_threads_at_once_run_as_one_alone() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cena-schema.md")).unwrap();
        let start = text.find("```toml\n").unwrap() + 8;
        let s = fisica::load(&text[start..start + text[start..].find("```").unwrap()]).unwrap();
        let run = |s: Scene| {
            let mut w = World::new(&s).unwrap();
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for i in 0..1500u32 {
                match i {
                    300 => w.add_body(&Body { mass: 4.0, yaw: 10.0, ..Body::new("carton-2", [0.3, 0.4, 0.2], [0.3, 0.0, 1.2]) }).unwrap(),
                    600 => w.set_speed("infeed", 0.8).unwrap(),
                    900 => w.remove_body("carton-1").unwrap(),
                    1000..=1100 => w.place("pusher", Placement::at([3.5, -0.4 + 0.0065 * f64::from(i - 1000), 0.95])).unwrap(),
                    _ => {}
                }
                w.step();
                for b in w.bodies() {
                    for f in b.at.iter().chain(b.rotation.iter()).chain(b.velocity.iter()) {
                        h = (h ^ f.to_bits()).wrapping_mul(0x100_0000_01b3);
                    }
                }
                for e in w.take_events() {
                    h = (h ^ (e.step << 1 | u64::from(e.entered))).wrapping_mul(0x100_0000_01b3);
                }
            }
            h
        };
        let alone = run(s.clone());
        let (a, b) = (s.clone(), s.clone());
        let (t1, t2) = (std::thread::spawn(move || run(a)), std::thread::spawn(move || run(b)));
        assert_eq!((t1.join().unwrap(), t2.join().unwrap()), (alone, alone));
    }

    #[test]
    fn a_body_rests_on_a_fixed_part_within_a_tenth_of_a_millimetre_and_sinks_into_a_lighter_one() {
        // On the floor, whatever its mass and size: 0.07 mm deep.
        for (mass, size) in [(1.0, 0.2), (100000.0, 0.2), (5.0, 0.02), (100000.0, 1.0)] {
            let half = size / 2.0;
            let mut w = world(&format!(
                "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n[[body]]\nname = \"box\"\nsize = [{size}, {size}, {size}]\nat = [0.0, 0.0, {}]\nmass = {mass}\n",
                half + 0.05
            ));
            steps(&mut w, 500);
            let deep = half - w.body("box").unwrap().at[2];
            assert!((0.0..1e-4).contains(&deep), "{mass} kg of {size} m: {deep} m deep");
        }
        // On a 1 kg box: ten times its mass sinks 2.6 mm into it, a hundred times 26 mm.
        let on_light = |top: f64| {
            let mut w = world(&format!(
                "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
                 [[body]]\nname = \"light\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.1]\nmass = 1\n\
                 [[body]]\nname = \"heavy\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.3005]\nmass = {top}\n"
            ));
            steps(&mut w, 1000);
            let (l, h) = (w.body("light").unwrap().at[2], w.body("heavy").unwrap().at[2]);
            0.2 - (h - l)
        };
        let (ten, hundred) = (on_light(10.0), on_light(100.0));
        assert!((0.002..0.0035).contains(&ten) && (0.02..0.035).contains(&hundred), "{ten} m, {hundred} m");
    }

    #[test]
    fn a_scene_s_time_counts_past_2_to_the_32_steps_and_holds_at_its_most() {
        let past = (1u64 << 32) + 1;
        assert_eq!(fisica::time_at(4, past), std::time::Duration::from_millis(4 * past));
        assert_eq!(fisica::time_at(100, u64::MAX), std::time::Duration::from_millis(u64::MAX));
    }
}

/// C-114's third refute round: the critic's probes (`~/.cache/veredas/critic-c114-r3/probes`), kept
/// as pins: belts with bodies added later (the belt the pair's first collider), every range at its
/// bounds, sensors overlapping a fixed shape and a belt, the late count of a step held past its
/// time, the depth at gravity 100, and the engine as the clause names it.
mod round_3 {
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use veredas_maquinar::fisica::{self, Body, Placement, Scene, World};

    fn scene(body: &str) -> Scene {
        fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap()
    }
    fn world(body: &str) -> World {
        World::new(&scene(body)).unwrap()
    }
    fn steps(w: &mut World, n: u64) {
        for _ in 0..n {
            w.step();
        }
    }

    /// A1: a body added later on a slow belt is carried (the wake serves the belt-first pair).
    #[test]
    fn an_added_body_on_a_slow_belt_is_carried() {
        let mut bad = Vec::new();
        for speed in [0.01f64, 0.03, 0.06, -0.03] {
            let mut w = world(&format!("[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = {speed}\n"));
            w.add_body(&Body { mass: 2.5, ..Body::new("carton", [0.3, 0.3, 0.3], [0.0, 0.0, 0.95]) }).unwrap();
            steps(&mut w, 50);
            let a = w.body("carton").unwrap().at;
            steps(&mut w, 1250);
            let b = w.body("carton").unwrap();
            let moved = b.at[0] - a[0];
            println!("A1 added body, belt {speed} m/s: moved {moved} m in 5 s (expected {}), v {:?}", speed * 5.0, b.velocity);
            if (moved as f64 - speed * 5.0).abs() >= 0.02 * speed.abs() * 5.0 + 0.002 {
                bad.push((speed, moved));
            }
        }
        assert!(bad.is_empty(), "added bodies a slow belt stopped carrying: {bad:?}");
    }

    /// A2: a body added later leaning on a belt's side (a frictionless slope pushing it there) is not
    /// carried.
    #[test]
    fn an_added_body_against_a_belt_side_is_not_carried() {
        let r = 20f64.to_radians();
        let (ny, nz) = (-r.sin(), r.cos());
        let (cy, cz) = (0.3 - 0.05 * ny + 0.7 * r.cos(), 0.05 - 0.05 * nz + 0.7 * r.sin());
        let (py, pz) = (0.3 + 0.35 * r.cos() + 0.1005 * ny, 0.05 + 0.35 * r.sin() + 0.1005 * nz);
        let mut w = world(&format!(
            "[[fixed]]\nname = \"slope\"\nsize = [8.0, 1.4, 0.1]\nat = [0.0, {cy}, {cz}]\nroll = 20\nfriction = 0\n\
             [[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 0.0, 0.25]\nspeed = 0.5\nfriction = 1.2\n"
        ));
        w.add_body(&Body { roll: 20.0, friction: 0.0, ..Body::new("part", [0.2, 0.2, 0.2], [0.0, py, pz]) }).unwrap();
        steps(&mut w, 250);
        let a = w.body("part").unwrap();
        steps(&mut w, 250);
        let b = w.body("part").unwrap();
        println!("A2 added body at the side: after 1 s {a:?}\nA2 after 2 s {b:?}");
        assert!(a.at[1] < 0.45, "the part reached the belt's side: {a:?}");
        assert!(b.at[0].abs() < 0.01 && b.velocity[0].abs() < 0.01, "carried by the belt's side: {b:?}");
    }

    /// A3: a body added later pressed up against a belt's underside by a table is not carried.
    #[test]
    fn an_added_body_under_a_belt_is_not_carried() {
        let mut w = world(
            "[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [0.0, 0.0, 1.05]\nspeed = 0.5\nfriction = 1.2\n\
             [[mover]]\nname = \"table\"\nsize = [0.6, 0.6, 0.05]\nat = [0.0, 0.0, 0.5]\nfriction = 0\n",
        );
        w.add_body(&Body { friction: 0.0, ..Body::new("part", [0.2, 0.2, 0.2], [0.0, 0.0, 0.626]) }).unwrap();
        steps(&mut w, 50);
        let top: f64 = 1.0 - 0.2 - 0.025 + 0.003;
        let mut z: f64 = 0.5;
        while z < top {
            z = (z + 0.002).min(top);
            w.place("table", Placement::at([0.0, 0.0, z])).unwrap();
            w.step();
        }
        let a = w.body("part").unwrap();
        steps(&mut w, 250);
        let b = w.body("part").unwrap();
        println!("A3 added body pressed under the belt: {a:?}\nA3 one second on: {b:?}");
        assert!((b.at[0] - a.at[0]).abs() < 0.01, "carried by the belt's underside: {a:?} -> {b:?}");
    }

    /// A4: a body added later standing on the floor against the belt's side (the main test's `side`,
    /// added instead of the file's) is not carried.
    #[test]
    fn an_added_body_standing_at_a_belt_side_is_not_carried() {
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [20.0, 4.0, 0.1]\nat = [0.0, 0.0, -0.05]\n\
             [[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.5]\nat = [0.0, 0.0, 0.25]\nspeed = 0.5\n",
        );
        w.add_body(&Body::new("side", [0.2, 0.2, 0.2], [0.0, 0.401, 0.1])).unwrap();
        steps(&mut w, 75);
        let s = w.body("side").unwrap();
        println!("A4 {s:?}");
        assert!(s.velocity[0].abs() < 0.01 && s.at[0].abs() < 0.001, "{s:?}");
    }

    #[test]
    fn every_range_takes_its_bounds_and_refuses_past_them() {
        let ok = |body: &str| fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).map(|_| ()).map_err(|e| format!("{body}: {e}"));
        let mut bad = Vec::new();
        for body in [
            "step_ms = 1\n".to_string(),
            "step_ms = 100\n".to_string(),
            "gravity = 100\n".to_string(),
            "[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nspeed = 100\n".to_string(),
            "[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nspeed = -100\n".to_string(),
            "[[fixed]]\nname = \"f\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nfriction = 10\n".to_string(),
            "[[fixed]]\nname = \"f\"\nsize = [10000.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n".to_string(),
            "[[fixed]]\nname = \"f\"\nsize = [1.0, 1.0, 1.0]\nat = [10000.0, -10000.0, 0.0]\n".to_string(),
            "[[sensor]]\nname = \"s\"\nradius = 10000.0\nat = [0.0, 0.0, 0.0]\n".to_string(),
        ] {
            if let Err(e) = ok(&body) {
                bad.push(e);
            }
        }
        let mut w = world("[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n");
        if let Err(e) = w.set_speed("b", -100.0) {
            bad.push(format!("set_speed -100: {e}"));
        }
        // Past the bound, or not finite: refused.
        for body in ["[[sensor]]\nname = \"s\"\nradius = nan\nat = [0.0, 0.0, 0.0]\n", "[[sensor]]\nname = \"s\"\nradius = 10000.5\nat = [0.0, 0.0, 0.0]\n", "gravity = inf\n"] {
            if ok(body).is_ok() {
                bad.push(format!("{body}: accepted"));
            }
        }
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn a_sensor_overlapping_a_fixed_shape_and_a_belt_holds_neither() {
        let s = fisica::load(&{
            let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cena-schema.md")).unwrap();
            let start = doc.find("```toml\n").unwrap() + 8;
            doc[start..start + doc[start..].find("```").unwrap()].to_string()
        })
        .unwrap();
        let (b1, belt) = (&s.sensors[0], &s.belts[0]);
        let b1_bottom = b1.at[2] - b1.size.unwrap()[2] / 2.0;
        let belt_top = belt.at[2] + belt.size[2] / 2.0;
        println!("E1 the reference's B1 bottom {b1_bottom} against the belt's top {belt_top}: overlap {}", b1_bottom < belt_top);
        let mut w = world(
            "[[fixed]]\nname = \"floor\"\nsize = [10.0, 4.0, 0.1]\nat = [0.0, 0.0, -0.05]\n\
             [[belt]]\nname = \"belt\"\nsize = [4.0, 0.6, 0.1]\nat = [0.0, 0.0, 0.75]\nspeed = 0.5\n\
             [[sensor]]\nname = \"on_floor\"\nsize = [0.5, 0.5, 0.5]\nat = [3.0, 0.0, 0.0]\n\
             [[sensor]]\nname = \"on_belt\"\nsize = [0.01, 0.8, 0.5]\nat = [0.0, 0.0, 0.75]\n\
             [[sensor]]\nname = \"ball\"\nradius = 0.3\nat = [-1.0, 0.0, 0.0]\n",
        );
        steps(&mut w, 10);
        let ev = w.take_events();
        println!("E1 {ev:?} {:?}", w.sensors());
        assert!(ev.is_empty() && w.sensors().iter().all(|s| !s.occupied()), "{ev:?} {:?}", w.sensors());
    }

    #[test]
    fn a_step_the_handle_held_past_its_time_is_counted_late() {
        let s = scene("[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n");
        let h = fisica::start(&s).unwrap();
        let starts: Arc<Mutex<Vec<(u64, Instant)>>> = Arc::new(Mutex::new(Vec::with_capacity(2000)));
        let st = Arc::clone(&starts);
        h.add_hook(Box::new(move |w: &mut World| {
            st.lock().unwrap().push((w.steps(), Instant::now()));
        }));
        std::thread::sleep(Duration::from_millis(200));
        let late0 = h.status().late;
        let holds = 40;
        for _ in 0..holds {
            h.with(|_| std::thread::sleep(Duration::from_millis(9)));
            std::thread::sleep(Duration::from_millis(100));
        }
        let late = h.status().late - late0;
        drop(h);
        let starts = starts.lock().unwrap().clone();
        // The step taken after `n` steps is due at begun + (n + 1) step; begun is at most the least
        // of (start - (n + 1) step), so each lateness read against it is at most the real one.
        let step = Duration::from_millis(4);
        let base = starts.iter().map(|(n, t)| *t - step * (*n as u32 + 1)).min().unwrap();
        let window_from = starts.iter().find(|(_, t)| *t >= starts[0].1 + Duration::from_millis(200)).unwrap().0;
        let begun_late = starts
            .iter()
            .filter(|(n, _)| *n >= window_from)
            .filter(|(n, t)| t.saturating_duration_since(base + step * (*n as u32 + 1)) > step)
            .count();
        let worst: Vec<f64> = {
            let mut v: Vec<f64> = starts.iter().filter(|(n, _)| *n >= window_from).map(|(n, t)| t.saturating_duration_since(base + step * (*n as u32 + 1)).as_secs_f64() * 1000.0).collect();
            v.sort_by(|a, b| b.partial_cmp(a).unwrap());
            v.truncate(12);
            v
        };
        println!("F1 {holds} holds of 9 ms: steps begun more than one step late (a lower bound) {begun_late}, counted late {late}; the latest (ms) {worst:?}");
        assert!(late as usize + 3 >= begun_late, "{begun_late} steps began more than one step after their time, {late} counted late");
        // And no more: a runner counting a step late past half a step, or past a tenth, passed the
        // floor alone (round 5).
        let (_, perhaps) = super::begun_late(&starts, step, window_from);
        assert!(late as usize <= perhaps + 3, "{late} counted late, at most {perhaps} began late");
    }

    /// F2: the same with holds of 6 ms (a step and a half): the step that waits begins up to 6 ms
    /// late, the next at most 2 ms; the clause counts the first, the runner never does.
    #[test]
    fn a_step_a_step_and_a_half_late_is_counted_late() {
        let s = scene("[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n");
        let h = fisica::start(&s).unwrap();
        let starts: Arc<Mutex<Vec<(u64, Instant)>>> = Arc::new(Mutex::new(Vec::with_capacity(2000)));
        let st = Arc::clone(&starts);
        h.add_hook(Box::new(move |w: &mut World| {
            st.lock().unwrap().push((w.steps(), Instant::now()));
        }));
        std::thread::sleep(Duration::from_millis(200));
        let late0 = h.status().late;
        let holds = 40;
        for _ in 0..holds {
            h.with(|_| std::thread::sleep(Duration::from_millis(6)));
            std::thread::sleep(Duration::from_millis(100));
        }
        let late = h.status().late - late0;
        drop(h);
        let starts = starts.lock().unwrap().clone();
        let step = Duration::from_millis(4);
        let base = starts.iter().map(|(n, t)| *t - step * (*n as u32 + 1)).min().unwrap();
        let window_from = starts.iter().find(|(_, t)| *t >= starts[0].1 + Duration::from_millis(200)).unwrap().0;
        let begun_late = starts.iter().filter(|(n, _)| *n >= window_from).filter(|(n, t)| t.saturating_duration_since(base + step * (*n as u32 + 1)) > step).count();
        println!("F2 {holds} holds of 6 ms: steps begun more than one step late (a lower bound) {begun_late}, counted late {late}");
        assert!(late as usize + 3 >= begun_late, "{begun_late} steps began more than one step after their time, {late} counted late");
        // And no more: a runner counting a step late past half a step, or past a tenth, passed the
        // floor alone (round 5).
        let (_, perhaps) = super::begun_late(&starts, step, window_from);
        assert!(late as usize <= perhaps + 3, "{late} counted late, at most {perhaps} began late");
    }

    #[test]
    fn the_depth_a_body_rests_grows_with_gravity() {
        // 0.07 mm at 9.81 m/s² (pinned in round 2), ten times that at gravity 100, whatever the step.
        let depth = |step_ms: u32, gravity: f64| {
            let mut w = world(&format!("step_ms = {step_ms}\ngravity = {gravity}\n[[fixed]]\nname = \"floor\"\nsize = [40.0, 40.0, 0.2]\nat = [0.0, 0.0, -0.1]\n[[body]]\nname = \"box\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.11]\n"));
            steps(&mut w, u64::from(5000 / step_ms));
            0.1 - w.body("box").unwrap().at[2]
        };
        for step_ms in [1, 4, 10, 100] {
            let d = depth(step_ms, 9.81);
            assert!((0.00005..0.0001).contains(&d), "{step_ms} ms at 9.81: {d}");
        }
        for step_ms in [4, 10] {
            let d = depth(step_ms, 100.0);
            assert!((0.0006..0.0008).contains(&d), "{step_ms} ms at 100: {d}");
        }
    }

    #[test]
    fn the_engine_is_rapier_0_36_1_with_enhanced_determinism_named_in_the_world_module_alone() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let workspace = std::fs::read_to_string(format!("{root}/Cargo.toml")).unwrap();
        // In double precision (round 4: in single, a slow belt far from the origin carried nothing).
        assert!(workspace.lines().any(|l| l.trim() == "rapier3d-f64 = { version = \"=0.36.1\", features = [\"enhanced-determinism\"] }"), "the workspace's rapier3d-f64 line");
        let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        assert!(manifest.lines().any(|l| l.trim() == "rapier3d-f64 = { workspace = true }"));
        let lock = std::fs::read_to_string(format!("{root}/Cargo.lock")).unwrap();
        assert!(lock.contains("name = \"rapier3d-f64\"\nversion = \"0.36.1\""));
        assert!(!lock.contains("name = \"rapier3d\"\n"), "no single-precision build beside it");
        // Named in `src/fisica/world.rs` alone.
        let mut naming = Vec::new();
        let mut dirs = vec![std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
        while let Some(d) = dirs.pop() {
            for e in std::fs::read_dir(d).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    dirs.push(p);
                } else if std::fs::read_to_string(&p).unwrap_or_default().contains("rapier3d") {
                    naming.push(p);
                }
            }
        }
        assert!(naming.len() == 1 && naming[0].ends_with(std::path::Path::new("src").join("fisica").join("world.rs")), "{naming:?}");
    }
}

/// C-114's fourth refute round: the critic's probes (`~/.cache/veredas/critic-c114-r4/probes`), kept
/// as pins: double precision far from the origin, drops at long steps resting on what they fall on,
/// the size and mass bounds, the refusals of every rule by kind, every kind's defaults, the runner
/// at a step of its scene's own.
mod round_4 {
    use std::time::{Duration, Instant};

    use veredas_maquinar::fisica::{self, Body, FisicaError};

    use super::{scene, steps, world};

    /// How far a carton on a belt at `x` moves in 5 s, and its velocity then.
    fn carried(x: f64, speed: f64, step_ms: u32) -> (f64, f64) {
        let mut w = world(&format!(
            "step_ms = {step_ms}\n[[belt]]\nname = \"belt\"\nsize = [6.0, 0.6, 0.1]\nat = [{x}, 0.0, 0.75]\nspeed = {speed}\n\
             [[body]]\nname = \"carton\"\nsize = [0.3, 0.3, 0.3]\nat = [{x}, 0.0, 0.95]\nmass = 2.5\n"
        ));
        let per_s = u64::from(1000 / step_ms);
        steps(&mut w, per_s / 5);
        let a = w.body("carton").unwrap();
        steps(&mut w, per_s * 5);
        let b = w.body("carton").unwrap();
        (b.at[0] - a.at[0], b.velocity[0])
    }

    #[test]
    fn a_belt_far_from_the_origin_carries_what_lies_on_it_as_one_near_it() {
        // In single precision a 0.01 m/s belt at 1000 m carried nothing and one at 100 m 24 % short.
        let mut off = Vec::new();
        for (x, speed, step_ms) in [(0.0, 0.01, 4), (30.0, 0.01, 4), (100.0, 0.01, 4), (1000.0, 0.01, 4), (1000.0, 0.05, 4), (5000.0, 0.05, 4), (9000.0, 0.01, 4), (9000.0, 0.5, 4), (30.0, 0.01, 1), (500.0, 0.1, 1), (-9000.0, -0.01, 4)] {
            let (moved, v) = carried(x, speed, step_ms);
            let expected = speed * 5.0;
            if ((moved - expected) / expected).abs() >= 0.02 || (v - speed).abs() > speed.abs() * 0.02 {
                off.push((x, speed, step_ms, moved, v));
            }
        }
        assert!(off.is_empty(), "(x, speed, step_ms, moved in 5 s, velocity) {off:?}");
    }

    #[test]
    fn far_from_the_origin_a_fall_keeps_to_gravity_and_a_body_rests_as_deep() {
        let mut w = world("[[body]]\nname = \"ball\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 9000.0]\n");
        steps(&mut w, 250);
        let exact = 9000.0 - 9.81 / 2.0;
        let z = w.body("ball").unwrap().at[2];
        assert!((z - exact).abs() <= 9.81 * 0.004 / 2.0 + 1e-4, "{z} against {exact}");
        // A 0.2 m box on floors whose tops are from 0 to 9000 m up: 0.05 to 0.1 mm deep on each.
        for top in [0.0f64, 8.0, 10.0, 20.0, 50.0, 1000.0, 9000.0] {
            let mut w = world(&format!(
                "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [3000.0, -4000.0, {}]\n[[body]]\nname = \"box\"\nsize = [0.2, 0.2, 0.2]\nat = [3000.0, -4000.0, {}]\n",
                top - 0.1,
                top + 0.11
            ));
            steps(&mut w, 1250);
            let depth = top + 0.1 - w.body("box").unwrap().at[2];
            assert!((0.00005..0.0001).contains(&depth), "on a floor at {top} m: {depth} m deep");
        }
    }

    #[test]
    fn a_mover_far_from_the_origin_reaches_each_pose_within_10_um() {
        let mut w = world("[[mover]]\nname = \"pusher\"\nsize = [0.05, 0.6, 0.3]\nat = [9000.0, 9000.0, 1.0]\n");
        for k in 1..=250u32 {
            let target = [9000.0 + 0.004 * f64::from(k), 9000.0 - 0.002 * f64::from(k), 1.0];
            w.place("pusher", fisica::Placement::at(target)).unwrap();
            w.step();
            let at = w.body("pusher").unwrap().at;
            let off = (0..3).map(|i| (at[i] - target[i]).abs()).fold(0.0, f64::max);
            assert!(off < 1e-5, "pose {k}: {at:?} against {target:?}");
        }
    }

    #[test]
    fn a_body_dropped_at_a_long_step_rests_on_what_it_falls_on() {
        // The seven drops of the round's sweep that crossed a fixed part whole (contacts recycled
        // from a step that found none): (step, side, floor's thickness, drop as 0.05 m times n).
        for (step_ms, size, floor, n) in [(10u32, 0.1f64, 0.02f64, 46u32), (20, 0.1, 0.02, 12), (20, 0.1, 0.02, 18), (75, 0.3, 0.1, 57), (75, 0.1, 0.02, 57), (100, 0.3, 0.1, 10), (100, 0.1, 0.02, 10)] {
            let drop = 0.05 * f64::from(n);
            let mut w = world(&format!(
                "step_ms = {step_ms}\n[[fixed]]\nname = \"floor\"\nsize = [10.0, 4.0, {floor}]\nat = [0.0, 0.0, {}]\n\
                 [[body]]\nname = \"b\"\nsize = [{size}, {size}, {size}]\nat = [0.0, 0.0, {}]\nmass = 2.5\n",
                -floor / 2.0,
                size / 2.0 + drop
            ));
            steps(&mut w, u64::from(5000 / step_ms));
            let b = w.body("b").unwrap();
            assert!((b.at[2] - size / 2.0).abs() < 0.002 && b.velocity[2].abs() < 0.01, "{step_ms} ms, {size} m dropped {drop} m onto {floor} m: {b:?}");
        }
    }

    #[test]
    fn a_millimetre_and_a_milligram_are_the_least_and_rest_on_a_floor() {
        for (mass, size) in [(0.000_001, 0.2), (1.0, 0.001), (0.000_001, 0.001)] {
            let half = size / 2.0;
            let mut w = world(&format!(
                "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n[[body]]\nname = \"box\"\nsize = [{size}, {size}, {size}]\nat = [0.0, 0.0, {}]\nmass = {mass}\n",
                half + 0.05
            ));
            steps(&mut w, 500);
            let b = w.body("box").unwrap();
            let deep = half - b.at[2];
            assert!((0.0..0.0001).contains(&deep) && b.velocity.iter().all(|v| v.abs() < 1e-3), "{mass} kg of {size} m: {deep} m deep, {b:?}");
        }
        let refused = |body: &str| fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).err().map(|e| e.to_string());
        assert_eq!(refused("[[body]]\nname = \"p\"\nsize = [0.0009, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n"), Some("body[0] (`p`).size: [0.0009, 1.0, 1.0] m, each side from 0.001 to 10000".into()));
        assert_eq!(refused("[[sensor]]\nname = \"s\"\nradius = 0.0009\nat = [0.0, 0.0, 0.0]\n"), Some("sensor[0] (`s`).radius: 0.0009 m, not from 0.001 to 10000".into()));
        assert_eq!(refused("[[body]]\nname = \"p\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\nmass = 0.0000009\n"), Some("body[0] (`p`).mass: 0.0000009 kg, not from 0.000001 to 100000".into()));
        assert_eq!(refused("[[sensor]]\nname = \"s\"\nradius = 0.001\nat = [0.0, 0.0, 0.0]\n"), None);
    }

    #[test]
    fn every_range_rule_is_refused_for_every_kind() {
        let refused = |body: &str| fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}"));
        let part = |kind: &str, extra: &str| format!("[[{kind}]]\nname = \"p\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n{extra}");
        let mut bad = Vec::new();
        let cases: Vec<(String, &str)> = vec![
            (part("belt", "friction = 11\n"), "belt[0] (`p`).friction: 11, not 0 to 10"),
            (part("body", "friction = -1\n"), "body[0] (`p`).friction: -1, not 0 to 10"),
            (part("mover", "").replace("[1.0, 1.0, 1.0]", "[0.0, 1.0, 1.0]"), "mover[0] (`p`).size: [0.0, 1.0, 1.0] m, each side from 0.001 to 10000"),
            (part("sensor", "").replace("[1.0, 1.0, 1.0]", "[1.0, -1.0, 1.0]"), "sensor[0] (`p`).size: [1.0, -1.0, 1.0] m, each side from 0.001 to 10000"),
            (part("body", "mass = nan\n"), "body[0] (`p`).mass: NaN kg, not from 0.000001 to 100000"),
            (part("body", "").replace("[1.0, 1.0, 1.0]", "[1.0, nan, 1.0]"), "body[0] (`p`).size: [1.0, NaN, 1.0] m, each side from 0.001 to 10000"),
            (part("belt", "speed = nan\n"), "belt[0] (`p`).speed: NaN m/s, not within 100 either way"),
            (part("belt", "speed = inf\n"), "belt[0] (`p`).speed: inf m/s, not within 100 either way"),
        ];
        for (body, message) in cases {
            match refused(&body) {
                Err(FisicaError::Invalid(m)) if m == message => {}
                other => bad.push(format!("{message}: {other:?}")),
            }
        }
        let mut w = world("[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n");
        assert_eq!(w.add_body(&Body { friction: 11.0, ..Body::new("n", [0.1; 3], [5.0, 0.0, 0.0]) }), Err(FisicaError::Invalid("body `n`.friction: 11, not 0 to 10".into())));
        assert!(w.set_speed("b", f64::NAN).is_err(), "a NaN speed taken");
        assert_eq!(w.speed("b"), Some(0.0));
        assert!(bad.is_empty(), "{bad:#?}");
    }

    #[test]
    fn every_kind_s_defaults_are_the_reference_s() {
        let s = scene("[[fixed]]\nname = \"f\"\nsize = [1.0, 1.0, 1.0]\nat = [0.0, 0.0, 0.0]\n[[belt]]\nname = \"b\"\nsize = [1.0, 1.0, 1.0]\nat = [5.0, 0.0, 0.0]\n[[body]]\nname = \"c\"\nsize = [1.0, 1.0, 1.0]\nat = [9.0, 0.0, 0.0]\n[[mover]]\nname = \"m\"\nsize = [1.0, 1.0, 1.0]\nat = [12.0, 0.0, 0.0]\n[[sensor]]\nname = \"s\"\nradius = 1.0\nat = [15.0, 0.0, 0.0]\n");
        assert_eq!((s.fixed[0].friction, s.belts[0].friction, s.bodies[0].friction, s.movers[0].friction), (0.6, 0.6, 0.6, 0.6));
        let angles = [
            [s.fixed[0].yaw, s.fixed[0].pitch, s.fixed[0].roll],
            [s.belts[0].yaw, s.belts[0].pitch, s.belts[0].roll],
            [s.bodies[0].yaw, s.bodies[0].pitch, s.bodies[0].roll],
            [s.movers[0].yaw, s.movers[0].pitch, s.movers[0].roll],
            [s.sensors[0].yaw, s.sensors[0].pitch, s.sensors[0].roll],
        ];
        assert!(angles.iter().flatten().all(|a| *a == 0.0), "{angles:?}");
    }

    /// On the machine's lock: a 10 ms and a 20 ms scene keep their own step.
    #[test]
    fn the_runner_keeps_its_scene_s_own_step() {
        let mut off = Vec::new();
        for step_ms in [10u64, 20] {
            let s = scene(&format!("step_ms = {step_ms}\n[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n"));
            let h = fisica::start(&s).unwrap();
            let starts = super::step_starts(&h);
            std::thread::sleep(Duration::from_millis(100));
            let (first, begun, late0) = (h.steps(), Instant::now(), h.status().late);
            std::thread::sleep(Duration::from_millis(1000));
            let (n, elapsed) = (h.steps() - first, begun.elapsed());
            let late = h.status().late - late0;
            drop(h);
            let due = (elapsed.as_millis() / u128::from(step_ms)) as u64;
            // The steps counted late are those that began more than one step after their time.
            let (surely, perhaps) = super::begun_late(&starts.lock().unwrap(), Duration::from_millis(step_ms), first);
            if !(n + 3 >= due && n <= due + 2) || (late as usize) + 3 < surely || late as usize > perhaps + 3 {
                off.push((step_ms, n, due, late));
            }
        }
        assert!(off.is_empty(), "(step_ms, steps, due, late) {off:?}");
    }

}

/// C-114's fifth refute round: the critic's probes (`~/.cache/veredas/critic-c114-r5/probes`), kept as
/// pins: a stack's contacts sinking with the load above, a body past 300 times the mass under it
/// sinking through, and the resting depth by a body's proportions.
mod round_5 {
    use super::{steps, world};

    /// A 1 kg box of 0.2 m on the floor and one `ratio` times its mass on it: (the lower one's depth
    /// in the floor, the upper's in the lower, the upper's bottom's height), in metres.
    fn stack(ratio: f64) -> (f64, f64, f64) {
        let mut w = world(&format!(
            "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n\
             [[body]]\nname = \"light\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.1]\nmass = 1\n\
             [[body]]\nname = \"heavy\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.3005]\nmass = {ratio}\n"
        ));
        steps(&mut w, 1000);
        let (l, h) = (w.body("light").unwrap().at[2], w.body("heavy").unwrap().at[2]);
        (0.1 - l, 0.2 - (h - l), h - 0.1)
    }

    #[test]
    fn a_stack_s_contacts_sink_with_the_load_above_them() {
        // Two equal bodies: the lower 0.14 mm into the floor (twice a body alone), the upper 0.5 mm
        // into it; ten times the mass on top: 0.7 mm and 2.7 mm.
        let (floor, upper, _) = stack(1.0);
        assert!((0.0001..0.0002).contains(&floor) && (0.0004..0.0006).contains(&upper), "{floor} {upper}");
        let (floor, upper, _) = stack(10.0);
        assert!((0.0006..0.0009).contains(&floor) && (0.002..0.0035).contains(&upper), "{floor} {upper}");
        // Five equal cartons: the lowest 0.34 mm into the floor, each less into the one under it.
        let mut text = String::from("[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n");
        for i in 0..5 {
            text += &format!("[[body]]\nname = \"c{i}\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, {}]\nmass = 2.5\n", 0.15 + 0.3005 * f64::from(i));
        }
        let mut w = world(&text);
        steps(&mut w, 1500);
        let z: Vec<f64> = (0..5).map(|i| w.body(&format!("c{i}")).unwrap().at[2]).collect();
        let into: Vec<f64> = (1..5).map(|i| 0.3 - (z[i] - z[i - 1])).collect();
        assert!((0.00025..0.00045).contains(&(0.15 - z[0])), "the lowest {} m into the floor", 0.15 - z[0]);
        assert!(into.windows(2).all(|w| w[0] > w[1]) && into.iter().all(|d| *d > 0.0), "each into the one under it {into:?}");
    }

    #[test]
    fn a_body_a_thousand_times_the_mass_under_it_sinks_through_to_the_floor() {
        // Past about 300 times the mass the upper sinks through the lower one; at 1000 its bottom
        // rests on what lies beneath, the floor.
        let (_, _, bottom) = stack(1000.0);
        assert!(bottom.abs() < 0.001, "the upper's bottom {bottom} m above the floor");
    }

    #[test]
    fn a_body_alone_rests_deeper_the_flatter_it_is() {
        // (size, lowest and highest depth in metres) at 4 and 20 ms: a cube 0.07 mm, a tall box less,
        // a carton more, all within 0.12 mm and none above the floor by more than 0.01 mm.
        for step_ms in [4u32, 20] {
            for (size, lo, hi) in [([0.2f64, 0.2, 0.2], 0.00006, 0.00008), ([0.2, 0.2, 1.0], 0.000015, 0.00003), ([0.3, 0.4, 0.2], 0.00008, 0.00011), ([0.2, 0.2, 0.05], 0.00005, 0.00012), ([0.05, 0.05, 2.0], -0.00001, 0.00003)] {
                let half = size[2] / 2.0;
                let mut w = world(&format!(
                    "step_ms = {step_ms}\n[[fixed]]\nname = \"floor\"\nsize = [40.0, 40.0, 0.2]\nat = [0.0, 0.0, -0.1]\n[[body]]\nname = \"box\"\nsize = [{}, {}, {}]\nat = [0.0, 0.0, {}]\n",
                    size[0], size[1], size[2], half + 0.005
                ));
                steps(&mut w, u64::from(4000 / step_ms));
                let deep = half - w.body("box").unwrap().at[2];
                assert!((lo..hi).contains(&deep), "{size:?} at {step_ms} ms: {deep} m deep");
            }
        }
    }
}

/// C-114's sixth refute round: the critic's probes (`~/.cache/veredas/critic-c114-r6/probes`), kept
/// as pins: columns of cartons and two-box stacks standing at every step (the solver's iterations
/// now grow with the step), drops onto a mover or a body resting on it (bodies swept against every
/// collider), a body alone within 0.12 mm at every step, the header's refusals by their text; on
/// the machine's lock, a 2 s stall caught up and never skipped, and a hook's stall counting the
/// step after it, not its own.
mod round_6 {
    use std::time::{Duration, Instant};

    use veredas_maquinar::fisica::{self, Body, World};

    fn world(body: &str) -> World {
        World::new(&fisica::load(&format!("format = \"veredas-cena\"\nversion = 1\n{body}")).unwrap()).unwrap()
    }

    fn steps(w: &mut World, n: u64) {
        for _ in 0..n {
            w.step();
        }
    }

    const FLOOR: &str = "[[fixed]]\nname = \"floor\"\nsize = [4.0, 4.0, 0.2]\nat = [0.0, 0.0, -0.1]\n";

    /// Ten equal 2.5 kg cartons of 0.3 m, stacked: at a 50 ms step they collapsed into one another
    /// and through the floor. At every step now, the lowest 0.5 to 1 mm into the floor, each up to
    /// 6 mm into the one under it, the top within 30 mm of 3 m.
    #[test]
    fn a_column_of_ten_cartons_stands_at_every_step() {
        let mut wrong = Vec::new();
        for step_ms in [4u32, 20, 50, 100] {
            let mut text = format!("step_ms = {step_ms}\n{FLOOR}");
            for i in 0..10 {
                text += &format!("[[body]]\nname = \"c{i}\"\nsize = [0.3, 0.3, 0.3]\nat = [0.0, 0.0, {}]\nmass = 2.5\n", 0.15 + 0.3005 * f64::from(i));
            }
            let mut w = world(&text);
            steps(&mut w, u64::from(6000 / step_ms));
            let z: Vec<f64> = (0..10).map(|i| w.body(&format!("c{i}")).unwrap().at[2]).collect();
            let floor = 0.15 - z[0];
            let into: Vec<f64> = (1..10).map(|i| 0.3 - (z[i] - z[i - 1])).collect();
            if !(0.0005..=0.001).contains(&floor) || into.iter().any(|d| !(0.0..=0.006).contains(d)) || z[9] + 0.15 < 2.97 {
                wrong.push(format!("{step_ms} ms: floor {floor:.5} m, each {into:.4?}, top {:.4}", z[9] + 0.15));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Eight cartons added one a second, each 1 cm above the top: at 50 to 100 ms they had sunk
    /// through each other to tens of metres under the floor. Now the lowest within 2 mm of 0.15 m,
    /// each 0.29 to 0.3 m above the one under it.
    #[test]
    fn a_column_built_one_carton_at_a_time_stands_at_long_steps() {
        let mut wrong = Vec::new();
        for step_ms in [20u32, 50, 100] {
            let mut w = world(&format!("step_ms = {step_ms}\n{FLOOR}"));
            let per_s = u64::from(1000 / step_ms);
            let mut top = 0.0f64;
            for i in 0..8 {
                w.add_body(&Body { mass: 2.5, ..Body::new(&format!("c{i}"), [0.3, 0.3, 0.3], [0.0, 0.0, top + 0.16]) }).unwrap();
                steps(&mut w, per_s);
                top = (0..=i).map(|k| w.body(&format!("c{k}")).unwrap().at[2] + 0.15).fold(f64::MIN, f64::max);
            }
            steps(&mut w, 3 * per_s);
            let z: Vec<f64> = (0..8).map(|i| w.body(&format!("c{i}")).unwrap().at[2]).collect();
            if (z[0] - 0.15).abs() > 0.002 || (1..8).any(|i| !(0.29..=0.3).contains(&(z[i] - z[i - 1]))) {
                wrong.push(format!("{step_ms} ms: centres {z:.4?}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Two 0.2 m boxes, the lower 1 kg, the upper `ratio` kg: (the lower into the floor, the upper
    /// into the lower, the upper's bottom above the floor), in metres.
    fn stack(step_ms: u32, ratio: f64) -> (f64, f64, f64) {
        let mut w = world(&format!(
            "step_ms = {step_ms}\n{FLOOR}[[body]]\nname = \"light\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.1]\nmass = 1\n\
             [[body]]\nname = \"heavy\"\nsize = [0.2, 0.2, 0.2]\nat = [0.0, 0.0, 0.3005]\nmass = {ratio}\n"
        ));
        steps(&mut w, u64::from(4000 / step_ms));
        let (l, h) = (w.body("light").unwrap().at[2], w.body("heavy").unwrap().at[2]);
        (0.1 - l, 0.2 - (h - l), h - 0.1)
    }

    /// At every step the depths 4 ms gives: ten times the mass on top, the lower 0.6 to 0.9 mm into
    /// the floor and the upper 2 to 3.5 mm into it (at 100 ms it had sunk through at ten times);
    /// a thousand times, the upper's bottom on the floor, the lower pressed 35 to 50 mm into it.
    #[test]
    fn two_boxes_stack_alike_at_every_step() {
        let mut wrong = Vec::new();
        for step_ms in [1u32, 4, 20, 50, 100] {
            let (floor, upper, _) = stack(step_ms, 10.0);
            if !(0.0006..=0.0009).contains(&floor) || !(0.002..=0.0035).contains(&upper) {
                wrong.push(format!("{step_ms} ms, ten times: floor {floor:.5}, upper {upper:.5}"));
            }
            let (floor, _, bottom) = stack(step_ms, 1000.0);
            if bottom.abs() > 0.001 || !(0.035..=0.05).contains(&floor) {
                wrong.push(format!("{step_ms} ms, a thousand times: floor {floor:.5}, bottom {bottom:.5}"));
            }
        }
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    /// Boxes of 0.1 and 0.3 m dropped 5 cm to 3 m onto a mover's table 2 or 10 cm thick and onto
    /// a 20 kg body lying on a shelf, at 20, 50 and 100 ms: each rests on what it fell on, within 5 mm
    /// (78 of 180 had crossed a mover at those steps).
    #[test]
    fn drops_onto_a_mover_or_a_body_rest_on_it() {
        let mut through = Vec::new();
        for step_ms in [20u32, 50, 100] {
            for (size, thick) in [(0.1f64, 0.02f64), (0.3, 0.1), (0.1, 0.1)] {
                for n in (1..=60u32).step_by(3) {
                    let drop = 0.05 * f64::from(n);
                    for target in ["mover", "body"] {
                        let part = match target {
                            "mover" => format!("[[mover]]\nname = \"t\"\nsize = [2.0, 2.0, {thick}]\nat = [0.0, 0.0, {}]\n", 1.0 - thick / 2.0),
                            _ => format!("[[fixed]]\nname = \"shelf\"\nsize = [3.0, 3.0, 0.1]\nat = [0.0, 0.0, {}]\n[[body]]\nname = \"t\"\nsize = [2.0, 2.0, {thick}]\nat = [0.0, 0.0, {}]\nmass = 20\n", 1.0 - thick - 0.05, 1.0 - thick / 2.0),
                        };
                        let mut w = world(&format!(
                            "step_ms = {step_ms}\n[[fixed]]\nname = \"ground\"\nsize = [20.0, 20.0, 0.2]\nat = [0.0, 0.0, -5.1]\n{part}\
                             [[body]]\nname = \"b\"\nsize = [{size}, {size}, {size}]\nat = [0.0, 0.0, {}]\nmass = 2.5\n",
                            1.0 + size / 2.0 + drop
                        ));
                        steps(&mut w, u64::from(3000 / step_ms));
                        let b = w.body("b").unwrap();
                        let top = if target == "body" { w.body("t").unwrap().at[2] + thick / 2.0 } else { 1.0 };
                        if (b.at[2] - size / 2.0 - top).abs() > 0.005 {
                            through.push(format!("{target} at {step_ms} ms, a {size} m box from {drop:.2} m onto {thick} m: z {:.3}", b.at[2]));
                        }
                    }
                }
            }
        }
        assert!(through.is_empty(), "{} of 360: {through:#?}", through.len());
    }

    /// Fourteen shapes from a 1 cm cube to a 10 m slab, alone on a fixed floor, at steps of 1 to
    /// 100 ms, and three at a milligram and at 100 t: each 0 to 0.12 mm deep (flat ones had rested
    /// 0.14 mm at 100 ms).
    #[test]
    fn a_body_alone_rests_within_0_12_mm_at_every_step() {
        let shapes: [[f64; 3]; 14] = [
            [0.2, 0.2, 0.2],
            [0.2, 0.2, 0.05],
            [0.2, 0.2, 0.02],
            [0.4, 0.4, 0.01],
            [1.0, 1.0, 0.005],
            [1.2, 0.8, 0.15],
            [2.0, 0.3, 0.02],
            [4.0, 4.0, 0.1],
            [0.05, 0.05, 0.002],
            [2.0, 0.05, 0.05],
            [0.6, 0.4, 0.4],
            [0.3, 0.4, 0.2],
            [10.0, 10.0, 0.5],
            [0.01, 0.01, 0.001],
        ];
        let depth = |step_ms: u32, size: [f64; 3], mass: f64| {
            let half = size[2] / 2.0;
            let mut w = world(&format!(
                "step_ms = {step_ms}\n[[fixed]]\nname = \"floor\"\nsize = [60.0, 60.0, 0.2]\nat = [0.0, 0.0, -0.1]\n[[body]]\nname = \"box\"\nsize = [{}, {}, {}]\nat = [0.0, 0.0, {}]\nmass = {mass}\n",
                size[0], size[1], size[2], half + 0.002
            ));
            steps(&mut w, u64::from(4000 / step_ms));
            half - w.body("box").unwrap().at[2]
        };
        let mut over = Vec::new();
        for step_ms in [1u32, 4, 10, 20, 50, 100] {
            for size in shapes {
                let d = depth(step_ms, size, 1.0);
                if !(-0.00001..=0.00012).contains(&d) {
                    over.push(format!("{size:?} at {step_ms} ms: {:.4} mm", d * 1000.0));
                }
            }
        }
        for step_ms in [4u32, 20, 100] {
            for size in [[0.2, 0.2, 0.02], [1.0, 1.0, 0.005], [0.2, 0.2, 0.2]] {
                for mass in [0.000_001, 100_000.0] {
                    let d = depth(step_ms, size, mass);
                    if !(-0.00001..=0.00012).contains(&d) {
                        over.push(format!("{size:?} of {mass} kg at {step_ms} ms: {:.4} mm", d * 1000.0));
                    }
                }
            }
        }
        assert!(over.is_empty(), "{over:#?}");
    }

    /// The header's refusals, by the text core's reader gives (the test had pinned a missing
    /// version by a word and not-TOML by its kind).
    #[test]
    fn the_header_s_refusals_read_by_their_text() {
        for (text, want) in [
            ("version = 1\n", "no `format`: a scene file states `format` and `version`"),
            ("format = \"veredas-cena\"\n", "no `version`: a scene file states `format` and `version`"),
            ("format = 3\nversion = 1\n", "not a scene file: `format` is not a string"),
            ("format = \"veredas-cena\"\nversion = \"1\"\n", "not a scene file: `version` is not a whole number from 0 to 4294967295"),
            ("this is [ not toml", "not a scene file: TOML parse error at line 1, column 6"),
            ("format = \"veredas-cena\"\nversion = 0\n", "version 0 of `veredas-cena` is not known here (this reader knows 1)"),
        ] {
            let got = fisica::load(text).unwrap_err().to_string();
            assert_eq!(got.lines().next(), Some(want), "{text:?}");
        }
    }

    /// On the machine's lock: a 2 s stall, ten times the earlier pins' 200 ms (a runner that gives
    /// up on steps more than 60 behind passed those): caught up within 3 steps of the clock, 490
    /// or more counted late.
    #[test]
    fn a_2_s_stall_is_caught_up_and_never_skipped() {
        let s = fisica::load("format = \"veredas-cena\"\nversion = 1\n[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n").unwrap();
        let h = fisica::start(&s).unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let (first, begun, late0) = (h.steps(), Instant::now(), h.status().late);
        let mut once = false;
        h.add_hook(Box::new(move |_w| {
            if !once {
                once = true;
                std::thread::sleep(Duration::from_millis(2000));
            }
        }));
        std::thread::sleep(Duration::from_millis(3000));
        let (n, elapsed) = (h.steps() - first, begun.elapsed());
        let due = (elapsed.as_millis() / 4) as u64;
        let late = h.status().late - late0;
        assert!(n + 3 >= due && n <= due + 2, "{n} steps against {due} due");
        assert!(late >= 490, "{late} counted late");
    }

    /// On the machine's lock: a hook stalling 9 ms every 25th step; the step whose hook stalled
    /// began on time and counts nothing, the one after it began late. The count lies within 3 of
    /// the steps that began late and never past those within a tenth of a step of late (a runner
    /// judging lateness after the hooks counted both, 80 against 40).
    #[test]
    fn a_hook_s_stall_counts_the_step_after_it_not_its_own() {
        use std::sync::{Arc, Mutex};
        let s = fisica::load("format = \"veredas-cena\"\nversion = 1\n[[body]]\nname = \"b\"\nsize = [0.1, 0.1, 0.1]\nat = [0.0, 0.0, 1000.0]\n").unwrap();
        let h = fisica::start(&s).unwrap();
        let starts: Arc<Mutex<Vec<(u64, Instant)>>> = Arc::new(Mutex::new(Vec::with_capacity(4000)));
        let st = Arc::clone(&starts);
        // The recording hook first, then the one that stalls.
        h.add_hook(Box::new(move |w: &mut World| st.lock().unwrap().push((w.steps(), Instant::now()))));
        std::thread::sleep(Duration::from_millis(100));
        let (late0, from) = (h.status().late, h.steps());
        h.add_hook(Box::new(move |w: &mut World| {
            if w.steps().is_multiple_of(25) {
                std::thread::sleep(Duration::from_millis(9));
            }
        }));
        std::thread::sleep(Duration::from_millis(4000));
        let late = h.status().late - late0;
        drop(h);
        let (surely, perhaps) = super::begun_late(&starts.lock().unwrap(), Duration::from_millis(4), from);
        assert!(late as usize + 3 >= surely && late as usize <= perhaps, "{late} counted late, {surely} to {perhaps} began late");
    }
}

//! C-114: `fisica`, a plant's scene stepped at a fixed step: the reference's worked example run as
//! its prose says, the physics held to numbers worked out by hand, every refusal by its place, the
//! same run twice bit for bit, and the runner in real time.

use std::time::{Duration, Instant};

use veredas_maquinar::fisica::{self, Body, BodyState, FisicaError, Kind, Placement, Scene, SensorEvent, World};

const G: f32 = 9.81;

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

fn at(w: &World, name: &str) -> [f32; 3] {
    w.body(name).unwrap().at
}

fn steps(w: &mut World, n: u64) {
    for _ in 0..n {
        w.step();
    }
}

/// Steps until `done`, keeping each step's pose of `name`; at most `max` steps.
fn run_until(w: &mut World, name: &str, max: u64, mut done: impl FnMut(&mut World, &[SensorEvent]) -> bool) -> (Vec<[f32; 3]>, Vec<SensorEvent>) {
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
    let t = w.time().as_secs_f32();
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
    let t = w.time().as_secs_f32();
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
        assert!((b.at[0] as f64 - x).abs() < 1e-5 && b.at[1] == 0.0 && b.at[2] == 0.2, "{i}: {b:?}");
    }
    // Ahead of the blade's face (0.525) at the blade's speed, 1.5 m/s, its back face into the
    // blade by no more than the allowed millimetre and one step's travel (6 mm).
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
fn slide(angle: f64) -> f32 {
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
    let a = G * (40f32.to_radians().sin() - 0.6 * 40f32.to_radians().cos());
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

fn bits(states: &[Vec<BodyState>]) -> Vec<(String, [u32; 10])> {
    states
        .iter()
        .flatten()
        .map(|b| {
            let v: Vec<u32> = b.at.iter().chain(b.rotation.iter()).chain(b.velocity.iter()).map(|f| f.to_bits()).collect();
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
        (part("fixed", "").replace("[1.0, 1.0, 1.0]", "[1.0, 0.0, 1.0]"), "fixed[0] (`p`).size: [1.0, 0.0, 1.0] m, each side over 0 and up to 10000"),
        (part("belt", "").replace("[1.0, 1.0, 1.0]", "[-1.0, 1.0, 1.0]"), "belt[0] (`p`).size: [-1.0, 1.0, 1.0] m, each side over 0 and up to 10000"),
        (part("body", "").replace("[1.0, 1.0, 1.0]", "[10001.0, 1.0, 1.0]"), "body[0] (`p`).size: [10001.0, 1.0, 1.0] m, each side over 0 and up to 10000"),
        (part("mover", "").replace("at = [0.0, 0.0, 0.0]", "at = [0.0, 0.0, -10001.0]"), "mover[0] (`p`).at: [0.0, 0.0, -10001.0] m, each coordinate within 10000 of the origin"),
        (part("fixed", "yaw = inf\n"), "fixed[0] (`p`).yaw: inf, not a finite angle"),
        (part("fixed", "roll = nan\n"), "fixed[0] (`p`).roll: NaN, not a finite angle"),
        (part("fixed", "friction = -0.1\n"), "fixed[0] (`p`).friction: -0.1, not 0 to 10"),
        (part("mover", "friction = 11\n"), "mover[0] (`p`).friction: 11, not 0 to 10"),
        (part("body", "mass = 0\n"), "body[0] (`p`).mass: 0 kg, not over 0 and up to 100000"),
        (part("body", "mass = 100001\n"), "body[0] (`p`).mass: 100001 kg, not over 0 and up to 100000"),
        (part("belt", "speed = -100.5\n"), "belt[0] (`p`).speed: -100.5 m/s, not within 100 either way"),
        (part("sensor", "radius = 1.0\n"), "sensor[0] (`p`): a sensor is a box (`size`) or a ball (`radius`), not both"),
        ("[[sensor]]\nname = \"s\"\nat = [0.0, 0.0, 0.0]\n".into(), "sensor[0] (`s`): a sensor is a box (`size`) or a ball (`radius`); it has neither"),
        ("[[sensor]]\nname = \"s\"\nradius = 0.0\nat = [0.0, 0.0, 0.0]\n".into(), "sensor[0] (`s`).radius: 0 m, not over 0 and up to 10000"),
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
    assert_eq!(w.add_body(&Body { mass: -1.0, ..Body::new("n", [0.1; 3], [0.0; 3]) }), Err(FisicaError::Invalid("body `n`.mass: -1 kg, not over 0 and up to 100000".into())));
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
    let begun = Instant::now();
    let mut h = fisica::start(&scene).unwrap();
    std::thread::sleep(Duration::from_millis(1000));
    let (steps, elapsed) = (h.steps(), begun.elapsed());
    // One step per 4 ms of real time: a late step is taken at once, never skipped.
    let due = (elapsed.as_millis() / 4) as u64;
    assert!(steps + 3 >= due && steps <= due + 1, "{steps} steps in {elapsed:?}");
    assert_eq!(h.time(), Duration::from_millis(4 * h.steps()));
    let s = h.status();
    assert_eq!((s.bodies, s.sensors), (1, 2));
    assert!(s.late <= s.steps);
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

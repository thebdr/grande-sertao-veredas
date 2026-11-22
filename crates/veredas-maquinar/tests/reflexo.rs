//! C-115: `reflexo`'s first half, ready-made models making a bench device's points behave as the
//! machine behind them: each model held to numbers worked out by hand, stepped by the test; every
//! refusal through the bench file; the reference's conveyor line driven by a Modbus client.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio_modbus::Slave;
use tokio_modbus::client::{Reader, Writer};
use veredas_maquinar::bench::{self, Bench};
use veredas_maquinar::fisica::{self, World};
use veredas_maquinar::reflexo::{self, Models, PointKind, Points, ReflexoConfig};
use veredas_maquinar::{ethernetip, modbus};

const STEP: Duration = Duration::from_millis(4);

/// A device's points in memory, one of them refusing writes when asked.
#[derive(Default)]
struct Mem {
    values: Mutex<BTreeMap<String, f64>>,
    kinds: BTreeMap<String, PointKind>,
    refusing: Mutex<Option<String>>,
}

impl Points for Mem {
    fn kind(&self, point: &str) -> Option<PointKind> {
        self.kinds.get(point).copied()
    }
    fn read(&self, point: &str) -> Result<f64, String> {
        self.values.lock().unwrap().get(point).copied().ok_or_else(|| format!("no point `{point}`"))
    }
    fn write(&self, point: &str, v: f64) -> Result<(), String> {
        if self.refusing.lock().unwrap().as_deref() == Some(point) {
            return Err("refused".into());
        }
        self.values.lock().unwrap().insert(point.to_string(), v);
        Ok(())
    }
}

impl Mem {
    fn get(&self, p: &str) -> f64 {
        self.read(p).unwrap()
    }
    fn set(&self, p: &str, v: f64) {
        self.values.lock().unwrap().insert(p.to_string(), v);
    }
}

fn mem(points: &[(&str, PointKind, f64)]) -> Arc<Mem> {
    let m = Mem { kinds: points.iter().map(|(n, k, _)| (n.to_string(), *k)).collect(), ..Mem::default() };
    for (n, _, v) in points {
        m.set(n, *v);
    }
    Arc::new(m)
}

fn models(text: &str, m: &Arc<Mem>) -> Models {
    let cfg: ReflexoConfig = toml::from_str(text).unwrap_or_else(|e| panic!("{e}\n{text}"));
    let points: Arc<dyn Points> = m.clone();
    Models::new(&cfg, STEP, &|p: &str| (p == "m").then(|| points.clone())).unwrap()
}

/// The scene of `docs/cena-schema.md`'s worked example.
fn cena() -> String {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/cena-schema.md")).unwrap();
    let start = doc.find("```toml\n").unwrap() + "```toml\n".len();
    doc[start..start + doc[start..].find("```").unwrap()].to_string()
}

use PointKind::{Bool, Number};

#[test]
fn a_drive_ramps_to_its_setpoint_exactly_and_through_zero_when_it_reverses() {
    let m = mem(&[("run", Bool, 0.0), ("sp", Number, 50.0), ("rev", Bool, 0.0), ("speed", Number, 9.0), ("running", Bool, 1.0), ("at", Bool, 1.0)]);
    let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = 100, decel = 200, tolerance = 0.5, speed = \"m.speed\", running = \"m.running\", at_speed = \"m.at\" } ]", &m);
    d.step(None);
    assert_eq!((m.get("speed"), m.get("running"), m.get("at")), (0.0, 0.0, 0.0), "stopped, its points written");
    // Run: 100 units a second, 0.4 a step of 4 ms, landing on 50 at the 125th step.
    m.set("run", 1.0);
    for k in 1..=125u32 {
        d.step(None);
        let s = m.get("speed");
        if k < 125 {
            assert!((s - 0.4 * f64::from(k)).abs() < 1e-9, "step {k}: {s}");
        }
        assert_eq!(m.get("running"), 1.0);
        assert_eq!(m.get("at"), if 50.0 - s <= 0.5 { 1.0 } else { 0.0 }, "step {k}: {s}");
    }
    assert_eq!((m.get("speed"), d.speed("d")), (50.0, Some(50.0)), "the setpoint exactly");
    assert_eq!(m.get("at"), 1.0);
    // A smaller setpoint: down at the deceleration, 0.8 a step.
    m.set("sp", 49.0);
    d.step(None);
    assert!((m.get("speed") - 49.2).abs() < 1e-9);
    d.step(None);
    assert_eq!(m.get("speed"), 49.0);
    // Stopped: 49 / 0.8 is 61.25 steps, so 0 at the 62nd, not running then.
    m.set("run", 0.0);
    for _ in 0..61 {
        d.step(None);
        assert_eq!((m.get("running"), m.get("at")), (1.0, 0.0));
    }
    d.step(None);
    assert_eq!((m.get("speed"), m.get("running")), (0.0, 0.0));
    // Reversed: away from 0 the other way; then forward again, through 0 at the deceleration first.
    m.set("sp", 50.0);
    m.set("run", 1.0);
    m.set("rev", 1.0);
    for _ in 0..125 {
        d.step(None);
    }
    assert_eq!(m.get("speed"), -50.0);
    m.set("rev", 0.0);
    d.step(None);
    assert!((m.get("speed") + 49.2).abs() < 1e-9, "decelerating toward 0 first");
    for _ in 0..62 {
        d.step(None);
    }
    assert_eq!(m.get("speed"), 0.0);
    d.step(None);
    assert!((m.get("speed") - 0.4).abs() < 1e-12, "then accelerating forward");
    assert_eq!(reflexo::ramp(1.0, 1.0, 5.0, 5.0, 0.1), 1.0);
}

#[test]
fn a_conveyor_moves_its_belt_at_its_speed_and_the_carton_goes_with_it() {
    let m = mem(&[("run", Bool, 1.0), ("sp", Number, 50.0), ("speed", Number, 0.0)]);
    let mut c = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, speed = \"m.speed\" } ]", &m);
    let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
    // The scene's own 0.5 m/s is the model's from the first step: 0.004 m/s.
    for k in 1..=200u32 {
        c.step(Some(&mut w));
        let speed = c.speed("c").unwrap();
        assert_eq!(w.speed("infeed"), Some(speed * 0.01), "step {k}");
        w.step();
    }
    assert_eq!(w.speed("infeed"), Some(0.5));
    // A quarter of a second at speed past the ramp: the carton moves with the belt.
    let x = w.body("carton-1").unwrap().at[0];
    for _ in 0..250 {
        c.step(Some(&mut w));
        w.step();
    }
    let moved = w.body("carton-1").unwrap().at[0] - x;
    assert!((moved - 0.5).abs() < 0.01, "{moved}");
    // A belt's speed is held to the scene's 100 m/s.
    let m = mem(&[("run", Bool, 1.0), ("sp", Number, 500.0)]);
    let mut c = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = -1, run = \"m.run\", setpoint = \"m.sp\", accel = 1000000, decel = 1 } ]", &m);
    c.step(Some(&mut w));
    assert_eq!(w.speed("infeed"), Some(-100.0));
}

/// A scene without gravity: a sensor at the origin and a mover far off.
fn sensor_scene() -> World {
    let s = "format = \"veredas-cena\"\nversion = 1\ngravity = 0\n[[sensor]]\nname = \"S\"\nradius = 0.1\nat = [0.0, 0.0, 0.0]\n[[mover]]\nname = \"m\"\nsize = [0.05, 0.05, 0.05]\nat = [5.0, 0.0, 0.0]\n";
    World::new(&fisica::load(s).unwrap()).unwrap()
}

#[test]
fn a_sensor_model_says_so_after_its_delays_to_the_step_and_closed_inverts_it() {
    let m = mem(&[("no", Bool, 0.0), ("nc", Bool, 0.0)]);
    let mut s = models(
        "sensor = [ { name = \"a\", sensor = \"S\", point = \"m.no\", on_delay_ms = 10, off_delay_ms = 20 }, { name = \"b\", sensor = \"S\", point = \"m.nc\", normally_closed = true } ]",
        &m,
    );
    let mut w = sensor_scene();
    let step = |s: &mut Models, w: &mut World| {
        s.step(Some(w));
        w.step();
    };
    step(&mut s, &mut w);
    assert_eq!((m.get("no"), m.get("nc")), (0.0, 1.0), "empty: open 0, closed 1");
    // The mover goes in; the scene's sensor holds it after two steps (placed, then seen).
    w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
    while !w.sensor("S").unwrap().occupied() {
        step(&mut s, &mut w);
    }
    // From the first model step that sees it: no delay at once; 10 ms of 4 ms steps on the third.
    let mut seen = Vec::new();
    for _ in 0..4 {
        step(&mut s, &mut w);
        seen.push((m.get("no"), m.get("nc")));
    }
    assert_eq!(seen, [(0.0, 0.0), (0.0, 0.0), (1.0, 0.0), (1.0, 0.0)]);
    assert_eq!(s.occupied("a"), Some(true));
    // Out again: 20 ms on the fifth step.
    w.place("m", fisica::Placement::at([5.0, 0.0, 0.0])).unwrap();
    while w.sensor("S").unwrap().occupied() {
        step(&mut s, &mut w);
    }
    let mut seen = Vec::new();
    for _ in 0..6 {
        step(&mut s, &mut w);
        seen.push(m.get("no"));
    }
    assert_eq!(seen, [1.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
    assert_eq!(m.get("nc"), 1.0);
    // In and out within the delay: the point never changes.
    w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
    step(&mut s, &mut w);
    w.place("m", fisica::Placement::at([5.0, 0.0, 0.0])).unwrap();
    for _ in 0..10 {
        step(&mut s, &mut w);
        assert_eq!(m.get("no"), 0.0);
    }
}

#[test]
fn an_actuator_strokes_at_an_even_speed_between_its_poses_and_reports_its_ends() {
    let m = mem(&[("ext", Bool, 0.0), ("back", Bool, 0.0), ("out", Bool, 1.0)]);
    let mut a = models(
        "actuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.ext\", retracted = { at = [3.5, -0.4, 0.95] }, extended = { at = [3.5, 0.25, 0.95], yaw = 90 }, stroke_ms = 100, retracted_switch = \"m.back\", extended_switch = \"m.out\" } ]",
        &m,
    );
    let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
    a.step(Some(&mut w));
    w.step();
    assert_eq!((m.get("back"), m.get("out"), a.position("a")), (1.0, 0.0, Some(0.0)));
    // 100 ms in 4 ms steps: 25 steps, 0.04 of the stroke each, the pose on the straight line.
    m.set("ext", 1.0);
    for k in 1..=25u32 {
        a.step(Some(&mut w));
        w.step();
        let p = a.position("a").unwrap();
        let at = w.body("pusher").unwrap();
        if k < 25 {
            assert!((p - 0.04 * f64::from(k)).abs() < 1e-9 && (m.get("back"), m.get("out")) == (0.0, 0.0), "{k}: {p}");
        }
        assert!((f64::from(at.at[1]) - (-0.4 + 0.65 * p)).abs() < 1e-5 && (f64::from(at.angles()[0]) - 90.0 * p).abs() < 1e-3, "{k}: {at:?}");
    }
    assert_eq!((a.position("a"), m.get("back"), m.get("out")), (Some(1.0), 0.0, 1.0));
    // Back, from wherever it is.
    m.set("ext", 0.0);
    for _ in 0..10 {
        a.step(Some(&mut w));
    }
    m.set("ext", 1.0);
    a.step(Some(&mut w));
    assert!((a.position("a").unwrap() - 0.64).abs() < 1e-9);
}

#[test]
fn points_are_converted_to_each_type_of_a_modbus_server_and_an_adapter() {
    let server = modbus::start(&toml::from_str(
        "port = 0\npoints = [ { name = \"b\", table = \"coil\", address = 0 }, { name = \"u\", table = \"holding-register\", address = 0 }, { name = \"i\", table = \"holding-register\", address = 1, type = \"i16\" }, { name = \"d\", table = \"holding-register\", address = 2, type = \"u32\" }, { name = \"n\", table = \"holding-register\", address = 4, type = \"i32\" }, { name = \"f\", table = \"holding-register\", address = 6, type = \"f32\" } ]",
    ).unwrap())
    .unwrap();
    let p = server.points();
    assert_eq!((p.kind("b"), p.kind("u"), p.kind("f"), p.kind("x")), (Some(Bool), Some(Number), Some(Number), None));
    for (point, v, read) in [
        ("b", 2.0, 1.0),
        ("b", f64::NAN, 0.0),
        ("u", 70_000.0, 65_535.0),
        ("u", -5.0, 0.0),
        ("u", 2.5, 3.0),
        ("u", f64::NAN, 0.0),
        ("i", -40_000.0, -32_768.0),
        ("d", 5e9, 4_294_967_295.0),
        ("n", -2.4, -2.0),
        ("f", 1.25, 1.25),
    ] {
        p.write(point, v).unwrap();
        assert_eq!(p.read(point).unwrap(), read, "{point} {v}");
    }
    assert_eq!(server.get("u").unwrap(), modbus::Value::U16(0));
    assert_eq!(p.read("x").unwrap_err(), "no point `x`");
    let adapter = ethernetip::start(&toml::from_str(
        "bind = \"127.0.0.1\"\nport = 0\nio_port = 0\nassemblies = [ { instance = 100, size = 4 } ]\npoints = [ { name = \"q\", assembly = 100, byte = 0, bit = 1 }, { name = \"u8\", assembly = 100, byte = 1 }, { name = \"i8\", assembly = 100, byte = 2, type = \"i8\" } ]",
    ).unwrap())
    .unwrap();
    let e = adapter.points();
    assert_eq!((e.kind("q"), e.kind("u8")), (Some(Bool), Some(Number)));
    for (point, v, read) in [("q", 1.0, 1.0), ("u8", 300.0, 255.0), ("i8", -200.0, -128.0), ("i8", 7.6, 8.0)] {
        e.write(point, v).unwrap();
        assert_eq!(e.read(point).unwrap(), read, "{point} {v}");
    }
    assert_eq!(adapter.get("q").unwrap(), ethernetip::Value::Bool(true));
}

#[test]
fn a_point_that_cannot_be_read_or_written_is_counted_and_named_and_the_others_go_on() {
    let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("speed", Number, 0.0), ("running", Bool, 0.0)]);
    let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 1000, decel = 1000, speed = \"m.speed\", running = \"m.running\" } ]", &m);
    *m.refusing.lock().unwrap() = Some("running".into());
    d.step(None);
    d.step(None);
    let s = d.status();
    assert_eq!((s.steps, s.errors, s.last_error.as_deref()), (2, 2, Some("drive `d`: m.running: refused")));
    assert_eq!(m.get("speed"), 8.0, "what came before the refused write was written");
    *m.refusing.lock().unwrap() = None;
    d.step(None);
    assert_eq!((d.status().errors, m.get("running")), (2, 1.0));
    assert_eq!(d.len(), 1);
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("veredas-reflexo-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("line.cena.toml"), cena()).unwrap();
    dir
}

/// The reference's sixth example.
fn sixth() -> String {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/bench-schema.md")).unwrap();
    doc.split("```toml\n").skip(1).map(|b| b[..b.find("```").unwrap()].to_string()).find(|b| b.contains("[[reflexo]]")).expect("the reference's example with models")
}

#[test]
fn every_refusal_is_named_by_its_place_through_the_bench_file() {
    let dir = scratch("refusals");
    let base = "format = \"veredas-bench\"\nversion = 1\n[[modbus]]\nname = \"m\"\nport = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0 }, { name = \"sp\", table = \"holding-register\", address = 0 }, { name = \"b\", table = \"discrete-input\", address = 0 } ]\n[[ethernetip]]\nname = \"x\"\nport = 0\nio_port = 0\nassemblies = [ { instance = 100, size = 2 } ]\npoints = [ { name = \"q\", assembly = 100, byte = 0, bit = 0 }, { name = \"n\", assembly = 100, byte = 1 } ]\n[[fisica]]\nname = \"sc\"\nscene = \"line.cena.toml\"\n[[reflexo]]\nname = \"r\"\n";
    let drive = |rest: &str| format!("drive = [ {{ name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1{rest} }} ]\n");
    let refused = |models: &str| match bench::load(&format!("{base}{models}"), &dir) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("loaded: {models}"),
    };
    let cases: Vec<(String, &str)> = vec![
        (drive("").replace("m.run", "run"), "reflexo[0] (`r`).drive[0] (`d`).run: `run` is not `<part>.<point>`"),
        (drive("").replace("m.run", "z.run"), "reflexo[0] (`r`).drive[0] (`d`).run: no Modbus server or EtherNet/IP adapter is named `z`"),
        (drive("").replace("m.run", "m.nope"), "reflexo[0] (`r`).drive[0] (`d`).run: `m` has no point `nope`"),
        (drive("").replace("m.run", "m.sp"), "reflexo[0] (`r`).drive[0] (`d`).run: `m.sp` is a number, a bool point is wanted"),
        (drive("").replace("setpoint = \"m.sp\"", "setpoint = \"x.q\""), "reflexo[0] (`r`).drive[0] (`d`).setpoint: `x.q` is a bool, a number point is wanted"),
        (drive(", reverse = \"x.n\""), "reflexo[0] (`r`).drive[0] (`d`).reverse: `x.n` is a number, a bool point is wanted"),
        (drive("").replace("accel = 1", "accel = 0"), "reflexo[0] (`r`).drive[0] (`d`).accel: 0, not a finite number over 0"),
        (drive("").replace("decel = 1", "decel = -1"), "reflexo[0] (`r`).drive[0] (`d`).decel: -1, not a finite number over 0"),
        (drive(", tolerance = -0.5"), "reflexo[0] (`r`).drive[0] (`d`).tolerance: -0.5, not a finite number from 0"),
        (drive(", speed = \"x.n\", running = \"x.n\""), "reflexo[0] (`r`).drive[0] (`d`).running: `x.n` is a number, a bool point is wanted"),
        (drive(", running = \"x.q\", at_speed = \"x.q\""), "reflexo[0] (`r`).drive[0] (`d`).at_speed: `x.q` is written by drive[0] (`d`) already"),
        (format!("{}{}", drive(""), drive("").replace("drive = [", "conveyor = [").replace("name = \"d\"", "name = \"d\", belt = \"infeed\", metres_per_unit = 1")), "reflexo[0] (`r`).conveyor[0]: `d` is already the name of a drive"),
        (drive("").replace("name = \"d\"", "name = \" d\""), "reflexo[0] (`r`).drive[0]: the name ` d` has surrounding whitespace"),
        ("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 1, run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1 } ]\n".into(), "reflexo[0] (`r`).conveyor[0] (`c`): a conveyor moves a belt of a scene; this part names no `scene`"),
        ("scene = \"sc\"\nconveyor = [ { name = \"c\", belt = \"outfeed\", metres_per_unit = 1, run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1 } ]\n".into(), "reflexo[0] (`r`).conveyor[0] (`c`).belt: the scene has no belt `outfeed`"),
        ("scene = \"sc\"\nconveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0, run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1 } ]\n".into(), "reflexo[0] (`r`).conveyor[0] (`c`).metres_per_unit: 0, not a finite number other than 0"),
        ("scene = \"nowhere\"\n".into(), "reflexo[0] (`r`).scene: no [[fisica]] part is named `nowhere`"),
        ("sensor = [ { name = \"s\", sensor = \"B1\", point = \"m.b\" } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`): a sensor reports a sensor of a scene; this part names no `scene`"),
        ("scene = \"sc\"\nsensor = [ { name = \"s\", sensor = \"B9\", point = \"m.b\" } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).sensor: the scene has no sensor `B9`"),
        ("scene = \"sc\"\nsensor = [ { name = \"s\", sensor = \"B1\", point = \"m.b\", on_delay_ms = 600001 } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).on_delay_ms: 600001, over 600000"),
        ("scene = \"sc\"\nsensor = [ { name = \"s\", sensor = \"B1\", point = \"m.b\" }, { name = \"t\", sensor = \"S1\", point = \"m.b\" } ]\n".into(), "reflexo[0] (`r`).sensor[1] (`t`).point: `m.b` is written by sensor[0] (`s`) already"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"arm\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 1 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).mover: the scene has no mover `arm`"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 0 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).stroke_ms: 0, not 1 to 600000"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0], yaw = inf }, stroke_ms = 1 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).extended.yaw: inf, not a finite angle"),
        ("actuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 1 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`): an actuator moves a mover of a scene; this part names no `scene`"),
    ];
    for (models, message) in cases {
        assert_eq!(refused(&models), message, "{models}");
    }
    // A scene file that does not load, named with its path.
    std::fs::write(dir.join("bad.cena.toml"), "format = \"veredas-cena\"\nversion = 1\nstep_ms = 0\n").unwrap();
    let err = bench::load(&base.replace("line.cena.toml", "bad.cena.toml"), &dir).err().unwrap().to_string();
    assert_eq!(err, format!("fisica[0] (`sc`).scene ({}): step_ms: 0, not 1 to 100", dir.join("bad.cena.toml").display()));
    let err = bench::load(&base.replace("line.cena.toml", "missing.toml"), &dir).err().unwrap().to_string();
    assert!(err.starts_with("fisica[0] (`sc`).scene: read "), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The line's four discrete inputs: running, the barrier, the pusher back, the pusher out.
async fn inputs(ctx: &mut tokio_modbus::client::Context) -> Vec<bool> {
    ctx.read_discrete_inputs(0, 4).await.unwrap().unwrap()
}

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap()
}

fn until(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let begun = Instant::now();
    while begun.elapsed() < within {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    done()
}

#[test]
fn the_reference_s_conveyor_line_is_driven_by_a_modbus_client_as_a_controller_drives_it() {
    let dir = scratch("line");
    let spec = bench::load(&sixth(), &dir).unwrap();
    assert_eq!(spec.plan()[2], "reflexo `line-models`: 3 model(s) on the scene `line`");
    let plant = Bench::start(spec).unwrap();
    let addr = plant.modbus("plc-io").unwrap().addr;
    let models = plant.reflexo("line-models").unwrap();
    runtime().block_on(async {
        let mut ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(1)).await.unwrap();
        // Stopped: the beam clear (normally closed, 1), the pusher back.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(inputs(&mut ctx).await, [false, true, true, false]);
        // Started: at its setpoint of 50 within half a second and a little, running.
        ctx.write_single_coil(0, true).await.unwrap().unwrap();
        let begun = Instant::now();
        loop {
            if ctx.read_input_registers(0, 1).await.unwrap().unwrap() == [50] {
                break;
            }
            assert!(begun.elapsed() < Duration::from_secs(3), "the conveyor at its setpoint");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(begun.elapsed() >= Duration::from_millis(450), "{:?}: the ramp takes half a second", begun.elapsed());
        assert!(inputs(&mut ctx).await[0], "running");
        // The carton reaches the barrier: the beam broken (0), then clear again.
        for want in [false, true] {
            let begun = Instant::now();
            while inputs(&mut ctx).await[1] != want {
                assert!(begun.elapsed() < Duration::from_secs(8), "the barrier {want}");
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        // Stopped; the pusher extended: out after its half-second stroke.
        ctx.write_single_coil(0, false).await.unwrap().unwrap();
        ctx.write_single_coil(1, true).await.unwrap().unwrap();
        let begun = Instant::now();
        while !inputs(&mut ctx).await[3] {
            assert!(begun.elapsed() < Duration::from_secs(3), "the pusher out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(begun.elapsed() >= Duration::from_millis(450), "{:?}", begun.elapsed());
        let now = inputs(&mut ctx).await;
        assert_eq!((now[0], now[2], now[3]), (false, false, true), "stopped, the pusher out and not back");
    });
    assert!(until(Duration::from_secs(2), || models.speed("infeed") == Some(0.0)));
    assert_eq!(models.position("pusher"), Some(1.0));
    let s = models.status();
    assert!(s.steps > 500 && s.errors == 0, "{s:?}");
    let lines = plant.lines();
    assert!(lines.iter().any(|l| l.starts_with("fisica line: ") && l.ends_with(" 1 body(ies), 2 sensor(s)")), "{lines:?}");
    assert!(lines.iter().any(|l| l.starts_with("reflexo line-models: 3 model(s), ") && l.ends_with(" 0 error(s)")), "{lines:?}");
    plant.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn drives_without_a_scene_step_on_a_thread_of_their_own() {
    let server = modbus::start(&toml::from_str("port = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0, value = \"true\" }, { name = \"sp\", table = \"holding-register\", address = 0, value = \"20\" }, { name = \"speed\", table = \"input-register\", address = 0 } ]").unwrap()).unwrap();
    let points: Arc<dyn Points> = Arc::new(server.points());
    let cfg: ReflexoConfig = toml::from_str("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 40, decel = 40, speed = \"m.speed\" } ]").unwrap();
    reflexo::validate(&cfg, None, &|_: &str, p: &str| Ok(if p == "run" { Bool } else { Number })).unwrap();
    let models = Models::new(&cfg, reflexo::STEP_WITHOUT_SCENE, &|p: &str| (p == "m").then(|| points.clone())).unwrap();
    let mut handle = reflexo::start(models, None).unwrap();
    // 20 at 40 a second: half a second.
    assert!(until(Duration::from_secs(3), || handle.speed("d") == Some(20.0)));
    assert!(until(Duration::from_millis(100), || server.get("speed").unwrap() == modbus::Value::U16(20)));
    assert_eq!(handle.models(), 1);
    handle.stop();
    let steps = handle.status().steps;
    std::thread::sleep(Duration::from_millis(30));
    assert_eq!(handle.status().steps, steps, "no step after the stop");
}

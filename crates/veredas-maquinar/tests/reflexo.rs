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

/// A device's points in memory, one of them refusing writes, or reads, when asked.
#[derive(Default)]
struct Mem {
    values: Mutex<BTreeMap<String, f64>>,
    kinds: BTreeMap<String, PointKind>,
    refusing: Mutex<Option<String>>,
    unreadable: Mutex<Option<String>>,
}

impl Points for Mem {
    fn kind(&self, point: &str) -> Option<PointKind> {
        self.kinds.get(point).copied()
    }
    fn read(&self, point: &str) -> Result<f64, String> {
        if self.unreadable.lock().unwrap().as_deref() == Some(point) {
            return Err("unreadable".into());
        }
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
    // 50 / 0.8 is 62.5 steps to 0: -0.4 at the 62nd; the 63rd is half a step past 0 at the
    // acceleration, 0.2, running all the way; 62.5 + 125 steps land on 50 at the 188th.
    for _ in 2..=62 {
        d.step(None);
        assert_eq!(m.get("running"), 1.0);
    }
    assert!((m.get("speed") + 0.4).abs() < 1e-9, "{}", m.get("speed"));
    d.step(None);
    assert!((m.get("speed") - 0.2).abs() < 1e-12, "then accelerating forward from 0: {}", m.get("speed"));
    assert_eq!(m.get("running"), 1.0);
    for k in 64..188 {
        d.step(None);
        assert!((m.get("speed") - 0.4 * (f64::from(k) - 62.5)).abs() < 1e-9, "step {k}: {}", m.get("speed"));
    }
    d.step(None);
    assert_eq!(m.get("speed"), 50.0, "on the setpoint at the first step at or after its whole time");
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
        assert!((at.at[1] - (-0.4 + 0.65 * p)).abs() < 1e-5 && (at.angles()[0] - 90.0 * p).abs() < 1e-3, "{k}: {at:?}");
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
        ("f", f64::NAN, 0.0),
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
        // Refute round 1: a name empty, a ramp not finite, a stroke past its range, a ratio NaN.
        (drive("").replace("name = \"d\"", "name = \"\""), "reflexo[0] (`r`).drive[0]: the drive has no name"),
        (drive("").replace("accel = 1", "accel = nan"), "reflexo[0] (`r`).drive[0] (`d`).accel: NaN, not a finite number over 0"),
        (drive("").replace("decel = 1", "decel = inf"), "reflexo[0] (`r`).drive[0] (`d`).decel: inf, not a finite number over 0"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 600001 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).stroke_ms: 600001, not 1 to 600000"),
        ("scene = \"sc\"\nconveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = nan, run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1 } ]\n".into(), "reflexo[0] (`r`).conveyor[0] (`c`).metres_per_unit: NaN, not a finite number other than 0"),
        // Refute round 2: a tolerance not finite, a ratio infinite, an off delay past its range, the
        // retracted pose, a switch another model writes or of the other kind, and a point two
        // parts' models would write.
        (drive(", tolerance = nan"), "reflexo[0] (`r`).drive[0] (`d`).tolerance: NaN, not a finite number from 0"),
        (drive(", tolerance = inf"), "reflexo[0] (`r`).drive[0] (`d`).tolerance: inf, not a finite number from 0"),
        ("scene = \"sc\"\nconveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = -inf, run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1 } ]\n".into(), "reflexo[0] (`r`).conveyor[0] (`c`).metres_per_unit: -inf, not a finite number other than 0"),
        ("scene = \"sc\"\nsensor = [ { name = \"s\", sensor = \"B1\", point = \"m.b\", off_delay_ms = 600001 } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).off_delay_ms: 600001, over 600000"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0], pitch = nan }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 1 } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).retracted.pitch: NaN, not a finite angle"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 1, retracted_switch = \"m.b\" } ]\nsensor = [ { name = \"s\", sensor = \"B1\", point = \"m.b\" } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).point: `m.b` is written by actuator[0] (`a`) already"),
        ("scene = \"sc\"\nactuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.run\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.0, 0.0] }, stroke_ms = 1, extended_switch = \"m.sp\" } ]\n".into(), "reflexo[0] (`r`).actuator[0] (`a`).extended_switch: `m.sp` is a number, a bool point is wanted"),
        (format!("{}[[reflexo]]\nname = \"r2\"\n{}", drive(", running = \"m.b\""), drive(", running = \"m.b\"")), "reflexo[1] (`r2`).drive[0] (`d`).running: `m.b` is written by reflexo[0] (`r`).drive[0] (`d`) already"),
    ];
    for (models, message) in cases {
        assert_eq!(refused(&models), message, "{models}");
    }
    // A scene file that does not load, named with its path.
    std::fs::write(dir.join("bad.cena.toml"), "format = \"veredas-cena\"\nversion = 1\nstep_ms = 0\n").unwrap();
    let err = bench::load(&base.replace("line.cena.toml", "bad.cena.toml"), &dir).err().unwrap().to_string();
    assert_eq!(err, format!("fisica[0] (`sc`).scene ({}): step_ms: 0, not 1 to 100", dir.join("bad.cena.toml").display()));
    let err = bench::load(&base.replace("line.cena.toml", "missing.toml"), &dir).err().unwrap().to_string();
    let read = format!("fisica[0] (`sc`).scene: read {}: ", dir.join("missing.toml").display());
    assert!(err.starts_with(&read) && err.len() > read.len(), "{err}");
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
    // The scene's own steps measure the ramp and the stroke: a runner late on the clock takes its
    // missed steps at once, so wall time can run behind the scene's.
    let scene = plant.fisica("line").unwrap();
    runtime().block_on(async {
        let mut ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(1)).await.unwrap();
        // Stopped: the beam clear (normally closed, 1), the pusher back.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(inputs(&mut ctx).await, [false, true, true, false]);
        // Started: at its setpoint of 50 within half a second and a little, running.
        let commanded = scene.steps();
        ctx.write_single_coil(0, true).await.unwrap().unwrap();
        let begun = Instant::now();
        loop {
            if ctx.read_input_registers(0, 1).await.unwrap().unwrap() == [50] {
                break;
            }
            assert!(begun.elapsed() < Duration::from_secs(3), "the conveyor at its setpoint");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        // 50 rounds from 49.5, the 124th step of 0.4.
        assert!(scene.steps() - commanded >= 124, "{} steps: the ramp takes half a second", scene.steps() - commanded);
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
        let commanded = scene.steps();
        ctx.write_single_coil(1, true).await.unwrap().unwrap();
        let begun = Instant::now();
        while !inputs(&mut ctx).await[3] {
            assert!(begun.elapsed() < Duration::from_secs(3), "the pusher out");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(scene.steps() - commanded >= 125, "{} steps: the stroke takes half a second", scene.steps() - commanded);
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

/// Refute round 1's probes (the critic's, `~/.cache/veredas/critic-c115/probes`), kept as pins.
mod round_1 {
    use super::*;

    /// An actuator's steps to its extended switch from the retracted end, then back.
    fn stroke_steps(stroke_ms: u32, world: Option<&mut World>) -> (u32, u32) {
        let m = mem(&[("ext", Bool, 0.0), ("back", Bool, 0.0), ("out", Bool, 0.0)]);
        let mut a = models(&format!("actuator = [ {{ name = \"a\", mover = \"pusher\", extend = \"m.ext\", retracted = {{ at = [3.5, -0.4, 0.95] }}, extended = {{ at = [3.5, 0.25, 0.95] }}, stroke_ms = {stroke_ms}, retracted_switch = \"m.back\", extended_switch = \"m.out\" }} ]"), &m);
        let mut world = world;
        let mut step = |a: &mut Models| {
            a.step(world.as_deref_mut());
            if let Some(w) = world.as_deref_mut() {
                w.step();
            }
        };
        step(&mut a);
        assert_eq!((m.get("back"), m.get("out")), (1.0, 0.0));
        m.set("ext", 1.0);
        let (mut up, mut down) = (0, 0);
        while m.get("out") != 1.0 {
            step(&mut a);
            up += 1;
            assert_eq!(m.get("back"), 0.0);
        }
        m.set("ext", 0.0);
        while m.get("back") != 1.0 {
            step(&mut a);
            down += 1;
            assert_eq!(m.get("out"), 0.0);
        }
        (up, down)
    }

    #[test]
    fn an_actuator_reaches_each_end_at_the_first_step_at_or_after_its_stroke() {
        // A 300 ms stroke at 4 ms out in 75 steps and back in 75, the mover with it; 40 ms in 10.
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        assert_eq!(stroke_steps(300, Some(&mut w)), (75, 75));
        assert!((w.body("pusher").unwrap().at[1] + 0.4).abs() < 1e-5);
        assert_eq!(stroke_steps(40, None), (10, 10));
        // Every stroke from 1 ms to 1 s: summed in floats, one in eight came a step late.
        let late: Vec<(u32, (u32, u32))> = (1..=1000).map(|ms| (ms, stroke_steps(ms, None))).filter(|(ms, got)| *got != (ms.div_ceil(4), ms.div_ceil(4))).collect();
        assert!(late.is_empty(), "strokes off their step (ms, (out, back)): {late:?}");
    }

    #[test]
    fn a_ramp_lands_on_its_setpoint_at_the_first_step_at_or_after_its_time_however_long() {
        // (setpoint, accel, decel): the steps of 4 ms each way are setpoint x 250 / rate, rounded up.
        for (sp, accel, decel) in [(1200u64, 30u64, 30u64), (3000, 7, 11), (1000, 3, 1000), (50, 100, 200), (999, 1000, 9)] {
            let m = mem(&[("run", Bool, 1.0), ("sp", Number, sp as f64)]);
            let mut d = models(&format!("drive = [ {{ name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = {accel}, decel = {decel} }} ]"), &m);
            for (rate, goal) in [(accel, sp as f64), (decel, 0.0)] {
                let steps = (sp * 250).div_ceil(rate);
                for _ in 1..steps {
                    d.step(None);
                }
                assert_ne!(d.speed("d"), Some(goal), "{sp} at {rate}: early, before step {steps}");
                d.step(None);
                assert_eq!(d.speed("d"), Some(goal), "{sp} at {rate}: at step {steps}");
                m.set("run", 0.0);
            }
        }
    }

    #[test]
    fn the_models_step_drives_conveyors_actuators_then_sensors_each_kind_in_the_file_s_order() {
        // A drive run by a sensor model's point reads it as the last step left it: still 0 on the
        // step the point turns on, 0.4 on the next.
        let m = mem(&[("s", Bool, 0.0), ("sp", Number, 10.0)]);
        let mut x = models("drive = [ { name = \"d\", run = \"m.s\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]\nsensor = [ { name = \"b\", sensor = \"S\", point = \"m.s\" } ]", &m);
        let mut w = sensor_scene();
        x.step(Some(&mut w));
        w.step();
        w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
        while m.get("s") == 0.0 {
            x.step(Some(&mut w));
            w.step();
            assert!(x.status().steps < 50);
        }
        assert_eq!(x.speed("d"), Some(0.0), "the drive stepped before the sensor");
        x.step(Some(&mut w));
        assert!((x.speed("d").unwrap() - 0.4).abs() < 1e-12);
        // Drive `b` runs on drive `a`'s running point: `a` first, so `b` starts in the same step.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("ra", Bool, 0.0)]);
        let mut x = models("drive = [ { name = \"a\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.ra\" }, { name = \"b\", run = \"m.ra\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]", &m);
        x.step(None);
        assert_eq!((x.speed("a"), m.get("ra")), (Some(0.4), 1.0));
        assert!((x.speed("b").unwrap() - 0.4).abs() < 1e-12, "b read a's running of this step: {:?}", x.speed("b"));
    }

    #[test]
    fn a_reversed_drive_is_at_speed_at_its_negated_setpoint() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 50.0), ("rev", Bool, 1.0), ("at", Bool, 0.0)]);
        let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = 1000, decel = 1000, at_speed = \"m.at\" } ]", &m);
        for _ in 0..20 {
            d.step(None);
        }
        assert_eq!((d.speed("d"), m.get("at")), (Some(-50.0), 1.0));
    }

    #[test]
    fn a_delay_counts_the_scene_disagreeing_without_a_break() {
        // Three visits of 8 ms each against a 10 ms on delay: never seen, 24 ms in all.
        let m = mem(&[("no", Bool, 0.0)]);
        let mut s = models("sensor = [ { name = \"a\", sensor = \"S\", point = \"m.no\", on_delay_ms = 10 } ]", &m);
        let mut w = sensor_scene();
        let step = |s: &mut Models, w: &mut World| {
            s.step(Some(w));
            w.step();
        };
        step(&mut s, &mut w);
        for _ in 0..3 {
            w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
            step(&mut s, &mut w);
            step(&mut s, &mut w);
            w.place("m", fisica::Placement::at([5.0, 0.0, 0.0])).unwrap();
            for _ in 0..5 {
                step(&mut s, &mut w);
                assert_eq!(m.get("no"), 0.0, "each visit shorter than 10 ms");
            }
        }
    }

    #[test]
    fn a_step_in_which_two_models_fail_is_counted_once_with_the_last_error() {
        // `a` reads a point that is not there, `b` writes one refused: two errors, one step.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("sb", Number, 0.0)]);
        *m.refusing.lock().unwrap() = Some("sb".into());
        let mut d = models("drive = [ { name = \"a\", run = \"m.run\", setpoint = \"m.nope\", accel = 1, decel = 1 }, { name = \"b\", run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1, speed = \"m.sb\" } ]", &m);
        d.step(None);
        let s = d.status();
        assert_eq!((s.steps, s.errors, s.last_error.as_deref()), (1, 1, Some("drive `b`: m.sb: refused")));
    }

    #[test]
    fn models_on_a_scene_step_at_the_scene_s_step() {
        // A scene of 10 ms steps: 100 units a second is 1 a step, read with the scene's count under
        // its lock (the models' own 4 ms would make it 0.4).
        let dir = scratch("slow");
        std::fs::write(dir.join("slow.cena.toml"), cena().replace("step_ms = 4", "step_ms = 10")).unwrap();
        let file = "format = \"veredas-bench\"\nversion = 1\n[[modbus]]\nname = \"m\"\nport = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0 }, { name = \"sp\", table = \"holding-register\", address = 0, value = \"50\" } ]\n[[fisica]]\nname = \"sc\"\nscene = \"slow.cena.toml\"\n[[reflexo]]\nname = \"r\"\nscene = \"sc\"\ndrive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]\n";
        let spec = bench::load(file, &dir).unwrap();
        assert_eq!(spec.plan()[2], "reflexo `r`: 1 model(s) on the scene `sc`");
        let plant = Bench::start(spec).unwrap();
        let scene = plant.fisica("sc").unwrap();
        let models = plant.reflexo("r").unwrap();
        plant.modbus("m").unwrap().set_value("run", modbus::Value::Bool(true)).unwrap();
        let read = || scene.with(|w| (w.steps(), models.speed("d").unwrap()));
        assert!(until(Duration::from_secs(2), || read().1 > 0.0));
        let a = read();
        std::thread::sleep(Duration::from_millis(200));
        let b = read();
        plant.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(b.0 > a.0 && b.1 < 50.0, "{a:?} {b:?}");
        assert!(((b.1 - a.1) / (b.0 - a.0) as f64 - 1.0).abs() < 1e-9, "{a:?} {b:?}");
    }

    #[test]
    fn no_scene_steps_without_its_models_however_long_another_scene_takes_to_build() {
        // A second scene of 10 000 fixed parts, built after the line's: the line waits for it, so
        // its models step before each of its steps from the first, and the carton stays put.
        let dir = scratch("two");
        let mut big = String::from("format = \"veredas-cena\"\nversion = 1\ngravity = 0\n");
        for i in 0..10_000 {
            big.push_str(&format!("[[fixed]]\nname = \"f{i}\"\nsize = [0.1, 0.1, 0.1]\nat = [{}.0, {}.0, 50.0]\n", i % 100 - 50, i / 100 - 50));
        }
        std::fs::write(dir.join("big.cena.toml"), big).unwrap();
        let file = sixth().replace("[[reflexo]]", "[[fisica]]\nname = \"big\"\nscene = \"big.cena.toml\"\n\n[[reflexo]]");
        let plant = Bench::start(bench::load(&file, &dir).unwrap()).unwrap();
        let scene = plant.fisica("line").unwrap();
        let models = plant.reflexo("line-models").unwrap();
        std::thread::sleep(Duration::from_millis(100));
        let (scene_steps, model_steps, x) = scene.with(|w| (w.steps(), models.status().steps, w.body("carton-1").unwrap().at[0]));
        plant.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(scene_steps > 0 && scene_steps == model_steps, "scene {scene_steps}, models {model_steps}");
        assert!((x - 0.3).abs() < 1e-3, "the carton at {x}, its conveyor never run");
    }

    #[test]
    fn drives_without_a_scene_step_every_4_ms() {
        let server = modbus::start(&toml::from_str("port = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0, value = \"true\" }, { name = \"sp\", table = \"holding-register\", address = 0, value = \"20\" } ]").unwrap()).unwrap();
        let points: Arc<dyn Points> = Arc::new(server.points());
        let cfg: ReflexoConfig = toml::from_str("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 40, decel = 40 } ]").unwrap();
        let models = Models::new(&cfg, reflexo::STEP_WITHOUT_SCENE, &|p: &str| (p == "m").then(|| points.clone())).unwrap();
        let begun = Instant::now();
        let mut handle = reflexo::start(models, None).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        // A step is never taken before its time; one late is taken at once, so a second holds 250
        // less what a stall of the host still owes (200 ms allowed).
        let steps = u128::from(handle.status().steps);
        let due = begun.elapsed().as_nanos() / 4_000_000;
        handle.stop();
        assert!(steps <= due && steps + 50 >= due, "{steps} steps, {due} due");
    }
}

/// Refute round 2's probes (the critic's, `~/.cache/veredas/critic-c115-r2/probes`), kept as pins:
/// the stepping order of every kind and within each, every adapter point type converted, an
/// actuator skipped at a failed switch write, the ramp's count however long, and a stop waiting
/// for a scene's step under way.
mod round_2 {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
    use std::sync::Condvar;

    #[test]
    fn conveyors_step_after_drives_and_actuators_after_conveyors() {
        // A conveyor run by a drive's running point starts in the same step.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("ra", Bool, 0.0)]);
        let mut x = models("drive = [ { name = \"a\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.ra\" } ]\nconveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.ra\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        x.step(Some(&mut w));
        assert!((x.speed("c").unwrap() - 0.4).abs() < 1e-12, "{:?}", x.speed("c"));
        // An actuator extended by a conveyor's running point moves in the same step.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("rc", Bool, 0.0)]);
        let mut x = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.rc\" } ]\nactuator = [ { name = \"p\", mover = \"pusher\", extend = \"m.rc\", retracted = { at = [3.5, -0.4, 0.95] }, extended = { at = [3.5, 0.25, 0.95] }, stroke_ms = 100 } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        x.step(Some(&mut w));
        assert!((x.position("p").unwrap() - 0.04).abs() < 1e-12, "{:?}", x.position("p"));
    }

    #[test]
    fn sensors_step_after_actuators() {
        // An actuator extended by a sensor model's point reads it as the last step left it.
        let m = mem(&[("s", Bool, 0.0)]);
        let mut x = models("actuator = [ { name = \"p\", mover = \"m\", extend = \"m.s\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 0.01, 0.0] }, stroke_ms = 100 } ]\nsensor = [ { name = \"b\", sensor = \"S\", point = \"m.s\" } ]", &m);
        let mut w = sensor_scene();
        w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
        while m.get("s") == 0.0 {
            x.step(Some(&mut w));
            w.step();
            assert!(x.status().steps < 50);
        }
        assert_eq!(x.position("p"), Some(0.0), "the actuator stepped before the sensor");
        x.step(Some(&mut w));
        assert!((x.position("p").unwrap() - 0.04).abs() < 1e-12);
    }

    #[test]
    fn conveyors_actuators_and_sensors_each_step_in_the_file_s_order() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("r1", Bool, 0.0)]);
        let mut x = models("conveyor = [ { name = \"c1\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.r1\" }, { name = \"c2\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.r1\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        x.step(Some(&mut w));
        assert!((x.speed("c2").unwrap() - 0.4).abs() < 1e-12, "{:?}", x.speed("c2"));
        let m = mem(&[("ext", Bool, 1.0), ("x1", Bool, 0.0)]);
        let mut x = models("actuator = [ { name = \"a1\", mover = \"pusher\", extend = \"m.ext\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 1.0, 0.0] }, stroke_ms = 1, extended_switch = \"m.x1\" }, { name = \"a2\", mover = \"pusher\", extend = \"m.x1\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 1.0, 0.0] }, stroke_ms = 100 } ]", &m);
        x.step(None);
        assert!((x.position("a2").unwrap() - 0.04).abs() < 1e-12, "{:?}", x.position("a2"));
        // Two sensor models refused in one step: the last named is the file's last.
        let m = mem(&[]);
        *m.refusing.lock().unwrap() = Some("z".into());
        let mut x = models("sensor = [ { name = \"a\", sensor = \"S\", point = \"m.z\" }, { name = \"b\", sensor = \"S\", point = \"m.z\" } ]", &m);
        x.step(None);
        assert_eq!(x.status().last_error.as_deref(), Some("sensor `b`: m.z: refused"));
    }

    #[test]
    fn every_adapter_point_type_is_converted() {
        let adapter = ethernetip::start(&toml::from_str(
            "bind = \"127.0.0.1\"\nport = 0\nio_port = 0\nassemblies = [ { instance = 100, size = 32 } ]\npoints = [ { name = \"q\", assembly = 100, byte = 0, bit = 1 }, { name = \"u16\", assembly = 100, byte = 2, type = \"u16\" }, { name = \"i16\", assembly = 100, byte = 4, type = \"i16\" }, { name = \"u32\", assembly = 100, byte = 8, type = \"u32\" }, { name = \"i32\", assembly = 100, byte = 12, type = \"i32\" }, { name = \"f32\", assembly = 100, byte = 16, type = \"f32\" } ]",
        ).unwrap())
        .unwrap();
        let e = adapter.points();
        for (point, v, read) in [("q", f64::NAN, 0.0f64), ("u16", 300.0, 300.0), ("u16", 70_000.0, 65_535.0), ("i16", -5.0, -5.0), ("u32", 2.6, 3.0), ("i32", -7.0, -7.0), ("f32", f64::NAN, 0.0), ("f32", 1.25, 1.25)] {
            e.write(point, v).unwrap();
            assert_eq!(e.read(point).unwrap().to_bits(), read.to_bits(), "{point} {v}");
        }
    }

    #[test]
    fn an_actuator_whose_switch_write_fails_is_skipped_for_the_rest_of_its_step() {
        let m = mem(&[("ext", Bool, 0.0), ("back", Bool, 0.0), ("out", Bool, 7.0)]);
        let mut a = models("actuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.ext\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 1.0, 0.0] }, stroke_ms = 100, retracted_switch = \"m.back\", extended_switch = \"m.out\" } ]", &m);
        *m.refusing.lock().unwrap() = Some("back".into());
        a.step(None);
        let s = a.status();
        assert_eq!((s.errors, s.last_error.as_deref(), m.get("out")), (1, Some("actuator `a`: m.back: refused"), 7.0));
    }

    #[test]
    fn a_ramp_s_steps_are_its_time_s_however_many() {
        // 25515 at 0.3 a second is 85 050 s, exactly 21 262 500 steps of 4 ms: a slack of a
        // billionth of a step counted one more (the division's rounding past 2^24 steps).
        for (distance, rate, steps) in [(25515.0, 0.3, 21_262_500u64), (25518.0, 0.3, 21_265_000), (51030.0, 0.6, 21_262_500), (51033.0, 0.6, 21_263_750), (25516.0, 0.3, 21_263_334), (1.0, 0.3, 834), (50.0, 100.0, 125), (49.0, 200.0, 62), (1.0, 200.0, 2)] {
            assert_eq!(reflexo::ramp_steps(distance, rate, 0.004), steps, "{distance} at {rate}");
        }
    }

    /// Points whose `run` read, once armed, holds the step inside it until opened.
    #[derive(Default)]
    struct Gate {
        armed: AtomicBool,
        entered: AtomicBool,
        open: Mutex<bool>,
        cv: Condvar,
    }

    impl Points for Gate {
        fn kind(&self, _: &str) -> Option<PointKind> {
            None
        }
        fn read(&self, point: &str) -> Result<f64, String> {
            if point == "run" && self.armed.load(SeqCst) {
                self.entered.store(true, SeqCst);
                let mut open = self.open.lock().unwrap();
                while !*open {
                    open = self.cv.wait(open).unwrap();
                }
            }
            Ok(if point == "run" { 1.0 } else { 10.0 })
        }
        fn write(&self, _: &str, _: f64) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn a_stop_waits_for_a_scene_s_step_under_way() {
        let gate = Arc::new(Gate::default());
        let points: Arc<dyn Points> = gate.clone();
        let cfg: ReflexoConfig = toml::from_str("drive = [ { name = \"d\", run = \"g.run\", setpoint = \"g.sp\", accel = 100, decel = 100 } ]").unwrap();
        let models = Models::new(&cfg, STEP, &|p: &str| (p == "g").then(|| points.clone())).unwrap();
        let s = "format = \"veredas-cena\"\nversion = 1\ngravity = 0\n[[sensor]]\nname = \"S\"\nradius = 0.1\nat = [0.0, 0.0, 0.0]\n";
        let mut scene = fisica::paused(&fisica::load(s).unwrap()).unwrap();
        let handle = reflexo::start(models, Some(&scene)).unwrap();
        scene.run().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        gate.armed.store(true, SeqCst);
        assert!(until(Duration::from_secs(2), || gate.entered.load(SeqCst)), "no step entered the gate");
        // A stop while the step is held inside its read does not return till the step ends.
        let returned = Arc::new(AtomicBool::new(false));
        let r = Arc::clone(&returned);
        let t = std::thread::spawn(move || {
            let mut handle = handle;
            handle.stop();
            r.store(true, SeqCst);
            handle
        });
        std::thread::sleep(Duration::from_millis(100));
        let early = returned.load(SeqCst);
        gate.armed.store(false, SeqCst);
        *gate.open.lock().unwrap() = true;
        gate.cv.notify_all();
        let handle = t.join().unwrap();
        let steps = handle.status().steps;
        std::thread::sleep(Duration::from_millis(50));
        let later = handle.status().steps;
        scene.stop();
        assert!(!early, "the stop returned while the scene's step was under way");
        assert_eq!(later, steps, "a step after the stop");
    }
}

/// Refute round 3's probes, kept.
mod round_3 {
    use super::*;

    #[test]
    fn an_infinite_setpoint_is_ramped_at_the_acceleration_and_left_at_the_deceleration() {
        assert_eq!(reflexo::ramp_steps(f64::INFINITY, 100.0, 0.004), u64::MAX);
        assert_eq!(reflexo::ramp_steps(4e9, 1e-300, 0.004), u64::MAX);
        // An f32 holding register a client the bench did not write sets to +inf.
        let server = modbus::start(&toml::from_str("port = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0, value = \"true\" }, { name = \"sp\", table = \"holding-register\", address = 0, type = \"f32\" }, { name = \"speed\", table = \"input-register\", address = 0, type = \"f32\" } ]").unwrap()).unwrap();
        let addr = server.addr;
        runtime().block_on(async {
            let mut ctx = tokio_modbus::client::tcp::connect_slave(addr, Slave(1)).await.unwrap();
            ctx.write_multiple_registers(0, &[0x7F80, 0x0000]).await.unwrap().unwrap();
        });
        assert_eq!(server.get("sp").unwrap(), modbus::Value::F32(f32::INFINITY));
        let points: Arc<dyn Points> = Arc::new(server.points());
        let cfg: ReflexoConfig = toml::from_str("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, speed = \"m.speed\" } ]").unwrap();
        let mut d = Models::new(&cfg, STEP, &|p: &str| (p == "m").then(|| points.clone())).unwrap();
        for k in 1..=200u32 {
            d.step(None);
            assert!((d.speed("d").unwrap() - 0.4 * f64::from(k)).abs() < 1e-9, "step {k}: {:?}", d.speed("d"));
        }
        assert_eq!(server.get("speed").unwrap(), modbus::Value::F32(80.0));
        // Back to 50: down from 80 at 0.4 a step, on 50 at the 75th.
        server.set_value("sp", modbus::Value::F32(50.0)).unwrap();
        d.step(None);
        assert!((d.speed("d").unwrap() - 79.6).abs() < 1e-9, "{:?}", d.speed("d"));
        for _ in 2..75 {
            d.step(None);
            assert!(d.speed("d").unwrap() > 50.0);
        }
        d.step(None);
        assert_eq!(d.speed("d"), Some(50.0));
        assert_eq!(d.status().errors, 0);
        server.stop();
        // A conveyor at an infinite setpoint: its belt at 0.4 units a step times its ratio.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, f64::INFINITY)]);
        let mut c = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        c.step(Some(&mut w));
        assert!((w.speed("infeed").unwrap() - 0.004).abs() < 1e-12, "{:?}", w.speed("infeed"));
    }

    /// The steps a drive takes from `-sp` (reversed) to `sp`, each step's running point with them.
    fn reversal(sp: u64, decel: u64, accel: u64, back: bool) -> (u64, Vec<f64>) {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, sp as f64), ("rev", Bool, if back { 0.0 } else { 1.0 }), ("running", Bool, 9.0)]);
        let mut d = models(&format!("drive = [ {{ name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = {accel}, decel = {decel}, running = \"m.running\" }} ]"), &m);
        let from = if back { sp as f64 } else { -(sp as f64) };
        while d.speed("d") != Some(from) {
            d.step(None);
        }
        m.set("rev", if back { 1.0 } else { 0.0 });
        let (mut k, mut running) = (0u64, Vec::new());
        while d.speed("d") != Some(-from) {
            d.step(None);
            k += 1;
            running.push(m.get("running"));
            assert!(k < 1_000_000);
        }
        (k, running)
    }

    #[test]
    fn a_ramp_through_0_lands_at_the_first_step_at_or_after_its_whole_time() {
        // -50 to +50 at a deceleration of 160 and an acceleration of 120: 50/160 + 50/120 s is
        // 182.29 steps of 4 ms, so the 183rd; running at every step, 0 passed within the 79th.
        let (k, running) = reversal(50, 160, 120, false);
        assert_eq!(k, 183);
        assert!(running.iter().all(|r| *r == 1.0), "a step not running during the reversal");
        // Every pair of rates from 60 to 260 in tens, both ways: 250 sp / decel + 250 sp / accel
        // steps, rounded up, in integers.
        let mut off = Vec::new();
        for decel in (60u64..=260).step_by(10) {
            for accel in (60u64..=260).step_by(10) {
                let want = (250 * 50 * accel + 250 * 50 * decel).div_ceil(decel * accel);
                for back in [false, true] {
                    let (k, _) = reversal(50, decel, accel, back);
                    if k != want {
                        off.push((decel, accel, back, k, want));
                    }
                }
            }
        }
        assert!(off.is_empty(), "reversals off their step (decel, accel, back, got, want): {off:?}");
    }

    /// A bench of two `[[reflexo]]` parts on one scene: a conveyor of `first`, run by the running
    /// point of a drive of `second`, at the scene step where the drive passes 2.
    fn two_parts(first_is_the_conveyor: bool) -> (f64, f64) {
        let dir = scratch(if first_is_the_conveyor { "two-parts-c" } else { "two-parts-d" });
        let belts = "[[reflexo]]\nname = \"belts\"\nscene = \"sc\"\nconveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.ra\", setpoint = \"m.sp\", accel = 100, decel = 100 } ]\n";
        let drives = "[[reflexo]]\nname = \"drives\"\nscene = \"sc\"\ndrive = [ { name = \"a\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.ra\" } ]\n";
        let (one, two) = if first_is_the_conveyor { (belts, drives) } else { (drives, belts) };
        let file = format!("format = \"veredas-bench\"\nversion = 1\n[[modbus]]\nname = \"m\"\nport = 0\npoints = [ {{ name = \"run\", table = \"coil\", address = 0, value = \"true\" }}, {{ name = \"sp\", table = \"holding-register\", address = 0, value = \"50\" }}, {{ name = \"ra\", table = \"discrete-input\", address = 0 }} ]\n[[fisica]]\nname = \"sc\"\nscene = \"line.cena.toml\"\n{one}{two}");
        let plant = Bench::start(bench::load(&file, &dir).unwrap()).unwrap();
        let scene = plant.fisica("sc").unwrap();
        let (belts, drives) = (plant.reflexo("belts").unwrap(), plant.reflexo("drives").unwrap());
        assert!(until(Duration::from_secs(5), || scene.with(|_| drives.speed("a").unwrap()) > 2.0));
        let speeds = scene.with(|_| (drives.speed("a").unwrap(), belts.speed("c").unwrap()));
        plant.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(speeds.0 < 40.0, "{speeds:?}");
        speeds
    }

    #[test]
    fn two_parts_on_one_scene_step_one_after_the_other_in_the_file_s_order() {
        // The conveyor's part first: it reads the drive's running point as the step before left
        // it, a step behind.
        let (a, c) = two_parts(true);
        assert!((a - c - 0.4).abs() < 1e-9, "a {a}, c {c}");
        // The drive's part first: the conveyor starts in the drive's step.
        let (a, c) = two_parts(false);
        assert!((a - c).abs() < 1e-9, "a {a}, c {c}");
    }

    #[test]
    fn at_speed_is_the_run_on_and_the_speed_within_its_tolerance_a_setpoint_of_0_included() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 0.0), ("running", Bool, 9.0), ("at", Bool, 9.0)]);
        let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, running = \"m.running\", at_speed = \"m.at\" } ]", &m);
        d.step(None);
        assert_eq!((m.get("running"), m.get("at")), (0.0, 1.0), "run on at a setpoint of 0");
        m.set("run", 0.0);
        d.step(None);
        assert_eq!((m.get("running"), m.get("at")), (0.0, 0.0), "run off");
        // Reversed at a setpoint of 0: 0, not -0, and at speed.
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 0.0), ("rev", Bool, 1.0), ("speed", Number, 9.0), ("at", Bool, 9.0)]);
        let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = 100, decel = 100, speed = \"m.speed\", at_speed = \"m.at\" } ]", &m);
        d.step(None);
        assert_eq!((m.get("speed").to_bits(), m.get("at")), (0.0f64.to_bits(), 1.0));
    }

    #[test]
    fn every_integer_type_rounds_half_away_from_0_and_holds_to_its_range() {
        let server = modbus::start(&toml::from_str(
            "port = 0\npoints = [ { name = \"u16\", table = \"holding-register\", address = 0 }, { name = \"i16\", table = \"holding-register\", address = 1, type = \"i16\" }, { name = \"u32\", table = \"holding-register\", address = 2, type = \"u32\" }, { name = \"i32\", table = \"holding-register\", address = 4, type = \"i32\" } ]",
        ).unwrap())
        .unwrap();
        let adapter = ethernetip::start(&toml::from_str(
            "bind = \"127.0.0.1\"\nport = 0\nio_port = 0\nassemblies = [ { instance = 100, size = 32 } ]\npoints = [ { name = \"u8\", assembly = 100, byte = 1 }, { name = \"i8\", assembly = 100, byte = 0, type = \"i8\" }, { name = \"u16\", assembly = 100, byte = 2, type = \"u16\" }, { name = \"i16\", assembly = 100, byte = 4, type = \"i16\" }, { name = \"u32\", assembly = 100, byte = 8, type = \"u32\" }, { name = \"i32\", assembly = 100, byte = 12, type = \"i32\" } ]",
        ).unwrap())
        .unwrap();
        let (mp, ep) = (server.points(), adapter.points());
        let devices: [(&str, &dyn Points, &[&str]); 2] = [("modbus", &mp, &["u16", "i16", "u32", "i32"]), ("adapter", &ep, &["u8", "i8", "u16", "i16", "u32", "i32"])];
        let mut wrong = Vec::new();
        for (device, p, types) in devices {
            for ty in types {
                let signed = ty.starts_with('i');
                let cases: &[(f64, f64)] = if signed { &[(-2.6, -3.0), (-2.5, -3.0), (-2.4, -2.0), (2.5, 3.0), (2.6, 3.0)] } else { &[(2.6, 3.0), (2.5, 3.0), (2.4, 2.0), (-0.4, 0.0), (-2.6, 0.0)] };
                for (v, want) in cases {
                    p.write(ty, *v).unwrap();
                    let got = p.read(ty).unwrap();
                    if got != *want {
                        wrong.push(format!("{device} {ty}: {v} read {got}, not {want}"));
                    }
                }
            }
        }
        server.stop();
        adapter.stop();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn a_belt_is_held_to_100_m_s_either_way() {
        for (sp, held) in [(500.0, 100.0), (-500.0, -100.0)] {
            let m = mem(&[("run", Bool, 1.0), ("sp", Number, sp)]);
            let mut c = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 1, run = \"m.run\", setpoint = \"m.sp\", accel = 1000000, decel = 1 } ]", &m);
            let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
            c.step(Some(&mut w));
            assert_eq!((c.speed("c"), w.speed("infeed"), c.status().errors), (Some(sp), Some(held), 0));
        }
    }

    #[test]
    fn a_conveyor_whose_write_fails_leaves_its_belt_for_the_rest_of_its_step() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 50.0), ("speed", Number, 0.0)]);
        let mut c = models("conveyor = [ { name = \"c\", belt = \"infeed\", metres_per_unit = 0.01, run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, speed = \"m.speed\" } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        *m.refusing.lock().unwrap() = Some("speed".into());
        c.step(Some(&mut w));
        assert_eq!((w.speed("infeed"), c.status().last_error.as_deref()), (Some(0.5), Some("conveyor `c`: m.speed: refused")));
        *m.refusing.lock().unwrap() = None;
        c.step(Some(&mut w));
        assert!((w.speed("infeed").unwrap() - 0.008).abs() < 1e-12, "{:?}", w.speed("infeed"));
    }

    #[test]
    fn an_actuator_whose_read_fails_is_skipped_for_the_rest_of_its_step() {
        let m = mem(&[("ext", Bool, 1.0), ("back", Bool, 7.0), ("out", Bool, 7.0)]);
        let mut a = models("actuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.ext\", retracted = { at = [0.0, 0.0, 0.0] }, extended = { at = [0.0, 1.0, 0.0] }, stroke_ms = 100, retracted_switch = \"m.back\", extended_switch = \"m.out\" } ]", &m);
        a.step(None);
        a.step(None);
        assert_eq!((a.position("a"), m.get("back"), m.get("out")), (Some(0.08), 0.0, 0.0));
        *m.unreadable.lock().unwrap() = Some("ext".into());
        m.set("back", 7.0);
        m.set("out", 7.0);
        a.step(None);
        let s = a.status();
        assert_eq!((a.position("a"), m.get("back"), m.get("out"), s.errors, s.last_error.as_deref()), (Some(0.08), 7.0, 7.0, 1, Some("actuator `a`: m.ext: unreadable")));
        *m.unreadable.lock().unwrap() = None;
        a.step(None);
        assert_eq!((a.position("a"), m.get("back")), (Some(0.12), 0.0));
    }

    #[test]
    fn an_off_delay_counts_the_scene_empty_without_a_break() {
        let m = mem(&[("no", Bool, 0.0)]);
        let mut s = models("sensor = [ { name = \"a\", sensor = \"S\", point = \"m.no\", off_delay_ms = 10 } ]", &m);
        let mut w = sensor_scene();
        let step = |s: &mut Models, w: &mut World| {
            s.step(Some(w));
            w.step();
        };
        w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
        while m.get("no") == 0.0 {
            step(&mut s, &mut w);
            assert!(s.status().steps < 50);
        }
        // Three absences of two steps (8 ms) against a 10 ms off delay: the point never drops.
        for _ in 0..3 {
            w.place("m", fisica::Placement::at([5.0, 0.0, 0.0])).unwrap();
            step(&mut s, &mut w);
            step(&mut s, &mut w);
            w.place("m", fisica::Placement::at([0.0, 0.0, 0.0])).unwrap();
            for _ in 0..4 {
                step(&mut s, &mut w);
                assert_eq!(m.get("no"), 1.0, "an absence shorter than the off delay seen");
            }
        }
        // One of three steps (12 ms) is seen.
        w.place("m", fisica::Placement::at([5.0, 0.0, 0.0])).unwrap();
        let mut k = 0;
        while m.get("no") == 1.0 {
            step(&mut s, &mut w);
            k += 1;
            assert!(k < 10);
        }
    }
}

/// C-115's fourth refute round: the critic's probes (`~/.cache/veredas/critic-c115-r4/probes`), kept
/// as pins: a drive skipped for the rest of its step, an actuator's line in every coordinate and
/// angle, a reversed drive's rates, a setpoint changed mid-reversal, nine point refusals, models on
/// an adapter's points and without a scene through the bench, a reversal's exact zero step.
mod round_4 {
    use super::*;

    const DRIVE: &str = "drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 100, decel = 100, tolerance = 100, speed = \"m.speed\", running = \"m.running\", at_speed = \"m.at\" } ]";

    #[test]
    fn a_drive_whose_read_fails_neither_ramps_nor_writes_for_the_rest_of_its_step() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("speed", Number, 7.0), ("running", Bool, 7.0), ("at", Bool, 7.0)]);
        let mut d = models(DRIVE, &m);
        d.step(None);
        d.step(None);
        assert!((d.speed("d").unwrap() - 0.8).abs() < 1e-12);
        for p in ["speed", "running", "at"] {
            m.set(p, 7.0);
        }
        *m.unreadable.lock().unwrap() = Some("sp".into());
        d.step(None);
        let s = d.status();
        assert_eq!((s.errors, s.last_error.as_deref()), (1, Some("drive `d`: m.sp: unreadable")));
        assert_eq!((d.speed("d"), m.get("speed"), m.get("running"), m.get("at")), (Some(0.8), 7.0, 7.0, 7.0));
    }

    #[test]
    fn a_drive_whose_write_fails_writes_nothing_after_it() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 10.0), ("speed", Number, 7.0), ("running", Bool, 7.0), ("at", Bool, 7.0)]);
        let mut d = models(DRIVE, &m);
        *m.refusing.lock().unwrap() = Some("running".into());
        d.step(None);
        assert_eq!((m.get("speed"), m.get("at")), (0.4, 7.0), "the at-speed point written after the refused running point");
    }

    #[test]
    fn an_actuator_moves_every_coordinate_and_angle_evenly() {
        let m = mem(&[("ext", Bool, 1.0)]);
        let mut a = models("actuator = [ { name = \"a\", mover = \"pusher\", extend = \"m.ext\", retracted = { at = [3.0, -0.4, 0.9] }, extended = { at = [3.4, 0.2, 1.3], yaw = 40, pitch = 20, roll = 30 }, stroke_ms = 100 } ]", &m);
        let mut w = World::new(&fisica::load(&cena()).unwrap()).unwrap();
        let mut off = Vec::new();
        for k in 1..=25u32 {
            a.step(Some(&mut w));
            w.step();
            let p = 0.04 * f64::from(k);
            let b = w.body("pusher").unwrap();
            let (want, want_angles) = ([3.0 + 0.4 * p, -0.4 + 0.6 * p, 0.9 + 0.4 * p], [40.0 * p, 20.0 * p, 30.0 * p]);
            let angles = b.angles();
            if (0..3).any(|i| (b.at[i] - want[i]).abs() > 1e-6 || (angles[i] - want_angles[i]).abs() > 1e-3) {
                off.push(format!("{k}: at {:?} for {want:?}, angles {angles:?} for {want_angles:?}", b.at));
            }
        }
        assert!(off.is_empty(), "{off:#?}");
    }

    #[test]
    fn a_reversed_drive_ramps_at_its_acceleration_away_from_0_and_its_deceleration_toward_it() {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 50.0), ("rev", Bool, 1.0)]);
        let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = 100, decel = 200 } ]", &m);
        // 0 to -50 at 100 a second: 0.4 a step, on -50 at the 125th.
        for k in 1..125u32 {
            d.step(None);
            assert!((d.speed("d").unwrap() + 0.4 * f64::from(k)).abs() < 1e-9, "step {k}: {:?}", d.speed("d"));
        }
        d.step(None);
        assert_eq!(d.speed("d"), Some(-50.0));
        // -50 to -20 at 200 a second: 0.8 a step, 37.5 steps, on -20 at the 38th.
        m.set("sp", 20.0);
        for k in 1..38u32 {
            d.step(None);
            assert!((d.speed("d").unwrap() + 50.0 - 0.8 * f64::from(k)).abs() < 1e-9, "step {k}: {:?}", d.speed("d"));
        }
        d.step(None);
        assert_eq!(d.speed("d"), Some(-20.0));
        // Stopped from -20: 25 steps at 0.8.
        m.set("run", 0.0);
        for _ in 1..25 {
            d.step(None);
            assert!(d.speed("d").unwrap() < 0.0);
        }
        d.step(None);
        assert_eq!(d.speed("d"), Some(0.0));
    }

    /// A drive at -50 reversed, then forward 30 steps into its ramp through 0 (at -30.8).
    fn mid_reversal() -> (Arc<Mem>, Models) {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, 50.0), ("rev", Bool, 1.0)]);
        let mut d = models("drive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = 120, decel = 160 } ]", &m);
        while d.speed("d") != Some(-50.0) {
            d.step(None);
        }
        m.set("rev", 0.0);
        for _ in 0..30 {
            d.step(None);
        }
        assert!((d.speed("d").unwrap() + 30.8).abs() < 1e-9, "{:?}", d.speed("d"));
        (m, d)
    }

    #[test]
    fn a_setpoint_changed_mid_reversal_is_ramped_to_and_landed_at_its_step() {
        // At -30.8 the setpoint becomes 30: 30.8 / 0.64 + 30 / 0.48 = 110.625 steps more, on 30 at
        // the 111th.
        let (m, mut d) = mid_reversal();
        m.set("sp", 30.0);
        for k in 1..111u32 {
            d.step(None);
            assert!(d.speed("d").unwrap() < 30.0, "step {k}: {:?}", d.speed("d"));
        }
        d.step(None);
        assert_eq!(d.speed("d"), Some(30.0));
        // The reverse put back mid-reversal: from -30.8 toward -50 at the acceleration again.
        let (m, mut d) = mid_reversal();
        m.set("rev", 1.0);
        d.step(None);
        assert!((d.speed("d").unwrap() + 30.8 + 0.48).abs() < 1e-9, "{:?}", d.speed("d"));
    }

    #[test]
    fn every_point_key_refuses_a_point_not_there_and_one_of_the_other_kind() {
        let dir = scratch("round4-keys");
        let base = "format = \"veredas-bench\"\nversion = 1\n[[modbus]]\nname = \"m\"\nport = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0 }, { name = \"sp\", table = \"holding-register\", address = 0 }, { name = \"b\", table = \"discrete-input\", address = 0 } ]\n[[fisica]]\nname = \"sc\"\nscene = \"line.cena.toml\"\n[[reflexo]]\nname = \"r\"\nscene = \"sc\"\n";
        let act = |extra: &str, extend: &str| format!("actuator = [ {{ name = \"a\", mover = \"pusher\", extend = \"{extend}\", retracted = {{ at = [0.0, 0.0, 0.0] }}, extended = {{ at = [0.0, 0.0, 0.0] }}, stroke_ms = 1{extra} }} ]\n");
        let drive = |extra: &str| format!("drive = [ {{ name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 1, decel = 1{extra} }} ]\n");
        let cases: Vec<(String, &str)> = vec![
            (act("", "m.nope"), "reflexo[0] (`r`).actuator[0] (`a`).extend: `m` has no point `nope`"),
            (act("", "m.sp"), "reflexo[0] (`r`).actuator[0] (`a`).extend: `m.sp` is a number, a bool point is wanted"),
            (act(", retracted_switch = \"m.sp\"", "m.run"), "reflexo[0] (`r`).actuator[0] (`a`).retracted_switch: `m.sp` is a number, a bool point is wanted"),
            (act(", retracted_switch = \"m.nope\"", "m.run"), "reflexo[0] (`r`).actuator[0] (`a`).retracted_switch: `m` has no point `nope`"),
            ("sensor = [ { name = \"s\", sensor = \"B1\", point = \"m.nope\" } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).point: `m` has no point `nope`"),
            ("sensor = [ { name = \"s\", sensor = \"B1\", point = \"m.sp\" } ]\n".into(), "reflexo[0] (`r`).sensor[0] (`s`).point: `m.sp` is a number, a bool point is wanted"),
            (drive(", speed = \"m.b\""), "reflexo[0] (`r`).drive[0] (`d`).speed: `m.b` is a bool, a number point is wanted"),
            (drive(", at_speed = \"m.sp\""), "reflexo[0] (`r`).drive[0] (`d`).at_speed: `m.sp` is a number, a bool point is wanted"),
            (drive(", speed = \"m.nope\""), "reflexo[0] (`r`).drive[0] (`d`).speed: `m` has no point `nope`"),
        ];
        let mut wrong = Vec::new();
        for (models, want) in cases {
            match bench::load(&format!("{base}{models}"), &dir) {
                Err(e) if e.to_string() == want => {}
                Err(e) => wrong.push(format!("{models}: {e}")),
                Ok(_) => wrong.push(format!("loaded: {models}")),
            }
        }
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(wrong.is_empty(), "{wrong:#?}");
    }

    #[test]
    fn the_bench_binds_a_model_to_an_adapter_s_points() {
        let dir = scratch("round4-adapter");
        let file = "format = \"veredas-bench\"\nversion = 1\n[[ethernetip]]\nname = \"x\"\nbind = \"127.0.0.1\"\nport = 0\nio_port = 0\nassemblies = [ { instance = 100, size = 4 } ]\npoints = [ { name = \"run\", assembly = 100, byte = 0, bit = 0, value = \"true\" }, { name = \"sp\", assembly = 100, byte = 1, value = \"2\" }, { name = \"speed\", assembly = 100, byte = 2 } ]\n[[reflexo]]\nname = \"r\"\ndrive = [ { name = \"d\", run = \"x.run\", setpoint = \"x.sp\", accel = 100, decel = 100, speed = \"x.speed\" } ]\n";
        let plant = Bench::start(bench::load(file, &dir).unwrap()).unwrap();
        assert!(until(Duration::from_secs(3), || plant.ethernetip("x").unwrap().get("speed").unwrap() == ethernetip::Value::U8(2)), "the adapter's speed point never reached 2");
        plant.stop();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_bench_steps_a_part_without_a_scene_every_4_ms() {
        let dir = scratch("round4-sceneless");
        let file = "format = \"veredas-bench\"\nversion = 1\n[[modbus]]\nname = \"m\"\nport = 0\npoints = [ { name = \"run\", table = \"coil\", address = 0, value = \"true\" }, { name = \"sp\", table = \"holding-register\", address = 0, value = \"1000\" } ]\n[[reflexo]]\nname = \"r\"\ndrive = [ { name = \"d\", run = \"m.run\", setpoint = \"m.sp\", accel = 250, decel = 250 } ]\n";
        let spec = bench::load(file, &dir).unwrap();
        assert_eq!(spec.plan()[1], "reflexo `r`: 1 model(s) every 4 ms");
        let begun = Instant::now();
        let plant = Bench::start(spec).unwrap();
        std::thread::sleep(Duration::from_secs(1));
        let models = plant.reflexo("r").unwrap();
        // 250 a second at 4 ms is 1 a step: the speed counts the steps. Two reads under two locks:
        // read again till no step came between them.
        let (steps, speed) = loop {
            let (before, speed, after) = (models.status().steps, models.speed("d").unwrap(), models.status().steps);
            if before == after {
                break (before, speed);
            }
        };
        let due = begun.elapsed().as_nanos() / 4_000_000;
        plant.stop();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!((speed - steps as f64).abs() < 1e-9, "a step of {} units, not 1 ({steps} steps)", speed / steps as f64);
        assert!(u128::from(steps) <= due && u128::from(steps) + 50 >= due, "{steps} steps, {due} due");
    }

    /// A reversal from -sp whose deceleration ends exactly on a step: that step's speed and running.
    fn zero_step(sp: u32, decel: u32, accel: u32) -> (u64, f64, f64) {
        let m = mem(&[("run", Bool, 1.0), ("sp", Number, f64::from(sp)), ("rev", Bool, 1.0), ("running", Bool, 9.0)]);
        let mut d = models(&format!("drive = [ {{ name = \"d\", run = \"m.run\", setpoint = \"m.sp\", reverse = \"m.rev\", accel = {accel}, decel = {decel}, running = \"m.running\" }} ]"), &m);
        while d.speed("d") != Some(-f64::from(sp)) {
            d.step(None);
        }
        m.set("rev", 0.0);
        assert_eq!(u64::from(sp) * 250 % u64::from(decel), 0);
        let z = u64::from(sp * 250 / decel);
        for _ in 0..z {
            d.step(None);
        }
        (z, d.speed("d").unwrap(), m.get("running"))
    }

    #[test]
    fn a_reversal_s_exact_zero_step_is_0_whatever_the_rounding() {
        // 9 / (18 x 0.004) came out 124.99999999999999 and the 125th step 1e-15, running.
        for (sp, rate) in [(50u32, 100u32), (9, 18), (11, 22), (7, 14), (13, 26)] {
            let (z, speed, running) = zero_step(sp, rate, rate);
            assert_eq!((speed.to_bits(), running), (0f64.to_bits(), 0.0), "-{sp} to {sp} at {rate}: step {z}");
        }
    }
}

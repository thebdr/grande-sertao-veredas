//! C-292, the parts of the proof that need no machine: the page found on a screen by its own canvas
//! colour, read from the release's page; the OVA read as VMware reads it, each fault named; the
//! proof's faults for a missing card line, a page not drawn and a changed disk. The proof of a real
//! machine is tests/maquina.rs (feature `maquina`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use entrega::machine::Recipe;
use entrega::maquina::{self, Proof};
use entrega::together::{Answer, Report};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A PPM of `w` by `h` pixels: its left `page` columns in `canvas`, the rest a gradient of other
/// colours, as a desktop's wallpaper.
fn ppm(w: usize, h: usize, page: usize, canvas: (u8, u8, u8)) -> Vec<u8> {
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for y in 0..h {
        for x in 0..w {
            if x < page {
                out.extend([canvas.0, canvas.1, canvas.2]);
            } else {
                out.extend([(x % 251) as u8, (y % 241) as u8, 200]);
            }
        }
    }
    out
}

#[test]
fn the_page_is_found_by_its_own_canvas_colour() {
    let head = entrega::git::text(&repo_root(), &["rev-parse", "HEAD"]).unwrap().unwrap().trim().to_string();
    let canvas = maquina::page_canvas(&repo_root(), &head).unwrap();
    assert_eq!(canvas, (0x14, 0x14, 0x16), "supervisão's `--canvas`");
    let drawn = ppm(400, 200, 200, canvas);
    let share = maquina::share_of(&drawn, canvas).unwrap();
    assert!((share - 0.5).abs() < 0.01, "{share}");
    // A desktop of many colours without the page: no share at all, however many colours it has.
    let bare = ppm(400, 200, 0, canvas);
    assert_eq!(maquina::share_of(&bare, canvas), Some(0.0));
    assert!(maquina::colours(&bare).unwrap() > 200, "a wallpaper alone clears any count of colours");
    assert_eq!(maquina::share_of(b"P5\n1 1\n255\n\0", canvas), None, "no binary PPM");
}

fn report(page: u16) -> Report {
    let snapshot = |place: f64| {
        let objects: Vec<serde_json::Value> = ["speed", "read-rate", "infeed", "outlet-1", "outlet-2", "outlet-3", "parcel-1", "parcel-2"]
            .iter()
            .map(|id| serde_json::json!({ "id": id, "value": 1.0, "stale": false, "place": place }))
            .collect();
        serde_json::json!({ "objects": objects, "session": { "health": { "plc": { "state": "live" }, "sorter": { "state": "live" } } } })
    };
    Report {
        page: Answer { status: page, body: "<html></html>".into() },
        first: snapshot(0.1),
        second: snapshot(0.2),
        apart: Duration::from_secs(2),
        write: Answer { status: 403, body: r#"{"error":"writes are turned off for this project"}"#.into() },
        bench_exit: None,
        supervisao_exit: None,
        bench_out: String::new(),
        supervisao_out: String::new(),
    }
}

fn proof(console: &str, share: f64, after: &str) -> Proof {
    Proof {
        vmdk: PathBuf::from("disk.vmdk"),
        ova: vec![],
        console: console.into(),
        plant: report(200),
        screen: PathBuf::from("screen.ppm"),
        colours: 1000,
        page_share: share,
        before: "a".into(),
        after: after.into(),
    }
}

#[test]
fn the_proof_names_a_missing_card_a_page_not_drawn_and_a_changed_disk() {
    let card = format!("VEREDAS-CARD ens4 {}", maquina::PROOF_MAC);
    let good = format!("vmw_pvscsi\nvmxnet3\nVEREDAS-SUMS-OK 2.0.0\n{card}\n");
    assert_eq!(maquina::proof_faults(&proof(&good, 0.3, "a"), "2.0.0"), Vec::<String>::new());
    let no_card = good.replace(&card, "VEREDAS-CARD ens4 52:54:00:12:34:56");
    assert_eq!(
        maquina::proof_faults(&proof(&no_card, 0.3, "a"), "2.0.0"),
        vec![format!("the guest named no card with the proof's address {}", maquina::PROOF_MAC)]
    );
    assert_eq!(
        maquina::proof_faults(&proof(&good, 0.05, "a"), "2.0.0"),
        vec!["the page covers 5 % of the screen in its canvas colour, under 15 %: the browser does not draw it (screen.ppm)".to_string()]
    );
    assert_eq!(maquina::proof_faults(&proof(&good, 0.3, "b"), "2.0.0"), vec!["the proof boot changed the shipped disk".to_string()]);
    assert_eq!(
        maquina::proof_faults(&proof(&good.replace("VEREDAS-SUMS-OK 2.0.0", "VEREDAS-SUMS-FAILED 2.0.0"), 0.3, "a"), "2.0.0"),
        vec!["the boot did not check the release's sums (VEREDAS-SUMS-OK 2.0.0)".to_string()]
    );
}

#[test]
fn an_ova_is_read_as_vmware_reads_it_each_fault_named() {
    let r = Recipe::load_file(&repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    let work = tempfile::tempdir().unwrap();
    let name = "veredas-9.9.9";
    let vmdk_name = format!("{name}-disk1.vmdk");
    let capacity = u64::from(r.machine.disk_gb) << 30;
    // Makes an OVA from a disk (`subformat`, `gb`) and a descriptor edited by `edit`, opened into a
    // folder of its own; returns its path and that folder.
    let make = |label: &str, subformat: &str, gb: u32, edit: &dyn Fn(String) -> String, order: [&str; 3]| -> (PathBuf, PathBuf) {
        let dir = work.path().join(label);
        std::fs::create_dir_all(&dir).unwrap();
        let disk = dir.join(&vmdk_name);
        entrega::vm::run("qemu-img", &["create", "-q", "-f", "vmdk", "-o", &format!("subformat={subformat}"), &disk.display().to_string(), &format!("{gb}G")], None).unwrap();
        let size = std::fs::metadata(&disk).unwrap().len();
        let ovf = edit(maquina::ovf(&r, name, &vmdk_name, size, capacity, "9.9.9"));
        std::fs::write(dir.join(format!("{name}.ovf")), &ovf).unwrap();
        let mf = format!(
            "SHA256({name}.ovf)= {}\nSHA256({vmdk_name})= {}\n",
            entrega::cache::sha256(&dir.join(format!("{name}.ovf"))).unwrap(),
            entrega::cache::sha256(&disk).unwrap()
        );
        std::fs::write(dir.join(format!("{name}.mf")), &mf).unwrap();
        let ova = work.path().join(format!("{label}.ova"));
        let names: Vec<String> = order.iter().map(|n| format!("{name}{n}")).collect();
        let (o, d) = (ova.display().to_string(), dir.display().to_string());
        let mut args = vec!["--format=ustar", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s", "-cf", &o, "-C", &d];
        args.extend(names.iter().map(String::as_str));
        entrega::vm::run("tar", &args, None).unwrap();
        let opened = work.path().join(format!("{label}-opened"));
        std::fs::create_dir_all(&opened).unwrap();
        entrega::vm::run("tar", &["-xf", &o, "-C", &opened.display().to_string()], None).unwrap();
        (ova, opened)
    };
    let same = |s: String| s;
    let right = [".ovf", ".mf", "-disk1.vmdk"];
    let faults = |label: &str, subformat: &str, gb: u32, edit: &dyn Fn(String) -> String, order: [&str; 3]| {
        let (ova, opened) = make(label, subformat, gb, edit, order);
        maquina::ova_faults(&r, &ova, "9.9.9", &opened).unwrap()
    };
    assert_eq!(faults("good", "streamOptimized", r.machine.disk_gb, &same, right), Vec::<String>::new());
    let f = faults("disordered", "streamOptimized", r.machine.disk_gb, &same, ["-disk1.vmdk", ".ovf", ".mf"]);
    assert!(f.len() == 1 && f[0].starts_with("the OVA's members are"), "{f:?}");
    // The disk itself: another subformat, another size than the descriptor states.
    let f = faults("sparse", "monolithicSparse", r.machine.disk_gb, &same, right);
    assert_eq!(f, ["the disk is no stream-optimized VMDK (\"vmdk\" \"monolithicSparse\")"]);
    let f = faults("small", "streamOptimized", 8, &same, right);
    assert_eq!(f, [format!("the disk holds {} bytes, the descriptor states {capacity}", 8u64 << 30)]);
    // The descriptor's values inside their own elements.
    let swap = |s: String| {
        let cpus = format!("<rasd:VirtualQuantity>{}</rasd:VirtualQuantity>", r.machine.cpus);
        let mem = format!("<rasd:VirtualQuantity>{}</rasd:VirtualQuantity>", r.machine.memory_mb);
        s.replacen(&cpus, "@CPU@", 1).replacen(&mem, &cpus, 1).replacen("@CPU@", &mem, 1)
    };
    let f = faults("swapped", "streamOptimized", r.machine.disk_gb, &swap, right);
    assert_eq!(
        f,
        [
            format!("the descriptor's processors are not one item of {}", r.machine.cpus),
            format!("the descriptor's memory is not one item of {} MB", r.machine.memory_mb)
        ]
    );
    let orphan = |s: String| s.replacen("<rasd:Parent>3</rasd:Parent>", "<rasd:Parent>9</rasd:Parent>", 1);
    let f = faults("orphan", "streamOptimized", r.machine.disk_gb, &orphan, right);
    assert_eq!(f, ["the descriptor's hard disk is not one item on vmdisk1 under the paravirtual SCSI controller"]);
    let broken = |s: String| s.replacen("</Envelope>", "", 1);
    let f = faults("broken", "streamOptimized", r.machine.disk_gb, &broken, right);
    assert!(f.len() == 1 && f[0].starts_with("the descriptor is not well-formed XML"), "{f:?}");
}

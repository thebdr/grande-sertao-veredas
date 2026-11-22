//! C-292: the tester machine, for real (feature `maquina`; docs/questions.md Q440). A trial cut of
//! HEAD, its bundle provisioned into the distribution's cloud image under QEMU and packed into an
//! OVA by the recipe; then the OVA read as VMware reads it (its members in order, the descriptor,
//! the manifest of sums), and the disk taken out of it booted copy-on-write on QEMU's models of
//! VMware's devices with a card address the provisioning never saw: the release's files checked
//! at boot, the plant live from the first boot with nobody logged in, reached from the host, its
//! write refused; the desktop drawn; the disk's bytes unchanged after. What only VMware proves
//! (the import, VMware's own devices and display, the guest tools) is C-292's stated remainder.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use entrega::cut::{self, Options};
use entrega::machine::Recipe;
use entrega::{cache, maquina, together, vm};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A PPM screen dump's distinct colours, counted on a coarse grid.
fn colours(ppm: &[u8]) -> usize {
    let text = String::from_utf8_lossy(&ppm[..ppm.len().min(64)]).into_owned();
    let mut fields = text.split_ascii_whitespace();
    assert_eq!(fields.next(), Some("P6"), "a PPM screen dump");
    let w: usize = fields.next().unwrap().parse().unwrap();
    let h: usize = fields.next().unwrap().parse().unwrap();
    let _max = fields.next().unwrap();
    let header = ppm.len() - w * h * 3;
    let mut seen = std::collections::BTreeSet::new();
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let i = header + (y * w + x) * 3;
            seen.insert((ppm[i], ppm[i + 1], ppm[i + 2]));
        }
    }
    seen.len()
}

#[test]
fn the_machine_is_built_by_the_recipe_and_its_shipped_disk_boots_the_plant_on_vmwares_devices() {
    let root = repo_root();
    let release = "0.0.0-trial.292";
    let out = tempfile::tempdir().unwrap();
    let work = root.join("target/entrega").join(release);
    let cut = cut::cut(&root, &Options { release: semver::Version::parse(release).unwrap(), out: out.path().to_path_buf(), work: work.clone(), run_suite: false })
        .unwrap_or_else(|e| panic!("the trial cut was refused: {e}"));
    assert_eq!(together::faults(&cut.report), Vec::<String>::new());
    let target = "x86_64-unknown-linux-gnu";
    let paths = maquina::from_release(&root, release, target).unwrap_or_else(|e| panic!("the machine was not built: {e}"));
    let recipe = Recipe::load_file(&root.join("tools/entrega/maquina/maquina.toml")).unwrap();

    // The OVA as VMware reads it: the descriptor first, the manifest second, the disk last.
    let name = format!("veredas-{release}");
    assert_eq!(paths.ova, work.join("maquina").join(format!("{name}.ova")));
    let members = vm::run("tar", &["-tf", &paths.ova.display().to_string()], None).unwrap();
    assert_eq!(members, format!("{name}.ovf\n{name}.mf\n{name}-disk1.vmdk\n"));
    let verbose = vm::run("tar", &["--full-time", "-tvf", &paths.ova.display().to_string()], None).unwrap();
    assert!(verbose.lines().all(|l| l.starts_with("-rw") && l.contains(" root/root ")), "{verbose}");
    // The OVA opened under target/ (on disk): /tmp is a tmpfs of the machine's memory.
    let opened = work.join("opened");
    let _ = std::fs::remove_dir_all(&opened);
    std::fs::create_dir_all(&opened).unwrap();
    vm::run("tar", &["-xf", &paths.ova.display().to_string(), "-C", &opened.as_path().display().to_string()], None).unwrap();
    let vmdk = opened.as_path().join(format!("{name}-disk1.vmdk"));
    let ovf = std::fs::read_to_string(opened.as_path().join(format!("{name}.ovf"))).unwrap();
    let size = std::fs::metadata(&vmdk).unwrap().len();
    let info: serde_json::Value = serde_json::from_str(&vm::run("qemu-img", &["info", "--output=json", &vmdk.display().to_string()], None).unwrap()).unwrap();
    assert_eq!(info["format"], "vmdk");
    assert_eq!(info["format-specific"]["data"]["create-type"], "streamOptimized");
    let capacity = info["virtual-size"].as_u64().unwrap();
    assert_eq!(capacity, u64::from(recipe.machine.disk_gb) << 30);
    for needle in [
        format!(r#"<File ovf:id="file1" ovf:href="{name}-disk1.vmdk" ovf:size="{size}"/>"#),
        format!(r#"ovf:capacity="{capacity}" ovf:capacityAllocationUnits="byte""#),
        "vmdk.html#streamOptimized".to_string(),
        format!("<vssd:VirtualSystemType>vmx-{}</vssd:VirtualSystemType>", recipe.machine.hardware_version),
        "<rasd:ResourceSubType>VirtualSCSI</rasd:ResourceSubType>".to_string(),
        "<rasd:ResourceSubType>VmxNet3</rasd:ResourceSubType>".to_string(),
        format!("<rasd:VirtualQuantity>{}</rasd:VirtualQuantity>", recipe.machine.memory_mb),
        r#"vmw:key="firmware" vmw:value="bios""#.to_string(),
    ] {
        assert!(ovf.contains(&needle), "the descriptor holds no {needle}:\n{ovf}");
    }
    let lint = std::process::Command::new("xmllint").arg("--noout").arg(opened.as_path().join(format!("{name}.ovf"))).output().unwrap();
    assert!(lint.status.success(), "the descriptor is not well-formed XML: {}", String::from_utf8_lossy(&lint.stderr));
    let mf = std::fs::read_to_string(opened.as_path().join(format!("{name}.mf"))).unwrap();
    assert_eq!(
        mf,
        format!(
            "SHA256({name}.ovf)= {}\nSHA256({name}-disk1.vmdk)= {}\n",
            cache::sha256(&opened.as_path().join(format!("{name}.ovf"))).unwrap(),
            cache::sha256(&vmdk).unwrap()
        )
    );

    // The shipped disk booted on VMware's devices, a card address the provisioning never saw.
    let before = cache::sha256(&vmdk).unwrap();
    let port = vm::free_port().unwrap();
    let qmp = vm::free_port().unwrap();
    let console = work.join("proof-console.log");
    let mut machine = maquina::boot(&recipe, &vmdk, port, Some(qmp), &console).unwrap();
    machine.wait_for(&format!("VEREDAS-SUMS-OK {release}"), Duration::from_secs(300)).unwrap();
    let text = machine.console();
    assert!(text.contains("vmw_pvscsi") && text.contains("vmxnet3"), "the guest saw VMware's devices");
    assert!(!text.contains("VEREDAS-SUMS-FAILED"));
    let addr = ([127, 0, 0, 1], port).into();
    let snapshot = || together::http(addr, "GET", "/s/loop/objects.json", None).and_then(|a| serde_json::from_str::<serde_json::Value>(&a.body).ok());
    let start = Instant::now();
    let first = loop {
        if let Some(s) = snapshot()
            && s["objects"].as_array().is_some_and(|v| !v.is_empty() && v.iter().all(|o| o["value"].is_number()))
        {
            break s;
        }
        assert!(start.elapsed() < Duration::from_secs(300), "the plant never came live from the host; console:\n{}", vm::tail(&machine.console(), 40));
        std::thread::sleep(Duration::from_secs(1));
    };
    std::thread::sleep(Duration::from_secs(2));
    let second = snapshot().unwrap();
    let page = together::http(addr, "GET", "/", None).unwrap();
    let write = together::http(addr, "POST", "/s/loop/portal/api/opc/write", Some(r#"{"source":"plc","tag":"ns=2;s=release3","value":"1"}"#)).unwrap();
    let report = together::Report {
        page,
        first,
        second,
        apart: Duration::from_secs(2),
        write,
        bench_exit: Some(0),
        supervisao_exit: Some(0),
        bench_out: String::new(),
        supervisao_out: String::new(),
    };
    assert_eq!(together::faults(&report), Vec::<String>::new());
    assert_eq!(report.write.body, r#"{"error":"writes are turned off for this project"}"#);

    // The desktop, logged in by itself, draws the browser: a screen of many colours, not a console.
    let screen = work.join("proof-screen.ppm");
    let mut many = 0;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(240) {
        std::thread::sleep(Duration::from_secs(10));
        if vm::screendump(qmp, &screen).is_ok() && screen.exists() {
            many = colours(&std::fs::read(&screen).unwrap());
            if many > 200 {
                break;
            }
        }
    }
    assert!(many > 200, "the screen shows {many} colours: no desktop with a page ({})", screen.display());
    machine.kill();
    assert_eq!(cache::sha256(&vmdk).unwrap(), before, "the proof boot changed the shipped disk");
}

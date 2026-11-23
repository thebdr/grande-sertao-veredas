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
    let promised = "VEREDAS-SSH absent\nVEREDAS-CLOUD-INIT disabled\nVEREDAS-ON-PATH ok\n[    7.123998] systemd[1]: Initializing machine ID from random generator.\n";
    let good = format!("vmw_pvscsi\nvmxnet3\nVEREDAS-SUMS-OK 2.0.0\n{card}\n{promised}");
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
    // What the provisioning promised, each named when the boot does not say it.
    for (said, instead, fault) in [
        ("VEREDAS-SSH absent", "VEREDAS-SSH present", "the machine holds an SSH server (no `VEREDAS-SSH absent` on the console)"),
        ("VEREDAS-CLOUD-INIT disabled", "VEREDAS-CLOUD-INIT enabled", "cloud-init is not disabled (no `VEREDAS-CLOUD-INIT disabled` on the console)"),
        ("VEREDAS-ON-PATH ok", "VEREDAS-ON-PATH missing veredas-portal", "the release's executables are not all on the PATH (no `VEREDAS-ON-PATH ok` on the console)"),
        (
            "systemd[1]: Initializing machine ID from random generator.",
            "systemd[1]: Detected first boot.",
            "the shipped disk carries a machine identity (no `Initializing machine ID from random generator` on the console)",
        ),
    ] {
        assert_eq!(maquina::proof_faults(&proof(&good.replace(said, instead), 0.3, "a"), "2.0.0"), vec![fault.to_string()], "{said}");
    }
    // The OVA's own faults are the proof's: `entrega prova` fails on an OVA VMware would refuse.
    let mut bad_ova = proof(&good, 0.3, "a");
    bad_ova.ova = vec!["the descriptor's processors are not one item of 2".to_string()];
    assert_eq!(maquina::proof_faults(&bad_ova, "2.0.0"), vec!["the descriptor's processors are not one item of 2".to_string()]);
}

#[test]
fn the_provisioning_is_judged_finished_and_trimmed_and_its_boot_check_said_before_any_login() {
    let ok = "ENTREGA-STEP cleanup\n/boot/efi: 114.8 MiB (120395776 bytes) trimmed on /dev/vda15\n/: 12.9 GiB (13857087488 bytes) trimmed on /dev/vda1\nENTREGA-PROVISION-OK\nENTREGA-PROVISION-EXIT 0\n";
    assert_eq!(maquina::provision_faults(ok), Vec::<String>::new());
    assert_eq!(maquina::provision_faults(&ok.replace("ENTREGA-PROVISION-EXIT 0", "ENTREGA-PROVISION-EXIT 1")), ["no ENTREGA-PROVISION-EXIT 0"]);
    assert_eq!(maquina::provision_faults(&ok.replace("ENTREGA-PROVISION-OK\n", "")), ["no ENTREGA-PROVISION-OK"]);
    assert_eq!(
        maquina::provision_faults(&ok.replace("/: 12.9 GiB (13857087488 bytes) trimmed on /dev/vda1\n", "")),
        ["the root's free space was not trimmed (no `/: ... trimmed on` line)"]
    );
    // The script the machine is provisioned by: the SSH server purged, the executables linked onto
    // the PATH, cloud-init off, the identity blanked, the space trimmed, and the boot's check said
    // before a login starts on the serial console (whose agetty hangs the line up).
    let r = Recipe::load_file(&repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    assert_eq!(r.machine.remove, ["openssh-server"]);
    let s = maquina::provision_script(&r, "veredas-9.9.9-x86_64-unknown-linux-gnu", "9.9.9");
    for line in [
        "if [ -n \"openssh-server\" ]; then apt-get purge -y openssh-server; fi",
        "for b in /opt/veredas/current/bin/*; do ln -sf \"$b\" /usr/local/bin/; done",
        "touch /etc/cloud/cloud-init.disabled",
        "truncate -s 0 /etc/machine-id",
        "fstrim -av || true",
    ] {
        assert!(s.lines().any(|l| l == line), "{line}");
    }
    let unit = &s[s.find("cat > /etc/systemd/system/veredas-verifica.service").unwrap()..];
    let unit = &unit[..unit.find("\nUNIT\n").unwrap()];
    assert!(unit.lines().any(|l| l == "Before=serial-getty@ttyS0.service getty@tty1.service"), "{unit}");
    assert!(unit.lines().any(|l| l == "StandardOutput=journal+console"), "{unit}");
    // A recipe that leaves out what it removes is refused: removal is stated, never assumed.
    let text = std::fs::read_to_string(repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    let without: String = text.lines().filter(|l| !l.starts_with("remove = ")).map(|l| format!("{l}\n")).collect();
    assert_eq!(text.lines().count(), without.lines().count() + 1);
    match Recipe::load(&without) {
        Err(e) => assert!(e.to_string().contains("missing field `remove`"), "{e}"),
        Ok(_) => panic!("a recipe without `remove` was read"),
    }
}

#[test]
fn the_base_image_is_refused_unless_it_matches_its_pinned_digest() {
    let r = Recipe::load_file(&repo_root().join("tools/entrega/maquina/maquina.toml")).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let image = cache.path().join(r.base_name());
    std::fs::write(&image, "not the distribution's image\n").unwrap();
    let got = entrega::cache::sha512(&image).unwrap();
    match entrega::cache::base_image_in(cache.path(), &r) {
        Err(entrega::vm::VmError::Failed { program, status, stderr }) => {
            assert_eq!((program.as_str(), status.as_str()), ("the base image's check", "refused"));
            assert_eq!(stderr, format!("{} has SHA-512 {got}, the recipe pins {}", image.display(), r.base.sha512));
        }
        other => panic!("not refused: {other:?}"),
    }
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
    // Another tar format than ustar (GNU's): its listing reads the same, its headers do not.
    let (_, opened) = make("gnu", "streamOptimized", r.machine.disk_gb, &same, right);
    let gnu = work.path().join("gnu-format.ova");
    let names: Vec<String> = right.iter().map(|n| format!("{name}{n}")).collect();
    let (o, d) = (gnu.display().to_string(), opened.display().to_string());
    let mut args = vec!["--format=gnu", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s", "-cf", &o, "-C", &d];
    args.extend(names.iter().map(String::as_str));
    entrega::vm::run("tar", &args, None).unwrap();
    assert_eq!(maquina::ova_faults(&r, &gnu, "9.9.9", &opened).unwrap(), [r#"the OVA is no POSIX ustar archive: its first header's magic is "ustar  \0""#]);
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
    // Read as VMware reads it: by namespace and by section, never a bare tag found anywhere.
    let no_default_ns = |s: String| s.replacen(r#"<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" "#, "<Envelope ", 1);
    let f = faults("nons", "streamOptimized", r.machine.disk_gb, &no_default_ns, right);
    assert_eq!(f, ["the descriptor's root is no OVF Envelope (its namespace or its name)"]);
    let other_ovf = |s: String| s.replacen(r#"xmlns="http://schemas.dmtf.org/ovf/envelope/1""#, r#"xmlns="http://schemas.dmtf.org/ovf/envelope/2""#, 1);
    let f = faults("ovf2", "streamOptimized", r.machine.disk_gb, &other_ovf, right);
    assert_eq!(f, ["the descriptor's root is no OVF Envelope (its namespace or its name)"]);
    let bad_rasd = |s: String| s.replacen("CIM_ResourceAllocationSettingData", "CIM_ResourceAllocationSettingDat", 1);
    let f = faults("rasd", "streamOptimized", r.machine.disk_gb, &bad_rasd, right);
    assert_eq!(f.len(), 5, "processors, memory, controller, disk and card all unread: {f:?}");
    let bad_vssd = |s: String| s.replacen("CIM_VirtualSystemSettingData", "CIM_VirtualSystemSettingDat", 1);
    let f = faults("vssd", "streamOptimized", r.machine.disk_gb, &bad_vssd, right);
    assert_eq!(f, [format!("the descriptor's hardware version is not vmx-{:02}", r.machine.hardware_version)]);
    let moved = |s: String| {
        let start = s.find("      <Item>").unwrap();
        let end = s.find(r#"      <vmw:Config ovf:required="false" vmw:key="firmware""#).unwrap();
        let items = s[start..end].to_string();
        s.replacen(&items, "", 1).replacen("    </OperatingSystemSection>", &format!("{items}    </OperatingSystemSection>"), 1)
    };
    let f = faults("moved", "streamOptimized", r.machine.disk_gb, &moved, right);
    assert_eq!(f.len(), 5, "items outside VirtualHardwareSection are no hardware: {f:?}");
}

#[test]
fn a_machine_is_built_only_from_a_cut_that_ran_to_its_end() {
    use std::process::Command;
    // A throwaway repository with one component and a release's bundle where a cut leaves it, its
    // `release.toml` naming the repository's commit; the commit holds no guide, so a machine let
    // through stops there, before any machine starts.
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(root).args(["-c", "user.name=entrega", "-c", "user.email=entrega@example.invalid", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), "/target/\n").unwrap();
    std::fs::create_dir_all(root.join("tools/entrega/maquina")).unwrap();
    std::fs::copy(repo_root().join("tools/entrega/maquina/maquina.toml"), root.join("tools/entrega/maquina/maquina.toml")).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the recipe, no guide"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let release = "0.0.0-trial.9";
    let target = "x86_64-unknown-linux-gnu";
    let work = root.join("target/entrega").join(release);
    let name = format!("veredas-{release}-{target}");
    let text = entrega::cut::manifest_text(
        &semver::Version::parse(release).unwrap(),
        &commit,
        target,
        "rustc 1.100.0 (0ef1bc386 2026-11-09)",
        &[("veredas-core".to_string(), "2.0.0".to_string(), vec![])],
        entrega::cut::NOT_RUN,
    );
    std::fs::create_dir_all(work.join(&name)).unwrap();
    std::fs::write(work.join(&name).join("release.toml"), &text).unwrap();
    entrega::bundle::pack(&work, &name, "1", &work.join(format!("{name}.tar.gz"))).unwrap();
    std::fs::remove_dir_all(work.join(&name)).unwrap();
    let refusal = |r: Result<maquina::Paths, entrega::vm::VmError>| match r {
        Err(entrega::vm::VmError::Failed { program, status, stderr }) => {
            assert_eq!((program.as_str(), status.as_str()), ("the tester machine", "refused"));
            stderr
        }
        Err(e) => panic!("another error: {e}"),
        Ok(p) => panic!("built: {}", p.ova.display()),
    };
    let trial = work.join(format!("{release}.toml"));
    let real = root.join("releases").join(format!("{release}.toml"));

    // The bundle a refused cut left: no manifest where a cut writes one.
    assert_eq!(
        refusal(maquina::from_release(root, release, target)),
        format!(
            "release {release} was not cut to its end: no manifest at {} or {}, and a bundle without one is what a refused cut leaves",
            real.display(),
            trial.display()
        )
    );
    // A manifest there that is not the bundle's own, beside a trial's or a real cut's.
    for at in [&trial, &real] {
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text.replace(entrega::cut::NOT_RUN, "cargo nextest run --workspace")).unwrap();
        assert_eq!(refusal(maquina::from_release(root, release, target)), format!("{} is not the bundle's own release.toml: the bundle is not that cut's", at.display()));
        std::fs::remove_file(at).unwrap();
    }
    // The cut's own manifest, a trial's or a real cut's: the machine goes on to the recipe and guide
    // at the commit the manifest names (no guide there), whatever the working tree holds (a recipe
    // that would not read).
    std::fs::write(root.join("tools/entrega/maquina/maquina.toml"), "format = \"veredas-machine\"\nversion = 9\n").unwrap();
    for at in [&trial, &real] {
        std::fs::write(at, &text).unwrap();
        assert_eq!(refusal(maquina::from_release(root, release, target)), format!("commit {commit} holds no tools/entrega/maquina/guide.md"));
        std::fs::remove_file(at).unwrap();
    }
    assert!(!work.join("maquina").exists(), "no machine was started");
}

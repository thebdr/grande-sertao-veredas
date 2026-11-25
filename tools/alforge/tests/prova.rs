//! C-292, the parts of the proof that need no machine: the page found on a screen by its own canvas
//! colour, read from the release's page; the OVA read as VMware reads it, each fault named; the
//! proof's faults for a missing card line, a page not drawn and a changed disk. The proof of a real
//! machine is tests/maquina.rs (feature `maquina`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use alforge::machine::Recipe;
use alforge::maquina::{self, Proof};
use alforge::together::{Answer, Report};

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
    let head = alforge::git::text(&repo_root(), &["rev-parse", "HEAD"]).unwrap().unwrap().trim().to_string();
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
        bundle: "ab".repeat(32),
    }
}

#[test]
fn the_proof_names_a_missing_card_a_page_not_drawn_and_a_changed_disk() {
    let card = format!("VEREDAS-CARD ens4 {}", maquina::PROOF_MAC);
    let promised = "VEREDAS-BUNDLE abababababababababababababababababababababababababababababababab\nVEREDAS-PACKAGES ok\nVEREDAS-SERVICES-USER ok\nVEREDAS-SSH absent\nVEREDAS-CLOUD-INIT disabled\nVEREDAS-ON-PATH ok\nVEREDAS-ATUALIZAR ok\n[    7.123998] systemd[1]: Initializing machine ID from random generator.\n";
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
        (
            "VEREDAS-PACKAGES ok",
            "VEREDAS-PACKAGES missing open-vm-tools open-vm-tools-desktop",
            "the recipe's packages are not all installed (no `VEREDAS-PACKAGES ok` on the console)",
        ),
        (
            "VEREDAS-SERVICES-USER ok",
            "VEREDAS-SERVICES-USER other: veredas-bancada veredas-supervisao",
            "the bench and supervisão are not services of the recipe's user (no `VEREDAS-SERVICES-USER ok` on the console)",
        ),
        (
            "VEREDAS-BUNDLE abababababababababababababababababababababababababababababababab",
            "VEREDAS-BUNDLE cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd",
            "the machine does not hold this cut's bundle (no `VEREDAS-BUNDLE abababababababababababababababababababababababababababababababab` on the console)",
        ),
        ("vmw_pvscsi\n", "", "the guest did not name vmw_pvscsi"),
        ("vmxnet3\n", "", "the guest did not name vmxnet3"),
        ("VEREDAS-SSH absent", "VEREDAS-SSH present", "the machine holds an SSH server (no `VEREDAS-SSH absent` on the console)"),
        ("VEREDAS-CLOUD-INIT disabled", "VEREDAS-CLOUD-INIT enabled", "cloud-init is not disabled (no `VEREDAS-CLOUD-INIT disabled` on the console)"),
        ("VEREDAS-ON-PATH ok", "VEREDAS-ON-PATH missing veredas-portal", "the release's executables are not all on the PATH (no `VEREDAS-ON-PATH ok` on the console)"),
        ("VEREDAS-ATUALIZAR ok", "VEREDAS-ATUALIZAR missing", "the update command is not installed (no `VEREDAS-ATUALIZAR ok` on the console)"),
        (
            "systemd[1]: Initializing machine ID from random generator.",
            "systemd[1]: Detected first boot.",
            "the shipped disk carries a machine identity (no `Initializing machine ID from random generator` on the console)",
        ),
    ] {
        assert_eq!(maquina::proof_faults(&proof(&good.replace(said, instead), 0.3, "a"), "2.0.0"), vec![fault.to_string()], "{said}");
    }
    // The OVA's own faults are the proof's: `alforge prova` fails on an OVA VMware would refuse.
    let mut bad_ova = proof(&good, 0.3, "a");
    bad_ova.ova = vec!["the descriptor's processors are not one item of 2".to_string()];
    assert_eq!(maquina::proof_faults(&bad_ova, "2.0.0"), vec!["the descriptor's processors are not one item of 2".to_string()]);
}

#[test]
fn the_provisioning_is_judged_finished_and_trimmed_and_its_boot_check_said_before_any_login() {
    let ok = "ALFORGE-STEP cleanup\n/boot/efi: 114.8 MiB (120395776 bytes) trimmed on /dev/vda15\n/: 12.9 GiB (13857087488 bytes) trimmed on /dev/vda1\nALFORGE-PROVISION-OK\nALFORGE-PROVISION-EXIT 0\n";
    assert_eq!(maquina::provision_faults(ok), Vec::<String>::new());
    assert_eq!(maquina::provision_faults(&ok.replace("ALFORGE-PROVISION-EXIT 0", "ALFORGE-PROVISION-EXIT 1")), ["no ALFORGE-PROVISION-EXIT 0"]);
    assert_eq!(maquina::provision_faults(&ok.replace("ALFORGE-PROVISION-OK\n", "")), ["no ALFORGE-PROVISION-OK"]);
    assert_eq!(
        maquina::provision_faults(&ok.replace("/: 12.9 GiB (13857087488 bytes) trimmed on /dev/vda1\n", "")),
        ["the root's free space was not trimmed (no `/: ... trimmed on` line)"]
    );
    // The script the machine is provisioned by: the SSH server purged, the executables linked onto
    // the PATH, cloud-init off, the identity blanked, the space trimmed, and the boot's check said
    // before a login starts on the serial console (whose agetty hangs the line up).
    let r = Recipe::load_file(&repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    assert_eq!(r.machine.remove, ["openssh-server"]);
    let s = maquina::provision_script(&r, "veredas-9.9.9-x86_64-unknown-linux-gnu");
    for line in [
        "if [ -n \"openssh-server\" ]; then apt-get purge -y openssh-server; fi",
        // `current` a relative link, as the update command moves it (C-293).
        "ln -sfn veredas-9.9.9-x86_64-unknown-linux-gnu /opt/veredas/current",
        "for b in /opt/veredas/current/bin/*; do ln -sf \"$b\" /usr/local/bin/; done",
        "chmod 755 /usr/local/sbin/veredas-atualizar",
        // The boot's check names the release `current` names, which an update moves.
        "rel=$(sed -n 's/^release = \"\\(.*\\)\"$/\\1/p' release.toml)",
        "if sha256sum --quiet -c SHA256SUMS; then echo \"VEREDAS-SUMS-OK $rel\"; else echo \"VEREDAS-SUMS-FAILED $rel\"; fi",
        "touch /etc/cloud/cloud-init.disabled",
        "truncate -s 0 /etc/machine-id",
        "echo \"VEREDAS-BUNDLE $(sha256sum SHA256SUMS | cut -d' ' -f1)\"",
        "fstrim -av || true",
    ] {
        assert!(s.lines().any(|l| l == line), "{line}");
    }
    // The update command installed as it ships, byte for byte, and Python 3, which it runs on, a
    // package of the recipe's.
    let installed = &s[s.find("cat > /usr/local/sbin/veredas-atualizar <<'ATUALIZAR'\n").unwrap() + "cat > /usr/local/sbin/veredas-atualizar <<'ATUALIZAR'\n".len()..];
    assert_eq!(&installed[..installed.find("ATUALIZAR\n").unwrap()], maquina::ATUALIZAR);
    assert!(!maquina::ATUALIZAR.lines().any(|l| l == "ATUALIZAR"), "the heredoc's end never inside the script");
    // Python 3 installed by the provisioning itself, not by the recipe: a release's own recipe may
    // come from before the command (2.0.0-alpha.1's does).
    assert!(s.lines().any(|l| l == format!("apt-get install -y {} python3", r.machine.packages.join(" "))), "python3 installed");
    let first = std::process::Command::new("git").arg("-C").arg(repo_root()).args(["show", "d74be940d51a4510e353ccd91303fb3691080462:tools/entrega/maquina/maquina.toml"]).output().unwrap();
    assert!(first.status.success() && !String::from_utf8_lossy(&first.stdout).contains("python3"), "the first release's recipe holds no python3");
    // The bench and supervisão, each the service's own main process: an executable's exit ends its
    // service (and systemd starts it again), where a shell's pipeline waits on the sleep.
    for (exe, file) in [("veredas-maquinar", "bench.toml"), ("veredas-supervisao", "supervisao.toml")] {
        let line = format!("ExecStart=/bin/bash -c 'exec /opt/veredas/current/bin/{exe} {file} < <(exec sleep infinity)'");
        assert!(s.lines().any(|l| l == line), "{line}");
    }
    for (form, ends) in [("exec false < <(exec sleep 20)", true), ("sleep 20 | exec false", false)] {
        let start = std::time::Instant::now();
        let mut child = std::process::Command::new(if ends { "bash" } else { "sh" })
            .args(["-c", form])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let done = child.try_wait().unwrap();
        if done.is_none() {
            child.kill().unwrap();
            child.wait().unwrap();
        }
        assert_eq!(done.is_some(), ends, "{form}: ended within {:?}", start.elapsed());
    }
    // The guide the user reads says how to update, as the tool that installs the command writes it.
    assert!(s.contains("cat >> /home/veredas/README-veredas.md <<'GUIDE'\n\n## Updating to a newer release\n"), "the guide's update section");
    // The release's sums read by the boot's check from the release itself.
    let rel_line = s.lines().find(|l| l.starts_with("rel=")).unwrap();
    let probe = tempfile::tempdir().unwrap();
    std::fs::write(probe.path().join("release.toml"), "# A suite release\nformat = \"veredas-release\"\nversion = 1\nrelease = \"2.0.0-alpha.7\"\n\n[[component]]\nname = \"veredas-core\"\nversion = \"2.0.0\"\n").unwrap();
    let said = std::process::Command::new("sh").arg("-c").arg(format!("{rel_line}; echo \"$rel\"")).current_dir(probe.path()).output().unwrap();
    assert_eq!(String::from_utf8(said.stdout).unwrap(), "2.0.0-alpha.7\n");
    // The boot's check asks the package database for every package the recipe installs, the guest
    // tools among them (their working is VMware's to show; their presence is not).
    assert!(r.machine.packages.iter().any(|p| p == "open-vm-tools"), "{:?}", r.machine.packages);
    let asked = format!("for p in {}; do dpkg-query -W -f='${{Status}}' \"$p\" 2>/dev/null | grep -q 'install ok installed' || missing=\"$missing $p\"; done", r.machine.packages.join(" "));
    assert!(s.lines().any(|l| l == asked), "{asked}");
    // The bench and supervisão, services of the recipe's user, and the boot's check asking systemd so.
    for service in ["veredas-bancada", "veredas-supervisao"] {
        let unit = &s[s.find(&format!("cat > /etc/systemd/system/{service}.service")).unwrap()..];
        let unit = &unit[..unit.find("\nUNIT\n").unwrap()];
        assert!(unit.lines().any(|l| l == format!("User={}", r.machine.user)), "{unit}");
    }
    assert!(s.contains(&format!("[ \"$(systemctl show -p User --value \"$s.service\")\" = \"{}\" ]", r.machine.user)));
    let unit = &s[s.find("cat > /etc/systemd/system/veredas-verifica.service").unwrap()..];
    let unit = &unit[..unit.find("\nUNIT\n").unwrap()];
    assert!(unit.lines().any(|l| l == "Before=serial-getty@ttyS0.service getty@tty1.service"), "{unit}");
    assert!(unit.lines().any(|l| l == "StandardOutput=journal+console"), "{unit}");
    // A recipe that leaves out what it removes is refused: removal is stated, never assumed.
    let text = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    let without: String = text.lines().filter(|l| !l.starts_with("remove = ")).map(|l| format!("{l}\n")).collect();
    assert_eq!(text.lines().count(), without.lines().count() + 1);
    match Recipe::load(&without) {
        Err(e) => assert!(e.to_string().contains("missing field `remove`"), "{e}"),
        Ok(_) => panic!("a recipe without `remove` was read"),
    }
}

#[test]
fn the_base_image_is_refused_unless_it_matches_its_pinned_digest() {
    let r = Recipe::load_file(&repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let image = cache.path().join(r.base_name());
    std::fs::write(&image, "not the distribution's image\n").unwrap();
    let got = alforge::cache::sha512(&image).unwrap();
    match alforge::cache::base_image_in(cache.path(), &r) {
        Err(alforge::vm::VmError::Failed { program, status, stderr }) => {
            assert_eq!((program.as_str(), status.as_str()), ("the base image's check", "refused"));
            assert_eq!(stderr, format!("{} has SHA-512 {got}, the recipe pins {}", image.display(), r.base.sha512));
        }
        other => panic!("not refused: {other:?}"),
    }
}

#[test]
fn an_ova_is_read_as_vmware_reads_it_each_fault_named() {
    let r = Recipe::load_file(&repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
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
        alforge::vm::run("qemu-img", &["create", "-q", "-f", "vmdk", "-o", &format!("subformat={subformat}"), &disk.display().to_string(), &format!("{gb}G")], None).unwrap();
        let size = std::fs::metadata(&disk).unwrap().len();
        let ovf = edit(maquina::ovf(&r, name, &vmdk_name, size, capacity, "9.9.9"));
        std::fs::write(dir.join(format!("{name}.ovf")), &ovf).unwrap();
        let mf = format!(
            "SHA256({name}.ovf)= {}\nSHA256({vmdk_name})= {}\n",
            alforge::cache::sha256(&dir.join(format!("{name}.ovf"))).unwrap(),
            alforge::cache::sha256(&disk).unwrap()
        );
        std::fs::write(dir.join(format!("{name}.mf")), &mf).unwrap();
        let ova = work.path().join(format!("{label}.ova"));
        let names: Vec<String> = order.iter().map(|n| format!("{name}{n}")).collect();
        let (o, d) = (ova.display().to_string(), dir.display().to_string());
        let mut args = vec!["--format=ustar", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s", "-cf", &o, "-C", &d];
        args.extend(names.iter().map(String::as_str));
        alforge::vm::run("tar", &args, None).unwrap();
        let opened = work.path().join(format!("{label}-opened"));
        std::fs::create_dir_all(&opened).unwrap();
        alforge::vm::run("tar", &["-xf", &o, "-C", &opened.display().to_string()], None).unwrap();
        (ova, opened)
    };
    let same = |s: String| s;
    let right = [".ovf", ".mf", "-disk1.vmdk"];
    let faults = |label: &str, subformat: &str, gb: u32, edit: &dyn Fn(String) -> String, order: [&str; 3]| {
        let (ova, opened) = make(label, subformat, gb, edit, order);
        maquina::ova_faults(&r, &ova, "9.9.9", &opened).unwrap()
    };
    assert_eq!(faults("good", "streamOptimized", r.machine.disk_gb, &same, right), Vec::<String>::new());
    // The members as packed: owned by another, read as 0600, their sums off; each named.
    let (_, members) = make("members", "streamOptimized", r.machine.disk_gb, &same, right);
    let names: Vec<String> = right.iter().map(|n| format!("{name}{n}")).collect();
    let repack = |label: &str, flags: &[&str]| -> PathBuf {
        let ova = work.path().join(format!("{label}.ova"));
        let (o, d) = (ova.display().to_string(), members.display().to_string());
        let mut args: Vec<&str> = flags.to_vec();
        args.extend(["-cf", o.as_str(), "-C", d.as_str()]);
        args.extend(names.iter().map(String::as_str));
        alforge::vm::run("tar", &args, None).unwrap();
        ova
    };
    for (label, flags, listed) in [
        ("owner", ["--format=ustar", "--owner=1000", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s"], "-rw-r--r-- 1000/0 "),
        ("mode", ["--format=ustar", "--owner=0", "--group=0", "--numeric-owner", "--mode=u=rw,go="], "-rw------- 0/0 "),
    ] {
        let f = maquina::ova_faults(&r, &repack(label, &flags), "9.9.9", &members).unwrap();
        assert_eq!(f.len(), 3, "{f:?}");
        assert!(f.iter().all(|x| x.starts_with(&format!("an OVA member is not a 0644 file owned by root: {listed}"))), "{f:?}");
    }
    std::fs::write(members.join(format!("{name}.mf")), format!("SHA256({name}.ovf)= {}\nSHA256({vmdk_name})= {}\n", "0".repeat(64), "0".repeat(64))).unwrap();
    let f = maquina::ova_faults(&r, &repack("sums", &["--format=ustar", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s"]), "9.9.9", &members).unwrap();
    assert!(f.len() == 1 && f[0].starts_with("the manifest's sums are not the members': "), "{f:?}");
    // The descriptor's file reference, the disk's capacity and the firmware, each read in its place.
    let longer = |s: String| s.replacen(&format!(r#"ovf:href="{vmdk_name}" ovf:size=""#), &format!(r#"ovf:href="{vmdk_name}" ovf:size="7"#), 1);
    let (ova, opened) = make("size", "streamOptimized", r.machine.disk_gb, &longer, right);
    let size = std::fs::metadata(opened.join(&vmdk_name)).unwrap().len();
    assert_eq!(
        maquina::ova_faults(&r, &ova, "9.9.9", &opened).unwrap(),
        [format!("the descriptor's References name no one file `{vmdk_name}` of {size} bytes as file1")]
    );
    let bigger = |s: String| s.replacen(&format!(r#"ovf:capacity="{capacity}""#), &format!(r#"ovf:capacity="1{capacity}""#), 1);
    assert_eq!(
        faults("capacity", "streamOptimized", r.machine.disk_gb, &bigger, right),
        [format!("the descriptor's DiskSection names no one stream-optimized disk vmdisk1 of {capacity} bytes on file1")]
    );
    let efi = |s: String| s.replacen(r#"vmw:key="firmware" vmw:value="bios""#, r#"vmw:key="firmware" vmw:value="efi""#, 1);
    assert_eq!(faults("efi", "streamOptimized", r.machine.disk_gb, &efi, right), ["the descriptor's firmware is not BIOS"]);
    // Each value VMware reads, offered wrong in its own place: another card, another controller,
    // another hardware version, another disk format, another file, memory in other units.
    let hw = format!("vmx-{:02}", r.machine.hardware_version);
    let hw_type = format!("<vssd:VirtualSystemType>{hw}</vssd:VirtualSystemType>");
    for (label, from, to, want) in [
        ("e1000", "<rasd:ResourceSubType>VmxNet3</rasd:ResourceSubType>", "<rasd:ResourceSubType>E1000</rasd:ResourceSubType>", vec!["the descriptor holds no one VMXNET3 card".to_string()]),
        (
            "lsilogic",
            "<rasd:ResourceSubType>VirtualSCSI</rasd:ResourceSubType>",
            "<rasd:ResourceSubType>lsilogic</rasd:ResourceSubType>",
            vec![
                "the descriptor holds no one paravirtual SCSI controller (VirtualSCSI)".to_string(),
                "the descriptor's hard disk is not one item on vmdisk1 under the paravirtual SCSI controller".to_string(),
            ],
        ),
        ("vmx13", hw_type.as_str(), "<vssd:VirtualSystemType>vmx-13</vssd:VirtualSystemType>", vec![format!("the descriptor's hardware version is not {hw}")]),
        (
            "sparse",
            "vmdk.html#streamOptimized",
            "vmdk.html#sparse",
            vec![format!("the descriptor's DiskSection names no one stream-optimized disk vmdisk1 of {capacity} bytes on file1")],
        ),
        ("units", "<rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits>", "<rasd:AllocationUnits>byte * 2^10</rasd:AllocationUnits>", vec![format!("the descriptor's memory is not one item of {} MB", r.machine.memory_mb)]),
    ] {
        let edit = |s: String| {
            assert_eq!(s.matches(from).count(), 1, "{from}");
            s.replacen(from, to, 1)
        };
        assert_eq!(faults(label, "streamOptimized", r.machine.disk_gb, &edit, right), want, "{label}");
    }
    let other_file = |s: String| s.replacen(&format!(r#"ovf:href="{vmdk_name}""#), r#"ovf:href="another-disk1.vmdk""#, 1);
    let (ova, opened) = make("href", "streamOptimized", r.machine.disk_gb, &other_file, right);
    let size = std::fs::metadata(opened.join(&vmdk_name)).unwrap().len();
    assert_eq!(
        maquina::ova_faults(&r, &ova, "9.9.9", &opened).unwrap(),
        [format!("the descriptor's References name no one file `{vmdk_name}` of {size} bytes as file1")]
    );
    // A pax archive (`--format=posix`) of the members, their sums right again: its listing reads the
    // same and its headers' magic is ustar's, but a reader that knows only ustar meets an extension
    // header first.
    std::fs::write(members.join(format!("{name}.mf")), format!(
        "SHA256({name}.ovf)= {}\nSHA256({vmdk_name})= {}\n",
        alforge::cache::sha256(&members.join(format!("{name}.ovf"))).unwrap(),
        alforge::cache::sha256(&members.join(&vmdk_name)).unwrap()
    )).unwrap();
    let pax = repack("pax", &["--format=posix", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s"]);
    let f = maquina::ova_faults(&r, &pax, "9.9.9", &members).unwrap();
    assert!(f.len() == 1 && f[0].starts_with("the OVA's raw headers are [\"./PaxHeaders/"), "{f:?}");
    // Another tar format than ustar (GNU's): its listing reads the same, its headers do not.
    let (_, opened) = make("gnu", "streamOptimized", r.machine.disk_gb, &same, right);
    let gnu = work.path().join("gnu-format.ova");
    let names: Vec<String> = right.iter().map(|n| format!("{name}{n}")).collect();
    let (o, d) = (gnu.display().to_string(), opened.display().to_string());
    let mut args = vec!["--format=gnu", "--owner=0", "--group=0", "--numeric-owner", "--mode=a+r,u+w,go-w,a-x,ug-s", "-cf", &o, "-C", &d];
    args.extend(names.iter().map(String::as_str));
    alforge::vm::run("tar", &args, None).unwrap();
    assert_eq!(maquina::ova_faults(&r, &gnu, "9.9.9", &opened).unwrap(), [r#"the OVA is no POSIX ustar archive: its first header's magic is "ustar  \0""#]);
    let f = faults("disordered", "streamOptimized", r.machine.disk_gb, &same, ["-disk1.vmdk", ".ovf", ".mf"]);
    assert!(f.len() == 2 && f[0].starts_with("the OVA's members are") && f[1].starts_with("the OVA's raw headers are"), "{f:?}");
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
        let out = Command::new("git").arg("-C").arg(root).args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), "/target/\n").unwrap();
    std::fs::create_dir_all(root.join("tools/alforge/maquina")).unwrap();
    std::fs::copy(repo_root().join("tools/alforge/maquina/maquina.toml"), root.join("tools/alforge/maquina/maquina.toml")).unwrap();
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the recipe, no guide"]);
    let commit = git(&["rev-parse", "HEAD"]);
    let release = "0.0.0-trial.9";
    let target = "x86_64-unknown-linux-gnu";
    let work = root.join("target/alforge").join(release);
    let name = format!("veredas-{release}-{target}");
    let text = alforge::cut::manifest_text(
        &semver::Version::parse(release).unwrap(),
        &commit,
        target,
        "rustc 1.100.0 (0ef1bc386 2026-11-09)",
        &[("veredas-core".to_string(), "2.0.0".to_string(), vec![])],
        alforge::cut::NOT_RUN,
    );
    std::fs::create_dir_all(work.join(&name)).unwrap();
    std::fs::write(work.join(&name).join("release.toml"), &text).unwrap();
    alforge::bundle::pack(&work, &name, "1", &work.join(format!("{name}.tar.gz"))).unwrap();
    std::fs::remove_dir_all(work.join(&name)).unwrap();
    let refusal = |r: Result<maquina::Paths, alforge::vm::VmError>| match r {
        Err(alforge::vm::VmError::Failed { program, status, stderr }) => {
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
        std::fs::write(at, text.replace(alforge::cut::NOT_RUN, "cargo nextest run --workspace")).unwrap();
        assert_eq!(refusal(maquina::from_release(root, release, target)), format!("{} is not the bundle's own release.toml: the bundle is not that cut's", at.display()));
        std::fs::remove_file(at).unwrap();
    }
    // The cut's own manifest, a trial's or a real cut's: the machine goes on to the recipe and guide
    // at the commit the manifest names (no guide there), whatever the working tree holds (a recipe
    // that would not read).
    std::fs::write(root.join("tools/alforge/maquina/maquina.toml"), "format = \"veredas-machine\"\nversion = 9\n").unwrap();
    for at in [&trial, &real] {
        std::fs::write(at, &text).unwrap();
        assert_eq!(refusal(maquina::from_release(root, release, target)), format!("commit {commit} holds no tools/alforge/maquina/guide.md"));
        std::fs::remove_file(at).unwrap();
    }
    // A real cut of the release after a trial of it: the trial's manifest left beside the bundle is
    // superseded, the real cut's decides, whichever the bundle is.
    std::fs::write(&real, &text).unwrap();
    std::fs::write(&trial, text.replace(alforge::cut::NOT_RUN, "an older trial")).unwrap();
    assert_eq!(refusal(maquina::from_release(root, release, target)), format!("commit {commit} holds no tools/alforge/maquina/guide.md"));
    std::fs::write(&real, text.replace(alforge::cut::NOT_RUN, "cargo nextest run --workspace")).unwrap();
    std::fs::write(&trial, &text).unwrap();
    assert_eq!(refusal(maquina::from_release(root, release, target)), format!("{} is not the bundle's own release.toml: the bundle is not that cut's", real.display()));
    assert!(!work.join("maquina").exists(), "no machine was started");
}

#[test]
fn a_release_is_proven_by_its_own_recipe_and_page_at_its_commit() {
    use std::process::Command;
    // A throwaway repository: the release's commit holds a recipe of 3072 MB, in the member's folder
    // under its former name (`tools/entrega`, as release 2.0.0-alpha.1's does), and a page canvas of
    // #102030; a later commit, HEAD, renames the folder `tools/alforge` and holds 2048 MB and
    // #405060; the working tree holds 1024 MB and #708090. The proof's setup reads the release's own.
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    let git = |args: &[&str]| {
        let out = Command::new("git").arg("-C").arg(root).args(["-c", "user.name=alforge", "-c", "user.email=alforge@example.invalid", "-c", "commit.gpgsign=false"]).args(args).output().unwrap();
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let recipe = std::fs::read_to_string(repo_root().join("tools/alforge/maquina/maquina.toml")).unwrap();
    let machine_memory = |mb: u32| {
        let at = recipe.find("[machine]").unwrap();
        let (head, tail) = recipe.split_at(at);
        let line = tail.lines().find(|l| l.starts_with("memory_mb = ")).unwrap();
        format!("{head}{}", tail.replacen(line, &format!("memory_mb = {mb}"), 1))
    };
    let state = |folder: &str, mb: u32, canvas: &str| {
        std::fs::create_dir_all(root.join(folder).join("maquina")).unwrap();
        std::fs::write(root.join(folder).join("maquina/maquina.toml"), machine_memory(mb)).unwrap();
        std::fs::create_dir_all(root.join("crates/veredas-supervisao/web/dist/assets")).unwrap();
        std::fs::write(root.join("crates/veredas-supervisao/web/dist/assets/app.css"), format!(":root{{--canvas: #{canvas};--ink:#eeeeee}}\n")).unwrap();
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join(".gitignore"), "/target/\n").unwrap();
    state("tools/entrega", 3072, "102030");
    git(&["add", "-A"]);
    git(&["commit", "-q", "-m", "the release's commit"]);
    let commit = git(&["rev-parse", "HEAD"]);
    git(&["mv", "tools/entrega", "tools/alforge"]);
    state("tools/alforge", 2048, "405060");
    git(&["commit", "-q", "-am", "a later commit: the member renamed"]);
    state("tools/alforge", 1024, "708090");
    let release = "0.0.0-trial.7";
    let target = "x86_64-unknown-linux-gnu";
    let work = root.join("target/alforge").join(release);
    let name = format!("veredas-{release}-{target}");
    std::fs::create_dir_all(work.join(&name)).unwrap();
    let text = alforge::cut::manifest_text(
        &semver::Version::parse(release).unwrap(),
        &commit,
        target,
        "rustc 1.100.0 (0ef1bc386 2026-11-09)",
        &[("veredas-core".to_string(), "2.0.0".to_string(), vec![])],
        alforge::cut::NOT_RUN,
    );
    std::fs::write(work.join(&name).join("release.toml"), &text).unwrap();
    alforge::bundle::write_sums(&work.join(&name)).unwrap();
    let sums = alforge::cache::sha256(&work.join(&name).join("SHA256SUMS")).unwrap();
    alforge::bundle::pack(&work, &name, "1", &work.join(format!("{name}.tar.gz"))).unwrap();
    // No OVA yet: refused, naming where `alforge maquina` builds it.
    let ova = work.join("maquina").join(format!("veredas-{release}.ova"));
    match maquina::proof_setup(root, release, target) {
        Err(alforge::vm::VmError::Failed { stderr, .. }) => assert_eq!(stderr, format!("{} is not there: build the machine first", ova.display())),
        other => panic!("not refused: {:?}", other.map(|s| (s.recipe.machine.memory_mb, s.ova, s.canvas))),
    }
    std::fs::create_dir_all(ova.parent().unwrap()).unwrap();
    std::fs::write(&ova, "").unwrap();
    // A machine of a cut that did not run to its end (no manifest where a cut writes it): refused.
    match maquina::proof_setup(root, release, target) {
        Err(alforge::vm::VmError::Failed { stderr, .. }) => assert!(stderr.starts_with(&format!("release {release} was not cut to its end")), "{stderr}"),
        other => panic!("not refused: {:?}", other.map(|s| s.bundle)),
    }
    std::fs::write(work.join(format!("{release}.toml")), &text).unwrap();
    let s = maquina::proof_setup(root, release, target).unwrap();
    assert_eq!((s.recipe.machine.memory_mb, s.ova, s.canvas, s.bundle), (3072, ova, (0x10, 0x20, 0x30), sums));
}

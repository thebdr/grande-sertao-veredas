//! C-292: the tester machine, for real (feature `maquina`; docs/questions.md Q440). A trial cut of
//! HEAD, its bundle provisioned into the distribution's cloud image under QEMU and packed into an
//! OVA by the recipe; then the OVA read as VMware reads it (its members in order, the descriptor,
//! the manifest of sums), and the disk taken out of it booted copy-on-write on QEMU's models of
//! VMware's devices with a card address the provisioning never saw: the release's files checked
//! at boot, the plant live from the first boot with nobody logged in, reached from the host, its
//! write refused; the desktop drawn; the disk's bytes unchanged after. What only VMware proves
//! (the import, VMware's own devices and display, the guest tools) is C-292's stated remainder.

use std::path::{Path, PathBuf};

use entrega::cut::{self, Options};
use entrega::machine::Recipe;
use entrega::{cache, maquina, together, vm};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn the_machine_is_built_by_the_recipe_and_its_shipped_disk_boots_the_plant_on_vmwares_devices() {
    let root = repo_root();
    let release = "0.0.0-trial.292";
    let work = root.join("target/entrega").join(release);
    // Where `entrega cut --trial` writes a trial's manifest: the machine is built only from a cut
    // that ran to its end.
    let cut = cut::cut(&root, &Options { release: semver::Version::parse(release).unwrap(), out: work.clone(), work: work.clone(), run_suite: false })
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
    // Owned by root by number (`0/0`), every member a plain file read as 0644 whatever the umask.
    assert!(verbose.lines().all(|l| l.starts_with("-rw-r--r-- 0/0 ")), "{verbose}");
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
        format!("<vssd:VirtualSystemType>vmx-{:02}</vssd:VirtualSystemType>", recipe.machine.hardware_version),
        "<rasd:ResourceSubType>VirtualSCSI</rasd:ResourceSubType>".to_string(),
        "<rasd:ResourceSubType>VmxNet3</rasd:ResourceSubType>".to_string(),
        format!("<rasd:VirtualQuantity>{}</rasd:VirtualQuantity>", recipe.machine.memory_mb),
        r#"vmw:key="firmware" vmw:value="bios""#.to_string(),
    ] {
        assert!(ovf.contains(&needle), "the descriptor holds no {needle}:\n{ovf}");
    }
    // Well-formed and read as VMware reads it: the proof's own `ova_faults`, asserted below (proof.ova).
    let mf = std::fs::read_to_string(opened.as_path().join(format!("{name}.mf"))).unwrap();
    assert_eq!(
        mf,
        format!(
            "SHA256({name}.ovf)= {}\nSHA256({name}-disk1.vmdk)= {}\n",
            cache::sha256(&opened.as_path().join(format!("{name}.ovf"))).unwrap(),
            cache::sha256(&vmdk).unwrap()
        )
    );

    // The shipped disk booted on VMware's devices, a card address the provisioning never saw: the
    // release's sums checked at boot, the plant read from the host, the desktop drawn.
    // As `entrega prova` sets it up: the cut's bundle's digest, the recipe and canvas at its commit.
    let setup = maquina::proof_setup(&root, release, target).unwrap();
    assert_eq!(setup.bundle, cache::sha256(&cut.extracted.join("SHA256SUMS")).unwrap());
    let proof = maquina::prove(&setup.recipe, &setup.ova, release, setup.canvas, &setup.bundle, &work.join("prova")).unwrap();
    assert_eq!(maquina::proof_faults(&proof, release), Vec::<String>::new());
    assert!(proof.console.contains(&format!("VEREDAS-SUMS-OK {release}")) && !proof.console.contains("VEREDAS-SUMS-FAILED"));
    assert!(proof.console.contains("vmw_pvscsi") && proof.console.contains("vmxnet3"), "the guest saw VMware's devices");
    assert!(proof.console.contains(&format!(" {}", maquina::PROOF_MAC)) && proof.console.contains("VEREDAS-CARD "), "the card the guest saw is the proof's own address");
    let r = &proof.plant;
    assert_eq!(r.page.status, 200);
    assert_eq!(r.write.status, 403);
    assert_eq!(r.write.body, r#"{"error":"writes are turned off for this project"}"#);
    for id in ["parcel-1", "parcel-2"] {
        let a = together::object(&r.first, id).unwrap()["place"].as_f64().unwrap();
        let b = together::object(&r.second, id).unwrap()["place"].as_f64().unwrap();
        assert!(a != b && (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b), "{id}: {a} then {b}");
    }
    assert_eq!(r.second["session"]["health"]["sorter"]["state"], "live");
    assert!(proof.ova.is_empty(), "{:?}", proof.ova);
    assert!(proof.page_share >= maquina::PAGE_SHARE, "the page covers {:.2} of the screen", proof.page_share);
    assert!(proof.colours > 200, "the screen shows {} colours", proof.colours);
    assert_eq!(proof.after, proof.before, "the proof boot changed the shipped disk");
    assert_eq!(proof.vmdk, work.join("prova/opened").join(format!("{name}-disk1.vmdk")));
    assert_eq!(cache::sha256(&proof.vmdk).unwrap(), cache::sha256(&vmdk).unwrap(), "the disk proven is the OVA's");
    let _ = std::fs::remove_dir_all(&work);
}

//! The tester machine (docs/entrega-plan.md, part two): a release's bundle provisioned unattended
//! into the distribution's cloud image under QEMU, the disk flattened into a stream-optimized VMDK,
//! an OVF descriptor for VMware's virtual hardware and a manifest of sums, tarred into an OVA. The
//! proof boots the disk taken out of the OVA on QEMU's models of VMware's devices.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cache;
use crate::machine::Recipe;
use crate::vm::{self, DataDisk, Hardware, Spec, Vm, VmError};

/// Bumped when what the provisioning does changes.
pub const PROVISIONING: u32 = 1;

/// The provisioning script run as root in the guest, its network up: the packages, the user, the
/// release under `/opt/veredas/`, the plant in the user's home, the services, the desktop's
/// automatic login and browser, the network handed to NetworkManager, cloud-init and the SSH
/// server gone, the disk's free space trimmed.
pub fn provision_script(r: &Recipe, bundle_dir: &str, release: &str) -> String {
    let m = &r.machine;
    let u = &m.user;
    let packages = m.packages.join(" ");
    let remove = m.remove.join(" ");
    let port = m.supervisao_port;
    format!(
        r#"set -eu
export DEBIAN_FRONTEND=noninteractive
echo "ENTREGA-STEP packages"
apt-get update
apt-get install -y {packages}
if [ -n "{remove}" ]; then apt-get purge -y {remove}; fi
apt-get -y autoremove --purge
echo "ENTREGA-STEP user"
id debian >/dev/null 2>&1 && userdel -r debian || true
useradd -m -s /bin/bash -G sudo {u}
for g in netdev systemd-journal; do if getent group $g >/dev/null; then usermod -aG $g {u}; fi; done
echo '{u}:{password}' | chpasswd
echo veredas > /etc/hostname
sed -i 's/^127.0.1.1.*/127.0.1.1 veredas/' /etc/hosts
grep -q '^127.0.1.1' /etc/hosts || echo '127.0.1.1 veredas' >> /etc/hosts
echo "ENTREGA-STEP release"
mkdir -p /opt/veredas
tar -xzf /entrega/bundle.tar.gz -C /opt/veredas
ln -sfn /opt/veredas/{bundle_dir} /opt/veredas/current
for b in /opt/veredas/current/bin/*; do ln -sf "$b" /usr/local/bin/; done
cp -r /opt/veredas/current/planta /home/{u}/planta
cp /entrega/guide.md /home/{u}/README-veredas.md
echo "ENTREGA-STEP services"
cat > /etc/systemd/system/veredas-bancada.service <<'UNIT'
[Unit]
Description=Veredas: the bench playing the plant (veredas-maquinar)
After=network.target

[Service]
User={u}
WorkingDirectory=/home/{u}/planta
# The executable runs until its stdin closes: a pipe from sleep holds it open.
ExecStart=/bin/sh -c 'sleep infinity | exec /opt/veredas/current/bin/veredas-maquinar bench.toml'
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT
cat > /etc/systemd/system/veredas-supervisao.service <<'UNIT'
[Unit]
Description=Veredas: supervisão serving the plant on port {port} (veredas-supervisao)
After=veredas-bancada.service
Wants=veredas-bancada.service

[Service]
User={u}
WorkingDirectory=/home/{u}/planta
ExecStart=/bin/sh -c 'sleep infinity | exec /opt/veredas/current/bin/veredas-supervisao supervisao.toml'
Restart=on-failure
RestartSec=3

[Install]
WantedBy=multi-user.target
UNIT
mkdir -p /usr/local/libexec
cat > /usr/local/libexec/veredas-verifica <<'SCRIPT'
#!/bin/sh
# The release's files checked against its SHA256SUMS, and the network cards the machine sees.
cd /opt/veredas/current
if sha256sum --quiet -c SHA256SUMS; then echo "VEREDAS-SUMS-OK {release}"; else echo "VEREDAS-SUMS-FAILED {release}"; fi
for c in /sys/class/net/*; do n=$(basename "$c"); [ "$n" = lo ] || echo "VEREDAS-CARD $n $(cat "$c/address")"; done
SCRIPT
chmod 755 /usr/local/libexec/veredas-verifica
cat > /etc/systemd/system/veredas-verifica.service <<'UNIT'
[Unit]
Description=Veredas: the release's files checked against its SHA256SUMS

[Service]
Type=oneshot
ExecStart=/usr/local/libexec/veredas-verifica
StandardOutput=journal+console

[Install]
WantedBy=multi-user.target
UNIT
systemctl enable veredas-bancada.service veredas-supervisao.service veredas-verifica.service
echo "ENTREGA-STEP desktop"
mkdir -p /etc/lightdm/lightdm.conf.d
printf '[Seat:*]\nautologin-user={u}\nautologin-session=xfce\n' > /etc/lightdm/lightdm.conf.d/50-veredas.conf
systemctl set-default graphical.target
H=/home/{u}
mkdir -p $H/.config/autostart $H/.config/xfce4/xfconf/xfce-perchannel-xml $H/Desktop
cp /etc/xdg/xfce4/panel/default.xml $H/.config/xfce4/xfconf/xfce-perchannel-xml/xfce4-panel.xml
cat > $H/.config/autostart/veredas-supervisao.desktop <<'DESK'
[Desktop Entry]
Type=Application
Name=Supervisão
Exec=chromium --no-first-run --no-default-browser-check --password-store=basic --start-maximized http://127.0.0.1:{port}/s/loop/
DESK
ln -sfn /opt/veredas/current/docs $H/Desktop/docs
ln -sfn $H/planta $H/Desktop/planta
ln -sfn $H/README-veredas.md $H/Desktop/README-veredas.md
chown -R {u}:{u} $H
echo "ENTREGA-STEP network"
rm -f /etc/netplan/*.yaml
systemctl disable systemd-networkd.service systemd-networkd.socket systemd-networkd-wait-online.service || true
systemctl enable NetworkManager.service
echo "ENTREGA-STEP cleanup"
apt-get clean
rm -rf /var/lib/apt/lists/*
touch /etc/cloud/cloud-init.disabled
rm -f /var/log/cloud-init.log /var/log/cloud-init-output.log
truncate -s 0 /etc/machine-id
rm -f /var/lib/dbus/machine-id
rm -rf /entrega
journalctl --rotate || true
journalctl --vacuum-time=1s || true
sync
fstrim -av || true
echo ENTREGA-PROVISION-OK
"#,
        password = m.password,
    )
}

/// Where a machine's files are made: the OVA alone is kept (its disk, descriptor and manifest are
/// inside it; the provisioning's disk and the loose copies are removed, a disk being 1 GB and more).
pub struct Paths {
    pub ova: PathBuf,
}

/// The OVF descriptor: one virtual system on VMware's hardware at the recipe's version, its disk
/// the stream-optimized VMDK named `vmdk` (`size` bytes, `capacity` bytes seen by the guest).
pub fn ovf(r: &Recipe, name: &str, vmdk: &str, size: u64, capacity: u64, release: &str) -> String {
    let m = &r.machine;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Envelope xmlns="http://schemas.dmtf.org/ovf/envelope/1" xmlns:ovf="http://schemas.dmtf.org/ovf/envelope/1" xmlns:rasd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData" xmlns:vssd="http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_VirtualSystemSettingData" xmlns:vmw="http://www.vmware.com/schema/ovf" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance">
  <References>
    <File ovf:id="file1" ovf:href="{vmdk}" ovf:size="{size}"/>
  </References>
  <DiskSection>
    <Info>Virtual disk information</Info>
    <Disk ovf:diskId="vmdisk1" ovf:fileRef="file1" ovf:capacity="{capacity}" ovf:capacityAllocationUnits="byte" ovf:format="http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized"/>
  </DiskSection>
  <NetworkSection>
    <Info>The list of logical networks</Info>
    <Network ovf:name="nat">
      <Description>The network the machine's card is on (VMware's NAT by default)</Description>
    </Network>
  </NetworkSection>
  <VirtualSystem ovf:id="{name}">
    <Info>The Veredas tester machine, suite release {release}</Info>
    <Name>{name}</Name>
    <OperatingSystemSection ovf:id="101" vmw:osType="other4xLinux64Guest">
      <Info>The operating system: Debian 13, 64-bit</Info>
      <Description>Debian GNU/Linux 13 (64-bit)</Description>
    </OperatingSystemSection>
    <VirtualHardwareSection>
      <Info>Virtual hardware requirements</Info>
      <System>
        <vssd:ElementName>Virtual Hardware Family</vssd:ElementName>
        <vssd:InstanceID>0</vssd:InstanceID>
        <vssd:VirtualSystemIdentifier>{name}</vssd:VirtualSystemIdentifier>
        <vssd:VirtualSystemType>vmx-{hw:02}</vssd:VirtualSystemType>
      </System>
      <Item>
        <rasd:AllocationUnits>hertz * 10^6</rasd:AllocationUnits>
        <rasd:Description>Number of virtual processors</rasd:Description>
        <rasd:ElementName>{cpus} virtual processor(s)</rasd:ElementName>
        <rasd:InstanceID>1</rasd:InstanceID>
        <rasd:ResourceType>3</rasd:ResourceType>
        <rasd:VirtualQuantity>{cpus}</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:AllocationUnits>byte * 2^20</rasd:AllocationUnits>
        <rasd:Description>Memory size</rasd:Description>
        <rasd:ElementName>{mem} MB of memory</rasd:ElementName>
        <rasd:InstanceID>2</rasd:InstanceID>
        <rasd:ResourceType>4</rasd:ResourceType>
        <rasd:VirtualQuantity>{mem}</rasd:VirtualQuantity>
      </Item>
      <Item>
        <rasd:Address>0</rasd:Address>
        <rasd:Description>SCSI Controller</rasd:Description>
        <rasd:ElementName>SCSI Controller 0</rasd:ElementName>
        <rasd:InstanceID>3</rasd:InstanceID>
        <rasd:ResourceSubType>VirtualSCSI</rasd:ResourceSubType>
        <rasd:ResourceType>6</rasd:ResourceType>
      </Item>
      <Item>
        <rasd:AddressOnParent>0</rasd:AddressOnParent>
        <rasd:ElementName>Hard Disk 1</rasd:ElementName>
        <rasd:HostResource>ovf:/disk/vmdisk1</rasd:HostResource>
        <rasd:InstanceID>4</rasd:InstanceID>
        <rasd:Parent>3</rasd:Parent>
        <rasd:ResourceType>17</rasd:ResourceType>
      </Item>
      <Item>
        <rasd:AddressOnParent>7</rasd:AddressOnParent>
        <rasd:AutomaticAllocation>true</rasd:AutomaticAllocation>
        <rasd:Connection>nat</rasd:Connection>
        <rasd:Description>VMXNET3 ethernet adapter on "nat"</rasd:Description>
        <rasd:ElementName>Network Adapter 1</rasd:ElementName>
        <rasd:InstanceID>5</rasd:InstanceID>
        <rasd:ResourceSubType>VmxNet3</rasd:ResourceSubType>
        <rasd:ResourceType>10</rasd:ResourceType>
      </Item>
      <Item ovf:required="false">
        <rasd:AutomaticAllocation>false</rasd:AutomaticAllocation>
        <rasd:ElementName>Video card</rasd:ElementName>
        <rasd:InstanceID>6</rasd:InstanceID>
        <rasd:ResourceType>24</rasd:ResourceType>
        <vmw:Config ovf:required="false" vmw:key="videoRamSizeInKB" vmw:value="16384"/>
      </Item>
      <vmw:Config ovf:required="false" vmw:key="firmware" vmw:value="bios"/>
      <vmw:Config ovf:required="false" vmw:key="tools.syncTimeWithHost" vmw:value="true"/>
      <vmw:Config ovf:required="false" vmw:key="powerOpInfo.powerOffType" vmw:value="soft"/>
    </VirtualHardwareSection>
  </VirtualSystem>
</Envelope>
"#,
        hw = m.hardware_version,
        cpus = m.cpus,
        mem = m.memory_mb,
    )
}

/// Builds the tester machine from a bundle (`<bundle_dir>.tar.gz`) into `work`, its OVA named
/// `veredas-<release>.ova`.
pub fn build(r: &Recipe, bundle: &Path, bundle_dir: &str, release: &str, guide: &str, work: &Path) -> Result<Paths, VmError> {
    let base = cache::base_image(r)?;
    std::fs::create_dir_all(work)?;
    let input = work.join("maquina-in");
    let _ = std::fs::remove_dir_all(&input);
    std::fs::create_dir_all(&input)?;
    std::fs::copy(bundle, input.join("bundle.tar.gz"))?;
    std::fs::write(input.join("guide.md"), guide)?;
    std::fs::write(input.join("provision.sh"), provision_script(r, bundle_dir, release))?;
    let in_tar = work.join("maquina-in.tar");
    vm::run("tar", &["-cf", &in_tar.display().to_string(), "-C", &input.display().to_string(), "."], None)?;
    let disk = work.join("maquina.qcow2");
    vm::overlay(&base, &disk, r.machine.disk_gb)?;
    // The provisioning logs to a file whose tail is echoed to the console after (agetty hangs up
    // ttyS0 when it starts; docs/entrega-plan.md, "Traps"); the log stays in the image.
    let user_data = "#cloud-config\nusers: []\nruncmd:\n  - [ sh, -c, 'mkdir -p /entrega && tar -xf /dev/disk/by-id/virtio-entrega-in -C /entrega && sh /entrega/provision.sh > /var/log/veredas-provision.log 2>&1; rc=$?; tail -n 60 /var/log/veredas-provision.log > /dev/ttyS0; echo \"ENTREGA-PROVISION-EXIT $rc\" > /dev/ttyS0' ]\npower_state:\n  mode: poweroff\n  delay: now\n  timeout: 120\n";
    let nonce = cache::sha256_text(&format!("{release}{:?}", std::time::SystemTime::now()));
    let seed = work.join("maquina-seed.iso");
    vm::seed(&work.join("maquina-seed"), &seed, user_data, &format!("entrega-maquina-{}", &nonce[..16]))?;
    let console = work.join("maquina-console.log");
    eprintln!("entrega: provisioning the tester machine; its console is {}", console.display());
    let mut m = Vm::start(&Spec {
        disk: disk.clone(),
        hardware: Hardware::Virtio,
        cpus: r.machine.cpus.max(4),
        memory_mb: r.machine.memory_mb,
        seed: Some(seed),
        data: vec![DataDisk { file: in_tar.clone(), serial: "entrega-in".into(), writable: false }],
        forwards: vec![],
        snapshot: false,
        mac: None,
        qmp: None,
        console: console.clone(),
    })?;
    m.wait_off(Duration::from_secs(90 * 60))?;
    let text = vm::read_console(&console);
    if !text.contains("ENTREGA-PROVISION-OK") || !text.contains("ENTREGA-PROVISION-EXIT 0") {
        return Err(VmError::Failed { program: "the tester machine's provisioning".into(), status: "no ENTREGA-PROVISION-OK".into(), stderr: vm::tail(&text, 60) });
    }
    let name = format!("veredas-{release}");
    let vmdk_name = format!("{name}-disk1.vmdk");
    let vmdk = work.join(&vmdk_name);
    let _ = std::fs::remove_file(&vmdk);
    eprintln!("entrega: flattening the disk into a stream-optimized VMDK");
    vm::run(
        "qemu-img",
        &[
            "convert",
            "-O",
            "vmdk",
            "-o",
            &format!("subformat=streamOptimized,adapter_type=lsilogic,hwversion={}", r.machine.hardware_version),
            &disk.display().to_string(),
            &vmdk.display().to_string(),
        ],
        None,
    )?;
    let size = std::fs::metadata(&vmdk)?.len();
    let capacity = u64::from(r.machine.disk_gb) << 30;
    let ovf_name = format!("{name}.ovf");
    let mf_name = format!("{name}.mf");
    std::fs::write(work.join(&ovf_name), ovf(r, &name, &vmdk_name, size, capacity, release))?;
    let mf = format!(
        "SHA256({ovf_name})= {}\nSHA256({vmdk_name})= {}\n",
        cache::sha256(&work.join(&ovf_name))?,
        cache::sha256(&vmdk)?
    );
    std::fs::write(work.join(&mf_name), mf)?;
    let ova = work.join(format!("{name}.ova"));
    let _ = std::fs::remove_file(&ova);
    vm::run_clean(
        "tar",
        &[
            "--format=ustar",
            "--owner=0",
            "--group=0",
            "--numeric-owner",
            "--mode=a+r,u+w,go-w,a-x,ug-s",
            "-cf",
            &ova.display().to_string(),
            "-C",
            &work.display().to_string(),
            &ovf_name,
            &mf_name,
            &vmdk_name,
        ],
        None,
    )?;
    for f in [&disk, &in_tar, &vmdk, &work.join(&ovf_name), &work.join(&mf_name)] {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir_all(&input);
    Ok(Paths { ova })
}

/// The address the proof boot gives the machine's card: VMware's own prefix, never the address
/// the provisioning's card had, so a network bound to that card would not come up.
pub const PROOF_MAC: &str = "00:50:56:3f:00:01";

/// Boots a VMDK copy-on-write on VMware's devices, supervisão's port forwarded to `port` on the
/// host's loopback, QEMU's monitor on `qmp` when given (for a screen dump).
pub fn boot(r: &Recipe, vmdk: &Path, port: u16, qmp: Option<u16>, console: &Path) -> Result<Vm, VmError> {
    Vm::start(&Spec {
        disk: vmdk.to_path_buf(),
        hardware: Hardware::Vmware,
        cpus: r.machine.cpus,
        memory_mb: r.machine.memory_mb,
        seed: None,
        data: vec![],
        forwards: vec![(port, r.machine.supervisao_port)],
        snapshot: true,
        mac: Some(PROOF_MAC.into()),
        qmp,
        console: console.to_path_buf(),
    })
}

/// Builds the tester machine of a release already cut: its bundle under
/// `target/entrega/<release>/`, the recipe and the guide read at the commit its manifest names.
pub fn from_release(root: &Path, release: &str, target: &str) -> Result<Paths, VmError> {
    let work = root.join("target/entrega").join(release);
    let bundle_dir = format!("veredas-{release}-{target}");
    let bundle = work.join(format!("{bundle_dir}.tar.gz"));
    if !bundle.exists() {
        return Err(VmError::Failed { program: "the tester machine".into(), status: "refused".into(), stderr: format!("{} is not there: cut the release first", bundle.display()) });
    }
    let text = vm::run("tar", &["-xzf", &bundle.display().to_string(), "-O", &format!("{bundle_dir}/release.toml")], None)?;
    let m = crate::manifest::load(&text).map_err(|e| VmError::Failed { program: "the bundle's manifest".into(), status: "refused".into(), stderr: e.to_string() })?;
    let at = |path: &str| -> Result<String, VmError> {
        crate::git::show(root, &m.commit, path)
            .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
            .ok_or_else(|| VmError::Failed { program: "the tester machine".into(), status: "refused".into(), stderr: format!("commit {} holds no {path}", m.commit) })
    };
    let recipe = Recipe::load(&at("tools/entrega/maquina/maquina.toml")?).map_err(|e| VmError::Failed { program: "the recipe".into(), status: "refused".into(), stderr: e.to_string() })?;
    let guide = at("tools/entrega/maquina/guide.md")?;
    build(&recipe, &bundle, &bundle_dir, release, &guide, &work.join("maquina"))
}

/// The colours of a PPM screen dump sampled on a grid of every fourth pixel; `None` for a file
/// that is no binary PPM.
fn samples(ppm: &[u8]) -> Option<Vec<(u8, u8, u8)>> {
    let head = String::from_utf8_lossy(&ppm[..ppm.len().min(64)]).into_owned();
    let mut fields = head.split_ascii_whitespace();
    if fields.next() != Some("P6") {
        return None;
    }
    let w: usize = fields.next()?.parse().ok()?;
    let h: usize = fields.next()?.parse().ok()?;
    fields.next()?;
    let header = ppm.len().checked_sub(w * h * 3)?;
    let mut out = Vec::new();
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let i = header + (y * w + x) * 3;
            out.push((ppm[i], ppm[i + 1], ppm[i + 2]));
        }
    }
    Some(out)
}

/// A PPM screen dump's distinct colours, on the grid; `None` for a file that is no binary PPM.
pub fn colours(ppm: &[u8]) -> Option<usize> {
    let s = samples(ppm)?;
    Some(s.iter().collect::<std::collections::BTreeSet<_>>().len())
}

/// The share of a PPM screen dump, on the grid, in exactly `colour`; `None` for no binary PPM.
pub fn share_of(ppm: &[u8], colour: (u8, u8, u8)) -> Option<f64> {
    let s = samples(ppm)?;
    Some(s.iter().filter(|c| **c == colour).count() as f64 / s.len().max(1) as f64)
}

/// The share of the screen the page must cover, in its own canvas colour, for the proof to say the
/// browser draws it (the first release's dump: 32 % with the window not yet maximised; a desktop
/// without the page, its wallpaper and panel, holds none of that exact colour).
pub const PAGE_SHARE: f64 = 0.15;

/// Supervisão's page canvas as the release's page states it (`--canvas: #rrggbb` in its built
/// stylesheet at `commit`), so the proof looks for the page it ships, whatever its design.
pub fn page_canvas(root: &Path, commit: &str) -> Result<(u8, u8, u8), VmError> {
    let css = crate::git::show(root, commit, "crates/veredas-supervisao/web/dist/assets/app.css")
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .unwrap_or_default();
    let hex = css
        .split("--canvas:")
        .nth(1)
        .map(str::trim_start)
        .and_then(|r| r.strip_prefix('#'))
        .map(|r| r.chars().take(6).collect::<String>());
    let byte = |h: &str, at: usize| h.get(at..at + 2).and_then(|b| u8::from_str_radix(b, 16).ok());
    match hex.as_deref().and_then(|h| Some((byte(h, 0)?, byte(h, 2)?, byte(h, 4)?))) {
        Some(c) => Ok(c),
        None => Err(VmError::Failed {
            program: "the proof".into(),
            status: "refused".into(),
            stderr: format!("commit {commit}: supervisão's page states no `--canvas` colour"),
        }),
    }
}

/// What is wrong with an OVA as VMware reads it, opened into `opened`: its members in order (the
/// descriptor, the manifest, the disk), owned by root by number and read as 0644; the disk itself a
/// stream-optimized VMDK (as `qemu-img` reads it) whose size the descriptor's capacity states; the
/// descriptor parsed as XML and every value read inside its own element: the file and its size,
/// the disk and its capacity and format, the hardware version, the processors, the memory, the
/// paravirtual SCSI controller, the disk under that controller, the VMXNET3 card, BIOS firmware;
/// the manifest's sums the members'. Empty when it is right.
pub fn ova_faults(r: &Recipe, ova: &Path, release: &str, opened: &Path) -> Result<Vec<String>, VmError> {
    let name = format!("veredas-{release}");
    let vmdk_name = format!("{name}-disk1.vmdk");
    let mut out = Vec::new();
    let members = vm::run("tar", &["-tf", &ova.display().to_string()], None)?;
    let want = format!("{name}.ovf\n{name}.mf\n{vmdk_name}\n");
    if members != want {
        out.push(format!("the OVA's members are {members:?}, not {want:?}"));
    }
    let listing = vm::run("tar", &["-tvf", &ova.display().to_string()], None)?;
    for l in listing.lines() {
        if !l.starts_with("-rw-r--r-- 0/0 ") {
            out.push(format!("an OVA member is not a 0644 file owned by root: {l}"));
        }
    }
    let ovf_path = opened.join(format!("{name}.ovf"));
    let vmdk = opened.join(&vmdk_name);
    let size = std::fs::metadata(&vmdk).map(|m| m.len()).unwrap_or(0);
    let capacity = u64::from(r.machine.disk_gb) << 30;
    // The disk as qemu-img reads it.
    match vm::run("qemu-img", &["info", "--output=json", &vmdk.display().to_string()], None) {
        Ok(json) => {
            let info: serde_json::Value = serde_json::from_str(&json).unwrap_or_default();
            if info["format"] != "vmdk" || info["format-specific"]["data"]["create-type"] != "streamOptimized" {
                out.push(format!("the disk is no stream-optimized VMDK ({} {})", info["format"], info["format-specific"]["data"]["create-type"]));
            }
            if info["virtual-size"].as_u64() != Some(capacity) {
                out.push(format!("the disk holds {} bytes, the descriptor states {capacity}", info["virtual-size"]));
            }
        }
        Err(e) => out.push(format!("the disk does not read as a VMDK: {e}")),
    }
    // The descriptor, parsed.
    let text = std::fs::read_to_string(&ovf_path).unwrap_or_default();
    match roxmltree::Document::parse(&text) {
        Err(e) => out.push(format!("the descriptor is not well-formed XML: {e}")),
        Ok(doc) => out.extend(descriptor_faults(r, &doc, &vmdk_name, size, capacity)),
    }
    let mf = std::fs::read_to_string(opened.join(format!("{name}.mf"))).unwrap_or_default();
    let want_mf = format!(
        "SHA256({name}.ovf)= {}\nSHA256({vmdk_name})= {}\n",
        cache::sha256(&ovf_path).unwrap_or_default(),
        cache::sha256(&vmdk).unwrap_or_default()
    );
    if mf != want_mf {
        out.push(format!("the manifest's sums are not the members': {mf:?}"));
    }
    Ok(out)
}

/// The OVF descriptor's faults, each value read inside its own element.
fn descriptor_faults(r: &Recipe, doc: &roxmltree::Document, vmdk_name: &str, size: u64, capacity: u64) -> Vec<String> {
    const OVF: &str = "http://schemas.dmtf.org/ovf/envelope/1";
    const VMW: &str = "http://www.vmware.com/schema/ovf";
    let mut out = Vec::new();
    let named = |n: &roxmltree::Node, local: &str| n.is_element() && n.tag_name().name() == local;
    let child_text = |n: &roxmltree::Node, local: &str| -> Option<String> {
        n.children().find(|c| named(c, local)).and_then(|c| c.text()).map(|t| t.trim().to_string())
    };
    let all = |local: &str| doc.descendants().filter(|n| named(n, local)).collect::<Vec<_>>();
    let files = all("File");
    match files.as_slice() {
        [f] if f.attribute((OVF, "id")) == Some("file1") && f.attribute((OVF, "href")) == Some(vmdk_name) && f.attribute((OVF, "size")) == Some(&size.to_string()) => {}
        _ => out.push(format!("the descriptor names no one file `{vmdk_name}` of {size} bytes as file1")),
    }
    let disks = all("Disk");
    let cap = capacity.to_string();
    match disks.as_slice() {
        [d] if d.attribute((OVF, "diskId")) == Some("vmdisk1")
            && d.attribute((OVF, "fileRef")) == Some("file1")
            && d.attribute((OVF, "capacity")) == Some(cap.as_str())
            && d.attribute((OVF, "capacityAllocationUnits")) == Some("byte")
            && d.attribute((OVF, "format")) == Some("http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized") => {}
        _ => out.push(format!("the descriptor names no one stream-optimized disk vmdisk1 of {capacity} bytes on file1")),
    }
    let system = format!("vmx-{:02}", r.machine.hardware_version);
    if !all("VirtualSystemType").iter().any(|n| n.text().map(str::trim) == Some(system.as_str())) {
        out.push(format!("the descriptor's hardware version is not {system}"));
    }
    let items = all("Item");
    let of_type = |t: &str| items.iter().filter(|i| child_text(i, "ResourceType").as_deref() == Some(t)).copied().collect::<Vec<_>>();
    match of_type("3").as_slice() {
        [cpu] if child_text(cpu, "VirtualQuantity") == Some(r.machine.cpus.to_string()) => {}
        _ => out.push(format!("the descriptor's processors are not one item of {}", r.machine.cpus)),
    }
    match of_type("4").as_slice() {
        [mem] if child_text(mem, "VirtualQuantity") == Some(r.machine.memory_mb.to_string()) && child_text(mem, "AllocationUnits").as_deref() == Some("byte * 2^20") => {}
        _ => out.push(format!("the descriptor's memory is not one item of {} MB", r.machine.memory_mb)),
    }
    let scsi = of_type("6");
    let controller = match scsi.as_slice() {
        [c] if child_text(c, "ResourceSubType").as_deref() == Some("VirtualSCSI") => child_text(c, "InstanceID"),
        _ => {
            out.push("the descriptor holds no one paravirtual SCSI controller (VirtualSCSI)".into());
            None
        }
    };
    match of_type("17").as_slice() {
        [d] if child_text(d, "HostResource").as_deref() == Some("ovf:/disk/vmdisk1") && controller.is_some() && child_text(d, "Parent") == controller => {}
        _ => out.push("the descriptor's hard disk is not one item on vmdisk1 under the paravirtual SCSI controller".into()),
    }
    match of_type("10").as_slice() {
        [n] if child_text(n, "ResourceSubType").as_deref() == Some("VmxNet3") => {}
        _ => out.push("the descriptor holds no one VMXNET3 card".into()),
    }
    let firmware = doc.descendants().any(|n| named(&n, "Config") && n.attribute((VMW, "key")) == Some("firmware") && n.attribute((VMW, "value")) == Some("bios"));
    if !firmware {
        out.push("the descriptor's firmware is not BIOS".into());
    }
    out
}

/// What the proof boot of an OVA's disk saw.
pub struct Proof {
    /// The disk taken out of the OVA.
    pub vmdk: PathBuf,
    /// What is wrong with the OVA as VMware reads it ([`ova_faults`]).
    pub ova: Vec<String>,
    /// The serial console when the plant was read.
    pub console: String,
    /// The plant read from the host through the forwarded port.
    pub plant: crate::together::Report,
    /// The last screen dump, its colours and the share of it in the page's canvas colour.
    pub screen: PathBuf,
    pub colours: usize,
    pub page_share: f64,
    /// The disk's SHA-256 before the boot, and after it.
    pub before: String,
    pub after: String,
}

/// The proof of a tester machine (docs/entrega-plan.md, "How a build is proven"): the OVA opened
/// into `work` and read as VMware reads it; its disk booted copy-on-write on VMware's devices with a
/// card address the provisioning never saw; the boot's check of the release's sums and of its cards
/// waited for on the console; the plant read from the host; the screen dumped until the page, in its
/// canvas colour `canvas`, covers [`PAGE_SHARE`] of it; the disk hashed again.
pub fn prove(r: &Recipe, ova: &Path, release: &str, canvas: (u8, u8, u8), work: &Path) -> Result<Proof, VmError> {
    let opened = work.join("opened");
    let _ = std::fs::remove_dir_all(&opened);
    std::fs::create_dir_all(&opened)?;
    vm::run("tar", &["-xf", &ova.display().to_string(), "-C", &opened.display().to_string()], None)?;
    let ova_found = ova_faults(r, ova, release, &opened)?;
    let vmdk = opened.join(format!("veredas-{release}-disk1.vmdk"));
    let before = cache::sha256(&vmdk)?;
    let port = vm::free_port()?;
    let qmp = vm::free_port()?;
    let console = work.join("proof-console.log");
    let mut m = boot(r, &vmdk, port, Some(qmp), &console)?;
    m.wait_for("VEREDAS-SUMS-", Duration::from_secs(300))?;
    let plant = crate::together::read_plant(([127, 0, 0, 1], port).into(), Duration::from_secs(300), Duration::from_secs(2));
    let screen = work.join("proof-screen.ppm");
    let (mut seen, mut share) = (0, 0.0);
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_secs(300) {
        std::thread::sleep(Duration::from_secs(10));
        let _ = std::fs::remove_file(&screen);
        if vm::screendump(qmp, &screen).is_ok()
            && let Ok(bytes) = std::fs::read(&screen)
        {
            seen = colours(&bytes).unwrap_or(0);
            share = share_of(&bytes, canvas).unwrap_or(0.0);
            if share >= PAGE_SHARE {
                break;
            }
        }
    }
    let console_text = m.console();
    m.kill();
    let after = cache::sha256(&vmdk)?;
    Ok(Proof { vmdk, ova: ova_found, console: console_text, plant, screen, colours: seen, page_share: share, before, after })
}

/// Why a proof does not show the machine working: the OVA as VMware reads it, the release's sums
/// checked at boot, the guest on VMware's disk and network drivers and naming the proof's card, the
/// plant from the host, the page drawn by the desktop's browser, the disk unchanged. Empty when it
/// does.
pub fn proof_faults(p: &Proof, release: &str) -> Vec<String> {
    let mut out = p.ova.clone();
    if !p.console.contains(&format!("VEREDAS-SUMS-OK {release}")) {
        out.push(format!("the boot did not check the release's sums (VEREDAS-SUMS-OK {release})"));
    }
    for driver in ["vmw_pvscsi", "vmxnet3"] {
        if !p.console.contains(driver) {
            out.push(format!("the guest did not name {driver}"));
        }
    }
    if !p.console.lines().any(|l| l.contains("VEREDAS-CARD ") && l.trim_end().ends_with(PROOF_MAC)) {
        out.push(format!("the guest named no card with the proof's address {PROOF_MAC}"));
    }
    out.extend(crate::together::plant_faults(&p.plant));
    if p.page_share < PAGE_SHARE {
        out.push(format!(
            "the page covers {:.0} % of the screen in its canvas colour, under {:.0} %: the browser does not draw it ({})",
            p.page_share * 100.0,
            PAGE_SHARE * 100.0,
            p.screen.display()
        ));
    }
    if p.before != p.after {
        out.push("the proof boot changed the shipped disk".into());
    }
    out
}

/// Proves the tester machine already built for a release (`entrega maquina`), the recipe read at
/// the commit its manifest names; the proof and its faults.
pub fn prove_release(root: &Path, release: &str, target: &str) -> Result<(Proof, Vec<String>), VmError> {
    let work = root.join("target/entrega").join(release);
    let bundle_dir = format!("veredas-{release}-{target}");
    let bundle = work.join(format!("{bundle_dir}.tar.gz"));
    let ova = work.join("maquina").join(format!("veredas-{release}.ova"));
    if !ova.exists() {
        return Err(VmError::Failed { program: "the proof".into(), status: "refused".into(), stderr: format!("{} is not there: build the machine first", ova.display()) });
    }
    let text = vm::run("tar", &["-xzf", &bundle.display().to_string(), "-O", &format!("{bundle_dir}/release.toml")], None)?;
    let m = crate::manifest::load(&text).map_err(|e| VmError::Failed { program: "the bundle's manifest".into(), status: "refused".into(), stderr: e.to_string() })?;
    let recipe_text = crate::git::show(root, &m.commit, "tools/entrega/maquina/maquina.toml")
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .ok_or_else(|| VmError::Failed { program: "the proof".into(), status: "refused".into(), stderr: format!("commit {} holds no recipe", m.commit) })?;
    let recipe = Recipe::load(&recipe_text).map_err(|e| VmError::Failed { program: "the recipe".into(), status: "refused".into(), stderr: e.to_string() })?;
    let canvas = page_canvas(root, &m.commit)?;
    let proof = prove(&recipe, &ova, release, canvas, &work.join("prova"))?;
    let faults = proof_faults(&proof, release);
    Ok((proof, faults))
}

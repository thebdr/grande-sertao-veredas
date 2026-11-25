//! The tester machine (docs/alforge-plan.md, part two): a release's bundle provisioned unattended
//! into the distribution's cloud image under QEMU, the disk flattened into a stream-optimized VMDK,
//! an OVF descriptor for VMware's virtual hardware and a manifest of sums, tarred into an OVA. The
//! proof boots the disk taken out of the OVA on QEMU's models of VMware's devices.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cache;
use crate::machine::Recipe;
use crate::vm::{self, DataDisk, Hardware, Spec, Vm, VmError};

/// Bumped when what the provisioning does changes.
pub const PROVISIONING: u32 = 2;

/// The machine's update command (docs/questions.md Q437, ruled "Update command"), installed by the
/// provisioning as `/usr/local/sbin/veredas-atualizar`: a newer release's bundle, given by path,
/// checked and installed beside the old one, `current` moved to it, the services restarted on it,
/// the plant's files left alone.
pub const ATUALIZAR: &str = include_str!("veredas-atualizar.py");

/// The provisioning script run as root in the guest, its network up: the packages, the user, the
/// release under `/opt/veredas/`, the plant in the user's home, the services, the desktop's
/// automatic login and browser, the update command, the network handed to NetworkManager,
/// cloud-init and the SSH server gone, the disk's free space trimmed.
pub fn provision_script(r: &Recipe, bundle_dir: &str) -> String {
    let m = &r.machine;
    let u = &m.user;
    let packages = m.packages.join(" ");
    let remove = m.remove.join(" ");
    let port = m.supervisao_port;
    format!(
        r#"set -eu
export DEBIAN_FRONTEND=noninteractive
echo "ALFORGE-STEP packages"
apt-get update
apt-get install -y {packages}
if [ -n "{remove}" ]; then apt-get purge -y {remove}; fi
apt-get -y autoremove --purge
echo "ALFORGE-STEP user"
id debian >/dev/null 2>&1 && userdel -r debian || true
useradd -m -s /bin/bash -G sudo {u}
for g in netdev systemd-journal; do if getent group $g >/dev/null; then usermod -aG $g {u}; fi; done
echo '{u}:{password}' | chpasswd
echo veredas > /etc/hostname
sed -i 's/^127.0.1.1.*/127.0.1.1 veredas/' /etc/hosts
grep -q '^127.0.1.1' /etc/hosts || echo '127.0.1.1 veredas' >> /etc/hosts
echo "ALFORGE-STEP release"
mkdir -p /opt/veredas
tar -xzf /alforge/bundle.tar.gz -C /opt/veredas
ln -sfn {bundle_dir} /opt/veredas/current
for b in /opt/veredas/current/bin/*; do ln -sf "$b" /usr/local/bin/; done
mkdir -p /usr/local/sbin
cat > /usr/local/sbin/veredas-atualizar <<'ATUALIZAR'
{atualizar}ATUALIZAR
chmod 755 /usr/local/sbin/veredas-atualizar
cp -r /opt/veredas/current/planta /home/{u}/planta
cp /alforge/guide.md /home/{u}/README-veredas.md
echo "ALFORGE-STEP services"
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
# The release's files checked against its SHA256SUMS, the network cards the machine sees, and
# what the provisioning promised: the recipe's packages installed, the bench and supervisão services
# of the recipe's user, no SSH server, cloud-init off, every executable on the PATH, the update
# command there. The release is the one `current` names, as an update moves it.
cd /opt/veredas/current
rel=$(sed -n 's/^release = "\(.*\)"$/\1/p' release.toml)
if sha256sum --quiet -c SHA256SUMS; then echo "VEREDAS-SUMS-OK $rel"; else echo "VEREDAS-SUMS-FAILED $rel"; fi
echo "VEREDAS-BUNDLE $(sha256sum SHA256SUMS | cut -d' ' -f1)"
for c in /sys/class/net/*; do n=$(basename "$c"); [ "$n" = lo ] || echo "VEREDAS-CARD $n $(cat "$c/address")"; done
if [ -e /usr/sbin/sshd ] || [ -e /lib/systemd/system/ssh.service ] || [ -e /usr/lib/systemd/system/ssh.service ]; then echo "VEREDAS-SSH present"; else echo "VEREDAS-SSH absent"; fi
if [ -e /etc/cloud/cloud-init.disabled ]; then echo "VEREDAS-CLOUD-INIT disabled"; else echo "VEREDAS-CLOUD-INIT enabled"; fi
missing=
for p in {packages}; do dpkg-query -W -f='${{Status}}' "$p" 2>/dev/null | grep -q 'install ok installed' || missing="$missing $p"; done
if [ -z "$missing" ]; then echo "VEREDAS-PACKAGES ok"; else echo "VEREDAS-PACKAGES missing$missing"; fi
others=
for s in veredas-bancada veredas-supervisao; do [ "$(systemctl show -p User --value "$s.service")" = "{u}" ] || others="$others $s"; done
if [ -z "$others" ]; then echo "VEREDAS-SERVICES-USER ok"; else echo "VEREDAS-SERVICES-USER other:$others"; fi
missing=
for b in /opt/veredas/current/bin/*; do n=$(basename "$b"); [ "$(command -v "$n")" = "/usr/local/bin/$n" ] || missing="$missing $n"; done
if [ -z "$missing" ]; then echo "VEREDAS-ON-PATH ok"; else echo "VEREDAS-ON-PATH missing$missing"; fi
if [ -x /usr/local/sbin/veredas-atualizar ] && python3 -c 'import tomllib, tarfile' 2>/dev/null; then echo "VEREDAS-ATUALIZAR ok"; else echo "VEREDAS-ATUALIZAR missing"; fi
SCRIPT
chmod 755 /usr/local/libexec/veredas-verifica
cat > /etc/systemd/system/veredas-verifica.service <<'UNIT'
[Unit]
Description=Veredas: the release's files checked against its SHA256SUMS
# Done before a login starts on the serial console: agetty hangs the line up when it starts, and a
# line written to the console meanwhile is lost (docs/alforge-plan.md, "Traps").
Before=serial-getty@ttyS0.service getty@tty1.service

[Service]
Type=oneshot
ExecStart=/usr/local/libexec/veredas-verifica
StandardOutput=journal+console

[Install]
WantedBy=multi-user.target
UNIT
systemctl enable veredas-bancada.service veredas-supervisao.service veredas-verifica.service
echo "ALFORGE-STEP desktop"
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
echo "ALFORGE-STEP network"
rm -f /etc/netplan/*.yaml
systemctl disable systemd-networkd.service systemd-networkd.socket systemd-networkd-wait-online.service || true
systemctl enable NetworkManager.service
echo "ALFORGE-STEP cleanup"
apt-get clean
rm -rf /var/lib/apt/lists/*
touch /etc/cloud/cloud-init.disabled
rm -f /var/log/cloud-init.log /var/log/cloud-init-output.log
truncate -s 0 /etc/machine-id
rm -f /var/lib/dbus/machine-id
rm -rf /alforge
journalctl --rotate || true
journalctl --vacuum-time=1s || true
sync
fstrim -av || true
echo ALFORGE-PROVISION-OK
"#,
        password = m.password,
        atualizar = ATUALIZAR,
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
    std::fs::write(input.join("provision.sh"), provision_script(r, bundle_dir))?;
    let in_tar = work.join("maquina-in.tar");
    vm::run("tar", &["-cf", &in_tar.display().to_string(), "-C", &input.display().to_string(), "."], None)?;
    let disk = work.join("maquina.qcow2");
    vm::overlay(&base, &disk, r.machine.disk_gb)?;
    // The provisioning logs to a file whose tail is echoed to the console after (agetty hangs up
    // ttyS0 when it starts; docs/alforge-plan.md, "Traps"); the log stays in the image.
    let user_data = "#cloud-config\nusers: []\nruncmd:\n  - [ sh, -c, 'mkdir -p /alforge && tar -xf /dev/disk/by-id/virtio-alforge-in -C /alforge && sh /alforge/provision.sh > /var/log/veredas-provision.log 2>&1; rc=$?; tail -n 60 /var/log/veredas-provision.log > /dev/ttyS0; echo \"ALFORGE-PROVISION-EXIT $rc\" > /dev/ttyS0' ]\npower_state:\n  mode: poweroff\n  delay: now\n  timeout: 120\n";
    let nonce = cache::sha256_text(&format!("{release}{:?}", std::time::SystemTime::now()));
    let seed = work.join("maquina-seed.iso");
    vm::seed(&work.join("maquina-seed"), &seed, user_data, &format!("alforge-maquina-{}", &nonce[..16]))?;
    let console = work.join("maquina-console.log");
    eprintln!("alforge: provisioning the tester machine; its console is {}", console.display());
    let mut m = Vm::start(&Spec {
        disk: disk.clone(),
        hardware: Hardware::Virtio,
        cpus: r.machine.cpus.max(4),
        memory_mb: r.machine.memory_mb,
        seed: Some(seed),
        data: vec![DataDisk { file: in_tar.clone(), serial: "alforge-in".into(), writable: false }],
        forwards: vec![],
        snapshot: false,
        mac: None,
        qmp: None,
        console: console.clone(),
        serial_in: false,
    })?;
    m.wait_off(Duration::from_secs(90 * 60))?;
    let text = vm::read_console(&console);
    let faults = provision_faults(&text);
    if !faults.is_empty() {
        return Err(VmError::Failed { program: "the tester machine's provisioning".into(), status: faults.join("; "), stderr: vm::tail(&text, 60) });
    }
    let name = format!("veredas-{release}");
    let vmdk_name = format!("{name}-disk1.vmdk");
    let vmdk = work.join(&vmdk_name);
    let _ = std::fs::remove_file(&vmdk);
    eprintln!("alforge: flattening the disk into a stream-optimized VMDK");
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
        serial_in: false,
    })
}

/// Builds the tester machine of a release already cut: its bundle under
/// `target/alforge/<release>/`, the recipe and the guide read at the commit its manifest names.
/// Whether release `release`'s cut ran to its end: its manifest written where a cut writes it
/// (`releases/<release>.toml` for a real cut, `<work>/<release>.toml` for a trial; a real cut's
/// decides over a trial's of the same release left beside the bundle, which the real cut's work
/// replaced) and the bundle's own `release.toml` byte for byte. A cut refused after packing (its
/// run together failed) leaves a bundle and no manifest; a machine is never built from such a
/// bundle.
/// Why the provisioning's console (its log's tail, then its exit) does not show it finished: no
/// `ALFORGE-PROVISION-OK`, another exit than 0, or the root's free space not trimmed (`fstrim`'s
/// line for `/`). Empty when it does.
pub fn provision_faults(console: &str) -> Vec<String> {
    let mut out = Vec::new();
    if !console.contains("ALFORGE-PROVISION-OK") {
        out.push("no ALFORGE-PROVISION-OK".to_string());
    }
    if !console.contains("ALFORGE-PROVISION-EXIT 0") {
        out.push("no ALFORGE-PROVISION-EXIT 0".to_string());
    }
    if !console.lines().any(|l| l.contains("/: ") && l.contains(" trimmed on ")) {
        out.push("the root's free space was not trimmed (no `/: ... trimmed on` line)".to_string());
    }
    out
}

pub fn cut_finished(root: &Path, release: &str, work: &Path, bundle_manifest: &str) -> Result<(), VmError> {
    let refused = |stderr: String| VmError::Failed { program: "the tester machine".into(), status: "refused".into(), stderr };
    let places = [root.join("releases").join(format!("{release}.toml")), work.join(format!("{release}.toml"))];
    let found: Vec<&PathBuf> = places.iter().filter(|p| p.exists()).take(1).collect();
    if found.is_empty() {
        return Err(refused(format!(
            "release {release} was not cut to its end: no manifest at {} or {}, and a bundle without one is what a refused cut leaves",
            places[0].display(),
            places[1].display()
        )));
    }
    for p in found {
        if std::fs::read_to_string(p)? != bundle_manifest {
            return Err(refused(format!("{} is not the bundle's own release.toml: the bundle is not that cut's", p.display())));
        }
    }
    Ok(())
}

pub fn from_release(root: &Path, release: &str, target: &str) -> Result<Paths, VmError> {
    let work = root.join("target/alforge").join(release);
    let bundle_dir = format!("veredas-{release}-{target}");
    let bundle = work.join(format!("{bundle_dir}.tar.gz"));
    if !bundle.exists() {
        return Err(VmError::Failed { program: "the tester machine".into(), status: "refused".into(), stderr: format!("{} is not there: cut the release first", bundle.display()) });
    }
    let text = vm::run("tar", &["-xzf", &bundle.display().to_string(), "-O", &format!("{bundle_dir}/release.toml")], None)?;
    cut_finished(root, release, &work, &text)?;
    let m = crate::manifest::load(&text).map_err(|e| VmError::Failed { program: "the bundle's manifest".into(), status: "refused".into(), stderr: e.to_string() })?;
    let at = |path: &str| -> Result<String, VmError> {
        crate::git::show(root, &m.commit, path)
            .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
            .ok_or_else(|| VmError::Failed { program: "the tester machine".into(), status: "refused".into(), stderr: format!("commit {} holds no {path}", m.commit) })
    };
    let folder = crate::folder_at(root, &m.commit)
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .unwrap_or(crate::FOLDERS[0]);
    let recipe = Recipe::load(&at(&format!("{folder}/maquina/maquina.toml"))?).map_err(|e| VmError::Failed { program: "the recipe".into(), status: "refused".into(), stderr: e.to_string() })?;
    let guide = at(&format!("{folder}/maquina/guide.md"))?;
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

/// An archive's raw tar headers in order, as a reader that knows only ustar walks them: each one's
/// name, type flag and magic; the data blocks between skipped by the size each header states.
fn raw_headers(archive: &Path) -> std::io::Result<Vec<(String, u8, Vec<u8>)>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(archive)?;
    let mut out = Vec::new();
    let mut head = [0u8; 512];
    while out.len() < 64 && f.read_exact(&mut head).is_ok() && head.iter().any(|b| *b != 0) {
        let field = |a: usize, b: usize| String::from_utf8_lossy(&head[a..b]).trim_end_matches('\0').to_string();
        let size = u64::from_str_radix(field(124, 136).trim(), 8).unwrap_or(0);
        out.push((field(0, 100), head[156], head[257..265].to_vec()));
        f.seek(SeekFrom::Current(i64::try_from(size.div_ceil(512) * 512).unwrap_or(i64::MAX)))?;
    }
    Ok(out)
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
    let members = vm::run_clean("tar", &["-tf", &ova.display().to_string()], None)?;
    let want = format!("{name}.ovf\n{name}.mf\n{vmdk_name}\n");
    if members != want {
        out.push(format!("the OVA's members are {members:?}, not {want:?}"));
    }
    // A POSIX ustar archive of three plain files, as a reader that knows only ustar walks it: every
    // raw header's magic, type and name (a pax or GNU extension header is a member to such a reader).
    let headers = raw_headers(ova)?;
    if let Some((_, _, magic)) = headers.iter().find(|(_, _, m)| m.as_slice() != b"ustar\x0000") {
        out.push(format!("the OVA is no POSIX ustar archive: its first header's magic is {:?}", String::from_utf8_lossy(magic)));
    }
    let raw: Vec<String> = headers.iter().map(|(n, t, _)| format!("{n} ({})", *t as char)).collect();
    let plain: Vec<String> = [format!("{name}.ovf"), format!("{name}.mf"), vmdk_name.clone()].iter().map(|n| format!("{n} (0)")).collect();
    if raw != plain {
        out.push(format!("the OVA's raw headers are {raw:?}, not the descriptor, the manifest and the disk as plain files"));
    }
    let listing = vm::run_clean("tar", &["-tvf", &ova.display().to_string()], None)?;
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

/// Whether `n` is the element `local` of namespace `ns`.
fn is(n: &roxmltree::Node, ns: &str, local: &str) -> bool {
    n.is_element() && n.tag_name().namespace() == Some(ns) && n.tag_name().name() == local
}

/// The elements `local` of namespace `ns` directly under `n`.
fn children<'a, 'i>(n: roxmltree::Node<'a, 'i>, ns: &str, local: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    n.children().filter(|c| is(c, ns, local)).collect()
}

/// The one node of a list, if it holds exactly one.
fn one<'a, 'i>(list: Vec<roxmltree::Node<'a, 'i>>) -> Option<roxmltree::Node<'a, 'i>> {
    if list.len() == 1 { Some(list[0]) } else { None }
}

/// The trimmed text of the element `local` of namespace `ns` directly under `n`.
fn text_of(n: roxmltree::Node, ns: &str, local: &str) -> Option<String> {
    n.children().find(|c| is(c, ns, local)).and_then(|c| c.text()).map(|t| t.trim().to_string())
}

/// The OVF descriptor's faults, read as VMware reads it: the Envelope in OVF's namespace, its
/// References, DiskSection and VirtualSystem directly under it, the hardware the Items directly
/// under the VirtualSystem's VirtualHardwareSection, each value an element of its own namespace
/// (RASD for an Item's, VSSD for the System's) inside its own element.
fn descriptor_faults(r: &Recipe, doc: &roxmltree::Document, vmdk_name: &str, size: u64, capacity: u64) -> Vec<String> {
    const OVF: &str = "http://schemas.dmtf.org/ovf/envelope/1";
    const VMW: &str = "http://www.vmware.com/schema/ovf";
    const RASD: &str = "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_ResourceAllocationSettingData";
    const VSSD: &str = "http://schemas.dmtf.org/wbem/wscim/1/cim-schema/2/CIM_VirtualSystemSettingData";
    let mut out = Vec::new();
    let root = doc.root_element();
    if !is(&root, OVF, "Envelope") {
        out.push("the descriptor's root is no OVF Envelope (its namespace or its name)".into());
        return out;
    }
    let file = one(children(root, OVF, "References")).and_then(|refs| one(children(refs, OVF, "File")));
    match file {
        Some(f) if f.attribute((OVF, "id")) == Some("file1") && f.attribute((OVF, "href")) == Some(vmdk_name) && f.attribute((OVF, "size")) == Some(&size.to_string()) => {}
        _ => out.push(format!("the descriptor's References name no one file `{vmdk_name}` of {size} bytes as file1")),
    }
    let cap = capacity.to_string();
    let disk = one(children(root, OVF, "DiskSection")).and_then(|ds| one(children(ds, OVF, "Disk")));
    match disk {
        Some(d)
            if d.attribute((OVF, "diskId")) == Some("vmdisk1")
                && d.attribute((OVF, "fileRef")) == Some("file1")
                && d.attribute((OVF, "capacity")) == Some(cap.as_str())
                && d.attribute((OVF, "capacityAllocationUnits")) == Some("byte")
                && d.attribute((OVF, "format")) == Some("http://www.vmware.com/interfaces/specifications/vmdk.html#streamOptimized") => {}
        _ => out.push(format!("the descriptor's DiskSection names no one stream-optimized disk vmdisk1 of {capacity} bytes on file1")),
    }
    let Some(hardware) = one(children(root, OVF, "VirtualSystem")).and_then(|vs| one(children(vs, OVF, "VirtualHardwareSection"))) else {
        out.push("the descriptor holds no one VirtualSystem with one VirtualHardwareSection".into());
        return out;
    };
    let system = format!("vmx-{:02}", r.machine.hardware_version);
    let found = one(children(hardware, OVF, "System")).and_then(|s| text_of(s, VSSD, "VirtualSystemType"));
    if found.as_deref() != Some(system.as_str()) {
        out.push(format!("the descriptor's hardware version is not {system}"));
    }
    let items = children(hardware, OVF, "Item");
    let of_type = |t: &str| items.iter().filter(|i| text_of(**i, RASD, "ResourceType").as_deref() == Some(t)).copied().collect::<Vec<_>>();
    match of_type("3").as_slice() {
        [cpu] if text_of(*cpu, RASD, "VirtualQuantity") == Some(r.machine.cpus.to_string()) => {}
        _ => out.push(format!("the descriptor's processors are not one item of {}", r.machine.cpus)),
    }
    match of_type("4").as_slice() {
        [mem] if text_of(*mem, RASD, "VirtualQuantity") == Some(r.machine.memory_mb.to_string()) && text_of(*mem, RASD, "AllocationUnits").as_deref() == Some("byte * 2^20") => {}
        _ => out.push(format!("the descriptor's memory is not one item of {} MB", r.machine.memory_mb)),
    }
    let controller = match of_type("6").as_slice() {
        [c] if text_of(*c, RASD, "ResourceSubType").as_deref() == Some("VirtualSCSI") => text_of(*c, RASD, "InstanceID"),
        _ => {
            out.push("the descriptor holds no one paravirtual SCSI controller (VirtualSCSI)".into());
            None
        }
    };
    match of_type("17").as_slice() {
        [d] if text_of(*d, RASD, "HostResource").as_deref() == Some("ovf:/disk/vmdisk1") && controller.is_some() && text_of(*d, RASD, "Parent") == controller => {}
        _ => out.push("the descriptor's hard disk is not one item on vmdisk1 under the paravirtual SCSI controller".into()),
    }
    match of_type("10").as_slice() {
        [n] if text_of(*n, RASD, "ResourceSubType").as_deref() == Some("VmxNet3") => {}
        _ => out.push("the descriptor holds no one VMXNET3 card".into()),
    }
    let firmware = hardware.children().any(|n| is(&n, VMW, "Config") && n.attribute((VMW, "key")) == Some("firmware") && n.attribute((VMW, "value")) == Some("bios"));
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
    /// The SHA-256 of the cut's bundle's `SHA256SUMS` (every file of it), which the boot's check
    /// must say the machine holds.
    pub bundle: String,
}

/// The proof of a tester machine (docs/alforge-plan.md, "How a build is proven"): the OVA opened
/// into `work` and read as VMware reads it; its disk booted copy-on-write on VMware's devices with a
/// card address the provisioning never saw; the boot's check of the release's sums and of its cards
/// waited for on the console; the plant read from the host; the screen dumped until the page, in its
/// canvas colour `canvas`, covers [`PAGE_SHARE`] of it; the disk hashed again.
pub fn prove(r: &Recipe, ova: &Path, release: &str, canvas: (u8, u8, u8), bundle: &str, work: &Path) -> Result<Proof, VmError> {
    let opened = work.join("opened");
    let _ = std::fs::remove_dir_all(&opened);
    std::fs::create_dir_all(&opened)?;
    vm::run_clean("tar", &["-xf", &ova.display().to_string(), "-C", &opened.display().to_string()], None)?;
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
    Ok(Proof { vmdk, ova: ova_found, console: console_text, plant, screen, colours: seen, page_share: share, before, after, bundle: bundle.to_string() })
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
    // The machine holds this cut's bundle, not another cut's of the same release.
    if p.bundle.is_empty() || !p.console.contains(&format!("VEREDAS-BUNDLE {}", p.bundle)) {
        out.push(format!("the machine does not hold this cut's bundle (no `VEREDAS-BUNDLE {}` on the console)", p.bundle));
    }
    // What the provisioning promised, as the boot's check says it and systemd at a first boot.
    for (line, what) in [
        ("VEREDAS-PACKAGES ok", "the recipe's packages are not all installed"),
        ("VEREDAS-SERVICES-USER ok", "the bench and supervisão are not services of the recipe's user"),
        ("VEREDAS-SSH absent", "the machine holds an SSH server"),
        ("VEREDAS-CLOUD-INIT disabled", "cloud-init is not disabled"),
        ("VEREDAS-ON-PATH ok", "the release's executables are not all on the PATH"),
        ("VEREDAS-ATUALIZAR ok", "the update command is not installed"),
        ("Initializing machine ID from random generator", "the shipped disk carries a machine identity"),
    ] {
        if !p.console.contains(line) {
            out.push(format!("{what} (no `{line}` on the console)"));
        }
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

/// Proves the tester machine already built for a release (`alforge maquina`), the recipe read at
/// the commit its manifest names; the proof and its faults.
pub fn prove_release(root: &Path, release: &str, target: &str) -> Result<(Proof, Vec<String>), VmError> {
    let s = proof_setup(root, release, target)?;
    let proof = prove(&s.recipe, &s.ova, release, s.canvas, &s.bundle, &root.join("target/alforge").join(release).join("prova"))?;
    let faults = proof_faults(&proof, release);
    Ok((proof, faults))
}

/// What `alforge prova <release>` proves with ([`proof_setup`]).
pub struct Setup {
    /// The recipe at the release's commit.
    pub recipe: Recipe,
    /// The release's OVA, where `alforge maquina` builds it.
    pub ova: PathBuf,
    /// Supervisão's page canvas at the release's commit.
    pub canvas: (u8, u8, u8),
    /// The SHA-256 of the release's bundle's `SHA256SUMS`, the bundle the machine must hold.
    pub bundle: String,
}

/// What `alforge prova <release>` proves with: the release's OVA where `alforge maquina` builds it,
/// a cut of the release that ran to its end ([`cut_finished`]) and the digest of its bundle's sums,
/// and the recipe and supervisão's page canvas at the commit the release's manifest names (never
/// HEAD's, never the working tree's), so an older release is proven by its own.
pub fn proof_setup(root: &Path, release: &str, target: &str) -> Result<Setup, VmError> {
    let work = root.join("target/alforge").join(release);
    let bundle_dir = format!("veredas-{release}-{target}");
    let bundle = work.join(format!("{bundle_dir}.tar.gz"));
    let ova = work.join("maquina").join(format!("veredas-{release}.ova"));
    if !ova.exists() {
        return Err(VmError::Failed { program: "the proof".into(), status: "refused".into(), stderr: format!("{} is not there: build the machine first", ova.display()) });
    }
    let text = vm::run("tar", &["-xzf", &bundle.display().to_string(), "-O", &format!("{bundle_dir}/release.toml")], None)?;
    cut_finished(root, release, &work, &text)?;
    let sums = vm::run("tar", &["-xzf", &bundle.display().to_string(), "-O", &format!("{bundle_dir}/SHA256SUMS")], None)?;
    let m = crate::manifest::load(&text).map_err(|e| VmError::Failed { program: "the bundle's manifest".into(), status: "refused".into(), stderr: e.to_string() })?;
    let folder = crate::folder_at(root, &m.commit)
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .unwrap_or(crate::FOLDERS[0]);
    let recipe_text = crate::git::show(root, &m.commit, &format!("{folder}/maquina/maquina.toml"))
        .map_err(|e| VmError::Io(std::io::Error::other(e.to_string())))?
        .ok_or_else(|| VmError::Failed { program: "the proof".into(), status: "refused".into(), stderr: format!("commit {} holds no recipe", m.commit) })?;
    let recipe = Recipe::load(&recipe_text).map_err(|e| VmError::Failed { program: "the recipe".into(), status: "refused".into(), stderr: e.to_string() })?;
    let canvas = page_canvas(root, &m.commit)?;
    Ok(Setup { recipe, ova, canvas, bundle: cache::sha256_text(&sums) })
}

/// What the update command did inside a tester machine (C-293): the shipped disk booted
/// copy-on-write on VMware's devices, a newer release's bundle handed in on a disk, the command run
/// at the serial console as a tester runs it, then run again on the same bundle.
#[derive(Debug)]
pub struct UpdateProof {
    /// The serial console, the login and every command's output on it.
    pub console: String,
    /// The plant read from the host after the update.
    pub plant: crate::together::Report,
    /// The shipped disk's SHA-256 before and after.
    pub before: String,
    pub after: String,
}

/// The lines the proof's commands print, each marker split by quotes in what is typed so the
/// console's echo of a command never reads as its output.
fn marked(console: &str, marker: &str) -> Vec<String> {
    console.lines().filter_map(|l| l.trim_end().strip_prefix(&format!("ALFORGE-{marker} ")).map(str::to_string)).collect()
}

/// Boots the disk of `vmdk` copy-on-write with `newer` (a release's bundle archive) on a data disk,
/// logs in on the serial console as the recipe's user, and runs the update command on it, then
/// again on the same bundle; what each printed is read back from the console, and the plant from
/// the host.
pub fn prove_update(r: &Recipe, vmdk: &Path, newer: &Path, work: &Path) -> Result<UpdateProof, VmError> {
    std::fs::create_dir_all(work)?;
    let name = newer.file_name().and_then(|n| n.to_str()).ok_or_else(|| VmError::Io(std::io::Error::other("the bundle has no file name")))?.to_string();
    let handed = work.join("update.tar");
    vm::run_clean("tar", &["-cf", &handed.display().to_string(), "-C", &newer.parent().unwrap_or(Path::new(".")).display().to_string(), &name], None)?;
    let before = cache::sha256(vmdk)?;
    let port = vm::free_port()?;
    let console = work.join("update-console.log");
    let mut m = Vm::start(&Spec {
        disk: vmdk.to_path_buf(),
        hardware: Hardware::Vmware,
        cpus: r.machine.cpus,
        memory_mb: r.machine.memory_mb,
        seed: None,
        data: vec![DataDisk { file: handed.clone(), serial: "alforge-update".into(), writable: false }],
        forwards: vec![(port, r.machine.supervisao_port)],
        snapshot: true,
        mac: Some(PROOF_MAC.into()),
        qmp: None,
        console: console.clone(),
        serial_in: true,
    })?;
    let u = &r.machine.user;
    let pw = &r.machine.password;
    m.wait_for("login:", Duration::from_secs(300))?;
    // The plant running from the boot before anything is typed.
    let _ = crate::together::read_plant(([127, 0, 0, 1], port).into(), Duration::from_secs(300), Duration::from_secs(2));
    m.type_line(u)?;
    m.wait_for("Password:", Duration::from_secs(60))?;
    m.type_line(pw)?;
    m.wait_for(&format!("{u}@"), Duration::from_secs(60))?;
    let sudo = format!("echo '{pw}' | sudo -S -p ''");
    // The plant's files as the tester holds them: the ones the release shipped, at their place in
    // the home folder (what the bench and supervisão write beside them at run time is theirs).
    let plant_sum = "$(cd ~/planta && xargs sha256sum < /tmp/plant-files | sha256sum | cut -c1-64)";
    for line in [
        // The kernel's messages off this console, so no line of theirs splits one of the proof's.
        format!("{sudo} dmesg -n 1; (cd /opt/veredas/current/planta && find . -type f | sort) > /tmp/plant-files; echo \"ALFORGE-\"\"PLANT-BEFORE {plant_sum}\""),
        format!("{sudo} tar -xf /dev/disk/by-id/virtio-alforge-update -C /tmp; {sudo} veredas-atualizar /tmp/{name}; echo \"ALFORGE-\"\"UPDATE-EXIT $?\""),
    ] {
        m.type_line(&line)?;
    }
    m.wait_for("ALFORGE-UPDATE-EXIT ", Duration::from_secs(300))?;
    // The services restarted on the new release: what each running executable is, the boot's
    // check of the release `current` now names, the plant's files, and the command again.
    std::thread::sleep(Duration::from_secs(10));
    for line in [
        "echo \"ALFORGE-\"\"CURRENT $(readlink /opt/veredas/current)\"; echo \"ALFORGE-\"\"RELEASES $(ls /opt/veredas | tr '\\n' ' ')\"".to_string(),
        "for p in $(pgrep -f 'bin/veredas-(maquinar|supervisao) '); do e=$(readlink /proc/$p/exe); case $e in /opt/veredas/*) echo \"ALFORGE-\"\"EXE $e\";; esac; done".to_string(),
        format!("{sudo} /usr/local/libexec/veredas-verifica | sed 's/^/ALFORGE-CHECK /'; echo \"ALFORGE-\"\"PLANT-AFTER {plant_sum}\""),
        format!("{sudo} veredas-atualizar /tmp/{name}; echo \"ALFORGE-\"\"AGAIN-EXIT $?\""),
    ] {
        m.type_line(&line)?;
    }
    m.wait_for("ALFORGE-AGAIN-EXIT ", Duration::from_secs(300))?;
    let plant = crate::together::read_plant(([127, 0, 0, 1], port).into(), Duration::from_secs(300), Duration::from_secs(2));
    let text = m.console();
    m.kill();
    let after = cache::sha256(vmdk)?;
    let _ = std::fs::remove_file(&handed);
    Ok(UpdateProof { console: text, plant, before, after })
}

/// What an [`UpdateProof`] shows wrong, each named: release `newer` (on `target`) installed by the
/// command beside `release`, `current` moved to it, both services running its executables, the
/// boot's check passing on it, the plant's files unchanged, the same bundle refused again, the
/// plant served from the host, the shipped disk unchanged.
pub fn update_faults(p: &UpdateProof, release: &str, newer: &str, target: &str) -> Vec<String> {
    let mut out = Vec::new();
    let old_dir = format!("veredas-{release}-{target}");
    let new_dir = format!("veredas-{newer}-{target}");
    let one = |marker: &str| marked(&p.console, marker).into_iter().next();
    if one("UPDATE-EXIT").as_deref() != Some("0") {
        out.push(format!("the update command did not install the bundle (it exited {})", one("UPDATE-EXIT").unwrap_or_else(|| "with nothing said".into())));
    }
    if one("CURRENT").as_deref() != Some(new_dir.as_str()) {
        out.push(format!("`current` names {}, not {new_dir}", one("CURRENT").unwrap_or_else(|| "nothing".into())));
    }
    let releases = one("RELEASES").unwrap_or_default();
    for d in [&old_dir, &new_dir] {
        if !releases.split_whitespace().any(|x| x == d) {
            out.push(format!("/opt/veredas holds no {d}"));
        }
    }
    let exes = marked(&p.console, "EXE");
    for b in ["veredas-maquinar", "veredas-supervisao"] {
        let want = format!("/opt/veredas/{new_dir}/bin/{b}");
        if !exes.contains(&want) {
            out.push(format!("no running {b} is the new release's ({want}): {exes:?}"));
        }
    }
    if exes.iter().any(|e| !e.starts_with(&format!("/opt/veredas/{new_dir}/"))) {
        out.push(format!("an executable of another release still runs: {exes:?}"));
    }
    let check = marked(&p.console, "CHECK");
    for line in [format!("VEREDAS-SUMS-OK {newer}"), "VEREDAS-ON-PATH ok".to_string(), "VEREDAS-ATUALIZAR ok".to_string()] {
        if !check.contains(&line) {
            out.push(format!("the boot's check on the new release does not say `{line}`"));
        }
    }
    match (one("PLANT-BEFORE"), one("PLANT-AFTER")) {
        (Some(b), Some(a)) if b == a && b.len() == 64 => {}
        (b, a) => out.push(format!("the plant's files changed or were not read ({} before, {} after)", b.unwrap_or_default(), a.unwrap_or_default())),
    }
    if one("AGAIN-EXIT").as_deref() != Some("2") {
        out.push(format!("the same bundle was not refused a second time (it exited {})", one("AGAIN-EXIT").unwrap_or_else(|| "with nothing said".into())));
    }
    out.extend(crate::together::plant_faults(&p.plant));
    if p.before != p.after {
        out.push("the update's boot changed the shipped disk".into());
    }
    out
}

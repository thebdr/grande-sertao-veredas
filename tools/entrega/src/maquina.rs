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
cat > /etc/systemd/system/veredas-verifica.service <<'UNIT'
[Unit]
Description=Veredas: the release's files checked against its SHA256SUMS

[Service]
Type=oneshot
WorkingDirectory=/opt/veredas/current
ExecStart=/bin/sh -c 'if sha256sum --quiet -c SHA256SUMS; then echo "VEREDAS-SUMS-OK {release}"; else echo "VEREDAS-SUMS-FAILED {release}"; fi'
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
Exec=chromium --no-first-run --no-default-browser-check --password-store=basic http://127.0.0.1:{port}/s/loop/
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

/// Where a machine's files are made.
pub struct Paths {
    pub ova: PathBuf,
    pub vmdk: PathBuf,
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
    vm::run(
        "tar",
        &["--format=ustar", "--owner=0", "--group=0", "--numeric-owner", "-cf", &ova.display().to_string(), "-C", &work.display().to_string(), &ovf_name, &mf_name, &vmdk_name],
        None,
    )?;
    for f in [&disk, &in_tar] {
        let _ = std::fs::remove_file(f);
    }
    let _ = std::fs::remove_dir_all(&input);
    Ok(Paths { ova, vmdk })
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

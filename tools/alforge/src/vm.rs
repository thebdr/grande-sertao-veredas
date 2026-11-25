//! A virtual machine under QEMU with KVM, run as the user (docs/alforge-plan.md, "What this machine
//! can build"): a system disk, a NoCloud seed, data disks holding tar streams, the serial console
//! written to a file, ports forwarded from the host's loopback. The traps it answers: the system
//! disk always boots first (`bootindex=0`), and every wait has a bound.

use std::io::Read;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VmError {
    #[error("{program} could not run: {source}")]
    Spawn { program: String, source: std::io::Error },
    #[error("{program} failed ({status}): {stderr}")]
    Failed { program: String, status: String, stderr: String },
    #[error("the machine did not {what} within {secs} s; the console's last lines:\n{tail}")]
    Timeout { what: String, secs: u64, tail: String },
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Which virtual hardware the machine sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hardware {
    /// virtio disks and network: the fastest under QEMU, for the builder and the provisioning.
    Virtio,
    /// VMware's paravirtual SCSI controller and VMXNET3 card on an i440BX-like board, what the
    /// OVA declares: the proof boot of the shipped disk.
    Vmware,
}

/// A data disk: a file the guest sees as a block device under `/dev/disk/by-id/virtio-<serial>`.
#[derive(Clone, Debug)]
pub struct DataDisk {
    pub file: PathBuf,
    pub serial: String,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub disk: PathBuf,
    pub hardware: Hardware,
    pub cpus: u32,
    pub memory_mb: u32,
    /// A NoCloud seed ISO (volume `cidata`), on an IDE CD.
    pub seed: Option<PathBuf>,
    pub data: Vec<DataDisk>,
    /// (port on the host's loopback, port in the guest).
    pub forwards: Vec<(u16, u16)>,
    /// Copy-on-write: nothing the guest writes reaches `disk`.
    pub snapshot: bool,
    /// The network card's address; QEMU's own default when absent.
    pub mac: Option<String>,
    /// A port on the host's loopback where QEMU's monitor (QMP) listens, for a screen dump.
    pub qmp: Option<u16>,
    pub console: PathBuf,
}

/// A running machine.
pub struct Vm {
    child: Child,
    console: PathBuf,
}

/// The arguments QEMU is run with for a spec.
pub fn args(spec: &Spec) -> Vec<String> {
    let mut a: Vec<String> = vec!["-accel", "kvm", "-cpu", "host", "-nodefaults", "-no-user-config"]
        .into_iter()
        .map(String::from)
        .collect();
    let disk = spec.disk.display().to_string();
    let snapshot = if spec.snapshot { ",snapshot=on" } else { "" };
    let format = if spec.disk.extension().is_some_and(|e| e == "vmdk") { "vmdk" } else { "qcow2" };
    a.push("-machine".into());
    a.push("pc".into());
    match spec.hardware {
        Hardware::Virtio => {
            a.push("-drive".into());
            a.push(format!("file={disk},if=none,id=sys,format={format},discard=unmap,detect-zeroes=unmap{snapshot}"));
            a.push("-device".into());
            a.push("virtio-blk-pci,drive=sys,bootindex=0".into());
        }
        Hardware::Vmware => {
            a.push("-device".into());
            a.push("pvscsi,id=scsi0".into());
            a.push("-drive".into());
            a.push(format!("file={disk},if=none,id=sys,format={format},discard=unmap,detect-zeroes=unmap{snapshot}"));
            a.push("-device".into());
            a.push("scsi-hd,drive=sys,bus=scsi0.0,bootindex=0".into());
        }
    }
    if let Some(seed) = &spec.seed {
        a.push("-drive".into());
        a.push(format!("file={},if=none,id=seed,format=raw,media=cdrom,readonly=on", seed.display()));
        a.push("-device".into());
        a.push("ide-cd,drive=seed".into());
    }
    for (i, d) in spec.data.iter().enumerate() {
        let ro = if d.writable { "" } else { ",readonly=on" };
        a.push("-drive".into());
        a.push(format!("file={},if=none,id=data{i},format=raw{ro}", d.file.display()));
        a.push("-device".into());
        a.push(format!("virtio-blk-pci,drive=data{i},serial={}", d.serial));
    }
    let fwd: String = spec.forwards.iter().map(|(h, g)| format!(",hostfwd=tcp:127.0.0.1:{h}-:{g}")).collect();
    a.push("-netdev".into());
    a.push(format!("user,id=net0{fwd}"));
    a.push("-device".into());
    let mac = spec.mac.as_ref().map(|m| format!(",mac={m}")).unwrap_or_default();
    a.push(match spec.hardware {
        Hardware::Virtio => format!("virtio-net-pci,netdev=net0{mac}"),
        Hardware::Vmware => format!("vmxnet3,netdev=net0{mac}"),
    });
    a.extend(["-smp", &spec.cpus.to_string(), "-m", &spec.memory_mb.to_string()].map(String::from));
    a.extend(["-vga", "std", "-display", "none", "-monitor", "none", "-no-reboot"].map(String::from));
    if let Some(p) = spec.qmp {
        a.push("-qmp".into());
        a.push(format!("tcp:127.0.0.1:{p},server=on,wait=off"));
    }
    a.push("-serial".into());
    a.push(format!("file:{}", spec.console.display()));
    a
}

impl Vm {
    /// Starts the machine once the host has the memory for it ([`wait_for_memory`]); QEMU's own
    /// temporary files (a copy-on-write boot's overlay) go to the cache, never to `/tmp`.
    pub fn start(spec: &Spec) -> Result<Vm, VmError> {
        wait_for_memory(AVAILABLE_BEFORE_A_MACHINE, Duration::from_secs(30 * 60))?;
        let _ = std::fs::remove_file(&spec.console);
        let tmp = crate::cache::dir()?.join("tmp");
        std::fs::create_dir_all(&tmp)?;
        let child = Command::new("qemu-system-x86_64")
            .args(args(spec))
            .env("TMPDIR", &tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| VmError::Spawn { program: "qemu-system-x86_64".into(), source })?;
        Ok(Vm { child, console: spec.console.clone() })
    }

    /// What the serial console holds so far, carriage returns dropped.
    pub fn console(&self) -> String {
        read_console(&self.console)
    }

    fn tail(&self) -> String {
        tail(&self.console(), 30)
    }

    /// Waits until the console holds `line`, the machine still running.
    pub fn wait_for(&mut self, line: &str, within: Duration) -> Result<(), VmError> {
        let start = Instant::now();
        loop {
            if self.console().contains(line) {
                return Ok(());
            }
            if let Some(status) = self.child.try_wait()? {
                return Err(VmError::Failed { program: "qemu-system-x86_64".into(), status: status.to_string(), stderr: self.stderr() + &self.tail() });
            }
            if start.elapsed() > within {
                return Err(VmError::Timeout { what: format!("print `{line}`"), secs: within.as_secs(), tail: self.tail() });
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// Waits for the machine to power itself off.
    pub fn wait_off(&mut self, within: Duration) -> Result<ExitStatus, VmError> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait()? {
                return Ok(status);
            }
            if start.elapsed() > within {
                let tail = self.tail();
                let _ = self.child.kill();
                let _ = self.child.wait();
                return Err(VmError::Timeout { what: "power off".into(), secs: within.as_secs(), tail });
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn stderr(&mut self) -> String {
        let mut s = String::new();
        if let Some(e) = self.child.stderr.as_mut() {
            let _ = e.read_to_string(&mut s);
        }
        s
    }

    /// Ends the machine at once (a copy-on-write boot loses nothing by it).
    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Asks QEMU's monitor at `port` to write the screen as a PPM file at `file`.
pub fn screendump(port: u16, file: &Path) -> Result<(), VmError> {
    use std::io::{BufRead, BufReader, Write};
    let s = std::net::TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut r = BufReader::new(s.try_clone()?);
    let mut w = s;
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut ask = |cmd: String| -> Result<String, VmError> {
        w.write_all(cmd.as_bytes())?;
        loop {
            let mut l = String::new();
            if r.read_line(&mut l)? == 0 {
                return Err(VmError::Io(std::io::Error::other("QMP closed")));
            }
            if l.contains("\"return\"") || l.contains("\"error\"") {
                return Ok(l);
            }
        }
    };
    ask("{\"execute\":\"qmp_capabilities\"}\n".into())?;
    let p = file.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    let answer = ask(format!("{{\"execute\":\"screendump\",\"arguments\":{{\"filename\":\"{p}\"}}}}\n"))?;
    if answer.contains("\"error\"") {
        return Err(VmError::Failed { program: "QMP screendump".into(), status: "error".into(), stderr: answer });
    }
    Ok(())
}

/// The memory the host must have available before a machine starts: under it the development
/// machine is short for its other work (the orchestrator's rule of 2026-11-22: under 12 GB, wait).
pub const AVAILABLE_BEFORE_A_MACHINE: u64 = 12 << 30;

/// The host's available memory in bytes, `MemAvailable` of `/proc/meminfo`.
pub fn available_memory() -> Result<u64, VmError> {
    let text = std::fs::read_to_string("/proc/meminfo")?;
    memory_available_in(&text).ok_or_else(|| VmError::Io(std::io::Error::other("/proc/meminfo holds no MemAvailable")))
}

/// `MemAvailable` of a `/proc/meminfo` text, in bytes; `None` when it holds none.
pub fn memory_available_in(meminfo: &str) -> Option<u64> {
    meminfo
        .lines()
        .find_map(|l| l.strip_prefix("MemAvailable:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
}

/// Waits until the host has `bytes` available, checking every 30 s, and refuses after `within`.
pub fn wait_for_memory(bytes: u64, within: Duration) -> Result<(), VmError> {
    let start = Instant::now();
    loop {
        let now = available_memory()?;
        if now >= bytes {
            return Ok(());
        }
        if start.elapsed() > within {
            return Err(VmError::Failed {
                program: "the machine's start".into(),
                status: "refused".into(),
                stderr: format!("the host has {} MB available, a machine waits for {} MB", now >> 20, bytes >> 20),
            });
        }
        eprintln!("alforge: {} MB available, waiting for {} MB before a machine starts", now >> 20, bytes >> 20);
        std::thread::sleep(Duration::from_secs(30));
    }
}

/// Holds the machine's lock for compiling the database engine (`/tmp/veredas-engine.lock`, the
/// one `flock` takes): one build of the engine at a time on the host, a builder machine's included.
pub fn engine_lock() -> Result<std::fs::File, VmError> {
    eprintln!("alforge: waiting for the engine's lock ({ENGINE_LOCK})");
    lock(Path::new(ENGINE_LOCK))
}

/// The host's lock for compiling the database engine, which `flock` takes too (CLAUDE.md).
pub const ENGINE_LOCK: &str = "/tmp/veredas-engine.lock";

/// The exclusive lock on the file at `path`, the one `flock` takes, waited for and held until the
/// file handed back is dropped.
pub fn lock(path: &Path) -> Result<std::fs::File, VmError> {
    let f = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    f.lock()?;
    Ok(f)
}

/// A console file's text, carriage returns dropped, invalid UTF-8 replaced.
pub fn read_console(p: &Path) -> String {
    std::fs::read(p).map(|b| String::from_utf8_lossy(&b).replace('\r', "")).unwrap_or_default()
}

/// The last `n` lines of a text.
pub fn tail(text: &str, n: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

/// A port free on the loopback now (the system's choice; free when asked, as any port is).
pub fn free_port() -> Result<u16, std::io::Error> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// Runs a program to its end; its stdout when it succeeded.
pub fn run(program: &str, args: &[&str], dir: Option<&Path>) -> Result<String, VmError> {
    let mut c = Command::new(program);
    c.args(args).stdin(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c.output().map_err(|source| VmError::Spawn { program: program.into(), source })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(VmError::Failed {
            program: format!("{program} {}", args.join(" ")),
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// `program` with the variables that change what tar and gzip write removed.
pub fn clean_command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.env_remove("TAR_OPTIONS").env_remove("GZIP").env_remove("GZIP_OPT");
    c
}

/// [`run`], without the variables that change what tar and gzip write (`TAR_OPTIONS`, `GZIP`,
/// `GZIP_OPT`): an archive packed twice from the same files is the same bytes whatever the shell holds.
pub fn run_clean(program: &str, args: &[&str], dir: Option<&Path>) -> Result<String, VmError> {
    let mut c = clean_command(program);
    c.args(args).stdin(Stdio::null());
    if let Some(d) = dir {
        c.current_dir(d);
    }
    let out = c.output().map_err(|source| VmError::Spawn { program: program.into(), source })?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(VmError::Failed {
            program: format!("{program} {}", args.join(" ")),
            status: out.status.to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// A copy-on-write disk over `base`, of `size_gb` gigabytes.
pub fn overlay(base: &Path, disk: &Path, size_gb: u32) -> Result<(), VmError> {
    let _ = std::fs::remove_file(disk);
    let fmt = if base.extension().is_some_and(|e| e == "vmdk") { "vmdk" } else { "qcow2" };
    run(
        "qemu-img",
        &["create", "-q", "-f", "qcow2", "-F", fmt, "-b", &base.display().to_string(), &disk.display().to_string(), &format!("{size_gb}G")],
        None,
    )?;
    Ok(())
}

/// A NoCloud seed: `user-data` and `meta-data` written into an ISO labelled `cidata` by xorriso.
pub fn seed(dir: &Path, iso: &Path, user_data: &str, instance_id: &str) -> Result<(), VmError> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("user-data"), user_data)?;
    std::fs::write(dir.join("meta-data"), format!("instance-id: {instance_id}\nlocal-hostname: veredas\n"))?;
    let _ = std::fs::remove_file(iso);
    run("xorriso", &["-as", "mkisofs", "-quiet", "-o", &iso.display().to_string(), "-V", "cidata", "-J", "-r", &dir.display().to_string()], None)?;
    Ok(())
}

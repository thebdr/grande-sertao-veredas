//! A private network for a test (Linux): a user and network namespace of the test's own, where it
//! holds `CAP_NET_RAW` and `CAP_NET_ADMIN` over that namespace only, with the veth pairs it asks
//! for. Raw frames cross the kernel's real Ethernet path; nothing of the host is touched; the
//! namespace and its links vanish when the test's process ends. No privilege is needed where the
//! system lets an unprivileged process make a user namespace (`docs/aranha-plan.md`, "Traps").
//!
//! A test calls [`isolated`] with its own name. The test's process is multi-threaded, and
//! `unshare(CLONE_NEWUSER)` refuses a multi-threaded process, so the namespace is made in a child:
//! the test binary run again for that one test, unshared between `fork` and `exec` and mapped to
//! root inside, where the same call runs the body. The parent waits, and fails with the child's
//! own output when the child fails or the body never ran (a misspelled name runs no test).
//!
//! Where the namespace cannot be made, the test fails and says why by name, with what the system
//! says of user namespaces: never a skip, never a pass.

use std::ffi::CString;
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;

/// The environment variable that marks the child run, holding the test's name.
pub const INNER: &str = "VEREDAS_NETNS_INNER";

/// The line the child prints once the body has run, followed by the test's name.
const RAN: &str = "veredas-netns: the body ran in the private network: ";

/// Run `body` in a private network holding the veth `pairs` (each end up), as the test named
/// `test` (its name as the test binary knows it, for `--exact`). Panics, naming why, when the
/// network cannot be made, when the body fails, or when the body never ran. In the child it never
/// returns: once the body has run the child exits, so what the test does after this call runs on
/// the host alone.
pub fn isolated<F: FnOnce()>(test: &str, pairs: &[(&str, &str)], body: F) {
    if std::env::var(INNER).as_deref() == Ok(test) {
        prepare(pairs);
        body();
        // On a line of its own: libtest has written "test <name> ... " without a line break.
        println!("\n{RAN}{test}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        std::process::exit(0);
    }
    // The name is the calling test's own (libtest names a test's thread after it), and a test
    // makes one private network: a copied test with another's name, or a second call, would run
    // a body that is not this one, or none.
    let own = std::thread::current().name().map(str::to_string);
    if own.as_deref() != Some(test) {
        panic!("netns::isolated({test:?}) was called from the test {own:?}: a test passes its own name");
    }
    thread_local! {
        static MADE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }
    if MADE.with(|m| m.replace(true)) {
        panic!("{test} asked for a second private network: one per test");
    }
    let out = match run_child(test) {
        Ok(out) => out,
        Err(why) => panic!("{why}"),
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let ran = stdout.lines().any(|l| l.trim_end() == format!("{RAN}{test}"));
    if !out.status.success() || !ran {
        let what = if out.status.success() { "the body never ran (is the test's name exact?)" } else { "the body failed" };
        panic!("{test} in its private network: {what} ({})\n--- its stdout ---\n{stdout}\n--- its stderr ---\n{stderr}", out.status);
    }
}

/// Run the test binary again for the test named `test` in a private network of its own, and hand
/// back what it printed; where the network cannot be made, why, with what the system says of
/// user namespaces.
pub fn run_child(test: &str) -> Result<std::process::Output, String> {
    child(test).output().map_err(|e| format!("the private network for {test} could not be made: {e}\n{}", diagnosis()))
}

/// The test binary, run again for one test, unshared into a user and network namespace of its
/// own between `fork` and `exec`, with this user mapped to root inside.
fn child(test: &str) -> Command {
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("the test binary's own path: {e}"));
    // Everything the forked child touches is made here, before the fork: after it, in a process
    // whose parent has other threads, nothing may allocate.
    // SAFETY: getuid and getgid cannot fail.
    let (uid, gid) = unsafe { (libc::getuid(), libc::getgid()) };
    let files = [
        (c("/proc/self/setgroups"), b"deny".to_vec()),
        (c("/proc/self/uid_map"), format!("0 {uid} 1").into_bytes()),
        (c("/proc/self/gid_map"), format!("0 {gid} 1").into_bytes()),
    ];
    let mut cmd = Command::new(exe);
    cmd.args(["--exact", test, "--nocapture", "--test-threads", "1"]).env(INNER, test);
    // SAFETY: the closure calls only async-signal-safe functions (unshare, open, write, close)
    // on data prepared before the fork, and allocates nothing.
    unsafe {
        cmd.pre_exec(move || {
            if libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) != 0 {
                return Err(io::Error::last_os_error());
            }
            for (path, content) in &files {
                let fd = libc::open(path.as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
                if fd < 0 {
                    return Err(io::Error::last_os_error());
                }
                let n = libc::write(fd, content.as_ptr().cast(), content.len());
                let err = io::Error::last_os_error();
                libc::close(fd);
                if n != content.len() as isize {
                    return Err(err);
                }
            }
            Ok(())
        });
    }
    cmd
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap_or_default()
}

/// Inside: loopback up, IPv6 off (so the links carry only what the test sends), then each veth
/// pair made and both ends up.
fn prepare(pairs: &[(&str, &str)]) {
    for knob in ["all", "default"] {
        // Best effort: a kernel without IPv6 has no such file, and the links work the same.
        let _ = std::fs::write(format!("/proc/sys/net/ipv6/conf/{knob}/disable_ipv6"), "1");
    }
    ip(&["link", "set", "lo", "up"]);
    for (a, b) in pairs {
        ip(&["link", "add", a, "type", "veth", "peer", "name", b]);
        ip(&["link", "set", a, "up"]);
        ip(&["link", "set", b, "up"]);
    }
    wait_pairs(pairs);
}

/// The EtherType of the fixture's own probes (IEEE 802's second local experimental type), which no
/// test sends.
pub const PROBE: u16 = 0x88B6;

/// Wait until each pair carries a frame each way: its carriers up, then a numbered probe sent from
/// each end until the other end hears the latest one sent (5 s at most, failing by name). A carrier
/// shows up before the end raised first sends what it is given: a frame sent at once from it can be
/// lost (measured: about one in ten), never after this.
pub fn wait_pairs(pairs: &[(&str, &str)]) {
    let ends: Vec<&str> = pairs.iter().flat_map(|(a, b)| [*a, *b]).collect();
    wait_up(&ends);
    for (a, b) in pairs {
        carries(a, b);
        carries(b, a);
    }
}

fn carries(from: &str, to: &str) {
    use crate::profinet::mac::Mac;
    use crate::wire::raw::{Options, RawError, RawLink};
    use std::time::{Duration, Instant};
    let rx = RawLink::open_with(to, Options { ethertype: Some(PROBE), promiscuous: false, outgoing: false })
        .unwrap_or_else(|e| panic!("the probe's listener on {to}: {e}"));
    let tx = RawLink::open(from, false).unwrap_or_else(|e| panic!("the probe's sender on {from}: {e}"));
    let end = Instant::now() + Duration::from_secs(5);
    let mut k: u32 = 0;
    loop {
        k += 1;
        let mut f = Vec::with_capacity(60);
        f.extend_from_slice(&Mac::BROADCAST.0);
        f.extend_from_slice(&tx.mac().0);
        f.extend_from_slice(&PROBE.to_be_bytes());
        f.extend_from_slice(b"veredas-netns probe ");
        f.extend_from_slice(&k.to_be_bytes());
        f.resize(60, 0);
        // A full queue loses the probe as a lost carrier would: it is sent again.
        match tx.send(&f) {
            Ok(()) | Err(RawError::QueueFull(_)) => {}
            Err(e) => panic!("the probe from {from}: {e}"),
        }
        // Probes are 50 ms apart, so an earlier one is heard or lost before the next is sent.
        let wait_end = Instant::now() + Duration::from_millis(50);
        while let Some(r) = rx.recv(wait_end.saturating_duration_since(Instant::now())).unwrap_or_else(|e| panic!("the probe on {to}: {e}")) {
            if r.bytes.get(34..38) == Some(&k.to_be_bytes()[..]) {
                return;
            }
        }
        if Instant::now() > end {
            panic!("no frame crossed from {from} to {to} in 5 s ({k} probes sent)");
        }
    }
}

/// Wait until each interface has its carrier (`LOWER_UP`): a frame sent the moment a veth end is
/// set up can be lost while its carrier comes up. Fails, naming the interface, after 5 s.
pub fn wait_up(ifaces: &[&str]) {
    let end = std::time::Instant::now() + std::time::Duration::from_secs(5);
    for iface in ifaces {
        loop {
            let out = Command::new(ip_tool()).args(["-o", "link", "show", iface]).output();
            let flags = out.map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
            if flags.split(['<', '>', ',']).any(|f| f == "LOWER_UP") {
                break;
            }
            if std::time::Instant::now() > end {
                panic!("{iface} has no carrier after 5 s: {flags}");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

/// Run `ip` with these arguments inside the private network, failing with its own words and what
/// the system says of user namespaces when it fails.
pub fn ip(args: &[&str]) {
    let out = Command::new(ip_tool()).args(args).output().unwrap_or_else(|e| panic!("ip {args:?}: {e}"));
    if !out.status.success() {
        panic!("ip {}: {}\n{}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim(), diagnosis());
    }
}

/// iproute2's `ip`, wherever this system keeps it.
fn ip_tool() -> &'static str {
    ["ip", "/usr/sbin/ip", "/sbin/ip", "/usr/bin/ip", "/bin/ip"]
        .into_iter()
        .find(|p| Command::new(p).arg("-V").output().map(|o| o.status.success()).unwrap_or(false))
        .unwrap_or_else(|| panic!("the private network needs iproute2's `ip`, and none runs here\n{}", diagnosis()))
}

/// What the system says of unprivileged user namespaces, for a failure's message.
fn diagnosis() -> String {
    let read = |p: &str| std::fs::read_to_string(p).map(|s| s.trim().to_string()).unwrap_or_else(|_| "(absent)".into());
    format!(
        "user namespaces here: kernel.apparmor_restrict_unprivileged_userns = {}, kernel.unprivileged_userns_clone = {}, \
         user.max_user_namespaces = {}, this process's AppArmor label = {:?}. Where the AppArmor restriction is 1 and the \
         label is `unconfined`, a new user namespace holds no capability: root sets the restriction to 0 once, or grants \
         `userns` to the test binaries in an AppArmor profile (docs/aranha-plan.md, \"Traps\").",
        read("/proc/sys/kernel/apparmor_restrict_unprivileged_userns"),
        read("/proc/sys/kernel/unprivileged_userns_clone"),
        read("/proc/sys/user/max_user_namespaces"),
        read("/proc/self/attr/current"),
    )
}

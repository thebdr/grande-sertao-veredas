#!/usr/bin/python3
"""veredas-atualizar: install a newer suite release on this tester machine.

    sudo veredas-atualizar <bundle.tar.gz>

The bundle is a release's archive as the cut packs it (docs/alforge-plan.md, "Delivery and
updates"), a file given by path: the tester copies it into the machine. It is installed only if
every one of its files matches its SHA256SUMS, its manifest is a `veredas-release` of this
machine's target cut for real (never a trial's), newer than the installed release and naming
exactly the executables it holds, every executable finds its libraries here, and the disk has the
room. It then goes beside the installed release under /opt/veredas/, `current` is moved to it in
one step, the PATH's links follow, and the bench and supervisão restart on it and are seen to stay
up. The plant's files in the home folder are left as they are; the old release stays where it was.
Anything refused, an I/O error included, leaves the machine as it was; what the update could not
remove of its own is said, and is no refusal "as it was" (exit 4). What a run killed midway left
(its staging folder, a scratch link) is removed at the next run's start, under the lock.

Interrupted (Ctrl-C, a closed terminal, a shutdown) before the install, it is a refusal, the
machine as it was; the install itself holds an interruption until it is done; interrupted while it
watches the services, it says the way back (exit 3).

Exit 0 when installed, 2 when refused (the reasons of the first check that fails said: the
archive, then its sums, its manifest, its executables' libraries, the PATH), 3 when installed but
the services did not stay up on it (the way back said), 4 when a failed install could not all be
put back (what is left said, the release's folder kept), 1 on a usage error. VEREDAS_ROOT (default /) roots every
path it touches; VEREDAS_RESERVE_MB (default 64) is the room it leaves free on the disk;
VEREDAS_SETTLE (default 5) the seconds it watches the services for, twice.
"""

import fcntl
import gzip
import hashlib
import os
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = os.environ.get("VEREDAS_ROOT", "/")
OPT = os.path.join(ROOT, "opt/veredas")
CURRENT = os.path.join(OPT, "current")
PATH_DIR = os.path.join(ROOT, "usr/local/bin")
# What a link on the PATH names: the executable through `current`, as the provisioning makes it.
THROUGH = "/opt/veredas/current/bin/"
SERVICES = ["veredas-bancada.service", "veredas-supervisao.service"]
RESERVE = int(os.environ.get("VEREDAS_RESERVE_MB", "64")) << 20
SETTLE = float(os.environ.get("VEREDAS_SETTLE", "5"))
# What a trial cut's manifest says it ran (alforge's `cut::NOT_RUN`): a trial is never installed.
TRIAL = "not run: a trial cut"


class Broken(Exception):
    """An install that failed and could not be put back: what is left, said as it stands."""

    def __init__(self, reasons):
        super().__init__("\n".join(reasons))
        self.reasons = reasons


class Alone(Exception):
    """A refusal said in its own line, without the heading (the bundle not there)."""

    def __init__(self, reasons):
        super().__init__("\n".join(reasons))
        self.reasons = reasons


BROKEN = "not installed, and the machine could not all be put back as it was:"


class Stopped(BaseException):
    """The run interrupted (Ctrl-C, a closed terminal, a shutdown), as a refusal before the install
    and as no verdict after it: a BaseException, so no reading's own `except Exception` takes it
    for a damaged bundle."""

    def __init__(self, name):
        super().__init__(name)
        self.name = name


# The signals a tester's interruption sends: each a `Stopped`. The install itself (a rename and a
# few links) holds them until it is done, so no run stops halfway through it.
STOPS = [signal.SIGINT, signal.SIGTERM, signal.SIGHUP]


# The signal that stopped this run, once taken: a shutdown sends SIGTERM and SIGHUP together, and
# Python runs each one's handler in turn; the second is ignored, so the first one's refusal or way
# back is said whole.
STOPPED_BY = []


def stopping(signum, _frame):
    if STOPPED_BY:
        return
    STOPPED_BY.append(signum)
    signal.pthread_sigmask(signal.SIG_BLOCK, STOPS)
    raise Stopped(signal.Signals(signum).name)


class Refused(Exception):
    def __init__(self, reasons):
        super().__init__("\n".join(reasons))
        self.reasons = reasons


def semver(text):
    """A SemVer's precedence key, or None for a text that is no SemVer."""
    core, _, _build = text.partition("+")
    core, dash, pre = core.partition("-")
    nums = core.split(".")
    if len(nums) != 3 or not all(n.isdigit() and (n == "0" or not n.startswith("0")) for n in nums):
        return None
    if dash and (not pre or any(p == "" for p in pre.split("."))):
        return None
    ids = []
    for p in pre.split(".") if dash else []:
        ids.append((0, int(p), "") if p.isdigit() else (1, 0, p))
    # A release without a pre-release ranks above every one with it.
    return (tuple(int(n) for n in nums), 1 if not dash else 0, tuple(ids))


def manifest(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, ValueError) as e:
        raise Refused([f"{os.path.basename(path)} does not read: {e}"])


def unpack(bundle, stage):
    """The archive extracted into `stage`: its one folder at the top, holding files and folders
    alone, every path inside it."""
    # The whole compressed stream read first, so gzip's own CRC and length judge every byte (tarfile
    # stops at the archive's end and never reaches them); whatever reading it raises is its refusal
    # (a corrupt stream may raise zlib's own error from a seek tarfile does not wrap).
    try:
        with gzip.open(bundle, "rb") as g:
            while g.read(1 << 20):
                pass
    except Exception as e:
        raise Refused([f"the bundle is no release bundle (a gzipped tar): {e}"])
    try:
        tf = tarfile.open(bundle, "r:gz")
    except Exception as e:
        raise Refused([f"the bundle is no release bundle (a gzipped tar): {e}"])
    with tf:
        try:
            members = tf.getmembers()
        except Exception as e:
            raise Refused([f"the bundle does not read to its end: {e}"])
        problems, tops = [], set()
        for m in members:
            parts = m.name.split("/")
            if m.name.startswith("/") or ".." in parts:
                problems.append(f"{m.name}: a path outside the bundle's folder")
            if not (m.isfile() or m.isdir()):
                problems.append(f"{m.name}: neither a file nor a folder")
            tops.add(parts[0])
        if len(tops) != 1:
            problems.append(f"the bundle holds {len(tops)} folders at its top, not one")
        if problems:
            raise Refused(problems)
        need = sum(m.size for m in members if m.isfile())
        free = shutil.disk_usage(OPT).free
        if need + RESERVE > free:
            raise Refused([f"the bundle needs {need >> 20} MB under {OPT} and {RESERVE >> 20} MB left free; {free >> 20} MB are free"])
        try:
            tf.extractall(stage, filter="data")
        except Exception as e:
            raise Refused([f"the bundle does not read to its end: {e}"])
    return tops.pop()


def check_sums(folder):
    listed = {}
    try:
        with open(os.path.join(folder, "SHA256SUMS"), encoding="utf-8") as f:
            for line in f:
                digest, _, name = line.rstrip("\n").partition("  ")
                listed[name] = digest
    except (OSError, ValueError) as e:
        raise Refused([f"the bundle holds no readable SHA256SUMS: {e}"])
    present = []
    for d, _, files in os.walk(folder):
        for name in files:
            rel = os.path.relpath(os.path.join(d, name), folder)
            if rel != "SHA256SUMS":
                present.append(rel)
    wrong = []
    for rel in sorted(present):
        if rel not in listed:
            wrong.append(f"{rel} is not in SHA256SUMS")
            continue
        with open(os.path.join(folder, rel), "rb") as f:
            if hashlib.sha256(f.read()).hexdigest() != listed[rel]:
                wrong.append(f"{rel} does not match its sum")
    for rel in sorted(set(listed) - set(present)):
        wrong.append(f"{rel} is in SHA256SUMS and not in the bundle")
    if wrong:
        raise Refused(wrong)


def check_manifest(folder, top, installed):
    m = manifest(os.path.join(folder, "release.toml"))
    wrong = []
    if m.get("format") != "veredas-release" or m.get("version") != 1:
        raise Refused([f"release.toml is `{m.get('format')}` version {m.get('version')}, not `veredas-release` version 1"])
    missing = [k for k in ["release", "commit", "target", "toolchain", "component", "tested"] if k not in m]
    if missing:
        raise Refused([f"release.toml states no `{k}`" for k in missing])
    release, target = str(m.get("release", "")), str(m.get("target", ""))
    if semver(release) is None:
        raise Refused([f"release.toml's release `{release}` is no SemVer"])
    # The manifest's shape, as every cut writes it: the keys the command reads, each of its kind.
    tested = m["tested"]
    shape = []
    if not isinstance(tested, dict) or not all(isinstance(tested.get(k), str) and tested.get(k).strip() for k in ["suite", "together"]):
        shape.append("release.toml's [tested] does not state its `suite` and `together`")
    components = m["component"]
    if not isinstance(components, list) or not components:
        shape.append("release.toml states no `[[component]]`")
    else:
        for c in components:
            if not (isinstance(c, dict) and isinstance(c.get("name"), str) and isinstance(c.get("version"), str) and semver(c["version"]) is not None
                    and isinstance(c.get("bins", []), list) and all(isinstance(b, str) for b in c.get("bins", []))):
                shape.append(f"release.toml's component `{c.get('name') if isinstance(c, dict) else c}` is not a name, a SemVer version and its executables")
    if shape:
        raise Refused(shape)
    if tested["suite"].startswith("not run"):
        wrong.append(f"the bundle is a trial cut's (its suite `{tested['suite']}`): only a release cut for real is installed")
    if target != installed["target"]:
        wrong.append(f"the bundle is built for {target}, this machine runs {installed['target']}")
    if top != f"veredas-{release}-{target}":
        wrong.append(f"the bundle's folder is {top}, not veredas-{release}-{target}")
    if semver(release) <= semver(str(installed["release"])):
        wrong.append(f"release {release} is not newer than the installed {installed['release']}")
    listed = [b for c in m.get("component", []) for b in c.get("bins", [])]
    wrong += [f"bin/{b} is named by more than one component" for b in sorted({b for b in listed if listed.count(b) > 1})]
    named = set(listed)
    held = set(os.listdir(os.path.join(folder, "bin"))) if os.path.isdir(os.path.join(folder, "bin")) else set()
    wrong += [f"bin/{b} is named by the manifest and not in the bundle" for b in sorted(named - held)]
    wrong += [f"bin/{b} is in the bundle and no component ships it" for b in sorted(held - named)]
    if os.path.lexists(os.path.join(OPT, top)):
        wrong.append(f"{os.path.join(OPT, top)} is there already")
    if wrong:
        raise Refused(wrong)
    return m


def check_starts(folder):
    """Every executable finds its libraries, at their versions, on this machine (`ldd`)."""
    wrong = []
    for b in sorted(os.listdir(os.path.join(folder, "bin"))):
        out = subprocess.run(["ldd", os.path.join(folder, "bin", b)], capture_output=True, text=True)
        missing = [line.strip() for line in (out.stdout + out.stderr).splitlines() if "not found" in line]
        if missing:
            wrong.append(f"bin/{b} would not start here: {'; '.join(missing)}")
    if wrong:
        raise Refused(wrong)


def leftovers():
    """What an earlier run killed midway left (a reboot, a closed terminal), removed under the lock,
    which proves no run owns it: its staging folder `/opt/veredas/.atualizar-*` and its scratch
    links `.<name>.atualizar-<pid>` beside `current` and on the PATH. Each one removed is said; one
    that cannot be is said too, and left."""
    # Only the shapes a run makes: a staging folder as `tempfile.mkdtemp` names it (eight of its
    # letters), and a scratch link `.<name>.atualizar-<pid>`; anything else is another's, and kept.
    found = []
    if os.path.isdir(OPT):
        found += [os.path.join(OPT, n) for n in sorted(os.listdir(OPT)) if re.fullmatch(r"\.atualizar-[a-z0-9_]{8}", n) and os.path.isdir(os.path.join(OPT, n)) and not os.path.islink(os.path.join(OPT, n))]
    for d in [OPT, PATH_DIR]:
        if os.path.isdir(d):
            found += [os.path.join(d, n) for n in sorted(os.listdir(d)) if re.fullmatch(r"\..+\.atualizar-[0-9]+", n) and os.path.islink(os.path.join(d, n))]
    for p in found:
        left = []
        if os.path.islink(p):
            try:
                os.unlink(p)
            except OSError as e:
                left.append(f"{p} ({e.strerror})")
        else:
            shutil.rmtree(p, onexc=lambda _f, q, x: left.append(f"{q} ({getattr(x, 'strerror', x)})"))
        if left:
            print(f"veredas-atualizar: what an earlier run left could not all be removed: {left[0]}", file=sys.stderr)
        else:
            print(f"veredas-atualizar: removed what an earlier run left: {p}")


def point(link, target):
    """`link` made to name `target` in one step: a new link under a name of this run's own, made
    (never an existing file removed for it), then renamed over it."""
    new = os.path.join(os.path.dirname(link), f".{os.path.basename(link)}.atualizar-{os.getpid()}")
    os.symlink(target, new)
    try:
        os.replace(new, link)
    except OSError:
        os.unlink(new)
        raise


def check_path(bins):
    """Each executable's place on the PATH is free or a link the releases made (through
    `current`): a file or folder of another's is never replaced."""
    wrong = []
    for b in bins:
        p = os.path.join(PATH_DIR, b)
        if os.path.lexists(p) and not (os.path.islink(p) and os.readlink(p) == THROUGH + b):
            wrong.append(f"{p} is not a link a release made: an executable of the release would replace it")
    if wrong:
        raise Refused(wrong)


def install(stage, top, old_top, bins):
    """The new release moved beside the old one, `current` moved to it, then the PATH's links: one
    per executable of the new release, none left for one it no longer has. An I/O error on the way
    puts back what was done: `current`, the links, the folder."""
    dest = os.path.join(OPT, top)
    links_before = {}
    if os.path.isdir(PATH_DIR):
        for name in os.listdir(PATH_DIR):
            p = os.path.join(PATH_DIR, name)
            if os.path.islink(p):
                links_before[name] = os.readlink(p)
    moved = False
    try:
        os.rename(os.path.join(stage, top), dest)
        moved = True
        point(CURRENT, top)
        os.makedirs(PATH_DIR, exist_ok=True)
        for b in bins:
            link = os.path.join(PATH_DIR, b)
            if not (os.path.islink(link) and os.readlink(link) == THROUGH + b):
                point(link, THROUGH + b)
        for name in sorted(os.listdir(PATH_DIR)):
            link = os.path.join(PATH_DIR, name)
            # A release's own link names its executable through `current` under its own name;
            # another link (a tester's alias) is kept.
            if os.path.islink(link) and os.readlink(link) == THROUGH + name and name not in bins:
                os.unlink(link)
    except OSError as e:
        # A link or a rename names what it was making second.
        why = f"{e.filename2 or e.filename or ''}: {e.strerror}".lstrip(": ")
        failed = []
        if old_top:
            try:
                if os.readlink(CURRENT) != old_top:
                    point(CURRENT, old_top)
            except OSError as r:
                failed.append(f"`current` could not be put back on {old_top} ({r.strerror}): it names {top}, kept in {dest}")
        for n, t in links_before.items():
            p = os.path.join(PATH_DIR, n)
            try:
                if not (os.path.islink(p) and os.readlink(p) == t):
                    point(p, t)
            except OSError as r:
                failed.append(f"{p} could not be put back on {t} ({r.strerror})")
        if os.path.isdir(PATH_DIR):
            for name in os.listdir(PATH_DIR):
                p = os.path.join(PATH_DIR, name)
                if name not in links_before and os.path.islink(p) and os.readlink(p).startswith(THROUGH):
                    try:
                        os.unlink(p)
                    except OSError as r:
                        failed.append(f"{p} could not be removed ({r.strerror})")
        if failed:
            # The folder `current` may still name is kept: nothing is deleted under a running link.
            kept = [f"the new release's folder {dest} is kept: a later update of this release is refused while it stands"] if moved else []
            raise Broken([why] + failed + kept)
        if moved:
            left = []
            shutil.rmtree(dest, onexc=lambda _f, p, x: left.append(f"{p} ({getattr(x, 'strerror', x)})"))
            if left:
                raise Broken([why, f"the new release's folder {dest} could not all be removed: {left[0]}; a later update of this release is refused while it stands (remove it with `sudo rm -rf {dest}`)"])
        raise Refused([why])


def services():
    """Each service's state and the moment its main process started, as systemd says them."""
    out = subprocess.run(["systemctl", "show", "-p", "ActiveState", "-p", "ExecMainStartTimestampMonotonic", *SERVICES], capture_output=True, text=True)
    return out.stdout.split("\n\n") if out.returncode == 0 else None


def staying_up():
    """Why the bench and supervisão did not stay up after their restart, or None: each active, and
    the same main process, at two looks `SETTLE` seconds apart (a release that refuses the kept
    plant exits, and systemd starts it again)."""
    import time
    time.sleep(SETTLE)
    first = services()
    time.sleep(SETTLE)
    second = services()
    if first is None or second is None:
        return "systemctl could not say their state"
    if any("ActiveState=active" not in s for s in first + second) or first != second:
        return "they did not stay active: " + " / ".join(" ".join(s.split()) for s in second)
    return None


def main(argv):
    if len(argv) != 2 or argv[1].startswith("-"):
        print("usage: sudo veredas-atualizar <bundle.tar.gz>", file=sys.stderr)
        return 1
    bundle = argv[1]
    # The release's folders as the archive's data filter makes its files: readable by the
    # services' user whatever umask the caller has.
    os.umask(0o022)
    # An interruption is taken from the start: before the bundle is read it stops the run with
    # nothing installed; what an earlier run or this one left, the next update removes.
    for sig in STOPS:
        signal.signal(sig, stopping)
    early = "veredas-atualizar: not installed, stopped by {} before the bundle was read; what this run or an earlier one left in /opt/veredas the next update removes"
    try:
        # One update at a time, the lock taken before anything is read: the installed release
        # and `current` are the ones this update replaces. The release's folder itself is locked,
        # so nothing is left behind.
        lock = os.open(OPT, os.O_RDONLY)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except Stopped as e:
        print(early.format(e.name), file=sys.stderr)
        return 2
    except BlockingIOError:
        print("veredas-atualizar: another update is running", file=sys.stderr)
        return 2
    except OSError as e:
        print(f"veredas-atualizar: {OPT}: {e.strerror}", file=sys.stderr)
        return 2
    try:
        installed = manifest(os.path.join(CURRENT, "release.toml"))
        old_top = os.readlink(CURRENT) if os.path.islink(CURRENT) else ""
        # The release `current` names, by its folder's name: a going back typed with a final `/`
        # (tab completion adds one) names the same folder.
        old_name = os.path.basename(os.path.normpath(old_top)) if old_top else ""
        leftovers()
        stage = tempfile.mkdtemp(prefix=".atualizar-", dir=OPT)
    except Stopped as e:
        print(early.format(e.name), file=sys.stderr)
        return 2
    except Refused as e:
        print("veredas-atualizar: the installed release does not read:\n" + str(e), file=sys.stderr)
        return 2
    except PermissionError:
        print(f"veredas-atualizar: {OPT} is not writable here: run it with sudo", file=sys.stderr)
        return 2
    # Why the update stopped: its exit code, the heading said before its reasons (none for a line
    # said alone), the reasons, and what is said after them. None once installed.
    stopped = None
    as_it_was = "not installed, the machine is as it was:"
    try:
        # An interruption from here on is a refusal: the staging folder removed, nothing installed.
        # The bundle read once, into the staging folder: its digest and its contents are of the
        # same bytes, whatever the file does meanwhile.
        copy = os.path.join(stage, "bundle.tar.gz")
        # Room for the copy first, the reserve kept (the files' own room is judged before they
        # are extracted).
        try:
            size = os.stat(bundle).st_size
        except FileNotFoundError as e:
            raise Alone([f"{bundle}: {e.strerror}"])
        free = shutil.disk_usage(OPT).free
        if size + RESERVE > free:
            raise Refused([f"the bundle's archive needs {size >> 20} MB under {OPT} and {RESERVE >> 20} MB left free; {free >> 20} MB are free"])
        h = hashlib.sha256()
        try:
            with open(bundle, "rb") as src, open(copy, "wb") as dst:
                while chunk := src.read(1 << 20):
                    h.update(chunk)
                    dst.write(chunk)
        except FileNotFoundError as e:
            raise Alone([f"{bundle}: {e.strerror}"])
        print(f"veredas-atualizar: {bundle} (SHA-256 {h.hexdigest()})")
        top = unpack(copy, stage)
        folder = os.path.join(stage, top)
        check_sums(folder)
        m = check_manifest(folder, top, installed)
        check_starts(folder)
        with open(os.path.join(folder, "SHA256SUMS"), "rb") as f:
            sums = hashlib.sha256(f.read()).hexdigest()
        print(f"veredas-atualizar: its SHA256SUMS {sums} (what the boot's check says as VEREDAS-BUNDLE)")
        bins = sorted(os.listdir(os.path.join(folder, "bin")))
        check_path(bins)
        # Held from here: the install runs whole, and what it did is judged and said, before an
        # interruption is taken (after the services' restart, as no verdict, the way back said).
        signal.pthread_sigmask(signal.SIG_BLOCK, STOPS)
        install(stage, top, old_top, bins)
    except Stopped as e:
        stopped = (2, as_it_was, [f"stopped by {e.name} before anything was installed"])
    except Alone as e:
        stopped = (2, None, e.reasons)
    except OSError as e:
        stopped = (2, as_it_was, [f"{e.filename2 or e.filename or bundle}: {e.strerror}"])
    except Refused as e:
        stopped = (2, as_it_was, e.reasons)
    except Broken as e:
        stopped = (4, BROKEN, e.reasons)
    except Exception as e:
        # Whatever else a bundle makes go wrong before it is installed is a refusal too.
        stopped = (2, as_it_was, [f"{type(e).__name__}: {e}"])
    # The staging folder removed, and judged: what it leaves is said, and a refusal that leaves it
    # is no longer "as it was". The next update removes it ([`leftovers`]). An interruption is
    # held from here through the verdict, so a refusal is said whole.
    signal.pthread_sigmask(signal.SIG_BLOCK, STOPS)
    left = []
    shutil.rmtree(stage, onexc=lambda _f, p, x: left.append(f"{p} ({getattr(x, 'strerror', x)})"))
    if left:
        line = f"its staging folder {stage} could not all be removed: {left[0]}; the next update removes it"
        if stopped is None:
            print(f"veredas-atualizar: {line}", file=sys.stderr)
        else:
            stopped = (4, BROKEN, stopped[2] + [line])
    if stopped is not None:
        code, heading, reasons = stopped
        if heading is None:
            for r in reasons:
                print(f"veredas-atualizar: {r}", file=sys.stderr)
        else:
            print(f"veredas-atualizar: {heading}", file=sys.stderr)
            for r in reasons:
                print(f"  {r}", file=sys.stderr)
        # The way back, when `current` does not name the release it named.
        if code == 4 and old_top and (not os.path.islink(CURRENT) or os.readlink(CURRENT) != old_top):
            print(f"veredas-atualizar: to go back: sudo ln -sfn {old_name} /opt/veredas/current, and the links named above", file=sys.stderr)
        return code
    old = os.path.join(OPT, old_name) if old_top else "(none)"
    print(f"veredas-atualizar: release {installed['release']} -> {m['release']} (commit {m.get('commit')})")
    print(f"veredas-atualizar: installed at {os.path.join(OPT, top)}; the old release kept at {old}")
    print("veredas-atualizar: the plant's files in the home folder left as they are; the release's own plant is in /opt/veredas/current/planta")
    print(f"veredas-atualizar: restarting the bench and supervisão on it, and watching them for {2 * SETTLE:g} s")
    sys.stdout.flush()
    # The restart run whole while an interruption is still held (one held since the install waits for
    # it), so the services run the release `current` names; then the watch, interruptible.
    restart = subprocess.run(["systemctl", "restart", *SERVICES], capture_output=True, text=True)
    failed = (restart.stderr.strip() or f"systemctl exited {restart.returncode}") if restart.returncode != 0 else None
    try:
        signal.pthread_sigmask(signal.SIG_UNBLOCK, STOPS)
        why = failed or staying_up()
    except Stopped as e:
        why = failed
        if why is None:
            back = f"sudo ln -sfn {os.path.basename(old)} /opt/veredas/current && sudo systemctl restart {' '.join(SERVICES)}"
            print(f"veredas-atualizar: installed, and stopped by {e.name} before the bench and supervisão were seen to stay up on it; to go back: {back}", file=sys.stderr)
            return 3
    if why:
        back = f"sudo ln -sfn {os.path.basename(old)} /opt/veredas/current && sudo systemctl restart {' '.join(SERVICES)}"
        print(f"veredas-atualizar: the bench and supervisão did not stay up on it ({why}); to go back: {back}", file=sys.stderr)
        return 3
    print("veredas-atualizar: the bench and supervisão restarted on it and stayed up")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

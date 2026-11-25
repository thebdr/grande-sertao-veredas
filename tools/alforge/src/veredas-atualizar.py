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
Anything refused, an I/O error included, leaves the machine as it was.

Exit 0 when installed, 2 when refused (every reason said), 3 when installed but the services did
not stay up on it (the way back said), 1 on a usage error. VEREDAS_ROOT (default /) roots every
path it touches; VEREDAS_RESERVE_MB (default 64) is the room it leaves free on the disk;
VEREDAS_SETTLE (default 5) the seconds it watches the services for, twice.
"""

import fcntl
import hashlib
import os
import shutil
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
    except (OSError, tomllib.TOMLDecodeError) as e:
        raise Refused([f"{os.path.basename(path)} does not read: {e}"])


def unpack(bundle, stage):
    """The archive extracted into `stage`: its one folder at the top, holding files and folders
    alone, every path inside it."""
    try:
        tf = tarfile.open(bundle, "r:gz")
    except (OSError, tarfile.TarError, EOFError) as e:
        raise Refused([f"{bundle} is no release bundle (a gzipped tar): {e}"])
    with tf:
        try:
            members = tf.getmembers()
        except (OSError, tarfile.TarError, EOFError) as e:
            raise Refused([f"{bundle} does not read to its end: {e}"])
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
        tf.extractall(stage, filter="data")
    return tops.pop()


def check_sums(folder):
    listed = {}
    try:
        with open(os.path.join(folder, "SHA256SUMS"), encoding="utf-8") as f:
            for line in f:
                digest, _, name = line.rstrip("\n").partition("  ")
                listed[name] = digest
    except OSError as e:
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
    if str(m["tested"].get("suite", "")).startswith("not run") if isinstance(m["tested"], dict) else True:
        wrong.append(f"the bundle is a trial cut's (its suite `{m['tested'].get('suite') if isinstance(m['tested'], dict) else m['tested']}`): only a release cut for real is installed")
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


def point(link, target):
    """`link` made to name `target` in one step (a new link renamed over it)."""
    new = link + ".new"
    if os.path.lexists(new):
        os.unlink(new)
    os.symlink(target, new)
    os.replace(new, link)


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
        for name in os.listdir(PATH_DIR):
            link = os.path.join(PATH_DIR, name)
            if os.path.islink(link) and os.readlink(link).startswith(THROUGH) and name not in bins:
                os.unlink(link)
    except OSError as e:
        for what in [lambda: old_top and point(CURRENT, old_top)] + [
            (lambda n=n, t=t: point(os.path.join(PATH_DIR, n), t)) for n, t in links_before.items()
        ]:
            try:
                what()
            except OSError:
                pass
        if os.path.isdir(PATH_DIR):
            for name in os.listdir(PATH_DIR):
                p = os.path.join(PATH_DIR, name)
                if name not in links_before and os.path.islink(p):
                    try:
                        os.unlink(p)
                    except OSError:
                        pass
        if moved:
            shutil.rmtree(dest, ignore_errors=True)
        # A link or a rename names what it was making second.
        raise Refused([f"{e.filename2 or e.filename or ''}: {e.strerror}".lstrip(": ")])


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
    try:
        installed = manifest(os.path.join(CURRENT, "release.toml"))
        old_top = os.readlink(CURRENT) if os.path.islink(CURRENT) else ""
        with open(bundle, "rb") as f:
            digest = hashlib.sha256(f.read()).hexdigest()
    except Refused as e:
        print("veredas-atualizar: the installed release does not read:\n" + str(e), file=sys.stderr)
        return 2
    except OSError as e:
        print(f"veredas-atualizar: {bundle}: {e.strerror}", file=sys.stderr)
        return 2
    print(f"veredas-atualizar: {bundle} (SHA-256 {digest})")
    try:
        # One update at a time: the release's folder itself locked, so nothing is left behind.
        lock = os.open(OPT, os.O_RDONLY)
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        stage = tempfile.mkdtemp(prefix=".atualizar-", dir=OPT)
    except PermissionError:
        print(f"veredas-atualizar: {OPT} is not writable here: run it with sudo", file=sys.stderr)
        return 2
    except BlockingIOError:
        print("veredas-atualizar: another update is running", file=sys.stderr)
        return 2
    try:
        top = unpack(bundle, stage)
        folder = os.path.join(stage, top)
        check_sums(folder)
        m = check_manifest(folder, top, installed)
        check_starts(folder)
        with open(os.path.join(folder, "SHA256SUMS"), "rb") as f:
            sums = hashlib.sha256(f.read()).hexdigest()
        print(f"veredas-atualizar: its SHA256SUMS {sums} (what the boot's check says as VEREDAS-BUNDLE)")
        bins = sorted(os.listdir(os.path.join(folder, "bin")))
        install(stage, top, old_top, bins)
    except OSError as e:
        print("veredas-atualizar: not installed, the machine is as it was:", file=sys.stderr)
        print(f"  {e.filename2 or e.filename or bundle}: {e.strerror}", file=sys.stderr)
        return 2
    except Refused as e:
        print("veredas-atualizar: not installed, the machine is as it was:", file=sys.stderr)
        for r in e.reasons:
            print(f"  {r}", file=sys.stderr)
        return 2
    finally:
        shutil.rmtree(stage, ignore_errors=True)
    old = os.path.join(OPT, os.path.basename(old_top)) if old_top else "(none)"
    print(f"veredas-atualizar: release {installed['release']} -> {m['release']} (commit {m.get('commit')})")
    print(f"veredas-atualizar: installed at {os.path.join(OPT, top)}; the old release kept at {old}")
    print("veredas-atualizar: the plant's files in the home folder left as they are; the release's own plant is in /opt/veredas/current/planta")
    restart = subprocess.run(["systemctl", "restart", *SERVICES], capture_output=True, text=True)
    why = (restart.stderr.strip() or f"systemctl exited {restart.returncode}") if restart.returncode != 0 else staying_up()
    if why:
        back = f"sudo ln -sfn {os.path.basename(old)} /opt/veredas/current && sudo systemctl restart {' '.join(SERVICES)}"
        print(f"veredas-atualizar: the bench and supervisão did not stay up on it ({why}); to go back: {back}", file=sys.stderr)
        return 3
    print("veredas-atualizar: the bench and supervisão restarted on it and stayed up")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

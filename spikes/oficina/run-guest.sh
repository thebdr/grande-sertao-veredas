#!/bin/sh
# Oficina's own-shell spike in a Linux guest on QEMU/KVM with no virtual GPU (the spike's point 6),
# in one command; its result in one file.
#
#   spikes/oficina/run-guest.sh
#
# The guest is Ubuntu 26.10's own cloud image (this machine's release, so the spike runs there as
# built here: the same glibc), verified against Ubuntu's published sums. It gets 4 cores, 4 GB, no
# display device, and this machine's folders at their own paths over 9p: the repository (the spike's
# build, Monaco, supervisao's host; written to only under the spike's `target/tmp`), and read-only
# `~/.local/opt` (Qt and the spike's dependencies), `~/.rustup` and `~/.cargo/bin` (rust-analyzer,
# nextest). The spike's tests run there from nextest archives, on Qt's `offscreen` platform and on
# X11 under Xvfb, where Mesa draws with llvmpipe; then the guest is shut down.
#
# Built first, here: the spike's tests in debug and release, and supervisao's host
# (`cargo build -p veredas-supervisao --bin veredas-supervisao` at the repository's root).
# The image and the guest's disk live in ~/.cache/veredas/oficina (OFICINA_GUEST_DIR names another);
# the result is target/guest/guest-run.txt in the spike's folder.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
CACHE=${OFICINA_GUEST_DIR:-$HOME/.cache/veredas/oficina}
RELEASE=https://cloud-images.ubuntu.com/releases/stonking/release
IMAGE_NAME=ubuntu-26.10-server-cloudimg-amd64.img
IMAGE=$CACHE/image/$IMAGE_NAME
GUEST=$CACHE/guest
OUT=$HERE/target/guest
RESULT=$OUT/guest-run.txt
PORT=${OFICINA_GUEST_SSH_PORT:-2229}
USER_NAME=$(id -un)
mkdir -p "$CACHE/image" "$GUEST" "$OUT"
: >"$RESULT"
log() { printf '%s\n' "$*" | tee -a "$RESULT"; }

# The image, fetched once from Ubuntu's own site; its published sums checked against Ubuntu's
# cloud-image signing key (the distribution's keyring), and the image against its sum.
if [ ! -f "$IMAGE" ]; then
    curl -sSf -o "$CACHE/image/SHA256SUMS" "$RELEASE/SHA256SUMS"
    curl -sSf -o "$CACHE/image/SHA256SUMS.gpg" "$RELEASE/SHA256SUMS.gpg"
    curl -sSf -o "$IMAGE.part" "$RELEASE/$IMAGE_NAME"
    mv "$IMAGE.part" "$IMAGE"
fi
gpgv --keyring /usr/share/keyrings/ubuntu-cloudimage-keyring.gpg "$CACHE/image/SHA256SUMS.gpg" "$CACHE/image/SHA256SUMS" >>"$RESULT" 2>&1 \
    || { log "the sums' signature does not hold"; exit 1; }
(cd "$CACHE/image" && grep " \*$IMAGE_NAME\$" SHA256SUMS | sha256sum -c -) >>"$RESULT" 2>&1 || { log "the image does not match its sum"; exit 1; }

# The tests, as nextest archives the guest runs without cargo.
(cd "$HERE" && cargo nextest archive --workspace --archive-file "$OUT/debug.tar.zst" >>"$RESULT" 2>&1)
(cd "$HERE" && cargo nextest archive --workspace --release --archive-file "$OUT/release.tar.zst" >>"$RESULT" 2>&1)

# A key for this guest alone, and the seed cloud-init reads: the user (this one's name and id), the
# libraries Qt, WebEngine and the spike load here (`ldd` over them, each file's package by
# `dpkg -S` on this machine) and what they open at run time (Mesa's drivers, fonts, Xvfb), and the
# folders mounted where they are here.
[ -f "$GUEST/key" ] || ssh-keygen -q -t ed25519 -N "" -C oficina-guest -f "$GUEST/key"
PACKAGES="libasound2t64 libatk1.0-0t64 libatk-bridge2.0-0t64 libatomic1 libatspi2.0-0t64 libavahi-client3 libcairo2 libcairo-gobject2 libcups2t64 libdbus-1-3 libdrm2 libegl1 libepoxy0 libfontconfig1 libfreetype6 libgbm1 libgdk-pixbuf-2.0-0 libgl1 libglib2.0-0t64 libglx0 libgtk-3-0t64 libharfbuzz0b libnspr4 libnss3 libopengl0 libpango-1.0-0 libpangocairo-1.0-0 libwayland-client0 libwayland-cursor0 libwayland-egl1 libx11-6 libx11-xcb1 libxcb1 libxcb-cursor0 libxcb-dri3-0 libxcb-glx0 libxcb-icccm4 libxcb-image0 libxcb-keysyms1 libxcb-randr0 libxcb-render0 libxcb-render-util0 libxcb-shape0 libxcb-shm0 libxcb-sync1 libxcb-util1 libxcb-xfixes0 libxcb-xkb1 libxcomposite1 libxcursor1 libxdamage1 libxext6 libxfixes3 libxi6 libxinerama1 libxkbcommon0 libxkbcommon-x11-0 libxkbfile1 libxrandr2 libxrender1 libxres1 libxtst6 mesa-libgallium libegl-mesa0 libglx-mesa0 libgl1-mesa-dri fonts-dejavu-core fontconfig xvfb xauth"
mounts=""
for share in "worktree $ROOT rw" "opt $HOME/.local/opt ro" "rustup $HOME/.rustup ro" "cargobin $HOME/.cargo/bin ro"; do
    set -- $share
    # A read-only share is cached by the guest's page cache (nothing changes it during a run).
    cache=$([ "$3" = ro ] && echo ",cache=loose" || echo "")
    mounts="$mounts
  - [sh, -c, 'mkdir -p $2 && mount -t 9p -o trans=virtio,version=9p2000.L,msize=1048576,$3$cache $1 $2']"
done
cat >"$GUEST/user-data" <<EOF
#cloud-config
users:
  - name: $USER_NAME
    uid: $(id -u)
    shell: /bin/bash
    sudo: ALL=(ALL) NOPASSWD:ALL
    ssh_authorized_keys: [$(cat "$GUEST/key.pub")]
package_update: true
packages: [$(echo $PACKAGES | sed 's/ /, /g')]
runcmd:$mounts
  - [sh, -c, 'for d in .local .cargo code; do chown $USER_NAME: $HOME/\$d; done; touch /var/lib/cloud/instance/oficina-ready']
EOF
printf 'instance-id: oficina-spike-%s\nlocal-hostname: oficina-guest\n' "$(date +%s)" >"$GUEST/meta-data"
xorriso -as mkisofs -quiet -output "$GUEST/seed.iso" -volid cidata -joliet -rock "$GUEST/user-data" "$GUEST/meta-data"

# A fresh disk over the image each run (its host keys new, so the last run's are forgotten), and the guest: KVM, 4 cores, 4 GB, no display device.
rm -f "$GUEST/disk.qcow2" "$GUEST/known_hosts"
qemu-img create -q -f qcow2 -F qcow2 -b "$IMAGE" "$GUEST/disk.qcow2" 20G
qemu-system-x86_64 -enable-kvm -machine q35 -cpu host -smp 4 -m 4096 \
    -drive file="$GUEST/disk.qcow2",if=virtio -drive file="$GUEST/seed.iso",media=cdrom \
    -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22 -device virtio-net-pci,netdev=n0 \
    -virtfs local,path="$ROOT",mount_tag=worktree,security_model=none,id=worktree \
    -virtfs local,path="$HOME/.local/opt",mount_tag=opt,security_model=none,readonly=on,id=opt \
    -virtfs local,path="$HOME/.rustup",mount_tag=rustup,security_model=none,readonly=on,id=rustup \
    -virtfs local,path="$HOME/.cargo/bin",mount_tag=cargobin,security_model=none,readonly=on,id=cargobin \
    -vga none -display none -serial file:"$GUEST/console.log" \
    -daemonize -pidfile "$GUEST/qemu.pid"
stop() {
    ssh -i "$GUEST/key" -p "$PORT" -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile="$GUEST/known_hosts" "$USER_NAME@127.0.0.1" sudo poweroff >/dev/null 2>&1 || true
    i=0
    while [ -f "$GUEST/qemu.pid" ] && kill -0 "$(cat "$GUEST/qemu.pid")" 2>/dev/null && [ $i -lt 60 ]; do sleep 1; i=$((i + 1)); done
    [ -f "$GUEST/qemu.pid" ] && kill "$(cat "$GUEST/qemu.pid")" 2>/dev/null || true
    rm -f "$GUEST/qemu.pid"
    # What the next run makes anew: the guest's disk (about 9 GB after a run) and the archives.
    rm -f "$GUEST/disk.qcow2" "$OUT/debug.tar.zst" "$OUT/release.tar.zst" "$OUT/code"
}
trap stop EXIT
guest() { ssh -i "$GUEST/key" -p "$PORT" -o BatchMode=yes -o ConnectTimeout=5 -o StrictHostKeyChecking=no -o UserKnownHostsFile="$GUEST/known_hosts" "$USER_NAME@127.0.0.1" "$@"; }

# Up once cloud-init has installed the packages and mounted the folders (20 minutes at most).
i=0
until guest test -f /var/lib/cloud/instance/oficina-ready 2>/dev/null; do
    i=$((i + 1))
    [ $i -lt 240 ] || { log "the guest never came up; its console: $GUEST/console.log"; exit 1; }
    sleep 5
done
log "== the guest"
guest 'uname -sr; . /etc/os-release; echo "$PRETTY_NAME"; ldd --version | head -1; nproc; free -m | sed -n 2p; lspci | grep -i -e vga -e display -e 3d || echo "no display device"' 2>&1 | tee -a "$RESULT"

# The runs: each one's output and exit code in the result.
SYSROOT=$(rustc --print sysroot)
run() {
    name=$1; shift
    log ""; log "== $name"
    # The exit code is the guest command's, not the pipe's (sh has no pipefail), and a failing run
    # is written, not taken for the script's own failure (a list is exempt from `set -e`).
    { guest "cd '$HERE' && mkdir -p \$HOME/debug \$HOME/release \$HOME/scratch && export PATH=\$HOME/.cargo/bin:\$PATH RUSTC='$SYSROOT/bin/rustc' OFICINA_SCRATCH=\$HOME/scratch && $*" 2>&1 && echo 0 >"$OUT/code" || echo $? >"$OUT/code"; } | tee -a "$RESULT"
    code=$(cat "$OUT/code")
    log "exit $code"
    VERDICTS="$VERDICTS
$(printf '%-48s exit %s' "$name" "$code")"
}
VERDICTS=""
# The two tests that read the suite's TOML texts through git are left out: the repository's
# `.git` (a worktree's, pointing into the main clone) is not mounted, and they draw nothing.
nextest="cargo-nextest nextest run --extract-overwrite --workspace-remap '$HERE' --no-fail-fast --no-capture -j 1"
suite="-E 'not test(/toml_text_of_the_suite/)'"
run "suite on offscreen" "$nextest --archive-file '$OUT/debug.tar.zst' --extract-to \$HOME/debug $suite"
run "suite on X11 (Xvfb, llvmpipe)" "OFICINA_TEST_PLATFORM=xcb OFICINA_EXPECT_DRAWING=software OFICINA_EXPECT_ADAPTER=llvmpipe xvfb-run -a -s '-screen 0 1920x1080x24' $nextest --archive-file '$OUT/debug.tar.zst' --extract-to \$HOME/debug $suite"
run "release timings on X11 (Xvfb, llvmpipe)" "OFICINA_TEST_PLATFORM=xcb xvfb-run -a -s '-screen 0 1920x1080x24' $nextest --archive-file '$OUT/release.tar.zst' --extract-to \$HOME/release -E 'test(under_a_second) | test(scrolled) | test(memory)'"
run "release timings on offscreen" "$nextest --archive-file '$OUT/release.tar.zst' --extract-to \$HOME/release -E 'test(under_a_second) | test(scrolled) | test(memory)'"
log ""
log "== verdicts$VERDICTS"

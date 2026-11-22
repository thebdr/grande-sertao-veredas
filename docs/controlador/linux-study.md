# Linux for real time and recovery: a study

**A study, 2026-11-22. It rules nothing.** `docs/robustness.md` ("The Linux study") owes it before any rule on the
operating system is written: "we need to study linux for rt applications, i suppose recovery mechanisms exist. [...]
if existing tested methods exist, we use them" (the architect, 2026-11-16). It checks the list the assistant named
there from memory against primary sources, corrects it where it was wrong, adds what it missed, and ends with the
choices it leaves for the architect (`docs/questions.md` Q226).

How it was done: every source below was opened on 2026-11-22, when mainline Linux stood at 7.4-rc3. Kernel files are
cited by their path in mainline (`https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/plain/<path>`).
Some sites refused automated reading (the Linux Foundation wiki, elixir.bootlin.com, sourceware.org,
smartmontools.org, packages.debian.org); they were not worked around, and other primary sources stand in. What no
primary source confirmed is marked **not verified**.

## The list, corrected

| The list said (2026-11-16) | The sources say |
|---|---|
| Real time in the mainline kernel since 6.12 | Right: merged 20 September 2024, released 17 November 2024; x86, arm64 and RISC-V then, 32-bit ARM and LoongArch since |
| (Veredas 1's runtime read `/sys/kernel/realtime`) | Not mainline: an out-of-tree patch Ubuntu still carries. Mainline says it in `uname -v` (`PREEMPT_RT`) |
| ext4 with small inodes and XFS without its larger timestamps stop in 2038 | Right; 256-byte ext4 inodes (to 2446) are the default since e2fsprogs 1.46.4 (2021), XFS `bigtime` since xfsprogs 5.15.0 (2022) |
| NTP's count starts over on 7 February 2036; the client must handle it | chrony and ntpd/NTPsec do; **systemd-timesyncd does not**, by its current source |
| Btrfs or ZFS mirroring with a scheduled repair run | Right, and a third tested way exists at the block layer: dm-integrity under a RAID1 |
| A read-only system under dm-verity with its error-correcting data | Right; the correction repairs what is read and never rewrites the damaged blocks |
| MRAM or FRAM for data written every cycle | Right; their retention depends on temperature (one FRAM: 5 years at 85 °C, 73 at 65 °C) |
| (not on the list) | systemd sets a correct clock back if it reads more than 15 years past its own epoch |
| (not on the list) | The Civil Infrastructure Platform keeps kernels for ten years each: 6.12-cip to 2035 |

## The findings

### 1. Real time
- **Mainline since 6.12.** Thomas Gleixner's pull request of 20 September 2024 enabled it "on the supported
  architectures ARM64, RISCV and X86" (https://lists.openwall.net/linux-kernel/2024/09/20/15); Linux 6.12 was
  released on 17 November 2024 with it (https://kernelnewbies.org/Linux_6.12). In 7.4-rc3, `ARCH_SUPPORTS_RT` is
  selected by x86, arm64, 32-bit ARM, RISC-V and LoongArch (`arch/<arch>/Kconfig`). `CONFIG_PREEMPT_RT` depends on
  `EXPERT` (`kernel/Kconfig.preempt`).
- **The kernel's own guide** for worst-case latency: the performance frequency governor, C-states capped
  (`processor.max_cstate=1`), integrated graphics off where possible, and no tickless mode for periodic control loops
  (https://docs.kernel.org/core-api/real-time/kernel-configuration.html).
- **What a process needs**: `SCHED_FIFO` up to its `RLIMIT_RTPRIO`, or anything with `CAP_SYS_NICE`; by default the
  kernel keeps 5 % of each second for tasks that are not real time (https://man7.org/linux/man-pages/man7/sched.7.html);
  `mlockall` with `MCL_CURRENT | MCL_FUTURE`, past `RLIMIT_MEMLOCK` with `CAP_IPC_LOCK`
  (https://man7.org/linux/man-pages/man2/mlockall.2.html). A systemd unit grants all of it without root:
  `CPUSchedulingPolicy=fifo`, `CPUSchedulingPriority=`, `CPUAffinity=`, `LimitRTPRIO=`
  (https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html). Cores are set apart with `isolcpus=`,
  `nohz_full=` and `irqaffinity=` (https://docs.kernel.org/admin-guide/kernel-parameters.html).
- **Telling a real-time kernel at run time**: mainline has no `/sys/kernel/realtime` (`kernel/ksysfs.c`); the file is
  an out-of-tree patch Ubuntu carries (https://lists.snapcraft.io/archives/kernel-team/2026-March/166192.html). The
  version string `uname -v` names `PREEMPT_RT` (`init/Makefile`). So "a target refused real-time scheduling stays in
  STOP" (`docs/controlador-plan.md`) is checked on the policy the process got, its locked memory and that string.
- **Measuring**: `cyclictest` from rt-tests, maintained at kernel.org; its README calls short runs "rather
  meaningless" and asks for production-like load, "preferably 24 hours or more"
  (https://git.kernel.org/pub/scm/utils/rt-tests/rt-tests.git/plain/README.markdown). OSADL measures many x86 and ARM
  boards twice a day (https://www.osadl.org/Latency-plots.latency-plots.0.html). One figure read: a Raspberry Pi 5
  under heavy stress, worst case over 9 ms without real time and under 225 µs with it (https://arxiv.org/abs/2604.19275).
  Figures for x86 or arm64 industrial PCs: not verified; they belong to the chosen board, its firmware and its
  drivers, measured for days.

### 2. Two system partitions, boot counting, fallback
Five maintained projects do it, each with years of releases:
- **RAUC** (LGPL-2.1): signed SquashFS bundles, signing mandatory; U-Boot, GRUB, barebox and EFI; Yocto, PTXdist and
  Buildroot (https://rauc.readthedocs.io/en/latest/, `integration.html`).
- **SWUpdate** (GPL-2.0 with an OpenSSL exception): signed CPIO images; U-Boot, GRUB and EFI Boot Guard; packaged in
  Debian and Ubuntu (https://sbabic.github.io/swupdate/swupdate.html). **EFI Boot Guard** (Siemens, GPL-2.0) arms
  the hardware watchdog before the operating system and refuses to boot without one
  (https://github.com/siemens/efibootguard). The CIP reference images use the two together
  (https://gitlab.com/cip-project/cip-core/isar-cip-core).
- **Mender** (Apache-2.0 client): the bootloader rolls back unless the client commits on the first boot; signatures
  opt-in (https://docs.mender.io/overview/introduction).
- **systemd-sysupdate with systemd-boot's boot counting**: UEFI only (https://systemd.io/AUTOMATIC_BOOT_ASSESSMENT/).
- **OSTree with greenboot** (LGPL), in Fedora CoreOS and IoT, RHEL CoreOS and Torizon (https://github.com/ostreedev/ostree,
  https://github.com/fedora-iot/greenboot).
- Underneath, U-Boot's own boot count runs another command after too many failed boots
  (https://docs.u-boot.org/en/latest/api/bootcount.html).

None of the sources ranks them by deployment: which is most used is not verified. What none of them decides is what
"booted well" means for a controller, which is the one thing the update framework asks of its integrator.

### 3. A read-only system under dm-verity
dm-verity checks every block read against a hash tree whose root is trusted, and restarts, panics or logs on
damage, as configured (https://docs.kernel.org/admin-guide/device-mapper/verity.html). Its forward error correction is
Reed-Solomon with 2 to 24 parity bytes per codeword, 2 recommended (about 0.8 % of the space), interleaved across the
device so long runs of bad blocks are recovered (same page; Android recovers 16 to 24 MiB of consecutive blocks on a
2 to 3 GiB partition, https://source.android.com/docs/security/features/verifiedboot/dm-verity). It corrects what it
reads and never writes back: the damage stays and the correction repeats; the count of corrected blocks is reported
and is what a maintenance request would watch.

### 4. The program's two copies: mirrors with a checksum per block
The architect's design ("two copies and a hash per block", a check on a schedule that repairs, `docs/robustness.md`)
exists, tested, in three forms:
- **Btrfs RAID1** (or DUP on one device): a scrub reads everything, checks each block's checksum and "will also
  automatically repair any damage by copying verified good data from one of the other replicas"; so does a normal read
  (https://btrfs.readthedocs.io/en/latest/Scrub.html). RAID1 and scrub are marked stable; RAID5 and 6 are not
  (https://btrfs.readthedocs.io/en/latest/Status.html). Files marked `+C` carry no checksum.
- **OpenZFS mirror or `copies=2`**: the same repair at scrub (https://openzfs.github.io/openzfs-docs/man/master/8/zpool-scrub.8.html),
  but outside the kernel by licence (https://openzfs.github.io/openzfs-docs/License.html), and each release names the
  newest kernel it supports: 7.3 while mainline is at 7.4-rc3 (https://raw.githubusercontent.com/openzfs/zfs/master/META).
- **dm-integrity under an LVM RAID1** (`--raidintegrity y`): a failed checksum is read from the other copy "and will
  also automatically write the good data back"; scrubbing by `lvchange --syncaction`
  (https://man7.org/linux/man-pages/man7/lvmraid.7.html, https://docs.kernel.org/admin-guide/device-mapper/dm-integrity.html).
  A plain RAID1 without integrity cannot tell which copy is right.

One device protects against bad blocks, not against the device's death; the second drive is the engineer's
parameter, as ruled.

### 5. Memory that keeps its content, hold-up power, a power-fail signal
- **FRAM**: an Infineon part rated for 10^15 writes, retention 5 years at 85 °C and 73 years at 65 °C, with its own
  error correction (datasheet 002-37845, https://www.infineon.com/assets/row/public/documents/10/49/infineon-cy15b108qi-8-mb-excelon-tm-lp-ferroelectric-ram-f-ram-serial-spi-1024k-x-8-20-mhz-automotive-grade-3-datasheet-en.pdf).
  **MRAM**: an Everspin part with "unlimited write endurance" and retention "greater than 20 years"
  (https://www.everspin.com/sites/default/files/MR2xH40_Datasheet.pdf). Both have mainline drivers (the AT25 driver
  for SPI FRAM; `drivers/mtd/spi-nor/everspin.c`).
- **Supercapacitors** age with voltage and heat; end of life is usually the resistance doubled and the capacitance down
  30 % (https://www.farnell.com/datasheets/4804888.pdf). A hold-up budget is sized against the aged part.
- **A power-fail signal reaches a process** as a GPIO edge through the GPIO character device
  (https://docs.kernel.org/userspace-api/gpio/chardev.html), as an input event through `gpio-keys`, or as a UPS event
  through NUT (https://networkupstools.org/docs/man/upsmon.html); the kernel's own action on "imminent voltage loss"
  is set by `hw_protection=` (https://docs.kernel.org/admin-guide/kernel-parameters.html), in sysfs since 6.15. On
  x86 industrial PCs the line is the vendor's: not verified.

So "decades" for a per-cycle save in FRAM holds only for a cool enclosure, and the atomic form of a per-cycle write
(two buffers and a check) is ours to design: no source found prescribes one.

### 6. Time past 2038
ext4's 128-byte inodes overflow in January 2038; larger inodes reach May 2446
(https://docs.kernel.org/filesystems/ext4/inodes.html), the default since e2fsprogs 1.46.4
(https://e2fsprogs.sourceforge.net/e2fsprogs-release.html). XFS `bigtime` reaches 2486, a kernel feature since 5.10
and the default of `mkfs.xfs` since xfsprogs 5.15.0 (https://git.kernel.org/pub/scm/fs/xfs/xfsprogs-dev.git/plain/doc/CHANGES);
XFS's older on-disk format cannot, and its support goes in September 2030 (https://docs.kernel.org/admin-guide/xfs.html).
Btrfs stores 64-bit seconds. On both 64-bit targets `time_t` is 64 bits (Rust's `libc`,
https://docs.rs/libc/latest/x86_64-unknown-linux-gnu/libc/type.time_t.html). An image made by an old tool keeps the
2038 limit, so the build checks it.

### 7. NTP and 7 February 2036
NTP's timestamp counts 32 bits of seconds from 1900 (https://www.eecis.udel.edu/~mills/y2k.html). **ntpd and
NTPsec** resolve each timestamp to the era nearest the date they were built (https://docs.ntpsec.org/latest/rollover.html);
**chrony** assumes a window starting 50 years before its build, so a build of 2026 covers about 1976 to 2112
(https://gitlab.com/chrony/chrony/-/raw/master/configure). **systemd-timesyncd** has no era logic in its current
source and refuses replies it reads as before its epoch
(https://raw.githubusercontent.com/systemd/systemd/main/src/timesync/timesyncd-manager.c, a reading of the code, not a
test); an Ubuntu report that it stops past 2036 was closed "Won't Fix"
(https://bugs.launchpad.net/ubuntu/+source/systemd/+bug/1905688). So the integrator's NTP server is not enough: the
client on the controller must be chrony or ntpd.

### 8. The hardware watchdog
`/dev/watchdog` resets the board when its pings stop; with `nowayout` nothing disarms it once started
(https://docs.kernel.org/watchdog/watchdog-api.html); the kernel can keep pinging one the firmware started until
user space takes over, within a bound (https://docs.kernel.org/watchdog/watchdog-parameters.html). systemd pings it
from PID 1 with `RuntimeWatchdogSec=` (off by default), and watches a service with `WatchdogSec=`, restarting it or
rebooting on a miss (https://www.freedesktop.org/software/systemd/man/latest/systemd-system.conf.html,
`systemd.service.html`, `systemd.unit.html`). A kernel lockup only prints, unless `softlockup_panic` or
`hardlockup_panic` and `panic=` turn it into a reboot (https://docs.kernel.org/admin-guide/lockup-watchdogs.html). PID
1's ping proves PID 1 runs, not the controller: the chain must end in the controller's own process.

### 9. The clock chip's lost time and low battery
Linux reports both through `RTC_VL_READ` (`RTC_VL_DATA_INVALID`, `RTC_VL_BACKUP_LOW`, `RTC_VL_BACKUP_EMPTY`;
`include/uapi/linux/rtc.h`), read by `hwclock --vl-read` (https://man7.org/linux/man-pages/man8/hwclock.8.html). Each
driver chooses: among those read, pcf8523, pcf2127, pcf85063, pcf8563, rv3028, rv3032, rv8803, rv3029c2, rx8010,
nct3018y and abx80x answer it; ds1307, ds3232, m41t80, isl1208, rx8025, rx8581, rtc-efi and **rtc-cmos, the x86 PC's
clock**, do not (`drivers/rtc/rtc-<name>.c`; the full list not verified). Neither chrony nor systemd reads it
(https://gitlab.com/chrony/chrony/-/raw/master/rtc_linux.c; systemd not verified beyond its documentation).
**systemd sets the clock back** when it reads more than 15 years past its own epoch, the newest of its build time,
`/usr/lib/clock-epoch` and the time it last synchronised (https://www.freedesktop.org/software/systemd/man/latest/systemd.html;
the 15 years a build option, https://raw.githubusercontent.com/systemd/systemd/main/meson_options.txt): a controller
left 15 years without an update and without synchronising would have a correct clock rewound.

So the architect's table of the clock battery and NTP (`docs/robustness.md`, "Time over decades") can be read on a
board whose clock chip answers `RTC_VL_READ`, and not on an x86 PC's own clock as it stands.

### 10. The operating system's own writes
journald keeps its logs on the drive unless `Storage=volatile`, up to 10 % of the file system by default
(https://www.freedesktop.org/software/systemd/man/latest/journald.conf.html); ext4 commits every 5 s
(https://docs.kernel.org/admin-guide/ext4.html), Btrfs every 30 s; `relatime` is the default since 2.6.30
(https://man7.org/linux/man-pages/man8/mount.8.html); `systemd.volatile=overlay` runs a read-only root under a layer
in memory (https://www.freedesktop.org/software/systemd/man/latest/systemd-fstab-generator.html). Volatile logs are
lost at a power cut; a crash record survives in pstore (below).

### 11. Reading a drive's wear
eMMC 5.0 and later report their life used in 10 % steps and a pre-end-of-life warning, which the kernel shows as
`life_time` and `pre_eol_info` in sysfs (`drivers/mmc/core/mmc.c`; decoded by mmc-utils,
https://git.kernel.org/pub/scm/utils/mmc/mmc-utils.git/plain/mmc_cmds.c). NVMe reports a percentage used, allowed past
100 (NVM Express Base Specification 2.1). SATA SSDs report through the device statistics log
(https://manpages.debian.org/trixie/smartmontools/smartctl.8.en.html). SD cards have "no standard way"; a few
industrial cards answer a vendor command (https://github.com/Ognian/sdmon). The thresholds that raise a request are
ours to set.

### 12. What the list missed
- **Ten-year kernels.** The Civil Infrastructure Platform, a Linux Foundation project of industrial vendors, keeps
  "Super-Long-Term Stable" kernels ten years each: 4.4 to 2027, 4.19 to 2029, 5.10 to 2031, 6.1 to 2033, 6.12 to 2035,
  a new line every two years, with real-time versions (https://www.cip-project.org/about/linux-kernel-core-packages,
  https://cip-project.org/blog/2026/08/06/a-decade-of-stability-10-years-of-the-cip-kernel-wg). Kernel.org's own
  long-term 6.12 ends in December 2028 (https://www.kernel.org/category/releases.html). For 6.12 CIP keeps no separate
  real-time branch, which fits real time being in mainline (the study's reading). Decades still means moving from
  one line to the next at least once.
- **Memory errors.** EDAC reports corrected and uncorrected memory errors, and a rising count of corrected ones is a
  maintenance signal (https://docs.kernel.org/admin-guide/RAS/main.html). It needs ECC memory on the board.
- **A crash leaves a record and a reboot**: `panic=` reboots after a panic, `oops=panic` makes an oops one
  (https://docs.kernel.org/admin-guide/kernel-parameters.html); ramoops keeps the last log in memory that survives a
  reset, and systemd-pstore files it at the next boot (https://docs.kernel.org/admin-guide/ramoops.html).
- **Memory pressure**: `OOMScoreAdjust=-1000` and `MemoryMin=` keep the kernel from killing or starving the controller
  (`systemd.exec.html`, `systemd.resource-control.html`).
- Named and not studied: Secure Boot and measured boot, firmware updates over decades, distributions' long support,
  power-safe writes of the controller's own files.

## What it means for what is ruled
- "Existing, tested Linux methods before our own" can be met for every recovery mechanism the round asked about: the
  update with fallback, the read-only system, the program's two copies with a check per block and a scheduled repair,
  the watchdog chain, the crash record.
- Three things the rulings assume need a choice to hold: the clock battery's maintenance request needs a clock chip
  whose driver answers `RTC_VL_READ`; the date over decades needs an NTP client that crosses 2036 and a guard against
  systemd's 15-year rewind; the per-cycle save needs memory that keeps its content at the enclosure's temperature.
- Real time is a property of the board, its firmware and its drivers: a board is accepted by `cyclictest` under load
  for a day or more, never by a figure from a datasheet.

## What it leaves for the architect (Q226)
- **The kernel line**: kernel.org's long-term (about four years) or CIP's (ten), and how a controller moves to the next.
- **The update framework**: RAUC, SWUpdate with EFI Boot Guard (CIP's choice), Mender, systemd-sysupdate (UEFI only)
  or OSTree with greenboot; and what "booted well" means for a controller.
- **The system partition's integrity**: dm-verity with or without correction, and what damage does (restart, stop,
  log and go on).
- **The program's two copies**: Btrfs, ZFS (outside the kernel) or dm-integrity under RAID1; one drive or two; the
  scrub's period and how its result becomes a maintenance request.
- **Time**: chrony or ntpd as the client (not systemd-timesyncd); clock chips whose drivers answer `RTC_VL_READ`; how
  systemd's 15-year rewind is neutralised.
- **The watchdog chain**: which layer arms it and which owns the last ping (the controller's process, by this study's
  reading of section 8).
- **Logs**: in memory or on the drive, with pstore for crashes.
- **Hardware requirements a target must meet**: ECC memory, a drive that reports its wear (eMMC or NVMe, not SD),
  memory that keeps its content for retained data where the target offers it, a power-fail line where it has hold-up
  power.

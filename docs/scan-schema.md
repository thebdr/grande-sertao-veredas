# The scan file (`veredas-scan`)

What a listening found, as aranha writes it and reads it back (`crates/veredas-aranha/src/scan.rs`,
contract C-146): the listener's counts, every device with every value it was given and the
witnesses for each, their conflicts, and the topology, beside the capture of the frames those
witnesses name. A **baseline** is a scan saved to compare a later one against. The file is TOML
(law L7), version 1; aranha's types are written as they are, so this page names what each holds
rather than repeating every type.

## Reading it

- `format = "veredas-scan"` and `version = 1` come first and are read first, by the suite's one
  header reader (core's `schema`): another format, another version, a missing or mistyped stamp is
  refused by name (`` `format` is `veredas-bench`, not `veredas-scan` ``, `version 2 of
  `veredas-scan` is not known here (this reader knows 1)`), before the body is looked at. A later
  version is read by a migration or refused, never guessed at.
- Then the body, whole: every key is one the reader takes, or the file is refused, naming each key
  nothing took, wherever it sits (`a scan file holds no key `colour`: refused, not ignored`); a key
  missing or of the wrong kind is no scan (`not a scan file: ...`); and what was read is what the
  file holds, written back as TOML's own values, or the file is refused, naming the first place it
  differs (an element past what its list takes, a table written as a list, a word written as a
  table, a repeat a set keeps once: `a scan file's `capture` is not what its reader keeps`).
- Read whole, it must be a scan: its devices by MAC, each once; no field holding two values last
  heard in one frame (a frame says one value of a field); its topology's links each stored
  lesser end first and by their ends, each pair of ends once (a link is its two ends: one cable is
  one link); and no witness, nor a port's last
  leaving or other frame, naming a frame past its capture's last record (`not a scan: ...`). A
  witness may name a frame before its capture's first: one the listener let go past its bound
  (counted in `stats.let_go`), which the capture does not hold. A scan that is no scan is refused
  when written too.
- Every number is one of TOML's integers (64 bits, signed). Stamps are nanoseconds since the
  epoch, the frames' clock, so a scan holds stamps until 2262; a scan holding a number past them
  is refused when it is written, never left for another reader to refuse.

## The keys

- `format`, `version`: the stamp.
- `tool`: what took it, `veredas-aranha <version>`.
- `taken_ns`: when it was written.
- `capture` (absent when no capture was written): the capture of the frames beside it, in the scan
  file's folder: `file` (its name), `first_frame` (the number of its first record) and `frames`
  (how many it holds). A witness's frame `n`, from `first_frame` on, is the capture's record
  `n - first_frame`, so every value witnessed there can be read again from the frame it came from
  (`wire::pcap`, C-139); a frame before it was let go.
- `stats`: the listener's counts (C-142): `heard`, `let_go` (frames let go past its bound), the
  heard frames by class (`by_class`: `dcp`, `cyclic`, `alarm`, `profinet-other`, `lldp`, `arp`,
  `ipv4`, `ipv6`, `ethercat`, `powerlink`, `mrp`, `ptp`, `other:0x....`, `malformed`),
  `observations` fused, `own` (left out as this machine's), the last fifty `damage` lines and the
  `damaged_frames`, the earliest and latest stamps heard (`first_ns`, `last_ns`, absent when nothing
  was heard), and each interface's account under `interfaces` by its name: its `mac` (absent for a
  capture), `heard`, `sent`, `refused`, `queued`, `unsent`, `withdrawn`, and why its worker
  `ended`, when it did.
- `devices`, by MAC (C-141): each with its `mac`; every field of what it said as the list of its
  values, each with the `value`, its `first` and `last` witness and its `count`, the first heard
  first: DCP's `name`, `alias`, `ip` (`ip`, `mask`, `gateway`), `ip_status` (`set`, `conflict`),
  `pn_id` (vendor and device ids), `vendor`, `role`, `instance`; LLDP's `chassis` (a key by the
  chassis id's subtype: `mac:...`, `local:...`, `ifname:...`, `net:...` or `other<subtype>:...`,
  the last two in hex), `sys_name`, `sys_desc`, `sys_caps`, `mgmt_ips` (each PDU's IPv4
  management addresses); `arp_ip`; its `ports` by short name, each with its `id`, `port_mac`,
  `desc`, `pn_status` (PROFINET's RTClass 2 and 3 port status, `rtc2` and `rtc3`), `delays`
  (`rx_local`, `rx_remote`, `tx_local`, `tx_remote` and `cable_local`, in nanoseconds), `mrp`
  (`domain_uuid`, `mrrt`), `pvid`, `mau_type` (fields as above), the interfaces it was `heard_on`
  and, `on` each of them, the `frames` heard there and the `last` one's stamp and frame number
  (what was heard before its last leaving is told from what was heard since), its last leaving
  frame and last other frame (`leaving`,
  `spoke`: a stamp and a frame number each, absent when none) and when it left (`left_ns`, absent
  unless its last frame said so), its `first_ns`, `last_ns` and `frames`; what it was `heard`
  doing (`dcp-heard`, `dcp-request-heard`, `lldp-heard`, `arp-heard`), on which interfaces
  (`heard_on`), its `first_ns`, `last_ns` and `frames`, and `by_source` (a port not yet known as a
  device's). A witness holds the `probe`, the `iface`, the stamp (`at_ns`) and the `frame` number.
- `conflicts`, ordered by kind and value: each with its `kind` (`duplicate-name`, `duplicate-ip`,
  `name-changed`, `ip-changed`, `reports-ip-conflict`), the `value` it is about and the `macs` of
  the devices.
- `topology` (C-145): its `nodes` (`id`: `dev:<mac>`, `if:<name>` or `chassis:<the chassis id's
  text>`; `kind`: `device`, `interface` or `chassis`, `label`, `mac`,
  `ports`, the MRP `rings` its ports are in), `links` (ends `a` and `b`, each a `node` and a `port`,
  the lesser first; `confidence`: `heard`, `reported` or `confirmed`; `witnesses`), `rings`
  (`domain`, `members`, `closed`), `components` (node ids, the largest first) and `notes` (what it
  did not draw, and why).

## An example

The file of the sample scan in `crates/veredas-aranha/tests/scan.rs`, which that test holds this
page to: a device's Hello, its port's LLDP and its ARP, and a second device of the same station
name, a second apart, replayed through a listener as `eth1`; one duplicate-name conflict, and the
cable from `eth1` to the first device's port.

```toml
format = "veredas-scan"
version = 1
tool = "veredas-aranha 2.0.0-alpha.1"
taken_ns = 1764000010000000000

[capture]
file = "sample.pcap"
first_frame = 0
frames = 4

[stats]
heard = 4
let_go = 0
observations = 4
own = 0
damage = []
damaged_frames = 0
first_ns = 1764000000000000000
last_ns = 1764000003000000000

[stats.by_class]
dcp = 2
lldp = 1
arp = 1

[stats.interfaces.eth1]
heard = 4
sent = 0
refused = 0
queued = 0
unsent = 0
withdrawn = 0
ended = "its capture is read to the end"

[[devices]]
mac = "02:00:00:00:00:01"
alias = []
vendor = []
role = []
instance = []
sys_name = []
sys_desc = []
sys_caps = []
mgmt_ips = []
heard = ["dcp-heard", "lldp-heard", "arp-heard"]
heard_on = ["eth1"]
first_ns = 1764000000000000000
last_ns = 1764000002000000000
frames = 3
by_source = false

[[devices.name]]
value = "bomba-1"
count = 1

[devices.name.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[devices.name.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[[devices.ip]]
count = 1

[devices.ip.value]
ip = "192.168.0.10"
mask = "255.255.255.0"
gateway = "0.0.0.0"

[devices.ip.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[devices.ip.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[[devices.ip_status]]
count = 1

[devices.ip_status.value]
set = 1
conflict = false

[devices.ip_status.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[devices.ip_status.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[[devices.pn_id]]
value = [42, 261]
count = 1

[devices.pn_id.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[devices.pn_id.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000000000000000
frame = 0

[[devices.chassis]]
value = "local:bomba-1"
count = 1

[devices.chassis.first]
probe = "lldp-heard"
iface = "eth1"
at_ns = 1764000001000000000
frame = 1

[devices.chassis.last]
probe = "lldp-heard"
iface = "eth1"
at_ns = 1764000001000000000
frame = 1

[[devices.arp_ip]]
value = "192.168.0.10"
count = 1

[devices.arp_ip.first]
probe = "arp-heard"
iface = "eth1"
at_ns = 1764000002000000000
frame = 2

[devices.arp_ip.last]
probe = "arp-heard"
iface = "eth1"
at_ns = 1764000002000000000
frame = 2

[devices.ports.port-001]
id = "port-001"
desc = []
pn_status = []
delays = []
mrp = []
pvid = []
mau_type = []
heard_on = ["eth1"]
spoke = [1764000001000000000, 1]
first_ns = 1764000001000000000
last_ns = 1764000001000000000
frames = 1

[[devices.ports.port-001.port_mac]]
value = "02:00:00:00:01:01"
count = 1

[devices.ports.port-001.port_mac.first]
probe = "lldp-heard"
iface = "eth1"
at_ns = 1764000001000000000
frame = 1

[devices.ports.port-001.port_mac.last]
probe = "lldp-heard"
iface = "eth1"
at_ns = 1764000001000000000
frame = 1

[devices.ports.port-001.on.eth1]
frames = 1
last = [1764000001000000000, 1]

[[devices]]
mac = "02:00:00:00:00:02"
alias = []
vendor = []
role = []
instance = []
chassis = []
sys_name = []
sys_desc = []
sys_caps = []
mgmt_ips = []
arp_ip = []
heard = ["dcp-heard"]
heard_on = ["eth1"]
first_ns = 1764000003000000000
last_ns = 1764000003000000000
frames = 1
by_source = false

[[devices.name]]
value = "bomba-1"
count = 1

[devices.name.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[devices.name.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[[devices.ip]]
count = 1

[devices.ip.value]
ip = "192.168.0.11"
mask = "255.255.255.0"
gateway = "0.0.0.0"

[devices.ip.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[devices.ip.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[[devices.ip_status]]
count = 1

[devices.ip_status.value]
set = 1
conflict = false

[devices.ip_status.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[devices.ip_status.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[[devices.pn_id]]
value = [42, 261]
count = 1

[devices.pn_id.first]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[devices.pn_id.last]
probe = "dcp-heard"
iface = "eth1"
at_ns = 1764000003000000000
frame = 3

[devices.ports]

[[conflicts]]
kind = "duplicate-name"
value = "bomba-1"
macs = ["02:00:00:00:00:01", "02:00:00:00:00:02"]

[topology]
rings = []
components = [["dev:02:00:00:00:00:01", "if:eth1"], ["dev:02:00:00:00:00:02"]]
notes = []

[[topology.nodes]]
id = "dev:02:00:00:00:00:01"
kind = "device"
label = "bomba-1"
mac = "02:00:00:00:00:01"
ports = ["port-001"]
rings = []

[[topology.nodes]]
id = "dev:02:00:00:00:00:02"
kind = "device"
label = "bomba-1"
mac = "02:00:00:00:00:02"
ports = []
rings = []

[[topology.nodes]]
id = "if:eth1"
kind = "interface"
label = "eth1"
ports = []
rings = []

[[topology.links]]
confidence = "heard"
witnesses = 1

[topology.links.a]
node = "dev:02:00:00:00:00:01"
port = "port-001"

[topology.links.b]
node = "if:eth1"
port = ""
```

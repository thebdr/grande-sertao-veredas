# The bench file: `veredas-bench`, version 1

A bench is the plant a consumer meets before the real one (`docs/veredas-plan.md`,
"veredas-maquinar: the bench"): the log emitters, the OPC UA, Modbus and EtherNet/IP servers, the
PROFINET devices, the emulated EtherCAT segments, the scenes and the models behind the devices'
points, and the telegram channels it talks to, each configured, none programmed. One bench file lists them all by name, and the
device's own files sit beside it: core's channel file (`docs/channel-schema.md`), the catalog
(`docs/catalog-schema.md`) and, when a controller rides a channel, its rules file
(`docs/controller-schema.md`). `veredas_maquinar::bench::load` reads the bench file and every
file it points at and checks each part as its own module checks it, so a bench file that loads
is a bench every part of which can start; `Bench::start` then runs every part behind one handle,
emitters first, then servers (OPC UA, then Modbus, then EtherNet/IP), then PROFINET devices, then
EtherCAT segments, then scenes, then their models, then channels, each kind in the file's order
(TOML groups a kind's tables under one key, so a file keeps no order across kinds), and the executable
`veredas-maquinar <bench.toml>` runs it from the command line (`--check` reads, checks and
prints the plan without starting anything). No device is named anywhere: a
plant's bench is a folder of files like the ones below, kept outside the suite's code.

Every plain-text config and project file of Veredas 2 follows this pattern (`docs/questions.md`
Q10, "TOML, suite-wide"): TOML, a `format` and a `version` first, documented defaults, a
misspelled key refused, an unknown format or version refused rather than guessed.

## The worked example: a scanner's log, a controller's OPC UA server, and a machine talking to its unit

A scanner writes a line every 200 ms into a daily file that rotates every 1000 lines; a
controller serves two tags over OPC UA on a port the machine chooses, a speed drawn between 1.0
and 2.0 every 500 ms and a run flag a client may write, and reacts to the flag written true by
setting the speed to 2.0 after 100 ms; and the machine of the controller file's worked example
listens on port 5020 for the unit of that example, which dials it. The channel file, the
catalog and the two rules files are those of the channel and controller references, written
beside this file under the names it gives; the emitter writes under `logs`, beside it too.
`crates/veredas-maquinar/tests/bench_file.rs` writes those files, loads this block, starts it,
sees every part do its work, and runs the executable on it; it holds a copy of this text, so an
edit here is an edit there.

```toml
format = "veredas-bench"
version = 1
seed = 7
out_dir = "logs"

[[emitter]]
name = "scanner"
template = "{ts} SCAN code={code} weight={weight}"
ts_format = "%Y-%m-%d %H:%M:%S%.3f"
values = [ { name = "code", low = 1000, high = 9999 }, { name = "weight", low = 0.1, high = 30.0, decimals = 2 } ]
interval_ms = 200
file = "%Y%m%d_scanner{n}.log"
max_lines = 1000

[[opc]]
name = "plc"
port = 0
tags = [
  { id = "speed", type = "float", value = "1.5", randomize = { low = 1.0, high = 2.0, decimals = 2, interval_ms = 500 } },
  { id = "run", type = "bool", access = "read-write" },
]
reactions = [ { on_write = "run", equals = "true", tag = "speed", then = [ { after_ms = 100, set = "2.0" } ] } ]

[[channel]]
name = "machine"
role = "listen"
port = 5020
channel = "channel.toml"
catalog = "catalog.toml"
controller = "machine.toml"

[[channel]]
name = "unit"
role = "connect"
port = 5020
channel = "channel.toml"
catalog = "catalog.toml"
controller = "unit.toml"
messages = [ { message = "status", fields = [ { name = "running", value = 1 }, { name = "speed", value = 50 } ] } ]
```

## A second example: a drive on Modbus/TCP

A drive answers as unit 1 on port 5502: a client writes its speed setpoint into a holding register
and starts it with a coil; the plant side answers with the speed reached, a float over two input
registers, and a ready input. `crates/veredas-maquinar/tests/modbus_server.rs` loads this block on
a port the machine chooses, drives it with a Modbus client the bench did not write, and sees the
plant side meet it.

```toml
format = "veredas-bench"
version = 1

[[modbus]]
name = "drive"
port = 5502
unit = 1
points = [
  { name = "setpoint", table = "holding-register", address = 0 },
  { name = "run", table = "coil", address = 0 },
  { name = "speed", table = "input-register", address = 0, type = "f32" },
  { name = "ready", table = "discrete-input", address = 0, value = "true" },
]
```

## A third example: an I/O block on EtherNet/IP

An I/O block answers on the standard ports: a scanner opens a Class 1 connection naming its
configuration, output and input assemblies, writes a start bit at its RPI, and reads back a ready
bit and a speed the plant side sets. `crates/veredas-maquinar/tests/ethernetip_adapter.rs` loads this
block on ports the machine chooses, opens the connection with codecs the bench did not write, and
sees the plant side meet it.

```toml
format = "veredas-bench"
version = 1

[[ethernetip]]
name = "remote-io"
bind = "127.0.0.1"
port = 44818
io_port = 2222
identity = { product_name = "bench remote io" }
assemblies = [ { instance = 100, size = 4 }, { instance = 150, size = 2 }, { instance = 151, size = 0 } ]
connections = [ { config = 151, output = 150, input = 100 } ]
points = [
  { name = "start", assembly = 150, byte = 0, bit = 0 },
  { name = "ready", assembly = 100, byte = 0, bit = 0 },
  { name = "speed", assembly = 100, byte = 2, type = "u16" },
]
```

## A fourth example: a PROFINET device not yet named

A device answers on a network interface of its own, not yet named, as it comes from its box: an
engineer's tool finds it by an Identify of every device, names it and gives it an address with a
Set, and finds it again by its new name. `crates/veredas-maquinar/tests/profinet_device.rs` runs this
block inside a private network of the test's own, the device on one end of a veth pair and the tool
on the other.

```toml
format = "veredas-bench"
version = 1

[[profinet]]
name = "fresh"
interface = "b0"
vendor_id = 7
device_id = 9
lldp_interval_ms = 1000
```

## A fifth example: an EtherCAT segment of three devices

A coupler, an output terminal of two bytes and an input terminal of one answer a master on a
network interface, as a segment answers it from the far end of its cable: the master finds them,
reads their EEPROMs, gives them addresses, takes them to OP and exchanges their process data.
`crates/veredas-maquinar/tests/ethercat_segment.rs` runs this block inside a private network of the
test's own, the segment on one end of a veth pair and the MainDevice core's master stands on
(ethercrab) on the other.

```toml
format = "veredas-bench"
version = 1

[[ethercat]]
name = "line"
interface = "b0"
devices = [
  { name = "coupler", product_code = 1, product_name = "bench coupler" },
  { name = "out16", product_code = 2, product_name = "bench outputs", outputs = [ { index = 0x7000, subindex = 1, bits = 8 }, { index = 0x7010, subindex = 1, bits = 8 } ] },
  { name = "in8", product_code = 3, product_name = "bench inputs", inputs = [ { index = 0x6000, subindex = 1, bits = 8 } ] },
]
```

## A sixth example: a conveyor line behind a Modbus server

A controller's I/O on a Modbus server: it runs a conveyor at a setpoint, reads the conveyor's
speed back, sees a light barrier across the belt, extends a pusher and reads its end switches.
Behind the points stands the scene of `docs/cena-schema.md`'s worked example, saved beside the
bench file as `line.cena.toml`: the conveyor's drive ramps at 100 units a second toward its
setpoint and moves the belt 0.01 m/s for each unit (50 is the belt's 0.5 m/s), the barrier is
normally closed (1 while the beam is clear), and the pusher takes half a second from one end to
the other. `crates/veredas-maquinar/tests/reflexo.rs` runs this block with a Modbus client the
bench did not write, as a controller would drive the line.

```toml
format = "veredas-bench"
version = 1

[[modbus]]
name = "plc-io"
port = 0
points = [
  { name = "conveyor_run", table = "coil", address = 0 },
  { name = "conveyor_setpoint", table = "holding-register", address = 0, value = "50" },
  { name = "conveyor_speed", table = "input-register", address = 0 },
  { name = "conveyor_running", table = "discrete-input", address = 0 },
  { name = "barrier", table = "discrete-input", address = 1 },
  { name = "pusher_extend", table = "coil", address = 1 },
  { name = "pusher_back", table = "discrete-input", address = 2 },
  { name = "pusher_out", table = "discrete-input", address = 3 },
]

[[fisica]]
name = "line"
scene = "line.cena.toml"

[[reflexo]]
name = "line-models"
scene = "line"
conveyor = [ { name = "infeed", belt = "infeed", metres_per_unit = 0.01, run = "plc-io.conveyor_run", setpoint = "plc-io.conveyor_setpoint", accel = 100, decel = 100, speed = "plc-io.conveyor_speed", running = "plc-io.conveyor_running" } ]
sensor = [ { name = "B1", sensor = "B1", point = "plc-io.barrier", normally_closed = true } ]
actuator = [ { name = "pusher", mover = "pusher", extend = "plc-io.pusher_extend", retracted = { at = [3.5, -0.4, 0.95] }, extended = { at = [3.5, 0.25, 0.95] }, stroke_ms = 500, retracted_switch = "plc-io.pusher_back", extended_switch = "plc-io.pusher_out" } ]
```

## Keys

Each key states in parentheses that it is required, or the value it takes when left out. Every
part has a name, exact, with no surrounding whitespace, no dot (a model names a point
`<part>.<point>`) and no control character (a name is one line of the plan and the status), and no
two parts share one. A path is relative to the bench file's folder.

**The top level**.
- `format` (required): always `"veredas-bench"`.
- `version` (required): this reader knows `1`. A file of another format or an unknown version is
  refused, never guessed.
- `seed` (none): the seed of the emitters' and the channels' draws, for a run that repeats:
  each emitter without a seed of its own draws from this seed plus its index, each channel and
  its controller from this seed; absent, an emitter and a channel take the clock, and a controller
  its rules file's `seed`, else the clock. The bench gives an OPC UA server, and a PROFINET device
  without a `seed` of its own, none: each takes the clock.
- `out_dir` (`"."`): where the emitters write, created when there is an emitter.
- `emitter` (none): `[[emitter]]` tables.
- `opc` (none): `[[opc]]` tables.
- `modbus` (none): `[[modbus]]` tables.
- `ethernetip` (none): `[[ethernetip]]` tables.
- `profinet` (none): `[[profinet]]` tables.
- `ethercat` (none): `[[ethercat]]` tables.
- `fisica` (none): `[[fisica]]` tables.
- `reflexo` (none): `[[reflexo]]` tables.
- `channel` (none): `[[channel]]` tables.

**An emitter** (`[[emitter]]`): a log a plant would write, line by line into rotating files.
- `name` (required): the part's name.
- `template` (required): the line, `{ts}` the timestamp and `{name}` a value, anything else
  verbatim.
- `ts_format` (required): the `chrono` format `{ts}` renders with.
- `values` (none): the values drawn fresh for every line, as inline tables.
- `interval_ms` (required): milliseconds between lines; `0` writes as fast as it can.
- `file` (required): the file name, `chrono` specifiers and `{n}` for the rotation index; a name
  that is empty, has a path separator or is reserved by the platform is refused.
- `max_lines` (`0`): rotate to the next index after this many lines; `0` never rotates on
  count.
- `encoding` (`"utf-8"`): `"utf-8"` or `"utf-16le"`, the latter with a byte-order mark.
- `seed` (none): a fixed seed replays the same values at the emitter's place among the file's
  emitters (its index picks the draw's stream); absent, the bench's seed plus the emitter's index,
  else the clock.

**A value** (an entry of `values`): drawn within `[low, high]`, rendered to `decimals`.
- `name` (required): the placeholder's name.
- `low` (required), `high` (required): finite, `low` not above `high`.
- `decimals` (`0`): the places rendered, at most 15.

**An OPC UA server** (`[[opc]]`): a controller's tags on a real endpoint.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the server binds and advertises, never `0.0.0.0`.
- `port` (`4855`): `0` lets the machine choose a free port.
- `user` (none): a user the server accepts by name and password; anonymous is always accepted.
- `max_nodes_per_read` (none): the most nodes one read may carry; absent, the stack's own limit.
- `max_sessions` (none): the most sessions held at once.
- `tags` (none): the tags, as inline tables.
- `reactions` (none): the reactions, as inline tables.

**A user** (the value of `user`).
- `name` (required), `password` (required): shown in no status or log.

**A tag** (an entry of `tags`).
- `id` (required): the string identifier in the bench's namespace; the node id is
  `ns=<index>;s=<id>`.
- `name` (`id`): the browse and display name a client sees.
- `type` (required): `int` (an Int32), `float` (a Double), `bool` or `text`.
- `access` (`"read"`): `read`, `write` (a command the controller takes but does not report) or
  `read-write`.
- `value` (the type's zero): the initial value as text, parsed per the type.
- `randomize` (none): a fresh value within a range on an interval; a text tag takes none.
- `ramp` (none): a value stepping through a range on an interval and wrapping, for plant
  motion (a place along a loop, a level filling); an int or a float, never with `randomize`.

**A draw** (the value of `randomize`).
- `low` (`0`), `high` (`0`): the range; an int draws the whole numbers in it, a bool flips.
- `decimals` (`0`): a float's places.
- `interval_ms` (`1000`): between draws; at least the server's tick of 20 ms.

**A ramp** (the value of `ramp`): each step adds `step` to what the tag holds, wrapped by the
range's width while past `high`; a value outside the range (a client's write, a plant-side set)
starts again at `low`, one inside it moves the ramp on from there.
- `low` (required), `high` (required): the range, finite, `low` below `high`, its width finite;
  the tag's initial value inside it.
- `step` (`1`): above 0 and at most the range's width (with places, counted in their units); an
  int's range and step are whole numbers.
- `decimals` (none): a float's places, which a float ramp names (an int takes none): every step is
  counted in units of the last place, so the ramp reaches its high exactly, and `low`, `high` and
  `step` must be whole numbers of those units, small enough to count exactly (at most 2^51 of
  them: with fifteen places, a range within ±2.25).
- `interval_ms` (`1000`): between steps; at least the server's tick of 20 ms.

**A reaction** (an entry of `reactions`): what the plant does when a client writes a tag.
- `on_write` (required): the tag watched.
- `equals` (none): the value that fires it, as text parsed per the tag's type; absent, any
  change fires it. Either way only what a client wrote or the plant side set is a change, never a
  draw, a ramp's step or a reaction's step; a float holding NaN is no change from NaN.
- `tag` (`on_write`): the tag the steps set.
- `then` (required): the steps, as inline tables, each at its delay from the moment of firing
  (steps falling due in one tick land in the order of their delays).

**A step** (an entry of `then`).
- `after_ms` (`0`): the delay.
- `set` (required): the value set, as text parsed per the target tag's type.

**A Modbus server** (`[[modbus]]`): a device's four tables over Modbus/TCP: coils and discrete
inputs (bits), input and holding registers (16-bit words), a client writing the coils and the
holding registers only.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the server listens on.
- `port` (`502`): the protocol's port; on Linux a port below 1024 needs a privilege, so a bench
  there names another; `0` lets the machine choose a free port.
- `unit` (none): the unit identifier the device plays, a request to another unit answered with
  exception 0B as a gateway answers for a device that is not there, 0 and 255 included (the ones a
  client addressing a device on TCP directly sends: leave `unit` out for such a device); absent,
  every unit is the device.
- `max_connections` (none): the most connections held at once; one more is accepted and closed at
  once.
- `coils` (none), `discrete_inputs` (none), `input_registers` (none), `holding_registers` (none):
  a table's size, at most 65536, its addresses 0 to the size less one; absent, the table reaches
  the end of its last point, and holds nothing without one.
- `points` (none): the points, as inline tables.

**A point** (an entry of `points`): a named value in one of the tables; no two points of a table
share an address.
- `name` (required): the name the bench's handle reads and sets it by.
- `table` (required): `coil`, `discrete-input`, `input-register` or `holding-register`.
- `address` (required): the protocol's address, from 0, as a request carries it.
- `type` (`bool` in a bit table, `u16` in a register table): `bool` in a coil or a discrete input;
  `u16`, `i16`, `u32`, `i32` or `f32` (a single-precision float) in a register table, the last
  three over two registers.
- `words` (`"high-first"`): which of a two-register point's registers holds the high word,
  `high-first` (the one at its address) or `low-first`; a point of one register or a bit takes
  none. A register's own two bytes are always high byte first, as the protocol sends them.
- `value` (the type's zero): the initial value as text, parsed per the type.

**An EtherNet/IP adapter** (`[[ethernetip]]`): a device's assemblies over EtherNet/IP, explicit
messages on a TCP session and Class 1 cyclic I/O over UDP.
- `name` (required): the part's name.
- `bind` (`"127.0.0.1"`): the address the adapter listens on, IPv4 as a dotted quad (EtherNet/IP's
  items carry IPv4 addresses alone; an IPv6 address or a host name is refused).
- `port` (`44818`): the encapsulation port, TCP for sessions and UDP for a ListIdentity; `0` lets
  the machine choose, the same port for both.
- `io_port` (`2222`): the Class 1 port, UDP; `0` lets the machine choose, the port then given in
  every Forward_Open's reply.
- `identity` (the defaults below): what ListIdentity and the Identity object report.
- `rpi_min_us` (`1000`), `rpi_max_us` (`10000000`): the RPIs accepted, in microseconds.
- `max_sessions` (none): the most sessions registered at once.
- `max_connections` (none): the most Class 1 connections open at once.
- `assemblies` (none): the assemblies, as inline tables.
- `connections` (none): the Class 1 connections the adapter accepts, as inline tables.
- `points` (none): named values in the assemblies, as inline tables.

**An identity** (the value of `identity`).
- `vendor_id` (`0`), `device_type` (`12`), `product_code` (`0`): the Identity object's first three
  attributes, 12 a communications adapter.
- `revision` (`"1.1"`): `major.minor`, the major 1 to 127, the minor 1 to 255.
- `serial` (`0`): the serial number.
- `product_name` (`"veredas bench adapter"`): at most 32 ASCII characters.

**An assembly** (an entry of `assemblies`): an instance of the Assembly object, its data a run of
bytes, zeros at the start.
- `instance` (required): 1 to 65535, no two alike.
- `size` (required): its bytes, at most 65000.

**A connection** (an entry of `connections`): a Class 1 connection the adapter accepts, by the
three assemblies its Forward_Open's path names; the size each direction states counts the data,
the 16-bit sequence count, and the run/idle header when the direction carries one.
- `config` (required): the configuration assembly, named first; a Forward_Open's data segment is
  written into it.
- `output` (required): the assembly the adapter consumes (the originator's outputs, O→T).
- `input` (required): the assembly the adapter produces (the originator's inputs, T→O).
- `output_header` (`"run-idle"`): what the consumed data carries before its bytes: `run-idle` (a
  32-bit run/idle header) or `modeless`.
- `input_header` (`"modeless"`): the same for the produced data.

**An assembly point** (an entry of `points` of an adapter): a named value in an assembly's bytes,
little-endian as CIP carries it; no two points of an assembly share a byte, unless both are bools
on different bits.
- `name` (required): the name the bench's handle reads and sets it by.
- `assembly` (required): the assembly's instance.
- `byte` (required): the offset of its first byte.
- `bit` (none): a bool's bit, 0 to 7; a bool names one and no other type takes one.
- `type` (`bool` with a bit, `u8` without): `bool`, `u8`, `i8`, `u16`, `i16`, `u32`, `i32` or `f32`.
- `value` (the type's zero): the initial value as text, parsed per the type.

**A PROFINET device** (`[[profinet]]`): a device's discovery and naming on a network interface, raw
frames through aranha's link (Linux): a Hello at start, an Identify it matches answered within the
request's response delay, a Set of its name, its address or the identification signal applied and
each block answered, a Get answered, LLDP on its port; at its address, ARP and the record read of
its I&M0 and its port's data.
- `name` (required): the part's name.
- `interface` (required): the network interface it answers on, by name.
- `station` (`""`): its station name, the standard's rule for one; empty for a device not yet
  named, which answers no Identify by name.
- `alias` (none): its alias name, a port name, a dot and a station name (`port-001.switch-1`).
- `vendor` (`"veredas bench device"`): the vendor's text for its type, 1 to 255 visible ASCII characters.
- `vendor_id` (`0`), `device_id` (`0`): its vendor and device ids.
- `ip` (none): its IP suite, as an inline table; absent, 0.0.0.0 and not set.
- `hello` (`true`): a Hello at start.
- `lldp_interval_ms` (`5000`): between LLDP frames, up to 3600000 (802.1AB's hour); `0` sends none.
- `port` (`"port-001"`): its port's name, `port-xyz` or `port-xyz-abcde` (xyz 001 to 255, abcde
  five digits).
- `seed` (none): the seed of its answers' spread over a response delay; the clock when absent.
- `order_id` (`"veredas bench"`): its I&M0 order id, 1 to 20 visible characters.
- `serial_number` (`""`): its I&M0 serial number, up to 16 visible characters; empty, its MAC's
  twelve hex digits.

**An IP suite** (the value of `ip`).
- `address` (required), `mask` (required): dotted quads.
- `gateway` (`"0.0.0.0"`): a dotted quad.

**An EtherCAT segment** (`[[ethercat]]`): SubDevices answering a master's frames on a network
interface, raw frames through aranha's link (Linux), each frame passed through the devices in order
and sent back; functional, never exact in time.
- `name` (required): the part's name.
- `interface` (required): the network interface the master's frames arrive on, by name.
- `devices` (none): the SubDevices in the order the frames pass them, as inline tables; a segment
  holds at least one.

**A SubDevice** (an entry of `devices`): what its EEPROM says of it, and its process data.
- `name` (required): the name the bench's handle reads and sets it by.
- `vendor_id` (`0`), `product_code` (`0`), `revision` (`0`), `serial` (`0`): its identity.
- `product_name` (`"veredas bench terminal"`): the name its EEPROM gives a master, 1 to 255 ASCII
  characters.
- `alias` (`0`): its station alias.
- `outputs` (none), `inputs` (none): its process-data entries, as inline tables, at most 255
  entries and 1024 bytes each way.

**A process-data entry** (an entry of `outputs` or `inputs`).
- `index` (required), `subindex` (required): the object the entry maps.
- `bits` (required): its size, 1 to 64.

**A scene** (`[[fisica]]`): a plant's scene (`docs/cena-schema.md`), its bodies moved in real time
at its step.
- `name` (required): the part's name.
- `scene` (required): the path of the scene file.

**Its models** (`[[reflexo]]`): ready-made models making a device's points behave as the machine
behind them. A point is named `<part>.<point>`, a point of a Modbus server or an EtherNet/IP
adapter of the file; a bool point carries 0 or 1, a number is converted to the point's type (an
integer rounded and held to its range; NaN written as 0). A part's models step together before
each step of their scene from its first, in this order: drives, conveyors, actuators, sensors, each
kind in the file's order; two parts moving one scene step one after the other in the file's order;
without a scene (drives alone) every 4 ms. A point is written by one model at most. A step in which a model cannot read or write a point is counted once, its last error named.
- `name` (required): the part's name.
- `scene` (none): the `[[fisica]]` part whose scene the models move; conveyors, sensors and
  actuators need one.
- `drive` (none), `conveyor` (none), `sensor` (none), `actuator` (none): the models, as inline
  tables.

**A drive** (an entry of `drive`): a speed ramped toward a setpoint while it runs, in the drive's
own unit.
- `name` (required): the model's name, unique in its part.
- `run` (required): a bool point read, run while 1.
- `setpoint` (required): a number point read, the speed wanted.
- `reverse` (none): a bool point read, the setpoint negated while 1.
- `accel` (required), `decel` (required): units a second the speed gains away from 0, and loses
  toward 0 or a smaller setpoint; finite and over 0. Through 0 when the sign changes, in one ramp:
  the step that passes 0 carries on at the acceleration. The setpoint landed on exactly, at the
  first step at or after its time; an infinite setpoint ramped toward at the acceleration.
- `tolerance` (`0`): how near the setpoint the speed counts as at it.
- `speed` (none): a number point written, the speed.
- `running` (none): a bool point written, 1 while the speed is not 0.
- `at_speed` (none): a bool point written, 1 while `run` is 1 and the speed within the tolerance of
  the setpoint (negated while reversed), a setpoint of 0 included.

**A conveyor** (an entry of `conveyor`): a drive whose speed moves a belt of the scene.
- `name` (required), `run` (required), `setpoint` (required): as a drive's.
- `reverse` (none), `accel` (required), `decel` (required), `tolerance` (`0`): as a drive's.
- `speed` (none), `running` (none), `at_speed` (none): as a drive's.
- `belt` (required): the scene's belt.
- `metres_per_unit` (required): the belt's surface speed in m/s for one unit of the drive's speed,
  not 0; the belt's speed held to the scene's 100 m/s.

**A sensor model** (an entry of `sensor`): a scene sensor reported on a point.
- `name` (required): the model's name.
- `sensor` (required): the scene's sensor.
- `point` (required): a bool point written, 1 while a body or a mover is inside.
- `normally_closed` (`false`): the point 0 while something is inside, 1 while it is empty.
- `on_delay_ms` (`0`), `off_delay_ms` (`0`): how long the scene's sensor stays occupied, or empty,
  without a break before the point says so, up to 600000; the first step at or after the delay.

**An actuator** (an entry of `actuator`): a cylinder moving a mover of the scene between two poses.
- `name` (required): the model's name.
- `mover` (required): the scene's mover.
- `extend` (required): a bool point read, extend while 1, retract while 0.
- `retracted` (required), `extended` (required): its two poses.
- `stroke_ms` (required): the time from one end to the other, 1 to 600000; the mover moves along
  the straight line between the poses at an even speed, its angles with it, and is at an end at the
  first step at or after its time.
- `retracted_switch` (none), `extended_switch` (none): bool points written, 1 at that end.

**A pose** (`retracted`, `extended`): as a scene file states one.
- `at` (required): the centre, in metres.
- `yaw` (`0`), `pitch` (`0`), `roll` (`0`): the angles, in degrees.

**A channel** (`[[channel]]`): one telegram link, listening or dialling.
- `name` (required): the part's name.
- `role` (required): `listen` (wait for the peer to connect) or `connect` (dial it).
- `host` (`"127.0.0.1"`): the address to bind, or the host to dial.
- `port` (required): the port to bind or dial.
- `channel` (required): the path of core's channel file.
- `catalog` (required): the path of the catalog file; its ids must ride in the channel file's
  framing, as the catalog's own check says (`docs/catalog-schema.md`, `id_field`).
- `controller` (none): the path of the controller rules file, read against the catalog; none
  for a channel that only sends its own messages.
- `messages` (none): the channel's own messages, as inline tables.

**A message** (an entry of `messages`): a message of the catalog the channel sends on an
interval, or on demand.
- `message` (required): its name in the catalog.
- `every_ms` (`0`): the interval; `0` is on demand only.
- `fields` (none): its fields, one for every name the message writes, each a field of the
  controller file's reference (`docs/controller-schema.md`, "A field") with the sources a plain
  channel has: no `from`, no `register`, no `decide`.

A misspelled key in any table, a part without a name or with a name another part has, an
emitter's file name or timestamp format that is bad, a value's range that is empty or not
finite, anything the OPC UA or the Modbus server's own checks refuse (a point over another or past
its table's end, a type its table cannot hold, a value that does not parse) or the EtherNet/IP
adapter's (an IPv6 bind, an assembly declared twice, a connection naming an assembly there is not, a point past
its assembly's bytes or over another) or the PROFINET device's (an empty interface, a station name
the standard refuses, a port not named `port-xyz` or `port-xyz-abcde`, an alias not a port and a station, an order id or a serial number I&M0 cannot carry) or the EtherCAT segment's (an empty interface, no device, a device's
name empty, padded or twice, a product name not 1 to 255 ASCII characters, an entry of 0 or over 64
bits, more than 255 entries or 1024 bytes a way), a scene file that cannot be read or does not load, a model whose scene, belt, sensor, mover or point is not there, a point of the other kind or written by two models, a ramp, a ratio, a stroke or a delay out of its range, a file a channel points at that cannot be read or does not load, an `id_field` that does not ride in the framing, a controller file that
does not load against the catalog, a message the catalog lacks or whose fields leave a name
unwritten or use a source a plain channel lacks, is refused when the bench file loads, with the
place named, before anything binds, opens or spawns. A part that cannot start (a port taken, a
listening channel's included, a folder that cannot be created, an interface that is not there)
stops the parts started before it and is named.

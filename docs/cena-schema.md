# The scene file: `veredas-cena`, version 1

A plant's scene for `fisica`, the bench's physics (`docs/maquinar-plan.md`, "`fisica`, as designed
before its code"): the shapes that stand still, the belts, the bodies that move, the movers the
plant side places and the sensors that report what is inside them. The bench moves the bodies;
Oficina's view draws them from the same file and the poses the bench sends.

Units are metres, kilograms and seconds. The z axis points up and gravity pulls along -z; a line
runs along x by habit, nothing requires it. Every part is a box but a ball sensor: `size` is its
full lengths along its own x, y and z, `at` its centre, and three angles in degrees turn it: `yaw`
about z, then `pitch` about the y the yaw left, then `roll` about the x the pitch left (an
intrinsic z-y-x rotation). A part's own +z is its top: a belt carries what lies on that face.

## A worked example

A carton rests at the head of a belt running at 0.5 m/s, a light barrier crosses the belt, and a
pusher waits beside it.

```toml
format = "veredas-cena"
version = 1
step_ms = 4
gravity = 9.81

[[fixed]]
name = "floor"
size = [10.0, 4.0, 0.1]
at = [3.0, 0.0, -0.05]

[[belt]]
name = "infeed"
size = [4.0, 0.6, 0.1]
at = [2.0, 0.0, 0.75]
speed = 0.5

[[body]]
name = "carton-1"
size = [0.3, 0.3, 0.3]
at = [0.3, 0.0, 0.95]
mass = 2.5

[[mover]]
name = "pusher"
size = [0.4, 0.05, 0.3]
at = [3.5, -0.4, 0.95]

[[sensor]]
name = "B1"
size = [0.01, 0.8, 0.05]
at = [2.0, 0.0, 0.9]

[[sensor]]
name = "S1"
radius = 0.05
at = [3.5, 0.25, 0.95]
```

The belt's top is at z = 0.8, so the carton (0.3 m high, centred at 0.95) stands on it. The belt
takes it to its own speed within a tenth of a second; the barrier `B1`, a beam 1 cm thick at
x = 2.0, sees it when its front edge reaches the beam, about 3.1 s in. The pusher, moved along +y
by the plant side, sweeps across the belt; `S1` sees the pusher's blade at the end of its stroke.

## The keys

**The file** (the top level):
- `format` (required): always `"veredas-cena"`.
- `version` (required): `1`.
- `step_ms` (`4`): the fixed step in milliseconds, 1 to 100. Every step moves the scene by this
  much time, and the bench runs one step per step of real time.
- `gravity` (`9.81`): gravity's pull along -z in m/s², 0 to 100.
- `fixed`, `belt`, `body`, `mover`, `sensor` (none): the parts, as arrays of tables.

Every part has a `name` (required), unique across the whole scene, not empty and without
surrounding whitespace; `at` (required, in metres, each coordinate within 10000 of the origin);
`yaw`, `pitch` and `roll` (`0`, degrees, any finite value). Boxes have a `size` (required, each
side from 0.001 to 10000 m: the engine rests a smaller part sunk into what it lies on). A contact's
friction is the mean of its two parts' frictions; a body falling onto a part rebounds a little
(from 1 m some 5 mm, from up to 1000 m some 6 cm), and no body moves faster than 400 m/s, the
engine's cap. The world computes in double precision, so a part far from the origin moves as one
near it, and alike at every step: the engine steps 4 ms at a time at most (a step of 100 ms is 25
of them), a mover carried to its pose evenly over them, and a body is swept against the fixed
parts and movers, so stacks stand, drops land and movers push at 100 ms as at 4.

**A fixed shape** (`[[fixed]]`): a box that never moves, a floor, a guide, a chute's side.
- `friction` (`0.6`): 0 to 10.

**A belt** (`[[belt]]`): a fixed box whose top face moves along its own x. A part lying on it is
carried by friction toward the belt's speed; a part touching its side is not.
- `speed` (`0`): the surface's speed along the belt's own x in m/s, within 100 either way; the
  plant side changes it while the scene runs.
- `friction` (`0.6`): 0 to 10.

**A body** (`[[body]]`): a box that falls, slides, is carried and pushed, the carton or the part.
- `mass` (`1`): in kilograms, from 0.000001 to 100000, spread evenly through the box.
- `friction` (`0.6`): 0 to 10.

**A mover** (`[[mover]]`): a box the plant side places, a pusher's blade or a lift's table. It
reaches each pose it is given at the next step and pushes what is in its way; nothing pushes it
and gravity leaves it where it is.
- `friction` (`0.6`): 0 to 10.

**A sensor** (`[[sensor]]`): a box or a ball that reports the bodies and movers inside it, a light
barrier's beam or a proximity switch's field. Fixed shapes and belts are never inside it.
- `size` or `radius` (one of them, required): a box's full lengths, or a ball's radius in metres
  (from 0.001 to 10000).

## What the reader refuses

Each with its place, for example `body[0] (`carton-1`).mass: 0 kg, not from 0.000001 to 100000`:
the header (not TOML, another format, another version, a key missing) as core's reader states it;
a key the reader does not know, in any table; `step_ms` outside 1 to 100; `gravity` outside 0 to
100 or not finite; a name empty, padded or already taken; a size, a centre, an angle, a mass, a
friction, a speed or a radius outside its range or not finite; a sensor with both `size` and
`radius`, or neither. A value of another kind than its key's (a negative or a text `step_ms`, a text for a
number) is refused as not TOML, the parser naming its key.

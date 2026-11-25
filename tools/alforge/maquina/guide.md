# The Veredas tester machine

This machine holds one suite release of Veredas 2, built from the repository by its recipe
(`docs/alforge-plan.md` in the release's docs). Nothing here needs building.

## What runs from the first boot
- **The bench** (`veredas-bancada.service`): `veredas-maquinar` playing the plant in
  `~/planta/bench.toml`: a controller on OPC UA (`opc.tcp://127.0.0.1:4855/`) whose parcels move
  along a loop and whose outlets fill, and a sorter writing its log under `~/planta/logs/`.
- **Supervisão** (`veredas-supervisao.service`): `veredas-supervisao` serving the plant
  `~/planta/supervisao.toml` on port 8902 of every interface of the machine: the browser opens
  it at login, and the host reaches it at `http://<this machine's address>:8902/`. The plant's
  writes are off: "Release" is offered and portal refuses it.

`systemctl status veredas-bancada veredas-supervisao` shows both; `journalctl -u veredas-supervisao`
shows what each printed. After editing a file of the plant, `sudo systemctl restart
veredas-bancada veredas-supervisao`.

## Where things are
- The release: `/opt/veredas/current/` (its executables in `bin/`, on every user's `PATH`; its
  manifest `release.toml`; `SHA256SUMS`, checked at every boot).
- The documents: `/opt/veredas/current/docs/`, the file references (`*-schema.md`) first.
- The plant: `~/planta/`, your own copy; the original is `/opt/veredas/current/planta/`.

## Running your own
Each executable reads its file and runs until its standard input closes (Ctrl+D in a terminal):
`veredas-maquinar <bench.toml>`, `veredas-portal <project.toml>`, `veredas-supervisao
<supervisao.toml>`; `--check` reads a file and prints its plan without starting anything.

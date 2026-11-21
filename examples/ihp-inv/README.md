# Example project: ihp-inv

Schematics linked with layouts (docs/LINKING.md) in IHP SG13G2. Needs the PDK:
`PDK_ROOT` pointing at the folder that holds `ihp-sg13g2` (or `pdk_root` in
`cdept.toml`), and KLayout (`cdept-klayout/build.sh`).

```
cdept.toml                      technology ihp-sg13g2, [klayout] tech = "sg13g2"
libs/cells/inv/schematic/       inverter: sg13_lv_pmos + sg13_lv_nmos, pins A Y VDD VSS
libs/cells/inv/symbol/          its symbol
libs/cells/inv/layout/inv.gds   generated from the schematic: two SG13_dev PCells (pmos,
                                nmos) bound to M2 and M1, four pins (Metal1.pin shapes
                                with their names on Metal1.text); then wired at A and
                                Y (scripts/wire-inv.py, the pins A and Y on those
                                wires), not at VDD and VSS
libs/cells/buf/schematic/       buffer: two instances of inv
libs/cells/buf/layout/buf.gds   generated: two references to inv's layout (x1, x2), four
                                pins
*/layout/*.gds.json             sidecars: references, the link (source, instances) and
                                the pins (docs/LINKING.md §9)
```

The layouts are what `cdept layout link` makes; the inverter is then wired a little
by `scripts/wire-inv.py` (gates joined in GatPoly with a contact to a Metal1 pad, the
drains joined in Metal1, the labels A and Y on those wires), so that the net check
(docs/LINKING.md §8) has opens to show: VDD and VSS are not wired.

## Try it

```sh
export PDK_ROOT=/opt/ext/OpenPDKs        # wherever ihp-sg13g2 is
cdept layout check-source cells inv      # in sync?
# change w=0.74u of M1 in libs/cells/inv/schematic/inv.sch, then
cdept layout check-source cells inv      # update M1: w 0.74u → 1u
cdept layout link cells inv              # the PCell follows
cdept layout pins cells inv              # the four pins, placed
cdept layout check-nets cells inv        # A, Y ok; VDD, VSS open (flylines in the GUI)
cdept layout check-nets cells buf        # the net between the two inverters is open
cdept layout lvs cells inv               # runs; does not match until it is wired
cdept gui                                # Layout › Link Panel, Pins Panel, Update
                                         # Connectivity, Nets panel, Probe Nets (N),
                                         # cross-probing (also inside x1 of buf)
```

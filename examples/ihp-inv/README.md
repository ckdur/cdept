# Example project: ihp-inv

Schematics linked with layouts (docs/LINKING.md) in IHP SG13G2. Needs the PDK:
`PDK_ROOT` pointing at the folder that holds `ihp-sg13g2` (or `pdk_root` in
`cdept.toml`), and KLayout (`cdept-klayout/build.sh`).

```
cdept.toml                      technology ihp-sg13g2, [klayout] tech = "sg13g2"
libs/cells/inv/schematic/       inverter: sg13_lv_pmos + sg13_lv_nmos, pins A Y VDD VSS
libs/cells/inv/symbol/          its symbol
libs/cells/inv/layout/inv.gds   generated from the schematic: two SG13_dev PCells (pmos,
                                nmos) bound to M2 and M1, pin labels on Metal1.text
libs/cells/buf/schematic/       buffer: two instances of inv
libs/cells/buf/layout/buf.gds   generated: two references to inv's layout (x1, x2)
*/layout/*.gds.json             sidecars: references and the link (source, instances)
```

The layouts are what `cdept layout link` makes; the devices are placed, not wired.

## Try it

```sh
export PDK_ROOT=/opt/ext/OpenPDKs        # wherever ihp-sg13g2 is
cdept layout check-source cells inv      # in sync?
# change w=0.74u of M1 in libs/cells/inv/schematic/inv.sch, then
cdept layout check-source cells inv      # update M1: w 0.74u → 1u
cdept layout link cells inv              # the PCell follows
cdept layout lvs cells inv               # runs; does not match until it is wired
cdept gui                                # Layout › Link Panel, cross-probing
```

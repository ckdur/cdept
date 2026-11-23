# Example project: ihp-inv

Schematics linked with layouts (docs/LINKING.md) in IHP SG13G2. Needs the PDK:
`PDK_ROOT` pointing at the folder that holds `ihp-sg13g2` (or `pdk_root` in a
`cdept.local.toml` next to `cdept.toml`), and KLayout (`cdept-klayout/build.sh`).

```
cdept.toml                      technology ihp-sg13g2, the library `cells`
tech.toml                       technology snapshot (docs/TECH.md): the PDK's xschemrc,
                                KLayout technology and decks, model corners, and the
                                PDK's libraries (sg13g2_pr, sg13g2_stdcell, …); made by
                                `cdept tech scan`, again by `cdept tech refresh`
tech.catalog.json               the lists behind it (layers, PCells, symbols, cells)
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
libs/cells/tb_inv*/             testbenches of the inverter (docs/SIMULATION.md, TESTS.md)
libs/cells/ota5t/               5-transistor OTA (PMOS pair, NMOS mirror, PMOS tail,
                                20 uA into ibias), symbol, and its testbench views:
                                ac_open, tran_step, dc_op, power, psrr, cmrr, ac_cload
                                and the suite (5 corners x 3 temperatures, 90 runs)
libs/cells/tb_ota5t*/           the OTA's testbench schematics: open loop (DC loop
                                through 1 TH, AC into inn through 1 F) and unity gain
libs/cells/flash3/              behavioural 3-bit flash ADC and its inl_dnl testbench
                                (schematic in libs/cells/tb_flash3)
templates/ac_load.toml          a project test template: AC response against the load,
                                with a [[sweep]] and optional Monte Carlo from parameters
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

## Simulate

```sh
cdept sim suite cells ota5t                 # the OTA over tt/ff/ss/fs/sf, -40/27/125 C:
                                            # gain, UGBW, phase margin, settling, PSRR, ...
cdept sim run cells ota5t --view ac_open    # open loop: 35.8 dB, UGBW 145 MHz, PM 59 deg (tt, 27 C)
cdept sim run cells ota5t --view ac_cload   # the load swept by the ac_load template
cdept sim run cells flash3 --view inl_dnl   # INL/DNL of the flash ADC
cdept sim suite cells tb_inv                # the inverter's tests
cdept gui                                   # Library Manager: double-click cells/ota5t/suite,
                                            # Run suite
```

Needs ngspice (`CDEPT_NGSPICE` or on `PATH`) and the xschem helper (`CDEPT_XSCHEM`).

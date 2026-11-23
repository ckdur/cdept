# Quickstart

This tutorial makes a project for the IHP SG13G2 PDK, adds a library with an inverter and
a testbench, netlists the inverter and runs the testbench at three corners. It uses the
command line; the same steps are available in the [GUI](gui.md). Every command below was
run as written; the output is trimmed.

You need CDePT, the xschem helper, ngspice, KLayout and a copy of the IHP PDK
([Installation](installation.md)). The cells come from `examples/ihp-inv` of the source tree.

Set the environment first. `PDK_ROOT` is the folder that holds `ihp-sg13g2`. The other
variables are only needed when the programs are not on `PATH` or next to `cdept`
([Environment variables](../reference/environment.md)):

```sh
export PDK_ROOT=/opt/ext/OpenPDKs
export CDEPT_XSCHEM=$HOME/cdept/target/debug/cdept-xschem
export CDEPT_NGSPICE=/opt/ext/OpenLane/bin/ngspice
export CDEPT_KLAYOUT=/opt/ext/OpenLane/bin/klayout
```

## 1. Create the project

```console
$ cdept init demo --technology ihp-sg13g2
scanning /opt/ext/OpenPDKs/ihp-sg13g2 …
created /home/user/demo/cdept.toml
PDK ihp-sg13g2 in /opt/ext/OpenPDKs (environment)
pdk      branch            dev
         commit            cb743f75aabc96e30450e9e53afc8d3c4867e5e2
         description       IHP SG13G2 130 nm SiGe BiCMOS
...
klayout  base_path         $PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech
         layer_properties  $PDK_ROOT/ihp-sg13g2/libs.tech/klayout/tech/sg13g2.lyp
         lvs               sg13g2
...
$ cd demo
$ ls
cdept.toml  tech.catalog.json  tech.toml
```

`cdept init` made a folder with three files. [`cdept.toml`](../reference/files/cdept-toml.md)
is the project: its name, the PDK it uses (`technology = "ihp-sg13g2"`) and the libraries,
none yet. [`tech.toml`](../reference/files/tech-toml.md) is the technology snapshot: the
scan read the PDK and recorded where each tool's set-up is, which model corners exist and
which libraries the PDK brings. `tech.catalog.json` holds the long lists behind it. The
scan starts KLayout to list the PDK's PCells; add `--files-only` to skip that.

Check what CDePT makes of it:

```console
$ cdept info
project      demo
file         /home/user/demo/cdept.toml
PDK          ihp-sg13g2  (project)
PDK_ROOT     /opt/ext/OpenPDKs  (environment)
snapshot     tech.toml  (PDK ihp-sg13g2)
xschem rc    /opt/ext/OpenPDKs/ihp-sg13g2/libs.tech/xschem/xschemrc
libraries    4 (0 with errors)
```

The four libraries are the PDK's (`sg13g2_pr`, `sg13g2_stdcell`, `sg13g2_io`,
`sg13g2_sram`), brought in by `tech.toml`. Read-only, and ready to use in schematics.

## 2. Look at the technology

```console
$ cdept tech notes
- devices: 2 device(s) netlist as a model no model library defines (inductor, inductor3)
- libraries: the PDK has two libraries named 'sg13g2_pr'; a project lists $PDK_ROOT/ihp-sg13g2/libs.tech/xschem/sg13g2_pr and not $PDK_ROOT/ihp-sg13g2/libs.ref/sg13g2_pr
...
```

`cdept tech show` prints the summary of the snapshot, and `cdept tech show <section>` one
section. `cdept tech notes` lists what the scan could not find or had to guess. Run
`cdept tech refresh` after the PDK changes. See [Technology](../concepts/technology.md).

## 3. Add a library and cells

```console
$ cdept lib add cells libs/cells
added cells (0 cells)
$ cdept cell new cells scratch --view schematic
created /home/user/demo/libs/cells/scratch/schematic/scratch.sch
```

`lib add` defined the library `cells` in `cdept.toml` and created its folder. `cell new`
made a cell with a first view, an empty schematic. In a library of the `cells` layout,
every cell is a folder and every view a folder inside it
([Project folders](../reference/files/project-folders.md)).

To have something to simulate, copy the inverter and its testbench from the examples:

```sh
cp -r $HOME/cdept/examples/ihp-inv/libs/cells/{inv,tb_inv} libs/cells/
```

```console
$ cdept ls cells
cells  /home/user/demo/libs/cells  [cells]
  inv                              layout schematic symbol
  scratch                          schematic
  tb_inv                           both dc_transfer mismatch montecarlo power schematic suite sweep testbench tran_delay vacask xyce_mc
```

CDePT found the new files by itself: a cell is its folder. The cell `tb_inv` has one
schematic (the testbench circuit) and several testbench views, each a folder with a
[`setup.toml`](../reference/files/setup-toml.md).

```console
$ cdept hier cells tb_inv
tb_inv
  inv (x1)
$ cdept used-by cells inv
cells/tb_inv/schematic  instance ×1 (x1)
```

## 4. Write a netlist

```console
$ cdept netlist cells inv -o inv.spice
$ cat inv.spice
**.subckt inv A Y VDD VSS
*.ipin A
*.opin Y
*.iopin VDD
*.iopin VSS
XM2 VDD A Y VDD sg13_lv_pmos w=1.5u l=0.13u ng=1 m=1 mm_ok=1
XM1 Y A VSS VSS sg13_lv_nmos w=0.74u l=0.13u ng=1 m=1 mm_ok=1
**.ends
.end
```

The netlist is written by xschem, through the helper, with the PDK's `xschemrc` in effect.
`-f` selects Spectre, Verilog, VHDL or tEDAx instead of SPICE; `--into-view` stores the
netlist as a view of the cell.

## 5. Run the testbench

The view `testbench` of `tb_inv` is a pulse on the input and a load on the output; its
setup asks for three corners and one measurement with a limit:

```toml
corners = ["tt", "ff", "ss"]
temperatures = [27]
save = ["v(in)", "v(out)"]
measures = [
  "tran tpd trig v(in) val=0.6 rise=1 targ v(out) val=0.6 fall=1",
]

[[analysis]]
kind = "tran"
step = "5p"
stop = "1.2n"

[[spec]]
name = "tpd"
max = 3e-10
```

Run it:

```console
$ cdept sim run cells tb_inv --view testbench
tt_27      running
ff_27      running
ss_27      running
tt_27      done     0.1s
ff_27      done     0.1s
ss_27      done     0.1s
point  result           tpd
tt_27  PASS      4.5138e-11
ff_27  PASS      3.8640e-11
ss_27  PASS      5.3923e-11

3 run(s): 3 pass, 0 fail, 0 error
index: /home/user/demo/simulations/results.db
```

CDePT netlisted the schematic, built one deck per corner and temperature from the
technology's model corners, ran ngspice on each in parallel, evaluated the measurement
and compared it with the spec. `--view` is needed because the cell has several
testbench views.

## 6. Look at the results

```console
$ cdept sim results cells tb_inv --view testbench
point  result           tpd
tt_27  PASS      4.5138e-11
ff_27  PASS      3.8640e-11
ss_27  PASS      5.3923e-11

3 run(s): 3 pass, 0 fail, 0 error
measure              n          min          max         mean          std
tpd                  3   3.8640e-11   5.3923e-11   4.5900e-11   7.6698e-12
```

The files of the runs are under `simulations/`:

```text
simulations/results.db
simulations/cells/tb_inv/testbench/netlist.spice
simulations/cells/tb_inv/testbench/tt_27/{deck.sp, out.raw, out.log, job.log, result.json}
simulations/cells/tb_inv/testbench/ff_27/...
simulations/cells/tb_inv/testbench/ss_27/...
```

`out.raw` is the waveform file; evaluate expressions on it with `cdept calc`
(`cdept calc --raw <file> --list` lists the vectors). The layout of the folder and the
files are described in [Results](../reference/files/results.md).

## 7. Open the project in the GUI

```sh
cdept -p demo
```

With no command, `cdept` opens the window on the project: the cells in the Library
Manager, a double click on `inv` opens its schematic in a tab, and the Simulation tab
runs `tb_inv`. See [The graphical interface](gui.md).

## Next

- [Run a testbench](../howto/run-testbench.md), [corners and sweeps](../howto/corners-and-sweeps.md),
  [Monte Carlo](../howto/monte-carlo.md)
- [Create a project for a PDK](../howto/new-project.md)
- [Run DRC and LVS](../howto/lvs-drc.md)

## See also

- [Introduction](introduction.md)
- [The command line](cli.md)
- [Command reference](../reference/cli.md)
- [Projects](../concepts/projects.md)

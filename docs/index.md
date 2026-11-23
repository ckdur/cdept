# CDePT

**CDePT** (Chip Design Project Tool) is a project manager and editor shell for analog and
mixed-signal chip design on open PDKs. It keeps schematics, layouts, netlists, HDL and
testbenches of a design together as *libraries of cells with views*, and starts the
open-source tools that work on them: xschem, KLayout, Magic, netgen, ngspice, Xyce,
VACASK, Qucs-S, Yosys and OpenROAD. The tools run as helper processes or in batch; CDePT
itself is MIT licensed and never links GPL code.

This manual has four parts. Start with the **User guide**; the **Concepts** pages say how
CDePT thinks about a design; the **How-to** pages are short recipes; the **Reference** is
where the file formats, the command line and the environment variables are defined.

The tools themselves are not documented here. Wherever a tool is involved, the manual says
what CDePT does with it and links to the page of the tool ([External tools](tools.md)).

```{toctree}
:caption: User guide
:maxdepth: 2

guide/introduction
guide/installation
guide/quickstart
guide/gui
guide/cli
```

```{toctree}
:caption: Concepts
:maxdepth: 2

concepts/projects
concepts/technology
concepts/simulation
tools
```

```{toctree}
:caption: How-to
:maxdepth: 1

howto/new-project
howto/run-testbench
howto/corners-and-sweeps
howto/monte-carlo
howto/suites
howto/custom-test-template
howto/other-simulators
howto/lvs-drc
```

```{toctree}
:caption: Reference
:maxdepth: 2

reference/files/index
reference/cli
reference/environment
reference/errata
```

```{toctree}
:caption: Design notes and tool pages
:maxdepth: 1

design
```

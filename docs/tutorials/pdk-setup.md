# Set up a project for a PDK

<!-- Made from cdept-gui/tutorials/pdk-setup.toml by `cdept tutorial pdk-setup --print`. Edit that file, not this one. -->

You make a CDePT project for a process design kit (PDK) and check what CDePT read
out of it: where the tools' set-up files are, which model corners exist and which
device libraries you get. Every analog design starts here. The text uses IHP SG13G2;
sky130 works the same way.

About 10 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial pdk-setup
```

## Before you start

- CDePT built from its source tree: `cargo build && cargo build -p cdept-xschem`.
  Where this says `cdept`, use `target/debug/cdept` unless you have installed it.
- A copy of a PDK on this machine. `PDK_ROOT` names the folder that *holds* the PDK,
  not the PDK itself: with the PDK in `/opt/ext/OpenPDKs/ihp-sg13g2`, run
  `export PDK_ROOT=/opt/ext/OpenPDKs` in the shell before starting CDePT.
  Without it the tutorial still works; you name the folder in the wizard.
- Optional: KLayout (to list the PDK's parameterised cells) and the xschem helper (to
  evaluate the PDK's xschemrc). The first step shows whether CDePT found them.

## 1. What you are going to do

A PDK (process design kit) is the foundry's package for one process: the devices,
their simulation models, the layers, and the set-up of each tool.

CDePT reads it once, when the project is made, and keeps what it found next to the
project. After that every tool CDePT starts is set up from there.

How this guide works: it outlines the thing to use in orange and ticks a step off when
you have done it. You do the clicking. You can drag this window out of the way, and
click any step in the list to read it again.

Found on this machine:

## 2. Open the New Project window

Open the File menu and click "New Project…".

## 3. Choose the project folder

A CDePT project is a folder. Type this into "Folder", replacing what is there:

    ~/cdept-tutorial/my-first-chip

The folder is created for you. If it already holds a project (you did this tutorial
before), use another name. "Name" can stay empty: the folder's name is used.

## 4. Choose the PDK

"Technology (PDK)" lists the PDKs CDePT found: name, family, version and the folder
that holds each. Select ihp-sg13g2 (or the PDK you work in).

Is yours not listed? CDePT looks in $PDK_ROOT, ~/.ciel, ~/.volare and the system's
share/pdk folders. Type the folder that holds your PDKs into "Other folder", then
press "Add". For example:

    /opt/ext/OpenPDKs

CDePT remembers that folder.

Leave "Scan files only" unticked: the full scan also asks KLayout and xschem.

## 5. Scan the PDK

Click "Next ›". CDePT now reads the PDK's files. This takes a few seconds. Nothing is
written yet, and nothing in the PDK is ever changed.

## 6. Read what the scan found

When the scan is done, the window shows its summary. Check that it makes sense:

- the lines for KLayout, Magic, netgen and xschem name files of your PDK;
- there is a default model corner (tt for IHP SG13G2);
- "Notes of the scan", at the bottom, lists what CDePT could not find or had to
  guess. A few notes are normal. They are worth reading once.

If a tool you need has no line, the PDK has no set-up for it, or CDePT does not
understand its layout.

## 7. Pick the PDK's libraries

The PDK brings libraries: device symbols (the primitives: transistors, resistors,
capacitors), standard cells, IO cells, memories. A ticked library appears in your
project, read-only.

For analog work you need the primitives (sg13g2_pr in IHP SG13G2). Test and example
libraries start unticked. Keeping the defaults is fine.

## 8. Create the project

Click "Create". CDePT writes the project file and the technology snapshot into your
folder, and opens the project.

## 9. Open the Technology tab

Your project is open. The toolbar shows its name and the PDK in use.

Now look at what was stored. Open the Project menu and click "Technology…".

## 10. Check the model corners

Click the "Models" page.

It lists the simulator's model files and the corners: tt is typical, ff fast, ss slow,
sf and fs mixed; "mc" or "mm" sets are for Monte Carlo and mismatch. Your testbenches
will choose among these names, so you never write .lib lines by hand.

## 11. Read the notes

Click the "Notes" page. These are the scan's notes again. If something behaves oddly
later (a device without a layout cell, a library you expected), look here first.

The other pages are for reference: "Layers", "Layout tools" (DRC and LVS decks),
"Devices" (which symbol becomes which layout cell) and "Libraries".

## 12. Find the PDK's devices

The Libraries panel on the left lists your project's libraries. A new project has
only the PDK's, marked with a lock because they are read-only.

Open one (sg13g2_pr holds the devices of IHP SG13G2) and click a cell, for example
sg13_lv_nmos. The Inspector on the right shows what the cell is.

## 13. What you now have

Your project is in <your project folder>:

- cdept.toml: the project (name, PDK, your libraries). Commit it.
- tech.toml: the technology snapshot. Commit it. It stores paths as $PDK_ROOT/…, so
  it works on other machines.
- tech.catalog.json: the long lists behind it (layers, symbols). It can be made again.
- cdept.local.toml: only when this machine needs it, for where the PDK is. Do not
  commit it.

The same from a terminal:

    cdept -p ~/cdept-tutorial/my-first-chip info
    cdept -p ~/cdept-tutorial/my-first-chip tech notes

When the PDK is updated, "cdept info" warns that the snapshot is out of date, and
"cdept tech refresh" makes it again.

Open this project later with File > Open Project…, or:

    cdept -p ~/cdept-tutorial/my-first-chip

# Inverter 1: your library and your first cell

<!-- Made from cdept-gui/tutorials/inv-library.toml by `cdept tutorial inv-library --print`. Edit that file, not this one. -->

You make a library of your own in a project, and in it a cell "inv" with an empty
schematic. Designs in CDePT are kept as libraries of cells, and each cell has views
(schematic, symbol, layout, testbenches).

About 5 minutes. To be guided through it inside CDePT:

```sh
cdept tutorial inv-library
```

## Before you start

- The tutorial [Set up a project for a PDK](pdk-setup.md), or a project of your own
  for IHP SG13G2.
- CDePT built with the xschem helper (`cargo build -p cdept-xschem`).

## 1. Get a project to work in

Either open the project you made in "Set up a project for a PDK" (File > Open
Project…) and press Continue, or let the guide make a fresh one with the button
below. That takes a few seconds: it reads the PDK.

The names used from here on are fixed (a library "work", a cell "inv"), so that the
guide can follow what you do.

## 2. Open the Library Manager

The Library Manager shows your design in three columns: libraries, the cells of the
selected library, and the views of the selected cell. Open it with Tools > Library
Manager.

The libraries with a lock are the PDK's. You cannot change them.

## 3. Make a library

Press "+ Library" in the toolbar.

## 4. Name it work

Type this as the name, then press "Create":

    work

The folder follows the name (libs/work, inside your project) and is created.

## 5. Select your library

In the first column, click "work". New cells go into the selected library.

## 6. Make a cell

Press "+ Cell".

## 7. Name it inv

Type this as the name:

    inv

Leave "First view" at the xschem schematic, then press "Create".

## 8. Open the schematic

Select the cell "inv" in the second column. Then double-click its view "schematic"
in the third. It opens in the embedded xschem, empty.

## 9. What you now have

In your project folder there is now:

    libs/work/inv/schematic/inv.sch

A library is a folder, a cell a folder in it, and each view a folder in the cell.
cdept.toml lists the library. It is all plain text and files, ready for git.

Next: "Inverter 2: draw the schematic".

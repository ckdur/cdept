# Writing a tutorial

A tutorial is one TOML file, `cdept-gui/tutorials/<id>.toml`. The guided version in the
GUI and the written page `docs/tutorials/<id>.md` both come from it.

## The file

```toml
schema = "cdept-tutorial"
version = 1
id = "pdk-setup"
title = "Set up a project for a PDK"
goal = "What the reader has at the end."
minutes = 10
before = "What to have before starting."

[[step]]
id = "open-wizard"
title = "Open the New Project window"
text = """
Paragraphs. A line indented by four spaces is a command, with a copy button.
"""
point = [
    { at = "menu.file.new_project", say = "Click \"New Project…\"" },
    { at = "menu.file", say = "Open the File menu" },
]
done_when = ["wizard.open"]
```

| Key of a step | Meaning |
|---|---|
| `point` | What to outline. The first entry that is on screen wins, so a menu entry comes before its menu. |
| `point[].unless` | The entry is not pointed at while this fact holds. |
| `done_when` | The step is ticked off when any of these facts holds. |
| `continue_button` | The step has a Continue button, for things to read. |
| `continue_needs` | Continue waits for all of these facts. A `point` with the same name as one of them is hidden once it holds. |
| `needs_hint` | Why Continue is greyed out. |
| `lost_without`, `back_to`, `lost_says` | When none of the `lost_without` facts holds, the user has left the place of the step (closed the dialog): the guide goes back to the earlier step `back_to` and says `lost_says`. |
| `status` | Lines about this machine: `pdk_root`, `klayout`, `xschem`. |

A step needs `done_when` or `continue_button`. In `text`, `{project}` becomes the folder
of the open project.

The guide never clicks or types for the user.

## Anchors and facts

A tutorial can only point at widgets that carry a name (an *anchor*) and ask about
things the program reports (a *fact*). The lists are `ANCHORS` and `FACTS` in
`cdept-gui/src/guide/mod.rs`. A new tutorial usually needs a few more:

- anchor: `guide::anchor(ui, "name", &response)` where the widget is drawn;
- fact: `guide::fact(ctx, "name")` in code that runs each frame while it is true, or a
  case in `CdeptApp::guide_fact` (`app.rs`) for what follows from the project;
- add the name to `ANCHORS` or `FACTS`.

Both calls do nothing while no tutorial runs.

What cannot be pointed at: anything inside the pictures xschem, KLayout and Magic draw
in their tabs. A step checks the result instead (a circuit, the layout facts below) and
shows a picture of what is wanted.

## Circuits, pictures and starting projects

- `[[circuit]]` says what a schematic must hold: parts by symbol, with attribute values
  and the net on each pin (`?name`: a net of the user's own, the same one wherever the
  name stands; `swap` for pins that may be either way round). The step asks for
  `circuit:<id>`, and the card lists what is still wrong. See `inv-schematic.toml`.
- A step's `picture = "name"` shows `docs/tutorials/pictures/name.png`.
- `start = ["base", "inv-lib"]` with `pdk` lays the folders
  `cdept-gui/tutorials/start/<layer>/` over a new project; a step with
  `start_button = true` offers to make it.
- Also `series` and `order`, which place the tutorial in the lists. A new file needs no
  registration (`build.rs`); add it to the tables of `docs/tutorials/index.md`.

### The layout tab

| Fact | Holds while |
|---|---|
| `layout.ready` | A layout tab is in front and its KLayout runs |
| `layout.cell:<cell>` | The top cell places an instance of `<cell>` (a PCell by its name) |
| `layout.at:<cell>:<x>,<y>[:<param>=<value>,…]` | An instance of `<cell>` is at x, y (µm) with these PCell parameters |
| `layout.covers:<layer>/<datatype>:<l>,<b>,<r>,<t>` | The cell's own shapes on the layer cover the box, however they were drawn |
| `layout.pin:<name>:<layer>/<datatype>:<l>,<b>,<r>,<t>` | The pin lies inside the box, wholly on metal of that layer |
| `layout.net_ok:<net>`, `layout.net_bad:<net>`, `layout.nets_clean` | The net check, while it is of the layout as it is |
| `layout.drc_clean`, `layout.drc_violations`, `layout.drc_rule:<rule>`, `layout.lvs_match`, `layout.lvs_mismatch` | The last DRC or LVS, while the layout is the one it checked |

Anchors: `layout.mode.<mode>` (marked while it is not the mode in use), `layout.layer:<name>`
(while not the current layer) and `layout.find:<name>` (the layer filter, only for the
current step's own entries), `layout.picture`, `layout.drc`, `layout.lvs`,
`layout.drc.rule:<rule>`, `layout.drc.show_all`, `layout.properties.ok`,
`layout.picker.library:<name>`, `layout.picker.filter`, `layout.picker.cell:<name>`,
`layout.picker.place`, `layout.picker.cancel`, `layout.pins.delete:<name>`,
`layout.pins.draw`, `menu.layout`, `menu.layout.place_library`, `menu.layout.pins_panel`,
`menu.layout.update_connectivity`, `menu.cell.layout`. Places inside the layout picture
cannot be pointed at: give coordinates.

## After editing

```sh
cargo build
target/debug/cdept tutorial <id> --print > docs/tutorials/<id>.md
cargo test -p cdept-gui guide          # names, and that the page is up to date
cdept-gui/scripts/tutorial.sh -o /tmp/replays <id>   # needs a display, the PDK and the tools
```

Give every tutorial a replay script, `cdept-gui/scripts/tutorials/<id>.script`.
`guide click` clicks the middle of what the guide outlines, so the replay fails when a
mark is in the wrong place or a step does not tick off. Look at its screenshots: a label
can cover what the reader needs to see.

A tutorial file outside the source tree works too: `cdept tutorial path/to/file.toml`.

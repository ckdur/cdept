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

What cannot be pointed at yet: anything drawn by xschem, KLayout or Magic inside their
tabs. Those tools draw their own picture, so the schematic and layout tutorials will
need the helpers to report where things are.

## After editing

```sh
cargo build
target/debug/cdept tutorial <id> --print > docs/tutorials/<id>.md
cargo test -p cdept-gui guide          # names, and that the page is up to date
cdept-gui/scripts/tutorial.sh /tmp/shots   # replays pdk-setup; needs a display and the PDK
```

Give every tutorial a replay script like `cdept-gui/scripts/tutorial-pdk-setup.script`.
`guide click` clicks the middle of what the guide outlines, so the replay fails when a
mark is in the wrong place or a step does not tick off. Look at its screenshots: a label
can cover what the reader needs to see.

A tutorial file outside the source tree works too: `cdept tutorial path/to/file.toml`.

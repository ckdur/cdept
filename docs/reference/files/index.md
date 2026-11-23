# File formats

The files CDePT reads and writes. They are plain text (TOML or JSON), diff-able, and
designed for version control. CDePT edits them in place and keeps comments and layout.

| File | What it holds |
|---|---|
| [`cdept.toml`](cdept-toml.md) | The project: its name, technology and library definitions. |
| [`cdept.local.toml`](cdept-local-toml.md) | Per-user settings of a project, not committed: the PDK folder and environment. |
| [`tech.toml`](tech-toml.md) | The technology snapshot: tool set-ups, decks, model corners and PDK libraries read out of a PDK. |
| [Category files](categories.md) | Named groups of cells of a library. |
| [Project folders](project-folders.md) | The layout of a project on disk: libraries, cells, views, and the files each view kind uses. |
| [`lib.toml`](project-folders.md#lib-toml) | Optional file in a library folder: format, description, categories, copy exclusions. |
| [`setup.toml`](setup-toml.md) | A testbench: analyses, corners, sweeps, measures and specs. |
| [`suite.toml`](suite-toml.md) | A suite: several testbenches run together. |
| [Test templates](test-template.md) | The schema of a test template, built in or written by the user. |
| [CACE datasheets](cace.md) | The CACE datasheet that `cdept cace import` and `export` read and write. |
| [Results](results.md) | What a simulation run leaves on disk: decks, raw files, `result.json`, `results.db`. |

```{toctree}
:maxdepth: 1

cdept-toml
cdept-local-toml
tech-toml
categories
project-folders
setup-toml
suite-toml
cace
test-template
results
```

## See also

- [Command reference](../cli.md)
- [Environment variables](../environment.md)

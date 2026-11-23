# suite.toml

A view of a cell that contains a `suite.toml` is a **suite**: a list of testbench views
that run together. The view may have any name (`suite` by convention). Unknown keys are
errors. A suite lists at least one `[[test]]`.

```toml
description = "inverter characterisation"
corners = ["tt", "ff", "ss"]
temperatures = [27]

[[test]]
view = "tran_delay"

[[test]]
view = "tb_inv_ac/ac_response"
name = "ac"
```

## Keys

| key | type | default | meaning |
|---|---|---|---|
| `description` | string | empty | Shown in the report. |
| `corners` | array of strings | empty | When not empty, replaces the `corners` of every test. |
| `temperatures` | array of numbers (deg C) | empty | When not empty, replaces the `temperatures` of every test. |
| `test` | array of tables | required | The tests, run in this order. |

### `[[test]]`

| key | type | default | meaning |
|---|---|---|---|
| `view` | string | required | The testbench view: `view` (a view of this cell), `cell/view` (this library), or `library/cell/view`. More than three parts is an error. |
| `name` | string | `cell/view` | The label in the report table. |

## Behaviour

`cdept sim suite <library> <cell> [--view V] [-j N] [--all-columns] [--json] [--no-store]`
runs every test (netlisting on one thread per test) and puts all points on one job queue.
The report has a row per test and point, a column per measurement that some test has a
spec on (`--all-columns` shows every measurement), the failing and missing specs, and a
summary line ending in `SUITE PASS` or `SUITE FAIL`. A test that cannot run (not a
testbench, template or net error, simulator missing) is an `ERROR` row and does not stop
the others. The exit status is non-zero when a test cannot run, a run fails or errors, or a
spec fails or is missing.

Each test's runs are stored as usual ([Results](results.md)). The suite adds
`simulations/<library>/<cell>/<suite view>/suite.json` (the report) and a row in the
`suite_runs` table of `results.db`.

## See also

- [Suites how-to](../../howto/suites.md), [TESTS.md](../../TESTS.md)
- [setup.toml](setup-toml.md), [Results](results.md)

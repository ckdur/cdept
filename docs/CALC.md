# CDePT: the measurement calculator (Phase 5, first slice)

Status 2026-11-23: a SPICE raw reader, a waveform function library, an expression
language, `[[output]]` in `setup.toml` and `cdept calc`. Code: `libcdept/src/raw/`
(reader), `libcdept/src/calc/` (`wave`, `measure`, `spectral`, `funcs`, `lang`, `sim`),
CLI `cdept calc`. Pure Rust, no new dependencies (own radix-2 FFT, hand-written parser).
Not built yet: a calculator panel and plots in the GUI, jitter, PSD/Welch, CSV/VCD readers.

The idea is the one of a circuit designer's calculator (pick signals, apply functions,
get numbers); the syntax is our own and small.

## 1. Raw files (`libcdept::raw`)

```rust
let raw = RawFile::open(path)?;            // mmap + headers; ascii is parsed
raw.plots()                                // Vec<Plot>: name, kind() = tran|ac|dc|noise|op, vars, points, complex
raw.find_plot("ac")                        // by index, kind or part of the name
let w: Waveform = raw.vector(plot, "v(out)")?;   // one column copied out, x = first variable
```

ngspice and Xyce flavours, binary (little-endian doubles) and ascii, real and complex
(`Flags: complex`), several plots in one file (our runs append the analyses to
`out.raw`). A file cut short gives the points that are there
(`declared_points` says what was announced). Names are matched without regard to case
(Xyce writes `OUT`, `V1#branch`; ngspice `v(out)`, `i(v1)`). A binary plot is not read
into memory until a vector is asked for, so a 100 MB file costs only the vectors used.

`Waveform { x: Arc<Vec<f64>>, y: Samples::{Real, Complex}, x_name, x_unit, name, unit }`.
Waveforms from one plot share `x`. `value_at` interpolates linearly.

## 2. The language

One expression, evaluated against an `Env`: the raw file, optionally one plot, and
names. Result: a number, a complex number, a waveform or a string.

```
ugbw(v("out") / v("in"))
delay(v("in"), v("out"), level=0.6, trig_edge="rise", targ_edge="fall")
db20(value_at(v("out", analysis="ac"), 1k))
enob(clip(v("dout"), 1u, 9u), window="rect")
pkpk(v("out")) / vdd
```

- Numbers take SPICE suffixes and units: `1u`, `5p`, `1meg` (`M` is milli, as in SPICE),
  `2.5k`, `10pF`, `1e-9`. Strings are `"…"` or `'…'`.
- Operators `+ - * / ^` (`^` binds tighter than unary minus, right-associative).
  Between waveforms they work on a common grid (the union of the two x grids when they
  differ); a number is broadcast; complex waveforms give complex waveforms.
- Calls take positional and named arguments, positional first: `cross(w, 0.5, "rise", n=2)`.
  Each function's parameter list is in the table below; `[x]` is optional.
- Names (case-insensitive) are the variables of the run point: `corner` (string), `temp`
  and `temperature` (°C), `seed`/`run` (Monte Carlo), every entry of `[variables]` (swept
  values win), the `.meas` results and earlier outputs of the same testbench, and `pi`.
- Function names are case-insensitive. A signal is looked up with `v("out")` (finds
  `v(out)` or `out`), `i("vdd")` (finds `i(vdd)` or `vdd#branch`), `sig("onoise_spectrum")`.
  A name that exists in several plots (tran and ac both have `v(out)`) is an error until
  you say which: `v("out", analysis="tran")` or `plot="…"`, or set the plot for the
  whole expression (`[[output]] plot = "ac"`, `cdept calc --plot ac`). Single-point
  plots (operating point) do not count as a clash.
- Errors name the position: `cdept calc` and `[[output]]` print the line with a caret.

## 3. Functions

`cdept calc --functions` prints the same table (names separated by `|` are aliases).
All reductions and timing functions take `from=` and `to=` (an x range) where listed.
Real waveforms only unless said; use `mag`, `db20`, `real`, `imag`, `phase` first.

Signals: `v(name, [plot], [analysis])`, `i(…)`, `sig(…)`, `xaxis(w)`.

Elementwise (numbers, complex, waveforms): `abs|mag`, `db20`, `db10`, `phase` (degrees,
unwrapped along x), `phase_rad`, `real`, `imag`, `conj`, `sqrt`, `exp`, `ln`, `log10`,
`sin cos tan atan`, `floor ceil round`, `pow(x, y)`.

Reductions (time-weighted, trapezoid): `min(w)`, `max(w)` (also `min(a, b)` of numbers),
`minx`, `maxx` (where), `avg|mean`, `rms`, `pkpk`, `integrate(w, [from], [to])`, `len`,
`xmin`, `xmax`.

Waveform to waveform: `derivative`, `integral` (running), `clip(w, from, to)`,
`resample(w, n=|step=, [from], [to])`, `value_at(w, x)` (`at`, `interp`; complex for a
complex waveform).

Timing: `cross(w, [level=0], [edge="either"], [n=1], …)` (n=-1 is the last),
`delay(trig, targ, [level], [trig_edge="rise"], [targ_edge="rise"], [trig_level],
[targ_level], [trig_n], [targ_n])` (the target crossing is the first at or after the
trigger; `level` is both thresholds, like `.meas` `val=`), `rise_time`, `fall_time`
(`lo=0.1 hi=0.9`, swing `vlow..vhigh`, default the waveform's min and max), `slew`
(V/s of the 10-90 % part), `overshoot`, `undershoot` (percent), `settling_time(w, tol=0.02)`,
`frequency`, `period`, `duty` (percent).

Frequency response (the complex AC result, or its magnitude): `bandwidth(w, drop=3)`,
`unity_gain_freq` (`ugbw`), `phase_margin`, `gain_margin`. These interpolate on log x.
Margins take the complex loop gain with the sign of the feedback included, so its phase
starts near 0. `phase_margin` is 180 plus the unwrapped phase at the unity-gain
frequency, `gain_margin` is minus the gain in dB where the phase reaches -180.

Spectra: `fft(w, window="hann", n, from, to)` (a complex waveform over frequency,
magnitude = peak amplitude, window gain corrected), and, on the time waveform directly,
`sndr`, `snr`, `thd` (dB, negative), `thd_percent`, `sfdr`, `enob`, `fundamental`, all
with `fund` (Hz, default the largest bin), `harmonics=5` (2nd to 5th, folded back below
fs/2), `window` (`rect`, `hann`, `blackman`), `n`, `from`, `to`. A uniform waveform with a
power-of-two number of points is used as it is; anything else (SPICE's adaptive steps) is
resampled to `n` points over `from..to` by linear interpolation, giving a whole number of
signal cycles in that range keeps the figures coherent. Signal power is the tone bin and
the window's main lobe; harmonic bins likewise; the rest is noise.

Noise: `integrate_noise(w, [from], [to], [squared])` (density V/√Hz to V rms, trapezoid),
`spot_noise(w, f)`.

Converters: `dnl`, `inl` (waveforms over the code, LSB), `dnl_max`, `inl_max`. `w` is
the output code. `method="edge"` (default): the code against a swept input, transitions
found at code-0.5 crossings, end-point fit unless `lsb=` is given. `method="hist"`: the
code sampled over a uniform ramp, widths from the counts (end codes left out).

## 4. In a simulation

```toml
[[output]]
name = "gain_db"
expr = 'db20(value_at(v("out", analysis="ac") / v("in", analysis="ac"), 1k))'
[[output]]
name = "pm"
expr = 'phase_margin(v("out", analysis="ac") / v("in", analysis="ac"))'
[[spec]]
name = "pm"
min = 60
```

After each run `cdept sim run` opens `out.raw` and evaluates the outputs in order. A
number joins the run's measures (an output wins over a `.meas` of the same name), is
checked against specs like any measure and is stored in `result.json` and
`results.db`. `result.json` lists the calculator's names in `calc` and failed ones with
the message in `calc_errors`; the table of `cdept sim run` marks the column with `*` and
prints the errors under the row. An output that fails (or is not a finite number) is
missing, so a spec on it fails. `plot = "ac"` limits the lookups of one output.
Expressions are parsed when the setup loads, so a typo is reported before any
simulation. Xyce raws work the same way.

## 5. Ad hoc

```
cdept calc 'delay(v("in"), v("out"), 0.6, trig_edge="rise", targ_edge="fall")' \
    --raw simulations/cells/tb_inv/testbench/tt_27/out.raw --var vdd=1.2
cdept calc --raw out.raw --list            # plots and vectors
cdept calc 'v("out", analysis="tran")' --raw out.raw --dump    # x y per line
```

## 6. Tests

Unit tests use synthetic waveforms: sine with a 1 % third harmonic (THD -40 dB, SFDR 40
dB, all three windows), quantised sines of 6/8/10 bits (ENOB within 0.3), a noisy sine
(SNR), an irregular time grid, an RC step (10-90 % rise `tau ln 9`, settling, slew), a
second-order step (overshoot 16.3 %), a two-pole amplifier (UGBW, phase margin,
bandwidth), a synthetic ADC (DNL/INL by edges and histogram), ngspice-style binary and
ascii raws with several plots, truncated files. Real: `libcdept/tests/real_sim.rs`
(`calculator_outputs_match_the_meas`): the calculator's `delay` on `tb_inv`'s `out.raw`
equals the `.meas` `tpd`; and `raw::tests::real_xyce_raw_reads_like_ngspice` reads a
Xyce raw (skipped without Xyce; `CDEPT_XYCE`).

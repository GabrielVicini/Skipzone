# Command Line Tools

Besides the desktop app, Skipzone ships a terminal predictor, `hfpredict`,
and twelve harnesses that check the model against measurements. You need none
of them to use the app. Every command below runs from the repository root, and
each tool builds on first use.

Only two tools touch the network: `wspr_corpus` and `wspr_live_check`. Four
read a `corpus/` directory of fetched data that is not in the repository. The
table below marks both.

## `hfpredict`

`hfpredict` answers "does a reflected path exist between these two points at
this frequency and time?" It lives in `crates/skipzone-predict/src/bin/`.

```bash
cargo run --release -p skipzone-predict --bin hfpredict -- \
    --tx 51.5,-0.1 --rx 40.7,-74.0 --freq 14.1 --utc 18 --month 3 --solar high
```

That prints the great-circle distance and the ionosphere it assumed. If a path
exists, it prints `RESULT: LIKELY CONNECTS` with the hop count and launch
elevation. Otherwise it prints `RESULT: UNLIKELY TO CONNECT`. `--tx`, `--rx`,
`--freq` and `--utc` are required. The other flags are:

```text
--month N           month 1..12, sets the season (default 6)
--solar low|high    solar-cycle level (default low)
--fof2 MHZ          override the looked-up F2 critical frequency
--hmf2 KM           override the looked-up F2 peak height
--scale-height KM   Chapman scale height (default 50)
--max-hops N        equal-hop path attempts, 1..10 (default 3)
```

Its model is much simpler than the app's. It uses one Chapman F2 layer with
foF2 and hmF2 from a coarse midlatitude table, no magnetic field and no
absorption. A "connects" result is geometric reachability, and it says nothing
about signal strength. For a result you can defend, pass measured values with
`--fof2` and `--hmf2`. The full list of limits is in its `--help`:

```bash
cargo run --release -p skipzone-predict --bin hfpredict -- --help
```

## The lab harnesses

These live in `crates/skipzone-lab/src/bin/` and run as
`cargo run --release -p skipzone-lab --bin <name>`. They drive the same
prediction code as the GUI.

| Tool | What it is for | Needs |
|---|---|---|
| `wspr_corpus` | Fetch a reproducible WSPR corpus once and write it to disk | Network |
| `wspr_calibrate` | Fit the unverified anchors against a saved corpus, with station effects as nuisance parameters | `corpus/` |
| `wspr_validate` | Score the model against a saved list of WSPR spots | A spot file |
| `wspr_live_check` | Fetch live WSPR spots and the sunspot number, score the model and list the worst misses | Network, unless given `--file` and `--ssn` |
| `iono_check` | Score the modelled foF2 and foE against measured ionosonde values, in MHz | `corpus/` |
| `benchmark` | Score the model, and the ITU-R P.533 baseline, on frozen corpora with month and region hold-outs | Frozen corpus files with a manifest; no network |
| `mode_audit` | Show why the solver admits a path and what absorption it charges it | Nothing |
| `zenith_probe` | Measure the solar-zenith law the D region produces, without tracing a ray | Nothing |
| `rise_wiring_check` | Check that the D-region rise anchor reaches a solve, and how much leverage the corpus has on it | `corpus/` |
| `solve_digest` | Print a stable fingerprint of everything `solve()` produces over a fixed scenario grid | Nothing |
| `profile_solve` | Count where the time in one solve goes, by trace, step and model evaluation | Nothing |
| `gen_fof2_grid` | Regenerate the bundled `fof2_grid.tsv` from its generator | Nothing |

The engine also has one example, `nvis_probe`, which prints where
high-elevation rays land in a simple layer. It is a diagnostic with no
assertions:

```bash
cargo run --release -p skipzone --example nvis_probe
```

### Fetching and calibrating

```bash
# Fetch a week of spots (positives) and the receivers that heard nothing (negatives)
cargo run --release -p skipzone-lab --bin wspr_corpus -- \
    --from 2026-07-02 --days 7 --out corpus/fit.tsv --neg corpus/fit_neg.tsv

# Fit against it, testing on a hold-out from a different week
cargo run --release -p skipzone-lab --bin wspr_calibrate -- \
    --fit corpus/fit.tsv --holdout corpus/holdout.tsv --negatives corpus/fit_neg.tsv
```

Build the hold-out with a second `wspr_corpus` run over a different span.
`wspr_calibrate` stops with an error if an input file is missing. Run either
tool with `--help` for the full flag list, including `--antenna`, `--noise-env`
and `--scan` on `wspr_calibrate`.

### Scoring

```bash
# A saved spot list: timestamp, TX call, MHz, SNR, TX grid, dBm, RX call, RX grid, km
cargo run --release -p skipzone-lab --bin wspr_validate -- spots.tsv --ssn 70

# A 20-minute live window, at most 300 spots
cargo run --release -p skipzone-lab --bin wspr_live_check -- --minutes 20 --limit 300

# A fixed past window on one band
cargo run --release -p skipzone-lab --bin wspr_live_check -- --band 14 --at "2026-07-24 03:22"

# Offline: a saved file and a given sunspot number
cargo run --release -p skipzone-lab --bin wspr_live_check -- --file spots.tsv --ssn 119

# Ionosonde comparison, with explicit paths (these are the defaults)
cargo run --release -p skipzone-lab --bin iono_check -- \
    corpus/ionosonde.tsv --ssn=corpus/ssn_daily.tsv --fit=corpus/fit.tsv
```

`iono_check` takes its flags as `--name=VALUE`. Add `--propose` to print
suggested anchor changes as well as the error. It fits nothing itself.

### Benchmarking

```bash
# Score every corpus named by a manifest; writes results.tsv, rows.tsv and report.md
cargo run --release -p skipzone-lab --bin benchmark -- run \
    --manifest corpus/benchmark/wspr-v1/MANIFEST.tsv --out corpus/benchmark/runs/first

# Compare two runs, paired on the rows both scored
cargo run --release -p skipzone-lab --bin benchmark -- compare \
    corpus/benchmark/runs/before/results.tsv corpus/benchmark/runs/after/results.tsv

# Write ITURHFProp input files, to run P.533 on another machine
cargo run --release -p skipzone-lab --bin benchmark -- p533-inputs \
    --manifest corpus/benchmark/wspr-v1/MANIFEST.tsv --out corpus/benchmark/p533
```

`benchmark` needs no network. It reads only files listed in a corpus
`MANIFEST.tsv`, checks each file's SHA-256 first, and stops if one differs. It
reads WSPR, Reverse Beacon Network, NCDXF beacon and GIRO ionosonde files.
Pass the ITURHFProp outputs back with `--p533 corpus/benchmark/p533`.
[The accuracy roadmap](https://github.com/GabrielVicini/Skipzone/blob/master/docs/notes/accuracy-roadmap.md)
describes the corpora, the splits and the metrics. No tool fetches the
benchmark corpora yet. `--help` lists every flag. The synthetic test fixtures
are in `crates/skipzone-lab/tests/fixtures/benchmark/`.

### Diagnostics

```bash
cargo run --release -p skipzone-lab --bin mode_audit
cargo run --release -p skipzone-lab --bin zenith_probe
cargo run --release -p skipzone-lab --bin profile_solve

# Corpus file and subsample size (defaults: corpus/fit.tsv and 600)
cargo run --release -p skipzone-lab --bin rise_wiring_check -- corpus/fit.tsv 1200
```

`solve_digest` is the regression lock for any refactor or optimisation.
[Contributing](Contributing#the-regression-lock) explains how to use it.

`gen_fof2_grid` regenerates the bundled foF2 grid.
[Contributing](Contributing#data-and-assets) says when and how to run it.

## The `corpus/` directory

`corpus/` is gitignored working data. Create it at the repository root.

| File | Where it comes from | Read by |
|---|---|---|
| `fit.tsv`, `fit_neg.tsv` | `wspr_corpus` | `wspr_calibrate`, `rise_wiring_check`, `iono_check` (median SSN only) |
| `holdout.tsv` | `wspr_corpus`, over a different span | `wspr_calibrate` |
| `ionosonde.tsv` | Fetched by hand from the Lowell GIRO DIDBase | `iono_check` |
| `ssn_daily.tsv` | SILSO daily sunspot number, as tab-separated year, month, day, SSN | `iono_check` |
| `benchmark/<name>-v<N>/` | A frozen corpus and its `MANIFEST.tsv`, fetched once and never refetched under the same version | `benchmark` |
| `benchmark/runs/` | One directory per `benchmark run` | `benchmark compare` |

No tool in the repository fetches the ionosonde or SILSO files. If
`iono_check` cannot read `ssn_daily.tsv` or `fit.tsv`, it carries on with a
fallback sunspot number and warns on stderr.

GIRO data is CC-BY-NC-SA 4.0, and each contributing station's operator must be
acknowledged in any published use. The other services, `db1.wspr.live`, SILSO
and NOAA SWPC, are volunteer- or publicly-run, so keep request rates modest.
All network calls go through one function in `crates/skipzone-lab/src/net.rs`.

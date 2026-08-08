# Validation Harnesses

Three independent things check this model, and they check different parts of it.
Nothing here runs in the GUI, and none of it is on the automated test path,
because all of it needs data that is fetched rather than committed.

| Harness | What it can see | What it cannot |
|---|---|---|
| Test suite (`tests/`) | Engine correctness against closed-form solutions and invariants | Anything about the real ionosphere |
| Ionosonde (`iono_check`) | Absolute foF2 and foE error in MHz | Absorption, noise, antennas, the link budget |
| WSPR (`wspr_*`) | How the received signal **varies** with frequency, path, zenith angle, hop count, layer | Any absolute level |

They are complementary on purpose. The test suite pins the physics engine
against maths. The ionosonde pins the ionosphere against measurement. WSPR pins
the end-to-end link against reality, but only in the dimensions it can identify.

## 1. The test suite

Runs on every `cargo test` and in CI. 241 tests, no network, no fetched data.
See [Engine Crate](Engine-Crate.md#testing) and
[Building and CI](Building-and-CI.md).

The engine's suite works by having analytic answers to compare against:
quasi-parabolic closed forms, the Bouguer invariant along a ray, apex
conditions, straight rays in vacuum, fifth-order convergence, absorption
reciprocity, and agreement between two independent homing methods.

The pattern is consistent: **analytic derivatives are the implementation and
finite differences are the test oracle**, never the reverse.

## 2. Ionosonde validation

`iono_check`. This is the only comparison in the project against a direct
measurement of the ionosphere itself.

An ionosonde measures foF2 and foE directly. It has none of WSPR's
identification problems: no unknown antenna, no unknown noise floor, no fading,
no station effects, no decode threshold. The comparison is therefore a straight
measurement of model error in MHz, with nothing absorbed and nothing fitted.

The sign convention is `MODEL - MEASURED` in MHz. **Positive means the model
reads high.**

### Data

`corpus/ionosonde.tsv`, fetched from the Lowell GIRO Data Center's DIDBase
(FastChar GetBest). GIRO releases under CC-BY-NC-SA 4.0 and asks that each
contributing station's operator be acknowledged. The file header carries the
licence and the rules-of-the-road link, and any published use of these numbers
must carry them too.

Daily sunspot numbers come from SILSO (`corpus/ssn_daily.tsv`). The corpus's own
SSN is used, so the comparison drives the model with the number it would have
been driven with on those days rather than a guess. Where SILSO has not
published a daily value yet (its series lags real time by about a month) the
corpus median is used, and the run says so.

### Rules for running it

**All four seasons are mandatory.** The seasonal windows sit months apart on the
solar cycle. Driving them all at one SSN would charge the model for that cycle
and call the difference model error, which is the same confound as fitting a
diurnal shape in one season.

**It does not fit anything.** It reports error. `--propose` will print suggested
anchor changes, but applying one belongs in a separate deliberate change with
its own test.

### What it found

- foE was measurably wrong, and was fixed as a result.
- foF2's shape error has a floor around 1.70 MHz that the current functional
  form cannot get below. That is a limit of the shape, not of the anchors.

## 3. WSPR validation

Three tools at increasing levels of commitment.

### `wspr_live_check`

Fetch live spots and the observed sunspot number, score, report. Good for a
quick "is anything obviously broken" pass. Not reproducible, because the spots
change every two minutes.

### `wspr_validate`

Score against a saved spot file. Reproducible, no network.

### `wspr_calibrate`

The full fixed-effects calibration. See [Calibration](Calibration.md).

### Why WSPR is a good datum, and where it stops

A WSPR spot is an unusually good validation datum: a measured SNR, at a known
time, between two known grid squares, at a known transmit power, with a known
decode threshold and a known reference bandwidth.

What it is not is a measurement of propagation alone. The transmitting antenna,
the accuracy of the claimed power, the receiving antenna and the receiver's
local noise floor are all in the number and none of them is in the archive. That
is why the calibration estimates station effects explicitly, and why absolute
levels are unidentifiable from this data no matter how much of it there is.

`wspr_report.rs` breaks a run down into the places the model is weakest, because
"how far off is the model overall" is the least useful summary available.
Reported cuts carry their sample size and thin ones are flagged; a cut that
found nothing reports "no error", not a zero error.

## The `corpus/` directory

Not in the repository. It is gitignored working data.

| File | Written by | Contents |
|---|---|---|
| `fit.tsv` | `wspr_corpus` | Positives, the spots to fit on |
| `fit_neg.tsv` | `wspr_corpus` | Negatives, receivers that heard nothing |
| `holdout_*.tsv` | `wspr_corpus` | Day- and region-separated hold-out sets |
| `ionosonde.tsv` | fetched from GIRO | Measured foF2 and foE |
| `ssn_daily.tsv` | fetched from SILSO | Daily sunspot number |
| `*.log`, `digest_*.txt` | run output | Working artefacts |

To build one from scratch:

```bash
cargo run --release -p skipzone-app --bin wspr_corpus -- \
    --from 2026-07-02 --days 7 --out corpus/fit.tsv --neg corpus/fit_neg.tsv
```

Then a hold-out over a different, non-adjacent span, and the calibration itself.
See [Command Line Tools](Command-Line-Tools.md).

## Network access

Exactly one module reaches the network: `app/src/net.rs`, one function, 60
second timeout, identified User-Agent. The GUI and the solver never call it. The
services are:

| Service | Used for |
|---|---|
| `db1.wspr.live` | WSPR spots, via a public ClickHouse endpoint |
| `www.sidc.be` (SILSO) | Estimated international sunspot number |
| `services.swpc.noaa.gov` | Observed solar cycle indices |

All three are volunteer- or publicly-run and ask that automated users be
identifiable, which is what the User-Agent is for. Be considerate with request
rates.

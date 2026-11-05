# Accuracy roadmap

**Written:** 2026-11-05, against `master` at `cead477`.
**Purpose:** a menu of accuracy improvements, with the evidence for each, so the
owner can choose the order. It recommends a benchmark first and gives the
candidates in roughly the order of expected gain per unit of effort. It does not
fix the order.

The current numbers come from the
[2026-08 verification report](../reports/2026-08-verification.md), run at
commit `d92bf9f`. Nothing in this note re-runs them.

## 1 Where things stand

| Component | Today | Weakness |
|---|---|---|
| Ray tracer | Verified against closed-form solutions, 21 analytic integration tests, convergence order measured at 5 | None known |
| foE | Bias -0.00 MHz, RMS 0.20 MHz over 28 952 GIRO soundings | None known |
| foF2 | Bias +0.03 MHz, RMS 1.10 MHz, r 0.894 over 70 219 GIRO soundings | Model SD 2.16 MHz against measured 2.45 MHz. A monthly climatology cannot follow day-to-day weather, and the daily SSN driver reads 0.39 MHz low against R12 |
| hmF2 | A fixed input, 300 km by default | M(3000)F2 is evaluated by `ccir.rs` and consumed by nothing |
| D-region absorption | Zenith exponent n = 1.39 | Published riometer work gives about 0.75. The WSPR corpus cannot tell the two apart (median spot moves 0.02 dB) |
| Collision frequency | Three textbook anchors | Not derived from a neutral atmosphere |
| Antenna model | Image theory over flat lossy ground | No test and no reference data |
| End-to-end SNR | About 8 dB RMS with station effects removed, slope 0.54 to 0.61, AUC 0.801, false positives up to 34.7 % (80.4 % at night) | Stale: run before the CCIR maps became the default. One season, one geographic wedge, SSN 83 to 126 |

"Fully accurate" is not a reachable goal for HF prediction. The ionosphere
varies from day to day and hour to hour in ways no climatology or nowcast
captures, and one path measured twice within ten minutes already scatters by
3.5 dB (report section 5.3). The goal here is narrower: measurably better than
today, on data the model never saw, with error bars that say how sure the
improvement is.

<!-- OWNER: is that the goal you want stated publicly, and is there a target figure (for example "single-path RMS under 6 dB on the region hold-out") you want to commit to? -->

## 2 Step 0: a fair benchmark, before any physics changes

Every method in section 3 is judged by one harness, built first and frozen. The
reason is the stale-baseline finding: the project does not know its current
end-to-end accuracy, so no improvement can be measured until it does.

Step 0 has two parts.

1. Re-run the existing calibration on the current default (CCIR R12 v1) with
   `wspr_calibrate`, as the report's commentary recommends. This takes about an
   hour and produces the numbers the benchmark is checked against.
2. Build `skipzone-lab --bin benchmark`, described below.

**Status (2026-11-05):** the `benchmark` binary exists, at
`crates/skipzone-lab/src/bin/benchmark.rs`, with its logic in
`crates/skipzone-lab/src/benchmark/` so that it can be tested. It has been run
only on the synthetic fixtures in `crates/skipzone-lab/tests/fixtures/benchmark/`
and has produced no numbers about the model. What remains: fetching and
freezing the real corpora (no fetcher exists yet), the first real run, and the
ITURHFProp run on the input files `benchmark p533-inputs` writes, on a machine
that has the corpora and a built ITURHFProp.

### 2.1 What the benchmark is

A new binary at `crates/skipzone-lab/src/bin/benchmark/`, laid out like
`wspr_calibrate/` (one module per concern). It reads frozen corpora from disk,
runs every model under test on the same rows, and writes one report. It fits
nothing about the physics. Station effects are the one exception, explained in
section 2.4.

It reuses existing code wherever it can:

| Need | Existing code |
|---|---|
| Solving one spot | `solving::solve_one` in `bin/wspr_calibrate/solving.rs`, and `scenario::resolve`, `scenario::build_models`, `solve::solve`, `solve::best_with_es_fallback` |
| Negatives (paths that were listening and heard nothing) | `corpus::negatives_from_cycle` in `crates/skipzone-lab/src/corpus.rs`, and `negatives::solve_negatives` in `bin/wspr_calibrate/negatives.rs` |
| Station effects | `fit::StationEffects::solve` in `crates/skipzone-lab/src/fit.rs` |
| Decode probability | `noise::decode_probability` in `crates/skipzone-predict/src/noise.rs` |
| Cuts by band, distance, illumination | `wspr_report::breakdowns` |
| Parallel solves | `compute::ComputePool` |
| Network access | `net::get_text`, the crate's only network function |

`skipzone-lab` is the only crate allowed network access
(`scripts/check-boundaries.py`). All fetchers below live there. The iono and
predict crates stay free of I/O.

### 2.2 Frozen corpora

Each corpus is fetched once by a separate command, written to
`corpus/benchmark/<name>-v<N>/`, and never refetched under the same version.
The directory holds the data files plus a `MANIFEST.tsv` with, per file: the
SHA-256, the row count, the query or URL that produced it, the fetch time, the
source's licence line and the attribution it requires. The benchmark refuses to
run if a hash does not match.

`corpus/` is gitignored and stays that way. The data licences (section 2.2.4)
do not allow most of it to sit in an MIT repository. The manifests are small,
contain no measurements, and can be committed under `docs/` or `scripts/` so a
reported number names exactly the data behind it.

<!-- OWNER: commit the manifests (and where), or keep them beside the data only? -->

#### 2.2.1 WSPR (existing)

- **Source:** the WSPRnet archive through the wspr.live ClickHouse endpoint
  (`https://db1.wspr.live/`), already used by `wsprlive.rs` and
  `bin/wspr_corpus.rs`.
- **Format:** the nine-column TSV that `wspr::parse_spots` reads, plus the
  per-day SSN column, written by `corpus::write_positives` and
  `corpus::write_negatives`.
- **What changes:** the corpus must cover more than one season and more than
  the European and North American wedge (report section 7.4). Twelve months,
  one week per month, with the same band and hour schedule `wspr_corpus` uses
  today, would let the month hold-out exist at all. The busiest-station filter
  (`BusiestFilter`) should be extended to ask for receivers in South America,
  Africa, Asia and Oceania by name, as the band schedule already does for quiet
  bands.
- **Volume:** the current corpus is 9 126 spots and about 76 000 negatives from
  fourteen cycles. A twelve-month version at the same density is about 100 000
  positives. At a few hundred milliseconds per solve that is several CPU-hours
  per model per run, so the benchmark needs the same stride thinning as
  `--max-spots`, with the stride recorded in the manifest.
- **Terms:** wspr.live's front page allows use "for your own research and
  projects if results are accessable free of charge for everyone" and forbids
  commercial use. It describes the data as raw WSPRnet reports, with duplicates
  and false spots. A third-party report says WSPRnet stopped publishing monthly
  archive files after `wsprspots-2026-05`; this was not confirmed from an
  official source. The endpoint itself could not be reached from the session
  that wrote this note (see section 6).

#### 2.2.2 Reverse Beacon Network (CW)

- **Source:** daily zip files at `https://www.reversebeacon.net/raw_data/`, one
  per UTC day (for example `20260201.zip`), covering the network's whole history.
- **Format:** CSV. A third-party check of a 2026-10-02 file gives the header
  `callsign,de_pfx,de_cont,freq,band,dx,dx_pfx,dx_cont,mode,db,date,speed,tx_mode`.
  `callsign` is the skimmer, `dx` the station heard, `db` the SNR the skimmer
  measured, `mode` the spot type (CQ, DX, beacon) and `tx_mode` the modulation.
  Older files lack `speed` and `tx_mode`. The parser must accept both layouts
  and must be checked against a real file before it is trusted.
- **What it adds:** many more receivers outside Europe and North America, a
  continent column on both ends (`de_cont`, `dx_cont`) that makes the region
  split cheap, and heavy low-band daytime activity during contests, which is
  where the zenith law has leverage (report section 7.2: the WSPR corpus has 50
  daytime 160 m spots).
- **What it lacks:** transmitter power and antennas are unknown, and the SNR is
  in the skimmer's own bandwidth, which is not WSPR's 2500 Hz reference. Both
  are constant per station or per skimmer, so they go into station effects. RBN
  aggregation also de-duplicates repeated spots of one call, so a missing spot
  in a short window is weaker evidence of a failed path than in WSPR. The
  window over which the negatives are built must be at least the
  de-duplication interval, which has to be measured from the data.
- **Negatives:** the same construction as `corpus::negatives_from_cycle`. A
  skimmer that spotted at least `MIN_RX_SPOTS_FOR_HEALTHY` other stations on a
  band in the window was listening; a station spotted by somebody else on that
  band in the window was transmitting; the pair with no spot is a negative.
- **Volume:** not measured here. The first fetch should record the daily row
  count in the manifest before the corpus design is fixed.
- **Fetching:** the files are zip archives, and `net::get_text` returns text.
  Either the lab crate gains a zip dependency (allowed by the boundary rules) or
  the files are fetched by hand, as the GIRO file is today.
- **Terms:** the download page says "Data from the RBN are freely available for
  study and analysis." No other terms were found. Redistribution terms are
  unknown, so the corpus is cached locally and not republished.

<!-- OWNER: is RBN in scope from the start, or after the WSPR rebuild? -->

#### 2.2.3 NCDXF/IARU International Beacon Project

- **Source:** the published schedule at
  `https://www.ncdxf.org/beacon/beaconschedule.html`. Eighteen beacons
  transmit in turn on 14.100, 18.110, 21.150, 24.930 and 28.200 MHz, ten seconds
  per beacon per band, so five transmit at once and the cycle repeats every
  three minutes. Each transmission sends the callsign at 22 wpm and a first
  dash at 100 W, then three one-second dashes at 10 W, 1 W and 0.1 W.
- **Why it matters:** this is the only source here where the transmission is
  certain and the power is known. A healthy receiver that did not report a
  beacon in its slot is a clean negative, and the four power steps give a
  decode threshold measurement on the same path within one second. It also
  gives a real base rate for the Brier score, which the WSPR corpus cannot (the
  calibrator's corpus is about 85 % positive by construction, see the comment on
  `PREDICTIVE_SPREAD_DB` in `noise.rs`).
- **Where the receptions come from:** RBN skimmers report beacon spots, and the
  RBN web interface offers an NCDXF spot-type filter. Whether and how those
  spots are tagged in the raw CSV was not confirmed and must be checked on a
  real file. Spots carry no power step, so only the 100 W callsign is usable
  unless a dedicated beacon monitor records the steps.
- **Status:** the schedule page lists most beacons as operating and at least
  one as off. Beacon status changes, so the corpus must record the status page
  at fetch time and drop any beacon marked off for that period.
- **Negatives:** the slot table is computed from the schedule (beacon index,
  band, UTC second), joined against skimmer health from the same RBN day file.
  Only bands 20 m to 10 m are covered, so this corpus says nothing about the
  low bands.
- **Terms:** the schedule is public information. The receptions inherit the RBN
  terms above.

#### 2.2.4 Ionosondes (for MUF and for methods 1 to 4)

- **Source:** the Lowell GIRO Data Center's DIDBase, already used by
  `iono_check` through `corpus/ionosonde.tsv`. The benchmark adds M(3000)F2 and
  hmF2 to the characteristics fetched, so MUF(3000)F2 = foF2 x M(3000)F2 can be
  scored.
- **Terms:** the DIDBase Rules of the Road
  (`https://giro.uml.edu/didbase/RulesOfTheRoad.html`) release the data under
  CC BY-NC-SA 4.0. They read the share-alike term as allowing the data to be
  shared only with users who hold an LGDC account, and they ask that the GIRO
  publication be cited and the persistent identifier
  `http://spase.info/SMWG/Observatory/GIRO` be acknowledged, with each
  station's operator credited. This means ionosonde data can be cached locally
  but cannot be committed to the repository or published as a frozen corpus.
  Derived products are not restricted.

### 2.3 Metrics

All metrics are computed per model, per corpus, per split, and in the cuts
`wspr_report` already defines (band, distance bucket, day/night at the
midpoint), with counts shown.

| Metric | Definition | Corpora |
|---|---|---|
| SNR bias | median and mean of (model - measured), dB, station effects removed | WSPR, RBN |
| SNR RMS and IQR | of the same residual | WSPR, RBN |
| Slope | regression of model on measured, station effects removed (measures compression) | WSPR, RBN |
| Brier score | mean of (P(decode) - y)^2 over positives and negatives, with y = 1 for a decode | all three |
| Brier skill score | 1 - Brier / Brier of the corpus base rate | all three |
| Reliability diagram | ten equal-width bins of predicted P(decode); per bin the observed decode fraction, the count, and a Wilson 95 % interval | all three |
| Brier decomposition | reliability, resolution and uncertainty terms (Murphy), so a change can be read as better calibration or better discrimination | all three |
| AUC | decode against non-decode, kept for continuity with the report | all three |
| False-positive rate | upper bound, as `fit::NegativeScore` reports it, split day and night | all three |
| MUF error | model MUF(3000)F2 at the station minus GIRO foF2 x M(3000)F2, bias and RMS in MHz | ionosondes |
| foF2 and hmF2 error | as `iono_check` reports them today | ionosondes |

**Error bars.** Spots on one day share one ionosphere, so they are not
independent. Every interval is a block bootstrap that resamples whole UTC days
within each split (1000 resamples, fixed seed recorded in the report). A
before/after comparison is a paired bootstrap of the difference on the same
resampled days, so the interval is on the improvement itself.

**MUF from Skipzone.** The engine has no direct MUF output. It is found by
bisection on frequency with `solve::solve` over a 3000 km path centred on the
ionosonde, or with the sweep in `crates/skipzone-predict/src/sweep.rs`. The
cost is a few dozen solves per sounding, so the MUF metric runs on a thinned
subset (one sounding per station per hour).

### 2.4 Splits

Never random. Adjacent spots share an ionosphere, so a random split scores
interpolation between near-duplicates.

- **Month hold-out:** leave-one-month-out over the twelve months of the rebuilt
  corpus. Anything fitted is fitted on eleven months and scored on the twelfth.
- **Region hold-out:** leave-one-region-out, with the region taken from the
  path midpoint, because that is where the ionosphere being tested sits.
  Proposed regions: Europe, North America, South America, Africa, Asia,
  Oceania, plus a separate "trans-equatorial" group for paths whose ends are in
  opposite hemispheres. RBN's continent columns are a cross-check.
- **Headline number:** the doubly held-out cell, a month and a region both
  absent from anything fitted. Single hold-outs are reported beside it.
- **Station effects:** a station in a held-out region has no fitted effect. The
  report gives two SNR figures and labels them:
  1. effects re-estimated on the held-out rows themselves, for every model
     alike. This fits only nuisance terms, and measures how well the model gets
     the shape (frequency, hour, distance) right;
  2. no effects at all (every station at the population value of zero), which
     measures the absolute level an unknown user would see.

  The Brier score and reliability diagram use figure 2, because a user's
  station has no fitted effect.
- **Ionosondes:** leave-station-out for anything fitted to ionosonde data
  (methods 1 to 3), so a correction is never scored at the station that
  produced it.

<!-- OWNER: are these the regions you want, and should the midpoint or the receiver define them? -->

### 2.5 Baselines

Each baseline runs on exactly the rows Skipzone runs on.

**ITU-R P.533 through ITURHFProp.** The ITU-R Study Group 3 reference
implementation, `https://github.com/ITU-R-Study-Group-3/ITU-R-HF`. Checked by
cloning it on 2026-11-05: head commit `82017594` (2025-05-15), library version
string `P533VER "14.2"`, README describing Recommendations P.533-14 and P.372-15.

- **Licence:** the README says the software "may be used by implementers in
  their implementation of the Recommendation as well as in revisions of the
  specific original Recommendation and in other ITU Recommendations, free from
  any copyright assertions", is provided as is with no warranty, and excludes
  ITU liability. There is no LICENSE file. Running it as an external baseline
  fits that grant. Vendoring its source into an MIT repository is less clearly
  covered, so the benchmark should fetch and build it at a pinned commit.
- **Build:** `Linux/Makefile` builds `libp533.so`, `libp372.so` and the
  `ITURHFProp` executable with `gcc -std=c99` (clang on macOS) and needs only
  `libm` and `libdl`; it links with `-z muldefs` on Linux. Data files (`ionos01.bin`
  to `ionos12.bin`, `COEFF01W.txt` to `COEFF12W.txt`, the P.1239 decile factors)
  come from `P372/Data` and `P533/Data` and are named by `DataFilePath` in each
  input file. The GitHub releases also carry prebuilt binaries. The build was
  not run for this note; a third-party pull request reports a warning-free Linux
  build of P533 and ITURHFProp. A Windows build exists in `Win32/`.
- **Inputs:** one plain-text input file per (path, month, hour) group, with
  `Path.L_tx.lat/lng`, `Path.L_rx`, `Path.year`, `Path.month`, `Path.hour`
  (integers 1 to 24, a list is allowed), `Path.SSN` (R12 as an integer, 1 to 311),
  `Path.frequency` (a list is allowed), `Path.txpower` in dB(kW), `Path.BW`,
  `Path.SNRr`, `Path.SNRXXp`, `Path.ManMadeNoise` (`RURAL` to match Skipzone's
  calibration default) and `TXAntFilePath`/`RXAntFilePath` set to `"ISOTROPIC"`
  to match the calibration's isotropic reference. For WSPR, `Path.BW` is 2500 Hz
  and `Path.SNRr` is -29 dB, the same values `WsprSpot::inputs_for` pins.
- **Outputs:** CSV selected by `RptFileFormat`. The benchmark needs
  `RPT_SNR`, `RPT_SNRD` (upper and lower decile deviations), `RPT_BMUF`,
  `RPT_BCR` and `RPT_DOMMODE`.
- **Turning P.533 into P(decode):** P.533 predicts a monthly median with decile
  deviations. P(SNR > threshold) follows from a split normal with the upper and
  lower deciles at +/-1.2816 sigma (the normal-distribution decile points).
  Whether this matches P.533's own reliability calculation exactly should be
  checked against `RPT_BCR` on a few rows. Feeding P.533 its intended driver (R12) and Skipzone
  its own is the fair comparison; a second P.533 run with the effective index of
  method 1 shows how much of any gain is the driver.
- **Cost:** not measured. Grouping rows by (path, month, hour) and passing all
  frequencies in one file keeps the call count down.

**VOACAP.** The Linux port `https://github.com/jawatson/voacapl`, cloned on
2026-11-05 at `c12a98b3` (2026-03-29). Its LICENSE carries the NTIA/ITS notice
that the software "is not subject to copyright protection in the U.S.", puts
J. A. Watson's modifications under CC0, and puts two utilities (`dst2csv.f90`,
`dst2ascii.f90`) under GPL-3. It builds with gfortran and autotools. It is
optional: P.533 is the reference the ITU maintains, and a published comparison
of NVIS paths found P.533 with the lowest mean error standard deviation of the
three programs it tested, against 10.66 dB for VOACAP. VOACAP is worth adding
only if the owner wants the comparison most amateur users will recognise.

<!-- OWNER: P.533 alone, or P.533 and VOACAP? -->

**The headline rule.** If Skipzone does not beat P.533 on the doubly held-out
cell, that is the first finding of the report, stated before any other number.

### 2.6 Output

Two files per run, both written under `corpus/benchmark/runs/<date>-<commit>/`:

1. `results.tsv`: one row per (model, corpus, split, cut, metric), with the
   value, the bootstrap interval, the row count and the corpus manifest hash.
   This is the file a PR diffs.
2. `report.md`: a human report in the style of the 2026-08 verification report,
   with the headline cell first, then the comparison against P.533, then each
   metric table, then the reliability diagrams drawn as text tables (bin, mean
   predicted, observed, count, interval). It records the commit, the
   `solve_digest` fingerprint, rustc version, the ITURHFProp commit, the seed
   and every licence and attribution line from the manifests.

A copy of `report.md` that carries no third-party measurements can be committed
as a dated report under `docs/reports/`. Per-row residuals stay in `corpus/`.

## 3 Candidate methods

In roughly the order of expected gain per unit of effort. Every expected-effect
figure is from published work on other models; none has been measured on
Skipzone.

### 3.1 Effective solar index fitted to live ionosondes

**What.** Replace the sunspot number that drives the CCIR maps with an index
fitted so the maps match recent GIRO foF2. The CCIR evaluation is linear in R12
in coefficient space (`A = F2[0] (1 - R/100) + F2[1] (R/100)` in `ccir.rs`), so foF2
at each station is exactly `a + b R` and the least-squares R is closed form:
`R_eff = sum b (y - a) / sum b^2`, with `y` the measured foF2. A
per-day, or per-day-and-hemisphere, R_eff absorbs the day-to-day global level.
It also closes the open R12-versus-daily gap (report section 4.1, 0.39 MHz).

- **Data source:** GIRO DIDBase, as `iono_check` uses today.
- **Licence:** CC BY-NC-SA 4.0 (section 2.2.4). The fitted index is a derived
  product, which the Rules of the Road do not restrict, so the index itself
  could be cached or published with attribution. The soundings cannot be.
- **Availability:** near real time for stations that autoscale and upload; the
  exact latency per station varies and should be measured from the data.
- **Effort:** S. The fit is a few lines on top of the existing evaluator, and
  `iono_check` already holds the comparison code.
- **Plugs in:**
  - fit: a pure function in `crates/skipzone-iono/src/ccir.rs`, for example
    `fit_effective_r12(obs: &[(lat, lon, month, utc_h, fof2)]) -> f64`, using
    `CcirMonth::plane` and `CcirPlane::fof2_at_modip`;
  - fetch: a new `crates/skipzone-lab/src/giro.rs`, calling `net::get_text`;
  - use: `scenario::ccir_peak` in `crates/skipzone-predict/src/scenario.rs`
    currently calls `map_r12_from_ssn_v2(inputs.ssn)`. An `Inputs` field for an
    explicit map index (with provenance in `Assumptions::fof2_source`) lets it
    bypass the conversion. foE keeps the sunspot number.
- **Expected effect:** the literature is mixed. An ionosonde-derived index
  (IG, Liu et al. 1983) is the IRI's long-standing alternative to R12; a review
  of indices for monthly medians found R12 at least as good as IG12 for monthly
  median foF2. A hemispheric variant of IG is reported to cut daytime and
  night-time foF2 errors by up to a factor of two. A Chinese regional study
  using an equivalent sunspot number for real-time updating reported only about
  0.75 % RMS reduction. The gain for Skipzone is mainly expected on the
  day-to-day level and on the 0.39 MHz driver gap, and less on local structure.
- **Measured by:** `iono_check` with leave-station-out (fit R_eff on the other
  stations, score the held-out one), then the benchmark's MUF and end-to-end
  cells.
- **Risks:** station coverage is uneven, so a global index is dominated by the
  European and American stations. An index fitted per day needs data from that
  day, so a forecast mode needs a persistence rule. The app has no network
  access (`skipzone-app`: no `ureq`), so live use in the GUI needs either a
  value the user pastes in or a boundary change.

<!-- OWNER: would you accept the app fetching one index per day (a boundary change), or keep the GUI offline and live use in the harnesses only? -->

### 3.2 Real-time assimilation of foF2 and hmF2

**What.** Interpolate the residual (measured - model) at ionosonde stations into
a smooth correction field over latitude, longitude and time, with a Gaussian
process or kriging, and add it to the model. Compare against IRTAM, which does
the same job inside IRI with GIRO data.

- **Data source:** GIRO DIDBase for the residuals; IRTAM coefficients from the
  GAMBIT system (`https://giro.uml.edu/GAMBIT/`), with a scripted URL of the
  form `lgdc.uml.edu/rix/gambit-coeffs?time=<ISO>&charName=<foF2|hmF2|B0|B1>`
  and a request spacing of about 15 seconds.
- **Licence:** GIRO CC BY-NC-SA 4.0. For IRTAM, the PyIRTAM paper (Forsythe
  et al. 2024) says coefficients two days old can be downloaded without an
  account, while the real-time set needs a GAMBIT account and is free only for
  academic investigations. No separate licence text for the coefficients was
  found. They come from GIRO data, so the same share-alike reading probably
  applies; this needs confirming with the data centre before anything is cached
  beyond local use.
- **Availability:** GIRO near real time; IRTAM real time with an account, two
  days late without; the GAMBIT page states the latest three days are not on the
  open page.
- **Effort:** M. A GP with a Matern or squared-exponential kernel has an
  analytic gradient, which the engine requires (`PeakDensitySource` must return
  analytic partials, or the `H = 0` diagnostic drifts). Kernel length scales
  must be fitted on held-out stations. hmF2 is harder than foF2: today the F2
  layer takes one peak radius (`SolarChapmanLayer::new` in `chapman.rs`), so a
  varying hmF2 needs the layer to accept a peak-height source with gradients.
  That change is shared with method 4.
- **Plugs in:** a new `crates/skipzone-iono/src/assimilate.rs` holding a
  `ResidualField` and an `AssimilatedF2Peak { base: CcirF2Peak, field }` that
  implements `PeakDensitySource`; a new `Fof2Backend::Assimilated` in `fof2.rs`
  selected in `scenario::build_models`. An IRTAM backend is a second option: the
  coefficients are in a CCIR-like format, so `ccir.rs`'s evaluator may read them
  with a different parser. That has to be checked against a real coefficient
  file.
- **Expected effect:** for IRTAM against plain IRI, a preliminary evaluation of
  about 15 million points found an average improvement factor of about 2, larger
  in disturbed periods (Bilitza et al. 2017). A later independent comparison
  found IRTAM better than IRI by a factor of about 1.6 on ionosonde foF2, but no
  better on COSMIC radio occultation, and concluded that the gain is confined to
  the neighbourhood of assimilated stations (Pignalberi et al.). The same pattern
  should be expected here: a large gain over Europe and North America, little in
  regions with no station.
- **Measured by:** leave-station-out foF2 and hmF2 RMS against IRTAM on the same
  soundings, then MUF and end-to-end cells. The region hold-out matters most
  here, because it shows the far-from-station case.
- **Risks:** depends on method 1 (assimilate residuals from the best index, not
  from a biased one). IRTAM terms may not allow anything beyond local academic
  use. Station data latency limits a forecast mode.

### 3.3 GNSS total electron content constraining NmF2

**What.** Use measured vertical TEC to scale NmF2 through the equivalent slab
thickness, `NmF2 = TEC / tau`, either as a direct multiplier on the F2 peak or
as extra pseudo-observations for the method 2 field. TEC maps cover the oceans
and the southern hemisphere, where ionosondes are sparse.

- **Data sources:**
  - IGS combined GIMs in IONEX format, at CDDIS
    (`https://cddis.nasa.gov/archive/gnss/products/ionex/`, by year and day of
    year), long names such as `IGS0OPSFIN` and `IGS0OPSRAP`. 5 x 2.5 degree grid,
    two-hour maps. Final about 11 days late, rapid under 24 hours.
  - CODE GIMs from AIUB (`ftp://ftp.aiub.unibe.ch/CODE/`), rapid and predicted in
    the main directory, finals by year, delivered to CDDIS weekly.
  - NOAA GloTEC, operational since about 2025-02-18 and replacing US-TEC, at
    `https://services.swpc.noaa.gov/products/glotec/`, in NetCDF and JSON, on a
    2.5 x 5 degree grid. NCEI archives GloTEC from 2025-02.
- **Licences and access:**
  - IGS: the "IGS Data and Product Disclaimer and Terms of Use" (2020-08-05)
    makes products openly available to scientific, educational and commercial
    users, places all risk on the user and expects acknowledgement. No CC
    licence applies to the products.
  - CDDIS: HTTPS needs a NASA Earthdata Login account (a `.netrc` entry for
    `urs.earthdata.nasa.gov`). The archive access page still documents anonymous
    FTP over TLS. Earthdata is migrating its sites through the end of 2026, so
    paths may move.
  - CODE: the AIUB server is anonymous; no licence statement was found beyond
    older documents describing the maps as freely available.
  - GloTEC: a US Government product. Archived SWPC and NOAA pages describe their
    data as public domain, asking for credit and no implied endorsement. The
    current SWPC disclaimer page could not be read from this session.
  - IGS and GloTEC can therefore be cached; CODE and IGS files carry no
    redistribution restriction that was found, but the attribution must travel.
- **Availability:** GloTEC real time; IGS and CODE rapid within a day; finals
  one to two weeks late.
- **Effort:** M to L. An IONEX parser and a gridded source with C1 interpolation
  are M (the `fof2.rs` grid already has a Catmull-Rom sampler with gradients).
  The slab-thickness model is the hard part.
- **Plugs in:** an IONEX and GloTEC reader in `crates/skipzone-lab/src/tec.rs`;
  a pure gridded `TecMap` in `crates/skipzone-iono/src/` (no I/O, built from
  parsed values); a `PeakDensitySource` that scales the CCIR peak by the TEC
  ratio, or residuals fed into method 2's `ResidualField`.
- **Expected effect:** uncertain. Published GNSS TEC and NmF2 variability is
  15 to 25 % (10 to 30 % in another version of the same paper, Jakowski and
  Hoque 2021), and the ratio tau varies more than either. Existing tau models
  are reported as not accurate enough to determine NmF2 on their own. Since foF2
  goes as the square root of NmF2, a 15 % tau error is about 7 % in foF2, about
  0.45 MHz at 6.3 MHz, which is smaller than today's 1.10 MHz RMS. Whether it
  helps depends on whether the tau error is smaller than the climatology's
  day-to-day error, which only the benchmark can say.
- **Measured by:** leave-station-out foF2 at GIRO stations not used for tau,
  with the region hold-out reported separately for ocean and southern paths.
- **Risks:** TEC includes the plasmasphere, which matters most at night;
  GIMs are smooth on a 2.5 x 5 degree grid and miss small structure; CDDIS
  access needs an account.

### 3.4 NeQuick 2 or IRI-2020 profile shape

**What.** Replace the single F2 Chapman layer at a fixed height and scale height
with a profile whose peak height and shape come from data. The first step is
cheap: derive hmF2 from M(3000)F2, which `ccir.rs` already evaluates (the
report notes this is unused). The full step is the NeQuick bottomside (Epstein
layers for E, F1 and F2 with thickness parameters derived from foF2, M(3000)F2
and foE).

- **Data sources:**
  - NeQuick G, already vendored in `crates/skipzone-iono/data/nequick/`: the
    CCIR files and the modip grid, from the Galileo ionospheric correction ICD,
    under EUPL-1.2 (the iono crate is already `MIT AND EUPL-1.2`). The EU
    released the NeQuick G source code under EUPL-1.2 as well.
  - NeQuick 2 from ICTP (`https://t-ict4d.ictp.it/nequick2`). The terms page
    restricts the web model and its results to scientific, personal,
    non-commercial use and requires written permission otherwise. No separate
    licence for the source code was found.
  - ITU-R P.531-16 (09/2025) ships a NeQuick program as a supplement file. The
    ITU's general notice forbids reproduction without written permission; whether
    that covers the supplement software was not established.
  - IRI from `https://irimodel.org/`. The site lists IRI-2020 (updated
    2025-09-25) and a new IRI-2026 (2026-09-28). The package ships README and
    LICENSE files, whose text could not be read from this session. PyIRI and
    PyIRTAM (Forsythe, NRL) are listed as MIT by a third-party index.
- **Licence consequence:** NeQuick G is the clean route. It is the NeQuick 2
  formulation driven by an effective ionisation level, and its licence is the one
  the repository already carries. Porting the profile formulas from the NeQuick G
  code keeps everything under EUPL-1.2.
- **Availability:** static models; nothing to fetch.
- **Effort:** S for hmF2 from M(3000)F2 (one formula, a peak-height source with
  gradients, shared with method 2). M to L for the full NeQuick bottomside,
  because the engine needs analytic horizontal partials of the whole profile,
  and the Epstein layer parameters depend on several gridded quantities.
- **Plugs in:** a peak-height source in `crates/skipzone-iono/src/chapman.rs`
  alongside `PeakDensitySource`; an `M3000` evaluator wrapper beside `CcirF2Peak`
  in `ccir.rs`; for the full profile, a new `crates/skipzone-iono/src/nequick.rs`
  implementing the engine's `ElectronDensity`, chosen in `scenario::build_models`
  in place of the F2 and E layers. `Inputs::hmf2_km` and `scale_height_km` would
  become overrides.
- **Expected effect:** affects which paths exist and at what elevation: skip
  distance, hop count, MUF on oblique paths. No published figure for this exact
  change was found in this session. The direct measures are MUF(3000)F2 error and
  hmF2 error at ionosondes.
- **Measured by:** MUF and hmF2 against GIRO; then the end-to-end cells, looking
  at the hop-geometry cuts that `report_hop_geometry` already prints.
- **Risks:** the derivation for any new layer should be written first under
  `docs/derivations/iono/`, as the project does for Chapman layers, and the
  gradient checks extended. A sharper profile may expose numerical issues at
  reflection that the smooth Chapman layer hides.

### 3.5 D-region: profile, collision frequency, flares and PCA

**What.** Three linked changes. (a) Replace the three collision-frequency anchors
with a profile computed from a neutral atmosphere model (NRLMSIS), which removes
three guessed constants and fixes the `H_nu` that sets the zenith exponent.
(b) Replace the alpha-Chapman D layer with an empirical D-region profile (IRI's
D-region options or FIRI-2018). (c) Add event-driven absorption for X-ray flares
and polar cap absorption from GOES data or NOAA D-RAP. All three feed the
engine's full complex Appleton-Hartree index unchanged.

- **Data sources and licences:**
  - NRLMSIS 2.1 from NRL (`https://map.nrl.navy.mil/map/pub/nrl/NRLMSIS/NRLMSIS2.1/`).
    The code is governed by an "Open Source Academic Research License Agreement"
    (`nrlmsis2.1_license.txt`), whose text could not be retrieved here. A wrapper
    project describes the MSIS 2.0 licence as "extremely restrictive". It cannot
    be assumed vendorable. Precomputing a small collision-frequency table offline
    and vendoring the table is an option only if the licence allows derived data.
  - FIRI-2018 (Friedrich, Pock and Torkar). The pyFIRI paper says the authors
    made the data "freely available" and gave permission to adopt them; no
    general licence was found.
  - IRI D-region options: IRI licence not read (section 3.4).
  - GOES X-ray flux: SWPC JSON under `https://services.swpc.noaa.gov/json/goes/primary/`
    (for example `xrays-7-day.json`), 1-minute averages in the 0.05-0.4 nm and
    0.1-0.8 nm passbands. GOES-R XRS uses a different irradiance calibration from
    earlier satellites, which changes flare classes. US Government data, treated as
    public domain (section 3.3).
  - D-RAP: `https://services.swpc.noaa.gov/text/drap_global_frequencies.txt`
    (current grid, ASCII), archive at NCEI from 2009-09. Driven by 1-minute GOES
    X-ray and 5-minute GOES proton flux. Model description: Sauer and Wilkinson
    2008. An experimental release 2 (DRAP2) is documented by NCEI (2025-07).
- **Availability:** NRLMSIS and FIRI static; GOES and D-RAP real time, archived
  at NCEI.
- **Effort:** L. Three separate data sources, a new layer and a new collision
  model, each with analytic gradients, plus a corpus that contains flares.
- **Plugs in:**
  - collisions: a new `CollisionFrequency` implementation (the engine trait in
    `crates/skipzone/src/collision.rs`, returning `nu` and its partials), in
    `skipzone-iono` or `skipzone-predict`, replacing `ExponentialCollisions::new`
    in `scenario::build_models`;
  - D profile: a new `ElectronDensity` replacing `SolarChapmanLayer::d_region`
    in the same function;
  - events: a GOES/D-RAP reader in `skipzone-lab`, and an event term in the D
    profile or an extra loss in `crates/skipzone-predict/src/solve/link_budget.rs`.
  The bounds in `calib::IonosphereAnchors` for `nu_ref_per_s`, `nu_ref_alt_km`
  and `nu_scale_height_km` go away for the collision model, which is the point.
- **Expected effect:** on the zenith exponent, empirical absorption laws use
  `cos^0.75 chi` (Davies 1965, as used in a GIRO absorption study), a 1960s
  survey found exponents between about 0.5 and 1.0 against 1.5 for a Chapman
  layer, and Rai (1969) derives a frequency-dependent exponent. The current
  1.39 is near the Chapman end. On the corpus, the report shows the zenith
  law is invisible above 30 m and worth up to 10 dB on single 160 m and 80 m
  spots. D-RAP validation over 14 riometers and 94 proton events found
  reasonable average performance with subauroral tuning needed, and a 2015
  physics-based approach outperformed it.
- **Measured by:** the end-to-end cells on 160 m and 80 m by day (needs the RBN
  corpus; WSPR has too few such spots), and a flare-time cut once the corpus spans
  a flare. Riometer data or oblique ionograms would test the zenith law directly;
  no source for them is proposed here.
- **Risks:** the existing corpus cannot see most of this (report section 7.2).
  Without the RBN low-band data, the benchmark may show no change even if the
  physics is better, and the merge rule (section 4) would then block it.

<!-- OWNER: if a D-region change is physically sound but invisible to the benchmark, does it wait for a corpus that can see it, or is there an exception? -->

### 3.6 NEC-2 antenna gain tables

**What.** Compute gain against elevation for each `AntennaKind` (dipole,
vertical, EFHW) over the ground types in `GroundType`, at a grid of heights and
frequencies, with `nec2c`, and load them through the existing
`TabulatedPattern`. Validate against published patterns.

- **Data source:** generated locally with `nec2c`
  (`https://github.com/KJ7LNW/nec2c`, cloned 2026-11-05 at `55be1e0e`,
  2024-12-16).
- **Licence:** inconsistent. The README says "nec2c is Public Domain, same as the
  original FORTRAN source" and asks that derived software stay public domain or
  use an open licence; the repository's `COPYING` file is GPL-3. Debian lists the
  upstream files as permissive. Running it as an external tool and committing
  only its numeric output avoids the question; vendoring its source does not.
- **Availability:** offline, run once per table set.
- **Effort:** M. The generator script and the table format are S; choosing
  wire geometries, feed models and ground parameters, and checking them against
  references, is the work.
- **Plugs in:** `crates/skipzone-predict/src/antenna/table.rs` (`TabulatedPattern`,
  `PatternBlock`), new `AntennaKind` variants or replacements in
  `antenna/mod.rs`, returned from `AntennaConfig::build`. The module docs
  already list the steps and a `data/antennas/` location.
- **Expected effect:** small on the benchmark. The report puts image theory 0.3 to
  0.6 dB optimistic against NEC-4, below the 8 dB residual, and absolute gain is
  absorbed into station effects. The elevation-dependent shape matters more: the
  calibrator's comments record that a fixed-height dipole tilts gain by 11.6 dB
  per end across 160 m to 10 m. NEC-2 with Sommerfeld-Norton ground is reported
  to agree with NEC-4 for wires a few percent of a wavelength or more above
  ground, and to diverge for wires very close to or touching lossy ground, which
  is the low vertical and radial case.
- **Measured by:** the NCDXF corpus, if the beacons' antennas are documented;
  otherwise only by the GUI's default-antenna predictions, which the benchmark
  scores with no station effects. Validation against published patterns is a
  unit test, separate from the benchmark.
- **Risks:** without a corpus that knows the antennas, the benchmark cannot
  show a gain, so this may not pass the merge rule however correct it is.

<!-- OWNER: priority for antennas, given that the report's own commentary deprioritises them? -->

### 3.7 Probabilistic output from an ensemble

**What.** Run N members with foF2, hmF2 and the absorption index perturbed by
their measured errors, and report P(decode) as the fraction of members that
close above the threshold, combined with fading. Today P(decode) is
`Phi(margin / sigma)` with one fitted sigma (`noise::decode_probability`,
`PREDICTIVE_SPREAD_DB = 7.3`), which cannot change the AUC. An ensemble can,
because members that lose the path entirely (below MUF, into the skip zone) are
not a monotone function of the median margin.

- **Data source:** the benchmark's own error estimates: foF2 and hmF2 error
  distributions from the ionosonde corpus (with their spatial correlation), the
  absorption spread from the calibration, the 3.5 dB fading figure from the
  report.
- **Licence:** none beyond the corpora already used.
- **Availability:** offline.
- **Effort:** M. Perturbing `Inputs` and `calib::Anchors` is simple; making N
  solves affordable is the work (N = 20 multiplies a few hundred milliseconds
  per solve by 20), along with correlated perturbations along the path.
- **Plugs in:** a new `crates/skipzone-predict/src/ensemble.rs` wrapping
  `scenario::resolve`, `scenario::build_models` and `solve::solve`, run on
  `compute::ComputePool`; `LinkBudget::decode_probability` in `noise.rs` reads
  the ensemble fraction when present. The calibrator's decode-probability
  section in `negatives.rs` becomes the check.
- **Expected effect:** better calibration (reliability term of the Brier score)
  and possibly better discrimination at the MUF and skip-zone edges, where most
  night-time false positives sit. No published ensemble result for HF
  link prediction was found in this session; one 2025 study propagates foF2
  and hmF2 confidence intervals through a ray tracer for illustration only.
- **Measured by:** Brier score and decomposition, reliability diagram, AUC, on
  the NCDXF corpus first (real base rate), then WSPR and RBN.
- **Risks:** depends on step 0 for the error sizes and benefits from methods 1
  and 2 shrinking them. Cost may force a smaller N in the GUI than in the
  benchmark.

### 3.8 Residual correction model (last, optional)

**What.** A small statistical model trained on benchmark residuals (features
such as band, local time, distance, hop count, layer, season, magnetic latitude)
that adds a correction to the predicted SNR, in the way `model_bias_db` adds a
constant today.

- **Data source:** the benchmark's own training splits.
- **Licence:** the model is a derived product; the GIRO terms allow derived
  products, and the WSPR terms require results to be free to everyone.
- **Effort:** M.
- **Plugs in:** training in `skipzone-lab` (a new binary beside `benchmark`); the
  fitted table in `skipzone-predict`, applied after `LinkBudget::new`.
- **Expected effect:** unknown. It can only remove structure the physics leaves
  in the residual. The report's negative results (a daytime systematic term worth
  about 0 dB, the model is spread-limited) suggest little systematic structure is
  left.
- **Accepted only if** the doubly held-out cell improves, with the paired
  bootstrap interval excluding zero. It never replaces a physics fix: a
  correction that tracks a known physical error (for example the zenith law) is a
  reason to fix the physics.
- **Risks:** overfitting the corpus's station mix; hiding errors that the
  physics methods would otherwise expose; station effects leaking into the
  features.

<!-- OWNER: do you want method 8 on the list at all? -->

## 4 Summary and the merge rule

| # | Method | Source | Licence / terms | Effort | Expected effect | Depends on |
|---|---|---|---|---|---|---|
| 0 | Benchmark | WSPR, RBN, NCDXF schedule, GIRO; ITURHFProp | wspr.live non-commercial, results free to all; RBN "freely available for study and analysis"; GIRO CC BY-NC-SA 4.0, share only with LGDC account holders; ITU implementer grant | M | None by itself. Gives the current numbers and the P.533 comparison | Re-run of `wspr_calibrate` |
| 1 | Effective index from ionosondes | GIRO DIDBase | CC BY-NC-SA 4.0; derived index unrestricted | S | Removes the 0.39 MHz driver gap and some day-to-day level error. Literature mixed, from about 0.75 % to a factor of two | 0 |
| 2 | foF2/hmF2 assimilation, against IRTAM | GIRO; IRTAM via GAMBIT | GIRO as above; IRTAM real-time needs an account, academic use; 2-day-old open | M | IRTAM: about 1.6 to 2 times better than IRI near stations, no gain far from them | 0, 1; hmF2 part shares work with 4 |
| 3 | GNSS TEC constraining NmF2 | IGS/CODE GIMs (CDDIS, AIUB); NOAA GloTEC | IGS open with attribution; CDDIS needs Earthdata Login; GloTEC US Government public domain | M-L | Uncertain; tau error of 10-30 % maps to about 5-15 % in foF2 | 0, 2 |
| 4 | Profile shape (hmF2 from M3000, NeQuick bottomside) | Vendored NeQuick G data; NeQuick G code | EUPL-1.2 (already carried); NeQuick 2 ICTP and IRI terms not cleared | S (hmF2) / M-L (full) | Better MUF, skip distance and hop geometry; no figure found | 0 |
| 5 | D-region profile, NRLMSIS collisions, GOES/D-RAP events | NRLMSIS 2.1, FIRI-2018, SWPC GOES JSON, D-RAP | NRLMSIS academic research licence (restrictive, unread); FIRI permission-based; SWPC public domain | L | Zenith exponent from 1.39 towards about 0.75; flare and PCA cases; visible only on low bands | 0, RBN corpus |
| 6 | NEC-2 antenna tables | nec2c, run locally | README public domain, repository GPL-3; only output committed | M | Small on the benchmark (0.3-0.6 dB level, absorbed by station effects); elevation shape matters more | 0, NCDXF corpus to measure |
| 7 | Ensemble P(decode) | Benchmark error estimates | None new | M | Better Brier reliability; possible AUC gain at MUF and skip edges | 0; better with 1 and 2 |
| 8 | Residual correction (optional) | Benchmark residuals | Derived | M | Unknown; report suggests little systematic structure left | 0 and everything before it |

**The rule.** Each method is its own pull request. The PR description carries the
benchmark's before and after `results.tsv` rows for the doubly held-out cell and
the single hold-outs, with the paired bootstrap interval on the difference. A
method that does not improve the held-out score is not merged, however sound its
physics. A method may make some cuts worse; the PR shows which ones, and the
owner decides whether the trade is acceptable.

<!-- OWNER: what counts as "improves": any metric, a named primary metric (for example Brier on the doubly held-out cell), or a set that must all hold? And is a significance threshold required, or the sign of the interval? -->

## 5 Open questions for the owner

<!-- OWNER: the order. Section 3 is ordered by expected gain per effort, which is an estimate; you may weight things differently (for example, the GUI's night-time false positives over the RMS). -->

<!-- OWNER: how much CPU time a benchmark run may take, which sets the corpus size and the ensemble N. -->

<!-- OWNER: whether the benchmark itself should run in CI on a small fixed subset, given that the corpora cannot be committed. -->

<!-- OWNER: whether you will apply for an LGDC account and a GAMBIT account, and an Earthdata Login, and in whose name; the licences tie data access to accounts. -->

<!-- OWNER: whether a commercial use of Skipzone is ever intended. GIRO (NC), wspr.live (no commercial use), ICTP NeQuick 2 (scientific use) and NRLMSIS (academic) all restrict it, and the choice of sources depends on the answer. -->

<!-- OWNER: whether VOACAP is worth the second baseline (section 2.5). -->

<!-- OWNER: whether the sources marked unconfirmed in section 6 should be re-checked before any work starts, and by whom. -->

## 6 How the sources were checked

Checked on 2026-11-05. The session that wrote this note could not open the
sources' own web pages: its network policy refused connections to
`giro.uml.edu`, `www.swpc.noaa.gov`, `services.swpc.noaa.gov`, `cddis.nasa.gov`,
`www.sidc.be` and `db1.wspr.live`. Three kinds of evidence were used instead,
marked in the references:

- **[repo]** the source repository was cloned and its licence file and README
  read directly: ITU-R-HF, voacapl, nec2c;
- **[search]** the claim rests on a web search index entry for the URL given,
  which may lag the live page;
- **[repo-local]** the claim comes from this repository's own files.

Every **[search]** licence entry should be confirmed on the live page before data
is cached or anything is vendored. The ones that matter most, because they decide
what can be stored:

- GIRO DIDBase Rules of the Road (the share-alike reading);
- IRTAM / GAMBIT coefficient terms (none found);
- NRLMSIS 2.1 licence text (not read);
- IRI-2020 and IRI-2026 LICENSE file (not read);
- NeQuick 2 source-code licence and the ITU P.531 supplement terms (none found);
- RBN redistribution terms (only "freely available for study and analysis");
- the current SWPC data disclaimer (only archived versions found).

No source was found to be gone. Two have changed: GloTEC moved from
`/experimental/products/glotec/` to `/products/glotec/` when it became
operational, replacing US-TEC; and IRI-2026 has been released since the IRI-2020
update this note was asked to cover.

## 7 References

All accessed 2026-11-05.

**Repository**

- [repo-local] Verification report, `docs/reports/2026-08-verification.md`.
- [repo-local] NeQuick G data provenance, `crates/skipzone-iono/data/nequick/_SOURCE_AND_LICENSE.txt`.

**Corpora**

- [search] wspr.live: https://wspr.live/
- [search] Third-party report of the WSPRnet archive gap: https://github.com/IONIS-AI/ionis-apps/issues/5
- [search] RBN raw data: https://www.reversebeacon.net/raw_data/
- [search] RBN CSV header, third-party check: https://github.com/qso-graph/qso-graph-spec/pull/5
- [search] NCDXF/IARU beacon schedule: https://www.ncdxf.org/beacon/beaconschedule.html
- [search] NCDXF beacon interference and band-plan note: https://www.ncdxf.org/beacon/beaconinterference.html
- [search] GIRO DIDBase Rules of the Road: https://giro.uml.edu/didbase/RulesOfTheRoad.html

**Baselines**

- [repo] ITU-R-HF (ITURHFProp, P533, P372), commit `82017594`: https://github.com/ITU-R-Study-Group-3/ITU-R-HF
- [search] Third-party Linux build fixes for ITU-R-HF: https://github.com/ITU-R-Study-Group-3/ITU-R-HF/pull/45
- [search] Recommendation ITU-R P.533-14: https://www.itu.int/dms_pubrec/itu-r/rec/p/R-REC-P.533-14-201908-I!!PDF-E.pdf
- [repo] voacapl, commit `c12a98b3`: https://github.com/jawatson/voacapl
- [search] NVIS comparison of REC533, VOACAP and ICEPAC: https://www.researchgate.net/publication/316860666_The_quality_of_the_prediction_for_the_NVIS_propagation_with_ITS-HF_propagation

**Ionosondes, indices and assimilation**

- [search] GAMBIT (IRTAM coefficients): https://giro.uml.edu/GAMBIT/
- [search] Forsythe et al. 2024, PyIRTAM, Space Weather: https://agupubs.onlinelibrary.wiley.com/doi/full/10.1029/2024SW003965
- [search] PyIRTAM licence listing: https://science.ecosyste.ms/projects/9504
- [search] Bilitza et al. 2017, IRI 2016, Space Weather: https://agupubs.onlinelibrary.wiley.com/doi/10.1002/2016SW001593
- [search] Pignalberi et al., IRI and IRTAM comparison: https://www.researchgate.net/publication/353681728_Towards_a_Real-Time_Description_of_the_Ionosphere_A_Comparison_between_International_Reference_Ionosphere_IRI_and_IRI_Real-Time_Assimilative_Mapping_IRTAM_Models
- [search] Galkin et al. 2012, Assimilation of GIRO data into a real-time IRI: https://www.researchgate.net/publication/258710236_Assimilation_of_GIRO_data_into_a_real-time_IRI
- [search] Ionosonde-based indices for IRI solar-cycle variation: https://www.sciencedirect.com/science/article/abs/pii/S1364682617301724
- [search] Review of ionospheric indices (R12 against IG12): https://elpub.wdcb.ru/journals/ijga/v01/gai98239/gai98239.htm
- [search] foF2 instantaneous mapping with an equivalent sunspot number: https://www.researchgate.net/publication/251986305_A_study_on_the_ionospheric_parameter_foF2_instantaneous_mapping_based_on_equivalent_sunspot_number
- [search] Effective solar indices for ionospheric modelling, review: https://link.springer.com/article/10.1007/s10712-017-9438-y
- [search] Kriging method of foF2 instantaneous mapping: https://www.researchgate.net/publication/233401255_The_Kriging_method_of_ionospheric_parameter_f0F2_instantaneous_mapping

**GNSS TEC**

- [search] IGS products: https://igs.org/products/
- [search] IGS Data and Product Disclaimer and Terms of Use (2020-08-05): https://iwww.igs.org/wp-content/uploads/2020/09/IGS-Data-and-Product-Disclaimer-and-Terms-of-Use-200805.pdf
- [search] IGS long product filename guidelines: https://files.igs.org/pub/resource/guidelines/Guidelines_for_Long_Product_Filenames_in_the_IGS_v2.1.pdf
- [search] CDDIS archive access: https://www.earthdata.nasa.gov/centers/cddis-daac/archive-access
- [search] CDDIS final ionosphere products: https://www.earthdata.nasa.gov/data/catalog/cddis-gnss-fnl-ion-prod-1
- [search] CODE global ionosphere maps: http://aiuws.unibe.ch/ionosphere/
- [search] SWPC GloTEC: https://www.spaceweather.gov/products/glotec
- [search] NWS Service Change Notice 25-04, GloTEC operational: https://www.weather.gov/media/notification/pdf_2025/scn25-04_glotec_ionosphere_model_t2o.pdf
- [search] NCEI total electron content archive: https://www.ncei.noaa.gov/products/space-weather/ionospheric-program/total-electron-content
- [search] Jakowski and Hoque 2021, global equivalent slab thickness model: https://www.swsc-journal.org/articles/swsc/full_html/2021/01/swsc200087/swsc200087.html

**Profiles**

- [search] IRI model home: https://irimodel.org/
- [search] CCMC IRI 2020 listing (package contents): https://ccmc.gsfc.nasa.gov/models/IRI~2020/
- [search] NeQuick 2 terms of use (ICTP): https://t-ict4d.ictp.it/nequick2/terms-of-use
- [search] NeQuick G code release (EUPL-1.2): https://www.euspa.europa.eu/newsroom-events/news-archive/nequick-g-code-available-download
- [search] NeQuick G terms: https://www.gsc-europa.eu/support-to-developers/ionospheric-correction-algorithms/nequick-g-source-code/nequick-g-terms-and
- [search] Recommendation ITU-R P.531-16: https://www.itu.int/dms_pubrec/itu-r/rec/p/R-REC-P.531-16-202509-I!!PDF-E.pdf

**D region and events**

- [search] NRLMSIS 2.1 distribution (via its unofficial mirror's README): https://github.com/jacobwilliams/NRLMSIS2.1
- [search] NRLMSIS 2.0 Python interface (licence warning): https://github.com/space-physics/nrlmsis2.0
- [search] pyFIRI and FIRI-2018 data permission: https://www.sciencedirect.com/science/article/pii/S2352711021001461
- [search] SWPC GOES JSON directory: https://services.swpc.noaa.gov/json/goes/primary/
- [search] SWPC GOES X-ray flux: https://www.swpc.noaa.gov/products/goes-x-ray-flux
- [search] SWPC D-RAP: https://www.spaceweather.gov/products/d-region-absorption-predictions-d-rap
- [search] Global D-RAP documentation: https://www.spaceweather.gov/content/global-d-region-absorption-prediction-documentation
- [search] Sauer and Wilkinson 2008, technical description: https://www.ngdc.noaa.gov/stp/drap/Sauer_DRAP_Tech_Description.pdf
- [search] D-RAP validation report: https://ngdc.noaa.gov/stp/drap/DRAP-V-Report1.pdf
- [search] DRAP2 experimental release: https://www.ncei.noaa.gov/sites/default/files/2025-07/SWxDRAP2.pdf
- [search] NCEI D-RAP archive: https://www.ncei.noaa.gov/products/space-weather/ionospheric-program/d-region-absorption-prediction
- [search] Archived SWPC data statement (public domain): https://webarchive.library.unt.edu/web/20121014023244/http://www.swpc.noaa.gov/Data/about_data.html
- [search] HF wave absorption from GIRO data (Davies 1965 cos^0.75 law): https://www.sciencedirect.com/science/article/abs/pii/S0273117718309396
- [search] Seasonal anomaly in ionospheric absorption (exponent range): https://www.nature.com/articles/physci243112b0
- [search] Rai 1969, absorption against zenith angle and frequency: https://agupubs.onlinelibrary.wiley.com/doi/abs/10.1029/RS004i009p00757

**Antennas and ensembles**

- [repo] nec2c, commit `55be1e0e`: https://github.com/KJ7LNW/nec2c
- [search] nec2c packaging licence (Ubuntu): https://launchpad.net/ubuntu/noble/+source/nec2c/+copyright
- [search] K9LA, NEC-4 against NEC-2 near ground: https://k9la.us/Feb21_The_Advantage_of_the_NEC-4_Engine.pdf
- [search] Cebik, Sommerfeld-Norton and other grounds: https://antenna2.github.io/cebik/content/amod/amod99.html
- [search] foF2 and hmF2 uncertainty through ray tracing (LIFT): https://arxiv.org/pdf/2502.15093

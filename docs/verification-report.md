# Skipzone Verification Report

**Report date:** 2026-08-10
**Configuration under test:** commit `d92bf9f`, `skipzone` 0.1.0 / `skipzone-app` 0.1.0, rustc 1.97.1, Windows 11 x86-64
**Status of the subject:** pre-alpha. Not a production propagation forecast.
**Prepared** from the repository's own test suite and the
measurement logs in `corpus/`. Every number below is quoted from a run that is
reproducible with the commands in the Reproduction section, or from a log file
committed in the working tree. Nothing here is estimated.

---

## 1. Scope

This report answers three questions, kept deliberately separate because they
have different evidence behind them and are routinely conflated:

1. **Is the engine correct?** Does the ray tracer solve the equations it claims
   to solve, to the accuracy it claims?
2. **Is the ionosphere right?** Do the modelled layer characteristics match
   independent measurements of the real ionosphere?
3. **Are the predictions right?** Does an end-to-end prediction of received
   signal-to-noise ratio match observed propagation?

These are three tiers and the answers get progressively weaker as you go down
them. That ordering is the single most important thing in this document. A
project can be flawless at tier 1 and still be of limited predictive use, and
Skipzone is closer to that description than to any other.

Out of scope: usability, the map interface, performance beyond what is noted in
passing, and anything about the application's fitness for regulatory or safety
purposes (it has none).

---

## 2. Summary of findings

| Tier | Subject | Method | Verdict |
|---|---|---|---|
| 1 | Ray tracer and magnetoionic theory | 273 automated tests, incl. 21 analytic/invariant integration tests | **Verified.** Agrees with closed-form solutions and passes every physical invariant tested. |
| 2 | foE (E layer critical frequency) | 28 952 GIRO ionosonde soundings, nothing fitted | **Verified.** Bias -0.00 MHz, RMS 0.20 MHz. |
| 2 | foF2 (F2 layer critical frequency) | 70 219 GIRO soundings, four seasonal windows, nothing fitted | **Verified with a known deficiency.** Bias +0.03 MHz, RMS 1.10 MHz, correlation 0.894. Compresses: model SD 2.16 vs measured 2.45 MHz. |
| 3 | End-to-end SNR prediction | 9 126-spot WSPR corpus, fit plus month hold-out | **Partially validated, and the headline figures are STALE.** See section 6. |
| - | Absorption zenith law | `bin/zenith_probe`, `bin/rise_wiring_check` | **Not validated.** Model gives n = 1.39 against roughly 0.75 from published riometer work. Demonstrated to be invisible to the available corpus. |
| - | Antenna model | None | **Not validated.** No test, no reference data. |
| - | D-region absolute anchors, collision frequency | None | **Not validated.** Order-of-magnitude textbook constants, so documented in source. |

**Overall:** the engine is verified, the ionospheric layer characteristics are
validated against real measurements, and the end-to-end link budget is the weak
tier. On any single path prediction the honest figure from the last full
calibration is roughly **8 dB RMS**, unbiased, with useful but not strong
discrimination between paths that decode and paths that do not (AUC 0.801).

---

## 3. Tier 1: engine verification

**Method.** 273 automated tests pass on the configuration under test
(`cargo test --release --workspace`): 34 engine unit tests, 21 engine
integration tests across four analytic suites, and 218 application unit tests.
The integration suites are the load-bearing ones, because they check the tracer
against answers derived independently of it.

| Suite | Tests | What it establishes |
|---|---|---|
| `analytic_field_free` | 6 | Vacuum rays are straight; the quasi-parabolic layer matches its closed-form solution and an independent quadrature; Chapman and linear-parabolic layers match Bouguer quadrature; Bouguer's invariant and the apex condition hold along a ray; **the integrator's convergence order is measured at 5**. |
| `magnetoionic_3d` | 7 | With zero field the O and X modes are **bit-identical**; with zero density rays stay straight despite the field; forward and backward traces are reciprocal; modes split with field; vertical reflection conditions hold; the spitze meridian case fails with a typed error rather than silently; the high-frequency geometric limit is recovered. |
| `absorption` | 5 | No electrons or no collisions gives exactly zero absorption; absorption is monotone and positive; absorption is reciprocal; the field-free case matches quadrature. |
| `homing` | 3 | Field-free quasi-parabolic homing agrees with the closed form and between methods; homing with field and collisions agrees between methods; a skip-zone target correctly returns no bracket. |

**Assessment.** This is genuinely strong verification, and better than is normal
for a project at this stage. The tests are not smoke tests: they compare against
closed-form solutions, cross-check one numerical method against another, and
assert exact (bit-identical) results where the physics demands exactness rather
than merely asserting closeness. Measuring the integrator's convergence order
rather than asserting a tolerance is the kind of check that catches a silently
degraded scheme.

**What tier 1 does not establish.** That the equations are the right equations
for the real ionosphere, or that the medium handed to the tracer resembles the
sky. A ray tracer can be perfect and the prediction still wrong, because the
prediction is mostly about the medium and the link budget.

---

## 4. Tier 2: ionospheric model validation

**Source.** Lowell GIRO Data Center DIDBase, CC-BY-NC-SA 4.0, 17 stations.
70 219 accepted soundings; 45 461 rejected below confidence 75. Four seasonal
windows: 2025-10, 2026-01, 2026-04, 2026-07. Sign convention throughout is
model minus measured, in MHz. **Nothing is fitted and nothing is absorbed** into
a nuisance parameter, which is what makes this tier much stronger evidence than
tier 3.

### 4.1 foF2

| Backend | n | measured | bias | RMS | median abs | model SD | corr |
|---|---|---|---|---|---|---|---|
| bundled grid (previous default) | 70 219 | 6.34 | +0.07 | 1.70 | 1.02 | 1.91 | 0.723 |
| CCIR daily v2 | 70 219 | 6.34 | +0.31 | 1.20 | 0.65 | 2.44 | 0.888 |
| CCIR R12 v2 | 70 219 | 6.34 | +0.92 | 1.50 | 1.01 | 2.52 | 0.886 |
| **CCIR R12 v1 (current default)** | 70 219 | 6.34 | **+0.03** | **1.10** | 0.71 | 2.16 | **0.894** |
| CCIR daily v1 | 70 219 | 6.34 | -0.39 | 1.13 | 0.67 | 2.08 | 0.903 |

Measured foF2 has SD 2.45 MHz. The current default has SD 2.16, so it
**compresses**: it reads low on high days and high on low days, and no level
scale can repair that.

Two systematic effects were identified and are documented rather than papered
over:

- **Sunspot number units.** SILSO rescaled the sunspot number by 1.44 in 2015;
  the CCIR maps were fitted in 1966. Feeding the modern value in unconverted
  leaves +0.92 MHz of bias. Converting leaves +0.03 MHz. This is a unit
  conversion, not a fit.
- **R12 versus daily driver.** The maps want a twelve-month mean and a running
  application only ever holds a daily number. foF2 is concave in the driver, so
  a daily number reads **0.39 MHz low**. NOAA publishes no smoothed value for
  any month under six months old, so this is a real and currently open gap, not
  a tuning choice.

Improvement is uniform: every local-solar-time bin, every latitude band and
every season improves against the previous backend, and 15 of 16 stations
improve. The largest single gain is southern-hemisphere winter, RMS 2.41 to
0.87.

### 4.2 foE

| Cut | n | measured | bias | RMS | median abs |
|---|---|---|---|---|---|
| all | 28 952 | 2.72 | -0.00 | **0.20** | 0.12 |

Bias stays within 0.09 MHz and RMS within 0.22 MHz across all eight
local-solar-time bins. This is the best-characterised quantity in the model and
the only anchor in it that was set from measurement (3.30 to 2.99 MHz overhead
quiet foE). It matters more than its size suggests: the E layer decides which
low bands are screened off F2 by day, so its error propagates into **which paths
exist**, not merely into their loss.

**Assessment.** Tier 2 is the strongest part of the project's empirical case and
is honestly reported by the harness itself. The `iono_check` output states in
its own words what it does not settle: an ionosonde is a vertical sounding at
one point, so it says what the layer is, not what a 2000 km oblique path through
it loses. Absorption, ground reflection and the noise floor are untouched by
this evidence.

One caveat the harness also states: all four windows sit at **one position on
the solar cycle** (R12 92 to 110), so the solar-activity law itself is untested.

---

## 5. Tier 3: end-to-end validation against WSPR

### 5.1 The stale-baseline finding

**This is the most important finding in this report.**

The last full end-to-end calibration was run against the **previous** foF2
backend. On 2026-08-09 the CCIR maps became the default, which per section 4.1
moves foF2 by of order a megahertz and therefore changes **which paths exist**.
The full WSPR calibration has not been re-run since.

Everything in section 5.2 is therefore a **stale baseline**. It describes a
configuration the application no longer ships. It is reported here because it is
the only end-to-end evidence that exists, and because its structural conclusions
(the model is spread-limited, not bias-limited) are unlikely to be overturned by
a better foF2. Its *numbers* should not be quoted as current.

Behaviour fingerprints from either side of the change are preserved in
`corpus/digest_before_ccir.txt` and `corpus/digest_after_ccir.txt`. Re-running
the calibration is roughly a one-hour job and is the correct next step before
any accuracy claim is published.

### 5.2 Stale baseline results

Source: `corpus/rise_run.log`, 9 126-spot corpus, isotropic reference antenna at
both ends, hold-out is a different month. Station effects are treated as
nuisance parameters and removed, which is the only way to compare a physics
model against a population of unknown antennas and unknown claimed power.

| | fit | hold-out |
|---|---|---|
| RMS error, station effects removed | 7.90 dB | 8.00 dB |
| IQR of error, effects removed | 10.49 dB | 10.63 dB |
| median error, effects removed | +0.31 dB | +0.19 dB |
| slope vs measured, effects removed | 0.54 | 0.61 |
| R2, effects removed | 0.43 | 0.53 |
| AUC (decode vs non-decode) | 0.801 | - |
| model found a path | 2758 of 3042 (91 %) | 2781 of 3079 (90 %) |

Fit-to-hold-out gap in RMS is +0.09 dB, so the fitted values transfer to days
the fit never saw. That is a real and non-trivial result: it rules out gross
overfitting.

**Read this as:** unbiased, useful for ranking, roughly 8 dB on any single
prediction, and it **compresses** (reality swings about 10 dB, the model swings
about 6).

### 5.3 Discrimination and false positives

- **AUC 0.801.** Useful separation between paths that decoded and paths that did
  not. This number is invariant to any constant shift of the predictions, so it
  cannot be improved by moving the level, and it is invariant to the hit rate,
  so it cannot be improved by predicting more paths.
- **False positive rate, upper bound: 34.7 %** (level-matched, 34.8 %). Split at
  the midpoint terminator, 42.8 % by day and 80.4 % by night after the fit. The
  night figure is poor and is not disguised in the source log.
- **Fitted predictive spread sigma 15.21 dB.** For reference, repeat
  measurements of one real path scatter 3.5 dB and the model's out-of-sample
  error is around 7.3 to 8 dB. A fitted sigma near the latter says the spread an
  operator faces is dominated by **model error, not by fading**.

The distinction between "91 % of spots got a path" and "52 % baseline hit rate"
in the same log is not a contradiction: the first counts any path the solver
admitted, the second counts spots meeting the decode threshold before fitting.
Both are reported; neither is the headline.

### 5.4 Parameters that hit their bounds

Five fitted parameters ran to the edge of their physically defensible ranges.
The project's stated rule is that a bound being reached is a finding, not an
obstacle, and **no bound was widened to unstick the fit**. The log then does
something better than most calibration reports: it profiles the objective along
each parameter's own range and classifies whether the bound hit is evidence.

| Parameter | Verdict |
|---|---|
| absorption scale | REAL: objective falls to the edge (clamped at 0.2000, i.e. nu = 1.00e6 /s) |
| atm slope day | REAL: objective falls to the edge |
| atm day-to-night step | REAL: objective falls to the edge |
| atm step slope | weak: identified over part of the range only |
| atm step curve | **FLAT: 100 % of the range within 0.05 dB of best. The bound is arbitrary, not evidence.** |
| atm Fa 1 MHz day | interior optimum |

The absorption scale wanting to sit outside every published collision frequency
is a statement that **the error is somewhere else in the model**. Reporting the
FLAT verdict, which actively retracts one of the log's own warnings, is
unusually disciplined practice and is the reason this section can be trusted.

---

## 6. Capability statement

Verified to do the following, on the evidence above:

- Trace three-dimensional rays through a magnetised collisional plasma using the
  Haselgrove equations in Hamiltonian form, with the full complex
  Appleton-Hartree refractive index and no quasi-longitudinal or
  quasi-transverse approximation. Verified against closed-form solutions.
- Use a real IGRF-14 geomagnetic field, with O and X mode splitting.
- Reproduce the F2 critical frequency to about 1.1 MHz RMS and the E critical
  frequency to about 0.20 MHz RMS against ionosonde measurement, unfitted, over
  four seasons and 17 stations.
- Rank paths by likely quality with useful discrimination (AUC 0.801, stale
  baseline).
- Produce an unbiased median prediction with station effects removed.
- Distinguish a deterministic path from one only sporadic E can reach, and
  report the latter with an occurrence probability rather than folding the two
  together.
- Explain every verdict: which layer, which mode, what was charged, and why a
  path was refused.
- Run offline and deterministically. The solver has no network dependency and
  `solve_digest` gives a byte-level behaviour fingerprint.

---

## 7. Limitation statement

### 7.1 Quantitative limits

- **Single-path SNR accuracy is roughly 8 dB RMS** (stale baseline). Do not
  treat a prediction as better than about plus or minus 8 dB.
- **Predictions compress.** Slope against measured is 0.54 to 0.61 with station
  effects removed. Strong paths read weak and weak paths read strong.
- **False positive rate around 35 %**, and around 80 % at night in the
  level-matched night cut. The model says "workable" too often after dark.
- **No per-path variability model.** Every prediction is a point estimate
  against a quantity that fluctuates by around 10 dB. The predicted-decode
  probability layer is a calibration wrapper over one sigma, not a physical
  variability model.

### 7.2 Components with no validation at all

| Component | Status |
|---|---|
| **Antenna model** | Image theory over a flat lossy half-space. No conductor loss, no radial I2R, no Sommerfeld near-field loss, no terrain. Believed 0.3 to 0.6 dB optimistic against NEC-4, but that is a source comment, not a test. `TabulatedPattern` exists as the extension point and ships no data. |
| **D-region absolute anchors** | Peak Ne 1e9, peak altitude 85 km, scale height 6 km, night floor 0.10. Order-of-magnitude values, explicitly documented in source as not fitted and not traceable to a single citable table. |
| **Collision frequency** | Three textbook anchors (5e6 /s at 70 km, scale height 6.7 km). Not derived from a neutral-atmosphere model. |
| **Absorption zenith law** | The model imposes A(chi) proportional to cos^n chi with n derived as 1/2 + f*H_D/H_nu. Shipping anchors give **n = 1.39** against roughly **0.75** from riometer and A1 absorption measurements. |
| **hmF2** | A direct user input defaulting to 300 km. M(3000)F2 is parsed and evaluated but consumed by nothing, so the Bilitza relation that would derive hmF2 from data already in memory is not applied. |

The zenith law deserves emphasis because it is a case of a *known* discrepancy
that the available evidence **cannot** resolve. `bin/rise_wiring_check` measured
it directly: collapsing n from 1.39 to 0.50 is worth 10.22 dB on the most
sensitive spot and **+0.02 dB at the median**; identifiable spread is 0.92 dB
against a 13.6 dB residual IQR; only 36 of 679 spots move by more than 1 dB, 33
of those are 160 m and 80 m, and **none above 30 m**. The anchor is verified to
be live (a 7.5 dB lever on a 1.838 MHz noon path), so this is a null result about
the data, not a wiring fault. Resolving it requires external evidence: riometer
absorption against zenith angle, or oblique ionograms.

### 7.3 What the WSPR corpus can never identify

This is a property of the data, not a defect of the method. Any quantity that is
constant for a given station is absorbed exactly into that station's fixed
effect:

| Quantity | Identifiable from this corpus? |
|---|---|
| absorption magnitude, from its variation | yes |
| absorption vs frequency (cross-band within a station) | yes |
| absorption vs zenith angle (diurnal within a station) | yes |
| atmospheric noise day-night difference | yes |
| atmospheric noise absolute level | **no** |
| receiver noise environment | **no** |
| noise model latitude terms | **no** |
| absolute antenna gain, either end | **no** |
| claimed transmit power accuracy | **no** |
| seasonal swing | **no** (one month of corpus) |

The calibrated quantities are **shapes, not levels**.

### 7.4 Measured defects in the corpus itself

- **Pseudo-replication.** 30 receiver callsigns but only 22 distinct sites.
  Around 40 % of spots from four sites receive twelve independent fixed effects.
- **One season.** All three corpora are northern summer within seven weeks, so
  every seasonal term is untested end to end.
- **One geographic wedge.** Receivers are European and North American
  mid-latitude only. No southern, equatorial, trans-equatorial, Asia-Pacific or
  auroral paths. IGRF is about 72 % of solve cost and is exercised over one dip
  range.
- **Distance.** 80 % of spots under 2000 km, only 102 beyond 8000 km. Multi-hop
  and per-hop sea/land ground handling are barely exercised.
- **Solar activity.** SSN 83 to 126 only. No solar minimum, no flare, no SID, no
  storm.
- **Only 50 daytime 160 m and 172 daytime 80 m spots**, which is precisely where
  the zenith law's leverage lives.

Band coverage is the one dimension the corpus samples well, at 840 spots per
band by construction.

---

## 8. Negative results on record

Recorded here because a well-bounded null is a result, and because re-deriving
them costs hours:

- **A daytime systematic correction term is worth about 0 dB RMS.** Confirmed by
  four full-corpus fits. The model is spread-limited, not bias-limited.
- **No multiplicative absorption scale can produce the observed zenith
  pattern.** The residual is flat or rises as the sun sets while applied
  absorption falls.
- **The sporadic-E sheet's derived tunnelling loss is about zero.** The apparent
  bias was a selection bug plus foEs, not a missing loss term.
- **The noise model is unidentifiable along its main direction.** Three
  parameterisations with day/night steps differing by more than 20 dB score
  RMS-effects-removed 7.81, 7.83 and 7.88, and AUC 0.771, 0.776, 0.775.

---

## 9. Reproduction

```bash
cargo test --release --workspace
```

```bash
cargo run --release -p skipzone-lab --bin iono_check
```

```bash
cargo run --release -p skipzone-lab --bin zenith_probe
```

```bash
cargo run --release -p skipzone-lab --bin solve_digest
```

The full WSPR calibration that produced section 5.2, roughly one hour:

```bash
cargo run --release -p skipzone-lab --bin wspr_calibrate -- --fit corpus/fit.tsv --holdout corpus/holdout_month.tsv --negatives corpus/fit_neg.tsv --scan 300
```

`corpus/` is gitignored working data. `iono_check` and the WSPR harnesses fetch
from GIRO, SILSO and wspr.live; the engine, the solver and the GUI never touch
the network.

---

## 10. Commentary

*This section is opinion, clearly separated from the findings above.*

**The verification is inverted relative to most propagation software, and that
is to the project's credit.** The usual pattern is a confident prediction with
no traceable derivation underneath. Skipzone is the reverse: the physics is
derived from first principles before implementation, cross-checked against
analytic limits, and then the project is candid that the resulting predictions
carry about 8 dB of error. The tier 1 evidence is stronger than the tier 3
evidence by a wide margin, and the documentation says so rather than letting the
former imply the latter.

**The most valuable artefact here is not the model, it is the identification
analysis.** The table in section 7.3 and the bound-hit profiling in section 5.4
are the parts I would point another engineer at. Deciding in advance what a
dataset can and cannot resolve, then declining to fit the rest, is the step
almost everyone skips. The calibration log actively retracting one of its own
warnings ("FLAT: the bound is arbitrary, not evidence") is something I have
rarely seen in a research codebase.

**The stale baseline is the one real reporting problem.** The headline accuracy
figures describe a configuration that no longer ships. That is not misconduct,
it is an unfinished step clearly flagged in `HANDOFF.md`, but until the
calibration is re-run the project genuinely does not know its current end-to-end
accuracy. Given that foF2 RMS improved from 1.70 to 1.10 MHz and correlation
from 0.723 to 0.894, there is reason to expect improvement, and expectation is
not measurement. **This should be the next task, ahead of any new physics.**

**I would rank the outstanding work as follows.** First, re-run the calibration,
because nothing can be re-baselined until it is. Second, rebuild the corpus:
section 7.4 lists defects that limit what any future fit can conclude, and
adding seasons and southern-hemisphere paths would buy more than any modelling
change. Third, derive the collision frequency from a neutral-atmosphere model,
which removes three guessed anchors and halves the free parameters in the
zenith exponent. The antenna model is the one I would deprioritise despite it
being unvalidated, because a 0.3 to 0.6 dB error is invisible against an 8 dB
residual.

**On fitness for use.** For amateur operators asking "which band, roughly when,
and is this path plausible", the model is useful today: it is unbiased, it ranks
correctly, and it explains itself. For anyone needing a calibrated absolute
signal level, it is not suitable and does not claim to be. The night-time false
positive rate is the failure mode a user is most likely to encounter, and it is
worth stating in the interface rather than only in a report.

**The compression is the most interesting open problem.** A slope of 0.54 to
0.61 against measured, combined with an unbiased median, is the signature of a
model that has the right average and too little dynamic range. That pattern
usually indicates a missing source of variance rather than a wrong constant, and
it is consistent with the project's own conclusion that it is spread-limited.
The foF2 backend also compresses (SD 2.16 against 2.45), so at least part of the
end-to-end compression may be inherited directly from the ionospheric model.
That is a testable hypothesis and, as far as I can tell from the repository, it
has not yet been tested.

---

## 11. Conclusion

The engine is **verified**. The ionospheric layer characteristics are
**validated** against 70 219 independent measurements with nothing fitted. The
end-to-end link budget is **partially validated on a stale baseline** and is the
limiting tier.

Skipzone's own README describes the project as pre-alpha and states that no
accuracy report will be published until the first tagged release. On the
evidence assembled here that self-assessment is accurate and, if anything,
conservative about the engine and appropriately cautious about the predictions.

No claim in the repository was found to overstate what the evidence supports.
That is the finding I would most want on the record.

# Calibration

`app/src/calib.rs`, `app/src/fit.rs`, `app/src/corpus.rs`, and the
`wspr_calibrate` binary.

**Read this page before believing any calibrated number in the project.** The
central claim is not "the model is fitted", it is "here is exactly what the data
can and cannot identify, and the fit is confined to the first part."

## The anchors

`calib.rs` gathers every unverified value in the app into one place so a
calibration run can vary it and an ordinary run can ignore it. Nothing new is
introduced there. Each value already existed as a module constant with a
physical name, a unit and a docstring: the D-region and collision anchors in
`scenario`, the foE anchor in `fof2`, the sporadic-E climatology in
`sporadic_e`, and the atmospheric noise surrogate in `noise`. What the module
adds is the ability to change them from outside, plus the plausible range each
is allowed to move in.

### Why the bounds live in the code

A calibration free to put the D region eight times denser than any published
value will do exactly that, if that is what minimises its residual, and it will
then report an excellent fit to a model that is physically wrong.

So each anchor carries the range it is defensible over, and a fit that wants to
leave that range is required to say so rather than quietly widen it.
`Bounded::clamped` returns both the clamped value and whether it clamped, and a
caller that discards the flag has thrown away the finding.

**A bound being reached is a finding, not a nuisance.** It means the residual
the fit is chasing is not actually produced by the quantity it is pushing, and
the error lies somewhere else in the model.

### What is deliberately not an anchor

Free-space loss, the antenna patterns, the WSPR reference bandwidth, the decode
threshold, and everything in the engine crate. Those are either derived results
or load-bearing definitions. None of them is a calibration target.

## The identification problem

A WSPR spot's measured SNR is not a measurement of propagation alone:

```text
measured = physics + tx_effect + rx_effect + fading + error
```

`tx_effect` bundles the transmitting antenna, which is unknown and worth tens of
dB, with the accuracy of the claimed power. `rx_effect` bundles the receiving
antenna with the receiver site's local noise floor. Neither is in the archive
and both are large.

Regress physics parameters on raw measured SNR and those two unknowns do not
vanish. They get absorbed by whichever physical parameter is most flexible,
which here is D-region absorption or the noise model. The residual goes down and
the model gets worse. **That is the default outcome, not an unlucky one.**

## What is done instead

`tx_effect` and `rx_effect` are estimated explicitly, as nuisance parameters,
jointly with the physics. The physics is then identified from variation *within*
a station rather than across stations:

- One transmitter heard by many receivers in one cycle: the TX effect and the
  claimed power are common to all of them, so they cancel.
- One TX to RX pair on several bands at once: both effects are common, so the
  **frequency dependence** of absorption is cleanly identified.
- One TX to RX pair over many hours: both effects are fixed, so the **diurnal
  variation** isolates the solar-zenith-angle dependence of the D region.

## What that makes unidentifiable, on purpose

Any quantity constant for a given station is absorbed into that station's effect
and cannot be recovered. This is not a defect of the method. It is an honest
statement about what WSPR contains.

| Quantity | Identified by | Identifiable |
|---|---|---|
| absorption magnitude | overall level, plus its frequency and zenith pattern | yes |
| atmospheric noise day/night **difference** | diurnal variation within a station | yes |
| atmospheric noise frequency **slopes** | cross-band within a station | yes |
| atmospheric noise **absolute** level | nothing, it is a constant | no, absorbed |
| receiver noise environment | a constant per receiver | no, absorbed |
| latitude terms of the noise model | a receiver's latitude never changes | no, absorbed |
| seasonal swing | needs more than one season in the corpus | not from one month |
| absolute antenna gain | a constant per station | no, absorbed |

A fit here can calibrate **how the signal varies** with frequency, path length,
zenith angle, hop count and layer. It cannot calibrate absolute levels, and
anything claiming to have done so from WSPR has fitted station population
statistics and called them physics.

The calibration report prints this table at the end of every run, so the reader
cannot miss it.

## Why the inner loop is cheap

Re-solving a spot costs a few hundred milliseconds and a fit needs thousands of
evaluations. Two **measured** facts make almost all of that unnecessary.

1. **Absorption is exactly linear in the collision frequency.** Doubling
   `NU_REF_PER_S` doubles the reported absorption to four significant figures,
   which the test `the_absorption_scale_is_linear` pins. In the D region
   `nu << omega`, so the absorption coefficient is proportional to
   `Ne nu / omega^2` and scaling either factor scales the whole line integral.
2. **The D-region and collision parameters do not move the ray.** The D region's
   plasma frequency is about 0.3 MHz, so at 7 MHz `X` is about 0.002 and there
   is no refraction to speak of.

Together those mean the expensive part (the ray) can be solved once and cached,
and the absorption level refitted underneath it analytically.

## What cannot be cached, and is scanned instead

The cached fit works only where those two facts hold. They do not hold for the
E-layer geometry, foE or foEs: moving those changes **which geometries close**,
so a path appears or disappears. That is a step change in the objective, not
something a least-squares gradient can follow.

Moving the D-region or collision profile does not change the ray, but it changes
the *shape* of absorption against frequency and zenith angle, and the shape is
exactly what the absorption scale cannot absorb.

So each of those is set to a few values across its plausible range, the corpus
is re-solved, and the absorption scale is refitted underneath each one. What
improves is then the shape rather than the level, which is the only thing a scan
of this kind can honestly claim.

## The corpus

`corpus.rs` holds a saved, reproducible WSPR corpus: positives, negatives, and
the per-day sunspot number each was observed under. Built once by `wspr_corpus`
and then reused, so every later run scores the same spots.

Handling rules that are enforced by test:

- Duplicates collapse to one observation.
- A distance mismatch between the claimed grids and the reported distance is
  dropped **and reported**, not silently kept.
- Malformed rows are reported, not skipped quietly.
- A station with fewer than `MIN_SPOTS_PER_STATION` spots cannot have its effect
  estimated and is handled explicitly.
- Both file formats round-trip.

**Negatives** are receivers that were listening on that band at that time and
did not decode the transmitter. Without them there is no false-positive rate,
only a fit to the cases that worked. `false_positive_rate_is_over_every_negative`
pins that the rate is computed over all of them.

## Sporadic E is excluded from the fit by default

`best_with_es_fallback` consults Es only where nothing deterministic closed, so
an Es spot records that the deterministic tracer failed to close a path which
demonstrably existed. The spot is a decode that really happened. Fitting physics
to the sheet's answer is fitting the fallback.

Measured on the corpus: Es spots were 41 percent of the solved spots at +21 dB,
and their presence **inverted** the fitted slope (0.69 to 0.59) while the same
fit without them left it alone (0.73 to 0.74).

They are still solved, still reported, and still scored. They are just not
fitted to. `--include-es` puts them back.

## The antenna assumption

The calibration default is **isotropic**, deliberately, and not the GUI's 10 m
dipole.

A station's absolute gain is constant for that station and is absorbed exactly
into its fixed effect, so a flat reference throws away nothing this corpus could
identify.

A dipole at a fixed height **in metres** is a different matter. It is 0.06
wavelengths up on 160 m and 0.94 on 10 m, so its gain at the 5 degree launch
angle a long path uses climbs 11.6 dB per end across that span, 23 dB across the
pair, and at 30 degrees the same tilt reverses sign. That is a band-shaped,
elevation-coupled term, so a per-station constant cannot absorb it and it lands
in the residual, where the only things able to chase it are the absorption scale
and the atmospheric noise slopes. Measured: they all run to their bounds doing
exactly that.

The flat reference is therefore not a simplification. It is the removal of an
assumption the data cannot see past. `--antenna dipole` puts it back and
reproduces the older runs.

## How the hold-out is separated

By **day**, and separately by **region**. A random row split would not be a
hold-out at all: adjacent spots share an ionosphere, so a model fitted on half
of a cycle predicts the other half of the same cycle for reasons that have
nothing to do with generalisation.

A day-level jackknife (`jackknife.rs`) refits with each day held out, to show
how much any single day is carrying the answer.

## What the calibration can measure about itself

The report includes, beyond the fit itself:

- **Bound profiles**: the objective as a function of each anchor across its
  whole range, so a flat direction is visible rather than inferred.
- **A local-minimum check**: whether the reported optimum is actually one.
- **Skill**: against a null model, so an improvement is stated relative to
  something.
- **Confound census**: which cuts of the data are thin enough that their medians
  are their own noise. Cells thinner than 30 spots print no error at all.
- **Layer races**: where F2, E and Es compete for the same spot.
- **Terminator step, absorption range, hop geometry**: the specific shapes the
  fit is or is not reproducing.
- **Decode probability and false-positive rate**, over the negatives.
- **The identification table** above.

## Results worth carrying forward

Two conclusions from past runs are recorded so they are not rediscovered at
cost:

- **The daytime residual term is worth about 0 dB RMS.** The model is
  spread-limited, not bias-limited. Adding a daytime correction does not help
  because there is no systematic daytime bias left to remove.
- **The Es sheet is a near-perfect mirror.** The derived tunnelling loss is about
  zero. The apparent Es bias was a selection bug plus the foEs value, never a
  missing loss term.

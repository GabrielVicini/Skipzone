# Accuracy

The ray tracer is verified against closed-form solutions.
The modelled ionosphere matches ionosonde measurements with nothing fitted.
A single predicted SNR is good to roughly 8 dB, on a calibration that predates the current foF2 model.
Antennas and several D-region inputs have no validation at all.

Every number here comes from the [verification report](https://github.com/GabrielVicini/Skipzone/blob/master/docs/reports/2026-08-verification.md) dated 2026-08-10, run at commit `d92bf9f`.
The code has changed since then, so these numbers may lag it.

The report checks three things, and the evidence gets weaker each time.
They are whether the tracer solves its equations, whether the modelled layers match the real sky, and whether predicted SNR matches what stations received.

## Engine: verified

273 tests passed on the report's configuration, including 21 integration tests against independent answers.
Those tests compare the tracer with closed-form ray paths and cross-check one numerical method against another.
With the magnetic field set to zero, O and X give bit-identical results, as the physics requires.
The integrator's convergence order is measured, and it comes out at 5.
Whether the modelled sky resembles the real one is the next section's question.

## Ionosphere: validated against measurement

The comparison uses GIRO ionosonde data from 17 stations over four seasonal windows (2025-10, 2026-01, 2026-04 and 2026-07).
Nothing is fitted to it.

**foE** is the best-characterised quantity in the model: bias -0.00 MHz, RMS 0.20 MHz.
It matters because the E layer decides which low bands are screened off by day.

**foF2**, from the CCIR maps, has bias +0.03 MHz, RMS 1.10 MHz and correlation 0.894.
It also compresses: the model's spread is 2.16 MHz against a measured 2.45 MHz.
In practice it reads low on high days and high on low days, and no level adjustment can fix that.

Two further gaps apply to foF2:

- The CCIR maps expect a twelve-month smoothed sunspot number. The app holds a single daily value instead, and that makes foF2 read about 0.39 MHz low.
- All four windows sit at one point in the solar cycle (R12 92 to 110). Behaviour at solar minimum or maximum is untested.

## End to end: on a stale baseline

The last full WSPR calibration ran against the previous foF2 model.
On 2026-08-09 the CCIR maps became the default, which changes which paths exist.
The calibration has not been re-run since, so the figures below describe a configuration the app no longer ships.

| Measure (station effects removed) | Fit | Hold-out month |
|---|---|---|
| RMS error | 7.90 dB | 8.00 dB |
| Median error | +0.31 dB | +0.19 dB |
| Slope against measured | 0.54 | 0.61 |
| R² | 0.43 | 0.53 |
| Model found a path | 91 % | 90 % |
| AUC, decoded vs not decoded | 0.801 | - |

The fit transfers to a month it never saw, with only 0.09 dB more RMS error.
The slope below 1 means predictions compress: strong paths read weak and weak paths read strong.
Whether part of it comes from the foF2 compression above is untested.

False positives are the weakest point.
The false-positive rate is at most 34.7 % overall, and 42.8 % by day and 80.4 % by night after the fit.
The model calls a path workable after dark far too often.

The errors in the table are measured with each station's offset removed.
The hold-out median error is then +0.19 dB, so what remains is mostly scatter.
Separately, the calibration measured a global offset of 5.1 dB, and the app subtracts it from every SNR by default.
It was measured on 2026-07-29, before the foF2 change, so it is stale too.

## Not validated

**Antennas.** The patterns use image theory over flat, lossy ground.
There is no conductor or radial loss, no near-field ground loss and no terrain.
No test or reference data checks them.

**Absorption against sun angle.** The D region makes absorption follow the solar zenith angle with an exponent of 1.39.
Published riometer work suggests about 0.75.
The WSPR corpus can't tell the two apart: dropping the exponent from 1.39 to 0.50 moves the median spot by 0.02 dB.
It matters most on 160 m and 80 m, and no spot above 30 m moved by more than 1 dB.
Resolving it needs riometer data or oblique ionograms.

**D-region inputs.** The peak density, its height, its scale height and the collision frequency are textbook order-of-magnitude values.
None is fitted or traced to a single citable table.

**hmF2.** The F2 peak height is a setting with a default of 300 km. The model doesn't derive it from data.

## Limits of the test data

The WSPR corpus has gaps that cap what any fit can conclude:

- One season: all of it is northern summer, within seven weeks.
- One region: European and North American mid-latitude receivers only.
- Mostly short paths: 80 % under 2000 km, and only 102 spots beyond 8000 km.
- Moderate sun only: sunspot number 83 to 126, with no flares or storms.

WSPR also can't measure anything that is constant per station.
Absolute antenna gain, the claimed transmit power and the absolute noise level all fall in that group.
So the calibration tunes shapes and trends, and leaves those absolute levels unmeasured.

## What this means for you

Use Skipzone for "which band, roughly when, and is this path plausible".
Compare paths with each other more than you trust any one absolute SNR.
Be sceptical of night-time openings.
Don't use it where you need a calibrated absolute signal level.

<!-- OWNER: what you plan to re-run or fix next, and when you expect updated numbers -->

## Checking it yourself

The report's commands reproduce every number on this page.
The engine and ionosphere checks run with:

```bash
cargo test --workspace --all-targets
cargo run --release -p skipzone-lab --bin iono_check
```

Only `wspr_corpus` and `wspr_live_check` go online, to download spots from wspr.live.
`iono_check` reads GIRO ionosonde and SILSO sunspot files that you fetch by hand.
[Command Line Tools](Command-Line-Tools#the-corpus-directory) says how, and covers the WSPR harnesses.

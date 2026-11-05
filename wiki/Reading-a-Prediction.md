# Reading a Prediction

A prediction answers three questions in turn.
Does any ray geometry reach the receiver? Is the signal strong enough to decode?
Is the layer carrying it reliably there?
The verdict line at the top of the **Point-to-point trace** panel combines all three.

<!-- SCREENSHOT: the top of the trace panel, showing the verdict chip and the F2 / E / Es lines -->

## The verdict line

| Verdict | Colour | What it means |
|---|---|---|
| **USABLE** | green | A path closed and its SNR clears your threshold. |
| **PATH FOUND, BELOW THRESHOLD** | amber | A path closed, but the signal is too weak for your mode. The line says how many dB short. |
| **SPORADIC E ONLY** | amber | No F2 or E path reaches this range. A sporadic E sheet would, and the line gives how often that sheet is present. |
| **NO PATH** | red | Nothing reaches the receiver. The line says whether you are in the skip zone or above the MUF. |
| **NO VERDICT** | red | The tracer failed numerically. This says nothing about the real path; see **Engine errors**. |

USABLE and BELOW THRESHOLD also carry a word: *very likely*, *likely*, *marginal*, *unlikely* or *very unlikely*.
It is the chance the path decodes, allowing for the model's typical error on a single path ([Accuracy](Accuracy)).
A path 1 dB over threshold is close to a coin flip, so read the word alongside the verdict.

## The layer lines

Under the verdict there is one line per layer: **F2**, **E** and **Es**.
A layer that found a path shows its own SNR, margin and hop count.
A layer that found nothing shows why:

| Status | Meaning |
|---|---|
| path found | At least one ray reached the receiver through this layer. |
| no bracketing elevation (skip zone or out of range) | The layer reflects, but no launch angle lands a ray at this distance. |
| frequency above this layer's MUF at every elevation | Rays pass straight through this layer at every angle. |
| tracer failed (numerical, not a physical verdict) | The calculation broke down. The model has no answer here. |
| not attempted | Sporadic E was switched off, or too unlikely to be worth solving. |

## SNR and the threshold

SNR is the received signal power over the noise power in your receiver bandwidth, in dB.
The noise comes from three sources at the receiver: atmospheric, man-made and galactic.
**Great-circle reference** shows that noise floor even when no path exists.

Your **Mode** sets the threshold and the bandwidth it applies in:

- CW: 3 dB in 500 Hz.
- SSB voice: 10 dB in 2400 Hz.
- RTTY (45 baud): 6 dB in 300 Hz.
- FT8: -4 dB in 50 Hz.

You can change both in **Settings**.

**Selected mode - totals** breaks the SNR down: free-space spreading, ionospheric absorption, ground reflection loss, antenna gain at each end, and each noise term.
The atmospheric noise term is an approximation of my own, and its absolute level is indicative only.

## MUF

The maximum usable frequency (MUF) is the highest frequency that a layer still reflects back to Earth over your path.
Above it, the ray escapes into space.
The trace panel does not print a single MUF figure.
To find it, run **Point-to-Point > Best FREQ**: the band strip turns red ("no path") above the MUF.

<!-- SCREENSHOT: the Best FREQ band strip, with usable, below-threshold and no-path regions visible -->

## Skip zone

The skip zone is the ring around the transmitter where no sky wave lands.
Rays at steep angles pass through the layer, and rays at shallow angles come down further out.
Inside the skip zone a lower frequency, or a different layer, may still work.
The app reports this as "no bracketing elevation" for that layer.

## Above the MUF, or tracer failed

"Frequency above this layer's MUF at every elevation" means the ionosphere won't return this frequency on this path.
"Tracer failed (numerical, not a physical verdict)" means the calculation broke down, so the model has no answer.
Failures are listed under **Engine errors**; please report them.

## O and X modes

The Earth's magnetic field splits every HF ray into two: the ordinary (O) and extraordinary (X) modes.
They take slightly different routes and suffer different absorption ([How It Works](How-It-Works#the-magnetic-field-splits-each-ray) explains why).
**Modes found** lists each one separately, for example "O-mode, 1 hop(s), 2300 km group, SNR 12.3 dB".
If you untick **Use IGRF-14** in **Settings > Magnetic field**, the field is zero and the two modes become identical.

## Sporadic E (Es)

Sporadic E is a thin, patchy sheet of ionisation near 100 km.
It comes and goes ([How It Works](How-It-Works#sporadic-e) says when).
Es is only there some of the time, so the app uses an Es path only when no F2 or E path exists.
An Es result shows "% occurrence" beside its SNR: the climatological chance that the sheet is present.
On an area coverage map, Es-only tiles are drawn faded by that probability.

**Settings > Sporadic E** lets you turn Es off or set your own foEs and occurrence.

## When nothing connects

If no ray lands, **Closest near-misses (elevation sweep)** shows the closest landings for each hop count.
It shows how close each hop count came to the receiver.
If every layer reports its MUF exceeded, try a lower frequency.
**Assumed values (all of them)** lists every input the model used, including foF2 at the path midpoint.

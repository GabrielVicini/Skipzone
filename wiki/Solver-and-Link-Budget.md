# Solver and Link Budget

`app/src/solve/`. Drives the engine's homing and tracer to produce every mode
that connects, plus full per-hop geometry for drawing, plus a near-miss report
when nothing connects. It calls the engine's public API only and implements no
physics of its own.

| File | Contents |
|---|---|
| `mod.rs` | The top-level `solve` driver and the three `best_*` accessors |
| `types.rs` | Result structs the UI renders |
| `tracing.rs` | Per-hop tracing and homing helpers |
| `link_budget.rs` | Free-space spreading and Fresnel ground reflection |

## Multi-hop handling, and what it costs

The engine's homing solves a **single hop**. For an N-hop path the solver homes
one hop of `1/N` of the great-circle arc, which is exact when the medium is
height-only, then actually propagates N hops by specular ground reflection and
reports where the ray really lands.

With a magnetic field the medium is not spherically symmetric, so the terminal
miss is a genuine diagnostic of the equal-hop assumption rather than something
to hide. It is reported, not swallowed.

## The candidate enumeration

Every (hop count, elevation bracket) pair is one independent candidate ray with
its own terminal homing search and its own propagation, sharing nothing but the
read-only models. They are enumerated first and then run across the compute
pool, and results are folded back **in order**, so the solution list and the
error list come out the same whatever order the threads finish in.

A hop count whose per-hop arc is over the horizon for every layer in the model
cannot produce a ray, so it is skipped before tracing. That skip is recorded
distinctly from "found no bracket", because the two mean different things.

## Layer attribution

`LayerMode` is `F2`, `E` or `Es`, and it is attributed from the **apex altitude
the engine reports**, not assumed from the launch angle. A solution is filed
under the layer it actually turned in.

`E_ATTRIBUTION_TOP_KM` is the boundary between E and F2 attribution.

## Why a layer produced nothing: `LayerStatus`

Distinguishing these is the point. `NoBracket` used to be swallowed and rendered
identically to a genuine "nothing arrives", so a map cell inside the F2 skip
zone looked the same as one beyond every possible mode.

| Status | Meaning |
|---|---|
| `Solved` | At least one geometry closed through this layer. |
| `NoBracket` | The layer exists and rays reflect from it, but no launch elevation puts one at the target range. The target is inside this layer's skip zone or beyond its maximum range. A different layer may still reach it. |
| `Penetrates` | Rays reflect from nothing here at any elevation. The frequency is above this layer's MUF for every geometry. |
| `Failed` | The tracer failed on this stack. A numerical outcome, **never** to be shown as a physical one: the model could not answer, which is not the same as nothing arriving. |
| `NotAttempted` | For Es, "disabled, or too unlikely to be worth the second solve". The probability says which. |

## Two stacks, three verdicts

The solver builds **two** model stacks and keeps their results in two separate
lists: `solutions` (deterministic, F2 and E) and `es_solutions` (sporadic E).
Three accessors reduce them, and which one a caller uses is a correctness
decision.

### `best_by_snr`

The strongest SNR among the deterministic solutions, or `None`. Shared by every
caller that has to reduce a whole solve to one number (the frequency sweep and
the coverage grid) so the two can never disagree about which mode a scenario is
being judged by. Sporadic E is deliberately not considered.

### `best_es`

The strongest Es-supported path, if any. Separate because it comes with a
probability attached and must not be compared with a deterministic path as
though it were one.

### `best_with_es_fallback`

The best deterministic path if one closed, and only otherwise the best
Es-supported one.

This is not "the strongest SNR of the two lists", and the reason is the most
important single piece of design in the solver. It used to be, and that was a
selection bug rather than a preference.

An Es reflection at 100 km has a shorter ray path than the F2 alternative, less
spreading loss, and a shorter slant transit of the absorbing D region. On raw
SNR it therefore wins **by construction** wherever it is geometrically possible,
not because the ionosphere favoured it. Ordering the two lists together by SNR
does not compare two hypotheses; it just prefers the lower layer, and it does so
while silently discarding the one thing that distinguishes them, namely that F2
is there every day and Es is there only some fraction of the time.

Folding the probability into the SNR is not the fix either. That would put a
likelihood into a quantity measured in dB, which is the exact false equivalence
the two lists exist to prevent.

So the rule is **ordinal**: a path that is simply there outranks a path that
might be there, and Es is consulted only when nothing deterministic closed at
all. That is also the case Es was added for, a 17 m signal at 400 km where F2
genuinely has no solution, so the fallback keeps the capability it was built for
while losing its ability to outbid a perfectly good F2 path.

Callers must still carry the winner's `probability`. An Es answer returned here
is a "maybe", and reporting it as an opening without its occurrence figure is
the same conflation one step further down.

## The three-state verdict

`PathState` replaced an older `connects` boolean, because "a path closes
geometrically" and "a path anyone can hear" are different claims.

| State | Meaning |
|---|---|
| `Usable` | A path closed and its SNR clears the decode threshold. |
| `BelowThreshold` | A path closed but nobody would hear it. |
| `NoPath` | No geometry closed. |

`ModeReport` always reports the continuous SNR whether or not it clears the
threshold; the state applies the threshold on top.

## Link budget

`link_budget.rs`. Pure functions, no engine calls, no ionospheric physics.

**Free-space spreading**, the standard Friis form:

```
L_fs = 32.44 + 20 log10(f_MHz) + 20 log10(d_km)
```

The distance is the **total ray arc length**, the physical path the energy
travels, not the great-circle range.

**Ground reflection**, from the Fresnel power reflection coefficient of a lossy
dielectric half-space. The complex relative permittivity is
`eps_r - j sigma/(omega eps0)` in the ITU-R P.527 form, and

```
R_h = (sin g - w) / (sin g + w)
R_v = (eps_c sin g - w) / (eps_c sin g + w)
w   = sqrt(eps_c - cos^2 g)
```

with `g` the grazing angle. A sky wave is elliptically polarised after its
ionospheric reflection, so the average power coefficient `(|R_h|^2 + |R_v|^2)/2`
is used and the loss is `-10 log10` of it.

`fresnel_coefficient` is shared with the antenna models, which need the same two
numbers for their image-theory patterns. One implementation, so the two cannot
drift.

**Ground type is per hop.** Each bounce is classified from its own position by
`coastline`, so a path that leaves land, crosses ocean and lands on land again
is charged three different reflection losses. This is pinned by the test
`auto_detect_classifies_each_bounce_from_its_own_position`.

**Absorption** comes out of the engine in nepers and is converted with
`NEPERS_TO_DB = 20/ln(10)`, the field-amplitude convention.

## When nothing connects

A near-miss sweep runs and reports the smallest terminal miss found, in km,
along with the elevation scan bounds it searched. The UI shows this instead of
a bare "no path", because "you missed by 90 km at 1 hop" and "nothing reflects
at this frequency at all" are different problems for the operator.

`above_muf_explains_itself_rather_than_going_silent` is the test that pins this
behaviour.

## Step tuning

`tracing.rs` carries a `StepTuning` type with three presets: `for_scan`,
`for_search` and `for_thin_sheet`. The thin-sheet variant exists because Es is
about 1.5 km semi-thickness and an integrator step sized for the F2 layer will
step straight over it.

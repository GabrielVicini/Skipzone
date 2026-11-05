# Ionosphere Model

The engine has no ionosphere. It has an `ElectronDensity` trait. This page
describes what the application hands it, and, just as importantly, how much of
that is measured and how much is an anchor.

**Read the provenance sections before quoting any magnitude from this model.**
The project's position is that the shapes and trends are defensible and the
absolute levels frequently are not, and the code says so in the module
documentation, in the UI, and here.

## The layer stack

| Layer | Height | Profile | Slant factor | Peak density from |
|---|---|---|---|---|
| D | ~70 km | Chapman | `Ch(X, chi)` at the local zenith angle | constant overhead anchor |
| E | ~110 km | Chapman | `Ch(X, chi)` at the local zenith angle | constant overhead anchor (foE) |
| Es | ~100 km | Quasi-parabolic, thin | none | probabilistic foEs |
| F2 | ~300 km | Chapman | overhead (none) | **CCIR numerical maps** |

`crates/skipzone-iono/src/chapman.rs` provides one reusable layer type, `SolarChapmanLayer`, that
D, E and F2 are all built from, plus the grazing-incidence function itself.

## Two horizontal knobs, deliberately separate

### The ionising-flux slant factor

For a layer in photochemical equilibrium this is `Ch(X, chi)` evaluated at the
**local** solar zenith angle of each sampled point.

This replaces the engine's `ChapmanLayer::with_zenith_angle`, and the reason
matters. The engine's plane-parallel form has a hard limit at 85 degrees zenith
angle, past which `sec(chi)` diverges and the layer has to be switched off. That
produced a discontinuity exactly at the terminator, which is precisely where HF
propagation is most interesting. `Ch(X, chi)` is finite through and past 90
degrees, so absorption and E-layer ionisation fade smoothly instead of
switching off at a plane, and a path crossing the terminator is ionised only on
its sunlit part.

The 85 degree omit logic must not come back. If a change makes it look
necessary, the bug is elsewhere.

Implementation notes, all derived in `docs/derivations/chapman-grazing.md`:

- `Ch(X, chi)` is computed through the scaled complementary error function
  `erfcx`, evaluated by the Abramowitz and Stegun 7.1.14 continued fraction at
  48 levels, with continuant rescaling above 1e250 so a large argument cannot
  overflow. The forward and backward recurrences are checked against each other
  by test.
- The horizontal gradients are required, not optional. Omitting them lets the
  Hamiltonian drift along the ray, which the invariant tests catch.
- There is a night guard, and deep night is exactly vacuum.
- The constant-source case is bit-identical to the older D-region
  implementation, which is pinned by test.
- References: Smith and Smith 1972 for the function, A&S 7.1.14 for the
  continued fraction.

### The absorption zenith law is DERIVED, and it is the model's largest known error

The layer imposes a solar-zenith law on absorption that nothing chooses:

```text
A(chi) ~ cos^n chi,    n = 1/2 + H_D / H_nu
```

Two contributions, neither of them a free parameter:

- the realised peak is `Nm Ch^{-1/2}`, worth exactly **1/2**;
- the **whole profile translates upward** by `H_D ln Ch` - wings included - and
  the collision frequency falls on `H_nu`, so the `Ne nu` product loses a
  further `Ch^{-H_D/H_nu}`.

Away from the terminator `Ch -> sec chi`, and the two combine into the formula
above. `chapman.rs`'s
`the_zenith_exponent_is_one_half_plus_the_scale_height_ratio` pins it against
the real layer and the real collision model, across the whole box the
calibration is allowed to move both scale heights in.

With the shipping anchors (`H_D = 6.0 km`, `H_nu = 6.7 km`) that is
**n = 1.39**. Riometer and A1 absorption measurements report **n ~ 0.75**, and
the band-fixed WSPR cells want 0.2-0.6. The model is roughly **twice as steep in
the exponent as reality**, and that single defect orders the whole unexplained
residual block by sun angle rather than by frequency.

#### Why no calibration can fix it

This is the important part, and `bin/zenith_probe` measures every line of it:

| what was tried | reachable `n` |
|---|---|
| both scale heights across their full bounds | 0.944 to 2.500 |
| adding a chi-independent component (the night floor) | needs 0.42, vs a literature night/day ratio of ~0.10 |
| all three anchors at their extreme corner at once | 0.787 |

The target 0.75 sits **outside** the calibration's own three-parameter box. So
a run that drives `d_scale_height_km`, `nu_scale_height_km` and
`d_night_floor_fraction` to their bounds is not a stuck optimiser - it is the
bound-reached finding described in [Calibration](Calibration), reporting
correctly that the residual is not produced by any of them.

There is also a hard floor. With `H_nu -> infinity` the rise term vanishes and
`n -> 1/2`. **An alpha-Chapman D region cannot produce `n < 1/2` under any
anchors at all**, because `Nm Ch^{-1/2}` alone is worth exactly one half. Part
of what the WSPR cells ask for is below what the functional form admits.

#### The knob that does reach it: `d_rise_fraction`

The shape had to change, not a constant. The rise term assumes the entire
profile is in photochemical equilibrium and translates with the ionisation peak.
The real lower D region - 60 to 75 km, where `nu` is largest and where the
absorption kernel actually lives - recombines fast enough that `Ne` tracks
production at roughly **fixed height**, which is the `n = 1/2` law with no rise
term at all.

So the layer now takes a **rise fraction** `f`, and the law generalises to

```text
A(chi) ~ cos^n chi,    n = 1/2 + f H_D / H_nu
```

Writing `u = z - f ln Ch`, the layer is `Ne = Nm Ch^{-1/2} exp(1/2 (1 - u -
e^{-u}))`. Both endpoints are derived rather than chosen:

| `f` | what it is |
|---|---|
| 1 | the textbook alpha-Chapman layer - the `Ch^{-1/2}` prefactor and the `+ln Ch` inside `u` cancel exactly, so this is the old form back again |
| 0 | amplitude law only, `Nm Ch^{-1/2}` at fixed height, `n = 1/2` |

Measured against the derived line across `f`, both scale heights and 0-70
degrees, the worst disagreement is **0.0094**. The literature `n = 0.75` lands
at `f = 0.285`, against the closed form `(0.75 - 1/2) H_nu/H_D = 0.279`.

**The shipping default is still `f = 1`.** `solve_digest` is byte-identical to
its pre-change baseline, so every earlier run reproduces exactly. The value that
matches the literature exponent has *not* been adopted, because it has not been
fitted against the corpus - and the corpus that would judge it is one northern
summer of Europe/North America paths (see [Calibration](Calibration)). Moving a
shape anchor on the strength of a single literature number is precisely the
quiet recalibration this project refuses elsewhere.

Implementation notes:

- `f = 1` keeps its **own branch** with the original arithmetic, so the default
  layer stays bit-identical rather than merely close.
  `partial_rise_reduces_to_the_textbook_layer_at_f_one` forces the general path
  at `f = 1` and checks the two agree to better than 1e-12.
- The analytic `chi` gradient is the product rule over both places `Ch` appears
  - the prefactor and the shift inside `u` - giving
  `dNe/dchi = Ne (Ch'/Ch) (-1/2) [1 + f(e^{-u} - 1)]`. It is checked against
  central differences across `f` and on both slant branches, because a gradient
  that disagrees with its density does not produce a wrong number here, it
  drives Hamiltonian drift far away in the solver.
- Enabling `f < 1` costs about **+14 % on the D layer's own density sample**
  (one `ln`, one `sqrt`). That is one layer of four, and IGRF is ~72 % of a
  solve, so scale before quoting. The default path pays nothing: `full_rise` is
  a whole-solve constant and the branch predicts perfectly.

### The overhead-sun peak density

Queried at each sampled point's horizontal position. This is what lets F2 follow
a map instead of being one scalar for the whole domain.

## Why F2 gets no zenith law

D and E really are close to alpha-Chapman photochemical equilibrium, so their
day/night behaviour is *derived* from `Ch(X, chi)`.

F2 is not. It is transport-dominated, which is exactly why it survives the night
and why the winter anomaly exists, and it is why every operational prediction
system (CCIR, URSI, IRI) takes foF2 from an empirical map rather than a
zenith-angle law. Feeding F2 both a map and a zenith law would double-count the
diurnal variation. So F2 uses the overhead slant factor and gets all of its
horizontal structure from the map.

## foF2: three backends, all surfaced

`crates/skipzone-iono/src/fof2.rs` and `crates/skipzone-iono/src/ccir.rs`.

- `Fof2Backend::Ccir` — **the default.** The CCIR numerical maps, real measured
  coefficients. See the next section.
- `Fof2Backend::ConstantSsn` uses `fof2_from_ssn` everywhere. NmF2 is taken
  linear in sunspot number, calibrated to representative midlatitude anchors of
  about 4.5 MHz at SSN 0 and about 10 MHz at SSN 150. There is deliberately no
  hidden day/night branch: SSN is the only driver, so an operator who selects
  this backend sees exactly the foF2 the engine is given.
- `Fof2Backend::Gridded` uses the bundled climatology grid, sampled at each
  point's own latitude and local solar time, interpolated with Catmull-Rom.

Whichever ran is written into `Assumptions::fof2_source` verbatim, **including**
when the bundled data failed to load and the constant took over.

## The CCIR maps: what replaced the guess, and by how much

`crates/skipzone-iono/src/ccir.rs`, reading `crates/skipzone-iono/data/nequick/ccir11.txt` .. `ccir22.txt` and
`crates/skipzone-iono/data/nequick/modipNeQG_wrapped.txt`.

This wiki used to say that the bundled grid's layout followed the operational
maps and that "dropping a real IRI or CCIR table into the same file layout is a
file replacement, not a code change". **That was wrong, and the data was already
here.** The CCIR maps are not a latitude-by-local-time table. They are a
six-harmonic Fourier series in *universal time* over a Legendre-like expansion in
**modified dip latitude**, and nothing in the app knew what modip was. What was
needed was the evaluator, not the file.

```text
A[j][k] = F2[0][j][k] (1 - R12/100) + F2[1][j][k] (R12/100)
CF2[j]  = A[j][0] + sum_{i=1..6} ( A[j][2i-1] sin(i T) + A[j][2i] cos(i T) ),  T = 15 UT - 180 deg
foF2    = sum_k CF2[k] sin(modip)^k
        + sum_{m=1..8} sum_k CF2[..] sin(modip)^k cos(lat)^m { cos(m lon), sin(m lon) }
```

The term counts per longitude order are `[12, 12, 9, 5, 2, 1, 1, 1, 1]`, which is
where `12 + 2*32 = 76` comes from; M(3000)F2 uses `[7, 8, 6, 3, 2, 1, 1]` for 49.

### What it is worth, measured

`bin/iono_check`, **70 219 GIRO ionosonde soundings**, 17 stations, four seasonal
windows (2025-10, 2026-01, 2026-04, 2026-07). Nothing fitted, nothing absorbed.

| backend | bias | RMS | median abs | SD of model | correlation |
|---|---|---|---|---|---|
| bundled grid | +0.07 | 1.70 | 1.02 | 1.91 | 0.723 |
| climatology | +0.07 | 1.70 | 1.02 | 1.91 | 0.723 |
| CCIR, daily SSN v2 | +0.31 | 1.20 | 0.65 | 2.44 | 0.888 |
| CCIR, R12 v2 | +0.92 | 1.50 | 1.01 | 2.52 | 0.886 |
| **CCIR, R12 v1** | **+0.03** | **1.10** | 0.71 | 2.16 | **0.894** |
| CCIR, daily SSN v1 | -0.39 | 1.13 | 0.67 | 2.08 | 0.903 |

Measured foF2 has SD 2.45 MHz and mean 6.34 MHz. So:

- **RMS 1.70 → 1.10 MHz, a 35 % reduction.** The handoff called foF2's 1.70 MHz
  "a SHAPE limit, not a constant", and said retuning the five diurnal constants
  buys 3.6 %. That was right, and this is the shape.
- **The compression is gone.** The old dispersion ratio was 1.91/2.45 = 0.78 -
  the model swung less than the ionosphere. CCIR is 0.88 to 1.00 depending on
  driver.
- **Correlation 0.723 → 0.894.**
- Every local-time bin, every latitude band and every season improves. The
  biggest single win is southern-hemisphere winter: RMS 2.41 → 0.87, where the
  climatology's winter-anomaly term read +1.99 MHz high. A single-season corpus
  could never have shown that.
- 15 of the 16 quotable stations improve; one (MHJ45) worsens, 1.24 → 1.33.

### The sunspot scale is a unit, and it was wrong

SILSO rescaled the international sunspot number by about **1.44** in July 2015.
The CCIR maps were fitted in 1966, so their `R12 = 100` page means the *old*
scale's hundred. Every sunspot number this project handles - `spaceweather`'s
SIDC and NOAA feeds, `corpus/ssn_daily.tsv` - is the new one.

Feeding the new number straight in leaves a **+0.92 MHz** bias. Dividing by 1.44
leaves **+0.03 MHz**. Nothing was fitted to produce that: the factor is SILSO's
published recalibration and the bias fell out of it. `ccir::SSN_V2_OVER_V1`.

### The one known gap, stated rather than corrected

The maps want **R12**, a twelve-month mean. The app only ever holds an observed
daily or monthly number - NOAA reports `smoothed_ssn` as `-1` for any month less
than six months old, so a running app cannot have R12 for "now".

foF2 is concave in the driver, so averaging a wide daily spread through it reads
low: measured, **-0.39 MHz** of bias against R12's +0.03. That is a driver
mismatch with a known mechanism, and it is left in the open rather than papered
over with an output scale. Feeding a smoothed sunspot number moves it; an
operator who has one should enter it.

### Modified dip latitude

`tan(modip) = I / sqrt(cos lat)`, `I` the magnetic dip at 300 km. The shipped
grid is IGRF epoch ~2001, and it is read rather than recomputed from the IGRF
this project already carries: the coefficients were fitted in the modip of their
own era, so recomputing the coordinate under a modern field would evaluate the
map away from the coordinate it was fitted in.

`tests::grid_modip_agrees_with_igrf_dip` checks the shipped grid against this
project's own IGRF at epoch 2001 anyway - RMS under 2 degrees, worst under 6 -
which is the test that would catch a transposed or misaligned lattice.

The table is read two ways, deliberately:

- `ModipGrid::at` is the four-point Lagrange form the maps are specified to be
  read with, and is what any comparison against another implementation should
  use;
- `ModipGrid::sample_c1` is a Catmull-Rom tensor product, used by the tracer,
  because the Lagrange form's *derivative* jumps at every cell edge and the ray
  equations integrate straight through those jumps. Same reason `Fof2Plane` is
  bicubic rather than bilinear.

They agree at every node exactly. Between nodes the worst disagreement in modip
is 0.57 degrees, next to a pole where modip runs 14 degrees in one grid row - and
that is the wrong quantity to bound, because foF2 sees modip only through
`sin(modip)`, whose derivative vanishes exactly there. The disagreement that
reaches the tracer is under **0.05 MHz** anywhere on the globe, pinned by test.

### Cost

The F2 peak sample went from 342 ns to 482 ns per call, **+41 %**, measured by
`bin/profile_solve`. Against a field sample of about 950 ns that is roughly
**+10 % on one RHS evaluation** of the ray equations. The IGRF share of a solve
drops from about 75 % to about 65 % because the denominator grew, not because
IGRF got cheaper.

### Provenance of the bundled grid, read this

**This backend is no longer the default** - `Fof2Backend::Ccir` is. Everything
below still applies to it, and it remains selectable.


`crates/skipzone-iono/src/assets/fof2_grid.tsv` is laid out the way the operational maps are: a
latitude by local-solar-time table per season, tabulated at two solar-activity
levels and interpolated in NmF2 between them, which is the CCIR/URSI idiom.

**The numbers in it are not CCIR, URSI or IRI data.** Those coefficient sets are
not in this repository and are not fetched at runtime. The table is generated by
`app/src/bin/gen_fof2_grid.rs` from `climatology_fof2`, which is an
order-of-magnitude climatology of the author's own construction. Its *shape*
follows the published behaviour of foF2 (afternoon diurnal maximum, equatorial
anomaly crests either side of the magnetic equator, high-latitude trough, winter
anomaly in daytime midlatitudes). Its *absolute anchors* are calibrated only to
`fof2_from_ssn`, the same coarse midlatitude anchor the scalar model used.

Treat foF2 magnitudes as indicative. The defensible part is that foF2 now varies
with latitude, local time and season, so the terminator, the equatorial anomaly
and the day/night asymmetry exist at all. Dropping a real IRI or CCIR table into
the same file layout is a file replacement, not a code change.

Do not confuse this grid with `crates/skipzone-iono/data/nequick/`, which is real CCIR data, vendored
and licensed but **not read by any code**. See
[Data and Assets](Data-and-Assets).

### What the ionosonde comparison measured

`iono_check` scores this against measured GIRO ionosonde characteristics. Two
results are worth carrying forward:

- foE was measurably wrong and was fixed as a result.
- foF2's shape error has a floor around 1.70 MHz that the synthetic
  climatology's functional form cannot get below. That is a limit of the shape,
  not of the anchors, so no amount of retuning constants closes it. **Replacing
  the shape does**: the CCIR maps reach 1.10 MHz on the same soundings. That is
  what the section above measures, and it is why the default moved.

All four seasons are mandatory when running this comparison. A single season
confounds the seasonal term with the solar-cycle position.

## Sporadic E

`crates/skipzone-iono/src/sporadic_e.rs`. Es is the reason a 17 m signal is heard at 400 km when
F2 has no solution at that geometry at all. It differs from every other layer in
two ways that shape the whole module.

**It is not photochemical.** Es is metallic ions concentrated into a sheet by
wind shear, not a production and recombination balance with solar flux. So it
gets no Chapman profile and no `Ch(X, chi)`. The shape is a thin
`QuasiParabolicLayer`, which is also the engine's closed-form validation target.

**It is probabilistic.** Es either is or is not there. Reporting a path it
supports as simply "available", next to an F2 path that is available every day,
would be a false equivalence. So the module produces an **occurrence
probability** alongside foEs, and the solver keeps Es-supported paths in a
separate verdict carrying that probability. They are never folded into the
deterministic yes/no. See
[Solver and Link Budget](Solver-and-Link-Budget#two-stacks-three-verdicts).

Provenance: order-of-magnitude anchors, not a fitted model and not traceable to
a citable table. The shape follows midlatitude Es climatology (strong summer
maximum, diurnal cycle with mid-morning and early-evening peaks, temperate
latitude maximum) and the magnitudes sit in published ranges (midlatitude summer
daytime occurrence a few tens of percent, winter night a few percent, foEs
typically 3 to 10 MHz when present). The trends are the defensible part.

Deliberately not modelled: equatorial Es, which is a different mechanism
entirely (the equatorial electrojet) with much higher occurrence; auroral Es;
and any spatial patchiness. A real Es cloud is tens to a few hundred km across
and this models a uniform blanket, which is another reason the occurrence
probability is reported rather than a yes/no.

One finding is worth recording because it cost real effort to establish: the Es
sheet is a near-perfect mirror. The derived tunnelling loss through it is
approximately zero. An apparent Es bias in early results was a selection bug
plus the foEs value, never a missing loss term. Do not go looking for one again.

## The noise floor

`app/src/noise.rs` answers the question the ray tracer cannot: given that a path
geometrically closes, would anyone actually hear it. It touches no ray tracing,
absorption or ground reflection maths. It consumes the total system loss those
produce and adds a transmitter power and a noise floor.

Verified against Recommendation ITU-R P.372-9 (Radio noise, 08/2007), read
directly rather than recalled:

- Eq. (2), the external noise factor `fa = pn / (k t0 b)`, with `t0` taken as
  290 K, and `Fa = 10 log10(fa)` dB.
- Eq. (6), available noise power `Pn = Fa + B - 204` dBW, where
  `B = 10 log10(b)` and `-204 = 10 log10(k t0)`.
- Eq. (11) and Table 1, man-made noise median `Fam = c - d log10(f)` with f in
  MHz, valid 0.3 to 250 MHz, with (c, d) of City (76.8, 27.7), Residential
  (72.5, 27.7), Rural (67.2, 27.7), Quiet rural (53.6, 28.6), Galactic
  (52.0, 23.0).

**Not** from any reference: the atmospheric noise term. P.372 supplies
atmospheric noise as world maps of `Fam` at 1 MHz (four-hour time blocks by
season, from CCIR Report 322) plus separate frequency-dependence curves. There
is no closed-form equation for it in the Recommendation and this project does
not ship the map coefficients. `atmospheric_noise_figure_db` is a log-linear
surrogate with day/night, season and latitude terms whose shape follows the
published curves and whose absolute anchors are unverified. This is surfaced in
the UI on every run.

Everything is combined on a power basis and converted once, explicitly, at the
boundaries.

## Anchors and how to change them

Every unverified value above is gathered in `app/src/calib.rs` with the range it
is physically defensible over, so a calibration run can vary it and an ordinary
run can ignore it. Nothing new is introduced there; each value already existed
as a named module constant with a unit and a docstring. See
[Calibration](Calibration).

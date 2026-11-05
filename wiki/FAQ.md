# FAQ

Each answer links to the page that covers it in full. The glossary at the end
defines the radio terms used across the wiki.

## Using Skipzone

### Can I rely on a prediction?

Not yet. Skipzone is pre-alpha. The trends it shows, such as how a path changes
with frequency or time of day, are more trustworthy than its absolute signal
levels. [Accuracy](Accuracy) says what has been measured.

### Does the app need an internet connection?

No. Its data is compiled into the binary, and
[Architecture](Architecture#why-i-split-it-this-way) explains why it stays
offline. Two of the validation tools in
[Command Line Tools](Command-Line-Tools) go online.

### Which platforms does it run on?

CI builds and tests on Windows, Ubuntu 24.04 and macOS.
[Getting Started](Getting-Started) has the install steps.

<!-- OWNER: which platform you use and test on day to day, if you want readers to know. -->

### Why is there no browser version?

A WebAssembly build existed and was removed. The browser target has no
threads, so it needed its own sequential code paths through the solver.

<!-- OWNER: why you dropped the browser build, if you want to say -->

### Why does `hfpredict` disagree with the app?

`hfpredict` uses a much simpler model. It has one F2 layer from a coarse
table, with no magnetic field and no absorption. It only checks whether a ray
can reach the receiver. The app models the full ionosphere and prices the
path with a link budget. See [Command Line Tools](Command-Line-Tools#hfpredict).

### Where does the physics come from?

From derivations in the repository.
[How It Works](How-It-Works#the-maths) gives the overview and lists them.

## Developing Skipzone

### Can I use the ray tracer in my own program?

Yes. The engine crate, `skipzone`, has two dependencies and no system
libraries. It traces through any electron-density model you give it. Use
`skipzone-iono` if you want Skipzone's own ionosphere, or
`skipzone-predict` for the whole headless prediction. See
[Architecture](Architecture) for what each crate holds and why they are split.

### What licence is it under?

MIT, except `crates/skipzone-iono/data/nequick/`, which is EUPL-1.2. That
data is compiled into `skipzone-iono`, so a program that uses that crate must
ship the data's licence notice. The engine alone is plain MIT. Details are in
[Contributing](Contributing#data-and-assets).

### How do I report a bug or suggest a change?

Open an issue on the
[GitHub repository](https://github.com/GabrielVicini/Skipzone/issues).
[Contributing](Contributing) covers building, testing and what a pull request
needs to pass.

<!-- OWNER: your preferred channel for questions and whether you want issues before pull requests. -->

## Glossary

**Absorption.** Signal energy lost to collisions between electrons and air
molecules as the wave passes through the ionosphere. It is strongest in the D
region and at low frequencies.

**Anchor.** Skipzone's name for a model constant that no published table pins
down, such as how dense the D region is. Each one has a plausible range, and
the WSPR calibration adjusts some of them.

**Appleton-Hartree equation.** The formula for the refractive index of a
radio wave in a magnetised plasma with collisions. Skipzone uses its full
complex form.

**CCIR maps.** Worldwide coefficient maps of monthly median foF2 and
M(3000)F2, published by the CCIR (now ITU-R). Skipzone's default foF2 comes
from them.

**Chapman layer.** An idealised profile of the ionisation that sunlight
produces in an exponential atmosphere. It peaks at one height and falls off
above and below.

**Corpus.** A saved set of WSPR spots or ionosonde readings that the lab
harnesses read, so every calibration run scores the same data.

**Critical frequency.** The highest frequency a layer reflects when the wave
goes straight up. Above it, a vertical wave passes through.

**D region.** The lowest part of the ionosphere, roughly 60 to 90 km up. It
exists by day, absorbs HF rather than reflecting it, and fades after sunset.

**E region.** The ionosphere from about 90 to 150 km. It reflects the lower HF
bands by day and weakens at night.

**Elevation angle.** The angle above the horizon at which a ray leaves the
antenna. Low angles reach far; high angles come down close.

**Es (sporadic E).** Thin, patchy sheets of dense ionisation in the E region
that appear unpredictably. They can reflect frequencies far above the normal
MUF for short periods.

**F region and F2 layer.** The highest and densest part of the ionosphere,
above about 150 km. The F2 layer is its main peak and carries most long HF
paths.

**foE.** The critical frequency of the E layer.

**foF2.** The critical frequency of the F2 layer. It sets how high a
frequency the ionosphere can return to Earth.

**Great circle.** The shortest path between two points on the Earth's surface.
A radio path follows it unless the ionosphere bends the ray sideways.

**Gyrofrequency.** The frequency at which electrons circle the Earth's magnetic
field lines, about 1 to 1.6 MHz. Near it, the O and X modes behave very
differently.

**Haselgrove equations.** Ray-tracing equations for a medium whose refractive
index depends on position and direction. Skipzone integrates them in three
dimensions.

**HF.** High frequency, 3 to 30 MHz. These are the bands the ionosphere can
reflect over long distances.

**hmF2.** The height of the F2 layer's peak, usually 250 to 400 km.

**Homing.** Searching for the launch angle that puts a ray down at the
receiver.

**Hop.** One trip up to the ionosphere and back down to the ground. Long paths
take several hops.

**IGRF.** The International Geomagnetic Reference Field, the standard model of
the Earth's magnetic field. Skipzone uses the 14th generation, IGRF-14.

**Ionosonde.** A radar that sweeps frequency straight up to measure the
critical frequencies and heights of the layers overhead.

**Link budget.** The sum of gains and losses along a path, which gives the
power at the receiver.

**M(3000)F2.** The ratio of the MUF for a 3000 km path to foF2. It links the
vertical critical frequency to what works on a long oblique path.

**Maidenhead locator.** A grid square code, such as `FN31`, that amateurs use
to give a position.

**MUF.** Maximum usable frequency: the highest frequency that reflects along a
given path. Above it, the ray passes through the layer.

**Noise floor.** The background radio noise at the receiver, from lightning,
man-made sources and space. A signal below it cannot be heard.

**NVIS.** Near-vertical incidence skywave. High-angle radiation that reflects
almost overhead and covers the area out to a few hundred kilometres.

**O and X modes.** The Earth's magnetic field splits a radio wave in the
ionosphere into two waves, ordinary (O) and extraordinary (X). They bend and
absorb differently and can take different paths.

**R12.** The 12-month smoothed sunspot number. The CCIR maps are indexed by it.

**Skip zone.** The ring around a transmitter where nothing is heard. It is too
far for the ground wave and too close for the first sky-wave hop to land.

**SNR.** Signal-to-noise ratio, in dB. WSPR quotes it in a 2500 Hz reference
bandwidth.

**Solar zenith angle.** The angle between the Sun and straight overhead. It
controls how strongly sunlight ionises the D and E regions.

**SSN.** Sunspot number, the usual index of solar activity. A higher SSN
means a denser ionosphere and higher usable frequencies.

**WSPR.** Weak Signal Propagation Reporter, a low-power beacon mode used by
radio amateurs. Receiving stations upload each decode with its SNR, and Skipzone
checks its predictions against that archive.

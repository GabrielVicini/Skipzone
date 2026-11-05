# Skipzone

Skipzone predicts whether an HF radio signal will get from one station to another.
It traces rays in 3D through a modelled ionosphere and the Earth's magnetic field.
Then it estimates the signal-to-noise ratio at the receiver.
You use it through a desktop map app.

<!-- SCREENSHOT: the main window with a TX and RX placed, the traced ray drawn on the map, and the trace panel open on the right showing a USABLE verdict -->

## Who it's for

- Radio amateurs planning a path: which band, roughly when, and whether a path is plausible.
- HF researchers who want a ray tracer whose physics is derived and tested in the open.

<!-- OWNER: why you built Skipzone, in your own words -->

## Getting it

There are no binary releases yet. When there are, they'll be on
[GitHub Releases](https://github.com/GabrielVicini/Skipzone/releases).
Until then you build from source with Rust 1.92 or newer:

```bash
git clone https://github.com/GabrielVicini/Skipzone.git
cd Skipzone
cargo run --release
```

## Status

Pre-alpha, version 0.1.0. Predicted signal levels are indicative only.
[Accuracy](Accuracy) says what has been checked and how well.

<!-- OWNER: write the current status in your own words -->

New here? Start with [Getting Started](Getting-Started). To work on the code, see [Contributing](Contributing).

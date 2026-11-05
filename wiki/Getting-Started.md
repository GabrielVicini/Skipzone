# Getting Started

You build the app, place two stations, run **Calculate**, and read the verdict.

## 1. Install

There are no binary releases yet, so you build from source.
When there are releases, they'll be on [GitHub Releases](https://github.com/GabrielVicini/Skipzone/releases).

You need [Rust](https://www.rust-lang.org/tools/install) 1.95 or newer.
On Debian or Ubuntu, install the X11 and Wayland headers first:

```bash
sudo apt-get install -y --no-install-recommends \
  libxkbcommon-dev libwayland-dev libxcb1-dev \
  libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libx11-dev libgl1-mesa-dev
```

Then clone and run:

```bash
git clone https://github.com/GabrielVicini/Skipzone.git
cd Skipzone
cargo run --release
```

The first build takes a few minutes. The binary lands at `target/release/skipzone-app-bin`.
CI builds and tests on Windows, Ubuntu 24.04 and macOS.

<!-- SCREENSHOT: the app just after launch, with the default TX (Denver) and RX (London) and no trace panel -->

## 2. Place your stations

The top bar has three menus (**Point-to-Point**, **Coverage Maps**, **Help**).
Under them are two rows, **TX** and **RX**.
The app starts with TX near Denver and RX in London, at the current UTC date and time.

You can set each station two ways:

- **Click the map.** The highlighted badge, **TX** or **RX**, decides which station a click moves. Click the other badge to switch.
- **Type it.** Pick **Grid** and type a Maidenhead locator such as `IO91wm`, or pick **Lat/Lon** and type decimal degrees. Text turns red until it parses.

Drag to pan and scroll to zoom. **Fit path** (bottom right) frames both stations.
The **Night** checkbox beside it shades the night side of the Earth.

<!-- SCREENSHOT: the header with the TX row in Grid mode and the RX row in Lat/Lon mode, and the TX badge selected -->

## 3. Set the station and the time

The panel on the right edge of the map holds the per-run settings:

- **Frequency**, in MHz. Drag the value or click to type.
- **Mode**: CW, SSB voice, RTTY (45 baud) or FT8. Each one sets a bandwidth and an SNR threshold.
- **Power**, from 1 W to 1500 W.

**Antennas** opens the antenna picker for each end.
The default at both ends is a horizontal half-wave dipole at 10 m.

The time slider and date sit at the bottom left. Time is always UTC.

Open **Settings** and check the **Sunspot number**.
The app does not fetch it, and the default is 70.
Set it to a recent value for the date you are planning.

<!-- SCREENSHOT: the right-hand controls (Frequency, Mode, Power, Antennas, Settings) and the bottom-left time and date controls -->

## 4. Calculate

Choose **Point-to-Point > Calculate**.
The top-right corner shows "calculating" while the solve runs, usually a few seconds.
The **Point-to-point trace** panel then opens on the right, and the traced ray appears on the map.

The result does not update when you change an input.
Press **Calculate** at the top of the trace panel to run it again.

<!-- SCREENSHOT: the map with the traced ray and hop markers, and the trace panel open beside it -->

## 5. Read the result

The coloured line at the top of the trace panel is the verdict.
Green **USABLE** means a path closed and the predicted SNR clears your mode's threshold.
The lines under it give each layer (F2, E, Es) its own SNR or the reason it found nothing.

Below that, **Modes found** lists each O and X path the solver found.
Click one to fill **Selected mode - totals** with its loss, antenna gain and noise breakdown.

[Reading a Prediction](Reading-a-Prediction) explains every verdict and term.
Treat any single SNR as good to roughly 8 dB; [Accuracy](Accuracy) has the evidence.

<!-- SCREENSHOT: close-up of the verdict chip and the per-layer lines beneath it -->

## Next steps

- **Point-to-Point > Best FREQ** sweeps 2 to 30 MHz on the same path and charts SNR against frequency. **Tune to best** copies the winner into **Frequency**.
- **Coverage Maps > Area coverage** colours the map by predicted SNR from your transmitter. Press **Run coverage** in that dialog.

<!-- SCREENSHOT: the Best FREQ dialog with the band strip and the SNR chart -->
<!-- SCREENSHOT: an area coverage map with the SNR colour key -->

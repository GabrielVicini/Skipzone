//! Command line surface: [`Args`], its documented defaults, the usage
//! text and the parser.

use skipzone_predict::antenna::AntennaKind;
use skipzone_predict::noise::NoiseEnvironment;
pub(crate) struct Args {
    pub(crate) fit: String,
    pub(crate) holdout: Option<String>,
    pub(crate) negatives: Option<String>,
    pub(crate) noise_env: NoiseEnvironment,
    pub(crate) rounds: usize,
    pub(crate) trim_tails: bool,
    /// How many spots to subsample for the re-solve scan; 0 skips it.
    pub(crate) scan: usize,
    /// Cap on how many negatives to solve. A cycle census produces tens of
    /// thousands - 76 252 from fourteen cycles - and each costs a full solve, so
    /// scoring all of them would take hours. Thinned deterministically by a fixed
    /// stride, which preserves the mix of cycles, bands and ranges.
    pub(crate) max_negatives: usize,
    /// Cap on spots per corpus. A solve costs a few hundred milliseconds, so a
    /// 9000-spot corpus is nearly an hour per set. Thinned by a fixed stride from
    /// the whole file, which preserves the band, hour and day balance the corpus
    /// was built to have - a prefix would be one day on one band.
    pub(crate) max_spots: usize,
    /// The antenna assumed at both ends. See [`Args::default`] for why the
    /// calibration default is not the GUI default.
    pub(crate) antenna: AntennaKind,
    /// Height above ground of that antenna, m. Ignored by the isotropic
    /// reference, which is the calibration default.
    pub(crate) antenna_height_m: f64,
    /// Keep the spots the model could only reach through the sporadic-E fallback
    /// in the fit. Off by default - see [`crate::solving::drop_es`] for why they are not physics.
    pub(crate) include_es: bool,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            fit: "corpus/fit.tsv".to_string(),
            holdout: None,
            negatives: None,
            noise_env: NoiseEnvironment::Rural,
            rounds: 8,
            trim_tails: true,
            scan: 0,
            max_negatives: 2000,
            max_spots: 3000,
            // Isotropic rather than the GUI's 10 m dipole.
            //
            // A station's absolute gain is constant per station and absorbed into
            // its fixed effect, so a flat reference loses nothing the corpus could
            // identify (see the identification table at the end of the report).
            //
            // A dipole at a fixed height in metres is 0.06 wavelengths up on 160 m
            // and 0.94 on 10 m, so its gain at a 5 deg launch climbs 11.6 dB per
            // end across that span, and at 30 deg the tilt reverses sign. A
            // per-station constant cannot absorb that band- and elevation-coupled
            // term, so it lands in the residual and drives the absorption scale and
            // noise slopes to their bounds. Pass `--antenna dipole` to use it.
            antenna: AntennaKind::Isotropic,
            antenna_height_m: 10.0,
            // Es spots are excluded from the fit by default. `best_with_es_fallback`
            // uses Es only where nothing deterministic closed, so an Es spot marks
            // a real decode the deterministic tracer failed to close, and fitting
            // to it fits the fallback. On this corpus they were 41 % of solved
            // spots at +21 dB and lowered the fitted slope (0.69 -> 0.59), while
            // the fit without them barely moved (0.73 -> 0.74). They are still
            // solved, reported and scored.
            include_es: false,
        }
    }
}

pub(crate) const USAGE: &str = "\
usage: wspr_calibrate [options]

  --fit PATH        corpus to fit on (default corpus/fit.tsv)
  --holdout PATH    corpus to test on: a DIFFERENT week, ideally a different month
  --negatives PATH  negatives file, for the false-positive rate and a skill score
  --noise-env NAME  receiver noise environment: city, residential, rural,
                    quiet-rural (default rural). An ASSUMPTION, not a measurement,
                    and very nearly unidentifiable here - see the report.
  --rounds N        alternating fit rounds (default 8)
  --keep-tails      do NOT drop the extreme 1 % of station effects
  --scan N          also scan the anchors that need a re-solve (D-region and
                    collision geometry, foE, E-layer geometry, Es), using an N-spot
                    subsample. Costs a full re-solve per value, so start small.
  --max-spots N     how many spots per corpus to solve (default 3000). Thinned by
                    a fixed stride, so the band, hour and day balance the corpus
                    was built to have is preserved.
  --max-negatives N how many negatives to solve (default 2000). Thinned the same
                    way, from the whole file rather than its first N rows.
  --antenna NAME    antenna ASSUMED at both ends: isotropic, dipole, vertical,
                    efhw (default isotropic). The default is deliberately NOT the
                    GUI's 10 m dipole: absolute gain is absorbed into the station
                    effect anyway, whereas a fixed height in METRES imposes a band
                    tilt of its own that the station effects cannot absorb. Pass
                    `dipole` to reproduce runs made before this flag existed.
  --antenna-height M height above ground of that antenna, m (default 10). Ignored
                    by the isotropic reference.
  --include-es      keep the spots only the sporadic-E fallback reached IN the fit.
                    They are excluded by default: Es answers only where nothing
                    deterministic closed, so such a spot measures the fallback and
                    not the ionosphere. They are reported either way.
";

pub(crate) fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = |what: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{what} needs a value"))
        };
        match flag.as_str() {
            "--fit" => a.fit = val("--fit")?,
            "--holdout" => a.holdout = Some(val("--holdout")?),
            "--negatives" => a.negatives = Some(val("--negatives")?),
            "--noise-env" => {
                a.noise_env = match val("--noise-env")?.as_str() {
                    "city" => NoiseEnvironment::City,
                    "residential" => NoiseEnvironment::Residential,
                    "rural" => NoiseEnvironment::Rural,
                    "quiet-rural" => NoiseEnvironment::QuietRural,
                    other => return Err(format!("unknown --noise-env {other}")),
                };
            }
            "--rounds" => {
                a.rounds = val("--rounds")?
                    .parse()
                    .map_err(|e| format!("bad --rounds: {e}"))?;
            }
            "--keep-tails" => a.trim_tails = false,
            "--scan" => {
                a.scan = val("--scan")?
                    .parse()
                    .map_err(|e| format!("bad --scan: {e}"))?;
            }
            "--max-spots" => {
                a.max_spots = val("--max-spots")?
                    .parse()
                    .map_err(|e| format!("bad --max-spots: {e}"))?;
            }
            "--max-negatives" => {
                a.max_negatives = val("--max-negatives")?
                    .parse()
                    .map_err(|e| format!("bad --max-negatives: {e}"))?;
            }
            "--antenna" => {
                a.antenna = match val("--antenna")?.as_str() {
                    "isotropic" => AntennaKind::Isotropic,
                    "dipole" => AntennaKind::HorizontalDipole,
                    "vertical" => AntennaKind::VerticalMonopole,
                    "efhw" => AntennaKind::Efhw,
                    other => return Err(format!("unknown --antenna {other}")),
                };
            }
            "--antenna-height" => {
                a.antenna_height_m = val("--antenna-height")?
                    .parse()
                    .map_err(|e| format!("bad --antenna-height: {e}"))?;
            }
            "--include-es" => a.include_es = true,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

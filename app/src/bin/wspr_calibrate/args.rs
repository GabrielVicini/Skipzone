//! Command line surface: [`Args`], its documented defaults, the usage
//! text and the parser.

use skipzone_app::antenna::AntennaKind;
use skipzone_app::noise::NoiseEnvironment;
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
    /// The antenna ASSUMED at both ends. See [`Args::default`] for why the
    /// calibration default is not the GUI default.
    pub(crate) antenna: AntennaKind,
    /// Height above ground of that antenna, m. Ignored by the isotropic
    /// reference, which is the calibration default.
    pub(crate) antenna_height_m: f64,
    /// Keep the spots the model could only reach through the sporadic-E fallback
    /// in the fit. Off by default - see [`drop_es`] for why they are not physics.
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
            // ISOTROPIC, deliberately, and NOT the GUI's 10 m dipole.
            //
            // A station's absolute gain is constant for that station and is
            // absorbed exactly into its fixed effect, so a flat reference throws
            // away nothing this corpus could ever identify - the identification
            // table at the foot of the report says so.
            //
            // A dipole at a fixed HEIGHT IN METRES is a different matter. It is
            // 0.06 wavelengths up on 160 m and 0.94 on 10 m, so its gain at the
            // 5 deg launch angle a long path uses climbs 11.6 dB per end across
            // that span - 23 dB across the pair - and at 30 deg the same tilt
            // REVERSES SIGN. That is a band-shaped, elevation-coupled term, so a
            // per-station constant cannot absorb it and it lands in the residual,
            // where the only things able to chase it are the absorption scale and
            // the atmospheric noise slopes. Measured: they all run to their
            // bounds doing so.
            //
            // So the flat reference is not a simplification, it is the removal of
            // an assumption the data cannot see past. Pass `--antenna dipole` to
            // put it back and reproduce the older runs.
            antenna: AntennaKind::Isotropic,
            antenna_height_m: 10.0,
            // Es spots are EXCLUDED from the fit by default. `best_with_es_fallback`
            // consults Es only where nothing deterministic closed, so an Es spot
            // records that the deterministic tracer failed to close a path which
            // demonstrably existed - the spot is a decode that really happened.
            // Fitting physics to the sheet's answer is fitting the fallback.
            // Measured on this corpus: they were 41 % of the solved spots at
            // +21 dB, and their presence inverted the fitted slope (0.69 -> 0.59)
            // while the same fit without them left it alone (0.73 -> 0.74).
            // They are still solved, still reported, and still scored.
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

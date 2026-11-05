//! Skipzone's predictions for benchmark rows.
//!
//! The per-row solve is the one `wspr_calibrate` runs (`solving::solve_one`
//! and `negatives::solve_negatives`): `scenario::resolve`,
//! `scenario::build_models`, `solve::solve`, then
//! `solve::best_with_es_fallback`. What differs is what is kept: the
//! calibration rebuilds the SNR without the shipped bias, while the
//! benchmark scores the model as the app ships it.

use skipzone_predict::antenna::{AntennaConfig, AntennaKind};
use skipzone_predict::scenario::{self, Inputs, destination_point, ground_point, to_lat_lon};
use skipzone_predict::solve::{self, LayerMode};

use skipzone::units::Radians;

use super::corpus::{Obs, Sounding};
use crate::wspr_report::band_label;

/// The model under test, as the app ships it, with isotropic antennas at both
/// ends. Isotropic matches the calibration reference and the P.533 inputs
/// (docs/notes/accuracy-roadmap.md §2.5), and absolute antenna gain is a
/// station constant either way.
#[must_use]
pub fn base_inputs() -> Inputs {
    let iso = AntennaConfig {
        kind: AntennaKind::Isotropic,
        ..AntennaConfig::default()
    };
    Inputs {
        tx_antenna: iso,
        rx_antenna: iso,
        ..Inputs::default()
    }
}

/// What the model said about one row.
#[derive(Clone, Debug, PartialEq)]
pub struct Prediction {
    /// Shipped SNR of the reported path, dB, or `None` when no path closed.
    pub snr_db: Option<f64>,
    /// P(decode): the fading probability at the shipped spread times the
    /// probability the supporting layer is there (1 for F2 and E, the
    /// occurrence probability for an Es fallback). 0 with no path.
    pub p_decode: f64,
    /// SNR at or over the threshold, the app's hard verdict.
    pub decodable: bool,
    pub layer: Option<&'static str>,
}

/// Solve one row.
#[must_use]
pub fn predict(obs: &Obs, base: &Inputs) -> Prediction {
    let inputs = obs.inputs(base);
    let a = scenario::resolve(&inputs);
    let none = Prediction {
        snr_db: None,
        p_decode: 0.0,
        decodable: false,
        layer: None,
    };
    let Ok(models) = scenario::build_models(&inputs, &a) else {
        return none;
    };
    let out = solve::solve(&inputs, &a, &models);
    let Some(best) = solve::best_with_es_fallback(&out) else {
        return none;
    };
    Prediction {
        snr_db: Some(best.link.snr_db),
        p_decode: best.link.decode_probability() * best.probability,
        decodable: best.link.snr_db >= best.link.threshold_db,
        layer: Some(best.layer.label()),
    }
}

/// Model values at an ionosonde.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IonoPrediction {
    pub fof2_mhz: f64,
    pub hmf2_km: f64,
    /// Highest frequency a one-hop F2 path closes over 3000 km centred on the
    /// station, MHz. `None` if none closes at [`MUF_LO_MHZ`] or one still
    /// closes at [`MUF_HI_MHZ`].
    pub muf3000_mhz: Option<f64>,
}

/// Bisection bracket and step count for the MUF search. Nine halvings of
/// 38 MHz resolve it to under 0.1 MHz, at eleven solves a sounding.
pub const MUF_LO_MHZ: f64 = 2.0;
pub const MUF_HI_MHZ: f64 = 40.0;
const MUF_STEPS: usize = 9;

/// Inputs for the 3000 km east-west path centred on a station.
#[must_use]
pub fn muf_path_inputs(s: &Sounding, base: &Inputs) -> Inputs {
    let c = ground_point(s.lat, s.lon);
    let half = Radians::new(1_500_000.0 / scenario::EARTH_RADIUS_M);
    let (tx_lat, tx_lon) = to_lat_lon(&destination_point(&c, Radians::from_degrees(270.0), half));
    let (rx_lat, rx_lon) = to_lat_lon(&destination_point(&c, Radians::from_degrees(90.0), half));
    let t = s.timestamp;
    Inputs {
        tx_lat,
        tx_lon,
        rx_lat,
        rx_lon,
        freq_mhz: 10.0,
        utc_hours: f64::from(t.3) + f64::from(t.4) / 60.0,
        year: t.0,
        month: t.1,
        day_of_month: t.2,
        ssn: s.ssn,
        // MUF(3000)F2 is a one-hop F2 figure; Es and extra hops would only add
        // solves.
        max_hops: 1,
        es_enabled: false,
        ..base.clone()
    }
}

/// Model foF2, hmF2 and MUF(3000)F2 at a station
/// (docs/notes/accuracy-roadmap.md §2.3, "MUF from Skipzone").
///
/// foF2 and hmF2 are read from `scenario::resolve` at the path midpoint,
/// which is the station. `with_muf` false skips the bisection.
#[must_use]
pub fn predict_iono(s: &Sounding, base: &Inputs, with_muf: bool) -> IonoPrediction {
    let inputs = muf_path_inputs(s, base);
    let a = scenario::resolve(&inputs);
    let mut p = IonoPrediction {
        fof2_mhz: a.fof2_mhz,
        hmf2_km: a.hmf2_km,
        muf3000_mhz: None,
    };
    if !with_muf {
        return p;
    }
    let Ok(models) = scenario::build_models(&inputs, &a) else {
        return p;
    };
    // The sweep's pattern: one set of models, re-solved per frequency.
    let closes = |f: f64| {
        let mut fi = inputs.clone();
        fi.freq_mhz = f;
        solve::solve(&fi, &a, &models)
            .solutions
            .iter()
            .any(|x| x.layer == LayerMode::F2)
    };
    if !closes(MUF_LO_MHZ) || closes(MUF_HI_MHZ) {
        return p;
    }
    let (mut lo, mut hi) = (MUF_LO_MHZ, MUF_HI_MHZ);
    for _ in 0..MUF_STEPS {
        let mid = 0.5 * (lo + hi);
        if closes(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    p.muf3000_mhz = Some(0.5 * (lo + hi));
    p
}

/// The cut labels of a path row, in the order the report lists them: band,
/// distance bucket, midpoint day or night (the cuts `wspr_report` defines).
#[must_use]
pub fn path_cuts(obs: &Obs) -> Vec<String> {
    let g = obs.geometry();
    vec![
        format!("band={}", band_label(obs.freq_mhz)),
        format!(
            "distance={}",
            crate::wspr_report::distance_bucket(g.range_km)
        ),
        format!(
            "midpoint={}",
            if g.midpoint_night { "night" } else { "day" }
        ),
    ]
}

/// The cut labels of an ionosonde row: the station and a three-hour local
/// solar time bin, as `iono_check` cuts.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn iono_cuts(s: &Sounding) -> Vec<String> {
    let utc = f64::from(s.timestamp.3) + f64::from(s.timestamp.4) / 60.0;
    let lst = (utc + s.lon / 15.0).rem_euclid(24.0);
    let b = (lst / 3.0).floor() as usize * 3;
    vec![
        format!("station={}", s.station),
        format!("lst={b:02}-{:02}", b + 3),
    ]
}

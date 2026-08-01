//! Why does the solver admit a path, and what does it charge it?
//!
//! # The question this exists to answer
//!
//! The WSPR calibrator's worst surviving cell is 160 m by DAY, reached off the E
//! layer: 81 spots, 547 km median range, midpoint sun at 82 deg, and the model
//! reads +21.7 dB optimistic on them. Correcting foE (which really was 9 % high)
//! moved which spots landed in that cell without moving the residual at all, so
//! over-screening is not the cause.
//!
//! Two explanations remain and they need opposite repairs:
//!
//!  * the mode should not be ADMITTED - 1.8 MHz off the daytime E layer is not a
//!    real propagation path and the solver is offering something that does not
//!    exist;
//!  * the mode is real but MISPRICED - it is admitted correctly and the D-region
//!    absorption charged to it is far too small.
//!
//! Neither can be told apart from the WSPR corpus, because every 160 m daytime
//! spot in it is in the same cell. But they make completely different predictions
//! about a CONTROLLED sweep, which is what this binary runs: one fixed geometry,
//! one fixed sun angle, frequency varied. Absorption is proportional to
//! `1/(f + f_H)^2`, so on a fixed path it MUST rise steeply as frequency falls.
//! If the model instead charges 1.8 MHz LESS than 7 MHz on the same path, the
//! absorption integral is wrong and no amount of mode gating will fix it.
//!
//! The corpus cannot do this because its 160 m daytime spots are short and near
//! the terminator while its 40 m daytime spots are long and at high sun - band,
//! range and zenith all move together. Here they are held still by construction.
//!
//! Run:
//! ```text
//! cargo run --release -p skipzone-app --bin mode_audit
//! ```

use std::process::ExitCode;

use skipzone_app::antenna::{AntennaConfig, AntennaKind};
use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve;

/// Transmitter latitude for every sweep, degrees. Mid-latitude Europe, which is
/// where the corpus actually lives.
const TX_LAT_DEG: f64 = 52.0;

/// Put the receiver `range_km` due east of the transmitter at the same latitude.
///
/// Exact rather than approximate: for two points sharing a latitude `phi`,
/// `cos(theta) = sin^2 phi + cos^2 phi cos(dlon)`, so the longitude separation
/// that realises a given central angle is closed-form. A flat `km / 111.32` would
/// be several per cent out at 52 deg and would move the range between bands,
/// which is the one thing this sweep exists to hold still.
fn place(inputs: Inputs, range_km: f64) -> Inputs {
    let theta = range_km * 1e3 / scenario::EARTH_RADIUS_M;
    let phi = TX_LAT_DEG.to_radians();
    let cos_dlon =
        ((theta.cos() - phi.sin() * phi.sin()) / (phi.cos() * phi.cos())).clamp(-1.0, 1.0);
    Inputs {
        tx_lat: TX_LAT_DEG,
        tx_lon: 0.0,
        rx_lat: TX_LAT_DEG,
        rx_lon: cos_dlon.acos().to_degrees(),
        ..inputs
    }
}

/// Where the model's zenith law comes from, decomposed.
///
/// The sweep shows the model charging 43.4 dB at chi=29 and 2.6 dB at chi=88 on
/// one fixed 547 km 1.8 MHz path - an effective `cos^1.3..1.5 chi` law, where the
/// literature for non-deviative absorption is about `cos^0.75`. That over-steepness
/// is the largest unexplained defect left in the model, so before anyone changes a
/// zenith exponent it is worth knowing WHICH factor produces it.
///
/// Absorption goes as the integral of `Ne * nu` along the ray. On a fixed path the
/// ray geometry is fixed, so only two things can move with the sun:
///
///  * the realised peak DENSITY. An alpha-Chapman layer on the grazing function
///    realises `Nm / sqrt(Ch)`, and `Ch -> sec chi` for moderate chi, so this
///    alone contributes about `(cos chi)^0.5` - far too shallow to explain 1.3.
///  * the peak ALTITUDE. The same layer puts its peak at `+H ln(Ch)`, which at
///    chi=85 is some 16 km higher. The electron-neutral collision frequency falls
///    exponentially with height on a ~6.7 km scale, so a 16 km rise multiplies
///    `nu` at the peak by `exp(-16/6.7)`, about 0.09.
///
/// If the second term dominates, the over-steepness is not the density law at all
/// and no exponent on `cos chi` is the right repair: the layer is carrying its
/// ionisation upward out of the collisional region faster than the real D region
/// does. This prints both factors so the arithmetic is checkable instead of
/// asserted.
fn print_d_region_decomposition(probe: &skipzone_app::scenario::Assumptions) {
    let nu_at = |alt_km: f64| {
        scenario::NU_REF_PER_S
            * (-(alt_km - scenario::NU_REF_ALT_KM) / scenario::NU_SCALE_HEIGHT_KM).exp()
    };
    let nu = nu_at(probe.d_region_peak_alt_km);
    let nu_unrisen = nu_at(scenario::D_REGION_PEAK_ALT_KM);
    println!(
        "    D region: realised peak {:.2e} m^-3 at {:.1} km (risen {:+.1} km), \
         nu there {:.2e} /s",
        probe.d_region_peak_ne,
        probe.d_region_peak_alt_km,
        probe.d_region_peak_alt_km - scenario::D_REGION_PEAK_ALT_KM,
        nu
    );
    println!(
        "              Ne x nu = {:.2e}   of which the RISE alone costs a factor {:.3}",
        probe.d_region_peak_ne * nu,
        nu / nu_unrisen
    );
}

/// One row of the sweep.
struct Row {
    freq_mhz: f64,
    layer: String,
    hops: u32,
    absorption_db: f64,
    free_space_db: f64,
    ground_db: f64,
    gain_db: f64,
    arc_km: f64,
    snr_db: f64,
}

fn sweep(base: &Inputs, label: &str, range_km: f64, utc_hours: f64) -> Vec<Row> {
    let mut out = Vec::new();
    for freq_mhz in [1.838, 3.570, 5.366, 7.040, 10.140, 14.097, 18.106, 21.096] {
        let inputs = Inputs {
            freq_mhz,
            utc_hours,
            ..base.clone()
        };
        // Place the receiver `range_km` away on a due-east bearing from the
        // transmitter, so the whole sweep shares one geometry.
        let inputs = place(inputs, range_km);
        let a = scenario::resolve(&inputs);
        let Ok(models) = scenario::build_models(&inputs, &a) else {
            continue;
        };
        let res = solve::solve(&inputs, &a, &models);
        let Some(best) = solve::best_with_es_fallback(&res) else {
            out.push(Row {
                freq_mhz,
                layer: "-- no path --".to_string(),
                hops: 0,
                absorption_db: f64::NAN,
                free_space_db: f64::NAN,
                ground_db: f64::NAN,
                gain_db: f64::NAN,
                arc_km: f64::NAN,
                snr_db: f64::NAN,
            });
            continue;
        };
        out.push(Row {
            freq_mhz,
            layer: best.layer.label().to_string(),
            hops: best.hops,
            absorption_db: best.total_absorption_db,
            free_space_db: best.free_space_loss_db,
            ground_db: best.ground_reflection_loss_db,
            gain_db: best.total_gain_db,
            arc_km: best.total_arc_km,
            snr_db: best.link.snr_db,
        });
    }
    let probe = scenario::resolve(&place(
        Inputs {
            utc_hours,
            ..base.clone()
        },
        range_km,
    ));
    println!(
        "\n--- {label}: {range_km:.0} km, {utc_hours:.1} UTC, midpoint sun {:.0} deg",
        probe.solar.zenith_angle_deg
    );
    print_d_region_decomposition(&probe);
    println!(
        "  {:>8} {:<12} {:>5} {:>9} {:>9} {:>8} {:>8} {:>7} {:>8}",
        "f [MHz]", "layer", "hops", "arc [km]", "free sp", "ground", "absorb", "gain", "SNR"
    );
    for r in &out {
        if r.absorption_db.is_nan() {
            println!("  {:>8.3} {:<12}", r.freq_mhz, r.layer);
            continue;
        }
        // Free-space loss over the RAY ARC, printed beside the arc itself, so the
        // one sanity check that matters can be done by eye: the arc can never be
        // shorter than the ground range, and more hops must mean a longer arc.
        println!(
            "  {:>8.3} {:<12} {:>5} {:>9.0} {:>9.1} {:>8.1} {:>8.1} {:>7.1} {:>8.1}",
            r.freq_mhz,
            r.layer,
            r.hops,
            r.arc_km,
            r.free_space_db,
            r.ground_db,
            r.absorption_db,
            r.gain_db,
            r.snr_db
        );
    }
    out
}

/// How much leverage does the night floor have on the ZENITH LAW, and what does
/// it cost at night?
///
/// The over-steep `cos^1.4` law is not a tunable coefficient: the rise and the
/// density reduction both fall out of the one alpha-Chapman expression
/// `Ne = Nm exp((1 - z - Ch e^-z)/2)`, whose maximum sits at `z = ln Ch` with
/// value `Nm/sqrt(Ch)`. Damping one would mean abandoning the form. So the
/// question is not which coefficient to move but which MECHANISM supplies the
/// low-altitude ionisation the model loses at grazing incidence.
///
/// The night floor is exactly that mechanism and it already exists. It is an
/// overhead-shaped layer (`Ch = 1`) pinned at the UNRISEN peak altitude, so it
/// does not climb out of the collisional region as the sun sets, and the sampler
/// takes the larger of it and the sunlit layer pointwise. At chi = 81 it is
/// already the dominant term at 85 km.
///
/// But it is ONE number serving two physically different jobs - a twilight ledge
/// and a night-time cosmic-ray residual - so more of it must help the day and
/// hurt the night. This measures both sides of that trade before anyone proposes
/// moving it. Nothing here is applied.
fn night_floor_leverage(base: &Inputs) {
    println!("\n--- NIGHT-FLOOR LEVERAGE ON THE ZENITH LAW -----------------");
    println!("  The one existing anchor that adds ionisation the sun did not make, and so does");
    println!("  NOT rise with chi. Absorption in dB at 1.838 MHz, 547 km, at four sun angles.");
    println!("  It is one number doing two jobs, so watch the night column for what it costs.");
    println!(
        "\n  {:>12} {:>10} {:>10} {:>10} {:>10}",
        "floor frac", "chi 29", "chi 61", "chi 81", "night"
    );
    for frac in [0.0, 0.05, 0.10, 0.175, 0.25] {
        let mut cells = Vec::new();
        for utc in [12.0, 16.5, 18.8, 0.0] {
            let mut inputs = Inputs {
                freq_mhz: 1.838,
                utc_hours: utc,
                ..base.clone()
            };
            inputs.anchors.ionosphere.d_night_floor_fraction.value = frac;
            let inputs = place(inputs, 547.0);
            let a = scenario::resolve(&inputs);
            let Ok(models) = scenario::build_models(&inputs, &a) else {
                cells.push(f64::NAN);
                continue;
            };
            let res = solve::solve(&inputs, &a, &models);
            cells.push(
                solve::best_with_es_fallback(&res).map_or(f64::NAN, |s| s.total_absorption_db),
            );
        }
        let show = |v: f64| {
            if v.is_nan() {
                "  no path".to_string()
            } else {
                format!("{v:8.1}")
            }
        };
        println!(
            "  {:>12.3} {:>10} {:>10} {:>10} {:>10}",
            frac,
            show(cells[0]),
            show(cells[1]),
            show(cells[2]),
            show(cells[3])
        );
    }
    println!("\n  A floor that flattens the law raises the chi 81 column much more, in relative");
    println!("  terms, than the chi 29 column - the sunlit layer already dominates at high sun.");
    println!("  The night column is the price: 160 m F2 night already reads 6.6 dB PESSIMISTIC");
    println!("  in the WSPR fit, so absorption added there makes an existing error worse.");
    println!("  If the day gain and the night cost cannot be had at one value, the anchor needs");
    println!("  SPLITTING into a twilight ledge and a night residual - two different physics.");
}

fn main() -> ExitCode {
    println!("=== MODE ADMISSION AND PRICING AUDIT ===========================\n");
    println!("One geometry, one sun angle, frequency varied. Non-deviative absorption goes as");
    println!("1/(f + f_H)^2, so on a FIXED path it must rise steeply as frequency falls. A model");
    println!("that charges 1.8 MHz less than 7 MHz on the same path has a broken absorption");
    println!("integral, and gating which modes are admitted cannot repair that.");
    println!("\nThe WSPR corpus cannot ask this: its 160 m daytime spots are short and near the");
    println!("terminator while its 40 m daytime spots are long and at high sun, so band, range");
    println!("and zenith all move together. Here they are held still.");

    // ISOTROPIC at both ends, matching what `wspr_calibrate --antenna isotropic`
    // assumes. An earlier version of this binary used `Inputs::default()`, i.e. the
    // GUI's 10 m dipole, and then reported loss NET OF GAIN - so two dipoles at a
    // favourable elevation put the figure 10-16 dB below free space and made the
    // column impossible to read. Worse, it was not comparable to the cells it
    // exists to investigate. The loss terms are now printed separately too.
    let antenna = AntennaConfig {
        kind: AntennaKind::Isotropic,
        ..AntennaConfig::default()
    };
    let base = Inputs {
        ssn: 98.0,
        month: 7,
        day_of_month: 5,
        tx_antenna: antenna,
        rx_antenna: antenna,
        ..Inputs::default()
    };

    // The cell under investigation: 547 km, sun near the terminator, which is
    // where the corpus's 160 m daytime spots actually sit.
    sweep(&base, "A) the 160 m day cell's own geometry", 547.0, 16.5);
    // The same path at high sun. If absorption is working, every frequency gets
    // MORE absorption here than in A, and the low bands get much more.
    sweep(&base, "B) same path, high sun", 547.0, 12.0);
    // The 40 m day cell's geometry, for the cross-check the corpus conflates.
    sweep(&base, "C) the 40 m day cell's geometry", 1207.0, 12.0);
    // And at night, where absorption should be near zero at every frequency.
    sweep(&base, "D) same short path, night", 547.0, 0.0);
    // THE CELL ITSELF. The corpus's 160 m daytime spots sit at a median midpoint
    // zenith of 82 deg, not 61, and the model charges them only 5.7 dB there
    // against the 20.6 dB it charges the same path at 61 deg. Absorption is not
    // supposed to collapse toward the terminator: an alpha-Chapman layer on the
    // grazing function realises a peak of Nm/sqrt(Ch), so the density falls only
    // as sqrt(cos chi) while the ray's traverse is unchanged at fixed range. Going
    // from 61 to 82 deg should cost roughly a factor 0.55, not a factor 3.6.
    sweep(
        &base,
        "E) THE CELL: same path, sun near the terminator",
        547.0,
        18.8,
    );
    sweep(&base, "F) same path, deeper twilight", 547.0, 19.6);

    night_floor_leverage(&base);

    println!("\n=== HOW TO READ IT =========================================");
    println!("  Down each block, absorption must RISE as frequency falls - steeply, roughly as");
    println!("  1/(f + f_H)^2 with f_H about 1 MHz at D-region heights, so 1.8 MHz should carry");
    println!("  several times what 7 MHz does on the same path.");
    println!("  Between A and B, every row must gain absorption as the sun rises.");
    println!("  In D, every row should be near zero.");
    println!("  A 160 m row that closes by day with LESS absorption than 40 m is the defect,");
    println!("  and it is a pricing defect, not an admission one.");
    ExitCode::SUCCESS
}

//! The "assumed values (all of them)" collapsing panel: every derived and
//! assumed quantity the engine was fed, laid bare for the operator.

use egui::{CollapsingHeader, Ui};

use crate::scenario::Assumptions;
use crate::ui::widgets::{data_grid, kv, sub_head};

pub fn assumptions_panel(ui: &mut Ui, a: &Assumptions) {
    CollapsingHeader::new("Assumed values (all of them)")
        .default_open(false)
        .show(ui, |ui| {
            data_grid(ui, "assume", 2, |ui| {
                kv(ui, "foF2 (midpoint)", format!("{:.2} MHz", a.fof2_mhz));
                // Shown as three samples of ONE field, not as three numbers: if
                // they differ, the F2 layer really does vary along the path,
                // which is the thing the gridded backend exists to do.
                kv(
                    ui,
                    "  at TX / RX",
                    format!("{:.2} / {:.2} MHz", a.fof2_tx_mhz, a.fof2_rx_mhz),
                );
                kv(ui, "  backend", a.fof2_backend.label().to_string());
                kv(ui, "  source", a.fof2_source.clone());
                kv(ui, "hmF2", format!("{:.0} km", a.hmf2_km));
                kv(ui, "  source", a.hmf2_source.clone());
                kv(
                    ui,
                    "Chapman scale H",
                    format!("{:.0} km", a.scale_height_km),
                );
                kv(ui, "NmF2", format!("{:.4e} m^-3", a.nm_per_m3));
                kv(
                    ui,
                    "Path midpoint",
                    format!("{:.2}, {:.2}", a.midpoint_lat, a.midpoint_lon),
                );
                kv(
                    ui,
                    "Midpoint LST",
                    format!(
                        "{:.2} h ({})",
                        a.lst_hours,
                        if a.is_day { "day" } else { "night" }
                    ),
                );
                kv(ui, "Season", a.season.label().to_string());

                sub_head(ui, "SOLAR GEOMETRY (MIDPOINT)");
                kv(ui, "Day of year", a.solar.day_of_year.to_string());
                kv(
                    ui,
                    "Declination",
                    format!("{:.3} deg", a.solar.declination_deg),
                );
                kv(
                    ui,
                    "Hour angle",
                    format!("{:.3} deg", a.solar.hour_angle_deg),
                );
                kv(
                    ui,
                    "Solar zenith chi",
                    format!("{:.3} deg", a.solar.zenith_angle_deg),
                );
                kv(
                    ui,
                    "Solar elevation",
                    format!("{:.3} deg", a.solar.elevation_deg),
                );

                sub_head(ui, "D REGION (DRIVES ABSORPTION)");
                kv(
                    ui,
                    "D layer",
                    if a.d_region_active {
                        "producing at midpoint (varies along path)".to_string()
                    } else {
                        "night at midpoint (still active on any sunlit part)".to_string()
                    },
                );
                kv(
                    ui,
                    "  midpoint peak Ne",
                    format!("{:.4e} m^-3", a.d_region_peak_ne),
                );
                kv(
                    ui,
                    "  midpoint peak alt",
                    format!("{:.2} km", a.d_region_peak_alt_km),
                );
                kv(ui, "  basis", a.d_region_source.clone());

                sub_head(ui, "E REGION");
                kv(ui, "foE overhead", format!("{:.2} MHz", a.foe_overhead_mhz));
                kv(
                    ui,
                    "  realised at midpoint",
                    format!("{:.2} MHz", a.foe_midpoint_mhz),
                );
                kv(
                    ui,
                    "  peak altitude",
                    format!("{:.0} km", a.e_region_peak_alt_km),
                );
                kv(ui, "  basis", a.foe_source.clone());

                sub_head(ui, "SPORADIC E (PROBABILISTIC)");
                kv(ui, "foEs", format!("{:.2} MHz", a.sporadic_e.foes_mhz));
                kv(
                    ui,
                    "  occurrence",
                    format!("{:.0} %", 100.0 * a.sporadic_e.probability),
                );
                kv(
                    ui,
                    "  solved this run",
                    if a.es_solved {
                        "yes - Es paths reported separately, with their probability".to_string()
                    } else {
                        "no - too unlikely to be worth a second pass, or switched off".to_string()
                    },
                );
                kv(
                    ui,
                    "  sheet",
                    format!(
                        "{:.1} km semi-thick at {:.0} km",
                        a.sporadic_e.semi_thickness_km, a.sporadic_e.height_km
                    ),
                );
                kv(ui, "  basis", a.sporadic_e.source.clone());

                sub_head(ui, "COLLISION FREQUENCY");
                kv(ui, "nu0", format!("{:.3e} /s", a.nu0_per_s));
                kv(ui, "  at altitude", format!("{:.0} km", a.nu_ref_alt_km));
                kv(
                    ui,
                    "  scale height",
                    format!("{:.1} km", a.nu_scale_height_km),
                );
                kv(ui, "  basis", a.collision_source.clone());
                kv(ui, "Field model", a.field_desc.clone());
                kv(ui, "Ground radius", format!("{:.1} km", a.r_ground_m / 1e3));
                kv(ui, "Domain top", format!("{:.1} km", a.r_top_m / 1e3));
            });
        });
}

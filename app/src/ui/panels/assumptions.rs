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
                kv(ui, "foF2", format!("{:.2} MHz", a.fof2_mhz));
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

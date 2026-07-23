//! Left panel: every value the operator can change, plus the RUN TRACE button.

use egui::{ComboBox, DragValue, Grid, RichText, Ui};

use crate::scenario::{GroundType, Inputs, PlaceMode, fof2_from_ssn};
use crate::ui::widgets::{card, hint, labelled_drag, section};

/// Left panel: everything the operator can change. Returns true if Run was hit.
pub fn inputs_panel(ui: &mut Ui, inputs: &mut Inputs, place: &mut PlaceMode) -> bool {
    let mut run = false;
    ui.heading("Inputs");
    ui.add_space(4.0);

    if ui
        .add_sized(
            [ui.available_width(), 30.0],
            egui::Button::new(RichText::new("RUN TRACE").strong()),
        )
        .clicked()
    {
        run = true;
    }
    hint(
        ui,
        "Solving is synchronous; a failed path runs a sweep and may take a second.",
    );

    ui.add_space(8.0);
    ui.separator();

    section(ui, "Endpoints");
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Map click sets").small());
            ui.selectable_value(place, PlaceMode::Tx, "TX");
            ui.selectable_value(place, PlaceMode::Rx, "RX");
        });
        ui.add_space(4.0);
        Grid::new("endpoints")
            .num_columns(3)
            .spacing([6.0, 4.0])
            .show(ui, |ui| {
                ui.label(RichText::new("TX lat/lon").small());
                ui.add(
                    DragValue::new(&mut inputs.tx_lat)
                        .speed(0.05)
                        .range(-89.9..=89.9),
                );
                ui.add(
                    DragValue::new(&mut inputs.tx_lon)
                        .speed(0.05)
                        .range(-180.0..=180.0),
                );
                ui.end_row();
                ui.label(RichText::new("RX lat/lon").small());
                ui.add(
                    DragValue::new(&mut inputs.rx_lat)
                        .speed(0.05)
                        .range(-89.9..=89.9),
                );
                ui.add(
                    DragValue::new(&mut inputs.rx_lon)
                        .speed(0.05)
                        .range(-180.0..=180.0),
                );
                ui.end_row();
            });
    });

    section(ui, "Signal");
    card(ui, |ui| {
        Grid::new("signal")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Frequency",
                    DragValue::new(&mut inputs.freq_mhz)
                        .speed(0.1)
                        .range(0.5..=60.0)
                        .suffix(" MHz"),
                );
                labelled_drag(
                    ui,
                    "Time (UTC)",
                    DragValue::new(&mut inputs.utc_hours)
                        .speed(0.25)
                        .range(0.0..=24.0)
                        .suffix(" h"),
                );
                labelled_drag(ui, "Month", DragValue::new(&mut inputs.month).range(1..=12));
                labelled_drag(
                    ui,
                    "Day of month",
                    DragValue::new(&mut inputs.day_of_month).range(1..=31),
                );
                labelled_drag(
                    ui,
                    "Max hops",
                    DragValue::new(&mut inputs.max_hops).range(1..=8),
                );
            });
    });

    section(ui, "Ionosphere");
    card(ui, |ui| {
        Grid::new("iono")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Sunspot number",
                    DragValue::new(&mut inputs.ssn)
                        .speed(1.0)
                        .range(0.0..=300.0),
                );
                labelled_drag(
                    ui,
                    "hmF2 (peak height)",
                    DragValue::new(&mut inputs.hmf2_km)
                        .speed(1.0)
                        .range(90.0..=600.0)
                        .suffix(" km"),
                );
                labelled_drag(
                    ui,
                    "Chapman scale H",
                    DragValue::new(&mut inputs.scale_height_km)
                        .speed(1.0)
                        .range(5.0..=200.0)
                        .suffix(" km"),
                );
                labelled_drag(
                    ui,
                    "Domain top",
                    DragValue::new(&mut inputs.domain_top_km)
                        .speed(10.0)
                        .range(400.0..=2000.0)
                        .suffix(" km"),
                );
            });
        hint(
            ui,
            &format!(
                "foF2 = {:.2} MHz, derived from SSN (NmF2 linear in SSN, coarse midlat \
                 anchor - not a path/season/time prediction). hmF2 and scale H are yours \
                 directly; there is no climatology table underneath.",
                fof2_from_ssn(inputs.ssn)
            ),
        );
    });

    section(ui, "Absorption inputs");
    card(ui, |ui| {
        hint(
            ui,
            "By default the D-region electron density is derived from the solar \
             zenith angle at the path midpoint (alpha-Chapman, Nm x sqrt(cos chi)), \
             and nu comes from a fixed neutral-atmosphere profile. nu itself is NOT \
             a function of zenith angle - it follows neutral density.",
        );
        ui.add_space(4.0);
        ui.checkbox(
            &mut inputs.collision_manual,
            "Override collision profile manually",
        );
        if inputs.collision_manual {
            ui.add_space(4.0);
            Grid::new("coll")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    labelled_drag(
                        ui,
                        "nu at ref alt",
                        DragValue::new(&mut inputs.nu0_per_s)
                            .speed(1e3)
                            .range(0.0..=1e9)
                            .suffix(" /s"),
                    );
                    labelled_drag(
                        ui,
                        "ref altitude",
                        DragValue::new(&mut inputs.nu_ref_alt_km)
                            .speed(1.0)
                            .range(50.0..=300.0)
                            .suffix(" km"),
                    );
                    labelled_drag(
                        ui,
                        "nu scale H",
                        DragValue::new(&mut inputs.nu_scale_height_km)
                            .speed(0.5)
                            .range(2.0..=100.0)
                            .suffix(" km"),
                    );
                });
        }
    });

    section(ui, "Magnetic field");
    card(ui, |ui| {
        ui.checkbox(
            &mut inputs.use_field,
            "Use IGRF-14 (off = zero field, O/X degenerate)",
        );
        if inputs.use_field {
            ui.add_space(4.0);
            ui.add(
                DragValue::new(&mut inputs.igrf_epoch)
                    .speed(0.1)
                    .range(1900.0..=2030.0)
                    .prefix("epoch "),
            );
        }
    });

    section(ui, "Surface (ground reflections)");
    card(ui, |ui| {
        ComboBox::from_id_salt("ground_type")
            .selected_text(inputs.ground_type.label())
            .show_ui(ui, |ui| {
                for g in GroundType::ALL {
                    ui.selectable_value(&mut inputs.ground_type, g, g.label());
                }
            });
        let (eps_r, sigma) = inputs.ground_type.constants();
        hint(
            ui,
            &format!(
                "Surface at the intermediate ground bounces, used for the Fresnel \
                 reflection loss in the link budget. eps_r = {eps_r:.0}, sigma = {sigma} S/m \
                 (ITU-R P.527 / P.368 HF-band values). One choice approximates the whole \
                 path - there is no coastline database here.",
            ),
        );
    });

    ui.add_space(8.0);
    run
}

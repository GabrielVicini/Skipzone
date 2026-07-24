//! The Settings dialog: every modelling assumption the operator can change.
//!
//! Split from the per-run controls on purpose. Frequency, mode and power live
//! on the map overlay because they change between runs; what is in here defines
//! the *model* - the ionosphere, the absorbing layer, the noise environment,
//! the field and the ground - and is set once and left alone.

use egui::{ComboBox, Context, DragValue, Grid, RichText, Ui, vec2};

use crate::fof2::Fof2Backend;
use crate::noise::{
    MAN_MADE_VALID_MAX_MHZ, MAN_MADE_VALID_MIN_MHZ, NoiseEnvironment, OperatingMode,
};
use crate::scenario::{GroundType, Inputs, fof2_from_ssn};
use crate::state::{Session, UiState};
use crate::ui::widgets::{card, hint, labelled_drag, section};

pub fn show(ctx: &Context, session: &mut Session, ui_state: &mut UiState) {
    let inputs = &mut session.inputs;
    // The open flag and the overlay toggle live in the same struct, so the
    // dialog's `&mut open` is taken as a copy and written back; only the one
    // field the body actually edits is lent to it.
    let mut open = ui_state.modals.settings;
    let show_coastlines = &mut ui_state.show_coastlines;
    super::chrome::dialog(
        ctx,
        "settings_dialog",
        "Settings",
        &mut open,
        vec2(520.0, 560.0),
        |ui| body(ui, inputs, show_coastlines),
    );
    ui_state.modals.settings = open;
}

fn body(ui: &mut Ui, inputs: &mut Inputs, show_coastlines: &mut bool) {
    ionosphere(ui, inputs);
    absorption(ui, inputs);
    noise(ui, inputs);
    magnetic_field(ui, inputs);
    surface(ui, inputs, show_coastlines);
    geometry(ui, inputs);
}

fn ionosphere(ui: &mut Ui, inputs: &mut Inputs) {
    section(ui, "Ionosphere");
    card(ui, |ui| {
        Grid::new("settings_iono")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("foF2 model");
                ComboBox::from_id_salt("fof2_backend")
                    .selected_text(inputs.fof2_backend.label())
                    .show_ui(ui, |ui| {
                        for b in Fof2Backend::ALL {
                            ui.selectable_value(&mut inputs.fof2_backend, b, b.label());
                        }
                    });
                ui.end_row();
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
            });
        hint(
            ui,
            &match inputs.fof2_backend {
                Fof2Backend::ConstantSsn => format!(
                    "foF2 = {:.2} MHz everywhere, derived from SSN alone (NmF2 linear in SSN, \
                     coarse midlat anchor - not a path/season/time prediction). This is the \
                     previous behaviour, kept selectable.",
                    fof2_from_ssn(inputs.ssn)
                ),
                Fof2Backend::Gridded => format!(
                    "foF2 varies across the domain: sampled at each ray point's own latitude \
                     and local solar time from the bundled climatology grid, which reduces to \
                     {:.2} MHz at its reference point (45 deg, equinox, 14 LST) for this SSN. \
                     The grid is NOT CCIR/URSI/IRI data - see the foF2 source line in the \
                     assumptions panel. hmF2 and scale H remain yours directly.",
                    fof2_from_ssn(inputs.ssn)
                ),
            },
        );
    });

    section(ui, "Sporadic E");
    card(ui, |ui| {
        hint(
            ui,
            "Sporadic E is the layer that puts a signal at a few hundred km when F2 cannot. \
             It is also the only layer that may simply not be there, so paths that need it \
             are reported SEPARATELY, with an occurrence probability, and are never folded \
             into the deterministic verdict.",
        );
        ui.add_space(4.0);
        ui.checkbox(&mut inputs.es_enabled, "Model sporadic E");
        ui.add_enabled_ui(inputs.es_enabled, |ui| {
            ui.checkbox(
                &mut inputs.es_manual,
                "Override foEs and occurrence (otherwise from season, local time and latitude)",
            );
            ui.add_enabled_ui(inputs.es_manual, |ui| {
                Grid::new("settings_es")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        labelled_drag(
                            ui,
                            "foEs",
                            DragValue::new(&mut inputs.foes_mhz)
                                .speed(0.1)
                                .range(1.0..=30.0)
                                .suffix(" MHz"),
                        );
                        labelled_drag(
                            ui,
                            "Occurrence probability",
                            DragValue::new(&mut inputs.es_probability)
                                .speed(0.01)
                                .range(0.0..=1.0),
                        );
                    });
            });
        });
    });
}

fn absorption(ui: &mut Ui, inputs: &mut Inputs) {
    section(ui, "Absorption");
    card(ui, |ui| {
        hint(
            ui,
            "By default the D-region electron density follows the solar zenith angle at \
             every point along the ray (alpha-Chapman on the Chapman grazing function), \
             and nu comes from a fixed neutral-atmosphere profile. nu itself is NOT a \
             function of zenith angle - it follows neutral density.",
        );
        ui.add_space(4.0);
        ui.checkbox(
            &mut inputs.collision_manual,
            "Override collision profile manually",
        );
        if inputs.collision_manual {
            ui.add_space(4.0);
            Grid::new("settings_collisions")
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
}

fn noise(ui: &mut Ui, inputs: &mut Inputs) {
    section(ui, "Receiver noise and the usable/unusable verdict");
    card(ui, |ui| {
        ui.label(RichText::new("RX man-made noise environment").small());
        ComboBox::from_id_salt("settings_noise_env")
            .selected_text(inputs.noise_env.label())
            .show_ui(ui, |ui| {
                for env in NoiseEnvironment::ALL {
                    ui.selectable_value(&mut inputs.noise_env, env, env.label());
                }
            });
        if !crate::noise::man_made_range_is_valid(inputs.freq_mhz) {
            hint(
                ui,
                &format!(
                    "{:.2} MHz is outside the {MAN_MADE_VALID_MIN_MHZ}-{MAN_MADE_VALID_MAX_MHZ} \
                     MHz range ITU-R P.372 declares its man-made noise fit valid over; the \
                     figure shown is an extrapolation.",
                    inputs.freq_mhz
                ),
            );
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Mode preset (sets bandwidth + threshold)").small());
        ComboBox::from_id_salt("settings_op_mode")
            .selected_text(inputs.op_mode.label())
            .show_ui(ui, |ui| {
                for mode in OperatingMode::ALL {
                    if ui
                        .selectable_value(&mut inputs.op_mode, mode, mode.label())
                        .clicked()
                    {
                        let (bandwidth_hz, threshold_db) = mode.defaults();
                        inputs.bandwidth_hz = bandwidth_hz;
                        inputs.snr_threshold_db = threshold_db;
                    }
                }
            });

        ui.add_space(4.0);
        Grid::new("settings_snr")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "TX power",
                    DragValue::new(&mut inputs.tx_power_w)
                        .speed(5.0)
                        .range(0.1..=10_000.0)
                        .suffix(" W"),
                );
                labelled_drag(
                    ui,
                    "RX bandwidth",
                    DragValue::new(&mut inputs.bandwidth_hz)
                        .speed(10.0)
                        .range(10.0..=20_000.0)
                        .suffix(" Hz"),
                );
                labelled_drag(
                    ui,
                    "SNR threshold",
                    DragValue::new(&mut inputs.snr_threshold_db)
                        .speed(0.5)
                        .range(-30.0..=60.0)
                        .suffix(" dB"),
                );
            });
        hint(
            ui,
            "A path is called USABLE only when its SNR clears this threshold; a path that \
             closes geometrically but falls short is reported separately. The mode preset \
             only seeds these numbers - both stay editable, so the verdict is never decided \
             by a constant baked into the code. Thresholds are operating-practice figures, \
             not a cited standard.",
        );
    });
}

fn magnetic_field(ui: &mut Ui, inputs: &mut Inputs) {
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
}

fn surface(ui: &mut Ui, inputs: &mut Inputs, show_coastlines: &mut bool) {
    section(ui, "Surface (ground reflections)");
    card(ui, |ui| {
        ComboBox::from_id_salt("settings_ground")
            .selected_text(inputs.ground_type.label())
            .show_ui(ui, |ui| {
                for ground in GroundType::ALL_SELECTABLE {
                    ui.selectable_value(&mut inputs.ground_type, ground, ground.label());
                }
            });

        if inputs.ground_type.is_auto() {
            ui.add_space(6.0);
            ui.label(RichText::new("Land fallback (soil type is not in the data)").small());
            ComboBox::from_id_salt("settings_ground_land")
                .selected_text(inputs.ground_land_fallback.label())
                .show_ui(ui, |ui| {
                    for ground in GroundType::LAND_TYPES {
                        ui.selectable_value(
                            &mut inputs.ground_land_fallback,
                            ground,
                            ground.label(),
                        );
                    }
                });
            hint(
                ui,
                "Each hop's reflection point is tested against the Natural Earth 1:50m \
                 land and lakes polygons: inside a lake gives fresh water, inside land \
                 gives the fallback above, outside both gives sea water. The datasets \
                 record where the water is, not how wet the soil is, so auto-detection \
                 decides water vs. land only. The per-hop table shows the surface chosen \
                 for every bounce and the reason. The 1:50m coastline is generalised to \
                 a km or two, so a point that close to the shore can fall on the wrong \
                 side - immaterial between bounces hundreds of km apart.",
            );

            ui.add_space(6.0);
            ui.checkbox(
                show_coastlines,
                "Debug: draw the land / lake polygons on the map",
            );
            match crate::coastline::get() {
                Ok(c) => hint(ui, &c.summary()),
                Err(e) => {
                    ui.colored_label(
                        crate::ui::theme::FAIL,
                        RichText::new(format!("coastline data unavailable: {e}")).small(),
                    );
                    hint(
                        ui,
                        "Every hop will fall back to the land type above until the two \
                         shapefiles are in place.",
                    );
                }
            }
        } else {
            let (eps_r, sigma) = inputs.ground_type.constants();
            hint(
                ui,
                &format!(
                    "Surface at the intermediate ground bounces, used for the Fresnel \
                     reflection loss in the link budget. eps_r = {eps_r:.0}, sigma = {sigma} \
                     S/m (ITU-R P.527 / P.368 HF-band values). This one choice applies to the \
                     whole path; pick auto-detect to have each bounce classified against the \
                     coastline instead."
                ),
            );
        }
    });
}

fn geometry(ui: &mut Ui, inputs: &mut Inputs) {
    section(ui, "Solver geometry");
    card(ui, |ui| {
        Grid::new("settings_geometry")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                labelled_drag(
                    ui,
                    "Max hops",
                    DragValue::new(&mut inputs.max_hops).range(1..=8),
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
            "Multi-hop paths home one hop of 1/N the great-circle arc, then propagate N \
             hops by specular ground reflection; the terminal miss in the trace readout is \
             the error that assumption incurs.",
        );
    });
}

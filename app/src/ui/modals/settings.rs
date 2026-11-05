//! The Settings dialog: every modelling assumption the operator can change.
//!
//! Frequency, mode and power live on the map overlay because they change
//! between runs. This dialog defines the model (ionosphere, absorbing layer,
//! noise environment, field and ground), which is usually set once.

use egui::{ComboBox, Context, DragValue, Grid, RichText, Ui, vec2};

use crate::state::{AppState, Modal};
use crate::ui::actions::{Action, InputChange, edited};
use crate::ui::widgets::{card, hint, labelled_drag, section};
use skipzone_iono::fof2::Fof2Backend;
use skipzone_predict::noise::{
    MAN_MADE_VALID_MAX_MHZ, MAN_MADE_VALID_MIN_MHZ, NoiseEnvironment, OperatingMode,
};
use skipzone_predict::scenario::{GroundType, Inputs, fof2_from_ssn};

pub fn show(ctx: &Context, state: &AppState, out: &mut Vec<Action>) {
    let open = state.ui.modals.settings;
    let inputs = &state.inputs.scenario;
    let show_coastlines = state.inputs.display.show_coastlines;
    super::chrome::dialog(
        ctx,
        Modal::Settings,
        open,
        "Settings",
        vec2(520.0, 560.0),
        out,
        |ui, out| body(ui, inputs, show_coastlines, out),
    );
}

fn body(ui: &mut Ui, inputs: &Inputs, show_coastlines: bool, out: &mut Vec<Action>) {
    ionosphere(ui, inputs, out);
    absorption(ui, inputs, out);
    noise(ui, inputs, out);
    magnetic_field(ui, inputs, out);
    surface(ui, inputs, show_coastlines, out);
    geometry(ui, inputs, out);
}

/// Let `widget` edit a copy of one input, and emit `change` if it moved.
fn edit<T: Copy + PartialEq>(
    out: &mut Vec<Action>,
    value: T,
    change: fn(T) -> InputChange,
    widget: impl FnOnce(&mut T),
) {
    if let Some(v) = edited(value, widget) {
        out.push(Action::SetInput(change(v)));
    }
}

fn ionosphere(ui: &mut Ui, inputs: &Inputs, out: &mut Vec<Action>) {
    section(ui, "Ionosphere");
    card(ui, |ui| {
        Grid::new("settings_iono")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                ui.label("foF2 model");
                edit(out, inputs.fof2_backend, InputChange::Fof2Backend, |v| {
                    ComboBox::from_id_salt("fof2_backend")
                        .selected_text(v.label())
                        .show_ui(ui, |ui| {
                            for b in Fof2Backend::ALL {
                                ui.selectable_value(v, b, b.label());
                            }
                        });
                });
                ui.end_row();
                edit(out, inputs.ssn, InputChange::Ssn, |v| {
                    labelled_drag(
                        ui,
                        "Sunspot number",
                        DragValue::new(v).speed(1.0).range(0.0..=300.0),
                    );
                });
                edit(out, inputs.hmf2_km, InputChange::Hmf2Km, |v| {
                    labelled_drag(
                        ui,
                        "hmF2 (peak height)",
                        DragValue::new(v)
                            .speed(1.0)
                            .range(90.0..=600.0)
                            .suffix(" km"),
                    );
                });
                edit(
                    out,
                    inputs.scale_height_km,
                    InputChange::ScaleHeightKm,
                    |v| {
                        labelled_drag(
                            ui,
                            "Chapman scale H",
                            DragValue::new(v)
                                .speed(1.0)
                                .range(5.0..=200.0)
                                .suffix(" km"),
                        );
                    },
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
                Fof2Backend::Ccir => format!(
                    "The CCIR numerical maps - real measured coefficients, not a climatology of \
                     this project's construction. foF2 is sampled at each ray point's own \
                     latitude and LONGITUDE, from a Fourier series in universal time over an \
                     expansion in modified dip latitude, so the equatorial anomaly follows the \
                     MAGNETIC equator and two points at the same local time in different sectors \
                     differ. Driven at R12 = {:.0} (SSN {:.0} converted to the pre-2015 sunspot \
                     scale the maps were fitted on). Measured against 70219 GIRO ionosonde \
                     soundings: RMS error 1.10 MHz, against 1.70 for the bundled grid. hmF2 and \
                     scale H remain yours directly.",
                    skipzone_iono::ccir::map_r12_from_ssn_v2(inputs.ssn),
                    inputs.ssn
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
        edit(out, inputs.es_enabled, InputChange::EsEnabled, |v| {
            ui.checkbox(v, "Model sporadic E");
        });
        ui.add_enabled_ui(inputs.es_enabled, |ui| {
            edit(out, inputs.es_manual, InputChange::EsManual, |v| {
                ui.checkbox(
                    v,
                    "Override foEs and occurrence (otherwise from season, local time and latitude)",
                );
            });
            ui.add_enabled_ui(inputs.es_manual, |ui| {
                Grid::new("settings_es")
                    .num_columns(2)
                    .spacing([10.0, 4.0])
                    .show(ui, |ui| {
                        edit(out, inputs.foes_mhz, InputChange::FoesMhz, |v| {
                            labelled_drag(
                                ui,
                                "foEs",
                                DragValue::new(v)
                                    .speed(0.1)
                                    .range(1.0..=30.0)
                                    .suffix(" MHz"),
                            );
                        });
                        edit(
                            out,
                            inputs.es_probability,
                            InputChange::EsProbability,
                            |v| {
                                labelled_drag(
                                    ui,
                                    "Occurrence probability",
                                    DragValue::new(v).speed(0.01).range(0.0..=1.0),
                                );
                            },
                        );
                    });
            });
        });
    });
}

fn absorption(ui: &mut Ui, inputs: &Inputs, out: &mut Vec<Action>) {
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
        edit(
            out,
            inputs.collision_manual,
            InputChange::CollisionManual,
            |v| {
                ui.checkbox(v, "Override collision profile manually");
            },
        );
        if inputs.collision_manual {
            ui.add_space(4.0);
            Grid::new("settings_collisions")
                .num_columns(2)
                .spacing([10.0, 4.0])
                .show(ui, |ui| {
                    edit(out, inputs.nu0_per_s, InputChange::Nu0PerS, |v| {
                        labelled_drag(
                            ui,
                            "nu at ref alt",
                            DragValue::new(v).speed(1e3).range(0.0..=1e9).suffix(" /s"),
                        );
                    });
                    edit(out, inputs.nu_ref_alt_km, InputChange::NuRefAltKm, |v| {
                        labelled_drag(
                            ui,
                            "ref altitude",
                            DragValue::new(v)
                                .speed(1.0)
                                .range(50.0..=300.0)
                                .suffix(" km"),
                        );
                    });
                    edit(
                        out,
                        inputs.nu_scale_height_km,
                        InputChange::NuScaleHeightKm,
                        |v| {
                            labelled_drag(
                                ui,
                                "nu scale H",
                                DragValue::new(v)
                                    .speed(0.5)
                                    .range(2.0..=100.0)
                                    .suffix(" km"),
                            );
                        },
                    );
                });
        }
    });
}

fn noise(ui: &mut Ui, inputs: &Inputs, out: &mut Vec<Action>) {
    section(ui, "Receiver noise and the usable/unusable verdict");
    card(ui, |ui| {
        ui.label(RichText::new("RX man-made noise environment").small());
        edit(out, inputs.noise_env, InputChange::NoiseEnv, |v| {
            ComboBox::from_id_salt("settings_noise_env")
                .selected_text(v.label())
                .show_ui(ui, |ui| {
                    for env in NoiseEnvironment::ALL {
                        ui.selectable_value(v, env, env.label());
                    }
                });
        });
        if !skipzone_predict::noise::man_made_range_is_valid(inputs.freq_mhz) {
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
                    // Picking a preset seeds the bandwidth and threshold below.
                    if ui
                        .selectable_label(inputs.op_mode == mode, mode.label())
                        .clicked()
                    {
                        out.push(Action::SetInput(InputChange::OpMode(mode)));
                    }
                }
            });

        ui.add_space(4.0);
        Grid::new("settings_snr")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                edit(out, inputs.tx_power_w, InputChange::TxPower, |v| {
                    labelled_drag(
                        ui,
                        "TX power",
                        DragValue::new(v)
                            .speed(5.0)
                            .range(0.1..=10_000.0)
                            .suffix(" W"),
                    );
                });
                edit(out, inputs.bandwidth_hz, InputChange::BandwidthHz, |v| {
                    labelled_drag(
                        ui,
                        "RX bandwidth",
                        DragValue::new(v)
                            .speed(10.0)
                            .range(10.0..=20_000.0)
                            .suffix(" Hz"),
                    );
                });
                edit(
                    out,
                    inputs.snr_threshold_db,
                    InputChange::SnrThresholdDb,
                    |v| {
                        labelled_drag(
                            ui,
                            "SNR threshold",
                            DragValue::new(v)
                                .speed(0.5)
                                .range(-30.0..=60.0)
                                .suffix(" dB"),
                        );
                    },
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

fn magnetic_field(ui: &mut Ui, inputs: &Inputs, out: &mut Vec<Action>) {
    section(ui, "Magnetic field");
    card(ui, |ui| {
        edit(out, inputs.use_field, InputChange::UseField, |v| {
            ui.checkbox(v, "Use IGRF-14 (off = zero field, O/X degenerate)");
        });
        if inputs.use_field {
            ui.add_space(4.0);
            edit(out, inputs.igrf_epoch, InputChange::IgrfEpoch, |v| {
                ui.add(
                    DragValue::new(v)
                        .speed(0.1)
                        .range(1900.0..=2030.0)
                        .prefix("epoch "),
                );
            });
        }
    });
}

fn surface(ui: &mut Ui, inputs: &Inputs, show_coastlines: bool, out: &mut Vec<Action>) {
    section(ui, "Surface (ground reflections)");
    card(ui, |ui| {
        edit(out, inputs.ground_type, InputChange::Ground, |v| {
            ComboBox::from_id_salt("settings_ground")
                .selected_text(v.label())
                .show_ui(ui, |ui| {
                    for ground in GroundType::ALL_SELECTABLE {
                        ui.selectable_value(v, ground, ground.label());
                    }
                });
        });

        if inputs.ground_type.is_auto() {
            ui.add_space(6.0);
            ui.label(RichText::new("Land fallback (soil type is not in the data)").small());
            edit(
                out,
                inputs.ground_land_fallback,
                InputChange::GroundLandFallback,
                |v| {
                    ComboBox::from_id_salt("settings_ground_land")
                        .selected_text(v.label())
                        .show_ui(ui, |ui| {
                            for ground in GroundType::LAND_TYPES {
                                ui.selectable_value(v, ground, ground.label());
                            }
                        });
                },
            );
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
            edit(out, show_coastlines, InputChange::ShowCoastlines, |v| {
                ui.checkbox(v, "Debug: draw the land / lake polygons on the map");
            });
            match skipzone_predict::coastline::get() {
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

fn geometry(ui: &mut Ui, inputs: &Inputs, out: &mut Vec<Action>) {
    section(ui, "Solver geometry");
    card(ui, |ui| {
        Grid::new("settings_geometry")
            .num_columns(2)
            .spacing([10.0, 4.0])
            .show(ui, |ui| {
                edit(out, inputs.max_hops, InputChange::MaxHops, |v| {
                    labelled_drag(ui, "Max hops", DragValue::new(v).range(1..=8));
                });
                edit(out, inputs.domain_top_km, InputChange::DomainTopKm, |v| {
                    labelled_drag(
                        ui,
                        "Domain top",
                        DragValue::new(v)
                            .speed(10.0)
                            .range(400.0..=2000.0)
                            .suffix(" km"),
                    );
                });
            });
        hint(
            ui,
            "Multi-hop paths home one hop of 1/N the great-circle arc, then propagate N \
             hops by specular ground reflection; the terminal miss in the trace readout is \
             the error that assumption incurs.",
        );
    });
}

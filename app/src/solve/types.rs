//! Result types produced by the solver and consumed by the UI: one traced
//! hop, an assembled multi-hop solution, a near-miss record, and the overall
//! outcome of a solve.

use skipzone::magnetoionic::Mode;

use crate::noise::LinkBudget;

#[must_use]
pub fn mode_label(m: Mode) -> &'static str {
    match m {
        Mode::Ordinary => "O",
        Mode::Extraordinary => "X",
    }
}

/// One traced hop, with everything the UI wants to show about it.
#[derive(Clone)]
pub struct HopDetail {
    pub index: u32,
    pub launch_elev_deg: f64,
    pub launch_az_deg: f64,
    pub arrival_elev_deg: f64,
    pub arrival_az_deg: f64,
    pub apex_alt_km: f64,
    /// X = (fp/f)^2 at the apex, from the engine's own apex record. At an
    /// isotropic reflection this should sit at the plasma condition.
    pub apex_x: f64,
    pub apex_lat_lon: (f64, f64),
    pub ground_range_km: f64,
    pub group_km: f64,
    pub phase_km: f64,
    pub arc_km: f64,
    pub absorption_db: f64,
    /// Ground-reflection loss [dB] incurred where this hop lands, when another
    /// hop follows (0 for the final hop, which arrives at the receiver).
    pub ground_loss_db: f64,
    /// Surface used at that reflection, `None` when this hop does not reflect.
    /// Constant across hops for a manual selection; per-hop when the surface is
    /// auto-detected from the coastline.
    pub ground_label: Option<&'static str>,
    /// Why that surface was picked, when auto-detection made the choice.
    /// `None` for a manual selection, where there is nothing to explain.
    pub ground_reason: Option<String>,
    pub steps: usize,
    pub hamiltonian_drift: f64,
    pub outcome: &'static str,
    /// Ground-track polyline for this hop, decimated, (lat, lon).
    pub polyline: Vec<(f64, f64)>,
    /// Landing point of this hop.
    pub end_lat_lon: (f64, f64),
}

#[derive(Clone)]
pub struct Solution {
    pub mode: Mode,
    pub hops: u32,
    pub hop_details: Vec<HopDetail>,
    pub total_group_km: f64,
    pub total_phase_km: f64,
    pub total_arc_km: f64,
    pub total_absorption_db: f64,
    /// Free-space spreading loss over the total ray path, dB.
    pub free_space_loss_db: f64,
    /// Summed Fresnel loss over the intermediate ground reflections, dB.
    pub ground_reflection_loss_db: f64,
    /// Number of intermediate ground reflections (hops - 1 for a landed path).
    pub num_ground_reflections: u32,
    /// Basic transmission loss = free-space + absorption + ground reflection, dB.
    /// PROPAGATION only: deliberately excludes antenna gains (carried separately
    /// below) and any statistical excess-system-loss term.
    pub total_system_loss_db: f64,
    /// Transmitting antenna gain [dBi] at this solution's launch elevation.
    pub tx_gain_dbi: f64,
    /// Receiving antenna gain [dBi] at this solution's arrival elevation.
    pub rx_gain_dbi: f64,
    /// Launch elevation of the first hop [deg] - the angle `tx_gain_dbi` was
    /// read at. Duplicated out of `hop_details` so the UI can show the pairing
    /// without digging.
    pub tx_elev_deg: f64,
    /// Arrival elevation of the last hop [deg], where `rx_gain_dbi` was read.
    pub rx_elev_deg: f64,
    /// `tx_gain_dbi + rx_gain_dbi` [dB]: what the antennas add back to (or take
    /// off) the propagation loss.
    pub total_gain_db: f64,
    /// Received power, noise floor and SNR for this path: the judgment layer
    /// that decides whether a closing geometry is actually audible. Built from
    /// `total_system_loss_db - total_gain_db` plus the transmitter power and the
    /// noise floor; it changes nothing about the loss terms above.
    pub link: LinkBudget,
    pub total_ground_km: f64,
    /// Distance from the final landing point to the requested receiver.
    pub terminal_miss_km: f64,
    /// Miss reported by the single-hop homing that produced the launch angles.
    pub homing_miss_m: f64,
    pub max_hamiltonian_drift: f64,
    pub total_steps: usize,
    /// Time of flight from the group path, ms.
    pub group_delay_ms: f64,
    /// Non-fatal note, e.g. a later hop failing after the first succeeded.
    pub note: Option<String>,
}

#[derive(Clone)]
pub struct NearMiss {
    pub mode: Mode,
    pub hops: u32,
    pub elevation_deg: f64,
    pub landed_range_km: f64,
    pub target_range_km: f64,
    pub miss_km: f64,
    pub note: String,
}

pub struct SolveOutcome {
    pub solutions: Vec<Solution>,
    /// The noise floor every solution above was judged against. Present even
    /// when nothing was found, so the panel can still show what the receiver
    /// would have been listening through.
    pub noise: crate::noise::NoiseFloor,
    /// SNR threshold in force for this solve, dB.
    pub snr_threshold_db: f64,
    pub near_misses: Vec<NearMiss>,
    /// Plain-language outcome of the elevation sweep when nothing homed -
    /// notably the case where no elevation reflects at all, which produces no
    /// "closest landing" and would otherwise leave the operator with a blank
    /// panel.
    pub sweep_notes: Vec<String>,
    /// Every typed engine error encountered, verbatim, with context.
    pub errors: Vec<String>,
    pub great_circle_km: f64,
    pub bearing_deg: f64,
    pub reverse_bearing_deg: f64,
    pub elapsed_ms: f64,
}

//! Full dump of every solution the default scenario produces, per hop, so the
//! output of two builds can be diffed exactly.

use skipzone_app::scenario::{self, Inputs};
use skipzone_app::solve;

fn main() {
    for (name, inputs) in cases() {
        println!("
######## {name}");
        run(&inputs);
    }
}

fn cases() -> Vec<(&'static str, Inputs)> {
    vec![
        ("Denver->London 14.1 (default)", Inputs::default()),
        ("Denver->NY 10.1 midday", Inputs { freq_mhz: 10.1, month: 6, day_of_month: 21, utc_hours: 19.0, rx_lat: 40.7, rx_lon: -74.0, max_hops: 3, ..Inputs::default() }),
        ("Denver->NY 7.1 midday", Inputs { freq_mhz: 7.1, month: 6, day_of_month: 21, utc_hours: 19.0, rx_lat: 40.7, rx_lon: -74.0, max_hops: 3, ..Inputs::default() }),
        ("Denver->Tokyo 21.1", Inputs { freq_mhz: 21.1, rx_lat: 35.7, rx_lon: 139.7, ..Inputs::default() }),
        ("Denver->Sydney 21.1", Inputs { freq_mhz: 21.1, rx_lat: -33.9, rx_lon: 151.2, ..Inputs::default() }),
        ("Denver->Buenos Aires 18.1", Inputs { freq_mhz: 18.1, rx_lat: -34.6, rx_lon: -58.4, ..Inputs::default() }),
        ("18.1 Es short", Inputs { freq_mhz: 18.1, month: 7, day_of_month: 24, utc_hours: 3.37, tx_lat: 40.0, tx_lon: -105.0, rx_lat: 43.6, rx_lon: -105.0, max_hops: 2, ..Inputs::default() }),
    ]
}

fn run(inputs: &Inputs) {
    let inputs = inputs.clone();
    let a = scenario::resolve(&inputs);
    let models = scenario::build_models(&inputs, &a).expect("models");
    let out = solve::solve(&inputs, &a, &models);

    println!("great circle {:.3} km", out.great_circle_km);
    println!("solutions {}  es {}", out.solutions.len(), out.es_solutions.len());
    for (i, s) in out.solutions.iter().chain(&out.es_solutions).enumerate() {
        println!(
            "[{i}] {:?} {:?} hops {} group {:.6} km ground {:.6} km arc {:.6} km \
             homing_miss {:.6} m TERMINAL_MISS {:.6} km snr {:.6} dB steps {}",
            s.mode,
            s.layer,
            s.hops,
            s.total_group_km,
            s.total_ground_km,
            s.total_arc_km,
            s.homing_miss_m,
            s.terminal_miss_km,
            s.link.snr_db,
            s.total_steps,
        );
        for h in &s.hop_details {
            println!(
                "      hop {} launch {:.6} deg arrival {:.6} deg apex {:.3} km \
                 range {:.6} km end {:.5},{:.5} outcome {}",
                h.index,
                h.launch_elev_deg,
                h.arrival_elev_deg,
                h.apex_alt_km,
                h.ground_range_km,
                h.end_lat_lon.0,
                h.end_lat_lon.1,
                h.outcome,
            );
        }
    }
    for e in &out.errors {
        println!("ERROR {e}");
    }
}

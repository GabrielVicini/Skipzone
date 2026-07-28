//! Fetch a reproducible WSPR calibration corpus once and write it to disk.
//!
//! ```text
//! cargo run --release -p skipzone-app --bin wspr_corpus -- \
//!     --from 2026-07-02 --days 7 --out corpus/fit.tsv --neg corpus/fit_neg.tsv
//! ```
//!
//! Fetching is separated from fitting on purpose. The scoring query in
//! `wsprlive::Query` samples with `ORDER BY rand()`, so fitting against a live
//! query optimises against a moving target - a parameter change and a resample
//! are indistinguishable in the result. This writes a file; the fit reads it.
//!
//! What the run does, in order:
//!
//! 1. Walks a schedule of (day, hour, band) windows, so the corpus spans the
//!    diurnal cycle - which is what identifies the D region's zenith-angle
//!    dependence - and spans 1.8 to 28 MHz, which is what identifies the
//!    frequency dependence of absorption.
//! 2. Counts what the hygiene filters removed, per window, and reports it.
//! 3. Fetches the observed sunspot number PER DAY, not once for the whole span.
//! 4. De-duplicates, cross-checks each spot's distance against the great circle
//!    the solver itself computes, and drops stations too rare to carry a fixed
//!    effect - reporting the size of each cut.
//! 5. Builds a negatives set from full cycle censuses: paths that were attempted
//!    and did not decode. See `corpus`'s module docs for what that can mean.
//!
//! Exit status is 0 when a corpus was written, 1 when it could not be.

use std::collections::BTreeMap;
use std::process::ExitCode;

use skipzone_app::corpus::{
    self, CorpusSpot, MIN_SPOTS_PER_STATION, Negative, Rejections,
};
use skipzone_app::spaceweather::{self, Ssn, SsnSource};
use skipzone_app::wspr::{WsprSpot, parse_spots};
use skipzone_app::wsprlive::{
    self as wsprlive, BusiestFilter, CorpusQuery, HygieneCensus, StationEnd, Window, cycle_census_tsv,
};
use skipzone_app::{grid, scenario};

/// UTC hours the NEGATIVES schedule walks. Es occurrence and D-region absorption
/// both vary strongly through the day, so a negatives set drawn from one hour
/// would measure the false-positive rate of one ionosphere.
///
/// The POSITIVES no longer need this: each query covers a whole UTC day, and the
/// deterministic hash ordering samples uniformly across it, so all 24 hours are
/// represented without asking for them one at a time.
const SCHEDULE_HOURS: [u32; 8] = [0, 3, 6, 9, 12, 15, 18, 21];

/// Band centres to target explicitly, MHz - the WSPR dial frequencies.
///
/// A whole-archive sample follows real activity and returns almost nothing on
/// 160 m or 10 m. The absorption model's frequency dependence is exactly what a
/// two-band corpus cannot test, so the quiet bands are asked for by name.
const BANDS_MHZ: [f64; 10] = [
    1.836, 3.568, 5.364, 7.038, 10.138, 14.095, 18.104, 21.094, 24.924, 28.124,
];

struct Args {
    from: String,
    days: u32,
    out: String,
    neg: Option<String>,
    per_band: u32,
    per_window_any: u32,
    window_minutes: u32,
    min_km: u32,
    max_km: u32,
    neg_cycles: u32,
    salt: u32,
    ssn_override: Option<f64>,
    top_tx: u32,
    top_rx: u32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            from: "2026-07-02".to_string(),
            days: 7,
            out: "corpus.tsv".to_string(),
            neg: None,
            per_band: 120,
            per_window_any: 600,
            window_minutes: 20,
            min_km: 300,
            max_km: 20_000,
            neg_cycles: 10,
            salt: 1,
            ssn_override: None,
            top_tx: 80,
            top_rx: 30,
        }
    }
}

const USAGE: &str = "\
usage: wspr_corpus [options]

  --from YYYY-MM-DD   first UTC day of the span (default 2026-07-02)
  --days N            how many consecutive days (default 7)
  --out PATH          positives file to write (default corpus.tsv)
  --neg PATH          also build a negatives file here
  --per-band N        spots per (day, band); one query per day per band (default 120)
  --per-window N      extra all-band spots per day (default 600)
  --window-minutes N  width of each window (default 20)
  --min-km / --max-km path length bounds (default 300 / 20000)
  --neg-cycles N      how many cycle censuses to build negatives from (default 10)
  --salt N            changes the deterministic sample without randomising it
  --ssn N             use this SSN for every day instead of fetching per day
                      (records itself as an assumption in the file header)
  --top-tx N          restrict to the N busiest transmitters (default 80)
  --top-rx N          restrict to the N busiest receivers (default 30)
                      Both exist so every station has enough spots for its fixed
                      effect to be identified, which needs DENSITY not breadth:
                      measured, 477 spots spread over 500 TX and 150 RX left every
                      station below the threshold and the corpus empty. Small sets
                      with many spots each is the shape that works. See the corpus
                      module docs for what it costs in representativeness.
";

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("\ncorpus could not be built: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut val = |what: &str| -> Result<String, String> {
            it.next().ok_or_else(|| format!("{what} needs a value"))
        };
        match flag.as_str() {
            "--from" => a.from = val("--from")?,
            "--days" => a.days = num(&val("--days")?)? as u32,
            "--out" => a.out = val("--out")?,
            "--neg" => a.neg = Some(val("--neg")?),
            "--per-band" => a.per_band = num(&val("--per-band")?)? as u32,
            "--per-window" => a.per_window_any = num(&val("--per-window")?)? as u32,
            "--window-minutes" => a.window_minutes = num(&val("--window-minutes")?)? as u32,
            "--min-km" => a.min_km = num(&val("--min-km")?)? as u32,
            "--max-km" => a.max_km = num(&val("--max-km")?)? as u32,
            "--neg-cycles" => a.neg_cycles = num(&val("--neg-cycles")?)? as u32,
            "--salt" => a.salt = num(&val("--salt")?)? as u32,
            "--ssn" => a.ssn_override = Some(num(&val("--ssn")?)?),
            "--top-tx" => a.top_tx = num(&val("--top-tx")?)? as u32,
            "--top-rx" => a.top_rx = num(&val("--top-rx")?)? as u32,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    Ok(a)
}

fn num(s: &str) -> Result<f64, String> {
    s.parse::<f64>().map_err(|e| format!("bad number {s:?}: {e}"))
}

/// The great-circle range the SOLVER computes for a spot, km.
///
/// Used for the archive-distance cross-check and for the negatives' range
/// window. Deliberately the solver's own function rather than an independent
/// great circle: the point is to confirm the archive agrees with the geometry the
/// model will actually trace, not to test two implementations against each other.
fn solved_km(spot: &WsprSpot) -> f64 {
    let tx = scenario::ground_point(spot.tx_lat, spot.tx_lon);
    let rx = scenario::ground_point(spot.rx_lat, spot.rx_lon);
    skipzone::geo::central_angle(&tx, &rx).get() * scenario::EARTH_RADIUS_M / 1e3
}

#[allow(clippy::too_many_lines)] // one report, printed top to bottom
fn run(args: &Args) -> Result<(), String> {
    println!("=== BUILDING A REPRODUCIBLE WSPR CALIBRATION CORPUS ===\n");
    let days = day_span(&args.from, args.days)?;
    println!(
        "SPAN     {} to {} UTC ({} day(s)), one whole-day query per band",
        fmt_day(days[0]),
        fmt_day(days[days.len() - 1]),
        days.len(),
    );

    // ---------------------------------------------------------------- SSN
    // Per DAY, not one value for the span: a week spans real changes in solar
    // activity, and scoring all of it against one number charges the model for
    // an input error it did not make.
    let mut ssn_for_day: BTreeMap<(i32, u32, u32), Ssn> = BTreeMap::new();
    for &d in &days {
        let ssn = match args.ssn_override {
            Some(v) => Ssn {
                value: v,
                source: SsnSource::Operator,
                as_of: d,
                stdev: None,
            },
            None => spaceweather::ssn_for(d.0, d.1, d.2)
                .map_err(|e| format!("no sunspot number for {}: {e}", fmt_day(d)))?,
        };
        println!("SOLAR    {} -> {}", fmt_day(d), ssn);
        ssn_for_day.insert(d, ssn);
    }

    // ------------------------------------------------- who to draw spots from
    //
    // A uniform sample of the whole archive cannot support a fixed-effects
    // model: measured on one day, 224 uniformly-drawn spots spanned 173
    // stations, and requiring MIN_SPOTS_PER_STATION removed every one of them.
    // So the corpus is drawn from the busiest stations, which is the only way
    // the station effects are identified at all - and which biases the corpus
    // towards the better-equipped end of the network. That cost is reported here
    // and again with the station-effect distribution, never absorbed silently.
    let census_date = fmt_day(days[days.len() / 2]);
    let busiest = BusiestFilter {
        census_date: census_date.clone(),
        top_tx: args.top_tx,
        top_rx: args.top_rx,
    };
    let census_day = Window::Day {
        utc_date: census_date,
    };
    println!("\nRanking stations over {} ...", census_day.describe());
    // The ranking is applied inside the data query as a subquery (500 callsigns
    // in an IN clause makes a URL the endpoint rejects with HTTP 414). It is ALSO
    // fetched as a list here, purely so the run can report how many stations it
    // found and show the top of each end - a restriction that cannot be seen is
    // a restriction that gets forgotten.
    let top_tx = wsprlive::busiest_stations(
        &census_day,
        args.min_km,
        args.max_km,
        30.0,
        StationEnd::Transmitter,
        args.top_tx,
    )
    .map_err(|e| format!("could not rank transmitters: {e}"))?;
    let top_rx = wsprlive::busiest_stations(
        &census_day,
        args.min_km,
        args.max_km,
        30.0,
        StationEnd::Receiver,
        args.top_rx,
    )
    .map_err(|e| format!("could not rank receivers: {e}"))?;
    println!(
        "  restricting to the {} busiest transmitters and {} busiest receivers",
        top_tx.len(),
        top_rx.len()
    );
    println!(
        "  busiest transmitters: {}",
        top_tx
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  busiest receivers:    {}",
        top_rx
            .iter()
            .take(6)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  SELECTION BIAS, stated: these are the best-sited and best-equipped stations"
    );
    println!(
        "  in the network. Every station effect below therefore describes the ACTIVE"
    );
    println!("  CORE of WSPR, not its median member.");

    // ---------------------------------------------------------------- fetch
    let mut raw: Vec<CorpusSpot> = Vec::new();
    let mut census = HygieneCensus::default();
    let mut parse_problems = 0usize;
    let mut queries = 0usize;
    println!("\nFetching...");

    // ONE query per (day, band) over the WHOLE day, rather than one per
    // (day, hour, band).
    //
    // Two measured reasons. First, the station-ranking subquery costs about 7
    // seconds per query against this endpoint, so 88 queries a day took ten
    // minutes; ten queries a day take one. Second, and more important, the
    // deterministic `cityHash64` ordering samples uniformly across whatever window
    // it is given, so a whole-day window already spreads the sample over all 24
    // hours - the diurnal coverage the D-region zenith law needs comes free, and
    // asking for each hour separately bought nothing but queries.
    for &d in &days {
        let ssn = &ssn_for_day[&d];
        let window = Window::Day {
            utc_date: fmt_day(d),
        };
        // An all-band draw, which follows real band activity, plus a quota per
        // band so the quiet ends of HF are present at all.
        let mut targets: Vec<(Option<f64>, u32)> = vec![(None, args.per_window_any)];
        targets.extend(BANDS_MHZ.iter().map(|&b| (Some(b), args.per_band)));

        for (band, limit) in targets {
            if limit == 0 {
                continue;
            }
            let q = CorpusQuery {
                window: window.clone(),
                min_km: args.min_km,
                max_km: args.max_km,
                max_mhz: 30.0,
                limit,
                band_mhz: band,
                busiest: Some(busiest.clone()),
                salt: args.salt,
            };
            queries += 1;
            let tsv = q.fetch_tsv().map_err(|e| format!("{}: {e}", fmt_day(d)))?;
            let (spots, problems) = parse_spots(&tsv);
            parse_problems += problems.len();
            for spot in spots {
                raw.push(CorpusSpot {
                    spot,
                    ssn: ssn.value,
                    ssn_source: format!("{}", ssn.source),
                });
            }
            // The hygiene census is only meaningful for the unrestricted draw;
            // per-band counts would re-count the same rows.
            if band.is_none() {
                match q.hygiene_census() {
                    Ok(c) => census.add(c),
                    Err(e) => eprintln!("  ! hygiene census failed for {}: {e}", fmt_day(d)),
                }
            }
        }
        println!("  {} -> {} row(s) so far", fmt_day(d), raw.len());
    }

    println!("\n{queries} queries, {} raw row(s) returned", raw.len());
    if parse_problems > 0 {
        println!("  {parse_problems} unreadable row(s), reported not skipped");
    }

    // ------------------------------------------------------- what was excluded
    println!("\n--- WHAT THE HYGIENE FILTERS REMOVED -----------------------");
    println!("  (counted over the unrestricted draws only; the tests overlap, so");
    println!("   these explain the cut rather than partitioning it)\n");
    let pct = |n: u64| {
        if census.total == 0 {
            0.0
        } else {
            #[allow(clippy::cast_precision_loss)]
            let v = 100.0 * n as f64 / census.total as f64;
            v
        }
    };
    println!("  rows in the windows              {}", census.total);
    println!(
        "  not WSPR message type 1          {:>8}  ({:.1} %)  compound/hashed callsigns:",
        census.not_type1,
        pct(census.not_type1)
    );
    println!("                                             a hash cannot carry a station effect");
    println!(
        "  TX grid shorter than 6 chars     {:>8}  ({:.1} %)  +/-70 km is 17 % of a 400 km path",
        census.short_tx_grid,
        pct(census.short_tx_grid)
    );
    println!(
        "  RX grid shorter than 6 chars     {:>8}  ({:.1} %)",
        census.short_rx_grid,
        pct(census.short_rx_grid)
    );
    println!(
        "  implausible claimed power        {:>8}  ({:.1} %)",
        census.bad_power,
        pct(census.bad_power)
    );
    println!(
        "  survived all of them             {:>8}  ({:.1} %)",
        census.kept,
        pct(census.kept)
    );

    // ---------------------------------------------------------------- clean
    let before = raw.len();
    let (cleaned, mut rej) = corpus::clean(raw, solved_km);
    let spots = corpus::require_identifiable_stations(cleaned, &mut rej);
    report_rejections(before, spots.len(), &rej);

    if spots.is_empty() {
        return Err("every row was rejected; nothing to write".to_string());
    }

    describe_corpus(&spots);

    // ---------------------------------------------------------------- write
    let provenance = format!(
        "Skipzone WSPR calibration corpus\n\
         span {} to {} UTC, one whole-day deterministic sample per band\n\
         filters: {}\n\
         plus, applied locally: de-duplication by (cycle, TX, RX, band); archive \
         distance cross-checked against the solver's own great circle to within \
         {:.0} %; stations appearing fewer than {} times dropped so every station \
         effect is identified\n\
         sunspot number: {}\n\
         {} spots, {} distinct transmitters, {} distinct receivers",
        fmt_day(days[0]),
        fmt_day(days[days.len() - 1]),
        CorpusQuery {
            window: Window::Day {
                utc_date: fmt_day(days[0])
            },
            min_km: args.min_km,
            max_km: args.max_km,
            max_mhz: 30.0,
            limit: args.per_window_any,
            band_mhz: None,
            busiest: Some(busiest.clone()),
            salt: args.salt,
        }
        .describe_filter(),
        100.0 * corpus::MAX_DISTANCE_MISMATCH,
        MIN_SPOTS_PER_STATION,
        match args.ssn_override {
            Some(v) => format!("{v} supplied on the command line for EVERY day - an assumption"),
            None => "observed, fetched per day".to_string(),
        },
        spots.len(),
        distinct(&spots, |s| &s.spot.tx_call),
        distinct(&spots, |s| &s.spot.rx_call),
    );
    std::fs::write(&args.out, corpus::write_positives(&spots, &provenance))
        .map_err(|e| format!("{}: {e}", args.out))?;
    println!("\nWROTE    {} ({} spots)", args.out, spots.len());

    // ---------------------------------------------------------------- negatives
    if let Some(path) = &args.neg {
        let negatives = build_negatives(args, &days, &ssn_for_day)?;
        if negatives.is_empty() {
            println!("\nNo negatives could be constructed; the file was not written.");
        } else {
            let neg_provenance = format!(
                "Skipzone WSPR NEGATIVES: paths attempted in a cycle that did not decode.\n\
                 A negative is (TX transmitted in this cycle) AND (RX decoded at least {} \
                 OTHER stations on the SAME band in the SAME cycle) AND (RX did not decode TX).\n\
                 CAVEATS, both of which bias the measured false-positive rate UPWARD, i.e. \
                 against the model: TX's transmission may have collided at RX, and RX's \
                 antenna may be directional and pointed elsewhere. Neither is knowable from \
                 the archive, so a false-positive rate measured here is an UPPER BOUND.\n\
                 range window {} to {} km; {} cycle censuses over {} to {}\n\
                 {} negatives",
                corpus::MIN_RX_SPOTS_FOR_HEALTHY,
                args.min_km,
                args.max_km,
                args.neg_cycles,
                fmt_day(days[0]),
                fmt_day(days[days.len() - 1]),
                negatives.len(),
            );
            std::fs::write(path, corpus::write_negatives(&negatives, &neg_provenance))
                .map_err(|e| format!("{path}: {e}"))?;
            println!("WROTE    {path} ({} negatives)", negatives.len());
        }
    }

    Ok(())
}

/// Build negatives from whole-cycle censuses spread over the span.
///
/// Cycles are chosen deterministically across days, hours and bands rather than
/// clustered, because Es occurrence and D-region absorption both vary strongly
/// with time of day, and a negatives set drawn from one hour would measure the
/// false-positive rate of one ionosphere.
fn build_negatives(
    args: &Args,
    days: &[(i32, u32, u32)],
    ssn_for_day: &BTreeMap<(i32, u32, u32), Ssn>,
) -> Result<Vec<Negative>, String> {
    println!("\n--- BUILDING NEGATIVES -------------------------------------");
    let mut out = Vec::new();
    let mut fetched = 0usize;
    // One census per (day, hour, band) triple, walked so that consecutive cycles
    // differ in all three.
    let bands = [3.568, 7.038, 14.095, 18.104, 21.094];
    for i in 0..args.neg_cycles as usize {
        let d = days[i % days.len()];
        let hour = SCHEDULE_HOURS[(i * 3) % SCHEDULE_HOURS.len()];
        let band = bands[i % bands.len()];
        // WSPR-2 transmits on even minutes, so an odd minute holds only
        // stragglers and would look like a dead cycle.
        let minute = 2 * ((i as u32 * 7) % 30);
        let cycle = format!("{}-{:02}-{:02} {hour:02}:{minute:02}", d.0, d.1, d.2);
        let tsv = match cycle_census_tsv(&cycle, band) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  ! census for {cycle} at {band} MHz failed: {e}");
                continue;
            }
        };
        let (census, _problems) = parse_spots(&tsv);
        fetched += 1;
        if census.is_empty() {
            println!("  {cycle} {band:>7.3} MHz: empty cycle");
            continue;
        }
        let negs = corpus::negatives_from_cycle(
            &census,
            f64::from(args.min_km),
            f64::from(args.max_km),
            ssn_for_day[&d].value,
            solved_km,
        );
        println!(
            "  {cycle} {band:>7.3} MHz: {} spots -> {} negative(s)",
            census.len(),
            negs.len()
        );
        out.extend(negs);
    }
    println!("  {fetched} cycle(s) censused, {} negative(s) built", out.len());
    Ok(out)
}

fn report_rejections(before: usize, after: usize, rej: &Rejections) {
    println!("\n--- WHAT THE LOCAL CUTS REMOVED ----------------------------");
    println!("  raw rows                         {before}");
    println!(
        "  duplicate observations           {:>8}  same TX/RX/cycle/band more than once",
        rej.duplicate_spots
    );
    println!(
        "  archive distance disagreed       {:>8}  beyond {:.0} % of the solver's great circle",
        rej.distance_mismatch,
        100.0 * corpus::MAX_DISTANCE_MISMATCH
    );
    for (tx, rx, archive, solved) in &rej.worst_mismatches {
        println!("       {tx} -> {rx}: archive {archive:.0} km, solved {solved:.0} km");
    }
    println!(
        "  station seen < {MIN_SPOTS_PER_STATION} times          {:>8}  its fixed effect would be \
         unidentified",
        rej.station_too_rare
    );
    println!("  kept                             {after:>8}");
    if before > 0 {
        #[allow(clippy::cast_precision_loss)]
        let frac = 100.0 * after as f64 / before as f64;
        println!("  ({frac:.0} % of what was fetched survived; discarding a lot is correct here)");
    }
}

/// What the corpus actually contains, as opposed to what was asked for.
fn describe_corpus(spots: &[CorpusSpot]) {
    println!("\n--- WHAT THE CORPUS CONTAINS -------------------------------");
    println!(
        "  {} spots, {} transmitters, {} receivers",
        spots.len(),
        distinct(spots, |s| &s.spot.tx_call),
        distinct(spots, |s| &s.spot.rx_call)
    );

    let mut by_band: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_hour: BTreeMap<u32, usize> = BTreeMap::new();
    let mut by_day: BTreeMap<(i32, u32, u32), usize> = BTreeMap::new();
    for s in spots {
        *by_band
            .entry(skipzone_app::wspr_report::band_label(s.spot.freq_mhz).to_string())
            .or_default() += 1;
        *by_hour.entry(s.spot.timestamp.3).or_default() += 1;
        *by_day.entry(s.date()).or_default() += 1;
    }
    println!("\n  by band:");
    for (band, n) in &by_band {
        println!("    {band:<8} {n:>6}");
    }
    println!("\n  by UTC hour:");
    for (h, n) in &by_hour {
        println!("    {h:02}:00    {n:>6}");
    }
    println!("\n  by day:");
    for (d, n) in &by_day {
        println!("    {}  {n:>6}", fmt_day(*d));
    }

    // The reciprocal and multi-receiver structure the fixed-effects design needs.
    let mut pairs: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for s in spots {
        *pairs
            .entry((s.spot.tx_call.as_str(), s.spot.rx_call.as_str()))
            .or_default() += 1;
    }
    let reciprocal = pairs
        .keys()
        .filter(|(a, b)| pairs.contains_key(&(*b, *a)))
        .count();
    let mut cycle_tx: BTreeMap<(String, String), usize> = BTreeMap::new();
    for s in spots {
        *cycle_tx
            .entry((s.cycle(), s.spot.tx_call.clone()))
            .or_default() += 1;
    }
    let multi_rx = cycle_tx.values().filter(|&&n| n > 1).count();
    let mut pair_bands: BTreeMap<(&str, &str, String), usize> = BTreeMap::new();
    for s in spots {
        *pair_bands
            .entry((
                s.spot.tx_call.as_str(),
                s.spot.rx_call.as_str(),
                s.cycle(),
            ))
            .or_default() += 1;
    }
    println!("\n  structure the fixed-effects design relies on:");
    println!(
        "    distinct TX->RX pairs                          {:>6}",
        pairs.len()
    );
    println!(
        "    pairs that are reciprocal (A hears B and B hears A) {:>6}",
        reciprocal
    );
    println!(
        "    one TX heard by several RX in the same cycle    {:>6}  (isolates TX effect + power)",
        multi_rx
    );
    println!(
        "    one TX->RX pair on several bands in one cycle   {:>6}  (isolates frequency law)",
        pair_bands.values().filter(|&&n| n > 1).count()
    );
}

fn distinct(spots: &[CorpusSpot], key: impl Fn(&CorpusSpot) -> &String) -> usize {
    let mut seen: Vec<&String> = Vec::new();
    for s in spots {
        let k = key(s);
        if !seen.contains(&k) {
            seen.push(k);
        }
    }
    seen.len()
}

/// The consecutive UTC days a span covers.
///
/// Calendar arithmetic done here rather than pulled in as a dependency: the app
/// already has [`skipzone_app::clock`] for the reverse direction, and a span of
/// consecutive days needs only month lengths.
fn day_span(from: &str, days: u32) -> Result<Vec<(i32, u32, u32)>, String> {
    let p: Vec<&str> = from.split('-').collect();
    if p.len() != 3 {
        return Err(format!("--from {from:?} is not YYYY-MM-DD"));
    }
    let mut y: i32 = p[0].parse().map_err(|e| format!("bad year: {e}"))?;
    let mut m: u32 = p[1].parse().map_err(|e| format!("bad month: {e}"))?;
    let mut d: u32 = p[2].parse().map_err(|e| format!("bad day: {e}"))?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return Err(format!("--from {from:?} is out of range"));
    }
    let mut out = Vec::new();
    for _ in 0..days.max(1) {
        out.push((y, m, d));
        d += 1;
        if d > days_in_month(y, m) {
            d = 1;
            m += 1;
            if m > 12 {
                m = 1;
                y += 1;
            }
        }
    }
    Ok(out)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
            if leap { 29 } else { 28 }
        }
    }
}

fn fmt_day(d: (i32, u32, u32)) -> String {
    format!("{:04}-{:02}-{:02}", d.0, d.1, d.2)
}

// `grid` is re-exported through the crate root; referenced so the dependency on
// grid decoding (which `parse_spots` performs) is visible at this call site.
#[allow(unused_imports)]
use grid as _grid_used_by_parse_spots;

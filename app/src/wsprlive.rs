//! Live WSPR spot retrieval from the wspr.live archive.
//!
//! wspr.live exposes the WSPRnet archive as a public ClickHouse endpoint that
//! answers plain SQL over HTTP. That is used directly rather than through a
//! JSON API, for one specific reason: ClickHouse can be asked to format its
//! answer as TSV, and the columns can be ordered to be *exactly* the tab
//! separated layout [`crate::wspr::parse_spots`] already reads. So the network
//! path and the file path go through one parser, with one set of tests, and a
//! fetched run and a saved-file run cannot diverge in how a spot is read.
//!
//! # Ingest lag, and why the window stops short of now
//!
//! WSPR receivers upload in their own time. A spot decoded at 12:34 may reach
//! the database seconds later or minutes later, so the most recent few minutes
//! of the archive are always PARTIALLY populated - present, but missing an
//! unknown fraction of the stations that will eventually report. Sampling that
//! region silently biases a validation run: the spots that arrive quickest are
//! not a random subset of all spots (they skew towards well-connected,
//! well-run, often better-equipped stations).
//!
//! So a window never runs up to the present. It ends [`INGEST_LAG_MINUTES`]
//! before it, and [`Window::describe`] states the lag it applied so the report
//! can show it rather than the operator having to know. `--at` exists for the
//! same reason from the other direction: scoring a specific past instant needs
//! no lag allowance at all, because that data settled long ago.

use crate::net::{self, NetError};

/// Base endpoint. `db1` is the primary public replica.
const ENDPOINT: &str = "https://db1.wspr.live/";

/// How far short of the present a live window stops, minutes.
///
/// WSPR transmits on even-minute boundaries in two-minute cycles, so a decode
/// is available to upload at best two minutes after the transmission started.
/// Measured against the live archive, a cycle 2.5 minutes old held about a
/// third of the spots it eventually held, and one 4.5 minutes old was
/// indistinguishable from the settled ones:
///
/// ```text
///   20:24  13024   settled
///   20:26  13033   settled
///   20:28   4020   2.5 min old - 31 % arrived
///   20:30      5   0.5 min old
/// ```
///
/// Ten minutes is therefore a generous margin rather than a guess. It is still
/// only a default: [`Query::completeness`] measures the actual fill of the
/// window that was sampled, so a run reports what it got rather than trusting
/// this number.
pub const INGEST_LAG_MINUTES: u32 = 10;

/// The slice of the archive a run scores.
#[derive(Clone, Debug)]
pub enum Window {
    /// The most recent settled `minutes`, ending `INGEST_LAG_MINUTES` ago.
    Recent { minutes: u32 },
    /// A fixed instant, `minutes` wide, centred on `YYYY-MM-DD HH:MM` UTC.
    At { utc: String, minutes: u32 },
    /// A whole UTC day, `YYYY-MM-DD`. Used when counting which stations were
    /// busiest over a span, where a 20-minute sample would rank stations by luck.
    Day { utc_date: String },
}

impl Window {
    /// The SQL time predicate for this window.
    fn predicate(&self) -> String {
        match self {
            Self::Recent { minutes } => format!(
                "time >= now() - INTERVAL {} MINUTE AND time < now() - INTERVAL {} MINUTE",
                minutes + INGEST_LAG_MINUTES,
                INGEST_LAG_MINUTES
            ),
            Self::At { utc, minutes } => format!(
                "time >= toDateTime('{utc}:00') - INTERVAL {half} MINUTE \
                 AND time < toDateTime('{utc}:00') + INTERVAL {half} MINUTE",
                half = minutes.div_ceil(2)
            ),
            Self::Day { utc_date } => format!(
                "time >= toDateTime('{utc_date} 00:00:00') \
                 AND time < toDateTime('{utc_date} 00:00:00') + INTERVAL 1 DAY"
            ),
        }
    }

    /// A sentence the report can print, saying exactly what was sampled and
    /// what allowance was made for spots still arriving.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Recent { minutes } => format!(
                "the {minutes} minutes ending {INGEST_LAG_MINUTES} minutes ago \
                 (the last {INGEST_LAG_MINUTES} minutes are excluded: receivers upload on their \
                 own schedule, so the newest part of the archive is only partly filled and \
                 sampling it would favour the stations that report fastest)"
            ),
            Self::At { utc, minutes } => format!(
                "{minutes} minutes centred on {utc} UTC (a settled past window; no ingest-lag \
                 allowance is needed)"
            ),
            Self::Day { utc_date } => {
                format!("the whole UTC day {utc_date} (settled; no ingest-lag allowance is needed)")
            }
        }
    }
}

/// What to ask the archive for.
#[derive(Clone, Debug)]
pub struct Query {
    pub window: Window,
    /// Only spots at least this far apart, km. Very short spots are dominated
    /// by ground wave and near-vertical incidence, which is not what a
    /// skip-distance model is being asked about.
    pub min_km: u32,
    pub max_km: u32,
    /// Band centre in MHz, or `None` for every band.
    pub band_mhz: Option<f64>,
    /// Upper frequency limit, MHz. Defaults to [`HF_TOP_MHZ`].
    ///
    /// This exists to keep 6 m out of a default run, and it is a statement
    /// about SCOPE rather than a convenience. Above about 30 MHz the layers
    /// this model carries have no reflection to offer: an F2 critical frequency
    /// of 10-15 MHz gives an oblique MUF that runs out well below 50 MHz, so
    /// every 6 m opening in the archive is sporadic E, meteor scatter, TEP or
    /// aircraft scatter - three of which are not modelled here at all. Scoring
    /// them counts as a model failure something the model never claimed, and it
    /// spends the random sample's budget doing it.
    ///
    /// Raise it deliberately (`--max-mhz 60`) to look at how the Es stack does
    /// up there; that is a real question, just a different one.
    pub max_mhz: f64,
    /// Row cap. Every row costs a full solve, so this is the run's real budget.
    pub limit: u32,
}

/// Top of HF, MHz: the highest frequency this model claims a mechanism for.
/// Covers all of 10 m (28-29.7 MHz).
pub const HF_TOP_MHZ: f64 = 30.0;

impl Default for Query {
    fn default() -> Self {
        Self {
            window: Window::Recent { minutes: 10 },
            min_km: 300,
            max_km: 20_000,
            band_mhz: None,
            max_mhz: HF_TOP_MHZ,
            limit: 200,
        }
    }
}

impl Query {
    /// The frequency ceiling actually applied, MHz.
    ///
    /// An explicit `--band` wins over the default ceiling: asking for 6 m and
    /// then silently getting nothing back because of a limit the caller did not
    /// set would be the worst of both. A band above the ceiling raises it to
    /// cover that band, and [`Self::describe_filter`] says so.
    #[must_use]
    pub fn effective_max_mhz(&self) -> f64 {
        match self.band_mhz {
            Some(b) if b + 0.1 > self.max_mhz => b + 0.1,
            _ => self.max_mhz,
        }
    }

    /// One line naming every filter in force, for the report to print.
    #[must_use]
    pub fn describe_filter(&self) -> String {
        let mut s = format!("{} to {} km", self.min_km, self.max_km);
        match self.band_mhz {
            Some(m) => {
                s.push_str(&format!(", {} only", crate::wspr_report::band_label(m)));
                if m + 0.1 > self.max_mhz {
                    s.push_str(" (explicitly asked for, so the HF ceiling is lifted)");
                }
            }
            None => {
                s.push_str(&format!(
                    ", at or below {:.0} MHz (above that this model has no \
                     mechanism to offer, so 6 m and up are out of scope)",
                    self.effective_max_mhz()
                ));
            }
        }
        s.push_str(&format!(", random sample of up to {}", self.limit));
        s
    }

    /// The SQL sent to the archive.
    ///
    /// The projection is pinned to the nine columns `parse_spots` expects, in
    /// its order. The time format uses `%i` for minutes, not `%M`: in
    /// ClickHouse `%M` is the full month name, and using it yields timestamps
    /// like `2026-07-27 20:July` that the spot parser rejects outright.
    ///
    /// `code >= 0` drops a small number of rows the archive carries with -1,
    /// which is not one of the WSPR message types. The message type itself is
    /// deliberately NOT filtered on: types 1, 2 and 3 are the plain, compound
    /// and hashed-callsign forms of the same two-minute transmission, and all
    /// three carry an SNR on the same scale. (An earlier version of this filter
    /// asked for `code = 0`, which no row has, and quietly returned nothing.)
    ///
    /// The frequency ceiling is applied HERE rather than after fetching, so
    /// out-of-scope rows never compete for the random sample's budget.
    ///
    /// Sampling is by `rand()` rather than by taking the first N rows: the
    /// archive returns rows in insertion order, so a plain `LIMIT` would score
    /// whichever receivers happened to upload first - the same bias the ingest
    /// lag guards against, reintroduced through the back door.
    #[must_use]
    pub fn sql(&self) -> String {
        // Rounded, not truncated: `(50.294 - 0.1) * 1e6` lands a whisker below
        // 50 194 000 in binary floating point, and a bare cast would put the
        // band edge a hertz low.
        let hz = |mhz: f64| (mhz * 1e6).round() as i64;
        let band = match self.band_mhz {
            Some(mhz) => format!(
                " AND frequency BETWEEN {} AND {}",
                hz(mhz - 0.1),
                hz(mhz + 0.1)
            ),
            None => String::new(),
        };
        format!(
            "SELECT formatDateTime(time, '%Y-%m-%d %H:%i') AS ts, tx_sign, \
             round(frequency / 1000000, 6) AS mhz, snr, tx_loc, power, rx_sign, rx_loc, distance \
             FROM wspr.rx \
             WHERE {} AND code >= 0 AND distance BETWEEN {} AND {} \
             AND frequency <= {}{} \
             AND tx_loc != '' AND rx_loc != '' \
             ORDER BY rand() LIMIT {} FORMAT TSV",
            self.window.predicate(),
            self.min_km,
            self.max_km,
            (self.effective_max_mhz() * 1e6).round() as i64,
            band,
            self.limit
        )
    }

    /// How many spots the frequency ceiling removed from the window, and how
    /// many survived it.
    ///
    /// Excluding 6 m is a scope decision, and a scope decision that hides how
    /// much it excluded is just a silent filter. This counts both sides so the
    /// report can state the size of what it declined to score.
    ///
    /// # Errors
    /// Transport failures, and any error the archive reports.
    pub fn excluded_above_ceiling(&self) -> Result<(u64, u64), NetError> {
        let sql = format!(
            "SELECT countIf(frequency > {ceiling}) AS above, countIf(frequency <= {ceiling}) AS below \
             FROM wspr.rx WHERE {} AND code >= 0 AND distance BETWEEN {} AND {} FORMAT TSV",
            self.window.predicate(),
            self.min_km,
            self.max_km,
            ceiling = (self.effective_max_mhz() * 1e6).round() as i64,
        );
        let url = format!("{ENDPOINT}?query={}", urlencode(&sql));
        let body = net::get_text(&url)?;
        let (a, b) = body
            .trim()
            .split_once('\t')
            .ok_or_else(|| NetError::Data(format!("unexpected count reply: {body:?}")))?;
        Ok((a.trim().parse().unwrap_or(0), b.trim().parse().unwrap_or(0)))
    }

    /// Fetch the spots as the tab-separated text `parse_spots` reads.
    ///
    /// # Errors
    /// Transport failures, and any error the archive reports for the query.
    pub fn fetch_tsv(&self) -> Result<String, NetError> {
        let url = format!("{ENDPOINT}?query={}", urlencode(&self.sql()));
        let body = net::get_text(&url)?;
        // ClickHouse reports query errors in the body with a 200 in some
        // configurations, so a body that is an error message rather than rows
        // has to be caught here or it would parse as zero spots.
        if body.starts_with("Code:") || body.contains("DB::Exception") {
            return Err(NetError::Data(format!(
                "wspr.live rejected the query: {}",
                body.lines().next().unwrap_or(&body)
            )));
        }
        Ok(body)
    }
}

/// How full the sampled window was, against cycles old enough to have settled.
///
/// The ingest lag is a default, and a default is an assumption. This measures
/// the thing the assumption is about: it counts spots per two-minute cycle
/// inside the window, and compares them with the median cycle over a longer
/// span reaching further back. A window at 100 % was fully populated when it
/// was read; one at 40 % was read too early, and every statistic computed from
/// it is drawn from whichever receivers upload fastest.
#[derive(Clone, Debug)]
pub struct Completeness {
    /// Median spots per cycle inside the sampled window.
    pub window_median: u64,
    /// Median spots per cycle over the settled reference span.
    pub settled_median: u64,
    /// Cycles found inside the window.
    pub window_cycles: usize,
}

impl Completeness {
    /// Window fill as a fraction of a settled cycle. `None` when there is no
    /// reference to compare against.
    #[must_use]
    pub fn fraction(&self) -> Option<f64> {
        (self.settled_median > 0).then(|| {
            #[allow(clippy::cast_precision_loss)]
            let f = self.window_median as f64 / self.settled_median as f64;
            f
        })
    }

    /// Is the window full enough to score without a caveat?
    #[must_use]
    pub fn is_settled(&self) -> bool {
        self.fraction().is_some_and(|f| f >= 0.9)
    }
}

impl Query {
    /// Measure how full the sampled window actually was.
    ///
    /// # Errors
    /// Transport failures, and any error the archive reports.
    pub fn completeness(&self) -> Result<Completeness, NetError> {
        // Only even minutes carry a WSPR-2 cycle; the odd ones hold a handful
        // of stragglers and would drag any median to near zero.
        let sql = "SELECT toUnixTimestamp(time) AS t, count() AS n FROM wspr.rx \
                   WHERE time >= now() - INTERVAL 120 MINUTE AND toMinute(time) % 2 = 0 \
                   GROUP BY time ORDER BY time FORMAT TSV";
        let url = format!("{ENDPOINT}?query={}", urlencode(sql));
        let body = net::get_text(&url)?;
        let rows: Vec<(i64, u64)> = body
            .lines()
            .filter_map(|l| {
                let (a, b) = l.split_once('\t')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .collect();
        if rows.is_empty() {
            return Err(NetError::Data(
                "the archive returned no cycle counts to measure completeness against".into(),
            ));
        }
        // The window is the most recent `minutes` before the lag; the settled
        // reference is everything older than twice the lag.
        let newest = rows.iter().map(|(t, _)| *t).max().unwrap_or(0);
        let lag = i64::from(INGEST_LAG_MINUTES) * 60;
        let width = i64::from(match &self.window {
            Window::Recent { minutes } | Window::At { minutes, .. } => *minutes,
            // A whole day, in minutes. `completeness` is only meaningful for a
            // live window anyway; a settled day needs no lag allowance.
            Window::Day { .. } => 24 * 60,
        }) * 60;
        let win_hi = newest - lag;
        let win_lo = win_hi - width;
        let median = |mut v: Vec<u64>| -> u64 {
            if v.is_empty() {
                return 0;
            }
            v.sort_unstable();
            v[v.len() / 2]
        };
        let in_window: Vec<u64> = rows
            .iter()
            .filter(|(t, _)| *t > win_lo && *t <= win_hi)
            .map(|(_, n)| *n)
            .collect();
        let settled: Vec<u64> = rows
            .iter()
            .filter(|(t, _)| *t <= newest - 2 * lag)
            .map(|(_, n)| *n)
            .collect();
        Ok(Completeness {
            window_cycles: in_window.len(),
            window_median: median(in_window),
            settled_median: median(settled),
        })
    }
}

// --- Calibration corpus ---------------------------------------------------
//
// The live [`Query`] above is right for SCORING and wrong for FITTING, in two
// specific ways that both matter enough to justify a second query type rather
// than a flag on the first.
//
// * `ORDER BY rand()` returns a different sample every run. Fitting against that
//   is optimising against a moving target: a parameter change and a resample are
//   indistinguishable in the result.
// * `code >= 0` keeps message types 2 and 3, the compound and hashed-callsign
//   forms. That is right for scoring - they carry an SNR on the same scale - and
//   wrong here, because a two-way fixed-effects model needs to know WHICH
//   station a spot came from, and a hashed callsign is ambiguous by construction.
//
// A corpus query therefore differs from a scoring query in being reproducible
// and in insisting on identifiable stations and locatable endpoints.

/// Lowest claimed transmit power to accept, dBm. Below 0 dBm (1 mW) the claim is
/// almost always a mis-set radio rather than a real QRP station.
pub const MIN_PLAUSIBLE_TX_DBM: i32 = 0;
/// Highest claimed transmit power to accept, dBm. 47 dBm is 50 W, well above any
/// legitimate WSPR level; above it the field is being misused.
pub const MAX_PLAUSIBLE_TX_DBM: i32 = 47;

/// The WSPR message type a fixed-effects fit can use.
///
/// Type 1 is `callsign + 4-character grid + power`. Types 2 and 3 carry compound
/// and hashed callsigns respectively; a hash does not identify a station
/// uniquely, so a station effect estimated across hashed rows is estimated
/// across an unknown mixture of stations.
pub const IDENTIFIABLE_MESSAGE_CODE: i32 = 1;

/// A reproducible, hygiene-filtered query for one time window.
///
/// Sampling is by `cityHash64` of the row's identity rather than by `rand()`:
/// still a uniform sample uncorrelated with upload order, but the SAME sample
/// every time the query is issued, which is what makes a fit against it
/// meaningful. `salt` varies it deliberately when a different draw is wanted.
#[derive(Clone, Debug)]
pub struct CorpusQuery {
    pub window: Window,
    pub min_km: u32,
    pub max_km: u32,
    pub max_mhz: f64,
    pub limit: u32,
    /// Restrict to one band centre in MHz, or `None` for every band.
    ///
    /// A whole-archive sample follows real band activity, which loads 20 m and
    /// 40 m and leaves 160 m and 10 m almost unrepresented. Targeting a band
    /// explicitly is how the corpus gets the frequency spread that identifies the
    /// absorption law; without it the frequency dependence would be fitted from
    /// two bands.
    pub band_mhz: Option<f64>,
    /// Restrict both ends to the busiest stations, or `None` for any station.
    pub busiest: Option<BusiestFilter>,
    /// Changes which rows the deterministic sample selects, without making the
    /// selection unpredictable. Same salt, same rows, always.
    pub salt: u32,
}

/// Restrict a corpus to the busiest stations at each end, ranked over one
/// reference day.
///
/// # Why a corpus has to be restricted to busy stations
///
/// A two-way fixed-effects model needs enough spots per station to pin that
/// station's effect - see [`crate::corpus::MIN_SPOTS_PER_STATION`]. WSPR has
/// thousands of active stations, so a uniform sample of a few thousand spots
/// gives almost every one of them two or three rows, and then EVERY station
/// effect is unidentified and the whole corpus is unusable. Measured against this
/// archive: 224 uniformly-sampled spots spanned 173 stations, and requiring ten
/// spots per station removed all 224.
///
/// Concentrating on the busiest stations fixes that, and the cost has to be
/// stated rather than absorbed: busy stations are better sited, better equipped
/// and electrically quieter than the WSPR population as a whole. So the station
/// effects this yields describe the ACTIVE CORE of the network, not its median
/// member, and the physics identified from within-station variation is identified
/// over that subpopulation.
///
/// # Why a subquery rather than a list of callsigns
///
/// Both. The ranking is available as a list through [`busiest_stations`], which
/// is what the report prints. But putting 500 callsigns into the data query's
/// `IN` clause produced a URL the endpoint rejected with HTTP 414, so the query
/// itself carries the ranking as a SUBQUERY over the reference day. Same
/// selection, and it stays the same selection however many stations are asked
/// for.
#[derive(Clone, Debug)]
pub struct BusiestFilter {
    /// UTC date the ranking is computed over, `YYYY-MM-DD`.
    ///
    /// ONE day for the whole corpus, deliberately: ranking per window would let
    /// the set of stations drift between windows, and a station effect estimated
    /// over a drifting membership is not a fixed effect.
    pub census_date: String,
    pub top_tx: u32,
    pub top_rx: u32,
}

/// Which end of a path a station census counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StationEnd {
    Transmitter,
    Receiver,
}

impl StationEnd {
    fn column(self) -> &'static str {
        match self {
            Self::Transmitter => "tx_sign",
            Self::Receiver => "rx_sign",
        }
    }
}

impl CorpusQuery {
    /// The band window, or nothing when every band is wanted.
    fn band_predicate(&self) -> String {
        match self.band_mhz {
            Some(mhz) => format!(
                " AND frequency BETWEEN {} AND {}",
                ((mhz - 0.1) * 1e6).round() as i64,
                ((mhz + 0.1) * 1e6).round() as i64
            ),
            None => String::new(),
        }
    }

    /// `AND <col> IN (<the busiest N at that end over the reference day>)`, or
    /// nothing when the corpus is not restricted.
    ///
    /// The subquery repeats the hygiene and range predicates, so a station is
    /// ranked by the spots it produced that this corpus would ACCEPT, not by its
    /// total traffic. A receiver whose volume is all sub-300 km ground wave is
    /// not a busy station for these purposes.
    fn busiest_predicate(&self, end: StationEnd) -> String {
        let Some(b) = &self.busiest else {
            return String::new();
        };
        let (col, limit) = match end {
            StationEnd::Transmitter => ("tx_sign", b.top_tx),
            StationEnd::Receiver => ("rx_sign", b.top_rx),
        };
        if limit == 0 {
            return String::new();
        }
        format!(
            " AND {col} IN (SELECT {col} FROM wspr.rx WHERE {} AND {} \
             AND distance BETWEEN {} AND {} AND frequency <= {} \
             GROUP BY {col} ORDER BY count() DESC, {col} ASC LIMIT {limit})",
            Window::Day {
                utc_date: b.census_date.clone()
            }
            .predicate(),
            self.hygiene_sql(),
            self.min_km,
            self.max_km,
            (self.max_mhz * 1e6).round() as i64,
        )
    }

    /// The hygiene predicates, shared with the cycle census so the negatives are
    /// drawn from the same population as the positives. A negative built from a
    /// station the positives filter out would not be a comparable observation.
    fn hygiene_sql(&self) -> String {
        format!(
            "code = {IDENTIFIABLE_MESSAGE_CODE} \
             AND length(tx_loc) >= 6 AND length(rx_loc) >= 6 \
             AND power >= {MIN_PLAUSIBLE_TX_DBM} AND power <= {MAX_PLAUSIBLE_TX_DBM}"
        )
    }

    /// One line naming every filter in force, for the corpus file's provenance
    /// header. A saved corpus that does not record how it was selected cannot be
    /// interpreted later.
    #[must_use]
    pub fn describe_filter(&self) -> String {
        format!(
            "{} to {} km, at or below {:.0} MHz, WSPR message code {} only \
             (plain callsigns: types 2 and 3 are compound and hashed, and a hashed \
             call cannot carry a station effect), 6-character grids at BOTH ends \
             (a 4-character grid is +/-70 km, which on a 400 km path is a 17 % error \
             in the very quantity being fitted), claimed power {} to {} dBm, \
             deterministic sample of up to {} by cityHash64 with salt {}{}",
            self.min_km,
            self.max_km,
            self.max_mhz,
            IDENTIFIABLE_MESSAGE_CODE,
            MIN_PLAUSIBLE_TX_DBM,
            MAX_PLAUSIBLE_TX_DBM,
            self.limit,
            self.salt,
            match &self.busiest {
                Some(b) => format!(
                    ", restricted to the {} busiest transmitters and {} busiest receivers                      ranked over {} - NOT a representative sample of WSPR stations, but the                      only way a per-station fixed effect is identified at all",
                    b.top_tx, b.top_rx, b.census_date
                ),
                None => String::new(),
            }
        )
    }

    /// The SQL. Projection pinned to the nine columns
    /// [`crate::wspr::parse_spots`] reads, in its order, exactly as [`Query`]
    /// does - so a corpus file and a live fetch go through one parser.
    #[must_use]
    pub fn sql(&self) -> String {
        format!(
            "SELECT formatDateTime(time, '%Y-%m-%d %H:%i') AS ts, tx_sign, \
             round(frequency / 1000000, 6) AS mhz, snr, tx_loc, power, rx_sign, rx_loc, distance \
             FROM wspr.rx \
             WHERE {} AND {} AND distance BETWEEN {} AND {} AND frequency <= {}{}{}{} \
             ORDER BY cityHash64(concat(toString(time), tx_sign, rx_sign, \
             toString(frequency), toString({}))) LIMIT {} FORMAT TSV",
            self.window.predicate(),
            self.hygiene_sql(),
            self.min_km,
            self.max_km,
            (self.max_mhz * 1e6).round() as i64,
            self.band_predicate(),
            self.busiest_predicate(StationEnd::Transmitter),
            self.busiest_predicate(StationEnd::Receiver),
            self.salt,
            self.limit
        )
    }

    /// Fetch this window's rows as the TSV the spot parser reads.
    ///
    /// # Errors
    /// Transport failures, and any error the archive reports for the query.
    pub fn fetch_tsv(&self) -> Result<String, NetError> {
        fetch_rows(&self.sql())
    }

    /// How many rows each hygiene filter removed from this window, so the corpus
    /// can state the size of what it discarded rather than only what it kept.
    ///
    /// Counted in ONE pass with `countIf`, because six separate queries against a
    /// live archive would each see a slightly different table.
    ///
    /// # Errors
    /// Transport failures, and any error the archive reports.
    pub fn hygiene_census(&self) -> Result<HygieneCensus, NetError> {
        let sql = format!(
            "SELECT count() AS total, \
             countIf(code != {IDENTIFIABLE_MESSAGE_CODE}) AS not_type1, \
             countIf(length(tx_loc) < 6) AS short_tx_grid, \
             countIf(length(rx_loc) < 6) AS short_rx_grid, \
             countIf(power < {MIN_PLAUSIBLE_TX_DBM} OR power > {MAX_PLAUSIBLE_TX_DBM}) AS bad_power, \
             countIf({}) AS kept \
             FROM wspr.rx WHERE {} AND distance BETWEEN {} AND {} AND frequency <= {}{}{}{} \
             FORMAT TSV",
            self.hygiene_sql(),
            self.window.predicate(),
            self.min_km,
            self.max_km,
            (self.max_mhz * 1e6).round() as i64,
            self.band_predicate(),
            self.busiest_predicate(StationEnd::Transmitter),
            self.busiest_predicate(StationEnd::Receiver),
        );
        let body = fetch_rows(&sql)?;
        let f: Vec<u64> = body
            .trim()
            .split('\t')
            .map(|v| v.trim().parse().unwrap_or(0))
            .collect();
        if f.len() < 6 {
            return Err(NetError::Data(format!(
                "unexpected hygiene census reply: {body:?}"
            )));
        }
        Ok(HygieneCensus {
            total: f[0],
            not_type1: f[1],
            short_tx_grid: f[2],
            short_rx_grid: f[3],
            bad_power: f[4],
            kept: f[5],
        })
    }
}

/// What the hygiene filters removed from one window. Counts overlap - a row can
/// fail several tests at once - so they explain rather than partition.
#[derive(Clone, Copy, Debug, Default)]
pub struct HygieneCensus {
    pub total: u64,
    pub not_type1: u64,
    pub short_tx_grid: u64,
    pub short_rx_grid: u64,
    pub bad_power: u64,
    pub kept: u64,
}

impl HygieneCensus {
    /// Accumulate another window's counts into this one.
    pub fn add(&mut self, other: Self) {
        self.total += other.total;
        self.not_type1 += other.not_type1;
        self.short_tx_grid += other.short_tx_grid;
        self.short_rx_grid += other.short_rx_grid;
        self.bad_power += other.bad_power;
        self.kept += other.kept;
    }
}

/// EVERY spot in one two-minute cycle on one band, subject to the same hygiene
/// filters as the corpus.
///
/// This is the query the negatives set is built from, and it is deliberately
/// unfiltered by distance: deciding whether a receiver was healthy in a cycle
/// means counting everything it heard, including the short paths a calibration
/// corpus would not score.
///
/// # Errors
/// Transport failures, and any error the archive reports.
pub fn cycle_census_tsv(utc_cycle: &str, band_mhz: f64) -> Result<String, NetError> {
    let hz = |mhz: f64| (mhz * 1e6).round() as i64;
    let sql = format!(
        "SELECT formatDateTime(time, '%Y-%m-%d %H:%i') AS ts, tx_sign, \
         round(frequency / 1000000, 6) AS mhz, snr, tx_loc, power, rx_sign, rx_loc, distance \
         FROM wspr.rx \
         WHERE time = toDateTime('{utc_cycle}:00') \
         AND code = {IDENTIFIABLE_MESSAGE_CODE} \
         AND length(tx_loc) >= 6 AND length(rx_loc) >= 6 \
         AND power >= {MIN_PLAUSIBLE_TX_DBM} AND power <= {MAX_PLAUSIBLE_TX_DBM} \
         AND frequency BETWEEN {} AND {} FORMAT TSV",
        hz(band_mhz - 0.1),
        hz(band_mhz + 0.1),
    );
    fetch_rows(&sql)
}

/// The busiest stations at one end of a path over a window, most active first.
///
/// This is how a corpus finds the stations whose fixed effects can actually be
/// estimated; see [`CorpusQuery::restrict_tx`] for why that restriction is
/// necessary and what it costs in representativeness.
///
/// # Errors
/// Transport failures, and any error the archive reports.
pub fn busiest_stations(
    window: &Window,
    min_km: u32,
    max_km: u32,
    max_mhz: f64,
    end: StationEnd,
    limit: u32,
) -> Result<Vec<String>, NetError> {
    let sql = format!(
        "SELECT {col}, count() AS n FROM wspr.rx \
         WHERE {} AND code = {IDENTIFIABLE_MESSAGE_CODE} \
         AND length(tx_loc) >= 6 AND length(rx_loc) >= 6 \
         AND power >= {MIN_PLAUSIBLE_TX_DBM} AND power <= {MAX_PLAUSIBLE_TX_DBM} \
         AND distance BETWEEN {min_km} AND {max_km} AND frequency <= {} \
         GROUP BY {col} ORDER BY n DESC, {col} ASC LIMIT {limit} FORMAT TSV",
        window.predicate(),
        (max_mhz * 1e6).round() as i64,
        col = end.column(),
    );
    let body = fetch_rows(&sql)?;
    Ok(body
        .lines()
        .filter_map(|l| l.split('\t').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// Issue one query and reject a body that is a ClickHouse error rather than rows.
fn fetch_rows(sql: &str) -> Result<String, NetError> {
    let url = format!("{ENDPOINT}?query={}", urlencode(sql));
    let body = net::get_text(&url)?;
    if body.starts_with("Code:") || body.contains("DB::Exception") {
        return Err(NetError::Data(format!(
            "wspr.live rejected the query: {}",
            body.lines().next().unwrap_or(&body)
        )));
    }
    Ok(body)
}

/// Percent-encode everything that is not unreserved, so a query containing
/// spaces, quotes, commas and parentheses survives being put in a URL.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push('%');
            out.push_str(&format!("{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A live window must stop short of the present, and must express that as
    /// two bounds rather than one: `time >= start` alone would run to now.
    #[test]
    fn recent_window_excludes_the_unsettled_tail() {
        let p = Window::Recent { minutes: 30 }.predicate();
        assert!(
            p.contains(&format!("INTERVAL {} MINUTE", 30 + INGEST_LAG_MINUTES)),
            "{p}"
        );
        assert!(
            p.contains(&format!(
                "time < now() - INTERVAL {INGEST_LAG_MINUTES} MINUTE"
            )),
            "the newest {INGEST_LAG_MINUTES} minutes must be excluded: {p}"
        );
        assert!(Window::Recent { minutes: 30 }.describe().contains("upload"));
    }

    /// A past window is centred on the instant asked for and needs no lag.
    #[test]
    fn at_window_is_centred_and_needs_no_lag() {
        let p = Window::At {
            utc: "2026-07-24 03:22".into(),
            minutes: 20,
        }
        .predicate();
        assert!(p.contains("- INTERVAL 10 MINUTE"), "{p}");
        assert!(p.contains("+ INTERVAL 10 MINUTE"), "{p}");
        assert!(
            !p.contains("now()"),
            "a fixed window must not depend on now: {p}"
        );
    }

    /// The projection has to match what `parse_spots` reads, in order, or the
    /// two halves of the harness silently disagree.
    #[test]
    fn projection_matches_the_spot_parser() {
        let sql = Query::default().sql();
        let select = sql
            .split_once("SELECT ")
            .and_then(|(_, r)| r.split_once(" FROM "))
            .expect("a SELECT ... FROM")
            .0;
        let cols = top_level_columns(select);
        assert_eq!(cols.len(), 9, "parse_spots wants 9 fields: {cols:?}");
        assert!(cols[0].contains("formatDateTime"), "{:?}", cols[0]);
        assert!(cols[1].contains("tx_sign"));
        assert!(cols[2].contains("frequency"));
        assert!(cols[3].contains("snr"));
        assert!(cols[4].contains("tx_loc"));
        assert!(cols[5].contains("power"));
        assert!(cols[6].contains("rx_sign"));
        assert!(cols[7].contains("rx_loc"));
        assert!(cols[8].contains("distance"));
        assert!(sql.ends_with("FORMAT TSV"), "{sql}");
        // Real message types only, and a random sample rather than whichever
        // receivers uploaded first.
        assert!(sql.contains("code >= 0"), "{sql}");
        assert!(sql.contains("ORDER BY rand()"), "{sql}");
    }

    /// A row the parser can read must come out of the projection's shape. This
    /// is the contract between the two modules, pinned on a literal row in the
    /// format ClickHouse emits for it.
    #[test]
    fn a_clickhouse_row_parses_as_a_spot() {
        let row = "2026-07-27 03:22\tK1ABC\t14.097100\t-18\tFN42\t37\tW9XYZ\tEM48\t1420";
        let (spots, problems) = crate::wspr::parse_spots(row);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(spots.len(), 1);
        let s = &spots[0];
        assert_eq!(s.tx_call, "K1ABC");
        assert!((s.freq_mhz - 14.0971).abs() < 1e-9);
        assert!((s.snr_db + 18.0).abs() < 1e-9);
        assert!((s.reported_km - 1420.0).abs() < 1e-9);
        assert_eq!(s.timestamp, (2026, 7, 27, 3, 22));
    }

    /// Split a SELECT list on its top-level commas only. `formatDateTime(time,
    /// '...')` contains a comma that does not separate columns, so a naive
    /// split counts eleven columns where there are nine.
    fn top_level_columns(select: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut depth = 0i32;
        let mut current = String::new();
        for c in select.chars() {
            match c {
                '(' => {
                    depth += 1;
                    current.push(c);
                }
                ')' => {
                    depth -= 1;
                    current.push(c);
                }
                ',' if depth == 0 => {
                    out.push(current.trim().to_string());
                    current.clear();
                }
                _ => current.push(c),
            }
        }
        if !current.trim().is_empty() {
            out.push(current.trim().to_string());
        }
        out
    }

    #[test]
    fn top_level_split_ignores_commas_inside_calls() {
        let cols = top_level_columns("a, f(x, 'y'), b");
        assert_eq!(cols, vec!["a", "f(x, 'y')", "b"]);
    }

    /// A fill fraction below the bar has to read as unsettled, and a window
    /// with no reference to compare against must not claim to be settled.
    #[test]
    fn completeness_flags_an_underfilled_window() {
        let full = Completeness {
            window_median: 12_900,
            settled_median: 13_000,
            window_cycles: 10,
        };
        assert!(full.is_settled());
        assert!((full.fraction().unwrap() - 0.992).abs() < 0.01);

        let partial = Completeness {
            window_median: 4_020,
            settled_median: 13_000,
            window_cycles: 10,
        };
        assert!(!partial.is_settled(), "31 % full is not settled");

        let no_reference = Completeness {
            window_median: 100,
            settled_median: 0,
            window_cycles: 1,
        };
        assert!(no_reference.fraction().is_none());
        assert!(!no_reference.is_settled(), "unknown is not settled");
    }

    /// A default run must not fetch 6 m at all: the ceiling has to be in the
    /// SQL, so those rows never compete for the random sample's budget.
    #[test]
    fn default_query_excludes_six_metres() {
        let q = Query::default();
        assert!((q.effective_max_mhz() - HF_TOP_MHZ).abs() < 1e-9);
        // 50.294 MHz, the 6 m WSPR frequency, is above the ceiling.
        assert!(q.sql().contains("frequency <= 30000000"), "{}", q.sql());
        assert!(50.294 * 1e6 > q.effective_max_mhz() * 1e6);
        // All of 10 m survives it.
        assert!(29.7 * 1e6 <= q.effective_max_mhz() * 1e6);
        assert!(q.describe_filter().contains("30 MHz"));
    }

    /// Asking for 6 m explicitly must WORK, not silently return nothing
    /// because of a ceiling the caller never set.
    #[test]
    fn an_explicit_band_lifts_the_ceiling() {
        let q = Query {
            band_mhz: Some(50.294),
            ..Query::default()
        };
        assert!(q.effective_max_mhz() > 50.294);
        let sql = q.sql();
        assert!(sql.contains("frequency <= 50394000"), "{sql}");
        // And the band window itself is still applied.
        assert!(
            sql.contains("frequency BETWEEN 50194000 AND 50394000"),
            "{sql}"
        );
        assert!(
            q.describe_filter().contains("ceiling is lifted"),
            "{}",
            q.describe_filter()
        );
    }

    /// A band inside HF leaves the ceiling where it is.
    #[test]
    fn an_hf_band_does_not_lift_the_ceiling() {
        let q = Query {
            band_mhz: Some(14.0),
            ..Query::default()
        };
        assert!((q.effective_max_mhz() - HF_TOP_MHZ).abs() < 1e-9);
        assert!(!q.describe_filter().contains("lifted"));
    }

    #[test]
    fn urlencoding_covers_sql_punctuation() {
        assert_eq!(urlencode("a b"), "a%20b");
        assert_eq!(urlencode("'x'"), "%27x%27");
        assert_eq!(urlencode("a,b(c)"), "a%2Cb%28c%29");
        assert_eq!(urlencode("safe-_.~"), "safe-_.~");
    }
}

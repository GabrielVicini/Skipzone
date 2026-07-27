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
        Ok((
            a.trim().parse().unwrap_or(0),
            b.trim().parse().unwrap_or(0),
        ))
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
            p.contains(&format!("time < now() - INTERVAL {INGEST_LAG_MINUTES} MINUTE")),
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
        assert!(!p.contains("now()"), "a fixed window must not depend on now: {p}");
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
        assert!(sql.contains("frequency BETWEEN 50194000 AND 50394000"), "{sql}");
        assert!(q.describe_filter().contains("ceiling is lifted"), "{}", q.describe_filter());
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

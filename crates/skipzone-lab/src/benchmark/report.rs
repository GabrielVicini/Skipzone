//! The run's files: `results.tsv` (what a PR diffs), `rows.tsv` (per-row
//! scores, for the paired comparison) and `report.md`
//! (docs/notes/accuracy-roadmap.md §2.6).

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::bootstrap;
use super::metrics::{self, METRICS, Scored};
use super::pipeline::{Corpus, ModelRows, P533, Provenance, ResultRow, RunConfig, SKIPZONE};
use super::split::SplitKind;

/// Fewer rows than this and a value is flagged in the report, the same
/// floor the other lab harnesses quote at.
pub const MIN_QUOTABLE: usize = 30;

const RESULTS_HEADER: &str =
    "model\tcorpus\tsplit\tcut\tmetric\tvalue\tlo\thi\tinterval\tn\tmanifest_sha256";

fn num(v: f64) -> String {
    if v.is_finite() {
        format!("{v:.6}")
    } else {
        "NaN".to_string()
    }
}

fn parse_num(s: &str) -> f64 {
    s.parse().unwrap_or(f64::NAN)
}

/// Write `results.tsv`. A fixture run says so in its first line.
#[must_use]
pub fn results_tsv(rows: &[ResultRow], synthetic: bool) -> String {
    let mut s = String::new();
    if synthetic {
        let _ = writeln!(
            s,
            "# FIXTURE RUN: synthetic corpora. These numbers only test the harness."
        );
    }
    let _ = writeln!(s, "{RESULTS_HEADER}");
    for r in rows {
        let _ = writeln!(
            s,
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            r.model,
            r.corpus,
            r.split,
            r.cut,
            r.metric,
            num(r.value),
            num(r.lo),
            num(r.hi),
            r.interval,
            r.n,
            r.manifest_sha
        );
    }
    s
}

/// Read back what [`results_tsv`] wrote.
///
/// # Errors
/// A row with the wrong number of fields.
pub fn parse_results_tsv(text: &str) -> Result<Vec<ResultRow>, String> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.starts_with('#') || line.trim().is_empty() || line == RESULTS_HEADER {
            continue;
        }
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 11 {
            return Err(format!("results.tsv line {}: expected 11 fields", i + 1));
        }
        out.push(ResultRow {
            model: f[0].to_string(),
            corpus: f[1].to_string(),
            split: f[2].to_string(),
            cut: f[3].to_string(),
            metric: f[4].to_string(),
            value: parse_num(f[5]),
            lo: parse_num(f[6]),
            hi: parse_num(f[7]),
            interval: f[8].to_string(),
            n: f[9].parse().unwrap_or(0),
            manifest_sha: f[10].to_string(),
        });
    }
    Ok(out)
}

const ROWS_HEADER: &str = "model\tcorpus\tsplit\tid\tday\tcuts\tmeasured_db\tmodel_db\teffect_db\t\
p_decode\tdecoded\tdecodable\tfof2_model\tfof2_measured\thmf2_model\thmf2_measured\t\
muf_model\tmuf_measured";

fn opt(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |x| format!("{x:.6}"))
}

fn opt_bool(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "1",
        Some(false) => "0",
        None => "-",
    }
}

fn split_from(label: &str) -> Option<SplitKind> {
    [
        SplitKind::All,
        SplitKind::Month,
        SplitKind::Region,
        SplitKind::MonthRegion,
        SplitKind::Station,
    ]
    .into_iter()
    .find(|k| k.label() == label)
}

/// Write `rows.tsv`: every scored row, for the paired comparison. It holds
/// per-row residuals, so it stays under `corpus/` with the data.
#[must_use]
pub fn rows_tsv(groups: &[ModelRows]) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "{ROWS_HEADER}");
    for g in groups {
        for r in &g.rows {
            let pair = |p: Option<(f64, f64)>| (opt(p.map(|x| x.0)), opt(p.map(|x| x.1)));
            let (fm, fy) = pair(r.fof2);
            let (hm, hy) = pair(r.hmf2);
            let (mm, my) = pair(r.muf);
            let _ = writeln!(
                s,
                "{}\t{}\t{}\t{}\t{:04}-{:02}-{:02}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{fm}\t{fy}\t{hm}\t{hy}\t{mm}\t{my}",
                g.model,
                g.corpus,
                g.split.label(),
                r.id,
                r.day.0,
                r.day.1,
                r.day.2,
                r.cuts.join("|"),
                opt(r.measured_db),
                opt(r.model_db),
                opt(r.effect_db),
                opt(r.p_decode),
                opt_bool(r.decoded),
                opt_bool(r.decodable),
            );
        }
    }
    s
}

/// Read back what [`rows_tsv`] wrote, grouped as it was written.
///
/// # Errors
/// A malformed row.
pub fn parse_rows_tsv(text: &str) -> Result<Vec<ModelRows>, String> {
    let mut groups: Vec<ModelRows> = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line == ROWS_HEADER || line.trim().is_empty() {
            continue;
        }
        let bad = || format!("rows.tsv line {}: malformed", i + 1);
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 18 {
            return Err(bad());
        }
        let o = |s: &str| -> Option<f64> { s.parse().ok() };
        let b = |s: &str| match s {
            "1" => Some(true),
            "0" => Some(false),
            _ => None,
        };
        let pair = |a: &str, c: &str| Some((o(a)?, o(c)?));
        let d: Vec<&str> = f[4].split('-').collect();
        if d.len() != 3 {
            return Err(bad());
        }
        let day = (
            d[0].parse().map_err(|_| bad())?,
            d[1].parse().map_err(|_| bad())?,
            d[2].parse().map_err(|_| bad())?,
        );
        let row = Scored {
            id: f[3].to_string(),
            day,
            cuts: if f[5].is_empty() {
                Vec::new()
            } else {
                f[5].split('|').map(str::to_string).collect()
            },
            measured_db: o(f[6]),
            model_db: o(f[7]),
            effect_db: o(f[8]),
            p_decode: o(f[9]),
            decoded: b(f[10]),
            decodable: b(f[11]),
            fof2: pair(f[12], f[13]),
            hmf2: pair(f[14], f[15]),
            muf: pair(f[16], f[17]),
        };
        let split = split_from(f[2]).ok_or_else(bad)?;
        let same = groups
            .last()
            .is_some_and(|g| g.model == f[0] && g.corpus == f[1] && g.split == split);
        if !same {
            groups.push(ModelRows {
                model: f[0].to_string(),
                corpus: f[1].to_string(),
                manifest_sha: String::new(),
                split,
                rows: Vec::new(),
            });
        }
        if let Some(g) = groups.last_mut() {
            g.rows.push(row);
        }
    }
    Ok(groups)
}

/// The paired before/after table for `compare`: every model, corpus, split,
/// cut and metric present in both, headline split first.
#[must_use]
pub fn compare_tsv(
    before: &[ModelRows],
    after: &[ModelRows],
    seed: u64,
    resamples: usize,
) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "# Paired day-block bootstrap, {resamples} resamples, seed {seed}. delta = after - before;"
    );
    let _ = writeln!(
        s,
        "# lo and hi are the 95 % interval on delta. n is the rows both runs scored."
    );
    let _ = writeln!(
        s,
        "model\tcorpus\tsplit\tcut\tmetric\tbefore\tafter\tdelta\tlo\thi\tn"
    );
    let order = |k: SplitKind| match k {
        SplitKind::MonthRegion => 0,
        SplitKind::Month => 1,
        SplitKind::Region => 2,
        SplitKind::Station => 3,
        SplitKind::All => 4,
    };
    let mut pairs: Vec<(&ModelRows, &ModelRows)> = before
        .iter()
        .filter_map(|b| {
            after
                .iter()
                .find(|a| a.model == b.model && a.corpus == b.corpus && a.split == b.split)
                .map(|a| (b, a))
        })
        .collect();
    pairs.sort_by_key(|(b, _)| (order(b.split), b.corpus.clone(), b.model.clone()));
    for (b, a) in pairs {
        let a_cuts: BTreeMap<String, Vec<&Scored>> =
            super::pipeline::cut_groups(&a.rows).into_iter().collect();
        for (cut, rows) in super::pipeline::cut_groups(&b.rows) {
            let Some(arows) = a_cuts.get(&cut) else {
                continue;
            };
            let key = format!("{}|{}|{}", b.corpus, b.split.label(), cut);
            let p = bootstrap::paired(&rows, arows, bootstrap::seed_for(seed, &key), resamples);
            for (k, name) in METRICS.iter().enumerate() {
                let x = p[k];
                if !(x.before.is_finite() && x.after.is_finite()) {
                    continue;
                }
                let _ = writeln!(
                    s,
                    "{}\t{}\t{}\t{cut}\t{name}\t{}\t{}\t{}\t{}\t{}\t{}",
                    b.model,
                    b.corpus,
                    b.split.label(),
                    num(x.before),
                    num(x.after),
                    num(x.delta),
                    num(x.lo),
                    num(x.hi),
                    x.n
                );
            }
        }
    }
    s
}

/// Everything the report is written from.
pub struct ReportContext<'a> {
    pub corpora: &'a [Corpus],
    pub results: &'a [ResultRow],
    pub model_rows: &'a [ModelRows],
    pub cfg: &'a RunConfig,
    pub prov: &'a Provenance,
    pub p533_notes: &'a [String],
    pub p533_checks: &'a [(super::baseline::P533Row, f64)],
    pub synthetic: bool,
}

type Key<'a> = (&'a str, &'a str, &'a str, &'a str, &'a str);

fn index(results: &[ResultRow]) -> BTreeMap<Key<'_>, &ResultRow> {
    results
        .iter()
        .map(|r| {
            (
                (
                    r.model.as_str(),
                    r.corpus.as_str(),
                    r.split.as_str(),
                    r.cut.as_str(),
                    r.metric.as_str(),
                ),
                r,
            )
        })
        .collect()
}

fn cell(r: Option<&&ResultRow>) -> String {
    let Some(r) = r else {
        return "-".to_string();
    };
    let flag = if r.n < MIN_QUOTABLE { " \u{2020}" } else { "" };
    if r.lo.is_finite() && r.hi.is_finite() {
        format!("{:.3} [{:.3}, {:.3}] n={}{flag}", r.value, r.lo, r.hi, r.n)
    } else {
        format!("{:.3} n={}{flag}", r.value, r.n)
    }
}

/// The metrics the headline and split tables show.
const HEADLINE: [&str; 9] = [
    "brier",
    "brier_skill",
    "auc",
    "false_positive_rate",
    "snr_bias_mean_none",
    "snr_rms_none",
    "snr_rms_eff",
    "snr_slope_eff",
    "path_found_rate",
];

const IONO: [&str; 7] = [
    "fof2_bias",
    "fof2_rms",
    "hmf2_bias",
    "hmf2_rms",
    "muf_bias",
    "muf_rms",
    "muf_median_abs",
];

/// How Skipzone did against P.533 on one corpus's doubly held-out cell.
enum Verdict {
    Better,
    Worse,
    NotDistinguished,
}

/// Write `report.md`.
#[must_use]
#[allow(clippy::too_many_lines)]
pub fn report_md(ctx: &ReportContext<'_>) -> String {
    let ix = index(ctx.results);
    let mut s = String::new();
    let _ = writeln!(s, "# Skipzone accuracy benchmark\n");
    if ctx.synthetic {
        let _ = writeln!(
            s,
            "> **FIXTURE RUN.** At least one corpus in this run is synthetic test data. \
             The numbers below exercise the harness and say nothing about the model.\n"
        );
    }
    let _ = writeln!(
        s,
        "Commit `{}`, generated {} UTC. Values are model minus measured; \
         intervals are 95 % day-block bootstrap intervals ({} resamples, seed {}). \
         \u{2020} marks a value from fewer than {MIN_QUOTABLE} rows.\n",
        ctx.prov.commit, ctx.prov.generated_utc, ctx.cfg.resamples, ctx.cfg.seed
    );

    // The comparison against P.533 decides the opening line.
    let headline = SplitKind::MonthRegion;
    let mut comparisons = Vec::new();
    for c in ctx.corpora {
        let name = c.manifest.corpus.as_str();
        let find = |model: &str| {
            ctx.model_rows
                .iter()
                .find(|m| m.model == model && m.corpus == name && m.split == headline)
        };
        if let (Some(sz), Some(p5)) = (find(SKIPZONE), find(P533)) {
            let (a, b): (Vec<&Scored>, Vec<&Scored>) =
                (sz.rows.iter().collect(), p5.rows.iter().collect());
            let key = format!("{name}|{}|all|vs-p533", headline.label());
            let paired = bootstrap::paired(
                &b,
                &a,
                bootstrap::seed_for(ctx.cfg.seed, &key),
                ctx.cfg.resamples,
            );
            let k = metrics::index_of("brier").expect("brier is a metric");
            let verdict = if paired[k].hi < 0.0 {
                Verdict::Better
            } else if paired[k].lo > 0.0 {
                Verdict::Worse
            } else {
                Verdict::NotDistinguished
            };
            comparisons.push((name, paired, verdict));
        }
    }
    if comparisons.is_empty() {
        let _ = writeln!(
            s,
            "No P.533 results were given (`--p533`), so the headline rule \
             (docs/notes/accuracy-roadmap.md §2.5) cannot be applied yet.\n"
        );
    } else {
        let not_beaten: Vec<&str> = comparisons
            .iter()
            .filter(|c| !matches!(c.2, Verdict::Better))
            .map(|c| c.0)
            .collect();
        if not_beaten.is_empty() {
            let _ = writeln!(
                s,
                "**Skipzone beats P.533 on the doubly held-out cell** of every corpus scored by \
                 both, judged on the Brier score (paired interval on the difference below zero).\n"
            );
        } else {
            let _ = writeln!(
                s,
                "**Skipzone does not beat P.533 on the doubly held-out cell** of: {}. \
                 Judged on the Brier score, the paired interval on Skipzone minus P.533 is not \
                 below zero.\n",
                not_beaten.join(", ")
            );
        }
        let _ = writeln!(
            s,
            "The Brier score stands in as the primary metric until the owner names one \
             (docs/notes/accuracy-roadmap.md §4).\n"
        );
    }

    let models: Vec<(&str, &str)> = {
        let mut v: Vec<(&str, &str)> = ctx
            .model_rows
            .iter()
            .filter(|m| m.split != SplitKind::Station)
            .map(|m| (m.corpus.as_str(), m.model.as_str()))
            .collect();
        v.dedup();
        v
    };

    let _ = writeln!(
        s,
        "## Headline: month and region both held out\n\n\
         Every row is scored once, in the cell of its own month and midpoint region, by a \
         model that saw neither. `_none` is the SNR with no station effects (what an unknown \
         user sees); `_eff` has transmitter and receiver effects re-estimated on the cell's \
         own rows.\n"
    );
    let _ = writeln!(s, "| corpus | model | {} |", HEADLINE.join(" | "));
    let _ = writeln!(s, "|---|---|{}", "---|".repeat(HEADLINE.len()));
    for (corpus, model) in &models {
        let cells: Vec<String> = HEADLINE
            .iter()
            .map(|m| cell(ix.get(&(*model, *corpus, headline.label(), "all", *m))))
            .collect();
        let _ = writeln!(s, "| {corpus} | {model} | {} |", cells.join(" | "));
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "## Against P.533\n");
    if comparisons.is_empty() {
        let _ = writeln!(s, "Not run.\n");
    }
    for (name, paired, _) in &comparisons {
        let _ = writeln!(
            s,
            "### {name}\n\nPaired on the rows both scored, doubly held-out cell. \
             Delta is Skipzone minus P.533.\n"
        );
        let _ = writeln!(s, "| metric | P.533 | Skipzone | delta [95 %] | n |");
        let _ = writeln!(s, "|---|---|---|---|---|");
        for m in HEADLINE {
            let k = metrics::index_of(m).expect("headline metrics exist");
            let x = paired[k];
            if !(x.before.is_finite() && x.after.is_finite()) {
                continue;
            }
            let _ = writeln!(
                s,
                "| {m} | {:.3} | {:.3} | {:+.3} [{:+.3}, {:+.3}] | {} |",
                x.before, x.after, x.delta, x.lo, x.hi, x.n
            );
        }
        let _ = writeln!(s);
    }
    if !ctx.p533_checks.is_empty() {
        let _ = writeln!(
            s,
            "P(decode) from the split-normal deciles against P.533's own basic circuit \
             reliability, first rows. The roadmap asks for this check before the split \
             normal is trusted.\n"
        );
        let _ = writeln!(s, "| MHz | hour | SNR | DL | DU | P(decode) | BCR/100 |");
        let _ = writeln!(s, "|---|---|---|---|---|---|---|");
        for (r, p) in ctx.p533_checks {
            let _ = writeln!(
                s,
                "| {:.3} | {} | {:.1} | {:.1} | {:.1} | {p:.3} | {:.3} |",
                r.freq_mhz,
                r.hour,
                r.snr_db,
                r.snr_dl,
                r.snr_du,
                r.bcr_pct / 100.0
            );
        }
        let _ = writeln!(s);
    }
    for n in ctx.p533_notes {
        let _ = writeln!(s, "- {n}");
    }
    if !ctx.p533_notes.is_empty() {
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## By split\n");
    for (corpus, model) in &models {
        let _ = writeln!(s, "### {corpus}, {model}\n");
        let _ = writeln!(s, "| split | {} |", HEADLINE.join(" | "));
        let _ = writeln!(s, "|---|{}", "---|".repeat(HEADLINE.len()));
        for k in SplitKind::PATH_SPLITS {
            let cells: Vec<String> = HEADLINE
                .iter()
                .map(|m| cell(ix.get(&(*model, *corpus, k.label(), "all", *m))))
                .collect();
            let _ = writeln!(s, "| {} | {} |", k.label(), cells.join(" | "));
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "## Cuts, doubly held-out\n");
    for (corpus, model) in &models {
        let Some(m) = ctx.model_rows.iter().find(|m| {
            m.corpus == *corpus && m.model == *model && m.split == SplitKind::MonthRegion
        }) else {
            continue;
        };
        let _ = writeln!(s, "### {corpus}, {model}\n");
        let cols = [
            "snr_bias_mean_none",
            "snr_rms_none",
            "snr_rms_eff",
            "brier",
            "false_positive_rate",
        ];
        let _ = writeln!(s, "| cut | {} |", cols.join(" | "));
        let _ = writeln!(s, "|---|{}", "---|".repeat(cols.len()));
        for (cut, _) in super::pipeline::cut_groups(&m.rows).iter().skip(1) {
            let cells: Vec<String> = cols
                .iter()
                .map(|c| cell(ix.get(&(*model, *corpus, headline.label(), cut.as_str(), *c))))
                .collect();
            let _ = writeln!(s, "| {cut} | {} |", cells.join(" | "));
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(
        s,
        "## Reliability diagrams\n\nTen equal-width bins of predicted P(decode). The \
         interval is Wilson 95 % on the observed fraction. Probabilities do not depend on \
         the split, so one diagram per model and corpus.\n"
    );
    for (corpus, model) in &models {
        let Some(m) = ctx.model_rows.iter().find(|m| {
            m.corpus == *corpus && m.model == *model && m.split == SplitKind::MonthRegion
        }) else {
            continue;
        };
        let rows: Vec<&Scored> = m.rows.iter().collect();
        let py = metrics::prob_pairs(&rows);
        if py.is_empty() {
            continue;
        }
        let _ = writeln!(s, "### {corpus}, {model}\n");
        let _ = writeln!(s, "| bin | mean predicted | observed | count | interval |");
        let _ = writeln!(s, "|---|---|---|---|---|");
        for b in metrics::reliability(&py) {
            if b.n == 0 {
                let _ = writeln!(s, "| {:.1}-{:.1} | - | - | 0 | - |", b.lo, b.hi);
                continue;
            }
            let _ = writeln!(
                s,
                "| {:.1}-{:.1} | {:.3} | {:.3} | {} | {:.3}-{:.3} |",
                b.lo, b.hi, b.mean_p, b.observed, b.n, b.wilson.0, b.wilson.1
            );
        }
        let _ = writeln!(s);
    }
    if ctx.corpora.iter().any(|c| {
        c.obs
            .iter()
            .any(|o| o.source == super::corpus::Source::Wspr)
    }) {
        let _ = writeln!(
            s,
            "On WSPR corpora the decode fraction is set by how many negatives were \
             constructed, so the base rate, the Brier skill score and the uncertainty term \
             describe the sample and say little about the band.\n"
        );
    }

    let iono: Vec<&ModelRows> = ctx
        .model_rows
        .iter()
        .filter(|m| m.split == SplitKind::Station)
        .collect();
    if !iono.is_empty() {
        let _ = writeln!(
            s,
            "## Ionosondes\n\nMHz for foF2 and MUF(3000)F2, km for hmF2. Leave-one-station-out; \
             nothing is fitted yet. The model MUF is the highest frequency a one-hop F2 path \
             closes over 3000 km centred on the station, found by bisection.\n"
        );
        for m in iono {
            let _ = writeln!(s, "### {}, {}\n", m.corpus, m.model);
            let _ = writeln!(s, "| cut | {} |", IONO.join(" | "));
            let _ = writeln!(s, "|---|{}", "---|".repeat(IONO.len()));
            for (cut, _) in super::pipeline::cut_groups(&m.rows) {
                let cells: Vec<String> = IONO
                    .iter()
                    .map(|c| {
                        cell(ix.get(&(
                            m.model.as_str(),
                            m.corpus.as_str(),
                            "station",
                            cut.as_str(),
                            *c,
                        )))
                    })
                    .collect();
                let _ = writeln!(s, "| {cut} | {} |", cells.join(" | "));
            }
            let _ = writeln!(s);
        }
    }

    let _ = writeln!(s, "## Provenance\n");
    let _ = writeln!(s, "- Commit: `{}`", ctx.prov.commit);
    let _ = writeln!(s, "- solve_digest: {}", ctx.prov.digest);
    let _ = writeln!(s, "- rustc: {}", ctx.prov.rustc);
    let _ = writeln!(s, "- ITURHFProp: {}", ctx.prov.p533_commit);
    let _ = writeln!(
        s,
        "- Seed {}, {} resamples; station effects need {} rows per station in a cell; \
         at most {} rows per file{}",
        ctx.cfg.seed,
        ctx.cfg.resamples,
        ctx.cfg.min_station_rows,
        ctx.cfg.max_rows,
        if ctx.cfg.max_rows == 0 {
            " (0: no cap)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        s,
        "- Skipzone as shipped (`Inputs::default()`, including its model bias) with \
         isotropic antennas at both ends; WSPR rows at 2500 Hz and -29 dB, RBN and beacon \
         rows at the CW preset. P(decode) is the shipped fading probability times the \
         layer probability."
    );
    let _ = writeln!(s);
    for c in ctx.corpora {
        let m = &c.manifest;
        let _ = writeln!(s, "### Corpus {}\n", m.corpus);
        let _ = writeln!(
            s,
            "- Manifest `{}`, SHA-256 `{}`",
            m.path.display(),
            m.sha256
        );
        if m.synthetic {
            let _ = writeln!(s, "- **Synthetic fixture.**");
        }
        for (k, v) in &m.settings {
            if k != "corpus" {
                let _ = writeln!(s, "- {k}: {v}");
            }
        }
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| file | kind | SHA-256 | rows | fetched | source | licence | attribution |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|---|---|");
        for e in &m.entries {
            let _ = writeln!(
                s,
                "| {} | {} | `{}` | {} | {} | {} | {} | {} |",
                e.file,
                e.kind.label(),
                e.sha256,
                e.rows.map_or_else(|| "-".to_string(), |r| r.to_string()),
                e.fetched_utc,
                e.source,
                e.licence,
                e.attribution
            );
        }
        let _ = writeln!(s);
        for n in &c.notes {
            let _ = writeln!(s, "- {n}");
        }
        let _ = writeln!(s);
    }
    s
}

//! A whole benchmark run: load and verify the corpora, predict every row
//! with every model, score each split, bootstrap, and write the outputs.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use skipzone_iono::solar::Season;
use skipzone_predict::compute::{ComputePool, Execution, PoolConfig};
use skipzone_predict::noise::NoiseEnvironment;
use skipzone_predict::scenario::Inputs;

use crate::corpus::MIN_SPOTS_PER_STATION;
use crate::fit::{Cached, StationEffects};

use super::baseline::{self, GroupTable, P533Row};
use super::bootstrap;
use super::corpus::{self, Obs, Sounding, Station};
use super::manifest::{Kind, Manifest};
use super::metrics::{self, METRICS, Scored};
use super::predict::{self, Prediction};
use super::report;
use super::split::{self, Fold, NoFit, Region, SplitKey, SplitKind};

/// The model names in `results.tsv`.
pub const SKIPZONE: &str = "skipzone";
pub const P533: &str = "p533";

/// Everything a run needs.
#[derive(Clone, Debug)]
pub struct RunConfig {
    pub manifests: Vec<PathBuf>,
    pub out_dir: PathBuf,
    /// A directory `p533-inputs` wrote, with ITURHFProp's `out/` beside it.
    pub p533_dir: Option<PathBuf>,
    /// The ITURHFProp commit the owner built, for the report.
    pub p533_commit: Option<String>,
    /// A saved `solve_digest` output, whose hash the report records.
    pub digest_file: Option<PathBuf>,
    pub seed: u64,
    pub resamples: usize,
    /// Cap per corpus file, thinned by a fixed stride; 0 keeps every row.
    pub max_rows: usize,
    /// Run the MUF bisection on ionosonde rows.
    pub muf: bool,
    /// Rows a station needs in a fold before its effect is estimated.
    pub min_station_rows: usize,
    pub threads: Option<usize>,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            manifests: Vec::new(),
            out_dir: PathBuf::new(),
            p533_dir: None,
            p533_commit: None,
            digest_file: None,
            seed: bootstrap::DEFAULT_SEED,
            resamples: bootstrap::DEFAULT_RESAMPLES,
            max_rows: 0,
            muf: true,
            min_station_rows: MIN_SPOTS_PER_STATION,
            threads: None,
        }
    }
}

/// One verified corpus, loaded.
#[derive(Clone, Debug)]
pub struct Corpus {
    pub manifest: Manifest,
    pub obs: Vec<Obs>,
    pub soundings: Vec<Sounding>,
    pub r12: BTreeMap<(i32, u32), f64>,
    /// What the loaders read, dropped and thinned, for the report.
    pub notes: Vec<String>,
}

/// Keep at most `max` rows by a fixed stride, as `wspr_calibrate` thins:
/// a prefix would be one day on one band.
fn thin<T: Clone>(rows: Vec<T>, max: usize) -> (Vec<T>, usize) {
    if max == 0 || rows.len() <= max {
        return (rows, 1);
    }
    let stride = rows.len().div_ceil(max);
    (rows.into_iter().step_by(stride).collect(), stride)
}

fn note_problems(notes: &mut Vec<String>, file: &str, problems: &[String]) {
    if problems.is_empty() {
        return;
    }
    notes.push(format!("{file}: {} unreadable row(s)", problems.len()));
    for p in problems.iter().take(3) {
        notes.push(format!("  {p}"));
    }
}

/// Load one manifest and every corpus file it lists.
///
/// # Errors
/// A manifest or hash failure, or a manifest mixing path corpus families
/// (WSPR, RBN and beacon rows are scored apart).
#[allow(clippy::too_many_lines)]
pub fn load_corpus(path: &Path, max_rows: usize) -> Result<Corpus, String> {
    let manifest = Manifest::load(path)?;
    let mut notes = Vec::new();
    let families: BTreeSet<&str> = manifest
        .entries
        .iter()
        .filter_map(|e| match e.kind {
            Kind::Wspr | Kind::WsprNegatives => Some("wspr"),
            Kind::Rbn => Some("rbn"),
            Kind::Beacon => Some("beacon"),
            _ => None,
        })
        .collect();
    if families.len() > 1 {
        return Err(format!(
            "{}: mixes {families:?}; give each path corpus its own manifest",
            path.display()
        ));
    }

    let mut stations: BTreeMap<String, Station> = BTreeMap::new();
    for e in manifest.of_kind(Kind::Stations) {
        let l = corpus::load_stations(e);
        note_problems(&mut notes, &e.file, &l.problems);
        stations.extend(l.rows);
    }
    let mut ssn = BTreeMap::new();
    for e in manifest.of_kind(Kind::SsnDaily) {
        let l = corpus::load_ssn_daily(e);
        note_problems(&mut notes, &e.file, &l.problems);
        ssn.extend(l.rows);
    }
    let mut r12 = BTreeMap::new();
    for e in manifest.of_kind(Kind::R12) {
        let l = corpus::load_r12(e);
        note_problems(&mut notes, &e.file, &l.problems);
        r12.extend(l.rows);
    }

    let mut obs = Vec::new();
    let mut take = |file: &str, rows: Vec<Obs>, what: &str, notes: &mut Vec<String>| {
        let n = rows.len();
        let (kept, stride) = thin(rows, max_rows);
        if stride > 1 {
            notes.push(format!(
                "{file}: {} of {n} {what} kept (every {stride}th row)",
                kept.len()
            ));
        } else {
            notes.push(format!("{file}: {n} {what}"));
        }
        obs.extend(kept);
    };
    for e in manifest.of_kind(Kind::Wspr) {
        let l = corpus::load_wspr(e);
        note_problems(&mut notes, &e.file, &l.problems);
        take(&e.file, l.rows, "decodes", &mut notes);
    }
    for e in manifest.of_kind(Kind::WsprNegatives) {
        let l = corpus::load_wspr_negatives(e);
        note_problems(&mut notes, &e.file, &l.problems);
        take(&e.file, l.rows, "non-decodes", &mut notes);
    }
    let window = manifest
        .setting_f64("rbn_window_minutes")
        .map_or(10, |v| v.round().clamp(1.0, 60.0) as u32);
    for e in manifest.of_kind(Kind::Rbn) {
        let l = corpus::load_rbn(e, &stations, &ssn, window);
        note_problems(&mut notes, &e.file, &l.problems);
        notes.push(format!(
            "{}: window {window} min; dropped {} with no station position, {} with no SSN, \
             {} repeats of a pair in a window, {} not CW",
            e.file, l.no_position, l.no_ssn, l.repeats, l.not_cw
        ));
        take(&e.file, l.positives, "decodes", &mut notes);
        take(&e.file, l.negatives, "constructed non-decodes", &mut notes);
    }
    let off: BTreeSet<String> = manifest
        .settings
        .get("beacons_off")
        .map(|v| {
            v.split([',', ' '])
                .filter(|s| !s.is_empty())
                .map(corpus::normalise_call)
                .collect()
        })
        .unwrap_or_default();
    for e in manifest.of_kind(Kind::Beacon) {
        let l = corpus::load_beacons(e, &stations, &ssn, &off);
        note_problems(&mut notes, &e.file, &l.problems);
        take(&e.file, l.rows, "beacon slots", &mut notes);
    }
    let mut soundings = Vec::new();
    for e in manifest.of_kind(Kind::Ionosonde) {
        let l = corpus::load_ionosonde(e, &ssn);
        note_problems(&mut notes, &e.file, &l.problems);
        let n = l.rows.len();
        // One sounding per station per hour: the MUF costs a bisection each
        // (docs/notes/accuracy-roadmap.md §2.3).
        let mut seen = BTreeSet::new();
        let hourly: Vec<Sounding> = l
            .rows
            .into_iter()
            .filter(|s| {
                let t = s.timestamp;
                seen.insert((s.station.clone(), t.0, t.1, t.2, t.3))
            })
            .collect();
        let (kept, stride) = thin(hourly, max_rows);
        notes.push(format!(
            "{}: {n} soundings accepted, {} after one per station per hour{}",
            e.file,
            kept.len(),
            if stride > 1 {
                format!(" and a stride of {stride}")
            } else {
                String::new()
            }
        ));
        soundings.extend(kept);
    }
    for (k, r) in manifest.entries.iter().filter_map(|e| Some((e, e.rows?))) {
        notes.push(format!("{}: manifest records {r} rows", k.file));
    }
    Ok(Corpus {
        manifest,
        obs,
        soundings,
        r12,
        notes,
    })
}

/// A neutral `Cached` carrying only station indices, for
/// [`StationEffects::solve`], which reads nothing else.
fn station_slot(tx: usize, rx: usize) -> Cached {
    Cached {
        tx,
        rx,
        measured_db: 0.0,
        tx_power_dbm: 0.0,
        loss_without_absorption_db: 0.0,
        absorption_db: 0.0,
        freq_mhz: 0.0,
        bandwidth_hz: 0.0,
        rx_is_day: true,
        rx_season: Season::Equinox,
        rx_lat: 0.0,
        noise_env: NoiseEnvironment::Rural,
        layer: "",
        hops: 0,
        range_km: 0.0,
        probability: 1.0,
        date: (0, 0, 0),
        midpoint_zenith_deg: 0.0,
        alternative: None,
    }
}

/// Transmitter plus receiver effect for each row that has both SNRs, fitted
/// on exactly these rows (the roadmap's figure 1). Rows whose stations have
/// fewer than `min_rows` usable rows get none: a station seen twice absorbs
/// its own residual and would make the figure look better than it is.
#[must_use]
pub fn fold_effects(obs: &[&Obs], model_db: &[Option<f64>], min_rows: usize) -> Vec<Option<f64>> {
    let mut usable: Vec<bool> = obs
        .iter()
        .zip(model_db)
        .map(|(o, m)| m.is_some() && o.measured_db.is_some())
        .collect();
    // Drop rare stations until every remaining station has enough rows;
    // dropping one can make another rare.
    loop {
        let mut tx_n: BTreeMap<&str, usize> = BTreeMap::new();
        let mut rx_n: BTreeMap<&str, usize> = BTreeMap::new();
        for (o, u) in obs.iter().zip(&usable) {
            if *u {
                *tx_n.entry(&o.tx_call).or_default() += 1;
                *rx_n.entry(&o.rx_call).or_default() += 1;
            }
        }
        let mut changed = false;
        for (o, u) in obs.iter().zip(usable.iter_mut()) {
            if *u && (tx_n[o.tx_call.as_str()] < min_rows || rx_n[o.rx_call.as_str()] < min_rows) {
                *u = false;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let mut tx_ix: BTreeMap<&str, usize> = BTreeMap::new();
    let mut rx_ix: BTreeMap<&str, usize> = BTreeMap::new();
    let mut slots = Vec::new();
    let mut resid = Vec::new();
    let mut which = Vec::new();
    for (i, o) in obs.iter().enumerate() {
        if !usable[i] {
            continue;
        }
        let n = tx_ix.len();
        let t = *tx_ix.entry(&o.tx_call).or_insert(n);
        let n = rx_ix.len();
        let r = *rx_ix.entry(&o.rx_call).or_insert(n);
        slots.push(station_slot(t, r));
        if let (Some(m), Some(y)) = (model_db[i], o.measured_db) {
            resid.push(m - y);
        }
        which.push(i);
    }
    let mut out = vec![None; obs.len()];
    if slots.is_empty() {
        return out;
    }
    let fx = StationEffects::solve(&resid, &slots, tx_ix.len(), rx_ix.len());
    for (slot, i) in slots.iter().zip(which) {
        out[i] = Some(fx.tx[slot.tx] + fx.rx[slot.rx]);
    }
    out
}

/// Scored rows of one model on one corpus under one split.
#[derive(Clone, Debug)]
pub struct ModelRows {
    pub model: String,
    pub corpus: String,
    pub manifest_sha: String,
    pub split: SplitKind,
    pub rows: Vec<Scored>,
}

/// Score path rows for one model under every split. `preds[i]` is `None`
/// where the model has no answer for row `i` (P.533 rows with no output);
/// those rows are left out, so a model is scored only where it answered.
#[must_use]
pub fn score_paths(
    model: &str,
    c: &Corpus,
    preds: &[Option<Prediction>],
    min_station_rows: usize,
) -> Vec<ModelRows> {
    let answered: Vec<usize> = (0..c.obs.len()).filter(|&i| preds[i].is_some()).collect();
    let keys: Vec<SplitKey> = answered
        .iter()
        .map(|&i| {
            let o = &c.obs[i];
            let g = o.geometry();
            SplitKey {
                month: (o.timestamp.0, o.timestamp.1),
                region: Region::of_path(o.tx_lat, o.rx_lat, g.mid_lat, g.mid_lon),
            }
        })
        .collect();
    let cuts: Vec<Vec<String>> = answered
        .iter()
        .map(|&i| predict::path_cuts(&c.obs[i]))
        .collect();
    let mut out = Vec::new();
    for kind in SplitKind::PATH_SPLITS {
        let folds = split::path_folds(&keys, kind);
        let mut rows = Vec::with_capacity(answered.len());
        split::run_folds(&folds, &mut NoFit, |f: &Fold| {
            let obs: Vec<&Obs> = f.test.iter().map(|&k| &c.obs[answered[k]]).collect();
            let model_db: Vec<Option<f64>> = f
                .test
                .iter()
                .map(|&k| preds[answered[k]].as_ref().and_then(|p| p.snr_db))
                .collect();
            let fx = fold_effects(&obs, &model_db, min_station_rows);
            for (j, &k) in f.test.iter().enumerate() {
                let o = obs[j];
                let p = preds[answered[k]]
                    .as_ref()
                    .expect("answered rows have a prediction");
                rows.push(Scored {
                    id: o.id.clone(),
                    day: o.date(),
                    cuts: cuts[k].clone(),
                    measured_db: o.measured_db,
                    model_db: p.snr_db,
                    effect_db: fx[j],
                    p_decode: Some(p.p_decode),
                    decoded: Some(o.decoded),
                    decodable: Some(p.decodable),
                    ..Scored::default()
                });
            }
        });
        out.push(ModelRows {
            model: model.to_string(),
            corpus: c.manifest.corpus.clone(),
            manifest_sha: c.manifest.sha256.clone(),
            split: kind,
            rows,
        });
    }
    out
}

/// Score ionosonde rows, leave-one-station-out. Nothing is fitted yet, so
/// the folds only fix which rows a future correction could have seen.
#[must_use]
pub fn score_iono(c: &Corpus, preds: &[predict::IonoPrediction]) -> ModelRows {
    let names: Vec<String> = c.soundings.iter().map(|s| s.station.clone()).collect();
    let folds = split::station_folds(&names);
    let mut rows = Vec::new();
    split::run_folds(&folds, &mut NoFit, |f| {
        for &i in &f.test {
            let s = &c.soundings[i];
            let p = &preds[i];
            rows.push(Scored {
                id: s.id.clone(),
                day: (s.timestamp.0, s.timestamp.1, s.timestamp.2),
                cuts: predict::iono_cuts(s),
                fof2: Some((p.fof2_mhz, s.fof2)),
                hmf2: s.hmf2_km.map(|h| (p.hmf2_km, h)),
                muf: match (p.muf3000_mhz, s.muf3000()) {
                    (Some(m), Some(y)) => Some((m, y)),
                    _ => None,
                },
                ..Scored::default()
            });
        }
    });
    ModelRows {
        model: SKIPZONE.to_string(),
        corpus: c.manifest.corpus.clone(),
        manifest_sha: c.manifest.sha256.clone(),
        split: SplitKind::Station,
        rows,
    }
}

/// One line of `results.tsv`.
#[derive(Clone, Debug, PartialEq)]
pub struct ResultRow {
    pub model: String,
    pub corpus: String,
    pub split: String,
    pub cut: String,
    pub metric: String,
    pub value: f64,
    pub lo: f64,
    pub hi: f64,
    /// How the interval was formed: `boot95` (day-block bootstrap) or
    /// `wilson95` (reliability bins).
    pub interval: String,
    pub n: usize,
    pub manifest_sha: String,
}

/// The cut groups of a set of rows: `all`, then every label in order.
#[must_use]
pub fn cut_groups(rows: &[Scored]) -> Vec<(String, Vec<&Scored>)> {
    let mut labels: BTreeSet<&str> = BTreeSet::new();
    for r in rows {
        for c in &r.cuts {
            labels.insert(c);
        }
    }
    let mut out = vec![("all".to_string(), rows.iter().collect())];
    for l in labels {
        out.push((
            l.to_string(),
            rows.iter()
                .filter(|r| r.cuts.iter().any(|c| c == l))
                .collect(),
        ));
    }
    out
}

/// Every metric, every cut, with intervals, for one [`ModelRows`].
#[must_use]
pub fn results_for(m: &ModelRows, seed: u64, resamples: usize) -> Vec<ResultRow> {
    let mut out = Vec::new();
    for (cut, rows) in cut_groups(&m.rows) {
        let point = metrics::compute(&rows);
        let key = format!("{}|{}|{}", m.corpus, m.split.label(), cut);
        let iv = bootstrap::intervals(&rows, bootstrap::seed_for(seed, &key), resamples);
        for (k, name) in METRICS.iter().enumerate() {
            if point[k].n == 0 {
                continue;
            }
            out.push(ResultRow {
                model: m.model.clone(),
                corpus: m.corpus.clone(),
                split: m.split.label().to_string(),
                cut: cut.clone(),
                metric: (*name).to_string(),
                value: point[k].value,
                lo: iv[k].0,
                hi: iv[k].1,
                interval: "boot95".to_string(),
                n: point[k].n,
                manifest_sha: m.manifest_sha.clone(),
            });
        }
        if cut == "all" {
            let py = metrics::prob_pairs(&rows);
            if py.is_empty() {
                continue;
            }
            for (b, bin) in metrics::reliability(&py).iter().enumerate() {
                if bin.n == 0 {
                    continue;
                }
                let base = ResultRow {
                    model: m.model.clone(),
                    corpus: m.corpus.clone(),
                    split: m.split.label().to_string(),
                    cut: cut.clone(),
                    metric: format!("reliability_bin{b}_observed"),
                    value: bin.observed,
                    lo: bin.wilson.0,
                    hi: bin.wilson.1,
                    interval: "wilson95".to_string(),
                    n: bin.n,
                    manifest_sha: m.manifest_sha.clone(),
                };
                out.push(ResultRow {
                    metric: format!("reliability_bin{b}_mean_p"),
                    lo: f64::NAN,
                    hi: f64::NAN,
                    interval: "-".to_string(),
                    value: bin.mean_p,
                    ..base.clone()
                });
                out.push(base);
            }
        }
    }
    out
}

/// P.533's answers for one corpus.
#[derive(Clone, Debug, Default)]
pub struct P533Load {
    /// One entry per corpus row, `None` where P.533 has no answer.
    pub preds: Vec<Option<Prediction>>,
    /// What was joined and what was missing, for the report.
    pub notes: Vec<String>,
    /// The first few rows with their P(decode), to set beside P.533's own
    /// circuit reliability.
    pub checks: Vec<(P533Row, f64)>,
}

/// P.533 predictions for a corpus's rows from an ITURHFProp directory.
///
/// # Errors
/// An unreadable `groups.tsv`, a corpus hash that differs from the one the
/// inputs were written for, or an unparseable output file.
pub fn p533_predictions(dir: &Path, c: &Corpus) -> Result<P533Load, String> {
    let gpath = dir.join("groups.tsv");
    let text = std::fs::read_to_string(&gpath).map_err(|e| format!("{}: {e}", gpath.display()))?;
    let table: GroupTable = baseline::parse_groups_tsv(&text)?;
    if let Some((_, sha)) = table
        .manifests
        .iter()
        .find(|(n, _)| n == &c.manifest.corpus)
        && sha != &c.manifest.sha256
    {
        return Err(format!(
            "{}: the P.533 inputs were written for corpus {} at manifest {sha}, \
             but this run has {}; regenerate them with p533-inputs",
            gpath.display(),
            c.manifest.corpus,
            c.manifest.sha256
        ));
    }
    let mut notes = Vec::new();
    let mut outputs: BTreeMap<String, Vec<P533Row>> = BTreeMap::new();
    let mut missing_files = 0usize;
    let mut checks = Vec::new();
    let mut preds = Vec::with_capacity(c.obs.len());
    for o in &c.obs {
        let Some((gid, k)) = table.rows.get(&format!("{}/{}", c.manifest.corpus, o.id)) else {
            preds.push(None);
            continue;
        };
        if !outputs.contains_key(gid) {
            let p = dir.join("out").join(format!("{gid}.out"));
            match std::fs::read_to_string(&p) {
                Ok(t) => {
                    let rows =
                        baseline::parse_output(&t).map_err(|e| format!("{}: {e}", p.display()))?;
                    outputs.insert(gid.clone(), rows);
                }
                Err(_) => {
                    missing_files += 1;
                    outputs.insert(gid.clone(), Vec::new());
                }
            }
        }
        let hour = table.groups.get(gid).map_or(0, |g| g.1);
        let (_, required) = baseline::receiver_for(o.source);
        let hit = outputs[gid]
            .iter()
            .find(|r| r.hour == hour && baseline::khz(r.freq_mhz) == *k);
        preds.push(hit.map(|r| {
            let p = r.p_decode(required);
            if checks.len() < 10 {
                checks.push((r.clone(), p));
            }
            Prediction {
                snr_db: Some(r.snr_db),
                p_decode: p,
                decodable: r.snr_db >= required,
                layer: None,
            }
        }));
    }
    let answered = preds.iter().filter(|p| p.is_some()).count();
    let written = table.manifests.iter().any(|(n, _)| n == &c.manifest.corpus);
    notes.push(format!(
        "{}: P.533 answered {answered} of {} rows; {missing_files} group output file(s) missing{}",
        c.manifest.corpus,
        c.obs.len(),
        if written {
            ""
        } else {
            " (groups.tsv has no inputs for this corpus)"
        }
    ));
    Ok(P533Load {
        preds,
        notes,
        checks,
    })
}

/// Run facts recorded in the report.
#[derive(Clone, Debug, Default)]
pub struct Provenance {
    pub commit: String,
    pub rustc: String,
    pub digest: String,
    pub p533_commit: String,
    pub generated_utc: String,
}

fn command_line(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The git commit of the working tree, with `-dirty` if it has changes.
#[must_use]
pub fn git_commit() -> String {
    let Some(sha) = command_line("git", &["rev-parse", "HEAD"]) else {
        return "unknown (git not available)".to_string();
    };
    let dirty = command_line("git", &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|s| !s.is_empty());
    if dirty { format!("{sha}-dirty") } else { sha }
}

/// `(year, month, day)` from days since 1970-01-01 (Howard Hinnant's
/// civil-from-days).
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y as i32, m, d)
}

/// Current UTC time as `YYYY-MM-DD HH:MM`.
#[must_use]
#[allow(clippy::cast_possible_wrap)]
pub fn now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs()) as i64;
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        s / 3600,
        (s % 3600) / 60
    )
}

/// The default run directory, `corpus/benchmark/runs/<date>-<commit>/`
/// (docs/notes/accuracy-roadmap.md §2.6).
#[must_use]
pub fn default_out_dir(commit: &str) -> PathBuf {
    let date = now_utc().chars().take(10).collect::<String>();
    let short: String = commit.chars().take(8).collect();
    PathBuf::from(format!("corpus/benchmark/runs/{date}-{short}"))
}

/// What a run produced, for the caller and the tests.
#[derive(Clone, Debug)]
pub struct RunOutput {
    pub results: Vec<ResultRow>,
    pub model_rows: Vec<ModelRows>,
    pub synthetic: bool,
}

fn pool(threads: Option<usize>) -> Result<ComputePool, String> {
    ComputePool::new(PoolConfig {
        execution: Execution::Parallel,
        max_threads: threads,
    })
}

/// Skipzone's per-row predictions for path rows.
pub type PathModel<'a> = &'a (dyn Fn(&Obs) -> Prediction + Sync);
/// Skipzone's per-sounding predictions for ionosonde rows.
pub type IonoModel<'a> = &'a (dyn Fn(&Sounding) -> predict::IonoPrediction + Sync);

/// Run the benchmark and write `results.tsv`, `rows.tsv` and `report.md`
/// into `cfg.out_dir`, with Skipzone as shipped as the model under test.
///
/// # Errors
/// Any corpus, manifest or P.533 problem, or an unwritable output directory.
pub fn run(cfg: &RunConfig) -> Result<RunOutput, String> {
    let base: Inputs = predict::base_inputs();
    let muf = cfg.muf;
    run_with(
        cfg,
        &|o: &Obs| predict::predict(o, &base),
        &|s: &Sounding| predict::predict_iono(s, &base, muf),
    )
}

/// [`run`] with the model supplied. The tests pass a fast stand-in here so
/// the whole pipeline can be checked in a debug build, where a real solve
/// takes seconds.
///
/// # Errors
/// As [`run`].
#[allow(clippy::too_many_lines)]
pub fn run_with(
    cfg: &RunConfig,
    path_model: PathModel<'_>,
    iono_model: IonoModel<'_>,
) -> Result<RunOutput, String> {
    if cfg.manifests.is_empty() {
        return Err("no --manifest given".to_string());
    }
    let mut corpora = Vec::new();
    for m in &cfg.manifests {
        eprintln!("[benchmark] verifying {}", m.display());
        corpora.push(load_corpus(m, cfg.max_rows)?);
    }
    let names: BTreeSet<&str> = corpora.iter().map(|c| c.manifest.corpus.as_str()).collect();
    if names.len() != corpora.len() {
        return Err("two manifests name the same corpus".to_string());
    }
    let pool = pool(cfg.threads)?;
    let mut model_rows = Vec::new();
    let mut p533_notes = Vec::new();
    let mut p533_checks = Vec::new();
    for c in &corpora {
        if !c.obs.is_empty() {
            eprintln!(
                "[benchmark] {}: solving {} rows on {} threads",
                c.manifest.corpus,
                c.obs.len(),
                pool.threads()
            );
            let (preds, timing) = pool.map(&c.obs, |o| Some(path_model(o)));
            eprintln!(
                "[benchmark] {}: {:.1} s",
                c.manifest.corpus,
                timing.total.as_secs_f64()
            );
            model_rows.extend(score_paths(SKIPZONE, c, &preds, cfg.min_station_rows));
            if let Some(dir) = &cfg.p533_dir {
                let p = p533_predictions(dir, c)?;
                p533_notes.extend(p.notes);
                p533_checks.extend(p.checks);
                if p.preds.iter().any(Option::is_some) {
                    model_rows.extend(score_paths(P533, c, &p.preds, cfg.min_station_rows));
                }
            }
        }
        if !c.soundings.is_empty() {
            eprintln!(
                "[benchmark] {}: {} soundings{}",
                c.manifest.corpus,
                c.soundings.len(),
                if cfg.muf { ", with MUF bisection" } else { "" }
            );
            let (preds, _) = pool.map(&c.soundings, |s| iono_model(s));
            model_rows.push(score_iono(c, &preds));
        }
    }
    // Each group seeds its own resamples, so running them in parallel gives
    // the same file as running them in order.
    eprintln!(
        "[benchmark] bootstrapping {} group(s), {} resamples each",
        model_rows.len(),
        cfg.resamples
    );
    let (per_group, _) = pool.map(&model_rows, |m| results_for(m, cfg.seed, cfg.resamples));
    let results: Vec<ResultRow> = per_group.into_iter().flatten().collect();
    let synthetic = corpora.iter().any(|c| c.manifest.synthetic);
    let commit = git_commit();
    let prov = Provenance {
        commit,
        rustc: command_line("rustc", &["--version"]).map_or_else(
            || "unknown".to_string(),
            |v| format!("{v} (on PATH at run time)"),
        ),
        digest: match &cfg.digest_file {
            Some(p) => {
                let bytes = std::fs::read(p).map_err(|e| format!("{}: {e}", p.display()))?;
                format!("SHA-256 {} of {}", super::sha256::hex(&bytes), p.display())
            }
            None => "not recorded (pass --digest with a saved solve_digest output)".to_string(),
        },
        p533_commit: match (&cfg.p533_dir, &cfg.p533_commit) {
            (None, _) => "no P.533 run given".to_string(),
            (Some(_), Some(c)) => c.clone(),
            (Some(_), None) => "not recorded (pass --p533-commit)".to_string(),
        },
        generated_utc: now_utc(),
    };
    std::fs::create_dir_all(&cfg.out_dir).map_err(|e| format!("{}: {e}", cfg.out_dir.display()))?;
    let write = |name: &str, text: String| -> Result<(), String> {
        let p = cfg.out_dir.join(name);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
    };
    write("results.tsv", report::results_tsv(&results, synthetic))?;
    write("rows.tsv", report::rows_tsv(&model_rows))?;
    let ctx = report::ReportContext {
        corpora: &corpora,
        results: &results,
        model_rows: &model_rows,
        cfg,
        prov: &prov,
        p533_notes: &p533_notes,
        p533_checks: &p533_checks,
        synthetic,
    };
    write("report.md", report::report_md(&ctx))?;
    Ok(RunOutput {
        results,
        model_rows,
        synthetic,
    })
}

/// Before/after comparison of two runs: every metric, paired on the rows
/// both scored, with the paired day-block interval on the difference.
///
/// `before` and `after` are `results.tsv` paths; `rows.tsv` is read from
/// beside each.
///
/// # Errors
/// Unreadable files, or runs on different corpus versions (rows could not
/// be paired).
pub fn compare(before: &Path, after: &Path, seed: u64, resamples: usize) -> Result<String, String> {
    let load = |p: &Path| -> Result<(Vec<ResultRow>, Vec<ModelRows>), String> {
        let t = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let res = report::parse_results_tsv(&t)?;
        let rp = p.with_file_name("rows.tsv");
        let t = std::fs::read_to_string(&rp).map_err(|e| format!("{}: {e}", rp.display()))?;
        Ok((res, report::parse_rows_tsv(&t)?))
    };
    let (rb, mb) = load(before)?;
    let (ra, ma) = load(after)?;
    let shas = |r: &[ResultRow]| -> BTreeMap<String, String> {
        r.iter()
            .map(|x| (x.corpus.clone(), x.manifest_sha.clone()))
            .collect()
    };
    let (sb, sa) = (shas(&rb), shas(&ra));
    for (corpus, sha) in &sb {
        if let Some(other) = sa.get(corpus)
            && other != sha
        {
            return Err(format!(
                "corpus {corpus} differs between the runs (manifest {sha} against {other}); \
                 rows cannot be paired"
            ));
        }
    }
    Ok(report::compare_tsv(&mb, &ma, seed, resamples))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::benchmark::corpus::Source;

    #[test]
    fn civil_dates_by_hand() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_397), (2025, 11, 5));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn thinning_keeps_a_stride_not_a_prefix() {
        let (k, s) = thin((0..10).collect::<Vec<_>>(), 4);
        assert_eq!(s, 3);
        assert_eq!(k, vec![0, 3, 6, 9]);
        let (k, s) = thin((0..3).collect::<Vec<_>>(), 0);
        assert_eq!((k.len(), s), (3, 1));
    }

    #[test]
    fn effects_skip_rare_stations_and_absorb_constant_offsets() {
        let mk = |tx: &str, rx: &str, y: f64| Obs {
            id: String::new(),
            source: Source::Wspr,
            tx_call: tx.into(),
            tx_grid: String::new(),
            rx_call: rx.into(),
            rx_grid: String::new(),
            tx_lat: 0.0,
            tx_lon: 0.0,
            rx_lat: 0.0,
            rx_lon: 0.0,
            freq_mhz: 14.0,
            timestamp: (2026, 1, 1, 0, 0),
            tx_dbm: 30.0,
            ssn: 100.0,
            measured_db: Some(y),
            decoded: true,
        };
        // A reads 3 dB high at both receivers; B reads 1 dB low. C is seen once.
        let obs = [
            mk("A", "X", 0.0),
            mk("A", "Y", 0.0),
            mk("B", "X", 0.0),
            mk("B", "Y", 0.0),
            mk("C", "X", 0.0),
        ];
        let model = [Some(3.0), Some(3.0), Some(-1.0), Some(-1.0), Some(9.0)];
        let refs: Vec<&Obs> = obs.iter().collect();
        let fx = fold_effects(&refs, &model, 2);
        assert!(fx[4].is_none());
        // Global is +1; effects are +2 for A, -2 for B, 0 for the receivers.
        assert!((fx[0].unwrap() - 2.0).abs() < 1e-9);
        assert!((fx[2].unwrap() + 2.0).abs() < 1e-9);
    }
}

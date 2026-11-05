//! End-to-end runs of the benchmark on the synthetic fixtures in
//! `tests/fixtures/benchmark/`.
//!
//! A real solve takes seconds in a debug build, so the pipeline tests pass a
//! fast stand-in model through `pipeline::run_with`. Everything else
//! (manifest checks, loaders, splits, station effects, metrics, bootstrap,
//! the P.533 join, the output files and the comparison) is the real code.
//! `real_model_predicts_a_fixture_row` runs the actual solver on one row.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use skipzone_lab::benchmark::baseline;
use skipzone_lab::benchmark::corpus::{Obs, Sounding};
use skipzone_lab::benchmark::pipeline::{self, RunConfig};
use skipzone_lab::benchmark::predict::{self, IonoPrediction, Prediction};
use skipzone_lab::benchmark::report;
use skipzone_lab::benchmark::split::SplitKind;
use skipzone_predict::noise;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/benchmark")
}

fn manifests() -> Vec<PathBuf> {
    ["wspr-v1", "rbn-v1", "beacon-v1", "iono-v1"]
        .iter()
        .map(|d| fixtures().join(d).join("MANIFEST.tsv"))
        .collect()
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("skipzone-bench-e2e-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A stand-in model: SNR falls with range, plus an offset. Deterministic
/// and independent of the measurement.
fn stub(offset: f64) -> impl Fn(&Obs) -> Prediction + Sync {
    move |o: &Obs| {
        let g = o.geometry();
        let snr = -5.0 - g.range_km / 200.0 + offset;
        let threshold = match o.source {
            skipzone_lab::benchmark::corpus::Source::Wspr => -29.0,
            _ => skipzone_predict::noise::OperatingMode::Cw.defaults().1,
        };
        Prediction {
            snr_db: (g.range_km < 6500.0).then_some(snr),
            p_decode: if g.range_km < 6500.0 {
                noise::decode_probability(snr - threshold, noise::PREDICTIVE_SPREAD_DB)
            } else {
                0.0
            },
            decodable: g.range_km < 6500.0 && snr >= threshold,
            layer: Some("F2"),
        }
    }
}

fn iono_stub(s: &Sounding) -> IonoPrediction {
    IonoPrediction {
        fof2_mhz: 8.0 + s.lat / 100.0,
        hmf2_km: 300.0,
        muf3000_mhz: Some(24.0),
    }
}

fn config(out: &Path) -> RunConfig {
    RunConfig {
        manifests: manifests(),
        out_dir: out.to_path_buf(),
        p533_dir: Some(fixtures().join("p533")),
        p533_commit: Some("synthetic fixture".to_string()),
        resamples: 200,
        min_station_rows: 2,
        threads: Some(2),
        ..RunConfig::default()
    }
}

fn run(out: &Path, offset: f64) -> pipeline::RunOutput {
    pipeline::run_with(&config(out), &stub(offset), &iono_stub).unwrap()
}

#[test]
fn whole_pipeline_on_the_fixtures() {
    let out = scratch("whole");
    let done = run(&out, 0.0);
    assert!(done.synthetic);

    // Rows per corpus, from the fixture design: 28 WSPR decodes and 4
    // non-decodes; 8 RBN decodes after the repeat, the RTTY row and the
    // station with no position are dropped, plus the constructed
    // non-decodes; 12 beacon slots after the off beacon and the wrong slot;
    // 3 soundings after the low-confidence row and the repeat in the hour.
    let count = |corpus: &str, split: SplitKind, model: &str| {
        done.model_rows
            .iter()
            .find(|m| m.corpus == corpus && m.split == split && m.model == model)
            .map(|m| m.rows.len())
    };
    assert_eq!(
        count("wspr-fixture-v1", SplitKind::MonthRegion, "skipzone"),
        Some(32)
    );
    assert_eq!(
        count("beacon-fixture-v1", SplitKind::MonthRegion, "skipzone"),
        Some(12)
    );
    assert_eq!(
        count("iono-fixture-v1", SplitKind::Station, "skipzone"),
        Some(3)
    );
    let rbn = count("rbn-fixture-v1", SplitKind::MonthRegion, "skipzone").unwrap();
    assert!(
        rbn > 8,
        "RBN should carry constructed non-decodes, got {rbn}"
    );
    // P.533 answered every WSPR row but the group left without an output file.
    assert_eq!(
        count("wspr-fixture-v1", SplitKind::MonthRegion, "p533"),
        Some(30)
    );

    // Every split scores every answered row exactly once.
    for m in &done.model_rows {
        let ids: BTreeSet<&str> = m.rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids.len(), m.rows.len(), "{} {:?}", m.corpus, m.split);
        let all = done
            .model_rows
            .iter()
            .find(|x| x.corpus == m.corpus && x.model == m.model && x.split == SplitKind::All)
            .map_or(m.rows.len(), |x| x.rows.len());
        assert_eq!(m.rows.len(), all);
    }

    // The result rows a PR would diff are there, with intervals.
    let has = |model: &str, corpus: &str, split: &str, cut: &str, metric: &str| {
        done.results.iter().find(|r| {
            r.model == model
                && r.corpus == corpus
                && r.split == split
                && r.cut == cut
                && r.metric == metric
        })
    };
    let brier = has(
        "skipzone",
        "wspr-fixture-v1",
        "month+region",
        "all",
        "brier",
    )
    .unwrap();
    assert_eq!(brier.n, 32);
    assert!(brier.lo <= brier.value && brier.value <= brier.hi);
    assert_eq!(brier.interval, "boot95");
    assert_eq!(brier.manifest_sha.len(), 64);
    assert!(
        has(
            "skipzone",
            "wspr-fixture-v1",
            "month",
            "midpoint=day",
            "snr_rms_none"
        )
        .is_some()
    );
    assert!(has("skipzone", "iono-fixture-v1", "station", "all", "muf_bias").is_some());
    assert!(has("p533", "wspr-fixture-v1", "month+region", "all", "brier").is_some());
    let bin = done
        .results
        .iter()
        .find(|r| r.metric.starts_with("reliability_bin") && r.metric.ends_with("_observed"))
        .unwrap();
    assert_eq!(bin.interval, "wilson95");
    // MUF error: 24 MHz against foF2 x M(3000)F2 on the two soundings that
    // scaled M(3000)F2: 9.10 x 3.05 and 8.20 x 3.20.
    let muf = has("skipzone", "iono-fixture-v1", "station", "all", "muf_bias").unwrap();
    let want = ((24.0 - 9.10 * 3.05) + (24.0 - 8.20 * 3.20)) / 2.0;
    assert!((muf.value - want).abs() < 1e-9, "{} vs {want}", muf.value);
    assert_eq!(muf.n, 2);

    // The files, and what they must say.
    let results = std::fs::read_to_string(out.join("results.tsv")).unwrap();
    assert!(results.starts_with("# FIXTURE RUN"));
    let md = std::fs::read_to_string(out.join("report.md")).unwrap();
    assert!(md.contains("**FIXTURE RUN.**"));
    assert!(md.contains("## Against P.533"));
    assert!(md.contains("P.533 on the doubly held-out cell"));
    assert!(md.contains("ITURHFProp: synthetic fixture"));
    for m in manifests() {
        let sha = skipzone_lab::benchmark::sha256::hex(&std::fs::read(&m).unwrap());
        assert!(md.contains(&sha), "report lacks manifest hash {sha}");
    }
    // results.tsv reads back to the same rows.
    let back = report::parse_results_tsv(&results).unwrap();
    assert_eq!(back.len(), done.results.len());
}

#[test]
fn same_seed_gives_byte_identical_results() {
    let (a, b) = (scratch("det-a"), scratch("det-b"));
    run(&a, 0.0);
    run(&b, 0.0);
    let ra = std::fs::read(a.join("results.tsv")).unwrap();
    let rb = std::fs::read(b.join("results.tsv")).unwrap();
    assert_eq!(ra, rb);
}

#[test]
fn compare_pairs_two_runs() {
    let (a, b) = (scratch("cmp-a"), scratch("cmp-b"));
    run(&a, 0.0);
    run(&b, 2.0);
    let table = pipeline::compare(&a.join("results.tsv"), &b.join("results.tsv"), 1, 100).unwrap();
    let line = table
        .lines()
        .find(|l| {
            l.starts_with("skipzone\twspr-fixture-v1\tmonth+region\tall\tsnr_bias_mean_none\t")
        })
        .unwrap();
    let f: Vec<&str> = line.split('\t').collect();
    // delta, lo, hi: a 2 dB shift on every row is exactly 2 on every resample.
    for v in &f[7..10] {
        assert!((v.parse::<f64>().unwrap() - 2.0).abs() < 1e-6, "{line}");
    }
    // The headline split comes first.
    let first = table
        .lines()
        .find(|l| !l.starts_with('#') && !l.starts_with("model"));
    assert!(first.unwrap().contains("\tmonth+region\t"));
}

#[test]
fn a_changed_corpus_file_stops_the_run() {
    let d = scratch("tamper");
    let src = fixtures().join("wspr-v1");
    for f in ["MANIFEST.tsv", "spots.tsv", "negatives.tsv"] {
        std::fs::copy(src.join(f), d.join(f)).unwrap();
    }
    let mut text = std::fs::read_to_string(d.join("spots.tsv")).unwrap();
    text = text.replacen("\t-10\t", "\t-11\t", 1);
    std::fs::write(d.join("spots.tsv"), text).unwrap();
    let cfg = RunConfig {
        manifests: vec![d.join("MANIFEST.tsv")],
        out_dir: d.join("out"),
        ..RunConfig::default()
    };
    let err = pipeline::run_with(&cfg, &stub(0.0), &iono_stub).unwrap_err();
    assert!(err.contains("SHA-256 MISMATCH"), "{err}");
    assert!(err.contains("spots.tsv"), "{err}");
    assert!(
        !d.join("out").exists(),
        "nothing may be written after a mismatch"
    );
}

#[test]
fn p533_groups_fixture_matches_the_generator() {
    // The committed groups.tsv was written by `benchmark p533-inputs` from
    // the WSPR fixture. Regenerating it must give the same rows.
    let c = pipeline::load_corpus(&fixtures().join("wspr-v1/MANIFEST.tsv"), 0).unwrap();
    let (groups, index) = baseline::groups(&c.obs, &c.r12);
    let index: BTreeMap<String, (String, i64)> = index
        .into_iter()
        .map(|(k, v)| (format!("{}/{k}", c.manifest.corpus), v))
        .collect();
    let text = baseline::groups_tsv(
        &groups,
        &index,
        &[(c.manifest.corpus.clone(), c.manifest.sha256.clone())],
    );
    let committed = std::fs::read_to_string(fixtures().join("p533/groups.tsv")).unwrap();
    assert_eq!(text, committed);
    let table = baseline::parse_groups_tsv(&committed).unwrap();
    assert_eq!(table.rows.len(), 32);
    assert_eq!(table.groups.len(), groups.len());
    // One input file per group, carrying the keys the roadmap lists.
    let input = baseline::input_file(&groups[0], "./Data/");
    for key in [
        "Path.L_tx.lat",
        "Path.L_rx.lng",
        "Path.SSN",
        "Path.frequency",
        "Path.txpower",
        "Path.BW 2500",
        "Path.SNRr -29.0",
        "Path.ManMadeNoise \"RURAL\"",
        "TXAntFilePath \"ISOTROPIC\"",
        "RptFileFormat",
    ] {
        assert!(input.contains(key), "{key} missing from\n{input}");
    }
}

#[test]
fn real_model_predicts_a_fixture_row() {
    // The actual solver on one row, so the default path is exercised too.
    let c = pipeline::load_corpus(&fixtures().join("wspr-v1/MANIFEST.tsv"), 0).unwrap();
    let base = predict::base_inputs();
    let p = predict::predict(&c.obs[1], &base);
    assert!((0.0..=1.0).contains(&p.p_decode));
    if let Some(snr) = p.snr_db {
        assert!(snr.is_finite());
        assert_eq!(p.decodable, snr >= -29.0);
    } else {
        assert_eq!(p.p_decode, 0.0);
    }
}

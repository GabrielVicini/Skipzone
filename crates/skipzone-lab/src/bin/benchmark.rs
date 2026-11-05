//! Score Skipzone, and the P.533 baseline, on frozen corpora with month and
//! region hold-outs (docs/notes/accuracy-roadmap.md §2).
//!
//! ```text
//! cargo run --release -p skipzone-lab --bin benchmark -- run \
//!     --manifest corpus/benchmark/wspr-v1/MANIFEST.tsv --out corpus/benchmark/runs/x
//! ```
//!
//! It needs no network: every input is a file whose SHA-256 the manifest
//! records. The work is in `skipzone_lab::benchmark`; this file only parses
//! the command line.

use std::path::PathBuf;
use std::process::ExitCode;

use skipzone_lab::benchmark::{baseline, bootstrap, pipeline};

const USAGE: &str = "\
usage: benchmark <command> [options]

commands:
  run            score every model on the corpora and write results.tsv,
                 rows.tsv and report.md
  compare        paired before/after comparison of two runs
  p533-inputs    write ITURHFProp input files for the corpora

benchmark run --manifest FILE [--manifest FILE ...] [options]
  --manifest FILE     a corpus MANIFEST.tsv; repeat for several corpora. Every
                      file's SHA-256 is checked before anything is read.
  --out DIR           output directory (default
                      corpus/benchmark/runs/<date>-<commit>/)
  --p533 DIR          a directory written by p533-inputs, with ITURHFProp's
                      output files in DIR/out/<group>.out
  --p533-commit SHA   the ITURHFProp commit that produced them, for the report
  --digest FILE       a saved solve_digest output; its hash goes in the report
  --seed N            bootstrap seed (default 20261105)
  --resamples N       bootstrap resamples (default 1000)
  --max-rows N        at most N rows per corpus file, by a fixed stride
                      (default 0, every row)
  --min-station-rows N
                      rows a station needs in a cell before its effect is
                      estimated (default 10)
  --no-muf            skip the MUF bisection on ionosonde rows
  --threads N         cap on worker threads (default: every core)

benchmark compare BEFORE/results.tsv AFTER/results.tsv [--seed N] [--resamples N] [--out FILE]
  Pairs the rows both runs scored (rows.tsv beside each results.tsv) and
  prints after - before with a paired day-block interval, as TSV.

benchmark p533-inputs --manifest FILE [--manifest FILE ...] --out DIR [--data-path PATH]
  Writes DIR/in/<group>.in, one per path, month, hour and power, and
  DIR/groups.tsv. Run each input with ITURHFProp into DIR/out/<group>.out,
  then pass DIR to `run --p533`. --data-path is ITURHFProp's DataFilePath
  (default ./Data/).
";

fn number<T: std::str::FromStr>(flag: &str, v: &str) -> Result<T, String> {
    v.parse().map_err(|_| format!("bad {flag}: {v:?}"))
}

fn run(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let mut cfg = pipeline::RunConfig::default();
    let mut out = None;
    while let Some(flag) = it.next() {
        let mut val = |what: &str| it.next().ok_or_else(|| format!("{what} needs a value"));
        match flag.as_str() {
            "--manifest" => cfg.manifests.push(PathBuf::from(val("--manifest")?)),
            "--out" => out = Some(PathBuf::from(val("--out")?)),
            "--p533" => cfg.p533_dir = Some(PathBuf::from(val("--p533")?)),
            "--p533-commit" => cfg.p533_commit = Some(val("--p533-commit")?),
            "--digest" => cfg.digest_file = Some(PathBuf::from(val("--digest")?)),
            "--seed" => cfg.seed = number("--seed", &val("--seed")?)?,
            "--resamples" => cfg.resamples = number("--resamples", &val("--resamples")?)?,
            "--max-rows" => cfg.max_rows = number("--max-rows", &val("--max-rows")?)?,
            "--min-station-rows" => {
                cfg.min_station_rows = number("--min-station-rows", &val("--min-station-rows")?)?;
            }
            "--no-muf" => cfg.muf = false,
            "--threads" => cfg.threads = Some(number("--threads", &val("--threads")?)?),
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    cfg.out_dir = out.unwrap_or_else(|| pipeline::default_out_dir(&pipeline::git_commit()));
    let done = pipeline::run(&cfg)?;
    println!(
        "{} result rows written to {}{}",
        done.results.len(),
        cfg.out_dir.display(),
        if done.synthetic {
            " (FIXTURE RUN: synthetic corpora)"
        } else {
            ""
        }
    );
    Ok(())
}

fn compare(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let mut files = Vec::new();
    let mut seed = bootstrap::DEFAULT_SEED;
    let mut resamples = bootstrap::DEFAULT_RESAMPLES;
    let mut out = None;
    while let Some(arg) = it.next() {
        let mut val = |what: &str| it.next().ok_or_else(|| format!("{what} needs a value"));
        match arg.as_str() {
            "--seed" => seed = number("--seed", &val("--seed")?)?,
            "--resamples" => resamples = number("--resamples", &val("--resamples")?)?,
            "--out" => out = Some(PathBuf::from(val("--out")?)),
            "-h" | "--help" => return Err(USAGE.to_string()),
            f if f.starts_with("--") => return Err(format!("unknown flag {f}\n\n{USAGE}")),
            _ => files.push(PathBuf::from(arg)),
        }
    }
    let [before, after] = files.as_slice() else {
        return Err(format!("compare takes two results.tsv files\n\n{USAGE}"));
    };
    let table = pipeline::compare(before, after, seed, resamples)?;
    match out {
        Some(p) => std::fs::write(&p, &table).map_err(|e| format!("{}: {e}", p.display()))?,
        None => print!("{table}"),
    }
    Ok(())
}

fn p533_inputs(mut it: impl Iterator<Item = String>) -> Result<(), String> {
    let mut manifests = Vec::new();
    let mut out = None;
    let mut data_path = "./Data/".to_string();
    while let Some(flag) = it.next() {
        let mut val = |what: &str| it.next().ok_or_else(|| format!("{what} needs a value"));
        match flag.as_str() {
            "--manifest" => manifests.push(PathBuf::from(val("--manifest")?)),
            "--out" => out = Some(PathBuf::from(val("--out")?)),
            "--data-path" => data_path = val("--data-path")?,
            "-h" | "--help" => return Err(USAGE.to_string()),
            other => return Err(format!("unknown flag {other}\n\n{USAGE}")),
        }
    }
    let out = out.ok_or_else(|| format!("p533-inputs needs --out\n\n{USAGE}"))?;
    if manifests.is_empty() {
        return Err(format!("p533-inputs needs --manifest\n\n{USAGE}"));
    }
    let mut groups = Vec::new();
    let mut index = std::collections::BTreeMap::new();
    let mut names = Vec::new();
    for m in &manifests {
        let c = pipeline::load_corpus(m, 0)?;
        names.push((c.manifest.corpus.clone(), c.manifest.sha256.clone()));
        let (mut g, ix) = baseline::groups(&c.obs, &c.r12);
        // Group ids must be unique across corpora.
        let prefix = groups.len();
        let renumber: std::collections::BTreeMap<String, String> = g
            .iter()
            .enumerate()
            .map(|(i, x)| (x.id.clone(), format!("g{:06}", prefix + i)))
            .collect();
        for x in &mut g {
            x.id = renumber[&x.id].clone();
        }
        for (row, (gid, k)) in ix {
            // Row ids are unique within a corpus only.
            index.insert(
                format!("{}/{row}", c.manifest.corpus),
                (renumber[&gid].clone(), k),
            );
        }
        groups.extend(g);
    }
    let in_dir = out.join("in");
    std::fs::create_dir_all(&in_dir).map_err(|e| format!("{}: {e}", in_dir.display()))?;
    std::fs::create_dir_all(out.join("out")).map_err(|e| format!("{}: {e}", out.display()))?;
    for g in &groups {
        let p = in_dir.join(format!("{}.in", g.id));
        std::fs::write(&p, baseline::input_file(g, &data_path))
            .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    let p = out.join("groups.tsv");
    std::fs::write(&p, baseline::groups_tsv(&groups, &index, &names))
        .map_err(|e| format!("{}: {e}", p.display()))?;
    println!(
        "{} ITURHFProp input file(s) for {} row(s) written to {}",
        groups.len(),
        index.len(),
        in_dir.display()
    );
    Ok(())
}

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let result = match args.next().as_deref() {
        Some("run") => run(args),
        Some("compare") => compare(args),
        Some("p533-inputs") => p533_inputs(args),
        Some("-h" | "--help") => Err(USAGE.to_string()),
        None => Err(format!("no command given\n\n{USAGE}")),
        Some(other) => Err(format!("unknown command {other}\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) if e == USAGE => {
            print!("{USAGE}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

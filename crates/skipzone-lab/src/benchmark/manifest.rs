//! Corpus manifests: what each frozen file is, where it came from, and its
//! SHA-256, checked before anything is read.
//!
//! The layout follows docs/notes/accuracy-roadmap.md §2.2. One `MANIFEST.tsv`
//! sits beside the data files of one corpus version:
//!
//! ```text
//! # corpus: wspr-v1
//! # synthetic: no
//! file    kind    sha256    rows    fetched_utc    source    licence    attribution
//! fit.tsv wspr    <64 hex>  9126    2026-11-05T10:00Z    <query or URL>    <licence>    <attribution>
//! ```
//!
//! Columns are tab-separated. `# key: value` lines carry corpus-wide
//! settings; the ones read are listed on [`Manifest`]. A `synthetic: yes`
//! line marks a test fixture, and every report made from it says so.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::sha256;

/// What a corpus file holds, which decides the loader it goes through.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Positives as `corpus::write_positives` writes them.
    Wspr,
    /// Negatives as `corpus::write_negatives` writes them.
    WsprNegatives,
    /// A Reverse Beacon Network day file, CSV with a header row.
    Rbn,
    /// NCDXF/IARU beacon slots, one decode or no-decode record per row.
    Beacon,
    /// GIRO ionosonde characteristics.
    Ionosonde,
    /// Callsign to Maidenhead locator, for corpora that carry no positions.
    Stations,
    /// Daily sunspot number, year, month, day, SSN.
    SsnDaily,
    /// Smoothed sunspot number R12 by month, for the P.533 inputs.
    R12,
}

impl Kind {
    /// The spelling used in the manifest's `kind` column.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Wspr => "wspr",
            Self::WsprNegatives => "wspr-negatives",
            Self::Rbn => "rbn",
            Self::Beacon => "beacon",
            Self::Ionosonde => "ionosonde",
            Self::Stations => "stations",
            Self::SsnDaily => "ssn-daily",
            Self::R12 => "r12",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        [
            Self::Wspr,
            Self::WsprNegatives,
            Self::Rbn,
            Self::Beacon,
            Self::Ionosonde,
            Self::Stations,
            Self::SsnDaily,
            Self::R12,
        ]
        .into_iter()
        .find(|k| k.label() == s)
    }
}

/// One row of a manifest, with the file's verified contents.
#[derive(Clone, Debug)]
pub struct Entry {
    /// Path relative to the manifest's directory.
    pub file: String,
    pub kind: Kind,
    pub sha256: String,
    /// Row count as the fetcher recorded it. Reported beside what the loader
    /// read; the hash is what is enforced.
    pub rows: Option<usize>,
    pub fetched_utc: String,
    pub source: String,
    pub licence: String,
    pub attribution: String,
    /// The file's text. Read once, hashed, and parsed from this same buffer,
    /// so what is scored is exactly what was verified.
    pub text: String,
}

/// A verified corpus manifest.
#[derive(Clone, Debug)]
pub struct Manifest {
    pub path: PathBuf,
    /// The corpus name: the `corpus:` setting, else the directory name.
    pub corpus: String,
    /// SHA-256 of the manifest file itself. It names the whole corpus in
    /// `results.tsv`, because it covers every file hash.
    pub sha256: String,
    /// `synthetic: yes` marks a test fixture.
    pub synthetic: bool,
    /// Every `# key: value` setting, for the report. Read here:
    /// `rbn_window_minutes`, `beacons_off`, `stride`.
    pub settings: BTreeMap<String, String>,
    pub entries: Vec<Entry>,
}

impl Manifest {
    /// Read a manifest and every file it lists, and check each hash.
    ///
    /// # Errors
    /// Any unreadable file, malformed row, path outside the corpus directory
    /// or hash mismatch. A mismatch names the file and both hashes.
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let text = String::from_utf8(bytes.clone())
            .map_err(|e| format!("{}: not UTF-8: {e}", path.display()))?;
        let dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut m = parse(&text, path)?;
        m.sha256 = sha256::hex(&bytes);
        for e in &mut m.entries {
            let p = dir.join(&e.file);
            let data = std::fs::read(&p).map_err(|err| format!("{}: {err}", p.display()))?;
            let got = sha256::hex(&data);
            if got != e.sha256 {
                return Err(format!(
                    "SHA-256 MISMATCH for {} (manifest {}): manifest says {}, file is {}. \
                     A frozen corpus must not change; fetch a new version instead.",
                    p.display(),
                    path.display(),
                    e.sha256,
                    got
                ));
            }
            e.text = String::from_utf8(data)
                .map_err(|err| format!("{}: not UTF-8: {err}", p.display()))?;
        }
        Ok(m)
    }

    /// Entries of one kind, in manifest order.
    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }

    /// A numeric setting, if present and readable.
    #[must_use]
    pub fn setting_f64(&self, key: &str) -> Option<f64> {
        self.settings.get(key).and_then(|v| v.parse().ok())
    }
}

/// Parse the manifest text without touching the data files. Hashes are not
/// checked here; [`Manifest::load`] does that.
///
/// # Errors
/// A malformed row, an unknown kind, a hash that is not 64 hex digits, or a
/// file path that leaves the corpus directory.
pub fn parse(text: &str, path: &Path) -> Result<Manifest, String> {
    let mut settings = BTreeMap::new();
    let mut entries = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let where_ = format!("{} line {}", path.display(), i + 1);
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        if let Some(c) = t.strip_prefix('#') {
            if let Some((k, v)) = c.split_once(':') {
                settings.insert(k.trim().to_string(), v.trim().to_string());
            }
            continue;
        }
        let f: Vec<&str> = line.split('\t').map(str::trim).collect();
        if f[0] == "file" {
            continue; // the column header
        }
        if f.len() < 8 {
            return Err(format!(
                "{where_}: expected 8 tab-separated fields, got {}",
                f.len()
            ));
        }
        let kind = Kind::parse(f[1]).ok_or_else(|| format!("{where_}: unknown kind {:?}", f[1]))?;
        let file = f[0].to_string();
        // A manifest names files beside it, nothing else.
        if file.starts_with('/') || file.split(['/', '\\']).any(|c| c == "..") {
            return Err(format!(
                "{where_}: {file:?} is outside the corpus directory"
            ));
        }
        let sha = f[2].to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!("{where_}: {:?} is not a SHA-256 hex digest", f[2]));
        }
        entries.push(Entry {
            file,
            kind,
            sha256: sha,
            rows: f[3].parse().ok(),
            fetched_utc: f[4].to_string(),
            source: f[5].to_string(),
            licence: f[6].to_string(),
            attribution: f[7].to_string(),
            text: String::new(),
        });
    }
    if entries.is_empty() {
        return Err(format!("{}: lists no files", path.display()));
    }
    let corpus = settings.get("corpus").cloned().unwrap_or_else(|| {
        path.parent().and_then(Path::file_name).map_or_else(
            || "corpus".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    });
    let synthetic = settings
        .get("synthetic")
        .is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true" | "1"));
    Ok(Manifest {
        path: path.to_path_buf(),
        corpus,
        sha256: String::new(),
        synthetic,
        settings,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("skipzone-bench-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn manifest_for(file: &str, sha: &str) -> String {
        format!(
            "# corpus: t-v1\n# synthetic: yes\n\
             file\tkind\tsha256\trows\tfetched_utc\tsource\tlicence\tattribution\n\
             {file}\tssn-daily\t{sha}\t1\t-\tsynthetic\tnone\tnone\n"
        )
    }

    #[test]
    fn a_matching_hash_loads_and_keeps_the_text() {
        let d = scratch("ok");
        std::fs::write(d.join("ssn.tsv"), "2026\t1\t1\t100\n").unwrap();
        let sha = sha256::hex(b"2026\t1\t1\t100\n");
        std::fs::write(d.join("MANIFEST.tsv"), manifest_for("ssn.tsv", &sha)).unwrap();
        let m = Manifest::load(&d.join("MANIFEST.tsv")).unwrap();
        assert_eq!(m.corpus, "t-v1");
        assert!(m.synthetic);
        assert_eq!(m.entries[0].text, "2026\t1\t1\t100\n");
        assert_eq!(m.sha256.len(), 64);
    }

    #[test]
    fn a_changed_file_fails_loudly() {
        let d = scratch("bad");
        std::fs::write(d.join("ssn.tsv"), "2026\t1\t1\t101\n").unwrap();
        let sha = sha256::hex(b"2026\t1\t1\t100\n");
        std::fs::write(d.join("MANIFEST.tsv"), manifest_for("ssn.tsv", &sha)).unwrap();
        let err = Manifest::load(&d.join("MANIFEST.tsv")).unwrap_err();
        assert!(err.contains("SHA-256 MISMATCH"), "{err}");
        assert!(err.contains("ssn.tsv"), "{err}");
    }

    #[test]
    fn paths_outside_the_corpus_are_refused() {
        let sha = "0".repeat(64);
        for bad in ["../x.tsv", "/etc/passwd", "a/../../b"] {
            let err = parse(&manifest_for(bad, &sha), Path::new("m/MANIFEST.tsv")).unwrap_err();
            assert!(err.contains("outside"), "{bad}: {err}");
        }
    }

    #[test]
    fn a_malformed_hash_is_refused() {
        let err = parse(&manifest_for("a.tsv", "abc"), Path::new("MANIFEST.tsv")).unwrap_err();
        assert!(err.contains("SHA-256"), "{err}");
    }
}

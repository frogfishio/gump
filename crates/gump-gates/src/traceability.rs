//! Traceability ledger checks (CONFORMANCE.md §9 / DELIVERY W04).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::path::Path;

/// Allowed `status` column values.
pub const ALLOWED_STATUS: &[&str] = &["missing", "blocked", "implemented", "waived"];

/// Expected header columns, in order.
pub const HEADER: &[&str] = &[
    "requirement_id",
    "document",
    "section",
    "owner_crate",
    "test_name",
    "evidence_path",
    "status",
    "ticket",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Row {
    pub requirement_id: String,
    pub document: String,
    pub section: String,
    pub owner_crate: String,
    pub test_name: String,
    pub evidence_path: String,
    pub status: String,
    /// Owning delivery ticket (`GUMP-N###`) required while status is missing/blocked (N003 / W04).
    pub ticket: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    /// Structural integrity only (duplicates, columns, known status values).
    Structural,
    /// Release / strict CI: also reject `missing` and `blocked`.
    Strict,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Report {
    pub errors: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.errors.is_empty()
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ok() {
            write!(f, "traceability: ok")
        } else {
            writeln!(f, "traceability: FAILED")?;
            for e in &self.errors {
                writeln!(f, "  - {e}")?;
            }
            Ok(())
        }
    }
}

/// Parse a TSV ledger into rows (skips blank lines; requires header).
pub fn parse_tsv(text: &str) -> Result<Vec<Row>, String> {
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header = lines
        .next()
        .ok_or_else(|| "empty traceability ledger".to_string())?;
    let cols: Vec<&str> = header.split('\t').collect();
    if cols != HEADER {
        return Err(format!("bad header: expected {:?}, got {:?}", HEADER, cols));
    }

    let mut rows = Vec::new();
    for (idx, line) in lines.enumerate() {
        let line_no = idx + 2; // 1-based, after header
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() != HEADER.len() {
            return Err(format!(
                "line {line_no}: expected {} columns, got {}",
                HEADER.len(),
                parts.len()
            ));
        }
        rows.push(Row {
            requirement_id: parts[0].to_string(),
            document: parts[1].to_string(),
            section: parts[2].to_string(),
            owner_crate: parts[3].to_string(),
            test_name: parts[4].to_string(),
            evidence_path: parts[5].to_string(),
            status: parts[6].to_string(),
            ticket: parts[7].to_string(),
        });
    }
    Ok(rows)
}

/// Validate parsed rows under `mode`.
pub fn check_rows(rows: &[Row], known_crates: &BTreeSet<&str>, mode: Mode) -> Report {
    let mut report = Report::default();
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();

    for (i, row) in rows.iter().enumerate() {
        let line_no = i + 2;
        if row.requirement_id.is_empty() {
            report
                .errors
                .push(format!("line {line_no}: empty requirement_id"));
        }
        if let Some(prev) = seen.insert(row.requirement_id.as_str(), line_no) {
            report.errors.push(format!(
                "duplicate requirement_id {} (lines {prev} and {line_no})",
                row.requirement_id
            ));
        }
        if !ALLOWED_STATUS.contains(&row.status.as_str()) {
            report.errors.push(format!(
                "line {line_no}: unknown status {:?} for {}",
                row.status, row.requirement_id
            ));
        }
        if !known_crates.contains(row.owner_crate.as_str()) {
            report.errors.push(format!(
                "line {line_no}: unknown owner_crate {:?} for {}",
                row.owner_crate, row.requirement_id
            ));
        }
        if mode == Mode::Strict && (row.status == "missing" || row.status == "blocked") {
            report.errors.push(format!(
                "strict: {} has status {} (release CI rejects missing/blocked)",
                row.requirement_id, row.status
            ));
        }
        if row.status == "implemented"
            && (row.test_name == "pending" || row.evidence_path == "pending")
        {
            report.errors.push(format!(
                "line {line_no}: {} marked implemented but test/evidence still pending",
                row.requirement_id
            ));
        }
        // Ordinary / PR CI: every non-implemented invariant must name an owned ticket
        // so the release ledger cannot sit open against an empty work queue (N003).
        if (row.status == "missing" || row.status == "blocked") && !is_owned_ticket(&row.ticket) {
            report.errors.push(format!(
                "line {line_no}: {} status {} requires ticket GUMP-N### (got {:?})",
                row.requirement_id, row.status, row.ticket
            ));
        }
    }

    report
}

/// Delivery ticket id from `docs/v1/NEXT_ACTIONS.md` (`GUMP-N001` …).
pub fn is_owned_ticket(ticket: &str) -> bool {
    let rest = match ticket.strip_prefix("GUMP-N") {
        Some(r) => r,
        None => return false,
    };
    rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit())
}

/// Load ledger from disk and check.
pub fn check_file(
    path: &Path,
    known_crates: &BTreeSet<&str>,
    mode: Mode,
) -> Result<Report, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let rows = parse_tsv(&text)?;
    Ok(check_rows(&rows, known_crates, mode))
}

/// Product crates that may own requirements (excludes gates tooling).
pub fn default_owner_crates() -> BTreeSet<&'static str> {
    [
        "gump-types",
        "gump-cli",
        "gump-manifest",
        "gump-capsule",
        "gump-crypto",
        "gump-protocol",
        "gump-memory",
        "gump-transport",
        "gump-scheduler",
        "gump-agent",
        "gump-driver",
        "gump-telemetry",
        "gump-hiccup",
        "gump-connectors",
        "gump-server",
    ]
    .into_iter()
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ok_ledger() -> String {
        let mut s = HEADER.join("\t");
        s.push('\n');
        s.push_str(
            "INV-001\tdocs/v1/CONFORMANCE.md\t3\tgump-crypto\tcanary_scan\tspec/v1/evidence/inv001.md\timplemented\t\n",
        );
        s.push_str(
            "INV-002\tdocs/v1/CONFORMANCE.md\t3\tgump-capsule\tcorrupt_layers\tspec/v1/evidence/inv002.md\timplemented\t\n",
        );
        s
    }

    #[test]
    fn structural_accepts_missing_rows() {
        let text = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/traceability.tsv"),
        )
        .unwrap();
        let rows = parse_tsv(&text).unwrap();
        let report = check_rows(&rows, &default_owner_crates(), Mode::Structural);
        assert!(report.ok(), "{report}");
    }

    #[test]
    fn strict_rejects_missing_status() {
        let text = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/traceability.tsv"),
        )
        .unwrap();
        let rows = parse_tsv(&text).unwrap();
        let report = check_rows(&rows, &default_owner_crates(), Mode::Strict);
        assert!(
            !report.ok(),
            "expected strict failure on missing requirements"
        );
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("INV-001") && e.contains("missing")),
            "{report}"
        );
    }

    #[test]
    fn rejects_duplicate_ids() {
        let mut text = sample_ok_ledger();
        text.push_str(
            "INV-001\tdocs/v1/CONFORMANCE.md\t3\tgump-crypto\tother\tspec/v1/evidence/x.md\timplemented\t\n",
        );
        let rows = parse_tsv(&text).unwrap();
        let report = check_rows(&rows, &default_owner_crates(), Mode::Structural);
        assert!(!report.ok());
        assert!(report.errors.iter().any(|e| e.contains("duplicate")));
    }

    #[test]
    fn rejects_unknown_crate() {
        let mut s = HEADER.join("\t");
        s.push('\n');
        s.push_str("INV-001\tdocs/v1/CONFORMANCE.md\t3\tnot-a-crate\tt\te\timplemented\t\n");
        let rows = parse_tsv(&s).unwrap();
        let report = check_rows(&rows, &default_owner_crates(), Mode::Structural);
        assert!(!report.ok());
        assert!(
            report
                .errors
                .iter()
                .any(|e| e.contains("unknown owner_crate"))
        );
    }

    #[test]
    fn missing_status_requires_owned_ticket() {
        let mut s = HEADER.join("\t");
        s.push('\n');
        s.push_str(
            "INV-001\tdocs/v1/CONFORMANCE.md\t3\tgump-crypto\tpending\tpending\tmissing\t\n",
        );
        let rows = parse_tsv(&s).unwrap();
        let report = check_rows(&rows, &default_owner_crates(), Mode::Structural);
        assert!(!report.ok(), "{report}");
        assert!(
            report.errors.iter().any(|e| e.contains("requires ticket")),
            "{report}"
        );
    }

    #[test]
    fn owned_ticket_shape() {
        assert!(is_owned_ticket("GUMP-N003"));
        assert!(is_owned_ticket("GUMP-N017"));
        assert!(!is_owned_ticket(""));
        assert!(!is_owned_ticket("N003"));
        assert!(!is_owned_ticket("GUMP-N3"));
        assert!(!is_owned_ticket("GUMP-N0003"));
    }

    #[test]
    fn missing_requirement_fails_check_file_strict() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../spec/v1/traceability.tsv");
        let report = check_file(&path, &default_owner_crates(), Mode::Strict).unwrap();
        assert!(!report.ok());
        // Demonstrates W04 exit: missing requirement → non-success.
        assert!(report.errors.iter().any(|e| e.contains("strict:")));
    }
}

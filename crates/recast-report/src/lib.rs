//! The `recast-report/1` run report.
//!
//! One report records what a run established, what it did not, and on which
//! evidence. Document 07 section 6 fixes the schema: keys in a fixed order,
//! `items` sorted by stable identifier, and a limitations statement in the
//! human-readable rendering. This crate owns the types, the checks a report
//! must pass before it is published, and the Markdown rendering.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub mod gate;

/// Schema name carried by every report.
pub const SCHEMA: &str = "recast-report/1";

/// The limitations statement rendered verbatim in `report.md`.
pub const LIMITATIONS: &str = "\
Tests and fuzzing do not prove equivalence. A passing run establishes that the \
observed behavior agreed on the inputs that were executed; it says nothing \
about inputs that were not executed. Timing, performance, memory addresses, \
thread scheduling and other nondeterminism are excluded from the comparison. \
Rungs that were not run are listed under skipped rungs and are not evidence.";

/// The run-level verdict. The report never hides an item-level verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Verdict {
    /// Everything required by the profile passed.
    Accept,
    /// Everything passed, with notes a human should read.
    AcceptWithNotes,
    /// At least one item or rule failed.
    Reject,
}

impl Verdict {
    /// The spelling used in `report.md`.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Accept => "ACCEPT",
            Verdict::AcceptWithNotes => "ACCEPT_WITH_NOTES",
            Verdict::Reject => "REJECT",
        }
    }
}

/// One item's outcome, using the same values as the run verdict.
pub type ItemVerdict = Verdict;

/// A canonical `recast-report/1` document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub schema: String,
    pub run: Run,
    pub verdict: Verdict,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub determinism: Option<Determinism>,
    pub summary: Summary,
    #[serde(default)]
    pub evidence: BTreeMap<String, usize>,
    pub items: Vec<Item>,
    #[serde(default)]
    pub flagged: Vec<Flagged>,
    #[serde(default)]
    pub ub_sites: Vec<UbSite>,
    #[serde(rename = "unsafe_inventory", default)]
    pub unsafe_blocks: Vec<UnsafeBlock>,
    #[serde(default)]
    pub divergences: Vec<Divergence>,
    #[serde(default)]
    pub skipped_rungs: Vec<SkippedRung>,
    #[serde(default)]
    pub findings: Vec<Finding>,
}

/// Everything that identifies the run: versions, input hashes and environment.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub tool_version: String,
    pub rule_set_hash: String,
    pub schema_versions: BTreeMap<String, u32>,
    pub input: Input,
    #[serde(default)]
    pub toolchains: BTreeMap<String, String>,
    pub target: Target,
    pub profile: String,
    pub started_at: String,
    #[serde(default)]
    pub environment: Option<Environment>,
}

/// Hashes of the inputs a report can be reproduced from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Input {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_commands_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub config_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contracts_hash: Option<String>,
}

/// Target facts that decide integer widths and pointer layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub triple: String,
    pub int_bits: u8,
    pub pointer_bits: u8,
}

/// Where a run happened. Excluded from the determinism hash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub os: String,
    pub cpu: String,
    pub jobs: usize,
}

/// The determinism check result (GATE-005).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Determinism {
    pub checked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_hash: Option<String>,
    pub runs: u32,
    pub identical: bool,
}

/// Counts and metrics for the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    pub items: ItemCounts,
    #[serde(default)]
    pub by_tier: BTreeMap<String, TierCounts>,
    #[serde(rename = "unsafe")]
    pub unsafe_counts: UnsafeCounts,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage: Option<Coverage>,
}

/// Item totals for the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ItemCounts {
    pub total: usize,
    pub translated: usize,
    pub flagged_t4: usize,
    pub rejected: usize,
}

/// Functions per tier, with lines of code when measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct TierCounts {
    pub functions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc: Option<u64>,
}

/// `unsafe` inventory counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UnsafeCounts {
    pub generated_code_blocks: usize,
    pub runtime_crate_blocks: usize,
    pub ffi_boundary_blocks: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsafe_per_kloc: Option<f64>,
}

/// How much of the translated code the differential exercised.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Coverage {
    pub functions_exercised_pct: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_pct: Option<f64>,
}

/// One translated item and the evidence for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub source: Location,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rust: Option<RustLocation>,
    pub tier: String,
    #[serde(default)]
    pub rules: Vec<String>,
    pub verdict: ItemVerdict,
    pub evidence_class: String,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub ub_sites: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

/// A source location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub file: String,
    pub line: u32,
}

/// The generated Rust item an item maps to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustLocation {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// One rung's result for one item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub rung: String,
    pub result: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executions: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coverage_pct: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget: Option<String>,
}

/// An assumption the translation rests on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assumption {
    pub fact: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified_by: Option<String>,
}

/// A construct that could not be translated and was flagged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Flagged {
    pub id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stub: Option<String>,
    #[serde(default)]
    pub evidence: Vec<Evidence>,
}

/// A UB site with the original and translated behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UbSite {
    pub id: String,
    pub location: Location,
    pub confidence: String,
    pub original_behavior: String,
    pub translated_behavior: String,
    pub policy: String,
}

/// One `unsafe` block in the generated code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsafeBlock {
    pub file: String,
    pub line: u32,
    #[serde(rename = "fn")]
    pub function: String,
    pub tier: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub safety_comment: Option<String>,
    #[serde(default)]
    pub tests: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub miri: Option<String>,
}

/// A divergence between the original and the translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Divergence {
    pub id: String,
    pub class: String,
    pub reproducer: String,
    pub resolution: String,
}

/// A rung that did not run, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRung {
    pub rung: String,
    pub reason: String,
}

/// An analysis finding carried into the report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub rule: String,
    pub function: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Report {
    /// Check the invariants a published report must hold.
    ///
    /// Returns every violation it finds, not only the first, because a caller
    /// fixing a report needs the whole list.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();
        if self.schema != SCHEMA {
            errors.push(format!(
                "{SCHEMA}: expected schema {SCHEMA}, found {}",
                self.schema
            ));
        }
        let mut previous: Option<&str> = None;
        for item in &self.items {
            if item.id.is_empty() {
                errors.push("items: an item has an empty id".to_string());
            }
            if let Some(last) = previous {
                if last >= item.id.as_str() {
                    errors.push(format!(
                        "items: `{}` is not after `{last}`; items must be sorted by id and unique",
                        item.id
                    ));
                }
            }
            previous = Some(&item.id);
            if !matches!(item.tier.as_str(), "T0" | "T1" | "T2" | "T3" | "T4") {
                errors.push(format!("items: {} has unknown tier {}", item.id, item.tier));
            }
            if !matches!(
                item.evidence_class.as_str(),
                "E0" | "E1" | "E2" | "E3" | "E4" | "E5"
            ) {
                errors.push(format!(
                    "items: {} has unknown evidence class {}",
                    item.id, item.evidence_class
                ));
            }
            for evidence in &item.evidence {
                if !matches!(
                    evidence.rung.as_str(),
                    "V0" | "V1" | "V2" | "V3" | "V4" | "V5"
                ) {
                    errors.push(format!(
                        "items: {} has unknown rung {}",
                        item.id, evidence.rung
                    ));
                }
            }
        }
        for rung in &self.skipped_rungs {
            if !matches!(rung.rung.as_str(), "V0" | "V1" | "V2" | "V3" | "V4" | "V5") {
                errors.push(format!("skipped_rungs: unknown rung {}", rung.rung));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Render the report as `report.md`.
    ///
    /// The limitations statement is always present and never shortened.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        writeln!(out, "# recast report").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(
            out,
            "**Verdict: {}** (profile `{}`, recast {})",
            self.verdict.as_str(),
            self.run.profile,
            self.run.tool_version
        )
        .expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        let items = &self.summary.items;
        writeln!(
            out,
            "{} of {} items were translated; {} were rejected and {} were flagged for a boundary stub. The differential exercised {} of the {} functions.",
            items.translated,
            items.total,
            items.rejected,
            items.flagged_t4,
            self.summary
                .coverage
                .as_ref()
                .map(|coverage| {
                    format!("{:.1}%", coverage.functions_exercised_pct)
                })
                .unwrap_or_else(|| "0%".to_string()),
            items.total,
        )
        .expect("writing to a String cannot fail");

        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "## Key numbers").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "| Metric | Value |").expect("writing to a String cannot fail");
        writeln!(out, "| --- | --- |").expect("writing to a String cannot fail");
        writeln!(out, "| Items | {} |", items.total).expect("writing to a String cannot fail");
        for (tier, counts) in &self.summary.by_tier {
            writeln!(out, "| {tier} functions | {} |", counts.functions)
                .expect("writing to a String cannot fail");
        }
        writeln!(
            out,
            "| Generated `unsafe` blocks | {} |",
            self.summary.unsafe_counts.generated_code_blocks
        )
        .expect("writing to a String cannot fail");
        if !self.evidence.is_empty() {
            let classes = self
                .evidence
                .iter()
                .map(|(class, count)| format!("{class} {count}"))
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out, "| Evidence classes | {classes} |")
                .expect("writing to a String cannot fail");
        }

        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "## What needs human attention").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        let mut attention = Vec::new();
        for item in &self.items {
            if item.verdict != Verdict::Accept {
                attention.push(format!(
                    "- `{}` is {} ({})",
                    item.id,
                    item.verdict.as_str(),
                    item.source.file
                ));
            }
            if item.evidence_class == "E0" {
                attention.push(format!(
                    "- `{}` compiles but has no differential evidence ({})",
                    item.id, item.source.file
                ));
            }
            for note in &item.notes {
                attention.push(format!("- `{}`: {note}", item.id));
            }
        }
        for flagged in &self.flagged {
            attention.push(format!(
                "- `{}` was flagged: {}",
                flagged.id, flagged.reason
            ));
        }
        for site in &self.ub_sites {
            attention.push(format!(
                "- `{}` at {}:{} is {} ({})",
                site.id, site.location.file, site.location.line, site.confidence, site.policy
            ));
        }
        for divergence in &self.divergences {
            attention.push(format!(
                "- divergence `{}` ({}) in {}",
                divergence.id, divergence.class, divergence.reproducer
            ));
        }
        for finding in &self.findings {
            attention.push(format!(
                "- {} at {}:{}: {}",
                finding.rule, finding.file, finding.line, finding.message
            ));
        }
        if attention.is_empty() {
            attention
                .push("- Nothing: every translated item passed the rungs that ran.".to_string());
        }
        for line in attention {
            writeln!(out, "{line}").expect("writing to a String cannot fail");
        }

        if !self.skipped_rungs.is_empty() {
            writeln!(out).expect("writing to a String cannot fail");
            writeln!(out, "### Skipped rungs").expect("writing to a String cannot fail");
            writeln!(out).expect("writing to a String cannot fail");
            for rung in &self.skipped_rungs {
                writeln!(out, "- {}: {}", rung.rung, rung.reason)
                    .expect("writing to a String cannot fail");
            }
        }

        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "## Items").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "| Item | Tier | Evidence | Verdict |")
            .expect("writing to a String cannot fail");
        writeln!(out, "| --- | --- | --- | --- |").expect("writing to a String cannot fail");
        for item in &self.items {
            writeln!(
                out,
                "| `{}` | {} | {} | {} |",
                item.id,
                item.tier,
                item.evidence_class,
                item.verdict.as_str()
            )
            .expect("writing to a String cannot fail");
        }

        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "## Reproducibility").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "- Started: {}", self.run.started_at)
            .expect("writing to a String cannot fail");
        writeln!(out, "- Rule set: {}", self.run.rule_set_hash)
            .expect("writing to a String cannot fail");
        for (name, version) in &self.run.schema_versions {
            writeln!(out, "- Schema {name}: {version}").expect("writing to a String cannot fail");
        }
        for (name, hash) in [
            ("Source", &self.run.input.source_hash),
            ("Compile commands", &self.run.input.compile_commands_hash),
            ("Config", &self.run.input.config_hash),
            ("Contracts", &self.run.input.contracts_hash),
        ] {
            if let Some(hash) = hash {
                writeln!(out, "- {name}: {hash}").expect("writing to a String cannot fail");
            }
        }
        if let Some(environment) = &self.run.environment {
            writeln!(
                out,
                "- Environment: {} on {}, {} job(s)",
                environment.os, environment.cpu, environment.jobs
            )
            .expect("writing to a String cannot fail");
        }

        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "## Limitations").expect("writing to a String cannot fail");
        writeln!(out).expect("writing to a String cannot fail");
        writeln!(out, "{LIMITATIONS}").expect("writing to a String cannot fail");
        out
    }
}

/// Hash bytes as `sha256:<lowercase hex>`.
pub fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{digest:x}")
}

/// Hash a file's bytes, or `None` when the file cannot be read.
pub fn hash_file(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|bytes| hash_bytes(&bytes))
}

/// Format seconds since the Unix epoch as an ISO-8601 UTC timestamp.
pub fn iso8601(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let rem = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3_600,
        (rem % 3_600) / 60,
        rem % 60
    )
}

/// The current time as an ISO-8601 UTC timestamp.
pub fn now_iso8601() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    iso8601(seconds)
}

/// Days since the Unix epoch to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    if month <= 2 {
        year += 1;
    }
    (year, month, day)
}

/// Describe the machine a run happened on.
pub fn environment(jobs: usize) -> Environment {
    Environment {
        os: std::env::consts::OS.to_string(),
        cpu: cpu_name(),
        jobs,
    }
}

fn cpu_name() -> String {
    std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split_once(':'))
                .map(|(_, value)| value.trim().to_string())
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        Report {
            schema: SCHEMA.to_string(),
            run: Run {
                tool_version: "1.3.0".to_string(),
                rule_set_hash: hash_bytes(b"M1"),
                schema_versions: BTreeMap::from([("ast".to_string(), 1), ("tir".to_string(), 1)]),
                input: Input::default(),
                toolchains: BTreeMap::new(),
                target: Target {
                    triple: "x86_64-unknown-linux-gnu".to_string(),
                    int_bits: 32,
                    pointer_bits: 64,
                },
                profile: "standard".to_string(),
                started_at: "2026-11-03T00:00:00Z".to_string(),
                environment: None,
            },
            verdict: Verdict::Accept,
            determinism: None,
            summary: Summary {
                items: ItemCounts {
                    total: 1,
                    translated: 1,
                    flagged_t4: 0,
                    rejected: 0,
                },
                by_tier: BTreeMap::from([(
                    "T0".to_string(),
                    TierCounts {
                        functions: 1,
                        loc: None,
                    },
                )]),
                unsafe_counts: UnsafeCounts::default(),
                coverage: Some(Coverage {
                    functions_exercised_pct: 100.0,
                    lines_pct: None,
                }),
            },
            evidence: BTreeMap::from([("E2".to_string(), 1)]),
            items: vec![Item {
                id: "fn:c:@F@sum".to_string(),
                source: Location {
                    file: "sum.c".to_string(),
                    line: 3,
                },
                rust: Some(RustLocation {
                    path: "sum".to_string(),
                    file: Some("rust/src/lib.rs".to_string()),
                    line: None,
                }),
                tier: "T0".to_string(),
                rules: Vec::new(),
                verdict: Verdict::Accept,
                evidence_class: "E2".to_string(),
                evidence: vec![Evidence {
                    rung: "V2".to_string(),
                    result: "pass".to_string(),
                    executions: Some(2),
                    coverage_pct: None,
                    seed: None,
                    budget: None,
                }],
                assumptions: Vec::new(),
                ub_sites: Vec::new(),
                notes: Vec::new(),
            }],
            flagged: Vec::new(),
            ub_sites: Vec::new(),
            unsafe_blocks: Vec::new(),
            divergences: Vec::new(),
            skipped_rungs: vec![SkippedRung {
                rung: "V1".to_string(),
                reason: "not implemented in this build point".to_string(),
            }],
            findings: Vec::new(),
        }
    }

    #[test]
    fn epoch_and_a_known_timestamp_format() {
        assert_eq!(iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso8601(1_700_000_000), "2023-11-14T22:13:20Z");
    }

    #[test]
    fn hashes_have_the_documented_shape() {
        assert_eq!(
            hash_bytes(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn validation_rejects_unsorted_items() {
        let mut report = report();
        report.items.push(report.items[0].clone());
        let errors = report
            .validate()
            .expect_err("duplicate id must be rejected");
        assert!(errors[0].contains("sorted by id and unique"), "{errors:?}");
    }

    #[test]
    fn serialization_keeps_the_schema_key_order() {
        let json = serde_json::to_string(&report()).expect("serialize");
        let positions = [
            json.find("\"schema\"").expect("schema"),
            json.find("\"run\"").expect("run"),
            json.find("\"verdict\"").expect("verdict"),
            json.find("\"summary\"").expect("summary"),
            json.find("\"items\"").expect("items"),
            json.find("\"findings\"").expect("findings"),
        ];
        assert!(positions.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            json.contains("\"unsafe_inventory\""),
            "the report uses document 07's key for the unsafe inventory: {json}"
        );
    }

    #[test]
    fn markdown_always_carries_the_limitations_statement() {
        let markdown = report().to_markdown();
        assert!(markdown.contains("**Verdict: ACCEPT**"), "{markdown}");
        assert!(markdown.contains(LIMITATIONS), "{markdown}");
        assert!(markdown.contains("## Limitations"), "{markdown}");
    }

    #[test]
    fn round_trip_reports_are_equal() {
        let report = report();
        let json = serde_json::to_string(&report).expect("serialize");
        let back: Report = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(report, back);
        assert!(report.validate().is_ok());
    }
}

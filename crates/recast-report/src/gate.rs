//! Gate profiles and rule evaluation.
//!
//! The gate consumes a [`Report`] and returns a decision. It never re-runs
//! verification: a rule either reads the recorded evidence or records that the
//! evidence is missing. That separation is what lets `recast gate` answer
//! "would a stricter profile have accepted this run?" without translating or
//! executing anything again.

use std::fmt;
use std::str::FromStr;

use crate::{ItemVerdict, Report, Verdict};

/// A named policy bundle (document 07 section 4.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// T1 at most, every rung, unexercised functions rejected.
    Strict,
    /// T2 at most, V0/V1/V2/V4, unexercised functions noted.
    Standard,
    /// T3 at most, V0/V1, most findings noted rather than rejected.
    Permissive,
    /// Reports every rule but never rejects; intended for evaluation.
    Audit,
}

impl Profile {
    /// The spelling used in configuration and reports.
    pub const fn as_str(self) -> &'static str {
        match self {
            Profile::Strict => "strict",
            Profile::Standard => "standard",
            Profile::Permissive => "permissive",
            Profile::Audit => "audit",
        }
    }

    /// The policy this profile enforces.
    pub const fn policy(self) -> Policy {
        match self {
            Profile::Strict => Policy {
                max_tier: 1,
                allow_t3: false,
                allow_t4: false,
                require_rungs: &["V0", "V1", "V2", "V3", "V4"],
                unexercised: Disposition::Reject,
                require_verified_assumptions: true,
                enforce: true,
            },
            Profile::Standard => Policy {
                max_tier: 2,
                allow_t3: true,
                allow_t4: true,
                require_rungs: &["V0", "V1", "V2", "V4"],
                unexercised: Disposition::Note,
                require_verified_assumptions: false,
                enforce: true,
            },
            Profile::Permissive => Policy {
                max_tier: 3,
                allow_t3: true,
                allow_t4: true,
                require_rungs: &["V0", "V1"],
                unexercised: Disposition::Note,
                require_verified_assumptions: false,
                enforce: true,
            },
            Profile::Audit => Policy {
                max_tier: 4,
                allow_t3: true,
                allow_t4: true,
                require_rungs: &[],
                unexercised: Disposition::Note,
                require_verified_assumptions: false,
                enforce: false,
            },
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Profile {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "strict" => Ok(Profile::Strict),
            "standard" => Ok(Profile::Standard),
            "permissive" => Ok(Profile::Permissive),
            "audit" => Ok(Profile::Audit),
            other => Err(format!(
                "unknown profile `{other}`; expected strict, standard, permissive or audit"
            )),
        }
    }
}

/// How a profile treats something it allows but does not require.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    /// Reject the run.
    Reject,
    /// Accept the run and put the item under human attention.
    Note,
}

/// The resolved policy of a profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Policy {
    pub max_tier: u8,
    pub allow_t3: bool,
    pub allow_t4: bool,
    pub require_rungs: &'static [&'static str],
    pub unexercised: Disposition,
    pub require_verified_assumptions: bool,
    /// `false` only for `audit`: every rule still runs, but a failure is a note.
    pub enforce: bool,
}

/// The outcome of one gate rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleStatus {
    Pass,
    Fail,
    Note,
    NotApplicable,
}

impl RuleStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            RuleStatus::Pass => "pass",
            RuleStatus::Fail => "fail",
            RuleStatus::Note => "note",
            RuleStatus::NotApplicable => "not_applicable",
        }
    }
}

/// One gate rule's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleResult {
    pub id: &'static str,
    pub status: RuleStatus,
    pub message: String,
}

/// The gate's decision over one report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateResult {
    pub profile: Profile,
    pub verdict: Verdict,
    pub rules: Vec<RuleResult>,
}

impl GateResult {
    /// Rules that failed.
    pub fn failures(&self) -> impl Iterator<Item = &RuleResult> {
        self.rules
            .iter()
            .filter(|rule| rule.status == RuleStatus::Fail)
    }

    /// Rules that produced a note.
    pub fn notes(&self) -> impl Iterator<Item = &RuleResult> {
        self.rules
            .iter()
            .filter(|rule| rule.status == RuleStatus::Note)
    }

    /// Serialize the decision as `recast-gate/1`.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        #[derive(serde::Serialize)]
        struct Document<'a> {
            schema: &'a str,
            profile: &'a str,
            verdict: &'a str,
            rules: Vec<Rule<'a>>,
        }
        #[derive(serde::Serialize)]
        struct Rule<'a> {
            id: &'a str,
            status: &'a str,
            message: &'a str,
        }
        let mut text = serde_json::to_string_pretty(&Document {
            schema: "recast-gate/1",
            profile: self.profile.as_str(),
            verdict: self.verdict.as_str(),
            rules: self
                .rules
                .iter()
                .map(|rule| Rule {
                    id: rule.id,
                    status: rule.status.as_str(),
                    message: &rule.message,
                })
                .collect(),
        })?;
        text.push('\n');
        Ok(text)
    }

    /// Render the decision for a terminal.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "gate {}: {} ({} rule(s), {} failed)\n",
            self.profile,
            self.verdict.as_str(),
            self.rules.len(),
            self.failures().count()
        );
        for rule in &self.rules {
            if rule.status != RuleStatus::Pass {
                text.push_str(&format!(
                    "  {:<10} {} {}\n",
                    rule.id,
                    rule.status.as_str(),
                    rule.message
                ));
            }
        }
        text
    }
}

/// Evaluate every gate rule against `report`.
///
/// `crate_forbids_unsafe` is `Some(true)` when the generated crate carries
/// `#![forbid(unsafe_code)]`; `None` means the gate could not inspect the
/// crate, and GATE-001 is reported as not applicable rather than passing.
pub fn evaluate(
    report: &Report,
    profile: Profile,
    crate_forbids_unsafe: Option<bool>,
) -> GateResult {
    let policy = profile.policy();
    let mut rules = vec![
        gate_001(report, crate_forbids_unsafe),
        gate_002(report),
        gate_003(report, policy),
        gate_004(report, policy),
        gate_005(report, policy),
        gate_006(report, policy),
        gate_007(report),
        RuleResult {
            id: "GATE-008",
            status: RuleStatus::NotApplicable,
            message:
                "no exported FFI types in this report; layout assertions are not generated yet"
                    .to_string(),
        },
        gate_009(report),
    ];
    rules.extend(profile_rules(report, profile, policy));

    let failed = rules.iter().any(|rule| rule.status == RuleStatus::Fail);
    let noted = rules.iter().any(|rule| rule.status == RuleStatus::Note)
        || report
            .items
            .iter()
            .any(|item| item.verdict != ItemVerdict::Accept)
        || !report.flagged.is_empty()
        || !report.ub_sites.is_empty()
        || !report.divergences.is_empty()
        || !report.skipped_rungs.is_empty();
    let verdict = if failed && policy.enforce {
        Verdict::Reject
    } else if failed || noted {
        Verdict::AcceptWithNotes
    } else {
        Verdict::Accept
    };
    GateResult {
        profile,
        verdict,
        rules,
    }
}

fn gate_001(report: &Report, crate_forbids_unsafe: Option<bool>) -> RuleResult {
    match crate_forbids_unsafe {
        Some(true) => {
            let blocks = report.unsafe_blocks.len();
            let counted = report.summary.unsafe_counts.generated_code_blocks;
            if blocks == 0 && counted == 0 {
                RuleResult {
                    id: "GATE-001",
                    status: RuleStatus::Pass,
                    message: "the generated crate forbids unsafe code and contains none"
                        .to_string(),
                }
            } else {
                RuleResult {
                    id: "GATE-001",
                    status: RuleStatus::Fail,
                    message: format!(
                        "the generated crate forbids unsafe code but the report lists {blocks} block(s) and counts {counted}"
                    ),
                }
            }
        }
        Some(false) => RuleResult {
            id: "GATE-001",
            status: RuleStatus::NotApplicable,
            message: "the generated crate does not declare #![forbid(unsafe_code)]".to_string(),
        },
        None => RuleResult {
            id: "GATE-001",
            status: RuleStatus::NotApplicable,
            message: "the generated crate was not available to the gate".to_string(),
        },
    }
}

fn gate_002(report: &Report) -> RuleResult {
    if report.unsafe_blocks.is_empty() {
        return RuleResult {
            id: "GATE-002",
            status: RuleStatus::Pass,
            message: "no unsafe blocks were generated".to_string(),
        };
    }
    let missing: Vec<&str> = report
        .unsafe_blocks
        .iter()
        .filter(|block| block.safety_comment.is_none() || block.tests.is_empty())
        .map(|block| block.function.as_str())
        .collect();
    if missing.is_empty() {
        RuleResult {
            id: "GATE-002",
            status: RuleStatus::Pass,
            message: format!(
                "{} unsafe block(s) carry a SAFETY comment and a test",
                report.unsafe_blocks.len()
            ),
        }
    } else {
        RuleResult {
            id: "GATE-002",
            status: RuleStatus::Fail,
            message: format!(
                "unsafe block(s) without a SAFETY comment or a test: {}",
                missing.join(", ")
            ),
        }
    }
}

fn gate_003(report: &Report, policy: Policy) -> RuleResult {
    if report.flagged.is_empty() {
        return RuleResult {
            id: "GATE-003",
            status: RuleStatus::Pass,
            message: "no T4 stubs were generated".to_string(),
        };
    }
    if !policy.allow_t4 {
        return RuleResult {
            id: "GATE-003",
            status: RuleStatus::Fail,
            message: format!(
                "{} flagged item(s) exist and profile {} does not allow T4",
                report.flagged.len(),
                policy.max_tier
            ),
        };
    }
    let missing: Vec<&str> = report
        .flagged
        .iter()
        .filter(|flagged| {
            !flagged
                .evidence
                .iter()
                .any(|evidence| evidence.rung == "V2" && evidence.result == "pass")
        })
        .map(|flagged| flagged.id.as_str())
        .collect();
    if missing.is_empty() {
        RuleResult {
            id: "GATE-003",
            status: RuleStatus::Pass,
            message: format!(
                "{} T4 stub(s) have a passing boundary test",
                report.flagged.len()
            ),
        }
    } else {
        RuleResult {
            id: "GATE-003",
            status: RuleStatus::Fail,
            message: format!(
                "T4 stub(s) without a passing boundary test: {}",
                missing.join(", ")
            ),
        }
    }
}

fn gate_004(report: &Report, policy: Policy) -> RuleResult {
    let rejected: Vec<&str> = report
        .items
        .iter()
        .filter(|item| item.verdict == ItemVerdict::Reject)
        .map(|item| item.id.as_str())
        .collect();
    if rejected.is_empty() {
        return RuleResult {
            id: "GATE-004",
            status: RuleStatus::Pass,
            message: "no item was rejected".to_string(),
        };
    }
    let without_stub: Vec<&str> = rejected
        .iter()
        .copied()
        .filter(|id| !report.flagged.iter().any(|flagged| flagged.id == *id))
        .collect();
    if without_stub.is_empty() && policy.allow_t4 {
        RuleResult {
            id: "GATE-004",
            status: RuleStatus::Note,
            message: format!(
                "{} rejected item(s) were repaired as T4 stubs",
                rejected.len()
            ),
        }
    } else {
        RuleResult {
            id: "GATE-004",
            status: RuleStatus::Fail,
            message: format!(
                "rejected item(s) without a T4 fallback: {}",
                without_stub.join(", ")
            ),
        }
    }
}

fn gate_005(report: &Report, policy: Policy) -> RuleResult {
    let status = match &report.determinism {
        Some(determinism) if determinism.checked && determinism.identical => RuleStatus::Pass,
        Some(determinism) if determinism.checked => RuleStatus::Fail,
        _ => RuleStatus::Fail,
    };
    if status == RuleStatus::Fail && !policy.enforce {
        return RuleResult {
            id: "GATE-005",
            status: RuleStatus::Note,
            message: "determinism was not established; audit reports it without rejecting"
                .to_string(),
        };
    }
    match status {
        RuleStatus::Pass => RuleResult {
            id: "GATE-005",
            status,
            message: "two runs produced identical output".to_string(),
        },
        _ => RuleResult {
            id: "GATE-005",
            status,
            message: "determinism was not established (GATE-005)".to_string(),
        },
    }
}

fn gate_006(report: &Report, policy: Policy) -> RuleResult {
    let unverified: Vec<&str> = report
        .items
        .iter()
        .flat_map(|item| item.assumptions.iter())
        .filter(|assumption| assumption.verified_by.is_none())
        .map(|assumption| assumption.fact.as_str())
        .collect();
    if unverified.is_empty() {
        return RuleResult {
            id: "GATE-006",
            status: RuleStatus::Pass,
            message: "every recorded assumption names the rung that verified it".to_string(),
        };
    }
    if policy.require_verified_assumptions {
        RuleResult {
            id: "GATE-006",
            status: RuleStatus::Fail,
            message: format!(
                "unverified assumption(s) under a strict profile: {}",
                unverified.join(", ")
            ),
        }
    } else {
        RuleResult {
            id: "GATE-006",
            status: RuleStatus::Note,
            message: format!(
                "assumption(s) listed but not verified by a rung: {}",
                unverified.join(", ")
            ),
        }
    }
}

fn gate_007(report: &Report) -> RuleResult {
    let incomplete: Vec<&str> = report
        .ub_sites
        .iter()
        .filter(|site| {
            site.original_behavior.trim().is_empty() || site.translated_behavior.trim().is_empty()
        })
        .map(|site| site.id.as_str())
        .collect();
    if incomplete.is_empty() {
        RuleResult {
            id: "GATE-007",
            status: RuleStatus::Pass,
            message: format!(
                "{} UB site(s) list the original and translated behavior",
                report.ub_sites.len()
            ),
        }
    } else {
        RuleResult {
            id: "GATE-007",
            status: RuleStatus::Fail,
            message: format!("UB site(s) missing a behavior: {}", incomplete.join(", ")),
        }
    }
}

fn gate_009(report: &Report) -> RuleResult {
    if report.run.environment.is_some() && !report.run.toolchains.is_empty() {
        RuleResult {
            id: "GATE-009",
            status: RuleStatus::Pass,
            message: "the environment and tool versions are recorded".to_string(),
        }
    } else {
        RuleResult {
            id: "GATE-009",
            status: RuleStatus::Fail,
            message: "the report does not record the environment and tool versions".to_string(),
        }
    }
}

fn profile_rules(report: &Report, profile: Profile, policy: Policy) -> Vec<RuleResult> {
    let mut rules = Vec::new();
    let tiers: Vec<(&str, u8)> = report
        .items
        .iter()
        .filter_map(|item| tier_number(&item.tier).map(|tier| (item.id.as_str(), tier)))
        .collect();
    let highest = tiers.iter().map(|(_, tier)| *tier).max();

    rules.push(match highest {
        Some(tier) if tier > policy.max_tier => RuleResult {
            id: "PROFILE-001",
            status: RuleStatus::Fail,
            message: format!(
                "item tier T{tier} exceeds the maximum T{} for profile {profile}",
                policy.max_tier
            ),
        },
        Some(tier) => RuleResult {
            id: "PROFILE-001",
            status: RuleStatus::Pass,
            message: format!("highest item tier is T{tier} (limit T{})", policy.max_tier),
        },
        None => RuleResult {
            id: "PROFILE-001",
            status: RuleStatus::Pass,
            message: "no items to tier-check".to_string(),
        },
    });

    let t3 = report.items.iter().filter(|item| item.tier == "T3").count();
    rules.push(if t3 > 0 && !policy.allow_t3 {
        RuleResult {
            id: "PROFILE-002",
            status: RuleStatus::Fail,
            message: format!("{t3} T3 item(s) are not allowed by profile {profile}"),
        }
    } else {
        RuleResult {
            id: "PROFILE-002",
            status: RuleStatus::Pass,
            message: format!("{t3} T3 item(s)"),
        }
    });

    let t4 = report.flagged.len();
    rules.push(if t4 > 0 && !policy.allow_t4 {
        RuleResult {
            id: "PROFILE-003",
            status: RuleStatus::Fail,
            message: format!("{t4} T4 item(s) are not allowed by profile {profile}"),
        }
    } else {
        RuleResult {
            id: "PROFILE-003",
            status: RuleStatus::Pass,
            message: format!("{t4} T4 item(s)"),
        }
    });

    let missing_set: std::collections::BTreeSet<&str> = policy
        .require_rungs
        .iter()
        .copied()
        .filter(|rung| !rung_has_pass(report, rung))
        .chain(
            report
                .skipped_rungs
                .iter()
                .filter(|skipped| policy.require_rungs.contains(&skipped.rung.as_str()))
                .map(|skipped| skipped.rung.as_str()),
        )
        .collect();
    let missing: Vec<&str> = missing_set.into_iter().collect();
    let missing_any = !missing.is_empty();
    rules.push(RuleResult {
        id: "PROFILE-004",
        status: if missing_any {
            if policy.enforce {
                RuleStatus::Fail
            } else {
                RuleStatus::Note
            }
        } else {
            RuleStatus::Pass
        },
        message: if missing_any {
            format!(
                "profile {profile} requires {}; missing {}",
                policy.require_rungs.join(", "),
                missing.join(", ")
            )
        } else if policy.require_rungs.is_empty() {
            format!("profile {profile} requires no rungs")
        } else {
            format!(
                "every rung profile {profile} requires has a passing result ({})",
                policy.require_rungs.join(", ")
            )
        },
    });

    let unexercised: Vec<&str> = report
        .items
        .iter()
        .filter(|item| item.evidence_class == "E0")
        .map(|item| item.id.as_str())
        .collect();
    rules.push(if unexercised.is_empty() {
        RuleResult {
            id: "PROFILE-005",
            status: RuleStatus::Pass,
            message: "every item has differential evidence".to_string(),
        }
    } else {
        RuleResult {
            id: "PROFILE-005",
            status: match policy.unexercised {
                Disposition::Reject if policy.enforce => RuleStatus::Fail,
                _ => RuleStatus::Note,
            },
            message: format!(
                "{} item(s) have no differential evidence: {}",
                unexercised.len(),
                unexercised.join(", ")
            ),
        }
    });

    rules.push(if report.ub_sites.is_empty() {
        RuleResult {
            id: "PROFILE-006",
            status: RuleStatus::Pass,
            message: "no UB sites were recorded".to_string(),
        }
    } else {
        RuleResult {
            id: "PROFILE-006",
            status: RuleStatus::Note,
            message: format!(
                "{} UB site(s) are listed for human review under the {} policy",
                report.ub_sites.len(),
                profile
            ),
        }
    });

    rules
}

fn tier_number(tier: &str) -> Option<u8> {
    tier.strip_prefix('T')
        .and_then(|digits| digits.parse::<u8>().ok())
        .filter(|tier| *tier <= 4)
}

fn rung_has_pass(report: &Report, rung: &str) -> bool {
    report.items.iter().any(|item| {
        item.evidence
            .iter()
            .any(|evidence| evidence.rung == rung && evidence.result == "pass")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;
    use std::collections::BTreeMap;

    fn report() -> Report {
        Report {
            schema: SCHEMA.to_string(),
            run: Run {
                tool_version: "1.3.0".to_string(),
                rule_set_hash: hash_bytes(b"M1"),
                schema_versions: BTreeMap::from([("tir".to_string(), 1)]),
                input: Input::default(),
                toolchains: BTreeMap::from([("rustc".to_string(), "1.85".to_string())]),
                target: Target {
                    triple: "x86_64-unknown-linux-gnu".to_string(),
                    int_bits: 32,
                    pointer_bits: 64,
                },
                profile: "standard".to_string(),
                started_at: "2026-11-03T00:00:00Z".to_string(),
                environment: Some(Environment {
                    os: "linux".to_string(),
                    cpu: "test".to_string(),
                    jobs: 1,
                }),
            },
            verdict: Verdict::Accept,
            determinism: Some(Determinism {
                checked: true,
                output_hash: Some(hash_bytes(b"crate")),
                runs: 2,
                identical: true,
            }),
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
                mutation: None,
            },
            evidence: BTreeMap::from([("E2".to_string(), 1)]),
            items: vec![Item {
                id: "fn:c:@F@sum".to_string(),
                source: Location {
                    file: "sum.c".to_string(),
                    line: 3,
                },
                rust: None,
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
            skipped_rungs: Vec::new(),
            findings: Vec::new(),
        }
    }

    fn audit_report() -> Report {
        let mut report = report();
        // The audit profile requires no rungs; give it the V0 and V2 that exist.
        report.items[0].evidence.insert(
            0,
            Evidence {
                rung: "V0".to_string(),
                result: "pass".to_string(),
                executions: None,
                coverage_pct: None,
                seed: None,
                budget: None,
            },
        );
        report
    }

    #[test]
    fn profiles_parse_and_reject_unknown_names() {
        assert_eq!("strict".parse::<Profile>(), Ok(Profile::Strict));
        assert_eq!("AUDIT".parse::<Profile>(), Ok(Profile::Audit));
        assert!("custom".parse::<Profile>().is_err());
    }

    #[test]
    fn audit_profile_accepts_a_complete_t0_report() {
        let result = evaluate(&audit_report(), Profile::Audit, Some(true));
        assert_eq!(result.verdict, Verdict::Accept);
        assert!(result
            .rules
            .iter()
            .any(|rule| rule.id == "GATE-005" && rule.status == RuleStatus::Pass));
    }

    #[test]
    fn standard_profile_rejects_missing_required_rungs() {
        let result = evaluate(&audit_report(), Profile::Standard, Some(true));
        assert_eq!(result.verdict, Verdict::Reject);
        let rule = result
            .rules
            .iter()
            .find(|rule| rule.id == "PROFILE-004")
            .expect("rung rule");
        assert_eq!(rule.status, RuleStatus::Fail);
        assert!(
            rule.message.contains("V1") && rule.message.contains("V4"),
            "{rule:?}"
        );
    }

    #[test]
    fn forbidding_unsafe_with_an_inventory_fails_gate_001() {
        let mut report = audit_report();
        report.summary.unsafe_counts.generated_code_blocks = 1;
        let result = evaluate(&report, Profile::Audit, Some(true));
        let rule = result
            .rules
            .iter()
            .find(|rule| rule.id == "GATE-001")
            .expect("gate 001");
        assert_eq!(rule.status, RuleStatus::Fail);
    }

    #[test]
    fn a_missing_determinism_check_fails_gate_005() {
        let mut report = audit_report();
        report.determinism = None;
        let result = evaluate(&report, Profile::Standard, Some(true));
        let rule = result
            .rules
            .iter()
            .find(|rule| rule.id == "GATE-005")
            .expect("gate 005");
        assert_eq!(rule.status, RuleStatus::Fail);
    }

    #[test]
    fn strict_rejects_unexercised_functions() {
        let mut report = audit_report();
        report.items[0].evidence_class = "E0".to_string();
        report.items[0].evidence.clear();
        report.summary.coverage = Some(Coverage {
            functions_exercised_pct: 0.0,
            lines_pct: None,
        });
        let result = evaluate(&report, Profile::Strict, Some(true));
        assert_eq!(result.verdict, Verdict::Reject);
        let rule = result
            .rules
            .iter()
            .find(|rule| rule.id == "PROFILE-005")
            .expect("profile 005");
        assert_eq!(rule.status, RuleStatus::Fail);
    }
}

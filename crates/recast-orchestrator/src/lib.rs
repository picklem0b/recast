//! Pipeline driver, scheduler and report assembly.
//!
//! The orchestrator turns the outputs of the frontend, analysis, translation
//! and verification stages into the artifacts the user keeps: the generated
//! crate, `report.json`, `report.md` and the gate decision. It does no analysis
//! or verification of its own, so every number in a report comes from the
//! stage that produced it.

use std::collections::BTreeMap;
use std::path::Path;

use recast_ir::Program;
use recast_report::gate::Profile;
use recast_report::{
    hash_bytes, hash_file, Assumption, Coverage, Determinism, Divergence, Evidence, Finding, Input,
    Item, ItemCounts, Location, Report, Run, RustLocation, SkippedRung, Summary, Target,
    TierCounts, UbSite, UnsafeCounts, SCHEMA,
};
use recast_verify::{Case, Report as VerificationReport, SWEEP_SEED};

/// Everything the report needs from the stages that ran.
pub struct ReportInput<'a> {
    pub program: &'a Program,
    pub case: &'a Case,
    pub verification: &'a VerificationReport,
    /// Generated crate directory (`recast-out/rust`).
    pub crate_dir: &'a Path,
    /// Original C source the case exercises.
    pub source: &'a Path,
    /// The generated Rust source, as written to `crate_dir`.
    pub translated: &'a str,
    /// Relative path of the generated crate file used in item locations.
    pub rust_file: &'a str,
    pub profile: Profile,
    pub tool_version: &'a str,
    pub rule_set: &'a str,
    pub toolchains: BTreeMap<String, String>,
    pub config: Option<&'a Path>,
    pub determinism: Option<Determinism>,
    pub ub_policy: &'a str,
    pub jobs: usize,
    pub started_at: String,
}

/// Build a canonical `recast-report/1` document from a finished run.
pub fn build_report(input: &ReportInput<'_>) -> Report {
    let analysis = recast_analysis::analyze(input.program);
    let calls = count_calls(input.case);
    let sweeps: BTreeMap<&str, &recast_verify::SweepReport> = input
        .verification
        .sweeps
        .iter()
        .map(|sweep| (sweep.function.as_str(), sweep))
        .collect();

    let fuzz: BTreeMap<&str, &recast_verify::fuzz::FuzzReport> = input
        .verification
        .fuzz
        .iter()
        .map(|report| (report.function.as_str(), report))
        .collect();
    let hygiene_ok = input
        .verification
        .hygiene
        .as_ref()
        .is_some_and(|hygiene| hygiene.passed());
    let safety_ok = input
        .verification
        .safety
        .as_ref()
        .is_some_and(|safety| safety.passed());
    let safety_failed = input
        .verification
        .safety
        .as_ref()
        .is_some_and(|safety| !safety.passed());

    let mut items = Vec::new();
    for function in &input.program.functions {
        if function.body.is_none() {
            continue;
        }
        let name = function.name.as_str();
        let executions = calls.get(name).copied().unwrap_or(0);
        let sweep = sweeps.get(name).copied();
        let exercised = executions > 0 || sweep.is_some();
        // A differing call output invalidates the whole case, so every called
        // function carries the failure. A failing sweep belongs to its own
        // function and does not taint the others.
        let calls_failed = !input.verification.compiled
            || input.verification.calls_passed != input.verification.calls_total;
        let differential_failed = executions > 0 && calls_failed;
        let sweep_failed = sweep.is_some_and(|sweep| !sweep.passed);

        let mut evidence = vec![Evidence {
            rung: "V0".to_string(),
            result: if input.verification.compiled {
                "pass"
            } else {
                "fail"
            }
            .to_string(),
            executions: None,
            coverage_pct: None,
            seed: None,
            budget: None,
        }];
        if executions > 0 {
            evidence.push(Evidence {
                rung: "V2".to_string(),
                result: if differential_failed { "fail" } else { "pass" }.to_string(),
                executions: Some(executions as u64),
                coverage_pct: None,
                seed: None,
                budget: None,
            });
        }
        if let Some(sweep) = sweep {
            evidence.push(Evidence {
                rung: "V2".to_string(),
                result: if sweep.passed { "pass" } else { "fail" }.to_string(),
                executions: Some(sweep.iterations),
                coverage_pct: None,
                seed: Some(SWEEP_SEED),
                budget: Some(format!("{} executions", sweep.iterations)),
            });
        }
        if let Some(hygiene) = &input.verification.hygiene {
            evidence.push(Evidence {
                rung: "V1".to_string(),
                result: if hygiene.passed() { "pass" } else { "fail" }.to_string(),
                executions: None,
                coverage_pct: None,
                seed: None,
                budget: None,
            });
        }
        let fuzz_report = fuzz.get(name).copied();
        if let Some(fuzz) = fuzz_report {
            evidence.push(Evidence {
                rung: "V3".to_string(),
                result: if fuzz.passed { "pass" } else { "fail" }.to_string(),
                executions: Some(fuzz.executions),
                coverage_pct: None,
                seed: Some(fuzz.seed),
                budget: Some(format!("{} executions", fuzz.executions)),
            });
        }
        if let Some(safety) = &input.verification.safety {
            evidence.push(Evidence {
                rung: "V4".to_string(),
                result: if safety.passed() { "pass" } else { "fail" }.to_string(),
                executions: None,
                coverage_pct: None,
                seed: None,
                budget: None,
            });
        }
        let v2_ok = exercised && !sweep_failed && (executions == 0 || !calls_failed);
        let v3_ok = v2_ok && fuzz_report.is_some_and(|fuzz| fuzz.passed);
        let v4_ok = v3_ok && safety_ok;
        let item_failed = !input.verification.compiled
            || differential_failed
            || sweep_failed
            || input
                .verification
                .hygiene
                .as_ref()
                .is_some_and(|hygiene| !hygiene.passed())
            || fuzz_report.is_some_and(|fuzz| !fuzz.passed)
            || safety_failed;
        let evidence_class = if v4_ok {
            "E4"
        } else if v3_ok {
            "E3"
        } else if v2_ok {
            "E2"
        } else if hygiene_ok && input.verification.compiled {
            "E1"
        } else {
            "E0"
        };

        let function_findings: Vec<&recast_analysis::Finding> = analysis
            .findings
            .iter()
            .filter(|finding| finding.function == name)
            .collect();
        let ub_findings: Vec<&recast_analysis::Finding> = function_findings
            .iter()
            .copied()
            .filter(|finding| matches!(finding.rule.as_str(), "UB-001" | "UB-007" | "UB-008"))
            .collect();
        let mut notes: Vec<String> = Vec::new();
        if !exercised {
            notes.push("no V2 call or sweep exercised this function".to_string());
        }
        for finding in &function_findings {
            notes.push(format!("{}: {}", finding.rule, finding.message));
        }

        let mut assumptions = Vec::new();
        // A pointer or slice parameter is assumed non-null and valid; V2 passes
        // a live place for every generated call, so the assumption is recorded
        // as verified by the rung that exercised it.
        for param in &function.signature.params {
            if matches!(
                input.program.types.canonical(param.ty),
                Some(recast_ir::Type::Pointer { .. })
            ) {
                assumptions.push(Assumption {
                    fact: "PTR-001".to_string(),
                    text: Some(format!(
                        "{} is non-null and points at live storage",
                        param.name.as_deref().unwrap_or("a pointer parameter")
                    )),
                    verified_by: exercised.then(|| "V2".to_string()),
                });
            }
        }

        items.push(Item {
            id: function.id.to_string(),
            source: Location {
                file: function.provenance.span.file.clone(),
                line: function.provenance.span.line,
            },
            rust: Some(RustLocation {
                path: format!("generated::{}", function.name),
                file: Some(input.rust_file.to_string()),
                line: None,
            }),
            tier: "T0".to_string(),
            rules: Vec::new(),
            verdict: if item_failed {
                recast_report::Verdict::Reject
            } else {
                recast_report::Verdict::Accept
            },
            evidence_class: evidence_class.to_string(),
            evidence,
            assumptions,
            ub_sites: ub_findings
                .iter()
                .map(|finding| format!("{}#{}", function.id, finding.rule))
                .collect(),
            notes,
        });
    }
    items.sort_by(|left, right| left.id.cmp(&right.id));

    let mut findings: Vec<Finding> = analysis
        .findings
        .iter()
        .map(|finding| Finding {
            rule: finding.rule.clone(),
            function: finding.function.clone(),
            file: finding.file.clone(),
            line: finding.line,
            column: finding.column,
            message: finding.message.clone(),
        })
        .collect();
    if let Some(hygiene) = &input.verification.hygiene {
        for note in &hygiene.notes {
            findings.push(Finding {
                rule: "TOOL:V1".to_string(),
                function: "*".to_string(),
                file: input.source.display().to_string(),
                line: 1,
                column: 1,
                message: note.clone(),
            });
        }
    }
    if let Some(safety) = &input.verification.safety {
        for note in &safety.notes {
            findings.push(Finding {
                rule: "TOOL:V4".to_string(),
                function: "*".to_string(),
                file: input.source.display().to_string(),
                line: 1,
                column: 1,
                message: note.clone(),
            });
        }
    }
    findings.sort_by(|left, right| {
        (
            &left.file,
            left.line,
            left.column,
            &left.rule,
            &left.function,
        )
            .cmp(&(
                &right.file,
                right.line,
                right.column,
                &right.rule,
                &right.function,
            ))
    });

    let mut ub_sites = Vec::new();
    for finding in &findings {
        let translated = match finding.rule.as_str() {
            "UB-001" => "panics on signed overflow (overflow-checks are on)",
            "UB-007" => "panics when the shift count is outside the type width",
            "UB-008" => "panics on division or remainder by zero",
            _ => continue,
        };
        ub_sites.push(UbSite {
            id: format!("{}#{}", finding.function, finding.rule),
            location: Location {
                file: finding.file.clone(),
                line: finding.line,
            },
            confidence: "Likely".to_string(),
            original_behavior:
                "undefined by the C standard; the original may wrap, trap or do anything"
                    .to_string(),
            translated_behavior: translated.to_string(),
            policy: input.ub_policy.to_string(),
        });
    }
    ub_sites.sort_by(|left, right| left.id.cmp(&right.id));

    let exercised = items
        .iter()
        .filter(|item| matches!(item.evidence_class.as_str(), "E2" | "E3" | "E4"))
        .count();
    let rejected = items
        .iter()
        .filter(|item| item.verdict == recast_report::Verdict::Reject)
        .count();
    let mut evidence_counts = BTreeMap::new();
    for item in &items {
        *evidence_counts
            .entry(item.evidence_class.clone())
            .or_insert(0usize) += 1;
    }

    let mut skipped_rungs = vec![SkippedRung {
        rung: "V5".to_string(),
        reason: "formal evidence is not enabled by this profile".to_string(),
    }];
    if input.verification.hygiene.is_none() {
        skipped_rungs.push(SkippedRung {
            rung: "V1".to_string(),
            reason: "static hygiene was disabled for this run".to_string(),
        });
    }
    if input.verification.fuzz.is_empty() {
        skipped_rungs.push(SkippedRung {
            rung: "V3".to_string(),
            reason: "no fuzz calls were generated, or fuzzing was disabled".to_string(),
        });
    }
    if input.verification.safety.is_none() {
        skipped_rungs.push(SkippedRung {
            rung: "V4".to_string(),
            reason: "sanitizer and Miri checks were disabled for this run".to_string(),
        });
    }
    skipped_rungs.sort_by(|left, right| left.rung.cmp(&right.rung));

    let differential_failed = !input.verification.compiled
        || input.verification.calls_passed != input.verification.calls_total
        || input.verification.sweeps.iter().any(|sweep| !sweep.passed)
        || input.verification.fuzz.iter().any(|fuzz| !fuzz.passed);
    let divergence = differential_failed.then(|| Divergence {
        id: "D-001".to_string(),
        class: "unknown".to_string(),
        reproducer: format!("{}#{}", input.source.display(), input.verification.case),
        resolution: "the differential output differs; inspect the saved drivers and fuzz corpus under recast-out/verify/"
            .to_string(),
    });

    let run = Run {
        tool_version: input.tool_version.to_string(),
        rule_set_hash: hash_bytes(input.rule_set.as_bytes()),
        schema_versions: BTreeMap::from([
            ("ast".to_string(), 1),
            ("tir".to_string(), 1),
            ("report".to_string(), 1),
        ]),
        input: Input {
            source_hash: hash_file(input.source),
            compile_commands_hash: None,
            config_hash: input.config.and_then(hash_file),
            contracts_hash: None,
        },
        toolchains: input.toolchains.clone(),
        target: Target {
            triple: input.program.target.triple.clone(),
            int_bits: input.program.target.int_bits,
            pointer_bits: input.program.target.pointer_bits,
        },
        profile: input.profile.as_str().to_string(),
        started_at: input.started_at.clone(),
        environment: Some(recast_report::environment(input.jobs)),
    };

    let mut report = Report {
        schema: SCHEMA.to_string(),
        run,
        verdict: recast_report::Verdict::Accept,
        determinism: input.determinism.clone(),
        summary: Summary {
            items: ItemCounts {
                total: items.len(),
                translated: items.len() - rejected,
                flagged_t4: 0,
                rejected,
            },
            by_tier: BTreeMap::from([(
                "T0".to_string(),
                TierCounts {
                    functions: items.len(),
                    loc: None,
                },
            )]),
            unsafe_counts: UnsafeCounts {
                generated_code_blocks: input
                    .verification
                    .hygiene
                    .as_ref()
                    .map_or(0, |hygiene| hygiene.unsafe_tokens),
                ..UnsafeCounts::default()
            },
            coverage: Some(Coverage {
                functions_exercised_pct: if items.is_empty() {
                    0.0
                } else {
                    exercised as f64 * 100.0 / items.len() as f64
                },
                lines_pct: None,
            }),
            mutation: input.verification.mutations.as_ref().map(|mutation| {
                recast_report::MutationSummary {
                    generated: mutation.generated,
                    killed: mutation.killed(),
                    survived: mutation.survived.len(),
                    kill_rate: mutation.kill_rate(),
                    by_rung: mutation.by_rung.clone(),
                    killed_by_compile: mutation.killed_by_compile,
                    killed_by_check: mutation.killed_by_check,
                }
            }),
        },
        evidence: evidence_counts,
        items,
        flagged: Vec::new(),
        ub_sites,
        unsafe_blocks: Vec::new(),
        divergences: divergence.into_iter().collect(),
        skipped_rungs,
        findings,
    };
    let gate = recast_report::gate::evaluate(
        &report,
        input.profile,
        Some(input.translated.contains("#![forbid(unsafe_code)]")),
    );
    report.verdict = gate.verdict;
    report
}

fn count_calls(case: &Case) -> BTreeMap<&str, usize> {
    let mut counts = BTreeMap::new();
    for call in &case.calls {
        *counts.entry(call.function.as_str()).or_insert(0) += 1;
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use recast_ir::{FnSig, Function, ItemId, Linkage, Provenance, Span, Target};
    use recast_verify::{Call, Case, Report as VerificationReport};

    fn program() -> Program {
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        let void = program.types.intern(recast_ir::Type::Void);
        program.functions.push(Function {
            id: ItemId::function("sum"),
            name: "sum".to_string(),
            linkage: Linkage::External,
            signature: FnSig {
                params: Vec::new(),
                ret: void,
                variadic: false,
            },
            body: Some(recast_ir::Block { stmts: Vec::new() }),
            provenance: Provenance::source(Span::new("sum.c", 3, 1)),
        });
        program
    }

    fn report(compiled: bool, passed: bool) -> VerificationReport {
        VerificationReport {
            case: "unit".to_string(),
            compiled,
            calls_passed: usize::from(passed),
            calls_total: 1,
            sweeps: Vec::new(),
            hygiene: None,
            fuzz: Vec::new(),
            safety: None,
            mutations: None,
            failures: Vec::new(),
        }
    }

    fn input<'a>(
        program: &'a Program,
        verification: &'a VerificationReport,
        case: &'a Case,
    ) -> ReportInput<'a> {
        ReportInput {
            program,
            case,
            verification,
            crate_dir: Path::new("recast-out/rust"),
            source: Path::new("sum.c"),
            translated: "#![forbid(unsafe_code)]\npub fn sum() {}\n",
            rust_file: "rust/src/lib.rs",
            profile: Profile::Audit,
            tool_version: "1.3.3",
            rule_set: recast_analysis::RULE_SET,
            toolchains: BTreeMap::from([("rustc".to_string(), "1.85".to_string())]),
            config: None,
            determinism: Some(Determinism {
                checked: true,
                output_hash: Some(hash_bytes(b"crate")),
                runs: 2,
                identical: true,
            }),
            ub_policy: "report",
            jobs: 1,
            started_at: "2026-11-08T00:00:00Z".to_string(),
        }
    }

    fn case() -> Case {
        Case {
            schema: "recast-case/1".to_string(),
            calls: vec![Call {
                function: "sum".to_string(),
                args: Vec::new(),
            }],
            sweeps: Vec::new(),
            test_command: None,
        }
    }

    #[test]
    fn a_passing_run_produces_a_valid_e2_report() {
        let program = program();
        let verification = report(true, true);
        let case = case();
        let built = build_report(&input(&program, &verification, &case));
        built.validate().expect("valid report");
        assert_eq!(built.items.len(), 1);
        assert_eq!(built.items[0].evidence_class, "E2");
        assert_eq!(
            built.summary.coverage.unwrap().functions_exercised_pct,
            100.0
        );
        assert_eq!(built.verdict, recast_report::Verdict::AcceptWithNotes);
    }

    #[test]
    fn a_failed_call_rejects_the_item() {
        let program = program();
        let verification = report(true, false);
        let case = case();
        let built = build_report(&input(&program, &verification, &case));
        assert_eq!(built.items[0].verdict, recast_report::Verdict::Reject);
        assert_eq!(built.divergences.len(), 1);
        assert_eq!(built.summary.items.rejected, 1);
    }

    #[test]
    fn an_unexercised_function_is_e0_and_noted() {
        let program = program();
        let verification = report(true, true);
        let mut case = case();
        case.calls.clear();
        let built = build_report(&input(&program, &verification, &case));
        assert_eq!(built.items[0].evidence_class, "E0");
        assert_eq!(built.summary.coverage.unwrap().functions_exercised_pct, 0.0);
    }
}

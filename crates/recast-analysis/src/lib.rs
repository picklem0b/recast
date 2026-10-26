//! Memory, ownership and undefined-behavior analyses.
//!
//! M1 provides the scalar baseline: a derived control-flow graph, backward
//! liveness, definite assignment (`UB-005`) and constant-folding checks for
//! signed overflow (`UB-001`), shift counts (`UB-007`) and division by zero
//! (`UB-008`). Ownership, points-to and lifetime analyses arrive with M2.

pub mod cfg;
pub mod definite;
pub mod liveness;
pub mod ub;

use recast_ir::{Function, Program, Span};
use serde::Serialize;

/// Report schema name for the findings document.
pub const FINDINGS_SCHEMA: &str = "recast-findings/1";

/// One analysis finding tied to a rule identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub rule: String,
    pub function: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl Finding {
    /// Build a finding from a rule and the best available span.
    pub fn new(
        rule: impl Into<String>,
        function: impl Into<String>,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            rule: rule.into(),
            function: function.into(),
            file: span.file,
            line: span.line,
            column: span.column,
            message: message.into(),
        }
    }
}

/// Everything M1 learned about one function.
#[derive(Debug, Clone)]
pub struct FunctionReport {
    pub name: String,
    pub cfg: cfg::Cfg,
    pub liveness: liveness::Liveness,
    pub findings: Vec<Finding>,
}

/// The result of analysing a whole program.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub functions: Vec<FunctionReport>,
    pub findings: Vec<Finding>,
}

impl Analysis {
    /// Findings with the given rule identifier.
    pub fn findings_with<'a>(&'a self, rule: &'a str) -> impl Iterator<Item = &'a Finding> + 'a {
        self.findings
            .iter()
            .filter(move |finding| finding.rule == rule)
    }

    /// Serialize the findings document written next to the TIR.
    pub fn findings_json(&self) -> Result<String, serde_json::Error> {
        #[derive(Serialize)]
        struct Document<'a> {
            schema: &'a str,
            findings: &'a [Finding],
        }
        let mut text = serde_json::to_string_pretty(&Document {
            schema: FINDINGS_SCHEMA,
            findings: &self.findings,
        })?;
        text.push('\n');
        Ok(text)
    }
}

/// Run every M1 analysis over the program.
pub fn analyze(program: &Program) -> Analysis {
    let mut analysis = Analysis::default();
    for function in &program.functions {
        if function.body.is_none() {
            continue;
        }
        let graph = cfg::build(function);
        let live = liveness::solve(&graph);
        let mut findings = Vec::new();
        definite::check(function, program, &mut findings);
        ub::check(function, program, &mut findings);
        analysis.findings.extend(findings.iter().cloned());
        analysis.functions.push(FunctionReport {
            name: function.name.clone(),
            cfg: graph,
            liveness: live,
            findings,
        });
    }
    analysis
}

/// Analyse one function without building the whole program report.
pub fn analyze_function(function: &Function, program: &Program) -> Vec<Finding> {
    let mut findings = Vec::new();
    definite::check(function, program, &mut findings);
    ub::check(function, program, &mut findings);
    findings
}

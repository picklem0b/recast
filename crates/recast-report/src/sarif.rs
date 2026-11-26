//! SARIF 2.1.0 rendering for report findings.
//!
//! SARIF gives an IDE or a CI annotator the same findings the report holds.
//! Rules are de-duplicated by identifier; every result points at the source
//! location the analysis recorded.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use crate::Report;

/// Render `report` as a SARIF 2.1.0 document.
pub fn render(report: &Report) -> String {
    let mut rules = BTreeMap::new();
    for finding in &report.findings {
        rules.entry(finding.rule.clone()).or_insert_with(|| {
            json!({
                "id": finding.rule,
                "name": finding.rule,
                "shortDescription": { "text": rule_description(&finding.rule) },
                "helpUri": "https://github.com/picklem0b/recast/blob/main/docs/architecture/verification.md",
            })
        });
    }
    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|finding| {
            json!({
                "ruleId": finding.rule,
                "level": level(&finding.rule),
                "message": { "text": finding.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": finding.file },
                        "region": {
                            "startLine": finding.line,
                            "startColumn": finding.column.max(1),
                        }
                    }
                }],
                "properties": {
                    "item": finding.function,
                }
            })
        })
        .collect();
    let document = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "recast",
                    "version": report.run.tool_version,
                    "informationUri": "https://github.com/picklem0b/recast",
                    "rules": rules.into_values().collect::<Vec<_>>(),
                }
            },
            "properties": {
                "verdict": report.verdict.as_str(),
                "profile": report.run.profile,
                "schema": report.schema,
            },
            "results": results,
        }]
    });
    let mut text = serde_json::to_string_pretty(&document)
        .expect("a SARIF document built from a report always serializes");
    text.push('\n');
    text
}

fn level(rule: &str) -> &'static str {
    if rule.starts_with("UB-") || rule.starts_with("GATE-") {
        "error"
    } else {
        "warning"
    }
}

fn rule_description(rule: &str) -> &'static str {
    match rule {
        "UB-001" => "Signed integer overflow",
        "UB-005" => "Use of an uninitialized value",
        "UB-007" => "Shift count outside the type width",
        "UB-008" => "Division or remainder by zero",
        _ => "recast finding",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sarif_has_the_required_version_and_a_rule_per_finding() {
        let mut report = crate::tests::sample_report();
        report.findings.push(crate::Finding {
            rule: "UB-001".to_string(),
            function: "add".to_string(),
            file: "add.c".to_string(),
            line: 7,
            column: 3,
            message: "signed addition may overflow".to_string(),
        });
        let text = render(&report);
        let value: Value = serde_json::from_str(&text).expect("valid JSON");
        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["results"][0]["ruleId"], "UB-001");
        assert_eq!(
            value["runs"][0]["tool"]["driver"]["rules"][0]["id"],
            "UB-001"
        );
    }
}

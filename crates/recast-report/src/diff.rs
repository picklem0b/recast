//! Comparison of two runs.
//!
//! `recast diff` answers three questions: which items appeared or vanished,
//! which changed tier or evidence class, and which findings are new or fixed.
//! It reads canonical reports, so it works across machines and profiles.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::{Item, Report};

/// One item whose tier, evidence class or verdict changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ItemChange {
    pub id: String,
    pub before_tier: String,
    pub after_tier: String,
    pub before_evidence: String,
    pub after_evidence: String,
    pub before_verdict: String,
    pub after_verdict: String,
}

/// The difference between two reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportDiff {
    pub before_verdict: String,
    pub after_verdict: String,
    pub items_added: Vec<String>,
    pub items_removed: Vec<String>,
    pub items_changed: Vec<ItemChange>,
    pub findings_added: Vec<String>,
    pub findings_removed: Vec<String>,
}

impl ReportDiff {
    /// True when nothing changed.
    pub fn is_empty(&self) -> bool {
        self.items_added.is_empty()
            && self.items_removed.is_empty()
            && self.items_changed.is_empty()
            && self.findings_added.is_empty()
            && self.findings_removed.is_empty()
            && self.before_verdict == self.after_verdict
    }

    /// Serialize the diff as `recast-diff/1`.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        #[derive(Serialize)]
        struct Document<'a> {
            schema: &'a str,
            #[serde(flatten)]
            diff: &'a ReportDiff,
        }
        let mut text = serde_json::to_string_pretty(&Document {
            schema: "recast-diff/1",
            diff: self,
        })?;
        text.push('\n');
        Ok(text)
    }

    /// Render the diff for a terminal.
    pub fn to_text(&self) -> String {
        let mut text = format!(
            "verdict: {} -> {}\n",
            self.before_verdict, self.after_verdict
        );
        for id in &self.items_added {
            text.push_str(&format!("+ item {id}\n"));
        }
        for id in &self.items_removed {
            text.push_str(&format!("- item {id}\n"));
        }
        for change in &self.items_changed {
            text.push_str(&format!(
                "~ {}: {} {} {} -> {} {} {}\n",
                change.id,
                change.before_tier,
                change.before_evidence,
                change.before_verdict,
                change.after_tier,
                change.after_evidence,
                change.after_verdict
            ));
        }
        for finding in &self.findings_added {
            text.push_str(&format!("+ finding {finding}\n"));
        }
        for finding in &self.findings_removed {
            text.push_str(&format!("- finding {finding}\n"));
        }
        if self.is_empty() {
            text.push_str("no differences\n");
        }
        text
    }
}

/// Compare two canonical reports.
pub fn compare(before: &Report, after: &Report) -> ReportDiff {
    let before_items: BTreeMap<&str, &Item> = before
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();
    let after_items: BTreeMap<&str, &Item> = after
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect();

    let items_added = after_items
        .keys()
        .filter(|id| !before_items.contains_key(*id))
        .map(|id| (*id).to_string())
        .collect();
    let items_removed = before_items
        .keys()
        .filter(|id| !after_items.contains_key(*id))
        .map(|id| (*id).to_string())
        .collect();
    let mut items_changed = Vec::new();
    for (id, previous) in &before_items {
        let Some(next) = after_items.get(id) else {
            continue;
        };
        if previous.tier != next.tier
            || previous.evidence_class != next.evidence_class
            || previous.verdict != next.verdict
        {
            items_changed.push(ItemChange {
                id: (*id).to_string(),
                before_tier: previous.tier.clone(),
                after_tier: next.tier.clone(),
                before_evidence: previous.evidence_class.clone(),
                after_evidence: next.evidence_class.clone(),
                before_verdict: previous.verdict.as_str().to_string(),
                after_verdict: next.verdict.as_str().to_string(),
            });
        }
    }

    let before_findings = finding_keys(before);
    let after_findings = finding_keys(after);
    let findings_added = after_findings
        .difference(&before_findings)
        .cloned()
        .collect();
    let findings_removed = before_findings
        .difference(&after_findings)
        .cloned()
        .collect();

    ReportDiff {
        before_verdict: before.verdict.as_str().to_string(),
        after_verdict: after.verdict.as_str().to_string(),
        items_added,
        items_removed,
        items_changed,
        findings_added,
        findings_removed,
    }
}

fn finding_keys(report: &Report) -> BTreeSet<String> {
    report
        .findings
        .iter()
        .map(|finding| {
            format!(
                "{} {}:{}:{} {}",
                finding.rule, finding.file, finding.line, finding.column, finding.message
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_finding_and_an_evidence_change_are_reported() {
        let before = crate::tests::sample_report();
        let mut after = before.clone();
        after.items[0].evidence_class = "E4".to_string();
        after.findings.push(crate::Finding {
            rule: "UB-001".to_string(),
            function: "sum".to_string(),
            file: "sum.c".to_string(),
            line: 3,
            column: 1,
            message: "may overflow".to_string(),
        });
        let diff = compare(&before, &after);
        assert_eq!(diff.items_changed.len(), 1);
        assert_eq!(diff.findings_added.len(), 1);
        assert!(diff.findings_removed.is_empty());
        assert!(!diff.is_empty());
    }
}

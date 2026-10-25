#[test]
fn golden_cases_match_the_pipeline() {
    let root = xtask::default_root();
    let outcomes = xtask::check(&root).expect("golden cases run");
    let failures = outcomes
        .iter()
        .filter(|outcome| !outcome.status.ok())
        .map(|outcome| outcome.case.clone())
        .collect::<Vec<_>>();
    assert!(
        failures.is_empty(),
        "golden cases drifted: {failures:?}; run `cargo run -p xtask -- bless` and review the diff"
    );
    assert_eq!(outcomes.len(), 3, "M0 has three golden cases");
}

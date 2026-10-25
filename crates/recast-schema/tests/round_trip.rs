use recast_schema::Ast;

#[test]
fn exporter_documents_round_trip_without_loss() {
    for (name, json) in [
        ("add", include_str!("../../../tests/fixtures/ast/add.json")),
        ("sum", include_str!("../../../tests/fixtures/ast/sum.json")),
        (
            "checksum",
            include_str!("../../../tests/fixtures/ast/checksum.json"),
        ),
    ] {
        let ast = Ast::from_json(json).unwrap_or_else(|error| panic!("{name}: {error}"));
        let encoded = ast.to_json_pretty().expect("serialize");
        let decoded = Ast::from_json(&encoded).expect("parse the encoding");
        assert_eq!(ast, decoded, "{name} changed across a round trip");
    }
}

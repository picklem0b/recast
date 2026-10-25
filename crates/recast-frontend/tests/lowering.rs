use recast_frontend::lower;
use recast_schema::Ast;

fn program(json: &str) -> recast_ir::Program {
    let ast = Ast::from_json(json).expect("fixture is a valid recast-ast/1 document");
    lower(&ast).expect("fixture lowers to TIR")
}

#[test]
fn add_lowers_to_a_valid_program() {
    let program = program(include_str!("../../../tests/fixtures/ast/add.json"));
    program.validate().expect("lowered TIR is valid");

    assert_eq!(program.functions.len(), 1);
    let function = &program.functions[0];
    assert_eq!(function.name, "add");
    assert_eq!(function.id.as_str(), "fn:c:@F@add");
    assert_eq!(program.types.render(function.signature.ret), "i32");
    assert_eq!(program.types.render(function.signature.params[0].ty), "i32");
    assert_eq!(function.signature.params[0].name.as_deref(), Some("a"));
    assert!(function.body.is_some());
}

#[test]
fn sum_keeps_the_pointer_and_the_size_t_alias() {
    let program = program(include_str!("../../../tests/fixtures/ast/sum.json"));
    program.validate().expect("lowered TIR is valid");

    let function = &program.functions[0];
    assert_eq!(
        program.types.render(function.signature.params[0].ty),
        "*const i32"
    );
    let is_const = match program.types.get(function.signature.params[0].ty) {
        Some(recast_ir::Type::Pointer { is_const, .. }) => *is_const,
        other => panic!("expected a pointer parameter, found {other:?}"),
    };
    assert!(is_const, "const int * keeps its const qualifier");
    assert_eq!(
        program.types.render(function.signature.params[1].ty),
        "size_t"
    );

    let dump = program.dump();
    assert!(
        dump.contains("fn sum [fn:c:@F@sum] linkage=external"),
        "{dump}"
    );
    assert!(dump.contains("a: *const i32"), "{dump}");
    assert!(dump.contains("n: size_t"), "{dump}");
    assert!(dump.contains("for"), "{dump}");
}

#[test]
fn conditions_are_explicitly_cast_to_bool() {
    let program = program(include_str!("../../../tests/fixtures/ast/sum.json"));
    let dump = program.dump();
    assert!(
        dump.contains("cast implicit bool"),
        "the for condition carries an explicit bool cast:\n{dump}"
    );
}

#[test]
fn lowering_is_deterministic() {
    let json = include_str!("../../../tests/fixtures/ast/sum.json");
    let first = program(json);
    let second = program(json);
    assert_eq!(first.dump(), second.dump());
    assert_eq!(first.types, second.types);
}

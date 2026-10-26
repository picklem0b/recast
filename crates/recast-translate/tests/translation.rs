use std::sync::atomic::{AtomicU32, Ordering};

use recast_frontend::lower;
use recast_schema::Ast;
use recast_translate::translate;

fn source(json: &str) -> String {
    let ast = Ast::from_json(json).expect("fixture is a valid recast-ast/1 document");
    let program = lower(&ast).expect("fixture lowers to TIR");
    program.validate().expect("lowered TIR is valid");
    translate(&program).expect("fixture translates").source
}

fn compile(source: &str) -> Result<(), String> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir();
    let stem = format!("recast-translate-{}-{id}", std::process::id());
    let path = dir.join(format!("{stem}.rs"));
    let artifact = dir.join(format!("{stem}.rmeta"));
    std::fs::write(&path, source).expect("write generated Rust");
    let output = std::process::Command::new("rustc")
        .args(["--edition=2021", "--crate-type=lib", "--emit=metadata"])
        .arg("-o")
        .arg(&artifact)
        .arg(&path)
        .output()
        .expect("rustc is available");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&artifact);
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

#[test]
fn add_translates_to_a_checked_function() {
    let generated = source(include_str!("../../../tests/fixtures/ast/add.json"));
    assert_eq!(
        generated,
        "\
#![forbid(unsafe_code)]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]

/// recast: translated from tests/fixtures/add.c:1
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
"
    );
    compile(&generated).expect("generated Rust compiles");
}

#[test]
fn sum_collapses_the_pointer_and_length_pair() {
    let generated = source(include_str!("../../../tests/fixtures/ast/sum.json"));
    assert_eq!(
        generated,
        "\
#![forbid(unsafe_code)]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]

/// recast: translated from tests/fixtures/sum.c:3
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];
        i = i.wrapping_add(1);
    }
    s
}
"
    );
    compile(&generated).expect("generated Rust compiles");
}

#[test]
fn unsigned_arithmetic_wraps_explicitly() {
    let generated = source(include_str!("../../../tests/fixtures/ast/checksum.json"));
    assert!(
        generated.contains("a.wrapping_mul(31).wrapping_add(b)"),
        "{generated}"
    );
    compile(&generated).expect("generated Rust compiles");
}

#[test]
fn mutable_parameters_and_c_comparison_values_compile() {
    let generated = source(include_str!("../../../tests/fixtures/ast/integer.json"));
    assert!(
        generated.contains("pub fn count_down(mut n: i32) -> i32"),
        "{generated}"
    );
    assert!(generated.contains("(x < 3) as i32"), "{generated}");
    assert!(generated.contains("(x > 0 && y > 0) as i32"), "{generated}");
    compile(&generated).expect("generated Rust compiles");
}

#[test]
fn translation_is_deterministic() {
    let json = include_str!("../../../tests/fixtures/ast/sum.json");
    assert_eq!(source(json), source(json));
}

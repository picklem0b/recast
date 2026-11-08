use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

fn temp_dir(label: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("recast-cli-{}-{label}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create test directory");
    dir
}

fn recast(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_recast"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run recast")
}

fn install_fixtures(dir: &Path) {
    let ast = dir.join("recast-out/ast");
    std::fs::create_dir_all(&ast).expect("create ast directory");
    for (name, text) in [
        ("add", include_str!("../../../tests/fixtures/ast/add.json")),
        ("sum", include_str!("../../../tests/fixtures/ast/sum.json")),
        (
            "checksum",
            include_str!("../../../tests/fixtures/ast/checksum.json"),
        ),
    ] {
        std::fs::write(ast.join(format!("{name}.tast")), text).expect("install fixture");
    }
}

#[test]
fn version_prints_the_schemas() {
    let dir = temp_dir("version");
    let output = recast(&dir, &["version"]);
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("recast "), "{stdout}");
    assert!(stdout.contains("recast-ast/1"), "{stdout}");
    assert!(stdout.contains("tir/1"), "{stdout}");
}

#[test]
fn init_writes_templates_once() {
    let dir = temp_dir("init");
    let output = recast(&dir, &["init"]);
    assert!(output.status.success(), "{output:?}");
    assert!(dir.join("recast.toml").is_file());
    assert!(dir.join("recast-contracts.toml").is_file());
    let config = std::fs::read_to_string(dir.join("recast.toml")).expect("read config");
    assert!(config.contains("schema = 1"), "{config}");
    assert!(config.contains("language_level = \"L0\""), "{config}");

    let again = recast(&dir, &["init"]);
    assert_eq!(again.status.code(), Some(2), "{again:?}");
    let stderr = String::from_utf8_lossy(&again.stderr);
    assert!(stderr.contains("already exists"), "{stderr}");
}

#[test]
fn analyze_and_translate_produce_a_compiling_crate() {
    let dir = temp_dir("pipeline");
    install_fixtures(&dir);

    let analyzed = recast(&dir, &["analyze"]);
    assert!(analyzed.status.success(), "{analyzed:?}");
    let tir = std::fs::read_to_string(dir.join("recast-out/tir/program.tir")).expect("read TIR");
    assert!(tir.contains("fn sum [fn:c:@F@sum]"), "{tir}");

    let translated = recast(&dir, &["translate"]);
    assert!(translated.status.success(), "{translated:?}");
    let lib = dir.join("recast-out/rust/src/lib.rs");
    let source = std::fs::read_to_string(&lib).expect("read generated crate");
    assert!(source.contains("pub fn sum(a: &[i32]) -> i32"), "{source}");
    assert!(
        source.contains("pub fn add(a: i32, b: i32) -> i32"),
        "{source}"
    );
    let manifest = std::fs::read_to_string(dir.join("recast-out/rust/Cargo.toml"))
        .expect("read generated manifest");
    assert!(
        manifest.contains("overflow-checks = true"),
        "release builds keep C's panic-on-signed-overflow policy:\n{manifest}"
    );

    let artifact = dir.join("generated.rmeta");
    let compiled = Command::new("rustc")
        .args(["--edition=2021", "--crate-type=lib", "--emit=metadata"])
        .arg("-o")
        .arg(&artifact)
        .arg(&lib)
        .output()
        .expect("rustc is available");
    assert!(
        compiled.status.success(),
        "generated crate does not compile:\n{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
}

#[test]
fn verify_writes_a_report_and_gate_evaluates_it() {
    let dir = temp_dir("report");
    install_fixtures(&dir);
    let translated = recast(&dir, &["translate"]);
    assert!(translated.status.success(), "{translated:?}");

    let case = dir.join("case.json");
    std::fs::write(
        &case,
        r#"{"schema":"recast-case/1","calls":[{"function":"add","args":[1,2]}]}"#,
    )
    .expect("write case");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/add.c")
        .canonicalize()
        .expect("fixture source");

    let verified = recast(
        &dir,
        &[
            "verify",
            "--case",
            case.to_str().expect("case path"),
            "--source",
            source.to_str().expect("source path"),
            "--no-mutation",
            "--profile",
            "audit",
        ],
    );
    assert!(verified.status.success(), "{verified:?}");

    let report_path = dir.join("recast-out/report/report.json");
    let report: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&report_path).expect("read report"))
            .expect("parse report");
    assert_eq!(report["schema"], "recast-report/1");
    assert_eq!(report["run"]["profile"], "audit");
    assert_eq!(report["verdict"], "ACCEPT_WITH_NOTES");
    assert_eq!(report["determinism"]["identical"], true);
    let markdown =
        std::fs::read_to_string(dir.join("recast-out/report/report.md")).expect("read report.md");
    assert!(markdown.contains("## Limitations"), "{markdown}");
    assert!(markdown.contains("do not prove equivalence"), "{markdown}");

    let gated = recast(&dir, &["gate", "--profile", "standard"]);
    assert_eq!(gated.status.code(), Some(1), "{gated:?}");
    let gate: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("recast-out/verify/gate.json")).expect("read gate"),
    )
    .expect("parse gate");
    assert_eq!(gate["schema"], "recast-gate/1");
    assert_eq!(gate["verdict"], "REJECT");
    let rules = gate["rules"].as_array().expect("rules");
    assert!(rules
        .iter()
        .any(|rule| { rule["id"] == "PROFILE-004" && rule["status"] == "fail" }));

    let audited = recast(&dir, &["gate", "--profile", "audit"]);
    assert!(audited.status.success(), "{audited:?}");
}

#[test]
fn quiet_suppresses_the_human_summary() {
    let dir = temp_dir("quiet");
    install_fixtures(&dir);
    let output = recast(&dir, &["analyze", "--quiet"]);
    assert!(output.status.success(), "{output:?}");
    assert!(output.stdout.is_empty(), "{:?}", output.stdout);
}

#[test]
fn analyze_without_ast_is_an_input_error() {
    let dir = temp_dir("empty");
    let output = recast(&dir, &["analyze"]);
    assert_eq!(output.status.code(), Some(3), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("recast export"), "{stderr}");
}

#[test]
fn doctor_names_the_tools_it_checks() {
    let dir = temp_dir("doctor");
    let output = recast(&dir, &["doctor"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("rustc"), "{stdout}");
    assert!(stdout.contains("cargo"), "{stdout}");
    assert!(stdout.contains("recast-exporter"), "{stdout}");
}

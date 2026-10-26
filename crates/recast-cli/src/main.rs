//! The `recast` command line.
//!
//! M0 exposes the six commands that make the vertical slice usable:
//! `init`, `export`, `analyze`, `translate`, `version` and `doctor`.

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::{Args, Parser, Subcommand, ValueEnum};
use recast_frontend::lower;
use recast_ir::Program;
use recast_schema::{Ast, SCHEMA_NAME, SCHEMA_VERSION};
use recast_translate::translate;

const EXIT_OK: u8 = 0;
const EXIT_REJECT: u8 = 1;
const EXIT_USAGE: u8 = 2;
const EXIT_INPUT: u8 = 3;
const EXIT_TOOLCHAIN: u8 = 4;
const EXIT_INTERNAL: u8 = 70;

#[derive(Debug, Parser)]
#[command(name = "recast", version, about = "Translate C and C++ to Rust")]
struct Cli {
    #[command(flatten)]
    global: Global,

    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Debug, Args)]
struct Global {
    /// Location of recast.toml.
    #[arg(long, global = true, default_value = "recast.toml")]
    config: PathBuf,
    /// Location of recast-contracts.toml.
    #[arg(long, global = true, default_value = "recast-contracts.toml")]
    contracts: PathBuf,
    /// compile_commands.json to export.
    #[arg(long, global = true)]
    compdb: Option<PathBuf>,
    /// Output directory.
    #[arg(long, global = true, default_value = "recast-out")]
    out: PathBuf,
    /// Parallelism; never changes output.
    #[arg(long, global = true)]
    jobs: Option<usize>,
    /// Master seed.
    #[arg(long, global = true)]
    seed: Option<u64>,
    /// Restrict work to a module (repeatable).
    #[arg(long, global = true)]
    module: Vec<String>,
    /// Gate profile override.
    #[arg(long, global = true)]
    profile: Option<String>,
    /// stdout format.
    #[arg(long, global = true, value_enum, default_value = "human")]
    format: Format,
    /// Suppress progress output.
    #[arg(long, global = true)]
    quiet: bool,
    /// Increase verbosity.
    #[arg(long, short, global = true, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Forbid network access; recast never uses it.
    #[arg(long, global = true)]
    offline: bool,
    /// Disable the optional AI layer; it is off in M0.
    #[arg(long, global = true)]
    no_ai: bool,
    /// Path to the recast-exporter binary.
    #[arg(long, global = true)]
    exporter: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Format {
    Human,
    Json,
}

impl Global {
    /// `--quiet` suppresses human summaries, never the requested JSON.
    fn reports(&self) -> bool {
        !self.quiet || matches!(self.format, Format::Json)
    }
}

#[derive(Debug, Subcommand)]
enum CommandLine {
    /// Create recast.toml and recast-contracts.toml templates.
    Init,
    /// Run the exporter and write recast-ast/1 files.
    Export,
    /// Lower recast-ast/1 to TIR and write the TIR artifacts.
    Analyze,
    /// Translate to Rust and write the generated crate.
    Translate,
    /// Run the M1 verification ladder against a corpus case.
    Verify {
        /// Case description in recast-case/1 JSON.
        #[arg(long)]
        case: PathBuf,
        /// Original C source the case describes.
        #[arg(long)]
        source: PathBuf,
        /// Skip operator mutation testing.
        #[arg(long)]
        no_mutation: bool,
        /// Maximum mutants per case.
        #[arg(long, default_value_t = 24)]
        max_mutants: usize,
        /// Minimum mutation kill rate in percent (roadmap M1 requires 95).
        #[arg(long, default_value_t = 95.0)]
        min_kill_rate: f64,
    },
    /// Print version and schema versions.
    Version,
    /// Check the toolchain and print fixes.
    Doctor,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("recast: {}", error.message);
            ExitCode::from(error.code)
        }
    }
}

fn run(cli: Cli) -> Result<u8, CliError> {
    match cli.command {
        CommandLine::Init => init(&cli.global),
        CommandLine::Export => export(&cli.global),
        CommandLine::Analyze => analyze(&cli.global),
        CommandLine::Translate => translate_command(&cli.global),
        CommandLine::Verify {
            case,
            source,
            no_mutation,
            max_mutants,
            min_kill_rate,
        } => verify_command(
            &cli.global,
            &case,
            &source,
            !no_mutation,
            max_mutants,
            min_kill_rate,
        ),
        CommandLine::Version => version(&cli.global),
        CommandLine::Doctor => doctor(&cli.global),
    }
}

#[derive(Debug)]
struct CliError {
    code: u8,
    message: String,
}

impl CliError {
    fn new(code: u8, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    fn usage(message: impl Into<String>) -> Self {
        Self::new(EXIT_USAGE, message)
    }

    fn input(message: impl Into<String>) -> Self {
        Self::new(EXIT_INPUT, message)
    }

    fn toolchain(message: impl Into<String>) -> Self {
        Self::new(EXIT_TOOLCHAIN, message)
    }

    fn internal(message: impl Into<String>) -> Self {
        Self::new(EXIT_INTERNAL, message)
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn init(global: &Global) -> Result<u8, CliError> {
    let config_path = &global.config;
    let contracts_path = &global.contracts;
    if config_path.exists() {
        return Err(CliError::usage(format!(
            "{} already exists; remove it first",
            config_path.display()
        )));
    }
    if contracts_path.exists() {
        return Err(CliError::usage(format!(
            "{} already exists; remove it first",
            contracts_path.display()
        )));
    }

    let project = project_name();
    let compdb = detect_compdb();
    let config = format!(
        "\
schema = 1

[project]
name = \"{project}\"
root = \".\"
mode = \"library\"
language_level = \"L0\"
target = \"native\"

[inputs]
compile_commands = \"{compdb}\"

[translation]
max_tier = 0
int_overflow = \"panic\"
ub_policy = \"report\"

[output]
dir = \"{out}\"
rust_edition = \"2021\"
",
        out = global.out.display()
    );
    let contracts = "\
schema = 1

# Contracts record facts the analysis cannot derive.
# Add [[function]] entries as described in docs/guides/cli-and-configuration.md.
";
    std::fs::write(config_path, config).map_err(|error| {
        CliError::input(format!("cannot write {}: {error}", config_path.display()))
    })?;
    std::fs::write(contracts_path, contracts).map_err(|error| {
        CliError::input(format!(
            "cannot write {}: {error}",
            contracts_path.display()
        ))
    })?;

    if global.reports() {
        match global.format {
            Format::Human => {
                println!("created {}", config_path.display());
                println!("created {}", contracts_path.display());
                println!("compile database: {compdb}");
            }
            Format::Json => {
                println!(
                    "{{\"created\":[\"{}\",\"{}\"],\"compile_commands\":\"{}\"}}",
                    config_path.display(),
                    contracts_path.display(),
                    compdb
                );
            }
        }
    }
    Ok(EXIT_OK)
}

fn project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .map(|name| {
            let mut name = name
                .chars()
                .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
                .collect::<String>();
            if name.is_empty() || name.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
                name.insert_str(0, "project_");
            }
            name
        })
        .unwrap_or_else(|| "project".to_string())
}

fn detect_compdb() -> String {
    for candidate in [
        "compile_commands.json",
        "build/compile_commands.json",
        "cmake-build-debug/compile_commands.json",
        "cmake-build-release/compile_commands.json",
    ] {
        if Path::new(candidate).is_file() {
            return candidate.to_string();
        }
    }
    "build/compile_commands.json".to_string()
}

fn export(global: &Global) -> Result<u8, CliError> {
    let compdb = global
        .compdb
        .clone()
        .or_else(|| config_compile_commands(&global.config))
        .unwrap_or_else(|| PathBuf::from("compile_commands.json"));
    if !compdb.is_file() {
        return Err(CliError::input(format!(
            "compile database {} does not exist",
            compdb.display()
        )));
    }
    let exporter = find_exporter(global).ok_or_else(|| {
        CliError::toolchain(
            "recast-exporter was not found; build exporter/ or pass --exporter <path>",
        )
    })?;

    let output = Command::new(&exporter)
        .arg("--compdb")
        .arg(&compdb)
        .output()
        .map_err(|error| {
            CliError::toolchain(format!("cannot run {}: {error}", exporter.display()))
        })?;
    if !output.status.success() {
        return Err(CliError::input(format!(
            "recast-exporter failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let json = String::from_utf8(output.stdout)
        .map_err(|error| CliError::input(format!("exporter output is not UTF-8: {error}")))?;
    let ast = Ast::from_json(&json)
        .map_err(|error| CliError::input(format!("exporter output is invalid: {error}")))?;

    let ast_dir = global.out.join("ast");
    std::fs::create_dir_all(&ast_dir).map_err(|error| {
        CliError::input(format!("cannot create {}: {error}", ast_dir.display()))
    })?;
    let units = ast.translation_units.len();
    for unit in ast.translation_units {
        let name = stable_name(&unit.file);
        let document = Ast {
            schema: SCHEMA_NAME.to_string(),
            target: ast.target.clone(),
            translation_units: vec![unit],
        };
        let path = ast_dir.join(format!("{name}.tast"));
        let text = document
            .to_json_pretty()
            .map_err(|error| CliError::internal(error.to_string()))?;
        std::fs::write(&path, text).map_err(|error| {
            CliError::input(format!("cannot write {}: {error}", path.display()))
        })?;
    }
    if global.reports() {
        match global.format {
            Format::Human => {
                println!(
                    "exported {units} translation unit(s) to {}",
                    ast_dir.display()
                )
            }
            Format::Json => println!(
                "{{\"units\":{units},\"directory\":\"{}\"}}",
                ast_dir.display()
            ),
        }
    }
    Ok(EXIT_OK)
}

fn find_exporter(global: &Global) -> Option<PathBuf> {
    if let Some(path) = &global.exporter {
        return path.is_file().then_some(path.clone());
    }
    if let Some(path) = std::env::var_os("RECAST_EXPORTER") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Some(path) = sibling_exporter() {
        return Some(path);
    }
    which("recast-exporter")
}

fn sibling_exporter() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.join("recast-exporter"), dir.join("../recast-exporter")]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

fn analyze(global: &Global) -> Result<u8, CliError> {
    let (program, files) = load_program(global)?;
    write_tir(global, &program)?;
    let analysis = recast_analysis::analyze(&program);
    let findings = analysis.findings.len();
    write_findings(global, &analysis)?;
    let functions = program
        .functions
        .iter()
        .filter(|f| f.body.is_some())
        .count();
    if global.reports() {
        match global.format {
            Format::Human => {
                println!(
                    "analyzed {functions} function(s) from {files} translation unit(s); {findings} finding(s); wrote {}",
                    global.out.join("tir/program.tir").display()
                );
                for finding in &analysis.findings {
                    println!(
                        "  {} {}:{}:{} {}",
                        finding.rule, finding.file, finding.line, finding.column, finding.message
                    );
                }
            }
            Format::Json => println!(
                "{{\"functions\":{functions},\"translation_units\":{files},\"findings\":{findings},\"tir\":\"{}\"}}",
                global.out.join("tir/program.tir").display()
            ),
        }
    }
    Ok(EXIT_OK)
}

fn write_findings(global: &Global, analysis: &recast_analysis::Analysis) -> Result<(), CliError> {
    let text = analysis
        .findings_json()
        .map_err(|error| CliError::internal(error.to_string()))?;
    let path = global.out.join("findings.json");
    std::fs::write(&path, text)
        .map_err(|error| CliError::input(format!("cannot write {}: {error}", path.display())))?;
    Ok(())
}

fn translate_command(global: &Global) -> Result<u8, CliError> {
    let (program, files) = load_program(global)?;
    write_tir(global, &program)?;
    let translation = translate(&program)
        .map_err(|error| CliError::input(format!("translation failed: {error}")))?;

    let crate_dir = global.out.join("rust");
    let src_dir = crate_dir.join("src");
    std::fs::create_dir_all(&src_dir).map_err(|error| {
        CliError::input(format!("cannot create {}: {error}", src_dir.display()))
    })?;
    let package =
        read_project_name(&global.config).unwrap_or_else(|| "recast-generated".to_string());
    let manifest = format!(
        "\
[package]
name = \"{package}\"
version = \"0.1.0\"
edition = \"2021\"

[lib]
path = \"src/lib.rs\"

[profile.release]
overflow-checks = true
"
    );
    std::fs::write(crate_dir.join("Cargo.toml"), manifest).map_err(|error| {
        CliError::input(format!(
            "cannot write {}: {error}",
            crate_dir.join("Cargo.toml").display()
        ))
    })?;
    let lib = src_dir.join("lib.rs");
    std::fs::write(&lib, &translation.source)
        .map_err(|error| CliError::input(format!("cannot write {}: {error}", lib.display())))?;

    let functions = program
        .functions
        .iter()
        .filter(|f| f.body.is_some())
        .count();
    if global.reports() {
        match global.format {
            Format::Human => println!(
                "translated {functions} function(s) from {files} translation unit(s) to {}",
                crate_dir.display()
            ),
            Format::Json => println!(
                "{{\"functions\":{functions},\"translation_units\":{files},\"crate\":\"{}\"}}",
                crate_dir.display()
            ),
        }
    }
    Ok(EXIT_OK)
}

fn verify_command(
    global: &Global,
    case_path: &Path,
    source: &Path,
    mutation: bool,
    max_mutants: usize,
    min_kill_rate: f64,
) -> Result<u8, CliError> {
    let (program, _) = load_program(global)?;
    let case =
        recast_verify::Case::load(case_path).map_err(|error| CliError::input(error.to_string()))?;
    let crate_dir = global.out.join("rust");
    if !crate_dir.join("src/lib.rs").is_file() {
        return Err(CliError::input(format!(
            "{} has no generated crate; run recast translate first",
            crate_dir.display()
        )));
    }
    let mut options = recast_verify::Options::new(global.out.join("verify"));
    options.mutation = mutation;
    options.max_mutants = max_mutants;
    if let Some(cc) = std::env::var_os("RECAST_CC") {
        options.cc = PathBuf::from(cc);
    }
    if let Some(rustc) = std::env::var_os("RECAST_RUSTC") {
        options.rustc = PathBuf::from(rustc);
    }
    let report = recast_verify::verify(&program, &crate_dir, source, &case, &options)
        .map_err(|error| CliError::input(error.to_string()))?;

    if global.reports() {
        match global.format {
            Format::Human => {
                println!(
                    "verify {}: V0 {}, V2 {}/{}, {} sweep(s)",
                    report.case,
                    if report.compiled { "pass" } else { "fail" },
                    report.calls_passed,
                    report.calls_total,
                    report.sweeps.len()
                );
                for sweep in &report.sweeps {
                    println!(
                        "  sweep {} ({} iterations): {}",
                        sweep.function,
                        sweep.iterations,
                        if sweep.passed { "pass" } else { "FAIL" }
                    );
                }
                if let Some(mutations) = &report.mutations {
                    println!(
                        "  mutation: {}/{} killed ({:.1}%), {} compile failures",
                        mutations.killed(),
                        mutations.generated,
                        mutations.kill_rate(),
                        mutations.killed_by_compile
                    );
                }
            }
            Format::Json => {
                let mutations = report
                    .mutations
                    .as_ref()
                    .map(|m| {
                        format!(
                            "{{\"generated\":{},\"killed\":{},\"survived\":{},\"kill_rate\":{:.2}}}",
                            m.generated,
                            m.killed(),
                            m.survived.len(),
                            m.kill_rate()
                        )
                    })
                    .unwrap_or_else(|| "null".to_string());
                println!(
                    "{{\"case\":\"{}\",\"compiled\":{},\"calls\":[{},{}],\"sweeps\":{},\"mutation\":{mutations},\"passed\":{}}}",
                    report.case,
                    report.compiled,
                    report.calls_passed,
                    report.calls_total,
                    report.sweeps.len(),
                    report.passed()
                );
            }
        }
    }
    for failure in &report.failures {
        eprintln!("recast: {failure}");
    }
    if !report.passed() {
        return Ok(EXIT_REJECT);
    }
    if let Some(mutations) = &report.mutations {
        if mutations.kill_rate() < min_kill_rate {
            eprintln!(
                "recast: mutation kill rate {:.1}% is below the {:.1}% threshold; {} mutant(s) survived",
                mutations.kill_rate(),
                min_kill_rate,
                mutations.survived.len()
            );
            return Ok(EXIT_REJECT);
        }
    }
    Ok(EXIT_OK)
}

fn load_program(global: &Global) -> Result<(Program, usize), CliError> {
    let ast_dir = global.out.join("ast");
    let mut paths = Vec::new();
    let entries = std::fs::read_dir(&ast_dir).map_err(|error| {
        CliError::input(format!(
            "cannot read {}: {error}; run recast export first",
            ast_dir.display()
        ))
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| CliError::input(error.to_string()))?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("tast") {
            paths.push(path);
        }
    }
    paths.sort();
    if paths.is_empty() {
        return Err(CliError::input(format!(
            "no recast-ast/1 files in {}; run recast export first",
            ast_dir.display()
        )));
    }

    let mut target = None;
    let mut units = Vec::new();
    let mut names = BTreeSet::new();
    for path in &paths {
        let text = std::fs::read_to_string(path)
            .map_err(|error| CliError::input(format!("cannot read {}: {error}", path.display())))?;
        let ast = Ast::from_json(&text)
            .map_err(|error| CliError::input(format!("{}: {error}", path.display())))?;
        match &target {
            Some(existing) if existing != &ast.target => {
                return Err(CliError::input(format!(
                    "{} targets a different platform than the first export",
                    path.display()
                )))
            }
            Some(_) => {}
            None => target = Some(ast.target.clone()),
        }
        for unit in ast.translation_units {
            for function in &unit.functions {
                if !names.insert(function.name.clone()) {
                    return Err(CliError::input(format!(
                        "function {} is exported more than once",
                        function.name
                    )));
                }
            }
            units.push(unit);
        }
    }

    let merged = Ast {
        schema: SCHEMA_NAME.to_string(),
        target: target.expect("at least one export"),
        translation_units: units,
    };
    merged
        .validate()
        .map_err(|error| CliError::input(format!("merged export is invalid: {error}")))?;
    let units = merged.translation_units.len();
    let program =
        lower(&merged).map_err(|error| CliError::input(format!("lowering failed: {error}")))?;
    program.validate().map_err(|errors| {
        let messages = errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        CliError::input(format!("TIR validation failed: {messages}"))
    })?;
    Ok((program, units))
}

fn write_tir(global: &Global, program: &Program) -> Result<(), CliError> {
    let tir_dir = global.out.join("tir");
    std::fs::create_dir_all(&tir_dir).map_err(|error| {
        CliError::input(format!("cannot create {}: {error}", tir_dir.display()))
    })?;
    let path = tir_dir.join("program.tir");
    std::fs::write(&path, program.dump())
        .map_err(|error| CliError::input(format!("cannot write {}: {error}", path.display())))?;
    Ok(())
}

fn version(global: &Global) -> Result<u8, CliError> {
    let name = env!("CARGO_PKG_NAME");
    let version = env!("CARGO_PKG_VERSION");
    match global.format {
        Format::Human => {
            println!("recast {version}");
            println!("{SCHEMA_NAME} schema version {SCHEMA_VERSION}");
            println!("{} schema version {}", recast_ir::SCHEMA_NAME, recast_ir::SCHEMA_VERSION);
            println!("rule set: M1 (UB-001, UB-005, UB-007, UB-008)");
        }
        Format::Json => println!(
            "{{\"name\":\"{name}\",\"version\":\"{version}\",\"schemas\":{{\"ast\":\"{SCHEMA_NAME}/{SCHEMA_VERSION}\",\"tir\":\"{}/{}\"}},\"rules\":\"empty\"}}",
            recast_ir::SCHEMA_NAME,
            recast_ir::SCHEMA_VERSION
        ),
    }
    Ok(EXIT_OK)
}

fn doctor(global: &Global) -> Result<u8, CliError> {
    let mut rows = Vec::new();
    let clang = which_any(&["clang", "clang-21", "clang-20", "clang-19", "clang-18"]);
    let rustc = which("rustc");
    let cargo = which("cargo");
    let exporter = find_exporter(global);
    let z3 = which("z3");

    rows.push(("clang", clang.clone(), "install Clang 18 or newer"));
    rows.push(("rustc", rustc.clone(), "install Rust stable 1.85 or newer"));
    rows.push(("cargo", cargo.clone(), "install Rust stable 1.85 or newer"));
    rows.push((
        "recast-exporter",
        exporter.clone(),
        "build exporter/ and pass --exporter <path>",
    ));
    rows.push((
        "z3",
        z3.clone(),
        "optional at M0; required from M2 for ownership constraints",
    ));

    let required_missing = clang.is_none() || rustc.is_none() || cargo.is_none();
    match global.format {
        Format::Human => {
            for (name, path, fix) in &rows {
                match path {
                    Some(path) => println!("ok      {name} ({})", path.display()),
                    None => println!("missing {name}: {fix}"),
                }
            }
        }
        Format::Json => {
            let items = rows
                .iter()
                .map(|(name, path, fix)| {
                    format!(
                        "{{\"name\":\"{name}\",\"found\":{},\"path\":{},\"fix\":\"{fix}\"}}",
                        path.is_some(),
                        path.as_ref()
                            .map(|path| format!("\"{}\"", path.display()))
                            .unwrap_or_else(|| "null".to_string())
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            println!("{{\"tools\":[{items}]}}");
        }
    }
    if required_missing {
        Ok(EXIT_TOOLCHAIN)
    } else {
        Ok(EXIT_OK)
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn which_any(names: &[&str]) -> Option<PathBuf> {
    names.iter().find_map(|name| which(name))
}

fn config_compile_commands(config: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(config).ok()?;
    let mut in_inputs = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_inputs = line == "[inputs]";
            continue;
        }
        if in_inputs {
            if let Some(value) = line.strip_prefix("compile_commands") {
                let value = value.trim().trim_start_matches('=').trim();
                if let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                    return Some(PathBuf::from(value));
                }
            }
        }
    }
    None
}

fn read_project_name(config: &Path) -> Option<String> {
    let text = std::fs::read_to_string(config).ok()?;
    let mut in_project = false;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') {
            in_project = line == "[project]";
            continue;
        }
        if in_project {
            if let Some(value) = line.strip_prefix("name") {
                let value = value.trim().trim_start_matches('=').trim();
                if let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                    if !value.is_empty() {
                        return Some(value.to_string());
                    }
                }
            }
        }
    }
    None
}

fn stable_name(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

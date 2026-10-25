//! Versioned AST schema shared with the exporter.
//!
//! The exporter writes `recast-ast/1` as JSON. This crate owns the Rust view of
//! that data and the checks that keep a malformed export out of the pipeline.

use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The schema name written by the exporter and accepted by the frontend.
pub const SCHEMA_NAME: &str = "recast-ast/1";
/// The integer schema version. A breaking change bumps this value.
pub const SCHEMA_VERSION: u32 = 1;

/// A complete export: one target and one or more translation units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ast {
    pub schema: String,
    pub target: Target,
    pub translation_units: Vec<TranslationUnit>,
}

/// Target facts that decide integer widths and pointer layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub triple: String,
    pub pointer_bits: u8,
    pub int_bits: u8,
    pub long_bits: u8,
    pub long_long_bits: u8,
    pub char_signed: bool,
    pub endian: String,
}

/// One preprocessed source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranslationUnit {
    pub file: String,
    pub functions: Vec<Function>,
}

/// A function definition or declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    pub linkage: String,
    pub return_type: Ty,
    pub params: Vec<Param>,
    pub body: Option<Stmt>,
    pub span: Span,
}

/// A function parameter. The name is absent for unnamed declarations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Param {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub ty: Ty,
}

/// A source location in the original file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// A type as reported by the exporter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ty {
    Void,
    Bool,
    Int {
        bits: u8,
        signed: bool,
        spelling: String,
    },
    Float {
        bits: u8,
        spelling: String,
    },
    Pointer {
        pointee: Box<Ty>,
        pointee_const: bool,
    },
    Named {
        name: String,
        canonical: Box<Ty>,
    },
    Unknown {
        spelling: String,
    },
}

/// A statement in a function body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Stmt {
    Compound {
        stmts: Vec<Stmt>,
    },
    Decl {
        name: String,
        #[serde(rename = "type")]
        ty: Ty,
        init: Option<Expr>,
        span: Span,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
        span: Span,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
        span: Span,
    },
    If {
        cond: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
        span: Span,
    },
    Return {
        value: Option<Expr>,
        span: Span,
    },
    Expr {
        expr: Expr,
        span: Span,
    },
    Empty,
    Unsupported {
        spelling: String,
        span: Span,
    },
}

/// A typed expression. The `kind` object is flattened into the expression.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Expr {
    #[serde(rename = "type")]
    pub ty: Ty,
    #[serde(flatten)]
    pub kind: ExprKind,
}

/// The expression form.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExprKind {
    IntLit {
        value: i64,
    },
    FloatLit {
        value: f64,
    },
    DeclRef {
        name: String,
    },
    Binary {
        op: String,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Assign {
        op: String,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Unary {
        op: String,
        operand: Box<Expr>,
        postfix: bool,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Call {
        callee: String,
        args: Vec<Expr>,
    },
    Cast {
        to: Ty,
        expr: Box<Expr>,
        implicit: bool,
    },
    Paren {
        expr: Box<Expr>,
    },
    Unsupported {
        spelling: String,
    },
}

/// A schema or validation failure with the path that caused it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchemaError {
    pub path: String,
    pub message: String,
}

impl SchemaError {
    pub fn new(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.path, self.message)
    }
}

impl std::error::Error for SchemaError {}

impl Ast {
    /// Parse and validate an exported JSON document.
    pub fn from_json(input: &str) -> Result<Self, SchemaError> {
        let ast: Self = serde_json::from_str(input)
            .map_err(|error| SchemaError::new("$", error.to_string()))?;
        ast.validate()?;
        Ok(ast)
    }

    /// Serialize the document with stable field order.
    pub fn to_json_pretty(&self) -> Result<String, SchemaError> {
        serde_json::to_string_pretty(self).map_err(|error| SchemaError::new("$", error.to_string()))
    }

    /// Check the schema version and every node the M0 pipeline can consume.
    pub fn validate(&self) -> Result<(), SchemaError> {
        if self.schema != SCHEMA_NAME {
            return Err(SchemaError::new(
                "schema",
                format!("expected {SCHEMA_NAME}, found {}", self.schema),
            ));
        }
        validate_target(&self.target)?;

        let mut names = BTreeSet::new();
        for (unit_index, unit) in self.translation_units.iter().enumerate() {
            let unit_path = format!("translation_units[{unit_index}]");
            if unit.file.is_empty() {
                return Err(SchemaError::new(
                    format!("{unit_path}.file"),
                    "must name the source file",
                ));
            }
            for (function_index, function) in unit.functions.iter().enumerate() {
                let path = format!("{unit_path}.functions[{function_index}]");
                validate_function(function, &path)?;
                if !names.insert(function.name.clone()) {
                    return Err(SchemaError::new(
                        format!("{path}.name"),
                        format!("duplicate function name {}", function.name),
                    ));
                }
            }
        }

        Ok(())
    }
}

fn validate_target(target: &Target) -> Result<(), SchemaError> {
    if target.triple.is_empty() {
        return Err(SchemaError::new("target.triple", "must not be empty"));
    }
    if !matches!(target.pointer_bits, 32 | 64) {
        return Err(SchemaError::new("target.pointer_bits", "must be 32 or 64"));
    }
    if !matches!(target.int_bits, 16 | 32 | 64) {
        return Err(SchemaError::new("target.int_bits", "must be 16, 32 or 64"));
    }
    if !matches!(target.long_bits, 32 | 64) {
        return Err(SchemaError::new("target.long_bits", "must be 32 or 64"));
    }
    if !matches!(target.long_long_bits, 64) {
        return Err(SchemaError::new("target.long_long_bits", "must be 64"));
    }
    if target.endian != "little" && target.endian != "big" {
        return Err(SchemaError::new("target.endian", "must be little or big"));
    }
    Ok(())
}

fn validate_function(function: &Function, path: &str) -> Result<(), SchemaError> {
    if function.name.is_empty() {
        return Err(SchemaError::new(
            format!("{path}.name"),
            "must not be empty",
        ));
    }
    validate_span(&function.span, &format!("{path}.span"))?;
    validate_ty(&function.return_type, &format!("{path}.return_type"))?;

    for (index, param) in function.params.iter().enumerate() {
        validate_ty(&param.ty, &format!("{path}.params[{index}].type"))?;
    }

    if let Some(body) = &function.body {
        validate_stmt(body, &format!("{path}.body"))?;
    }

    Ok(())
}

fn validate_stmt(stmt: &Stmt, path: &str) -> Result<(), SchemaError> {
    match stmt {
        Stmt::Compound { stmts } => {
            for (index, child) in stmts.iter().enumerate() {
                validate_stmt(child, &format!("{path}.stmts[{index}]"))?;
            }
        }
        Stmt::Decl {
            name,
            ty,
            init,
            span,
        } => {
            if name.is_empty() {
                return Err(SchemaError::new(
                    format!("{path}.name"),
                    "must not be empty",
                ));
            }
            validate_span(span, &format!("{path}.span"))?;
            validate_ty(ty, &format!("{path}.type"))?;
            if let Some(init) = init {
                validate_expr(init, &format!("{path}.init"))?;
            }
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
            span,
        } => {
            validate_span(span, &format!("{path}.span"))?;
            if let Some(init) = init {
                validate_stmt(init, &format!("{path}.init"))?;
            }
            if let Some(cond) = cond {
                validate_expr(cond, &format!("{path}.cond"))?;
            }
            if let Some(step) = step {
                validate_expr(step, &format!("{path}.step"))?;
            }
            validate_stmt(body, &format!("{path}.body"))?;
        }
        Stmt::While { cond, body, span } => {
            validate_span(span, &format!("{path}.span"))?;
            validate_expr(cond, &format!("{path}.cond"))?;
            validate_stmt(body, &format!("{path}.body"))?;
        }
        Stmt::If {
            cond,
            then_branch,
            else_branch,
            span,
        } => {
            validate_span(span, &format!("{path}.span"))?;
            validate_expr(cond, &format!("{path}.cond"))?;
            validate_stmt(then_branch, &format!("{path}.then_branch"))?;
            if let Some(else_branch) = else_branch {
                validate_stmt(else_branch, &format!("{path}.else_branch"))?;
            }
        }
        Stmt::Return { value, span } => {
            validate_span(span, &format!("{path}.span"))?;
            if let Some(value) = value {
                validate_expr(value, &format!("{path}.value"))?;
            }
        }
        Stmt::Expr { expr, span } => {
            validate_span(span, &format!("{path}.span"))?;
            validate_expr(expr, &format!("{path}.expr"))?;
        }
        Stmt::Empty => {}
        Stmt::Unsupported { spelling, span } => {
            validate_span(span, &format!("{path}.span"))?;
            return Err(SchemaError::new(
                path,
                format!("unsupported construct {spelling}"),
            ));
        }
    }
    Ok(())
}

fn validate_expr(expr: &Expr, path: &str) -> Result<(), SchemaError> {
    validate_ty(&expr.ty, &format!("{path}.type"))?;
    match &expr.kind {
        ExprKind::IntLit { .. } | ExprKind::FloatLit { .. } => {}
        ExprKind::DeclRef { name } => {
            if name.is_empty() {
                return Err(SchemaError::new(
                    format!("{path}.name"),
                    "must not be empty",
                ));
            }
        }
        ExprKind::Binary { lhs, rhs, .. } | ExprKind::Assign { lhs, rhs, .. } => {
            validate_expr(lhs, &format!("{path}.lhs"))?;
            validate_expr(rhs, &format!("{path}.rhs"))?;
        }
        ExprKind::Unary { operand, .. } => {
            validate_expr(operand, &format!("{path}.operand"))?;
        }
        ExprKind::Index { base, index } => {
            validate_expr(base, &format!("{path}.base"))?;
            validate_expr(index, &format!("{path}.index"))?;
        }
        ExprKind::Call { callee, args } => {
            if callee.is_empty() {
                return Err(SchemaError::new(
                    format!("{path}.callee"),
                    "must not be empty",
                ));
            }
            for (index, arg) in args.iter().enumerate() {
                validate_expr(arg, &format!("{path}.args[{index}]"))?;
            }
        }
        ExprKind::Cast { to, expr, .. } => {
            validate_ty(to, &format!("{path}.to"))?;
            validate_expr(expr, &format!("{path}.expr"))?;
        }
        ExprKind::Paren { expr } => {
            validate_expr(expr, &format!("{path}.expr"))?;
        }
        ExprKind::Unsupported { spelling } => {
            return Err(SchemaError::new(
                path,
                format!("unsupported expression {spelling}"),
            ));
        }
    }
    Ok(())
}

fn validate_ty(ty: &Ty, path: &str) -> Result<(), SchemaError> {
    match ty {
        Ty::Void | Ty::Bool => Ok(()),
        Ty::Int { bits, spelling, .. } => {
            if *bits == 0 || spelling.is_empty() {
                return Err(SchemaError::new(
                    path,
                    "integer types need a width and a spelling",
                ));
            }
            Ok(())
        }
        Ty::Float { bits, spelling } => {
            if *bits == 0 || spelling.is_empty() {
                return Err(SchemaError::new(
                    path,
                    "float types need a width and a spelling",
                ));
            }
            Ok(())
        }
        Ty::Pointer { pointee, .. } => validate_ty(pointee, &format!("{path}.pointee")),
        Ty::Named { name, canonical } => {
            if name.is_empty() {
                return Err(SchemaError::new(
                    format!("{path}.name"),
                    "must not be empty",
                ));
            }
            validate_ty(canonical, &format!("{path}.canonical"))
        }
        Ty::Unknown { spelling } => Err(SchemaError::new(
            path,
            format!("unknown type {spelling} cannot enter the M0 pipeline"),
        )),
    }
}

fn validate_span(span: &Span, path: &str) -> Result<(), SchemaError> {
    if span.file.is_empty() {
        return Err(SchemaError::new(
            format!("{path}.file"),
            "must not be empty",
        ));
    }
    if span.line == 0 {
        return Err(SchemaError::new(format!("{path}.line"), "must be positive"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span {
        Span {
            file: "add.c".to_string(),
            line: 1,
            column: 1,
        }
    }

    fn int() -> Ty {
        Ty::Int {
            bits: 32,
            signed: true,
            spelling: "int".to_string(),
        }
    }

    fn sample() -> Ast {
        Ast {
            schema: SCHEMA_NAME.to_string(),
            target: Target {
                triple: "aarch64-unknown-linux-gnu".to_string(),
                pointer_bits: 64,
                int_bits: 32,
                long_bits: 64,
                long_long_bits: 64,
                char_signed: true,
                endian: "little".to_string(),
            },
            translation_units: vec![TranslationUnit {
                file: "add.c".to_string(),
                functions: vec![Function {
                    name: "add".to_string(),
                    linkage: "external".to_string(),
                    return_type: int(),
                    params: vec![
                        Param {
                            name: Some("a".to_string()),
                            ty: int(),
                        },
                        Param {
                            name: Some("b".to_string()),
                            ty: int(),
                        },
                    ],
                    body: Some(Stmt::Compound {
                        stmts: vec![Stmt::Return {
                            value: Some(Expr {
                                ty: int(),
                                kind: ExprKind::Binary {
                                    op: "+".to_string(),
                                    lhs: Box::new(Expr {
                                        ty: int(),
                                        kind: ExprKind::DeclRef {
                                            name: "a".to_string(),
                                        },
                                    }),
                                    rhs: Box::new(Expr {
                                        ty: int(),
                                        kind: ExprKind::DeclRef {
                                            name: "b".to_string(),
                                        },
                                    }),
                                },
                            }),
                            span: span(),
                        }],
                    }),
                    span: span(),
                }],
            }],
        }
    }

    #[test]
    fn round_trips_the_schema() {
        let ast = sample();
        ast.validate().expect("sample is valid");
        let json = ast.to_json_pretty().expect("serializes");
        let decoded = Ast::from_json(&json).expect("parses");
        assert_eq!(ast, decoded);
    }

    #[test]
    fn rejects_unknown_types() {
        let mut ast = sample();
        ast.translation_units[0].functions[0].return_type = Ty::Unknown {
            spelling: "auto".to_string(),
        };
        assert!(ast.validate().is_err());
    }

    #[test]
    fn rejects_duplicate_function_names() {
        let mut ast = sample();
        let function = ast.translation_units[0].functions[0].clone();
        ast.translation_units[0].functions.push(function);
        assert!(ast.validate().is_err());
    }
}

//! A small Rust syntax tree and printer.
//!
//! Translation produces this tree and never concatenates source text. Keeping
//! the tree typed means precedence and statement context are decided once, in
//! the printer, instead of at every call site.

use std::fmt::Write;

/// A Rust expression.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Path(String),
    /// An integer literal. `suffix` names the Rust type when Rust could not
    /// infer the value's C type, as in `31u32` or `18446744073709551615u64`.
    Int {
        value: i64,
        suffix: Option<String>,
    },
    Float(f64),
    Bool(bool),
    /// A UTF-8 string literal, used for panic messages.
    Str(String),
    /// A byte string literal, used for C string literals.
    Bytes(Vec<u8>),
    Unary {
        op: &'static str,
        expr: Box<Expr>,
    },
    Binary {
        op: &'static str,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Field {
        base: Box<Expr>,
        field: String,
    },
    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Method {
        receiver: Box<Expr>,
        method: String,
        args: Vec<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        ty: String,
    },
    Paren(Box<Expr>),
    If {
        cond: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },
    StructLit {
        path: String,
        fields: Vec<(String, Expr)>,
    },
    ArrayLit(Vec<Expr>),
    Repeat {
        value: Box<Expr>,
        len: u64,
    },
}

impl Expr {
    fn precedence(&self) -> u8 {
        match self {
            Expr::Binary { op, .. } => match *op {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" | "<" | "<=" | ">" | ">=" => 3,
                "|" => 4,
                "^" => 5,
                "&" => 6,
                "<<" | ">>" => 7,
                "+" | "-" => 8,
                "*" | "/" | "%" => 9,
                _ => 0,
            },
            Expr::Cast { .. } => 10,
            Expr::Unary { .. } => 11,
            Expr::Call { .. } | Expr::Method { .. } | Expr::Index { .. } | Expr::Field { .. } => 12,
            Expr::If { .. } => 0,
            _ => 13,
        }
    }

    fn render(&self, out: &mut String) {
        match self {
            Expr::Path(path) => out.push_str(path),
            Expr::Int { value, suffix } => match suffix {
                // An unsigned type has no negative literals, so a value stored
                // as a bit pattern prints as the number those bits stand for.
                Some(suffix) if suffix.starts_with('u') && *value < 0 => {
                    let _ = write!(out, "{}{suffix}", unsigned_value(*value, suffix));
                }
                Some(suffix) => {
                    let _ = write!(out, "{value}{suffix}");
                }
                None => {
                    let _ = write!(out, "{value}");
                }
            },
            Expr::Float(value) => {
                let _ = write!(out, "{value:?}");
            }
            Expr::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Expr::Str(value) => {
                out.push('"');
                for character in value.chars() {
                    match character {
                        '\\' => out.push_str("\\\\"),
                        '"' => out.push_str("\\\""),
                        '\n' => out.push_str("\\n"),
                        other => out.push(other),
                    }
                }
                out.push('"');
            }
            Expr::Bytes(value) => {
                out.push_str("b\"");
                for byte in value {
                    match byte {
                        b'\\' => out.push_str("\\\\"),
                        b'"' => out.push_str("\\\""),
                        b'\n' => out.push_str("\\n"),
                        b'\r' => out.push_str("\\r"),
                        b'\t' => out.push_str("\\t"),
                        0x20..=0x7e => out.push(*byte as char),
                        other => {
                            let _ = write!(out, "\\x{other:02x}");
                        }
                    }
                }
                out.push('"');
            }
            Expr::Unary { op, expr } => {
                out.push_str(op);
                if *op == "&mut" {
                    out.push(' ');
                }
                out.push_str(&expr.wrapped(11));
            }
            Expr::Binary { op, lhs, rhs } => {
                let precedence = self.precedence();
                if op.starts_with('<') && matches!(**lhs, Expr::Cast { .. }) {
                    // `x as T < y` reads as generic arguments for `T`, so the
                    // cast needs parentheses even though `as` binds tighter.
                    out.push('(');
                    lhs.render(out);
                    out.push(')');
                } else {
                    out.push_str(&lhs.wrapped(precedence));
                }
                let _ = write!(out, " {op} ");
                out.push_str(&rhs.wrapped(precedence + 1));
            }
            Expr::Index { base, index } => {
                out.push_str(&base.wrapped(12));
                out.push('[');
                out.push_str(&index.wrapped(0));
                out.push(']');
            }
            Expr::Field { base, field } => {
                out.push_str(&base.wrapped(12));
                let _ = write!(out, ".{field}");
            }
            Expr::Call { callee, args } => {
                out.push_str(&callee.wrapped(12));
                out.push('(');
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&arg.wrapped(0));
                }
                out.push(')');
            }
            Expr::Method {
                receiver,
                method,
                args,
            } => {
                out.push_str(&receiver.wrapped(12));
                let _ = write!(out, ".{method}(");
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&arg.wrapped(0));
                }
                out.push(')');
            }
            Expr::Cast { expr, ty } => {
                out.push_str(&expr.wrapped(10));
                let _ = write!(out, " as {ty}");
            }
            Expr::Paren(expr) => {
                out.push('(');
                out.push_str(&expr.wrapped(0));
                out.push(')');
            }
            Expr::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let _ = write!(out, "if {} {{ ", cond.wrapped(0));
                out.push_str(&then_branch.wrapped(0));
                out.push_str(" } else { ");
                out.push_str(&else_branch.wrapped(0));
                out.push_str(" }");
            }
            Expr::StructLit { path, fields } => {
                let _ = write!(out, "{path} {{ ");
                for (index, (name, value)) in fields.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{name}: {}", value.wrapped(0));
                }
                out.push_str(" }");
            }
            Expr::ArrayLit(elems) => {
                out.push('[');
                for (index, elem) in elems.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&elem.wrapped(0));
                }
                out.push(']');
            }
            Expr::Repeat { value, len } => {
                let _ = write!(out, "[{}; {len}]", value.wrapped(0));
            }
        }
    }

    /// An integer literal in Rust's default type, `i32`.
    pub fn int(value: i64) -> Self {
        Self::Int {
            value,
            suffix: None,
        }
    }

    /// An integer literal spelled with its Rust type, e.g. `31u32`.
    pub fn typed_int(value: i64, suffix: impl Into<String>) -> Self {
        Self::Int {
            value,
            suffix: Some(suffix.into()),
        }
    }

    fn wrapped(&self, minimum: u8) -> String {
        let rendered = self.render_to_string();
        if self.precedence() < minimum {
            format!("({rendered})")
        } else {
            rendered
        }
    }

    pub fn render_to_string(&self) -> String {
        let mut out = String::new();
        self.render(&mut out);
        out
    }
}

/// The number an unsigned literal's bits stand for, so a value stored as a
/// signed bit pattern prints at its own width.
fn unsigned_value(value: i64, suffix: &str) -> u64 {
    match suffix {
        "u8" => u64::from(value as u8),
        "u16" => u64::from(value as u16),
        "u32" => u64::from(value as u32),
        _ => value as u64,
    }
}

/// A pattern in a `match` arm.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    Int(i64),
    Default,
}

/// One `match` arm. Several integer patterns may share a body.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub patterns: Vec<Pattern>,
    pub body: Vec<Stmt>,
}

/// A Rust statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        mutable: bool,
        ty: String,
        init: Option<Expr>,
    },
    Assign {
        target: Expr,
        op: &'static str,
        value: Expr,
    },
    Expr(Expr),
    /// Evaluate an expression for its effects and drop the value, which is
    /// what a C `(void)expr;` asks for.
    Discard(Expr),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    /// A loop, optionally labeled so a `break` from inside a nested labeled
    /// block can still leave the loop itself.
    Loop {
        label: Option<String>,
        body: Vec<Stmt>,
    },
    /// A block, optionally labeled so an inner `continue` can leave it.
    Block {
        label: Option<String>,
        body: Vec<Stmt>,
    },
    Match {
        scrutinee: Expr,
        arms: Vec<MatchArm>,
    },
    Return(Option<Expr>),
    /// A function body's final expression, printed without a semicolon.
    Tail(Expr),
    Break,
    BreakLabel(String),
    Continue,
}

/// A generated item other than a function.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Struct {
        docs: Vec<String>,
        name: String,
        fields: Vec<(String, String)>,
    },
    Const {
        name: String,
        ty: String,
        value: Expr,
    },
    /// A compile-time check that a `#[repr(C)]` struct has the C layout.
    LayoutAssert { name: String, size: u64, align: u64 },
}

/// A function item.
#[derive(Debug, Clone, PartialEq)]
pub struct Fn {
    pub docs: Vec<String>,
    pub name: String,
    pub public: bool,
    pub params: Vec<(String, String)>,
    pub ret: String,
    pub body: Vec<Stmt>,
}

/// A whole generated file.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct File {
    pub attrs: Vec<String>,
    pub items: Vec<Item>,
    pub fns: Vec<Fn>,
}

impl File {
    pub fn render(&self) -> String {
        let mut out = String::new();
        for attr in &self.attrs {
            out.push_str(attr);
            out.push('\n');
        }
        for item in &self.items {
            out.push('\n');
            match item {
                Item::Struct { docs, name, fields } => {
                    for doc in docs {
                        let _ = writeln!(out, "/// {doc}");
                    }
                    out.push_str("#[repr(C)]\n#[derive(Clone, Copy)]\n");
                    let _ = writeln!(out, "pub struct {name} {{");
                    for (field, ty) in fields {
                        let _ = writeln!(out, "    pub {field}: {ty},");
                    }
                    out.push_str("}\n");
                }
                Item::Const { name, ty, value } => {
                    let _ = writeln!(
                        out,
                        "pub const {name}: {ty} = {};",
                        value.render_to_string()
                    );
                }
                Item::LayoutAssert { name, size, align } => {
                    out.push_str("const _: () = {\n");
                    let _ = writeln!(
                        out,
                        "    assert!(core::mem::size_of::<{name}>() == {size});"
                    );
                    let _ = writeln!(
                        out,
                        "    assert!(core::mem::align_of::<{name}>() == {align});"
                    );
                    out.push_str("};\n");
                }
            }
        }
        for (index, function) in self.fns.iter().enumerate() {
            if index > 0 || !self.items.is_empty() || !self.attrs.is_empty() {
                out.push('\n');
            }
            function.render(&mut out);
        }
        out
    }
}

impl Fn {
    fn render(&self, out: &mut String) {
        for doc in &self.docs {
            let _ = writeln!(out, "/// {doc}");
        }
        let visibility = if self.public { "pub fn " } else { "fn " };
        let params = self
            .params
            .iter()
            .map(|(name, ty)| format!("{name}: {ty}"))
            .collect::<Vec<_>>()
            .join(", ");
        let one_line = format!("{visibility}{}({params}) -> {} {{", self.name, self.ret);
        if one_line.len() <= 100 {
            out.push_str(&one_line);
            out.push('\n');
        } else {
            let _ = writeln!(out, "{visibility}{}(", self.name);
            for (name, ty) in &self.params {
                let _ = writeln!(out, "    {name}: {ty},");
            }
            let _ = writeln!(out, ") -> {} {{", self.ret);
        }
        if self.body.is_empty() {
            out.push_str("}\n");
            return;
        }
        for stmt in &self.body {
            stmt.render(out, 1);
        }
        out.push_str("}\n");
    }
}

impl Stmt {
    fn render(&self, out: &mut String, depth: usize) {
        let pad = "    ".repeat(depth);
        match self {
            Stmt::Let {
                name,
                mutable,
                ty,
                init,
            } => {
                let binding = if *mutable { "mut " } else { "" };
                match init {
                    Some(init) => {
                        let _ = writeln!(
                            out,
                            "{pad}let {binding}{name}: {ty} = {};",
                            init.render_to_string()
                        );
                    }
                    None => {
                        let _ = writeln!(out, "{pad}let {binding}{name}: {ty};");
                    }
                }
            }
            Stmt::Assign { target, op, value } => {
                let _ = writeln!(
                    out,
                    "{pad}{} {op} {};",
                    target.render_to_string(),
                    value.render_to_string()
                );
            }
            Stmt::Expr(expr) => {
                let _ = writeln!(out, "{pad}{};", expr.render_to_string());
            }
            Stmt::Discard(expr) => {
                let _ = writeln!(out, "{pad}let _ = {};", expr.render_to_string());
            }
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                let _ = writeln!(out, "{pad}if {} {{", cond.render_to_string());
                for stmt in then_body {
                    stmt.render(out, depth + 1);
                }
                match else_body {
                    Some(body) => {
                        let _ = writeln!(out, "{pad}}} else {{");
                        for stmt in body {
                            stmt.render(out, depth + 1);
                        }
                        let _ = writeln!(out, "{pad}}}");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}}}");
                    }
                }
            }
            Stmt::While { cond, body } => {
                let _ = writeln!(out, "{pad}while {} {{", cond.render_to_string());
                for stmt in body {
                    stmt.render(out, depth + 1);
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Loop { label, body } => {
                match label {
                    Some(label) => {
                        let _ = writeln!(out, "{pad}'{label}: loop {{");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}loop {{");
                    }
                }
                for stmt in body {
                    stmt.render(out, depth + 1);
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Block { label, body } => {
                match label {
                    Some(label) => {
                        let _ = writeln!(out, "{pad}'{label}: {{");
                    }
                    None => {
                        let _ = writeln!(out, "{pad}{{");
                    }
                }
                for stmt in body {
                    stmt.render(out, depth + 1);
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Match { scrutinee, arms } => {
                let _ = writeln!(out, "{pad}match {} {{", scrutinee.render_to_string());
                for arm in arms {
                    let patterns = arm
                        .patterns
                        .iter()
                        .map(|pattern| match pattern {
                            Pattern::Int(value) => value.to_string(),
                            Pattern::Default => "_".to_string(),
                        })
                        .collect::<Vec<_>>()
                        .join(" | ");
                    let _ = writeln!(out, "{pad}    {patterns} => {{");
                    for stmt in &arm.body {
                        stmt.render(out, depth + 2);
                    }
                    let _ = writeln!(out, "{pad}    }}");
                }
                let _ = writeln!(out, "{pad}}}");
            }
            Stmt::Return(Some(expr)) => {
                let _ = writeln!(out, "{pad}return {};", expr.render_to_string());
            }
            Stmt::Return(None) => {
                let _ = writeln!(out, "{pad}return;");
            }
            Stmt::Tail(expr) => {
                let _ = writeln!(out, "{pad}{}", expr.render_to_string());
            }
            Stmt::Break => {
                let _ = writeln!(out, "{pad}break;");
            }
            Stmt::BreakLabel(label) => {
                let _ = writeln!(out, "{pad}break '{label};");
            }
            Stmt::Continue => {
                let _ = writeln!(out, "{pad}continue;");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn precedence_keeps_the_tree_meaning() {
        let expr = Expr::Binary {
            op: "*",
            lhs: Box::new(Expr::Binary {
                op: "+",
                lhs: Box::new(Expr::Path("a".into())),
                rhs: Box::new(Expr::Path("b".into())),
            }),
            rhs: Box::new(Expr::int(2)),
        };
        assert_eq!(expr.render_to_string(), "(a + b) * 2");
    }

    #[test]
    fn a_cast_on_the_left_of_a_less_than_is_parenthesized() {
        let cast = || Expr::Cast {
            expr: Box::new(Expr::Path("a".into())),
            ty: "u32".into(),
        };
        for op in ["<", "<=", "<<"] {
            let expr = Expr::Binary {
                op,
                lhs: Box::new(cast()),
                rhs: Box::new(Expr::Path("b".into())),
            };
            assert_eq!(expr.render_to_string(), format!("(a as u32) {op} b"));
        }
        // Operators that cannot be read as generic arguments stay unchanged.
        let shift = Expr::Binary {
            op: ">>",
            lhs: Box::new(cast()),
            rhs: Box::new(Expr::Path("b".into())),
        };
        assert_eq!(shift.render_to_string(), "a as u32 >> b");
    }

    #[test]
    fn method_calls_chain_without_extra_parentheses() {
        let expr = Expr::Method {
            receiver: Box::new(Expr::Method {
                receiver: Box::new(Expr::Path("a".into())),
                method: "wrapping_mul".into(),
                args: vec![Expr::int(31)],
            }),
            method: "wrapping_add".into(),
            args: vec![Expr::Path("b".into())],
        };
        assert_eq!(
            expr.render_to_string(),
            "a.wrapping_mul(31).wrapping_add(b)"
        );
    }

    #[test]
    fn struct_literals_and_fields_render() {
        let value = Expr::StructLit {
            path: "Point".into(),
            fields: vec![("x".into(), Expr::int(1)), ("y".into(), Expr::int(2))],
        };
        assert_eq!(value.render_to_string(), "Point { x: 1, y: 2 }");
        let field = Expr::Field {
            base: Box::new(Expr::Path("p".into())),
            field: "x".into(),
        };
        assert_eq!(field.render_to_string(), "p.x");
    }

    #[test]
    fn byte_strings_escape() {
        let value = Expr::Bytes(b"a\"\n".to_vec());
        assert_eq!(value.render_to_string(), "b\"a\\\"\\n\"");
    }

    #[test]
    fn a_typed_literal_renders_its_suffix() {
        assert_eq!(Expr::typed_int(31, "u32").render_to_string(), "31u32");
        assert_eq!(
            Expr::typed_int(3000000000, "i64").render_to_string(),
            "3000000000i64"
        );
        // An unsigned type has no negative literals: the stored bits print as
        // the value they stand for, at the width of their own suffix.
        assert_eq!(Expr::typed_int(-1, "u8").render_to_string(), "255u8");
        assert_eq!(
            Expr::typed_int(-1, "u32").render_to_string(),
            "4294967295u32"
        );
        assert_eq!(
            Expr::typed_int(-1, "u64").render_to_string(),
            "18446744073709551615u64"
        );
        assert_eq!(Expr::int(7).render_to_string(), "7");
    }
}

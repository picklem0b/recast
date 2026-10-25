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
    Int(i64),
    Float(f64),
    Bool(bool),
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
            Expr::Call { .. } | Expr::Method { .. } | Expr::Index { .. } => 12,
            _ => 13,
        }
    }

    fn render(&self, out: &mut String) {
        match self {
            Expr::Path(path) => out.push_str(path),
            Expr::Int(value) => {
                let _ = write!(out, "{value}");
            }
            Expr::Float(value) => {
                let _ = write!(out, "{value:?}");
            }
            Expr::Bool(value) => out.push_str(if *value { "true" } else { "false" }),
            Expr::Unary { op, expr } => {
                out.push_str(op);
                out.push_str(&expr.wrapped(11));
            }
            Expr::Binary { op, lhs, rhs } => {
                let precedence = self.precedence();
                out.push_str(&lhs.wrapped(precedence));
                let _ = write!(out, " {op} ");
                out.push_str(&rhs.wrapped(precedence + 1));
            }
            Expr::Index { base, index } => {
                out.push_str(&base.wrapped(12));
                out.push('[');
                out.push_str(&index.wrapped(0));
                out.push(']');
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

/// A Rust statement.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Let {
        name: String,
        ty: String,
        init: Option<Expr>,
    },
    Assign {
        target: Expr,
        op: &'static str,
        value: Expr,
    },
    Expr(Expr),
    If {
        cond: Expr,
        then_body: Vec<Stmt>,
        else_body: Option<Vec<Stmt>>,
    },
    While {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Return(Option<Expr>),
    /// A function body's final expression, printed without a semicolon.
    Tail(Expr),
    Break,
    Continue,
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
    pub fns: Vec<Fn>,
}

impl File {
    pub fn render(&self) -> String {
        let mut out = String::new();
        for attr in &self.attrs {
            out.push_str(attr);
            out.push('\n');
        }
        for (index, function) in self.fns.iter().enumerate() {
            if index > 0 || !self.attrs.is_empty() {
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
        out.push_str(if self.public { "pub fn " } else { "fn " });
        let _ = write!(out, "{}(", self.name);
        for (index, (name, ty)) in self.params.iter().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            let _ = write!(out, "{name}: {ty}");
        }
        let _ = write!(out, ") -> {}", self.ret);
        out.push_str(" {\n");
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
            Stmt::Let { name, ty, init } => match init {
                Some(init) => {
                    let _ = writeln!(
                        out,
                        "{pad}let mut {name}: {ty} = {};",
                        init.render_to_string()
                    );
                }
                None => {
                    let _ = writeln!(out, "{pad}let mut {name}: {ty};");
                }
            },
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
            rhs: Box::new(Expr::Int(2)),
        };
        assert_eq!(expr.render_to_string(), "(a + b) * 2");
    }

    #[test]
    fn method_calls_chain_without_extra_parentheses() {
        let expr = Expr::Method {
            receiver: Box::new(Expr::Method {
                receiver: Box::new(Expr::Path("a".into())),
                method: "wrapping_mul".into(),
                args: vec![Expr::Int(31)],
            }),
            method: "wrapping_add".into(),
            args: vec![Expr::Path("b".into())],
        };
        assert_eq!(
            expr.render_to_string(),
            "a.wrapping_mul(31).wrapping_add(b)"
        );
    }
}

//! Structured intermediate representation and validation.
//!
//! TIR preserves source structure so translation can emit readable Rust. Every
//! item and node carries a stable identifier, and every expression carries a
//! type and a value category.

pub mod dump;
pub mod id;
pub mod program;
pub mod types;
pub mod validate;

pub use dump::dump;
pub use id::{ItemId, NodeId, TypeId};
pub use program::{
    Block, Callee, Endian, Expr, ExprKind, FileInfo, FnSig, Function, Linkage, Param, Program,
    Provenance, Span, Stmt, StmtKind, Target, ValueCategory, SCHEMA_NAME, SCHEMA_VERSION,
};
pub use types::{Type, TypeTable};
pub use validate::{validate, ValidationError};

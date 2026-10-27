//! Structured intermediate representation and validation.
//!
//! TIR preserves source structure so translation can emit readable Rust. Every
//! item and node carries a stable identifier, and every expression carries a
//! type and a value category.

pub mod dump;
pub mod id;
pub mod program;
pub mod slice;
pub mod types;
pub mod validate;

pub use dump::dump;
pub use id::{ItemId, NodeId, TypeId};
pub use program::{
    Block, Callee, Case, Endian, EnumDef, Enumerator, Expr, ExprKind, FieldDef, FileInfo, FnSig,
    Function, Global, Linkage, Param, Program, Provenance, RecordDef, Span, Stmt, StmtKind, Target,
    ValueCategory, SCHEMA_NAME, SCHEMA_VERSION,
};
pub use slice::{is_size_t, is_slice_element, slice_group, SliceGroup};
pub use types::{Type, TypeTable};
pub use validate::{validate, ValidationError};

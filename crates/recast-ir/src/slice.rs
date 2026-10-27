//! Recognition of the C slice idiom `(const T *a, ..., size_t n)`.
//!
//! Three stages decide which parameters form one slice: the translator that
//! emits the Rust signature, the verification driver that builds both call
//! sites, and the tier rules. When two of them disagree the generated crate
//! does not compile, so the rule lives here once, next to the TIR it reads.

use crate::id::TypeId;
use crate::program::{Param, Program};
use crate::types::Type;

/// The parameters that share one length: `(const T *a, const T *b, size_t n)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SliceGroup {
    /// Indexes of the pointer parameters, in declaration order.
    pub pointers: Vec<usize>,
    /// Index of the `size_t` parameter that carries the length.
    pub length: usize,
}

impl SliceGroup {
    /// Whether the parameter at `index` is one of the group's pointers.
    pub fn contains_pointer(&self, index: usize) -> bool {
        self.pointers.contains(&index)
    }
}

/// Whether a pointer to this type is a slice element. A pointer to a record or
/// to an array is one object behind the pointer, not a slice.
pub fn is_slice_element(program: &Program, id: TypeId) -> bool {
    matches!(
        program.types.canonical(id),
        Some(Type::Bool)
            | Some(Type::Char { .. })
            | Some(Type::Int { .. })
            | Some(Type::Float { .. })
    )
}

/// The `size_t` parameter, recognized by name or by the target's width.
pub fn is_size_t(program: &Program, id: TypeId) -> bool {
    match program.types.get(id) {
        Some(Type::Named { name, .. }) if name == "size_t" => true,
        Some(Type::Int {
            bits,
            signed: false,
            ..
        }) => u16::from(*bits) == u16::from(program.target.size_t_bits),
        _ => false,
    }
}

/// The single slice group a parameter list declares, if it declares one.
///
/// Every parameter before the `size_t` must be a pointer — a value between the
/// pointers and the length means the length belongs to something else — and at
/// least one of those pointers must have a slice element, so `(struct S *s,
/// size_t n)` stays two parameters rather than becoming a slice of records.
pub fn slice_group(program: &Program, params: &[Param]) -> Option<SliceGroup> {
    let length = params
        .iter()
        .position(|param| is_size_t(program, param.ty))?;
    if length == 0 {
        return None;
    }
    if !params[..length]
        .iter()
        .all(|param| matches!(program.types.get(param.ty), Some(Type::Pointer { .. })))
    {
        return None;
    }
    let pointers = (0..length)
        .filter(|index| {
            matches!(
                program.types.get(params[*index].ty),
                Some(Type::Pointer { pointee, .. }) if is_slice_element(program, *pointee)
            )
        })
        .collect::<Vec<_>>();
    if pointers.is_empty() {
        return None;
    }
    Some(SliceGroup { pointers, length })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::program::Target;
    use crate::types::TypeTable;

    fn param(name: &str, ty: TypeId) -> Param {
        Param {
            name: Some(name.to_string()),
            ty,
        }
    }

    /// `(const int *a, int *b, size_t n)`: both pointers share the length.
    #[test]
    fn pointers_before_a_size_t_are_one_slice() {
        let mut types = TypeTable::new();
        let int = types.intern(Type::int(32, true, "int"));
        let size_t = types.intern(Type::int(64, false, "unsigned long"));
        let const_pointer = types.intern(Type::Pointer {
            pointee: int,
            is_const: true,
        });
        let pointer = types.intern(Type::Pointer {
            pointee: int,
            is_const: false,
        });
        let length = types.intern(Type::size_t(size_t));
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        program.types = types;
        let group = slice_group(
            &program,
            &[
                param("a", const_pointer),
                param("b", pointer),
                param("n", length),
            ],
        )
        .expect("a slice group");
        assert_eq!(group.pointers, vec![0, 1]);
        assert_eq!(group.length, 2);
        assert!(group.contains_pointer(1));
    }

    /// A pointer to a record is one object, so the length belongs to no group.
    #[test]
    fn a_record_pointer_before_a_length_is_not_a_slice() {
        let mut types = TypeTable::new();
        let record = types.intern(Type::Record {
            name: "struct S".to_string(),
        });
        let pointer = types.intern(Type::Pointer {
            pointee: record,
            is_const: false,
        });
        let size_t = types.intern(Type::int(64, false, "unsigned long"));
        let length = types.intern(Type::size_t(size_t));
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        program.types = types;
        assert!(slice_group(&program, &[param("s", pointer), param("n", length)]).is_none());
    }

    /// A value between the pointers and the length breaks the idiom.
    #[test]
    fn a_scalar_before_a_length_is_not_a_slice() {
        let mut types = TypeTable::new();
        let int = types.intern(Type::int(32, true, "int"));
        let pointer = types.intern(Type::Pointer {
            pointee: int,
            is_const: true,
        });
        let size_t = types.intern(Type::int(64, false, "unsigned long"));
        let length = types.intern(Type::size_t(size_t));
        let mut program = Program::new(Target::lp64("x86_64-unknown-linux-gnu"));
        program.types = types;
        assert!(slice_group(
            &program,
            &[param("a", pointer), param("limit", int), param("n", length)]
        )
        .is_none());
    }
}

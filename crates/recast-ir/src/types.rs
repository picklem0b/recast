//! Interned TIR types.
//!
//! Structural equality is intentional: two `int` nodes from different files
//! share one [`TypeId`]. IDs are assigned in first-intern order, which keeps a
//! run deterministic as long as lowering order is deterministic.

use std::collections::HashMap;

use crate::id::TypeId;

/// A type as TIR sees it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Type {
    Void,
    Bool,
    Char {
        signed: bool,
    },
    Int {
        bits: u8,
        signed: bool,
        spelling: String,
    },
    Float {
        bits: u16,
        spelling: String,
    },
    Pointer {
        pointee: TypeId,
        is_const: bool,
    },
    Named {
        name: String,
        canonical: TypeId,
    },
    Unknown {
        spelling: String,
    },
}

impl Type {
    /// A target-independent integer type with its source spelling.
    pub fn int(bits: u8, signed: bool, spelling: impl Into<String>) -> Self {
        Self::Int {
            bits,
            signed,
            spelling: spelling.into(),
        }
    }

    /// The `size_t` alias for the target's unsigned pointer-sized integer.
    pub fn size_t(canonical: TypeId) -> Self {
        Self::Named {
            name: "size_t".to_string(),
            canonical,
        }
    }
}

/// The interned type table of one program.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TypeTable {
    entries: Vec<Type>,
    index: HashMap<Type, TypeId>,
}

impl TypeTable {
    /// An empty table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Intern a type and return its stable ID.
    pub fn intern(&mut self, ty: Type) -> TypeId {
        if let Some(id) = self.index.get(&ty) {
            return *id;
        }
        let id = TypeId::from_index(self.entries.len());
        self.entries.push(ty.clone());
        self.index.insert(ty, id);
        id
    }

    /// Look up a type by ID.
    pub fn get(&self, id: TypeId) -> Option<&Type> {
        self.entries.get(id.index())
    }

    /// Number of distinct types.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether no type has been interned.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate in ID order, which is the order used by the dump format.
    pub fn iter(&self) -> impl Iterator<Item = (TypeId, &Type)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, ty)| (TypeId::from_index(index), ty))
    }

    /// Follow `Named` aliases to the underlying type.
    pub fn canonical(&self, id: TypeId) -> Option<&Type> {
        let mut current = id;
        for _ in 0..=self.entries.len() {
            match self.get(current)? {
                Type::Named { canonical, .. } => current = *canonical,
                other => return Some(other),
            }
        }
        None
    }

    /// Whether `id`, or anything it aliases, is `Unknown`.
    pub fn contains_unknown(&self, id: TypeId) -> bool {
        let mut current = id;
        for _ in 0..=self.entries.len() {
            let Some(ty) = self.get(current) else {
                return true;
            };
            match ty {
                Type::Unknown { .. } => return true,
                Type::Named { canonical, .. } => current = *canonical,
                Type::Pointer { pointee, .. } => {
                    if self.contains_unknown(*pointee) {
                        return true;
                    }
                    return false;
                }
                _ => return false,
            }
        }
        true
    }

    /// Render a type for dumps and diagnostics.
    pub fn render(&self, id: TypeId) -> String {
        match self.get(id) {
            Some(Type::Void) => "()".to_string(),
            Some(Type::Bool) => "bool".to_string(),
            Some(Type::Char { signed }) => {
                if *signed {
                    "i8".to_string()
                } else {
                    "u8".to_string()
                }
            }
            Some(Type::Int { bits, signed, .. }) => {
                format!("{}{}", if *signed { 'i' } else { 'u' }, bits)
            }
            Some(Type::Float { bits, .. }) => format!("f{bits}"),
            Some(Type::Pointer { pointee, is_const }) => {
                let qualifier = if *is_const { "const" } else { "mut" };
                format!("*{qualifier} {}", self.render(*pointee))
            }
            Some(Type::Named { name, .. }) => name.clone(),
            Some(Type::Unknown { spelling }) => format!("<unknown {spelling}>"),
            None => format!("<missing {id}>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_types_share_an_id() {
        let mut table = TypeTable::new();
        let first = table.intern(Type::int(32, true, "int"));
        let second = table.intern(Type::int(32, true, "int"));
        assert_eq!(first, second);
        assert_eq!(table.len(), 1);
    }

    #[test]
    fn named_types_resolve_to_their_canonical_form() {
        let mut table = TypeTable::new();
        let unsigned_long = table.intern(Type::int(64, false, "unsigned long"));
        let size_t = table.intern(Type::size_t(unsigned_long));
        assert_eq!(
            table.canonical(size_t),
            Some(&Type::int(64, false, "unsigned long"))
        );
        assert!(!table.contains_unknown(size_t));
    }

    #[test]
    fn unknown_is_detected_through_aliases() {
        let mut table = TypeTable::new();
        let unknown = table.intern(Type::Unknown {
            spelling: "auto".to_string(),
        });
        let named = table.intern(Type::Named {
            name: "alias".to_string(),
            canonical: unknown,
        });
        assert!(table.contains_unknown(named));
    }
}

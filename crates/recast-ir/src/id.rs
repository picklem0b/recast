//! Stable identifiers used by TIR and by reports.

use std::fmt;

/// Identifies a top-level or member item.
///
/// M0 has functions only. The text form follows the Clang USR shape so later
/// items can be added without changing the representation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ItemId(String);

impl ItemId {
    /// Build an item identifier from its stable text form.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Build the identifier for a free function in C.
    pub fn function(name: &str) -> Self {
        Self(format!("fn:c:@F@{name}"))
    }

    /// Return the stable text form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Identifies a node inside an item by its pre-order ordinal.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId {
    item: ItemId,
    ordinal: u32,
}

impl NodeId {
    /// Build a node identifier for a position inside `item`.
    pub fn new(item: ItemId, ordinal: u32) -> Self {
        Self { item, ordinal }
    }

    /// The item that owns this node.
    pub fn item(&self) -> &ItemId {
        &self.item
    }

    /// The pre-order position of the node inside its item.
    pub fn ordinal(&self) -> u32 {
        self.ordinal
    }
}

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}#{}", self.item, self.ordinal)
    }
}

/// Identifies an interned type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypeId(u32);

impl TypeId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("type table exceeds u32::MAX entries"))
    }

    /// The position of the type in its table.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "t{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn function_ids_are_stable() {
        assert_eq!(ItemId::function("add").as_str(), "fn:c:@F@add");
    }

    #[test]
    fn node_ids_keep_their_item_and_ordinal() {
        let item = ItemId::function("sum");
        let node = NodeId::new(item.clone(), 7);
        assert_eq!(node.item(), &item);
        assert_eq!(node.ordinal(), 7);
        assert_eq!(node.to_string(), "fn:c:@F@sum#7");
    }
}

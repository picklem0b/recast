#![forbid(unsafe_code)]

/// recast: translated from input.c:1
pub fn checksum(a: u32, b: u32) -> u32 {
    a.wrapping_mul(31).wrapping_add(b)
}

#![forbid(unsafe_code)]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]

/// recast: translated from input.c:1
pub fn checksum(a: u32, b: u32) -> u32 {
    a.wrapping_mul(31).wrapping_add(b)
}

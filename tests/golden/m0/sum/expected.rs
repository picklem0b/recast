#![forbid(unsafe_code)]
#![allow(dead_code, non_camel_case_types, non_snake_case, unused_labels)]

/// recast: translated from input.c:3
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];
        i = i.wrapping_add(1);
    }
    s
}

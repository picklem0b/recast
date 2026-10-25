#![forbid(unsafe_code)]

/// recast: translated from input.c:3
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];
        i += 1;
    }
    s
}

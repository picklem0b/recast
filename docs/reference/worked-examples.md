# 14. Worked Examples

Concrete C/C++ inputs and the Rust output recast is specified to produce, with the facts and rules involved. These double as acceptance fixtures: a builder SHOULD turn each into a golden test (document 10 section 1.1). Output is shown at the _faithful_ stage unless marked _idiomatic_.

Notation: facts use the labels from document 05; `[T0]` etc. are tiers.

---

## Example 1: Pure integer function [T0]

**C**

```c
unsigned int checksum(unsigned int a, unsigned int b) {
    return (a * 31u) + b;
}
```

**Facts**: no pointers; unsigned arithmetic may overflow by range analysis.
**Rules**: TR-0xx (types), TR-4xx (arithmetic, document 06 section 4.2).

**Rust**

```rust
#![forbid(unsafe_code)]
/// recast: translated from util.c:1 [rules: TR-003, TR-401] [tier: T0]
pub fn checksum(a: u32, b: u32) -> u32 {
    a.wrapping_mul(31u32).wrapping_add(b)
}
```

Unsigned wraps explicitly so behavior does not depend on build profile.

---

## Example 2: Slice parameter [T0]

**C**

```c
int sum(const int *a, size_t n) {
    int s = 0;
    for (size_t i = 0; i < n; i++) s += a[i];
    return s;
}
```

**Facts**: `ownership[a] = BorrowedShared (Proven, OWN-004)`; `array_extent[a] = Len(n) (Proven, ARR-003: every access dominated by i < n)`; nullability of `a`: all call sites pass non-null (`Likely`, NUL via call-site evidence); `UB-001` possible on `s += a[i]` (signed overflow, `Likely`).
**Tier**: T0.

**Rust (faithful)**

```rust
#![forbid(unsafe_code)]
/// recast: translated from util.c:41 [rules: TR-201, TR-310] [tier: T0]
pub fn sum(a: &[i32]) -> i32 {
    let mut s: i32 = 0;
    let mut i: usize = 0;
    while i < a.len() {
        s += a[i];          // panics on overflow (overflow-checks = true)
        i += 1;
    }
    s
}
```

The parameter pair `(a, n)` collapses to a single slice because `Len(n)` is proven and `n` has no other use. If `n` had other uses, the output would keep `n` and add `debug_assert!(n <= a.len())`... or flag, per rule TR-205.

**Rust (idiomatic, IDM-001/003)**

```rust
pub fn sum(a: &[i32]) -> i32 {
    a.iter().sum()          // Iterator::sum panics on overflow when overflow-checks = true
}
```

(Equivalent overflow behavior is a precondition for IDM-003; verified by differential tests.)

**Report excerpt**: assumption "a non-null (3/3 call sites)"; UB site `UB-001` listed with translated behavior "panics".

**Callers**: a call `sum(arr, 10)` becomes `sum(&arr[..10])`; a call `sum(p, len)` where `p` is an owned `Vec<i32>` becomes `sum(&p[..len])` (with a bounds panic if `len > p.len()`, which the original would have read out of bounds, `UB-002`).

---

## Example 3: Linked list [T0, owning pointers]

**C**

```c
struct node { int val; struct node *next; };

struct node *push(struct node *head, int v) {
    struct node *n = malloc(sizeof *n);
    n->val = v; n->next = head;
    return n;
}

void free_list(struct node *h) {
    while (h) { struct node *t = h->next; free(h); h = t; }
}
```

**Facts**: `node.next`: `Owning`, nullable (OWN-001, OWN-003, OWN-008); `push.head`: `Consumes`; `push` returns `Fresh(Owning)`; `free_list.h`: `Consumes`; malloc result nullable unless `assume_malloc_succeeds`.

**Rust**

```rust
#![forbid(unsafe_code)]

pub struct Node {
    pub val: i32,
    pub next: Option<Box<Node>>,
}

/// recast: translated from list.c:5 [rules: TR-101, TR-201, TR-701] [tier: T0]
pub fn push(head: Option<Box<Node>>, v: i32) -> Box<Node> {
    Box::new(Node { val: v, next: head })
}

/// recast: translated from list.c:11 [rules: TR-202] [tier: T0]
pub fn free_list(h: Option<Box<Node>>) {
    // Iterative drop (document 06 section 6.7) — avoids stack overflow on long lists.
    let mut cur = h;
    while let Some(mut node) = cur {
        cur = node.next.take();
        // node dropped here, matching free(h)
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        // Iterative teardown of the tail so that dropping a long list does not recurse.
        let mut cur = self.next.take();
        while let Some(mut n) = cur {
            cur = n.next.take();
        }
    }
}
```

Notes: allocation failure is not modeled by default for `Box::new` (Rust aborts); the report lists `malloc`-failure handling difference unless the original checked the result (then the `Option` check is preserved as a branch that cannot occur... and is flagged as dead code candidate).

---

## Example 4: Nullable borrowed pointer [T0]

**C**

```c
const char *find_name(const struct person *p) {
    if (!p) return NULL;
    return p->name;          // name: char[32] field, NUL-terminated
}
```

**Facts**: `p`: `BorrowedShared`, `MaybeNull`; result: `BorrowOf(p)` (region tied to `p`), nullable; `name`: `Fixed(32)`, `NulTerminated`.

**Rust**

```rust
#![forbid(unsafe_code)]
use core::ffi::CStr;

/// recast: translated from person.c:3 [rules: TR-204, TR-210] [tier: T0]
pub fn find_name(p: Option<&Person>) -> Option<&CStr> {
    let p = p?;
    CStr::from_bytes_until_nul(&p.name).ok()   // None if the buffer lacks a NUL (original: UB read)
}
```

The `.ok()` path covers the original UB (`UB-002`) with defined behavior; listed in the report.

---

## Example 5: Aliasing forces a runtime-checked form [T2]

**C**

```c
void swap_add(int *a, int *b) {   // may be called as swap_add(&x, &x)
    *a += *b;
    *b = *a - *b;
    *a = *a - *b;
}
```

**Facts**: `a` and `b` may alias (call sites include `swap_add(&x, &x)`); both `BorrowedMut`; `int` is `Copy`.
**Tier**: T2, `Cell<i32>` (zero overhead, document 06 section 3 rule 6a).

**Rust**

```rust
#![forbid(unsafe_code)]
use core::cell::Cell;

/// recast: translated from swap.c:1 [rules: TR-230] [tier: T2 (Cell)]
pub fn swap_add(a: &Cell<i32>, b: &Cell<i32>) {
    a.set(a.get().wrapping_add(b.get()));  // int_overflow = "wrap" case shown; default uses checked add
    b.set(a.get().wrapping_sub(b.get()));
    a.set(a.get().wrapping_sub(b.get()));
}
```

Callers pass `&Cell<i32>`; `x` becomes `Cell<i32>` at its declaration. If no call site aliased, analysis would have proven `must_not_alias` and chosen `(&mut i32, &mut i32)` at T0.

---

## Example 6: Struct with owned buffer and destructor, C++ [T0]

**C++**

```cpp
class Buffer {
public:
    explicit Buffer(size_t n) : data_(new uint8_t[n]), size_(n) {}
    ~Buffer() { delete[] data_; }
    Buffer(const Buffer&) = delete;
    uint8_t& operator[](size_t i) { return data_[i]; }
    size_t size() const { return size_; }
private:
    uint8_t *data_;
    size_t size_;
};
```

**Facts**: `data_`: `Owning`, `Len(size_)`, non-null; copy deleted; `operator[]` returns a borrow of `*this`.

**Rust**

```rust
#![forbid(unsafe_code)]

pub struct Buffer {
    data_: Vec<u8>,      // new uint8_t[n] -> zero-initialized (original left indeterminate: UB-005 note)
}

impl Buffer {
    /// recast: translated from buffer.h:4 [rules: TR-110, TR-701] [tier: T0]
    pub fn new(n: usize) -> Self { Buffer { data_: vec![0u8; n] } }
    pub fn size(&self) -> usize { self.data_.len() }
}

impl core::ops::Index<usize> for Buffer {
    type Output = u8;
    fn index(&self, i: usize) -> &u8 { &self.data_[i] }          // panics out of bounds (original UB-002)
}
impl core::ops::IndexMut<usize> for Buffer {
    fn index_mut(&mut self, i: usize) -> &mut u8 { &mut self.data_[i] }
}
// No Drop impl needed: Vec frees the buffer. No Clone: copy constructor was deleted.
```

`size_` merged into the `Vec`'s length because every write to either keeps them equal (invariant proven by the field-pair analysis, ARR-005). If not proven, both fields stay with an assertion.

---

## Example 7: Inheritance and virtual functions [T0 with trait objects]

**C++**

```cpp
struct Shape { virtual ~Shape() = default; virtual double area() const = 0; };
struct Circle : Shape { double r; explicit Circle(double r) : r(r) {}
                        double area() const override { return 3.14159 * r * r; } };
struct Rect   : Shape { double w, h; Rect(double w, double h) : w(w), h(h) {}
                        double area() const override { return w * h; } };

double total(const std::vector<std::unique_ptr<Shape>>& v) {
    double t = 0; for (auto& s : v) t += s->area(); return t;
}
```

**Rust**

```rust
#![forbid(unsafe_code)]

pub trait Shape { fn area(&self) -> f64; }

pub struct Circle { pub r: f64 }
impl Circle { pub fn new(r: f64) -> Self { Circle { r } } }
impl Shape for Circle { fn area(&self) -> f64 { 3.14159 * self.r * self.r } }

pub struct Rect { pub w: f64, pub h: f64 }
impl Rect { pub fn new(w: f64, h: f64) -> Self { Rect { w, h } } }
impl Shape for Rect { fn area(&self) -> f64 { self.w * self.h } }

pub fn total(v: &[Box<dyn Shape>]) -> f64 {
    let mut t = 0.0;
    for s in v { t += s.area(); }
    t
}
```

`unique_ptr<Shape>` could be null in the original; if the analysis cannot prove non-null the element type is `Option<Box<dyn Shape>>` and `total` uses a checked unwrap that panics (`UB-006`).
_Closed-hierarchy variant_ (`closed_hierarchy_enum = true`): `enum AnyShape { Circle(Circle), Rect(Rect) }` with `match` in `area`.

---

## Example 8: Function-like macro preserved [T0]

**C**

```c
#define CLAMP(x, lo, hi) ((x) < (lo) ? (lo) : ((x) > (hi) ? (hi) : (x)))
int level(int v) { return CLAMP(v, 0, 10); }
```

**Classification**: `FnExprMultiEval` (arguments evaluated more than once). All call sites pass side-effect-free arguments of uniform type `int`, so a function is faithful.
**Rust**

```rust
#[inline]
fn clamp_i32(x: i32, lo: i32, hi: i32) -> i32 {
    if x < lo { lo } else if x > hi { hi } else { x }
}
pub fn level(v: i32) -> i32 { clamp_i32(v, 0, 10) }
```

A call such as `CLAMP(i++, 0, 10)` has a side-effecting argument: that use site is **expanded in place** (hazard rule, document 05 section 7.4), preserving triple-evaluation semantics, and recorded.

---

## Example 9: Control-flow macro stays a macro [T0]

**C**

```c
#define CHECK(x) do { if (!(x)) return -1; } while (0)
int load(const char *p) { CHECK(p != NULL); /* ... */ return 0; }
```

**Classification**: `ControlFlow` (contains `return`), cannot be a function.
**Rust**

```rust
macro_rules! check {
    ($x:expr) => { if !($x) { return -1; } };
}
pub fn load(p: Option<&CStr>) -> i32 {
    check!(p.is_some());
    0
}
```

---

## Example 10: Conditional compilation [T0, configurations]

**C**

```c
#ifdef USE_FAST
int hash(int x) { return x * 2654435761u; }
#else
int hash(int x) { return x ^ (x >> 16); }
#endif
```

**Atoms**: `USE_FAST` is a feature atom.
**Rust**

```rust
#[cfg(feature = "use_fast")]
pub fn hash(x: i32) -> i32 { x.wrapping_mul(2654435761u32 as i32) }
#[cfg(not(feature = "use_fast"))]
pub fn hash(x: i32) -> i32 { x ^ (x >> 16) }
```

`Cargo.toml` declares `use_fast = []`. In `single` mode only the active branch would be emitted.

---

## Example 11: `goto cleanup` [T0, control flow]

**C**

```c
int process(const char *path) {
    int rc = -1;
    FILE *f = fopen(path, "r");
    if (!f) goto out;
    char *buf = malloc(4096);
    if (!buf) goto close_file;
    rc = read_all(f, buf, 4096);
    free(buf);
close_file:
    fclose(f);
out:
    return rc;
}
```

**Facts**: `f` owning (`fopen` model, release `fclose`), `buf` owning; cleanup labels form forward-only chains.
**Rust**

```rust
pub fn process(path: &CStr) -> i32 {
    let mut rc: i32 = -1;
    'out: {
        let Some(mut f) = recast_rt::stdio::File::open(path, c"r") else { break 'out };
        // `f` is closed on every exit by Drop (matches fclose at close_file)
        let mut buf = vec![0u8; 4096];          // malloc(4096): assume_malloc_succeeds shown
        rc = read_all(&mut f, &mut buf);
        // buf freed by Drop (matches free(buf))
    }
    rc
}
```

Cleanup becomes `Drop`, so every `goto` target collapses into scope exit. The explain record notes the mapping label-to-scope.

---

## Example 12: Unsupported construct flagged [T4]

**C**

```c
#include <setjmp.h>
static jmp_buf env;
int decode(const uint8_t *buf, size_t n) {
    if (setjmp(env)) return -1;
    return parse(buf, n);      // parse() may longjmp(env, 1) on error
}
```

**Result**: `UNSUPPORTED:setjmp`. `decode` stays in C; interop generates a wrapper and a Rust stub with the signature the translation would have used.

**C wrapper (generated)**

```c
int32_t recast_wrap_decode(const uint8_t *buf, size_t n) { return decode(buf, n); }
```

**Rust stub (generated, `ffi` module)**

```rust
extern "C" { fn recast_wrap_decode(buf: *const u8, n: usize) -> i32; }

/// recast: FLAGGED (UNSUPPORTED:setjmp) — implemented by original C via FFI [tier: T4]
pub fn decode(buf: &[u8]) -> i32 {
    // SAFETY: buf is valid for buf.len() bytes for the call; the C function does not retain it.
    unsafe { recast_wrap_decode(buf.as_ptr(), buf.len()) }
}
```

Callers of `decode` in translated code are unaffected. The report lists the flag, the boundary `unsafe`, and the boundary test.

---

## Example 13: UB policy in action

**C**

```c
int avg(int a, int b) { return (a + b) / 2; }   // a + b may overflow: UB
```

- `ub_policy = "report"` (default): `UB-001` finding (`Likely`); Rust `(a + b) / 2` with `overflow-checks = true` panics on overflow; site listed with "original: undefined (observed: wraps on x86-64 -O2)".
- `ub_policy = "strict"`: item flagged (T4) so the original behavior including its UB is preserved.
- `ub_policy = "reject"`: run exits 1 with the finding if `Proven`.
- `int_overflow = "wrap"`: `a.wrapping_add(b) / 2`, matching typical hardware behavior; the report still lists the site.

---

## Example 14: Library mapping, `printf` [T0]

**C**

```c
printf("%-8s|%5.2f|%04x\n", name, ratio, flags);
```

**Rust**

```rust
use std::io::Write;
let _ = write!(std::io::stdout().lock(),
    "{:<8}|{:5.2}|{:04x}\n", name.display_lossy(), ratio, flags);
```

`name` is a `&CStr`; the mapping helper emits bytes without UTF-8 validation (`recast_rt::stdio` writes raw bytes for `%s`). For `%g` or `%a`, which Rust's formatter does not reproduce, `recast_rt::stdio::cformat` is used instead.

---

## Example 15: Tier summary of a tiny module (what the report shows)

| Function            | Source    | Tier | Strategy             | Evidence      | Notes                                                  |
| ------------------- | --------- | ---- | -------------------- | ------------- | ------------------------------------------------------ |
| `checksum`          | util.c:1  | T0   | value types          | E3            | wrapping unsigned                                      |
| `sum`               | util.c:41 | T0   | `&[i32]`             | E3            | assumption: non-null (3/3 call sites); `UB-001` listed |
| `push`, `free_list` | list.c    | T0   | `Box`, `Option<Box>` | E2            | malloc failure not modeled                             |
| `swap_add`          | swap.c:1  | T2   | `Cell<i32>`          | E2            | aliasing at 2 call sites                               |
| `decode`            | codec.c:9 | T4   | FFI stub             | E2 (boundary) | `UNSUPPORTED:setjmp`                                   |

Unsafe: generated code 0 blocks; `recast-rt` 7; FFI boundary 1. Verdict `ACCEPT_WITH_NOTES` under `standard`.

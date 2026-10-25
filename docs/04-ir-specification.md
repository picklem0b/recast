# 04. TIR: The Internal Representation

TIR (recast Intermediate Representation) is the single data structure every pipeline stage reads and writes. It is designed to satisfy four needs that pull in different directions:

1. **Faithfulness**: it must capture C/C++ semantics precisely enough that analysis and translation are sound.
2. **Structure**: it must preserve source structure (loops, `if`, `switch`, names, comments) so output Rust can be readable. A purely low-level IR (SSA/LLVM style) loses this.
3. **Analyzability**: it must support dataflow and points-to analysis. This is done by deriving a control-flow graph (CFG) on demand from the structured tree.
4. **Traceability**: every node must know where it came from (source span, macro expansion, configuration) so reports and explanations point at the original code.

**[Decision]** TIR is a **structured, typed, tree-shaped IR with side tables**, not SSA. Dataflow analyses build per-function CFGs and SSA-like value numbering as _derived, non-persistent_ views.

---

## 1. Core concepts

### 1.1 Identifiers

- `ItemId`: identifies a top-level or member item (function, record, enum, global, typedef, macro, template instantiation). **Derived from the Clang USR** (Unified Symbol Resolution string), normalized, plus a disambiguator for template instantiations (the canonical spelling of the type arguments). Stable across runs and across TUs.
- `NodeId`: identifies an expression, statement, pattern or type node inside an item. Defined as `(ItemId, ordinal)` where `ordinal` is the pre-order index of the node inside the item's body. Stable as long as the item's body is unchanged.
- `TypeId`: interned type identifier. Structurally equal types share one `TypeId` (hash-consing).
- `FileId` and `Span`: files are interned by normalized path relative to the project root; `Span { file, start_line, start_col, end_line, end_col }`.

All IDs are plain integers at runtime and have a stable text form for reports (for example `fn:c:@F@clamp#I#I#I#`).

### 1.2 Provenance

Every `Item` and `Node` carries `Provenance`:

```
Provenance {
  span: Span,                       // location in the *original* file (never in expanded text)
  macro_trail: Vec<MacroFrame>,     // empty if not from a macro; otherwise outermost-to-innermost
  config: ConfigCondition,          // boolean condition over macro/feature atoms under which this node exists
  synthesized: Option<SynthKind>,   // set if recast generated the node (for example implicit constructor)
}

MacroFrame {
  macro_id: ItemId,                 // the macro definition
  use_span: Span,                   // where the macro was invoked
  args: Vec<Span>,                  // spans of the argument spellings
}
```

`ConfigCondition` is a boolean formula over **configuration atoms**, such as `defined(USE_FAST)`, `__linux__`, `VERSION >= 3`. `true` means unconditional. See `05-analysis-specification.md` section 6.

### 1.3 Target description

The program model includes a `Target` record, captured by the exporter, because C/C++ semantics depend on it:

```
Target {
  triple: String,
  pointer_bits: u8,
  int_bits: u8, long_bits: u8, long_long_bits: u8,
  char_is_signed: bool,
  endian: Little | Big,
  max_align: u32,
  size_t_bits: u8,
  float_kinds: [ ... ],
  // layout oracle:
  fn size_of(TypeId) -> Option<u64>
  fn align_of(TypeId) -> Option<u32>
  fn offset_of(RecordId, FieldIdx) -> Option<u64>
}
```

Translation MUST use this record when choosing integer widths (for example, `int` becomes `i32` only if `int_bits == 32`). Multiple targets are not merged; each run has exactly one target.

---

## 2. Program structure

```
Program {
  target: Target,
  files: Vec<FileInfo>,
  items: Map<ItemId, Item>,            // ordered by ItemId text
  call_graph: derived (see section 8)
  configs: Vec<ConfigurationDescriptor>
  schema_version: u32,
  facts: FactTables                    // filled by analysis (section 7)
}
```

### 2.1 Items

```
Item = Function | Record | Enum | Typedef | Global | MacroDef | TemplateDef | Namespace | Using

Function {
  id: ItemId,
  name: QualifiedName,                   // namespace path plus identifier
  linkage: External | Internal | None,
  signature: FnSig,
  body: Option<Block>,                   // None for declarations only
  kind: Free | Method(MethodInfo) | Constructor(CtorInfo) | Destructor | ConversionOp | Operator(OpKind),
  attributes: Vec<Attr>,                 // [[noreturn]], inline, constexpr, __attribute__ ...
  template_origin: Option<TemplateInstance>,  // set if this is an instantiation
  provenance: Provenance,
}

FnSig {
  params: Vec<Param>,                    // Param { name: Option<String>, ty: TypeId, quals, default: Option<Expr> }
  ret: TypeId,
  variadic: bool,
  noexcept: ExceptionSpec,
  cc: CallingConv,                       // C, CxxDefault, others flagged
}

MethodInfo {
  record: ItemId,
  is_static: bool, is_virtual: bool, is_pure: bool, is_override: bool, is_final: bool,
  is_const: bool, ref_qualifier: None | Lvalue | Rvalue,
  access: Public | Protected | Private,
}

Record {                                 // struct, class, union
  id: ItemId,
  name: QualifiedName,
  kind: Struct | Class | Union,
  bases: Vec<BaseSpec>,                  // BaseSpec { record: TypeId, access, is_virtual }
  fields: Vec<Field>,                    // Field { name, ty, access, bit_width: Option<u32>, default_init: Option<Expr>, is_mutable }
  methods: Vec<ItemId>,
  nested: Vec<ItemId>,
  is_polymorphic: bool,                  // has virtual functions or virtual bases
  is_aggregate: bool,
  is_trivially_copyable: bool,
  layout: Option<RecordLayout>,          // sizes and offsets from Target
  provenance: Provenance,
}

Enum { id, name, scoped: bool, underlying: TypeId, variants: Vec<(String, ConstValue)>, provenance }

Typedef { id, name, aliased: TypeId, provenance }

Global { id, name, ty, init: Option<Expr>, storage: Static | Extern | ThreadLocal | Constexpr, is_const: bool, provenance }

MacroDef {
  id, name, params: Option<Vec<String>>,     // None = object-like
  body_tokens: Vec<Token>,
  class: MacroClass,                          // classified by the frontend, see 05 section 7
  uses: Vec<Span>, provenance
}

TemplateDef {                                  // kept for reference and for generic recovery
  id, kind: Function | Class | Alias | Variable,
  params: Vec<TemplateParam>,
  instantiations: Vec<ItemId>,                 // the concrete items Clang actually instantiated
}
```

**Template policy [Decision]:** the Clang AST gives fully type-checked instantiations. TIR stores each _used_ instantiation as an ordinary `Function`/`Record` with `template_origin` set. `TemplateDef` is retained only so the optional generic-recovery pass (`06`, section 12) can merge structurally identical instantiations back into Rust generics.

---

## 3. Types

```
Type =
  | Void | Bool
  | Int { bits: u8, signed: bool, spelling: IntSpelling }   // spelling keeps 'int', 'long', 'size_t', 'uint32_t', 'char' for diagnostics and naming
  | Char { signed: Option<bool> }                           // distinct from Int because char-ness matters for strings
  | Float { bits: 32 | 64 | 80 | 128 }
  | Pointer { pointee: TypeId, quals: Quals }
  | Reference { referent: TypeId, kind: Lvalue | Rvalue, quals: Quals }
  | MemberPointer { class: TypeId, member: TypeId }          // flagged unless trivial
  | Array { elem: TypeId, len: ArrayLen }                    // ArrayLen = Fixed(u64) | Incomplete | Variable(Expr)
  | Function { sig: FnSigType }                              // for function pointers: Pointer(Function)
  | Record(ItemId) | Enum(ItemId)
  | Named { item: ItemId, canonical: TypeId }                // typedef or using alias; keeps the name
  | TemplateParam(idx)                                       // only inside TemplateDef
  | Dependent                                                // unresolved; never present in instantiated code
  | Atomic(TypeId)
  | Vector { elem, lanes }                                   // SIMD; flagged initially
  | Unknown                                                  // exporter could not classify; forces T4

Quals { is_const: bool, is_volatile: bool, is_restrict: bool }
```

Rules:

- `Named` types MUST be preserved (they drive readable output: `size_t`, `FILE`, project typedefs).
- `volatile` is preserved and forces a flag unless it is on an MMIO-style global (outside scope; flagged).
- `restrict` is kept because it is an aliasing _assertion_ that analysis may use; but it is a promise by the original programmer and MUST be treated as a hypothesis checked by verification, not as a proof.
- `Unknown` and `Dependent` MUST NOT survive validation in code that is to be translated.

---

## 4. Statements, expressions and places

### 4.1 Statements

```
Stmt =
  | Decl { var: LocalId, ty: TypeId, init: Option<Init>, storage }
  | Expr(Expr)
  | If { cond: Expr, then: Block, else_: Option<Block>, cond_decl: Option<LocalId> }   // C++17 if-with-initializer supported
  | While { cond, body, label } | DoWhile { body, cond, label }
  | For { init: Option<Box<Stmt>>, cond: Option<Expr>, step: Option<Expr>, body, label }
  | RangeFor { var: LocalId, iterable: Expr, body, label }
  | Switch { scrutinee: Expr, cases: Vec<Case> }       // Case { labels: Vec<ConstExpr> | Default, body: Vec<Stmt>, falls_through: bool }
  | Break(Option<Label>) | Continue(Option<Label>)
  | Return(Option<Expr>)
  | Goto(Label) | LabelStmt(Label, Box<Stmt>)
  | Try { body: Block, handlers: Vec<Handler> }        // Handler { exc_ty: Option<TypeId>, var: Option<LocalId>, body: Block }
  | Throw(Option<Expr>)
  | Block(Block)
  | InlineAsm(AsmInfo)                                 // always flagged
  | Null
Block { stmts: Vec<Stmt>, scope: ScopeId }
```

### 4.2 Expressions

Each expression carries its `TypeId`, `value_category` (`LValue | PRValue | XValue`) and provenance.

```
Expr =
  | Lit(Literal)                                       // int, float, char, string (as bytes with encoding), bool, nullptr
  | Local(LocalId) | Global(ItemId) | Func(ItemId)
  | Unary { op, operand }                              // Neg, Not, BitNot, AddrOf, Deref, PreInc, PreDec, PostInc, PostDec
  | Binary { op, lhs, rhs }                            // arithmetic, bitwise, shifts, comparison, logical (short-circuit marked)
  | Assign { op: Plain | Compound(BinOp), lhs, rhs }
  | Cond { cond, then, else_ }
  | Call { callee: Callee, args: Vec<Expr> }           // Callee = Direct(ItemId) | Indirect(Expr) | Method{recv, item} | Virtual{recv, item}
  | Member { base: Expr, field: FieldRef, arrow: bool }
  | Index { base: Expr, index: Expr }                  // array/pointer subscript; semantics resolved by base type
  | Cast { kind: CastKind, to: TypeId, expr }          // Implicit(Int|Float|Pointer|...), CStyle, StaticCast, ReinterpretCast, ConstCast, DynamicCast
  | SizeOf(TypeId | Expr) | AlignOf(TypeId)
  | New { ty, array_len: Option<Expr>, args, placement: Option<Expr> }
  | Delete { ptr: Expr, array: bool }
  | Construct { ty, args, kind: Direct | Copy | Move | Default | Aggregate | Temporary }
  | Lambda { captures, params, body, ret }
  | InitList { ty, elems }
  | StmtExpr(Block)                                    // GNU statement expression; flagged
  | Comma(Vec<Expr>)
  | Paren(Expr)                                        // kept only to preserve evaluation-order hints; may be dropped
```

Semantic details that MUST be explicit in TIR (not left implicit) because translation needs them:

- **Implicit conversions**: integer promotions, usual arithmetic conversions, array-to-pointer and function-to-pointer decay, lvalue-to-rvalue conversion, user-defined conversions, `bool` conversions. The exporter reports these as `Cast { kind: Implicit(...) }` nodes; the frontend MUST keep them.
- **Evaluation order**: `Binary` with logical `&&`/`||` and `Cond` are marked short-circuit. For other operators and for call arguments, TIR records `eval_order: Specified(order) | Unspecified` per C++17 rules. Unspecified order with conflicting side effects is a UB finding (`UB-021`).
- **Temporaries and lifetime extension**: temporaries carry a `TempId` and a record of their lifetime-end point.
- **Value category** for move semantics: `XValue` marks moveable expressions (`std::move` results).
- **Default arguments, default member initializers, implicit special members**: made explicit as `synthesized` nodes.

### 4.3 Places

A **place** is an expression that denotes a memory location. The following forms are normalized into a canonical `Place`, used heavily by analysis:

```
Place { base: PlaceBase, projections: Vec<Projection> }
PlaceBase = Local(LocalId) | Global(ItemId) | Heap(AllocSiteId) | Temp(TempId) | Param(idx) | Return
Projection = Field(FieldIdx) | Deref | Index(Option<Expr>) | Downcast(RecordId) | BaseClass(RecordId)
```

The Frontend provides `place_of(expr) -> Option<Place>`. This makes field-sensitive reasoning (for example "the `next` field of the node `n` points to") straightforward.

### 4.4 Allocation sites

Every `new`, `malloc`/`calloc`/`realloc`, `std::make_unique`, `std::make_shared`, array allocation, and recognized allocator wrapper is registered as an `AllocSite` with: location, element type, count expression, allocation kind, and matching deallocation function expected (`delete`, `delete[]`, `free`). Mismatch between allocation and deallocation kind is a UB finding.

---

## 5. Control-flow graph (derived)

`cfg(function) -> Cfg` is built on demand:

- Basic blocks of straight-line statements; edges for branches, loops, `switch` (including fallthrough), `goto`, and exceptional edges for calls that may throw (when `noexcept` is not given).
- The CFG refers back to `NodeId`s, never copies nodes.
- Destructor calls at scope exits are inserted as explicit CFG nodes (C++ semantics: automatic destruction in reverse order), because ownership and lifetime analysis require them.
- CFGs are not serialized; they are cheap to rebuild and caching them risks staleness.

---

## 6. Locals and scopes

```
Local { id: LocalId, name: String, ty: TypeId, scope: ScopeId, kind: Param | Var | Temp | Capture, address_taken: bool, provenance }
Scope { id, parent: Option<ScopeId>, kind: Function | Block | Loop | Handler }
```

`address_taken` is computed by the frontend (any `AddrOf` or reference binding), because it determines whether a local can be a plain Rust binding or needs a stable location.

---

## 7. Fact tables (written by analysis)

TIR holds analysis results in side tables keyed by `NodeId`, `ItemId` or `Place`. Analyses never mutate the tree; translation reads both.

| Table              | Key                                | Value                                    | Defined in      |
| ------------------ | ---------------------------------- | ---------------------------------------- | --------------- | -------------- | --------------- | ---------------- | -------------- |
| `points_to`        | Place                              | Set of AbstractObject                    | `05` section 2  |
| `ownership`        | Pointer-typed Place or declaration | `OwnershipKind`                          | `05` section 3  |
| `nullability`      | Pointer-typed Place                | `NonNull                                 | MaybeNull`      | `05` section 4 |
| `mutability`       | Pointer/reference Place            | `ReadOnly                                | ReadWrite`      | `05` section 3 |
| `array_extent`     | Pointer-typed Place                | `Unknown                                 | Fixed(n)        | Len(Place)     | NulTerminated   | Sentinel(c)`     | `05` section 5 |
| `lifetime_regions` | Reference Place                    | Region variable and outlives constraints | `05` section 8  |
| `escape`           | AllocSite or Local                 | `NoEscape                                | EscapesToReturn | EscapesToHeap  | EscapesToThread | EscapesUnknown`  | `05` section 9 |
| `thread_shared`    | AllocSite, Global                  | bool                                     | `05` section 9  |
| `ub_findings`      | NodeId                             | Vec<UbFinding>                           | `05` section 10 |
| `tier_hint`        | ItemId                             | `Tier` lower bound required by analysis  | `06` section 3  |
| `confidence`       | any fact                           | `Proven                                  | Likely          | Assumed        | Unknown`        | `05` section 1.3 |

Every fact carries a `Justification` (list of rule IDs and evidence references) so `recast explain` can reconstruct why.

---

## 8. Derived program views

- **Call graph**: nodes are functions; edges are direct calls, resolved virtual calls (using class hierarchy and, optionally, points-to for receivers), and indirect calls through function pointers resolved by points-to. Unresolved indirect calls are edges to a distinguished `Unknown` node. SCCs are computed deterministically (Tarjan's algorithm with items sorted by `ItemId`).
- **Class hierarchy graph**: bases and derived records, with virtual overrides mapped.
- **Include and module graph**: used to organize output modules.

---

## 9. Serialization

- **Primary format**: a compact binary encoding (Serde + a stable format such as `bincode` with explicit versioning or `postcard`) suitable for caching.
- **Debug format**: JSON (`--emit-tir-json`), human-readable, with spans and IDs in text form.
- **Textual dump** (`recast dump-tir`): a readable pseudo-code listing used in golden tests.
- Every serialized file begins with a header `{ magic: "RECAST-TIR", schema_version, tool_version, target_hash }`. Readers MUST reject mismatched major versions.

---

## 10. Validation invariants

`tir.validate()` MUST run after frontend construction, after every analysis, and before translation. It MUST fail the run (global error) if any invariant is violated, because that indicates a recast bug.

1. Every `NodeId` and `ItemId` referenced exists.
2. Every `Expr` has a `TypeId` and a value category.
3. No `Dependent` or `Unknown` types in items scheduled for translation (items containing them are marked T4 before validation).
4. Every `Call` to a direct callee matches the callee's arity (accounting for default arguments and variadics).
5. All `Break`/`Continue` have an enclosing loop or switch; all `Goto` targets exist in the same function.
6. Every `AllocSite` referenced by a `New`/call is registered.
7. Every implicit conversion between differing types appears as an explicit `Cast`.
8. Provenance spans point inside real files; `synthesized` nodes are exactly those without a source span of their own.
9. Fact tables reference only existing keys.
10. `ConfigCondition` formulas are well-formed and satisfiable (an unsatisfiable condition means dead code and is recorded as a note).

---

## 11. Example (illustrative)

C source:

```c
int sum(const int *a, size_t n) {
  int s = 0;
  for (size_t i = 0; i < n; i++) s += a[i];
  return s;
}
```

TIR dump (`recast dump-tir`, abbreviated):

```
fn sum  [id fn:c:@F@sum#*1I#l#]  linkage=External
  params:
    a : *const i32            [Pointer{pointee=Int{32,signed}, quals={const}}]
    n : size_t                [Named{size_t -> Int{64,unsigned}}]
  ret : i32
  body:
    Decl s : i32 = Lit 0
    For
      init: Decl i : size_t = Lit 0
      cond: Binary Lt (Local i) (Local n)       [implicit Cast none; both size_t]
      step: Unary PostInc (Local i)
      body: Assign Compound(Add) (Local s)
                (Cast Implicit LValueToRValue
                  (Index (Local a) (Local i)))
    Return (Local s)
facts:
  ownership[a]    = Borrowed(shared)    confidence=Proven   rule=OWN-004
  nullability[a]  = NonNull             confidence=Likely   rule=NUL-002  (assumed by 'const int*' + loop use; contract recommended)
  array_extent[a] = Len(n)              confidence=Proven   rule=ARR-003  (indexed only with i < n)
  ub_findings     = []
  tier_hint[sum]  = T0
```

This is the form that flows into translation, which then emits `pub fn sum(a: &[i32]) -> i32` (see `14-worked-examples.md`, example 2).

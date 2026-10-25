# 13. Research and References

What is known about this problem, who has done what, and how recast relates. Statements marked **[Research]** elsewhere in the documents trace back here. Items verified during project research (October 2026) are marked ✔; items recalled from background knowledge and not re-verified are marked ○ and MUST be re-checked before being cited externally.

---

## 1. Prior art: translation systems

| System                                                   | Approach                                                                                                                             | Output                                                     | Key facts                                                                                                    | Relevance to recast                                                                                                        |
| -------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------- |
| **C2Rust** (Immunant / Galois) ✔                         | Clang-based transpiler; stages: transpile, refactor, postprocess                                                                     | Unsafe Rust that mirrors C semantics                       | Used as a baseline in most research; the postprocess stage can use LLMs in newer work                        | The "faithful but unsafe" baseline; recast's T3/T4 are comparable; recast adds ownership-based safe forms and verification |
| **Crown** (CAV 2023) ✔                                   | Constraint-based, field- and flow-sensitive ownership analysis solved with Z3                                                        | Ownership labels used to rewrite pointers                  | Reported to analyze ~500k lines in under 10 seconds; limitations on array pointers and mutable local borrows | Model for ownership inference (document 05 section 3); array-extent and lifetime analyses fill its documented gaps         |
| **Laertes** ✔                                            | Uses `rustc` error messages as an oracle to refine pointer types                                                                     | Fewer raw pointers in C2Rust output                        | Iterative rewrite and check loop                                                                             | Inspires optional compiler-oracle refinement (document 05 section 8.3)                                                     |
| **Hayroll** (PLDI 2026) ✔                                | Symbolic execution of the preprocessor to split configurations and map `#ifdef` to `#[cfg]`; macro classification and reconstruction | Rust with preserved configurability and macro abstractions | Targets two failures of preprocess-first translators                                                         | Architecture model for document 05 sections 6 and 7                                                                        |
| **Cpp2Rust** (PLDI 2026) ✔                               | Translate C++ to safe Rust using runtime-enforced ownership and mutability checks                                                    | Safe Rust with runtime checks                              | About 2% overhead on one benchmark, about 6x on a pointer-arithmetic-heavy one                               | Evidence for the T2 tier and for why pointer-arithmetic classification matters                                             |
| **&inator** (PLDI 2026) ✔                                | Correct and precise C-to-Rust interface translation                                                                                  | Interface-level translation                                | Treats the boundary as its own problem                                                                       | Motivation for document 08                                                                                                 |
| **C2SaferRust** ✔                                        | LLM-assisted refinement of C2Rust output                                                                                             | Fewer raw pointers (reported up to 38%)                    | Shows LLM gains and the need for validation                                                                  | ADR-003: AI may propose, verifier decides                                                                                  |
| **Encrust** ○                                            | Rewrites unsafe code to safe via encapsulation of C interfaces                                                                       | Safer Rust                                                 | Related to the interface/ownership boundary problem                                                          | Reference for T3 encapsulation style                                                                                       |
| **CRustS**, **Ownership guided C-to-Rust translation** ○ | Rule-based cleanups; ownership-guided rewriting                                                                                      | Safer C2Rust output                                        | Background on post-processing                                                                                | Idiom pass design                                                                                                          |
| **DARPA TRACTOR** ✔                                      | Government program to translate C to Rust automatically; MIT Lincoln Laboratory involved in evaluation                               | Program-dependent                                          | Signals large funding and demand                                                                             | Competitive and market context                                                                                             |
| **GrammaTech Platform One "Awardable"** (Sept 2025) ✔    | Vendor announcement related to C-to-Rust migration tooling                                                                           | n/a                                                        | Existing commercial competition                                                                              | Market context                                                                                                             |

---

## 2. Policy and market context ✔

- **CISA memory-safety roadmap guidance** asked software manufacturers to publish memory-safety roadmaps, with a deadline of 1 January 2026.
- U.S. government publications have urged moving to memory-safe languages for new and existing critical software.
- Large vendors (Google, Microsoft, Amazon, Cloudflare and others) have publicly described Rust adoption and the difficulty of safe interop with C++.
- Funding signals for adjacent formal-methods and AI-code companies in 2026 (for example Axiom at a reported $1.6B valuation, Harmonic, Pramaana, Modular) show investor appetite for provable or verified code, though none of these is a direct competitor to a C/C++-to-Rust translator.

_Caveat:_ market numbers are point-in-time figures from news coverage; re-verify before using them in any external material.

---

## 3. Technical references (background)

### 3.1 Language semantics

- ISO C and ISO C++ standards (the undefined-behavior annexes; C++ `[intro.execution]` for evaluation order; `[basic.life]` for object lifetime).
- _The Rust Reference_ and _The Rustonomicon_ (unsafe, layout, FFI, aliasing).
- Rust unsafe code guidelines (aliasing model discussions: Stacked Borrows, Tree Borrows) ○.
- Miri documentation (what it detects and its limits).

### 3.2 Analysis

- Andersen (1994), _Program Analysis and Specialization for the C Programming Language_ (inclusion-based points-to) ○.
- Steensgaard (1996), _Points-to Analysis in Almost Linear Time_ ○.
- Hardekopf and Lin on online cycle elimination and difference propagation for inclusion solvers ○.
- Rust NLL RFC 2094 (non-lexical lifetimes; basis for the region approach) ○.
- Miné, abstract domains for relational numeric analysis (difference-bound matrices, octagons) ○.

### 3.3 Verification

- Clang sanitizers: AddressSanitizer, UndefinedBehaviorSanitizer, MemorySanitizer, ThreadSanitizer documentation.
- libFuzzer and AFL++ documentation; structure-aware fuzzing.
- Kani Rust verifier (bounded model checking) ○.
- Alive2 and translation-validation literature (refinement checking at the LLVM IR level) ○.
- Differential testing background: McKeeman (1998), Csmith (Yang et al., PLDI 2011) ○.

### 3.4 Tooling

- Clang LibTooling and `compile_commands.json` specification.
- `bindgen`, `cbindgen`, `cxx`, Corrosion (CMake), `rules_rust` (Bazel) ○.
- `syn` and `prettyplease`/`rustfmt` for Rust code generation ○.
- Z3 SMT solver documentation and the Rust `z3` bindings ○.
- SARIF 2.1.0 specification for findings interchange ○.

---

## 4. Gaps in current knowledge (to resolve during build)

1. Exact current capability and limits of the 2026 research systems beyond their published abstracts (obtain papers and artifacts when available).
2. Real-world distribution of ownership patterns in target customer code (measure on the corpus at M2 and publish).
3. Best strategy for converting flow-dependent ownership (a pointer that is an owner on one path and a borrow on another) (Q-05).
4. Practical overhead of generation-checked handles versus `Rc` on graph-heavy workloads (benchmark at M2/M8).
5. Customer willingness to pay and procurement paths (business research outside this document set).

---

## 5. How to use this file

- Cite a source in code or documentation only if marked ✔, or after re-verification.
- When a research claim changes a design decision, create or update an ADR in document 12.
- Add new prior art here with the date checked.

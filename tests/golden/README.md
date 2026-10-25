# Golden cases

Each directory holds one case: `input.c`, the exported `ast.json` it was built
from, and `expected.rs`. `xtask check-goldens` compares the pipeline output;
`xtask bless` rewrites expectations and its diff must be reviewed.

M0 has no rule identifiers yet, so cases are grouped by milestone. M1 moves
them under their `TR-nnn` rule as each rule is specified.

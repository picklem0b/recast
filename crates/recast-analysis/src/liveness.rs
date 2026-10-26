//! Backward liveness over the derived CFG.
//!
//! A variable is live at a point when some path from that point reads it before
//! writing it. Translation uses liveness only for diagnostics in M1; the
//! verification report lists the live range size of every function.

use std::collections::BTreeSet;

use crate::cfg::{BlockId, Cfg};

/// Live-in and live-out sets for every block.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Liveness {
    pub live_in: Vec<BTreeSet<String>>,
    pub live_out: Vec<BTreeSet<String>>,
}

impl Liveness {
    /// Variables live on entry to a block.
    pub fn live_in(&self, block: BlockId) -> &BTreeSet<String> {
        &self.live_in[block.0]
    }

    /// Variables live on exit from a block.
    pub fn live_out(&self, block: BlockId) -> &BTreeSet<String> {
        &self.live_out[block.0]
    }

    /// Distinct variables that are live somewhere in the function.
    pub fn tracked(&self) -> BTreeSet<String> {
        let mut names = BTreeSet::new();
        for set in self.live_in.iter().chain(self.live_out.iter()) {
            names.extend(set.iter().cloned());
        }
        names
    }
}

/// Compute liveness for `cfg`. The iteration is monotone and bounded by the
/// number of blocks times the number of locals, so it always terminates.
pub fn solve(cfg: &Cfg) -> Liveness {
    let count = cfg.blocks.len();
    let mut live_in = vec![BTreeSet::new(); count];
    let mut live_out = vec![BTreeSet::new(); count];

    let mut changed = true;
    while changed {
        changed = false;
        for index in (0..count).rev() {
            let block = BlockId(index);
            let mut out = BTreeSet::new();
            for successor in cfg.successors(block) {
                out.extend(live_in[successor.0].iter().cloned());
            }
            let mut in_set = cfg.blocks[index].uses.clone();
            for name in out.difference(&cfg.blocks[index].defs) {
                in_set.insert(name.clone());
            }
            if in_set != live_in[index] || out != live_out[index] {
                live_in[index] = in_set;
                live_out[index] = out;
                changed = true;
            }
        }
    }

    Liveness { live_in, live_out }
}

//! Candidate index: an inverted index over "necessary atoms" to avoid scanning every rule.

use crate::{ast::*, facts::*};
use agave_transaction_view::transaction_data::TransactionData;
// FxHash: keys are tx-derived (not adversary-chosen) and a collision only costs a redundant
// eval, so trade SipHash's DoS-resistance for speed.
use rustc_hash::{FxHashMap, FxHashSet};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Kind {
    Signer,
    FeePayer,
    Program,
    ProgramDisc,
    Account,
    Always,
}

/// Hashable trigger key: kind + raw bytes.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct TrigKey {
    pub kind: Kind,
    pub bytes: Vec<u8>,
}

pub fn always_key() -> TrigKey {
    TrigKey {
        kind: Kind::Always,
        bytes: vec![],
    }
}

pub enum Trig {
    Keys(Vec<TrigKey>),
    Always,
}

fn one(kind: Kind, bytes: &[u8]) -> Trig {
    Trig::Keys(vec![TrigKey {
        kind,
        bytes: bytes.to_vec(),
    }])
}

/// Higher = more selective. ProgramDisc > Account/Signer/FeePayer > Program.
fn selectivity_rank(keys: &[TrigKey]) -> u8 {
    keys.iter()
        .map(|k| match k.kind {
            Kind::ProgramDisc => 4,
            Kind::Account => 3,
            Kind::Signer | Kind::FeePayer => 3,
            Kind::Program => 1,
            Kind::Always => 0,
        })
        .max()
        .unwrap_or(0)
}

/// AND: any ONE true conjunct suffices -> pick the most selective child's keys.
fn most_selective<I: Iterator<Item = Trig>>(it: I) -> Trig {
    let mut best: Option<Vec<TrigKey>> = None;
    for t in it {
        match t {
            Trig::Always => {}
            Trig::Keys(k) => {
                let rank = selectivity_rank(&k);
                let better = best.as_ref().is_none_or(|b| rank > selectivity_rank(b));
                if better {
                    best = Some(k);
                }
            }
        }
    }
    best.map_or(Trig::Always, Trig::Keys)
}

/// OR: must keep EVERY branch's keys (any branch could be the true one); Always if any branch is.
fn union_or_always<I: Iterator<Item = Trig>>(it: I) -> Trig {
    let mut acc = Vec::new();
    for t in it {
        match t {
            Trig::Always => return Trig::Always,
            Trig::Keys(mut k) => acc.append(&mut k),
        }
    }
    if acc.is_empty() {
        Trig::Always
    } else {
        Trig::Keys(acc)
    }
}

pub fn triggers(p: &Pred) -> Trig {
    use Pred::*;
    match p {
        SignerContains(pk) => one(Kind::Signer, pk.0.as_ref()),
        SignerIn(pks) => Trig::Keys(
            pks.0
                .iter()
                .map(|pk| TrigKey {
                    kind: Kind::Signer,
                    bytes: pk.as_ref().to_vec(),
                })
                .collect(),
        ),
        FeePayerIs(pk) => one(Kind::FeePayer, pk.0.as_ref()),
        AccountContains(pk) | WritableAccountContains(pk) | ReadonlyAccountContains(pk) => {
            one(Kind::Account, pk.0.as_ref())
        }
        AnyInstruction(ix) | InstructionAt { pred: ix, .. } => ix_triggers(ix),
        And(v) => most_selective(v.iter().map(triggers)),
        Or(v) => union_or_always(v.iter().map(triggers)),
        _ => Trig::Always,
    }
}

pub fn ix_triggers(p: &IxPred) -> Trig {
    use IxPred::*;
    match p {
        ProgramIdIs(pk) => one(Kind::Program, pk.0.as_ref()),
        IxAccountContains(pk) => one(Kind::Account, pk.0.as_ref()),
        AccountAt { pk, .. } => one(Kind::Account, pk.0.as_ref()),
        And(v) => {
            let prog = v.iter().find_map(|c| {
                if let ProgramIdIs(p) = c {
                    Some(p.0)
                } else {
                    None
                }
            });
            let d0 = v.iter().find_map(|c| match c {
                Discriminator { offset: 0, bytes } => Some(bytes.0.clone()),
                _ => None,
            });
            match (prog, d0) {
                // present_keys only emits ProgramDisc keys for disc lengths {8,4,1}; for any
                // other length fall back to the Program key, else the rule is never a candidate.
                (Some(p), Some(d)) if matches!(d.len(), 1 | 4 | 8) => {
                    let mut key = p.as_ref().to_vec();
                    key.extend_from_slice(&d);
                    one(Kind::ProgramDisc, &key)
                }
                (Some(p), Some(_)) => one(Kind::Program, p.as_ref()),
                (Some(p), None) => one(Kind::Program, p.as_ref()),
                _ => most_selective(v.iter().map(ix_triggers)),
            }
        }
        Or(v) => union_or_always(v.iter().map(ix_triggers)),
        _ => Trig::Always,
    }
}

pub fn rule_triggers(rule: &Rule) -> Trig {
    triggers(&rule.predicate)
}

/// Inverted index over rule **positions** (index in the rule `Vec`).
pub struct CandidateIndex {
    map: FxHashMap<TrigKey, Vec<usize>>,
    always: Vec<usize>,
}

impl CandidateIndex {
    pub fn build(rules: &[Rule]) -> Self {
        let mut map: FxHashMap<TrigKey, Vec<usize>> = FxHashMap::default();
        let mut always = Vec::new();
        for (i, r) in rules.iter().enumerate().filter(|(_, r)| r.enabled) {
            match rule_triggers(r) {
                Trig::Always => always.push(i),
                Trig::Keys(keys) => {
                    for k in keys {
                        map.entry(k).or_default().push(i);
                    }
                }
            }
        }
        Self { map, always }
    }

    /// Candidate rule positions for a tx (Full-ALT path).
    pub fn candidates<D: TransactionData>(&self, tx: &ViewFacts<D>) -> FxHashSet<usize> {
        let mut out: FxHashSet<usize> = self.always.iter().copied().collect();
        for key in present_keys(tx) {
            if let Some(ids) = self.map.get(&key) {
                out.extend(ids);
            }
        }
        out
    }
}

/// All trigger keys the tx exhibits. Skips Unresolved keys (cannot probe an unknown key).
pub fn present_keys<D: TransactionData>(tx: &ViewFacts<D>) -> Vec<TrigKey> {
    let mut v = Vec::new();
    for i in 0..tx.num_signers() {
        if let MaybeKey::Known(p) = tx.account(i) {
            v.push(TrigKey {
                kind: Kind::Signer,
                bytes: p.as_ref().to_vec(),
            });
        }
    }
    if let MaybeKey::Known(p) = tx.account(0) {
        v.push(TrigKey {
            kind: Kind::FeePayer,
            bytes: p.as_ref().to_vec(),
        });
    }
    for i in 0..tx.num_accounts() {
        if let MaybeKey::Known(p) = tx.account(i) {
            v.push(TrigKey {
                kind: Kind::Account,
                bytes: p.as_ref().to_vec(),
            });
        }
    }
    for j in 0..tx.num_instructions() {
        let Some(ix) = tx.instruction(j) else { continue };
        if let MaybeKey::Known(prog) = ix.program_id() {
            v.push(TrigKey {
                kind: Kind::Program,
                bytes: prog.as_ref().to_vec(),
            });
            let data = ix.data();
            for n in [8usize, 4, 1] {
                if data.len() >= n {
                    let mut k = prog.as_ref().to_vec();
                    k.extend_from_slice(&data[..n]);
                    v.push(TrigKey {
                        kind: Kind::ProgramDisc,
                        bytes: k,
                    });
                }
            }
        }
    }
    v
}

//! Evaluation: predicates against `TxFacts` using three-valued logic.

use crate::{ast::*, facts::*, tri::*, value::*};
use solana_pubkey::Pubkey;

fn cmp_i128(lhs: i128, op: Cmp, rhs: i128) -> bool {
    match op {
        Cmp::Eq => lhs == rhs,
        Cmp::Ne => lhs != rhs,
        Cmp::Lt => lhs < rhs,
        Cmp::Le => lhs <= rhs,
        Cmp::Gt => lhs > rhs,
        Cmp::Ge => lhs >= rhs,
    }
}
fn cmp_u(lhs: u64, op: Cmp, rhs: u64) -> bool {
    cmp_i128(lhs as i128, op, rhs as i128)
}
fn cmp_sz(lhs: usize, op: Cmp, rhs: usize) -> bool {
    cmp_i128(lhs as i128, op, rhs as i128)
}

/// Membership over `MaybeKey` slots with a per-index predicate (e.g. writable).
/// Known match -> True. Else: any Unresolved slot that *could* satisfy `pred` -> Unknown; else False.
fn maybe_contains(keys: &[MaybeKey], target: &Pubkey, pred: impl Fn(usize) -> bool) -> Tri {
    let mut unresolved_possible = false;
    for (i, k) in keys.iter().enumerate() {
        match k {
            MaybeKey::Known(p) if p == target && pred(i) => return Tri::True,
            MaybeKey::Unresolved if pred(i) => unresolved_possible = true,
            _ => {}
        }
    }
    if unresolved_possible {
        Tri::Unknown
    } else {
        Tri::False
    }
}

/// Two-valued: instruction data is always fully known. Out-of-bounds slice -> false (no match).
pub fn eval_slice(data: &[u8], offset: usize, kind: SliceKind, op: Cmp, val: &SliceVal) -> bool {
    use SliceKind::*;
    let len = match kind {
        Bytes => match val {
            SliceVal::Bytes(b) => b.0.len(),
            _ => return false,
        },
        U8 => 1,
        U16Le | U16Be => 2,
        U32Le | U32Be => 4,
        U64Le | U64Be | I64Le => 8,
    };
    let Some(slice) = data.get(offset..offset + len) else {
        return false;
    };
    match kind {
        Bytes => {
            let SliceVal::Bytes(b) = val else {
                return false;
            };
            match op {
                Cmp::Eq => slice == b.0.as_slice(),
                Cmp::Ne => slice != b.0.as_slice(),
                _ => false, // validate() forbids other ops on bytes
            }
        }
        _ => {
            let SliceVal::Num(rhs) = val else {
                return false;
            };
            let lhs: i128 = match kind {
                U8 => slice[0] as i128,
                U16Le => u16::from_le_bytes(slice.try_into().unwrap()) as i128,
                U16Be => u16::from_be_bytes(slice.try_into().unwrap()) as i128,
                U32Le => u32::from_le_bytes(slice.try_into().unwrap()) as i128,
                U32Be => u32::from_be_bytes(slice.try_into().unwrap()) as i128,
                U64Le => u64::from_le_bytes(slice.try_into().unwrap()) as i128,
                U64Be => u64::from_be_bytes(slice.try_into().unwrap()) as i128,
                I64Le => i64::from_le_bytes(slice.try_into().unwrap()) as i128,
                Bytes => unreachable!(),
            };
            cmp_i128(lhs, op, *rhs)
        }
    }
}

pub fn eval_pred(p: &Pred, tx: &TxFacts) -> Tri {
    use Pred::*;
    match p {
        And(v) => and3(v.iter().map(|c| eval_pred(c, tx))),
        Or(v) => or3(v.iter().map(|c| eval_pred(c, tx))),
        Not(c) => not3(eval_pred(c, tx)),
        Const(x) => b(*x),

        AnyInstruction(ix) => or3(tx.instructions.iter().map(|i| eval_ix(ix, i))),
        AllInstructions(ix) => and3(tx.instructions.iter().map(|i| eval_ix(ix, i))),
        InstructionAt { index, pred } => tx
            .instructions
            .get(*index)
            .map_or(Tri::False, |i| eval_ix(pred, i)),
        InstructionCount { op, n } => b(cmp_sz(tx.instructions.len(), *op, *n)),

        SignerContains(pk) => b(tx.signers.contains(&pk.0)),
        FeePayerIs(pk) => b(tx.fee_payer == pk.0),
        NumSigners { op, n } => b(cmp_sz(tx.signers.len(), *op, *n)),
        TxVersion(v) => b(tx.version == *v),
        UsesAlt(x) => b(tx.uses_alt == *x),

        AccountContains(pk) => maybe_contains(&tx.account_keys, &pk.0, |_| true),
        WritableAccountContains(pk) => maybe_contains(&tx.account_keys, &pk.0, |i| tx.writable[i]),
        ReadonlyAccountContains(pk) => maybe_contains(&tx.account_keys, &pk.0, |i| !tx.writable[i]),

        ComputeUnitPrice { op, n } => b(cmp_u(tx.compute_unit_price, *op, *n)),
        ComputeUnitLimit { op, n } => b(cmp_u(tx.compute_unit_limit, *op, *n)),
        PriorityFeeLamports { op, n } => b(cmp_u(tx.priority_fee_lamports, *op, *n)),
        TotalFeeLamports { op, n } => b(cmp_u(tx.total_fee_lamports, *op, *n)),
    }
}

pub fn eval_ix(p: &IxPred, ix: &IxFacts) -> Tri {
    use IxPred::*;
    let acc_contains = |target: &Pubkey, pred: &dyn Fn(usize) -> bool| -> Tri {
        let mut unresolved = false;
        for (i, k) in ix.accounts.iter().enumerate() {
            match k {
                MaybeKey::Known(p) if p == target && pred(i) => return Tri::True,
                MaybeKey::Unresolved if pred(i) => unresolved = true,
                _ => {}
            }
        }
        if unresolved {
            Tri::Unknown
        } else {
            Tri::False
        }
    };
    match p {
        And(v) => and3(v.iter().map(|c| eval_ix(c, ix))),
        Or(v) => or3(v.iter().map(|c| eval_ix(c, ix))),
        Not(c) => not3(eval_ix(c, ix)),
        Const(x) => b(*x),

        ProgramIdIs(pk) => match ix.program_id {
            MaybeKey::Known(p) => b(p == pk.0),
            MaybeKey::Unresolved => Tri::Unknown,
        },
        Discriminator { offset, bytes } => b(ix
            .data
            .get(*offset..offset + bytes.0.len())
            .is_some_and(|s| s == bytes.0.as_slice())),
        DataSlice {
            offset,
            kind,
            op,
            value,
        } => b(eval_slice(&ix.data, *offset, *kind, *op, value)),
        DataLen { op, n } => b(cmp_sz(ix.data.len(), *op, *n)),

        AccountAt { index, pk } => match ix.accounts.get(*index) {
            Some(MaybeKey::Known(k)) => b(k == &pk.0),
            Some(MaybeKey::Unresolved) => Tri::Unknown,
            None => Tri::False,
        },
        AccountPropsAt {
            index,
            is_writable,
            is_signer,
        } => b(is_writable.is_none_or(|w| ix.writable.get(*index) == Some(&w))
            && is_signer.is_none_or(|s| ix.signer.get(*index) == Some(&s))),
        IxAccountContains(pk) => acc_contains(&pk.0, &|_| true),
        DecodedName(name) => b(ix.decoded_name.as_deref() == Some(name.as_str())),
    }
}

// ---- rule -> match ----

/// Result of evaluating one rule against one tx.
pub struct RuleOutput {
    /// Did the rule's predicate match (after applying `on_unknown`)?
    pub matched: bool,
    /// If matched, whether to stop evaluating lower-priority rules (`stop_after_match`).
    pub stop: bool,
}

pub fn eval_rule(rule: &Rule, tx: &TxFacts) -> RuleOutput {
    let matched = matches!(resolve(eval_pred(&rule.predicate, tx), rule.on_unknown), Some(true));
    RuleOutput {
        matched,
        stop: matched && rule.stop_after_match,
    }
}

fn resolve(t: Tri, policy: OnUnknown) -> Option<bool> {
    match t {
        Tri::True => Some(true),
        Tri::False => Some(false),
        Tri::Unknown => match policy {
            OnUnknown::TreatTrue => Some(true),
            OnUnknown::Skip | OnUnknown::FailClosed => None,
        },
    }
}

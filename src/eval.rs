//! Evaluation: predicates against a `ViewFacts` using three-valued logic.

use crate::{ast::*, facts::*, tri::*, value::*};
use agave_transaction_view::transaction_data::TransactionData;
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
    let Some(end) = offset.checked_add(len) else {
        return false;
    };
    let Some(slice) = data.get(offset..end) else {
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
                _ => false,
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

/// Membership over the tx's full account list, restricted to slots matching `want`.
/// Known match -> True; else any Unresolved candidate -> Unknown; else False.
fn account_membership<D: TransactionData>(
    tx: &ViewFacts<D>,
    target: &Pubkey,
    want: impl Fn(usize) -> bool,
) -> Tri {
    let mut unresolved = false;
    for i in 0..tx.num_accounts() {
        if !want(i) {
            continue;
        }
        match tx.account(i) {
            MaybeKey::Known(p) if &p == target => return Tri::True,
            MaybeKey::Unresolved => unresolved = true,
            _ => {}
        }
    }
    if unresolved {
        Tri::Unknown
    } else {
        Tri::False
    }
}

pub fn eval_pred<D: TransactionData>(p: &Pred, tx: &ViewFacts<D>) -> Tri {
    use Pred::*;
    match p {
        And(v) => and3(v.iter().map(|c| eval_pred(c, tx))),
        Or(v) => or3(v.iter().map(|c| eval_pred(c, tx))),
        Not(c) => not3(eval_pred(c, tx)),
        Const(x) => b(*x),

        AnyInstruction(ix) => or3((0..tx.num_instructions())
            .filter_map(|j| tx.instruction(j))
            .map(|i| eval_ix(ix, &i))),
        AllInstructions(ix) => and3((0..tx.num_instructions())
            .filter_map(|j| tx.instruction(j))
            .map(|i| eval_ix(ix, &i))),
        InstructionAt { index, pred } => tx
            .instruction(*index)
            .map_or(Tri::False, |i| eval_ix(pred, &i)),
        InstructionCount { op, n } => b(cmp_sz(tx.num_instructions(), *op, *n)),

        SignerContains(pk) => b(tx.signer_contains(&pk.0)),
        FeePayerIs(pk) => b(tx.fee_payer_is(&pk.0)),
        NumSigners { op, n } => b(cmp_sz(tx.num_signers(), *op, *n)),
        TxVersion(v) => b(tx.version() == *v),
        UsesAlt(x) => b(tx.uses_alt() == *x),

        AccountContains(pk) => account_membership(tx, &pk.0, |_| true),
        WritableAccountContains(pk) => account_membership(tx, &pk.0, |i| tx.writable(i)),
        ReadonlyAccountContains(pk) => account_membership(tx, &pk.0, |i| !tx.writable(i)),

        ComputeUnitPrice { op, n } => b(cmp_u(tx.compute_unit_price(), *op, *n)),
        ComputeUnitLimit { op, n } => b(cmp_u(tx.compute_unit_limit(), *op, *n)),
        PriorityFeeLamports { op, n } => b(cmp_u(tx.priority_fee_lamports(), *op, *n)),
        TotalFeeLamports { op, n } => b(cmp_u(tx.total_fee_lamports(), *op, *n)),
    }
}

pub fn eval_ix<D: TransactionData>(p: &IxPred, ix: &IxView<'_, '_, D>) -> Tri {
    use IxPred::*;
    let acc_contains = |target: &Pubkey, want: &dyn Fn(usize) -> bool| -> Tri {
        let mut unresolved = false;
        for k in 0..ix.num_accounts() {
            if !want(k) {
                continue;
            }
            match ix.account(k) {
                Some(MaybeKey::Known(p)) if &p == target => return Tri::True,
                Some(MaybeKey::Unresolved) => unresolved = true,
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

        ProgramIdIs(pk) => match ix.program_id() {
            MaybeKey::Known(p) => b(p == pk.0),
            MaybeKey::Unresolved => Tri::Unknown,
        },
        Discriminator { offset, bytes } => b(offset
            .checked_add(bytes.0.len())
            .and_then(|end| ix.data().get(*offset..end))
            .is_some_and(|s| s == bytes.0.as_slice())),
        DataSlice {
            offset,
            kind,
            op,
            value,
        } => b(eval_slice(ix.data(), *offset, *kind, *op, value)),
        DataLen { op, n } => b(cmp_sz(ix.data().len(), *op, *n)),

        AccountAt { index, pk } => match ix.account(*index) {
            Some(MaybeKey::Known(k)) => b(k == pk.0),
            Some(MaybeKey::Unresolved) => Tri::Unknown,
            None => Tri::False,
        },
        AccountPropsAt {
            index,
            is_writable,
            is_signer,
        } => b(is_writable.is_none_or(|w| ix.writable(*index) == Some(w))
            && is_signer.is_none_or(|s| ix.signer(*index) == Some(s))),
        IxAccountContains(pk) => acc_contains(&pk.0, &|_| true),
        DecodedName(name) => b(ix.decoded_name() == Some(name.as_str())),
    }
}

// ---- rule -> match ----

/// Does this rule match the transaction (after applying its `on_unknown` policy)?
pub fn eval_rule<D: TransactionData>(rule: &Rule, tx: &ViewFacts<D>) -> bool {
    matches!(resolve(eval_pred(&rule.predicate, tx), rule.on_unknown), Some(true))
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

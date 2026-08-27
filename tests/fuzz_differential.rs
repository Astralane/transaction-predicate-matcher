//! Property-based (proptest) stress tests. Two properties:
//!   * `index_matches_bruteforce` — on fully-resolved txns, `match_view` == the `match_view_scan`
//!     oracle. Discriminators are restricted to {1,4,8} to isolate index soundness from FINDING 1.
//!   * `no_panics_on_wild_rules` — no panic for any valid rule over any sanitized tx (offsets kept
//!     below the usize-overflow threshold, pinned separately as FINDING 2).
//!
//! Heavier run: PROPTEST_CASES=20000 cargo test --test fuzz_differential

mod common;

use agave_transaction_view::transaction_view::SanitizedTransactionView;
use common::*;
use proptest::prelude::*;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_pubkey::Pubkey;
use transaction_predicate_matcher::ast::*;
use transaction_predicate_matcher::value::*;
use transaction_predicate_matcher::RuleSet;

// Small fixed pubkey pool so rules and transactions actually collide.
const POOL: u8 = 6;

fn arb_pk() -> impl Strategy<Value = Pubkey> {
    (0u8..POOL).prop_map(pk)
}
fn arb_cmp() -> impl Strategy<Value = Cmp> {
    prop_oneof![
        Just(Cmp::Eq), Just(Cmp::Ne), Just(Cmp::Lt),
        Just(Cmp::Le), Just(Cmp::Gt), Just(Cmp::Ge),
    ]
}

// ---- transaction generator (legacy, fully resolved => no Unknown/Deferred) ----

fn arb_instruction(n_keys: u8) -> impl Strategy<Value = CompiledInstruction> {
    (
        0u8..n_keys,                               // program_id_index
        prop::collection::vec(0u8..n_keys, 0..5),  // account indexes
        prop::collection::vec(any::<u8>(), 0..14), // data
    )
        .prop_map(|(prog, accts, data)| ci(prog, accts, data))
}

/// A structurally-valid legacy transaction's bytes. Header counts are chosen in-range; we still
/// `prop_assume` on the sanitizer to drop any combination it rejects.
fn arb_legacy_bytes() -> impl Strategy<Value = Vec<u8>> {
    (2u8..=POOL).prop_flat_map(|n_keys| {
        let keys: Vec<Pubkey> = (0..n_keys).map(pk).collect();
        (
            Just(keys),
            1u8..=n_keys,                                    // num_required_signatures
            0u8..n_keys,                                     // readonly_signed (clamped below)
            0u8..n_keys,                                     // readonly_unsigned (clamped below)
            prop::collection::vec(arb_instruction(n_keys), 0..5),
        )
            .prop_map(move |(keys, nrs, ro_s, ro_u, ixs)| {
                let n = keys.len() as u8;
                // readonly_signed must leave >=1 writable signer; readonly_unsigned must fit.
                let ro_s = ro_s.min(nrs.saturating_sub(1));
                let ro_u = ro_u.min(n - nrs);
                legacy_bytes(keys, nrs, ro_s, ro_u, ixs)
            })
    })
}

// ---- rule generators ----

/// Discriminator bytes with a length the inverted index supports.
fn arb_disc_supported() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 1..=1),
        prop::collection::vec(any::<u8>(), 4..=4),
        prop::collection::vec(any::<u8>(), 8..=8),
    ]
}

fn arb_ixpred(disc: BoxedStrategy<Vec<u8>>, max_off: usize) -> impl Strategy<Value = IxPred> {
    let leaf = prop_oneof![
        arb_pk().prop_map(|p| IxPred::ProgramIdIs(Pk(p))),
        arb_pk().prop_map(|p| IxPred::IxAccountContains(Pk(p))),
        (0usize..6, arb_pk()).prop_map(|(i, p)| IxPred::AccountAt { index: i, pk: Pk(p) }),
        (0usize..max_off, disc.clone())
            .prop_map(|(offset, b)| IxPred::Discriminator { offset, bytes: Bytes(b) }),
        (0usize..max_off, arb_cmp(), any::<i64>())
            .prop_map(|(offset, op, v)| IxPred::DataSlice {
                offset, kind: SliceKind::U64Le, op, value: SliceVal::Num(v as i128),
            }),
        (arb_cmp(), 0usize..16).prop_map(|(op, n)| IxPred::DataLen { op, n }),
        (0usize..6, any::<bool>(), any::<bool>()).prop_map(|(i, w, s)| IxPred::AccountPropsAt {
            index: i, is_writable: Some(w), is_signer: Some(s),
        }),
    ];
    leaf.prop_recursive(3, 16, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(IxPred::And),
            prop::collection::vec(inner.clone(), 1..4).prop_map(IxPred::Or),
            inner.prop_map(|c| IxPred::Not(Box::new(c))),
        ]
    })
}

fn arb_pred(disc: BoxedStrategy<Vec<u8>>, max_off: usize) -> impl Strategy<Value = Pred> {
    let ix = arb_ixpred(disc, max_off).boxed();
    let leaf = prop_oneof![
        arb_pk().prop_map(|p| Pred::SignerContains(Pk(p))),
        arb_pk().prop_map(|p| Pred::FeePayerIs(Pk(p))),
        arb_pk().prop_map(|p| Pred::AccountContains(Pk(p))),
        arb_pk().prop_map(|p| Pred::WritableAccountContains(Pk(p))),
        arb_pk().prop_map(|p| Pred::ReadonlyAccountContains(Pk(p))),
        (arb_cmp(), 0usize..8).prop_map(|(op, n)| Pred::NumSigners { op, n }),
        (arb_cmp(), 0usize..8).prop_map(|(op, n)| Pred::InstructionCount { op, n }),
        (arb_cmp(), 0u64..2_000_000_000).prop_map(|(op, n)| Pred::PriorityFeeLamports { op, n }),
        (arb_cmp(), 0u64..2_000_000).prop_map(|(op, n)| Pred::ComputeUnitPrice { op, n }),
        Just(Pred::UsesAlt(false)),
        Just(Pred::TxVersion(TxVer::Legacy)),
        ix.clone().prop_map(|i| Pred::AnyInstruction(Box::new(i))),
        ix.clone().prop_map(|i| Pred::AllInstructions(Box::new(i))),
        (0usize..6, ix).prop_map(|(index, p)| Pred::InstructionAt { index, pred: Box::new(p) }),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 1..4).prop_map(Pred::And),
            prop::collection::vec(inner.clone(), 1..4).prop_map(Pred::Or),
            inner.prop_map(|c| Pred::Not(Box::new(c))),
        ]
    })
}

/// A small ordered rule set. Only predicates that pass `validate` become rules.
fn arb_rules(disc: BoxedStrategy<Vec<u8>>, max_off: usize) -> impl Strategy<Value = Vec<Rule>> {
    prop::collection::vec(arb_pred(disc, max_off), 1..6).prop_map(|preds| {
        preds
            .into_iter()
            .enumerate()
            .filter_map(|(i, p)| Rule::from_row(i as i64 + 1, true, OnUnknown::Skip, 1, p).ok())
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 4000, ..ProptestConfig::default() })]

    /// Index-accelerated path must equal the brute-force oracle on fully-resolved txns.
    #[test]
    fn index_matches_bruteforce(
        bytes in arb_legacy_bytes(),
        rules in arb_rules(arb_disc_supported().boxed(), 32),
    ) {
        let Ok(view) = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG) else {
            return Ok(()); // sanitizer rejected this combination; skip
        };
        prop_assume!(!rules.is_empty());
        let rs = RuleSet::new(rules);
        let indexed = rs.match_view(&view, None);
        let scanned = rs.match_view_scan(&view, None);
        prop_assert_eq!(indexed, scanned, "index disagrees with brute force");
    }

    /// No panic for any valid rule over any sanitized tx (offsets/disc-lengths wide but below the
    /// usize-overflow threshold, which is a separate known finding).
    #[test]
    fn no_panics_on_wild_rules(
        bytes in arb_legacy_bytes(),
        rules in arb_rules(prop::collection::vec(any::<u8>(), 1..16).boxed(), 1_000_000),
    ) {
        let Ok(view) = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG) else {
            return Ok(());
        };
        prop_assume!(!rules.is_empty());
        let rs = RuleSet::new(rules);
        // Must not panic. (Return values may differ — disc-length gap — that's not what we assert here.)
        let _ = rs.match_view(&view, None);
        let _ = rs.match_view_scan(&view, None);
    }
}

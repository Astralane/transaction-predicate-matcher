//! Focused reproductions of the defects found stress-testing the matcher, one test per defect.

mod common;

use agave_transaction_view::transaction_view::SanitizedTransactionView;
use common::*;
use transaction_predicate_matcher::ast::*;
use transaction_predicate_matcher::value::*;
use transaction_predicate_matcher::{MatchResult, RuleSet};

fn rule(id: i64, predicate: Pred) -> Rule {
    Rule::from_row(id, true, OnUnknown::Skip, 1, predicate).unwrap()
}

/// Build a legacy tx whose single instruction is `program=keys[1]`, `data=ix_data`.
fn one_ix_tx(program: solana_pubkey::Pubkey, ix_data: Vec<u8>) -> Vec<u8> {
    legacy_bytes(vec![pk(200), program], 1, 0, 1, vec![ci(1, vec![], ix_data)])
}

// FINDING 1 — disc length ∉ {1,4,8}: present_keys emits ProgramDisc keys only for prefix lengths
// {8,4,1}, but ix_triggers keys on the full disc, so match_view drops the rule (scan still matches).
#[test]
fn finding1_discriminator_length_2_is_missed_by_index() {
    let program = pk(11);
    // 2-byte discriminator at offset 0.
    let disc2 = vec![0xABu8, 0xCD];
    let predicate = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(program)),
        IxPred::Discriminator { offset: 0, bytes: Bytes(disc2.clone()) },
    ])));
    let rs = RuleSet::new(vec![rule(7, predicate)]);

    // data starts with the 2-byte discriminator -> the rule logically matches.
    let mut data = disc2.clone();
    data.extend_from_slice(&[0xFF, 0xFF]);
    let bytes = one_ix_tx(program, data);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();

    let indexed = rs.match_view(&view, None);
    let scanned = rs.match_view_scan(&view, None);

    assert_eq!(scanned, MatchResult::Matched(7), "brute-force scan should match the 2-byte disc rule");
    // Pre-fix, the indexed path returned NoMatch here; this guards the regression.
    assert_eq!(
        indexed, scanned,
        "INDEX FALSE-NEGATIVE: match_view ({indexed:?}) disagrees with match_view_scan ({scanned:?}) \
         for a 2-byte discriminator — index only emits ProgramDisc keys for lengths 8/4/1"
    );
}

#[test]
fn finding1b_discriminator_lengths_1_4_8_are_fine() {
    // Control: the same construction with lengths that ARE indexed agrees, confirming the gap is
    // length-specific and not a general breakage.
    for disc in [vec![0x09u8], vec![1u8, 2, 3, 4], vec![1u8, 2, 3, 4, 5, 6, 7, 8]] {
        let program = pk(11);
        let predicate = Pred::AnyInstruction(Box::new(IxPred::And(vec![
            IxPred::ProgramIdIs(Pk(program)),
            IxPred::Discriminator { offset: 0, bytes: Bytes(disc.clone()) },
        ])));
        let rs = RuleSet::new(vec![rule(7, predicate)]);
        let mut data = disc.clone();
        data.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
        let bytes = one_ix_tx(program, data);
        let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
        assert_eq!(
            rs.match_view(&view, None),
            rs.match_view_scan(&view, None),
            "len {} should agree", disc.len()
        );
    }
}

// FINDING 2 — offset+len overflow: offset is an unbounded usize from rule JSON; a near-usize::MAX
// offset panics in eval_slice / Discriminator under overflow checks. Fix: offset.checked_add(len).
#[test]
fn finding2_discriminator_huge_offset_overflow() {
    let program = pk(11);
    let predicate = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(program)),
        // offset near usize::MAX; bytes len 4 -> offset + 4 overflows usize.
        IxPred::Discriminator { offset: usize::MAX - 1, bytes: Bytes(vec![1, 2, 3, 4]) },
    ])));
    let rs = RuleSet::new(vec![rule(7, predicate)]);
    let bytes = one_ix_tx(program, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    // Pre-fix this panicked (overflow) in overflow-checked builds instead of returning NoMatch.
    let r = rs.match_view(&view, None);
    assert_eq!(r, MatchResult::NoMatch, "huge offset must be a clean non-match, not a panic");
}

#[test]
fn finding2b_data_slice_huge_offset_overflow() {
    let program = pk(11);
    let predicate = Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: usize::MAX - 3,
        kind: SliceKind::U64Le,
        op: Cmp::Ge,
        value: SliceVal::Num(0),
    }));
    let rs = RuleSet::new(vec![rule(7, predicate)]);
    let bytes = one_ix_tx(program, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    let r = rs.match_view(&view, None);
    assert_eq!(r, MatchResult::NoMatch, "huge data_slice offset must be a clean non-match, not a panic");
}

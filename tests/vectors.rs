//! Spec §12 test vectors T1–T8 (adapted to the rule-id-only model), schema-version gating, and an
//! index/brute-force equivalence test (§14). Rules carry no decisions; the engine returns matched
//! rule ids and the consumer maps id → action.

use solana_pubkey::Pubkey;
use transaction_predicate_matcher::ast::*;
use transaction_predicate_matcher::eval::{eval_pred, eval_rule};
use transaction_predicate_matcher::facts::*;
use transaction_predicate_matcher::index::*;
use transaction_predicate_matcher::value::*;
use transaction_predicate_matcher::{Engine, MatchResult, Tri};

// ---- helpers ----

fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn wpk(n: u8) -> Pk {
    Pk(pk(n))
}
fn system() -> Pubkey {
    use std::str::FromStr;
    Pubkey::from_str("11111111111111111111111111111111").unwrap()
}

fn ix(program: Pubkey, data: Vec<u8>) -> IxFacts {
    IxFacts {
        program_id: MaybeKey::Known(program),
        accounts: vec![],
        writable: vec![],
        signer: vec![],
        data,
        decoded_name: None,
    }
}

fn ix_acc(program: Pubkey, data: Vec<u8>, accounts: Vec<MaybeKey>) -> IxFacts {
    let n = accounts.len();
    IxFacts {
        program_id: MaybeKey::Known(program),
        accounts,
        writable: vec![false; n],
        signer: vec![false; n],
        data,
        decoded_name: None,
    }
}

fn facts(instructions: Vec<IxFacts>) -> TxFacts {
    TxFacts {
        version: TxVer::V0,
        uses_alt: false,
        signers: vec![pk(200)],
        fee_payer: pk(200),
        account_keys: vec![MaybeKey::Known(pk(200))],
        writable: vec![true],
        has_unresolved: false,
        instructions,
        compute_unit_price: 0,
        compute_unit_limit: 200_000,
        priority_fee_lamports: 0,
        total_fee_lamports: 5000,
    }
}

const Z8: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn disc(bytes: &[u8]) -> IxPred {
    IxPred::Discriminator {
        offset: 0,
        bytes: Bytes(bytes.to_vec()),
    }
}

fn rule(id: i64, priority: i32, ou: OnUnknown, stop: bool, predicate: Pred) -> Rule {
    Rule::from_row(id, format!("r{id}"), true, priority, ou, stop, 1, predicate).unwrap()
}

fn fee_ge(n: u64) -> Pred {
    Pred::PriorityFeeLamports { op: Cmp::Ge, n }
}

fn with_fee(fee: u64) -> TxFacts {
    let mut f = facts(vec![]);
    f.priority_fee_lamports = fee;
    f
}

// ---- T1: same-instruction vs anywhere ----

#[test]
fn t1_same_instruction_vs_anywhere() {
    let p_a = pk(10);
    let p_b = pk(11);
    let tx = facts(vec![
        ix(p_a, vec![0xAA, 0, 0, 0, 0, 0, 0, 0]),
        ix(p_b, Z8.to_vec()),
    ]);

    let r1 = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_b)),
        disc(&Z8),
    ])));
    assert_eq!(eval_pred(&r1, &tx), Tri::True);

    let r2 = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_a)),
        disc(&Z8),
    ])));
    assert_eq!(eval_pred(&r2, &tx), Tri::False);

    let r3 = Pred::And(vec![
        Pred::AnyInstruction(Box::new(IxPred::ProgramIdIs(Pk(p_a)))),
        Pred::AnyInstruction(Box::new(disc(&Z8))),
    ]);
    assert_eq!(eval_pred(&r3, &tx), Tri::True);
}

// ---- T2: data_slice numeric ----

#[test]
fn t2_data_slice_numeric() {
    let mut data = vec![0x02, 0x00, 0x00, 0x00];
    data.extend_from_slice(&500_000_000u64.to_le_bytes());
    let tx = facts(vec![ix(system(), data)]);

    let ge = Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: 4,
        kind: SliceKind::U64Le,
        op: Cmp::Ge,
        value: SliceVal::Num(500_000_000),
    }));
    assert_eq!(eval_pred(&ge, &tx), Tri::True);

    let gt = Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: 4,
        kind: SliceKind::U64Le,
        op: Cmp::Gt,
        value: SliceVal::Num(500_000_000),
    }));
    assert_eq!(eval_pred(&gt, &tx), Tri::False);

    let oob = Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: 100,
        kind: SliceKind::U64Le,
        op: Cmp::Ge,
        value: SliceVal::Num(0),
    }));
    assert_eq!(eval_pred(&oob, &tx), Tri::False);
}

// ---- T3: Tri under unresolved ALT ----

fn t3_facts() -> TxFacts {
    let a = pk(1);
    let s = pk(200);
    TxFacts {
        version: TxVer::V0,
        uses_alt: true,
        signers: vec![s],
        fee_payer: s,
        account_keys: vec![MaybeKey::Known(a), MaybeKey::Unresolved],
        writable: vec![true, true],
        has_unresolved: true,
        instructions: vec![],
        compute_unit_price: 0,
        compute_unit_limit: 200_000,
        priority_fee_lamports: 0,
        total_fee_lamports: 5000,
    }
}

#[test]
fn t3_tri_unresolved() {
    let tx = t3_facts();
    let a = wpk(1);
    let b = wpk(2);
    let s = wpk(200);

    assert_eq!(eval_pred(&Pred::AccountContains(b), &tx), Tri::Unknown);
    assert_eq!(eval_pred(&Pred::AccountContains(a), &tx), Tri::True);
    assert_eq!(
        eval_pred(&Pred::Not(Box::new(Pred::AccountContains(b))), &tx),
        Tri::Unknown
    );
    let and = Pred::And(vec![Pred::SignerContains(s), Pred::AccountContains(b)]);
    assert_eq!(eval_pred(&and, &tx), Tri::Unknown);
    let or = Pred::Or(vec![Pred::SignerContains(s), Pred::AccountContains(b)]);
    assert_eq!(eval_pred(&or, &tx), Tri::True);
}

// ---- T4: first-match-exclusive routing via priority + stop_after_match ----
//
// Replaces the old Switch. Each fee tier is its own rule; descending priority + stop_after_match
// give "first match wins". The consumer maps the winning rule id to a routing action.

fn fee_router() -> Vec<Rule> {
    vec![
        rule(10, 30, OnUnknown::Skip, true, fee_ge(1_000_000_000)), // immediate
        rule(11, 20, OnUnknown::Skip, true, fee_ge(500_000_000)),   // auction
        rule(12, 10, OnUnknown::Skip, true, fee_ge(1)),             // standard
    ]
}

#[test]
fn t4_first_match_exclusive() {
    let engine = Engine::new(fee_router());
    let winner = |fee: u64| match engine.match_tx(&with_fee(fee)) {
        MatchResult::Matched(v) => v,
        MatchResult::Deferred => panic!("not deferred"),
    };
    assert_eq!(winner(2_000_000_000), vec![10]); // immediate (stops after match)
    assert_eq!(winner(500_000_000), vec![11]); // auction
    assert_eq!(winner(1), vec![12]); // standard
    assert_eq!(winner(0), Vec::<i64>::new()); // nothing matched -> consumer's default
}

// ---- T5: Unknown predicate + on_unknown policy ----

#[test]
fn t5_unknown_policy() {
    let tx = t3_facts();
    let b = wpk(2); // account_contains(b) is Unknown under t3 facts

    let skip = rule(1, 0, OnUnknown::Skip, false, Pred::AccountContains(b));
    assert!(!eval_rule(&skip, &tx).matched);

    let treat_true = rule(1, 0, OnUnknown::TreatTrue, false, Pred::AccountContains(b));
    assert!(eval_rule(&treat_true, &tx).matched);
}

// ---- T6: candidate index soundness (Full ALT) ----

#[test]
fn t6_index_soundness() {
    let p_a = pk(10);
    let p_b = pk(11);

    let r_pb = rule(
        100,
        0,
        OnUnknown::Skip,
        false,
        Pred::AnyInstruction(Box::new(IxPred::And(vec![
            IxPred::ProgramIdIs(Pk(p_b)),
            disc(&Z8),
        ]))),
    );
    let r_pa = rule(
        101,
        0,
        OnUnknown::Skip,
        false,
        Pred::AnyInstruction(Box::new(IxPred::And(vec![
            IxPred::ProgramIdIs(Pk(p_a)),
            disc(&Z8),
        ]))),
    );
    let always = rule(1, 0, OnUnknown::Skip, false, fee_ge(1)); // numeric fact -> Always trigger

    let rules = vec![r_pb, r_pa, always];
    let idx = CandidateIndex::build(&rules);

    let tx = facts(vec![
        ix(p_a, vec![0xAA, 0, 0, 0, 0, 0, 0, 0]),
        ix(p_b, Z8.to_vec()),
    ]);

    let cands = idx.candidates(&tx);
    assert!(cands.contains(&100), "P_b rule must be a candidate");
    assert!(cands.contains(&1), "Always rule must be a candidate");
    assert!(!cands.contains(&101), "P_a rule must NOT be a candidate");

    let engine = Engine::new(rules);
    let MatchResult::Matched(idx_ids) = engine.match_tx(&tx) else {
        panic!("expected matched");
    };
    assert_eq!(idx_ids, engine.match_tx_scan(&tx));
    assert_eq!(idx_ids, vec![100]); // only the P_b rule actually matches
}

// ---- T7: defer on partial ALT ----

#[test]
fn t7_defer_partial_alt() {
    let engine = Engine::new(fee_router());
    let tx = t3_facts(); // has_unresolved = true
    assert!(matches!(engine.match_tx(&tx), MatchResult::Deferred));
}

// ---- T8: load failures (now parse Pred, then validate) ----

#[test]
fn t8_load_failures() {
    // bad base58
    let j = r#"{"signer_contains":"not_base58!!"}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());

    // bad hex discriminator
    let j = r#"{"any_instruction":{"discriminator":{"offset":0,"bytes":"zz"}}}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());

    // data_slice value/kind mismatch -> parses structurally, fails validate
    let j = r#"{"any_instruction":{"data_slice":{"offset":0,"kind":"u64_le","op":"eq","value":"deadbeef"}}}"#;
    let p: Pred = serde_json::from_str(j).expect("parses structurally");
    assert!(validate(&p).is_err(), "kind/val mismatch must fail validate");

    // unknown atom -> deny_unknown_fields
    let j = r#"{"frobnicate":1}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());
}

// ---- schema versioning: future rules are skipped, not fatal ----

#[test]
fn schema_version_gating() {
    use transaction_predicate_matcher::{load_rules, LoadError, RawRule, ENGINE_SCHEMA_VERSION};

    let body = serde_json::json!({ "uses_alt": true });
    let future = RawRule {
        id: 1,
        name: "future".into(),
        enabled: true,
        priority: 0,
        on_unknown: OnUnknown::Skip,
        stop_after_match: false,
        schema_version: ENGINE_SCHEMA_VERSION + 1,
        predicate: body.clone(),
    };
    let current = RawRule {
        id: 2,
        name: "current".into(),
        enabled: true,
        priority: 0,
        on_unknown: OnUnknown::Skip,
        stop_after_match: false,
        schema_version: ENGINE_SCHEMA_VERSION,
        predicate: body,
    };

    let (ok, errs) = load_rules(vec![future, current]);
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].id, 2);
    assert_eq!(errs.len(), 1);
    assert!(matches!(
        errs[0],
        (1, LoadError::UnsupportedSchemaVersion { .. })
    ));
}

// ---- §14: index == brute force over assorted facts ----

#[test]
fn index_equals_bruteforce() {
    let p_a = pk(10);
    let p_b = pk(11);
    let dest = pk(50);

    let rules = vec![
        rule(
            100,
            5,
            OnUnknown::Skip,
            false,
            Pred::AnyInstruction(Box::new(IxPred::And(vec![
                IxPred::ProgramIdIs(Pk(p_b)),
                disc(&Z8),
            ]))),
        ),
        rule(101, 3, OnUnknown::Skip, false, Pred::AccountContains(Pk(dest))),
        rule(1, 1, OnUnknown::Skip, false, fee_ge(1)), // Always
    ];
    let engine = Engine::new(rules);

    let mut with_dest = facts(vec![ix_acc(p_a, vec![9], vec![MaybeKey::Known(dest)])]);
    with_dest.account_keys.push(MaybeKey::Known(dest));
    with_dest.writable.push(false);

    let cases = [
        facts(vec![ix(p_b, Z8.to_vec())]),
        facts(vec![ix(p_a, vec![0xAA])]),
        with_dest,
        with_fee(750_000_000),
    ];
    for (i, tx) in cases.iter().enumerate() {
        let MatchResult::Matched(idx) = engine.match_tx(tx) else {
            panic!("not deferred");
        };
        assert_eq!(idx, engine.match_tx_scan(tx), "case {i}");
    }
    // the dest case must actually match rule 101 via the index path.
    let MatchResult::Matched(d) = engine.match_tx(&cases[2]) else {
        unreachable!()
    };
    assert!(d.contains(&101));
}

//! Spec test vectors (adapted to the view-based, rule-id-only model), schema-version gating, and
//! an index/brute-force equivalence test. Rules return matched ids; the consumer maps id → action.

mod common;

use agave_transaction_view::transaction_view::SanitizedTransactionView;
use common::*;
use solana_message::v0::MessageAddressTableLookup;
use solana_pubkey::Pubkey;
use transaction_predicate_matcher::ast::*;
use transaction_predicate_matcher::eval::{eval_pred, eval_rule};
use transaction_predicate_matcher::index::*;
use transaction_predicate_matcher::value::*;
use transaction_predicate_matcher::{RuleSet, MatchResult, Tri, ViewFacts};

const Z8: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

fn wpk(n: u8) -> Pk {
    Pk(pk(n))
}
fn disc(bytes: &[u8]) -> IxPred {
    IxPred::Discriminator {
        offset: 0,
        bytes: Bytes(bytes.to_vec()),
    }
}
fn rule(id: i64, predicate: Pred) -> Rule {
    Rule::from_row(id, true, OnUnknown::Skip, 1, predicate).unwrap()
}
fn rule_ou(id: i64, ou: OnUnknown, predicate: Pred) -> Rule {
    Rule::from_row(id, true, ou, 1, predicate).unwrap()
}
fn fee_ge(n: u64) -> Pred {
    Pred::PriorityFeeLamports { op: Cmp::Ge, n }
}

// Macro: build a view from bytes and run a closure with its ViewFacts. (The view borrows `bytes`,
// so the binding has to live in the caller's scope.)
macro_rules! facts {
    ($bytes:expr, $alt:expr, $f:ident => $body:expr) => {{
        let bytes = $bytes;
        let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
        let $f = ViewFacts::new(&view, $alt);
        $body
    }};
}

// ---- T1: same-instruction vs anywhere ----

#[test]
fn t1_same_instruction_vs_anywhere() {
    let p_a = pk(10);
    let p_b = pk(11);
    // keys: [payer, P_a, P_b]; ix0 = P_a/0xAA…, ix1 = P_b/Z8
    let bytes = legacy_bytes(
        vec![pk(200), p_a, p_b],
        1,
        0,
        2,
        vec![
            ci(1, vec![], vec![0xAA, 0, 0, 0, 0, 0, 0, 0]),
            ci(2, vec![], Z8.to_vec()),
        ],
    );
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    let f = ViewFacts::new(&view, None);

    let r1 = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_b)),
        disc(&Z8),
    ])));
    assert_eq!(eval_pred(&r1, &f), Tri::True);

    let r2 = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_a)),
        disc(&Z8),
    ])));
    assert_eq!(eval_pred(&r2, &f), Tri::False);

    let r3 = Pred::And(vec![
        Pred::AnyInstruction(Box::new(IxPred::ProgramIdIs(Pk(p_a)))),
        Pred::AnyInstruction(Box::new(disc(&Z8))),
    ]);
    assert_eq!(eval_pred(&r3, &f), Tri::True);
}

// ---- T2: data_slice numeric ----

#[test]
fn t2_data_slice_numeric() {
    let mut data = vec![0x02, 0x00, 0x00, 0x00];
    data.extend_from_slice(&500_000_000u64.to_le_bytes());
    let bytes = legacy_bytes(vec![pk(200), system()], 1, 0, 1, vec![ci(1, vec![0], data)]);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    let f = ViewFacts::new(&view, None);

    let mk = |op, val| Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: 4,
        kind: SliceKind::U64Le,
        op,
        value: SliceVal::Num(val),
    }));
    assert_eq!(eval_pred(&mk(Cmp::Ge, 500_000_000), &f), Tri::True);
    assert_eq!(eval_pred(&mk(Cmp::Gt, 500_000_000), &f), Tri::False);

    let oob = Pred::AnyInstruction(Box::new(IxPred::DataSlice {
        offset: 100,
        kind: SliceKind::U64Le,
        op: Cmp::Ge,
        value: SliceVal::Num(0),
    }));
    assert_eq!(eval_pred(&oob, &f), Tri::False);
}

#[test]
fn t2_data_contains_j7_uri() {
    // RPC transaction body for XYZ/J7 creation signature
    // 5weRXT7rP5hrF4Ab1D5rpLdWqZqUvGxEZY6Cu6kJEfmhQFGcWLojaSjBsk6oPPNKqzbdbCxnUT84vSaPRmX3aYRz
    // at slot 438173858 (mint CpjesB7n6WV8V21CPAZ1yUV25J73iQLBZvx2Gma3pump).
    let bytes = hex::decode(concat!(
        "02f731242d12288b0cb4b81ad41c3e1ad37a773c7bf248459334479be378b581b0665f63a27a94343f5ff765f8cc65aef7c2ba5bff655559703c81ffb2a4257a0de3453babf6a5db96b1cf82c1f9983297d24942a11da45bdd47e8e5dacca8649a78f4dd3cc32f1f56f13b89537ab145b096939e265600683fafe2e17c5f2c7a018",
        "0020005103e6fee7cb51e2832bed22d33004dcc0853a96d1a1d06c3c4dac58b66528abd25afab14fa3c4958dee40b529e9ce24c905640bc5f04d48b1ae369818b6127020f07dd131776e9d220a5ee26d651779856eff659baf1dacbbc13f947e8807f46152425d2af2ca5e5637b5cba88f60a451d6893eca9b1ae44100b6624597451fdc071858952744f342c84cc8fe1567e2328b382b90db56d24ceabd68e42e80c1c4092a617915cb18c7a4c4c51659c2d7964fd87e86cd2435e55d47118940b60091a9e69fdca6be1999b8e6f4f9cb60bc29c377cffcdd754c1f704ee5de253a9103a9f4e902f2b3b1e2ce9e16c0acc233e4402794a11d2f6617ba5bef3f5198b8574a6bf7b76b8717316acb4049b1c26f8d56f72e4f1a892b4e2490c96b87f95923cea5c60396519dec70cc904999848f86b09ef2e2ef9c2d7d302729a6acdfc5d9eed88bc82b7b486a44909aa87d140daf5e3896bec5b8c47a1fc70cab8ebfcf14500000000000000000000000000000000000000000000000000000000000000000156e0f693665acf44db1568bf175baa5189cb97f5d2ff3b655d2bb6fd6d18b00306466fe5211732ffecadba72c39be7bc8ce5bbc5f7126b2c439b3a400000002f4c7ae322827d783dff60d0574ebef88bb766b96ab5f5d406b52b8d7e1621748c97258f4e2489f1bb3d1029148e0d830b5a1399daff1084048e7bd8dbe9f85982eaf79ecdd0e02d9d9e580223b134cf197e8cc076ce87c591e572a2793353c2070d01190502e09304000d000903d5dc3200000000000c10011702061b000b180f1115130a081d0c8001",
        "d6904cec5f8b31b40b00000041736820746865206f776c030000004173683c00000068747470733a2f2f6d657461646174612e6a37747261636b65722e696f2f6d657461646174612f653865343166336263643633343433632e6a736f6e3e6fee7cb51e2832bed22d33004dcc0853a96d1a1d06c3c4dac58b66528abd2500000f07000300010b181601010c121b1201020603000b18091d0c14041c1a0e101966063d1201daebea2eea2a92b75800000001b2c400000000010b0200070c0200000080c3c901000000000b0200050c0200000040420f000000000001fe5adb299631b95babd31505f37312db1af2b2718d682f45358e4a90b2a00d2d052c07040a24090911060b2a05012502"
    ))
    .unwrap();
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    let f = ViewFacts::new(&view, None);

    let predicate_json = serde_json::json!({
        "or": [
            {
                "any_instruction": {
                    "and": [
                        { "program_id_is": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P" },
                        { "discriminator": { "offset": 0, "bytes": "181ec828051c0777" } },
                        { "data_contains": "6d657461646174612e6a37747261636b65722e696f" }
                    ]
                }
            },
            {
                "any_instruction": {
                    "and": [
                        { "program_id_is": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P" },
                        { "discriminator": { "offset": 0, "bytes": "d6904cec5f8b31b4" } },
                        { "data_contains": "6d657461646174612e6a37747261636b65722e696f" }
                    ]
                }
            }
        ]
    });
    let rule = Rule::load_from_json(
        7,
        true,
        OnUnknown::Skip,
        ENGINE_SCHEMA_VERSION,
        &predicate_json,
    )
    .unwrap();
    assert!(eval_rule(&rule, &f));

    let wrong_uri_json = serde_json::json!({
        "any_instruction": {
            "data_contains": "6d657461646174612e6578616d706c652e636f6d"
        }
    });
    let wrong_uri = Rule::load_from_json(
        8,
        true,
        OnUnknown::Skip,
        ENGINE_SCHEMA_VERSION,
        &wrong_uri_json,
    )
    .unwrap();
    assert!(!eval_rule(&wrong_uri, &f));

    let empty_json = serde_json::json!({
        "any_instruction": { "data_contains": "" }
    });
    assert!(Rule::load_from_json(
        9,
        true,
        OnUnknown::Skip,
        ENGINE_SCHEMA_VERSION,
        &empty_json,
    )
    .is_err());
}

#[test]
fn bulk_signer_in_uses_indexed_set() {
    let signers = (0..11_000u32)
        .map(|n| {
            let mut bytes = [0u8; 32];
            bytes[..4].copy_from_slice(&n.to_le_bytes());
            Pubkey::new_from_array(bytes)
        })
        .collect::<Vec<_>>();
    let target = signers[10_999];
    let predicate_json = serde_json::json!({
        "signer_in": signers.iter().map(ToString::to_string).collect::<Vec<_>>()
    });
    let rule = Rule::load_from_json(
        10,
        true,
        OnUnknown::Skip,
        ENGINE_SCHEMA_VERSION,
        &predicate_json,
    )
    .unwrap();

    match rule_triggers(&rule) {
        Trig::Keys(keys) => assert_eq!(keys.len(), 11_000),
        Trig::Always => panic!("signer_in must be candidate-indexed"),
    }

    let rules = RuleSet::new(vec![rule]);
    let matching = legacy_bytes(
        vec![target, system()],
        1,
        0,
        1,
        vec![ci(1, vec![0], vec![])],
    );
    let matching_view =
        SanitizedTransactionView::try_new_sanitized(matching.as_slice(), &SANITIZE_CONFIG).unwrap();
    assert_eq!(
        rules.match_view(&matching_view, None),
        MatchResult::Matched(10)
    );

    let absent = Pubkey::new_from_array([0xff; 32]);
    let non_matching = legacy_bytes(
        vec![absent, system()],
        1,
        0,
        1,
        vec![ci(1, vec![0], vec![])],
    );
    let non_matching_view =
        SanitizedTransactionView::try_new_sanitized(non_matching.as_slice(), &SANITIZE_CONFIG).unwrap();
    assert_eq!(rules.match_view(&non_matching_view, None), MatchResult::NoMatch);

    let empty_json = serde_json::json!({ "signer_in": [] });
    assert!(Rule::load_from_json(
        11,
        true,
        OnUnknown::Skip,
        ENGINE_SCHEMA_VERSION,
        &empty_json,
    )
    .is_err());
}

// ---- T3 / T5 / T7 fixtures: a v0 tx with an unresolved ALT slot ----
//
// static = [payer(=signer), A(readonly)]; one uncached lookup contributes a writable ALT slot.
// account list: [payer, A, Unresolved].
fn unresolved_alt_bytes() -> Vec<u8> {
    v0_bytes(
        vec![pk(200), pk(5)],
        1,
        0,
        1, // A (index 1) readonly
        vec![ci(1, vec![0], vec![])],
        vec![MessageAddressTableLookup {
            account_key: pk(42),
            writable_indexes: vec![0],
            readonly_indexes: vec![],
        }],
    )
}

#[test]
fn t3_tri_unresolved() {
    let s = wpk(200); // payer / signer
    let a = wpk(5); // known static account
    let b = wpk(2); // never present -> hides behind the unresolved slot

    facts!(unresolved_alt_bytes(), None, f => {
        assert!(f.has_unresolved());
        assert_eq!(eval_pred(&Pred::AccountContains(a), &f), Tri::True);
        assert_eq!(eval_pred(&Pred::AccountContains(b), &f), Tri::Unknown);
        assert_eq!(
            eval_pred(&Pred::Not(Box::new(Pred::AccountContains(b))), &f),
            Tri::Unknown
        );
        let and = Pred::And(vec![Pred::SignerContains(s), Pred::AccountContains(b)]);
        assert_eq!(eval_pred(&and, &f), Tri::Unknown);
        let or = Pred::Or(vec![Pred::SignerContains(s), Pred::AccountContains(b)]);
        assert_eq!(eval_pred(&or, &f), Tri::True);
    });
}

// ---- T4: first-match-exclusive routing via priority + stop_after_match ----

// Priority is Vec order; the first matching rule's id is returned.
fn fee_router() -> Vec<Rule> {
    vec![
        rule(10, fee_ge(1_000_000_000)), // immediate
        rule(11, fee_ge(500_000_000)),   // auction
        rule(12, fee_ge(1)),             // standard
    ]
}

#[test]
fn t4_first_match() {
    let rules = RuleSet::new(fee_router());
    let winner = |fee: u64| {
        let bytes = fee_tx_bytes(fee);
        let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
        rules.match_view(&view, None)
    };
    assert_eq!(winner(2_000_000_000), MatchResult::Matched(10)); // immediate
    assert_eq!(winner(500_000_000), MatchResult::Matched(11)); // auction
    assert_eq!(winner(1), MatchResult::Matched(12)); // standard
    assert_eq!(winner(0), MatchResult::NoMatch);
}

// ---- T5: Unknown predicate + on_unknown policy ----

#[test]
fn t5_unknown_policy() {
    let b = wpk(2);
    facts!(unresolved_alt_bytes(), None, f => {
        let skip = rule(1, Pred::AccountContains(b));
        assert!(!eval_rule(&skip, &f));
        let treat_true = rule_ou(1, OnUnknown::TreatTrue, Pred::AccountContains(b));
        assert!(eval_rule(&treat_true, &f));
    });
}

// ---- T6: candidate index soundness (Full ALT) ----

#[test]
fn t6_index_soundness() {
    let p_a = pk(10);
    let p_b = pk(11);
    // Vec order: [0] = P_b rule (id 100), [1] = P_a rule (id 101), [2] = always (id 1).
    let r_pb = rule(100, Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_b)),
        disc(&Z8),
    ]))));
    let r_pa = rule(101, Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(p_a)),
        disc(&Z8),
    ]))));
    let always = rule(1, fee_ge(1));
    let rules = vec![r_pb, r_pa, always];
    let idx = CandidateIndex::build(&rules);

    let bytes = legacy_bytes(
        vec![pk(200), p_a, p_b],
        1,
        0,
        2,
        vec![
            ci(1, vec![], vec![0xAA, 0, 0, 0, 0, 0, 0, 0]),
            ci(2, vec![], Z8.to_vec()),
        ],
    );
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    let f = ViewFacts::new(&view, None);

    let cands = idx.candidates(&f);
    assert!(cands.contains(&0), "P_b rule must be a candidate");
    assert!(cands.contains(&2), "Always rule must be a candidate");
    assert!(!cands.contains(&1), "P_a rule must NOT be a candidate");

    let rs = RuleSet::new(rules);
    assert_eq!(rs.match_view(&view, None), MatchResult::Matched(100));
    assert_eq!(rs.match_view_scan(&view, None), MatchResult::Matched(100));
}

// ---- T7: defer on partial ALT ----

#[test]
fn t7_defer_partial_alt() {
    let engine = RuleSet::new(fee_router());
    let bytes = unresolved_alt_bytes();
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
    assert!(matches!(engine.match_view(&view, None), MatchResult::Deferred));
}

// ---- T8: load failures (parse Pred, then validate) ----

#[test]
fn t8_load_failures() {
    let j = r#"{"signer_contains":"not_base58!!"}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());

    let j = r#"{"any_instruction":{"discriminator":{"offset":0,"bytes":"zz"}}}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());

    let j = r#"{"any_instruction":{"data_slice":{"offset":0,"kind":"u64_le","op":"eq","value":"deadbeef"}}}"#;
    let p: Pred = serde_json::from_str(j).expect("parses structurally");
    assert!(validate(&p).is_err(), "kind/val mismatch must fail validate");

    let j = r#"{"frobnicate":1}"#;
    assert!(serde_json::from_str::<Pred>(j).is_err());
}

// ---- schema versioning: future rules skipped, not fatal ----

#[test]
fn schema_version_gating() {
    use transaction_predicate_matcher::{load_rules, LoadError, RawRule, ENGINE_SCHEMA_VERSION};

    let body = serde_json::json!({ "uses_alt": true });
    let mk = |id: i64, ver: u32| RawRule {
        id,
        enabled: true,
        on_unknown: OnUnknown::Skip,
        schema_version: ver,
        predicate: body.clone(),
    };

    // rule 1 is from the future (skipped), rule 2 is current (loads).
    let (ok, errs) = load_rules(vec![mk(1, ENGINE_SCHEMA_VERSION + 1), mk(2, ENGINE_SCHEMA_VERSION)]);
    assert_eq!(ok.len(), 1);
    assert_eq!(ok[0].id, 2);
    assert!(matches!(errs[0], (1, LoadError::UnsupportedSchemaVersion { .. })));
}

// ---- index == brute force over assorted facts ----

#[test]
fn index_equals_bruteforce() {
    let p_a = pk(10);
    let p_b = pk(11);
    let dest = pk(50);

    // Vec order: [0] P_b rule (100), [1] account=dest rule (101), [2] always/fee (1).
    let rules = vec![
        rule(100, Pred::AnyInstruction(Box::new(IxPred::And(vec![
            IxPred::ProgramIdIs(Pk(p_b)),
            disc(&Z8),
        ])))),
        rule(101, Pred::AccountContains(Pk(dest))),
        rule(1, fee_ge(1)),
    ];
    let rs = RuleSet::new(rules);

    let cases = [
        legacy_bytes(vec![pk(200), p_b], 1, 0, 1, vec![ci(1, vec![], Z8.to_vec())]),
        legacy_bytes(vec![pk(200), p_a], 1, 0, 1, vec![ci(1, vec![], vec![0xAA])]),
        // dest present as a static account
        legacy_bytes(vec![pk(200), p_a, dest], 1, 0, 2, vec![ci(1, vec![2], vec![9])]),
        fee_tx_bytes(750_000_000),
    ];
    for (i, bytes) in cases.iter().enumerate() {
        let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), &SANITIZE_CONFIG).unwrap();
        assert_eq!(
            rs.match_view(&view, None),
            rs.match_view_scan(&view, None),
            "case {i}"
        );
    }
    // dest case must hit the account rule (id 101).
    let view = SanitizedTransactionView::try_new_sanitized(cases[2].as_slice(), &SANITIZE_CONFIG).unwrap();
    assert_eq!(rs.match_view(&view, None), MatchResult::Matched(101));
}

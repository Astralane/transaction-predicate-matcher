//! ViewFacts construction: canonical account ordering, writability, ComputeBudget fee math, and
//! ALT resolution (resolved vs unresolved) including the RuleSet::match_view path.

mod common;

use agave_transaction_view::transaction_view::SanitizedTransactionView;
use common::*;
use solana_message::v0::MessageAddressTableLookup;
use solana_pubkey::Pubkey;
use std::collections::HashMap;
use std::sync::RwLock;
use transaction_predicate_matcher::value::{Pk, TxVer};
use transaction_predicate_matcher::{
    AccountLookupTableCache, AltLookup, RuleSet, MatchResult, MaybeKey, OnUnknown, Pred, Rule,
    ViewFacts,
};

#[test]
fn legacy_transfer_with_priority_fee() {
    let payer = pk(1);
    let dest = pk(2);
    let system = system();
    let cb = compute_budget();

    // keys: [payer(signer,writable), dest(writable), system(ro), cb(ro)]
    let mut tr_data = vec![0x02u8, 0, 0, 0];
    tr_data.extend_from_slice(&500_000_000u64.to_le_bytes());
    let bytes = legacy_bytes(
        vec![payer, dest, system, cb],
        1,
        0,
        2,
        vec![
            ci(3, vec![], cb_price(1000)),
            ci(2, vec![0, 1], tr_data),
        ],
    );
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice()).unwrap();
    let f = ViewFacts::new(&view, None);

    assert_eq!(f.version(), TxVer::Legacy);
    assert!(!f.uses_alt());
    assert!(!f.has_unresolved());
    assert_eq!(f.num_signers(), 1);
    assert!(f.fee_payer_is(&payer));
    assert!(matches!(f.account(0), MaybeKey::Known(p) if p == payer));

    // writability: payer + dest writable; system + cb readonly
    assert!(f.writable(0));
    assert!(f.writable(1));
    assert!(!f.writable(2));
    assert!(!f.writable(3));

    // fees: only SetComputeUnitPrice -> limit defaults to 200_000 * 1 non-cb ix; price 1000
    assert_eq!(f.compute_unit_price(), 1000);
    assert_eq!(f.compute_unit_limit(), 200_000);
    assert_eq!(f.priority_fee_lamports(), 200);
    assert_eq!(f.total_fee_lamports(), 5200);

    // transfer instruction: accounts resolved + writable
    let tr = f.instruction(1).unwrap();
    assert!(matches!(tr.program_id(), MaybeKey::Known(p) if p == system));
    assert_eq!(tr.num_accounts(), 2);
    assert_eq!(tr.writable(0), Some(true));
    assert_eq!(tr.writable(1), Some(true));
}

fn v0_with_lookup(table: solana_pubkey::Pubkey) -> Vec<u8> {
    v0_bytes(
        vec![pk(1), system()],
        1,
        0,
        1, // system readonly
        vec![ci(1, vec![0], vec![])],
        vec![MessageAddressTableLookup {
            account_key: table,
            writable_indexes: vec![0],
            readonly_indexes: vec![1],
        }],
    )
}

#[test]
fn v0_alt_resolved_vs_unresolved() {
    let table = pk(42);
    let writable_acct = pk(100);
    let readonly_acct = pk(101);
    let bytes = v0_with_lookup(table);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice()).unwrap();

    // No cache -> ALT slots unresolved. Order: [payer, system] ++ [writable] ++ [readonly].
    let f = ViewFacts::new(&view, None);
    assert_eq!(f.version(), TxVer::V0);
    assert!(f.uses_alt());
    assert!(f.has_unresolved());
    assert_eq!(f.num_accounts(), 4);
    assert!(matches!(f.account(2), MaybeKey::Unresolved));
    assert!(matches!(f.account(3), MaybeKey::Unresolved));
    assert!(f.writable(2)); // writable ALT slot
    assert!(!f.writable(3)); // readonly ALT slot

    // With cache -> resolved.
    let cache = AccountLookupTableCache::new().with_table(table, vec![writable_acct, readonly_acct]);
    let f = ViewFacts::new(&view, Some(&cache));
    assert!(!f.has_unresolved());
    assert!(matches!(f.account(2), MaybeKey::Known(p) if p == writable_acct));
    assert!(matches!(f.account(3), MaybeKey::Known(p) if p == readonly_acct));

    // match_view: defers without the cache, matches with it.
    let rule = Rule::from_row(
        7,
        true,
        OnUnknown::Skip,
        1,
        Pred::WritableAccountContains(Pk(writable_acct)),
    )
    .unwrap();
    let rs = RuleSet::new(vec![rule]);
    assert_eq!(rs.match_view(&view, None), MatchResult::Deferred);
    assert_eq!(rs.match_view(&view, Some(&cache)), MatchResult::Matched(7));
}

/// A custom lock-based ALT store, like a consumer's `Arc<RwLock<HashMap<…>>>`. Proves a
/// non-cache backend works through the `AltLookup` trait without cloning the address list.
struct LockedAlts(RwLock<HashMap<Pubkey, Vec<Pubkey>>>);

impl AltLookup for LockedAlts {
    fn resolve(&self, table: &Pubkey, index: u8) -> Option<Pubkey> {
        self.0.read().unwrap().get(table).and_then(|v| v.get(index as usize).copied())
    }
}

#[test]
fn custom_alt_lookup_backend() {
    let table = pk(42);
    let writable_acct = pk(100);
    let readonly_acct = pk(101);
    let bytes = v0_with_lookup(table);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice()).unwrap();

    let mut map = HashMap::new();
    map.insert(table, vec![writable_acct, readonly_acct]);
    let alts = LockedAlts(RwLock::new(map));

    let f = ViewFacts::new(&view, Some(&alts));
    assert!(!f.has_unresolved());
    assert!(matches!(f.account(2), MaybeKey::Known(p) if p == writable_acct));

    let rule = Rule::from_row(
        9, true, OnUnknown::Skip, 1,
        Pred::WritableAccountContains(Pk(writable_acct)),
    )
    .unwrap();
    let rs = RuleSet::new(vec![rule]);
    assert_eq!(rs.match_view(&view, Some(&alts)), MatchResult::Matched(9));
}

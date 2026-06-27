//! §10 fact-builder tests (feature `build-facts`): canonical ordering + ComputeBudget fee math.
#![cfg(feature = "build-facts")]

use solana_message::compiled_instruction::CompiledInstruction;
use solana_message::{Message as LegacyMessage, MessageHeader, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;
use transaction_predicate_matcher::build::{AltResolver, FactBuilder};
use transaction_predicate_matcher::facts::MaybeKey;
use transaction_predicate_matcher::value::TxVer;

struct NoAlts;
impl AltResolver for NoAlts {
    fn addresses(&self, _: &Pubkey) -> Option<Vec<Pubkey>> {
        None
    }
}

fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}

#[test]
fn legacy_transfer_with_priority_fee() {
    use std::str::FromStr;
    let cb = Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap();
    let system = Pubkey::from_str("11111111111111111111111111111111").unwrap();
    let payer = pk(1);
    let dest = pk(2);

    // keys: [payer(signer,writable), dest(writable), system(ro), cb(ro)]
    let account_keys = vec![payer, dest, system, cb];
    let header = MessageHeader {
        num_required_signatures: 1,
        num_readonly_signed_accounts: 0,
        num_readonly_unsigned_accounts: 2,
    };

    // ComputeBudget SetComputeUnitPrice(1000)
    let mut cb_data = vec![0x03u8];
    cb_data.extend_from_slice(&1000u64.to_le_bytes());
    let cb_ix = CompiledInstruction::new_from_raw_parts(3, cb_data, vec![]);

    // System transfer of 500_000_000 lamports: tag 2 (u32 LE) + amount (u64 LE), accounts [0,1]
    let mut tr_data = vec![0x02u8, 0, 0, 0];
    tr_data.extend_from_slice(&500_000_000u64.to_le_bytes());
    let tr_ix = CompiledInstruction::new_from_raw_parts(2, tr_data, vec![0, 1]);

    let msg = LegacyMessage {
        header,
        account_keys,
        recent_blockhash: Default::default(),
        instructions: vec![cb_ix, tr_ix],
    };
    let tx = VersionedTransaction {
        signatures: vec![],
        message: VersionedMessage::Legacy(msg),
    };

    let facts = FactBuilder::build(&tx, &NoAlts);

    assert_eq!(facts.version, TxVer::Legacy);
    assert!(!facts.uses_alt);
    assert!(!facts.has_unresolved);
    assert_eq!(facts.fee_payer, payer);
    assert_eq!(facts.signers, vec![payer]);

    // writability: payer + dest writable; system + cb readonly
    assert_eq!(facts.writable, vec![true, true, false, false]);
    assert!(matches!(facts.account_keys[0], MaybeKey::Known(p) if p == payer));

    // fees: no SetComputeUnitLimit -> default 200_000 * 1 non-cb ix; price 1000 micro-lamports
    assert_eq!(facts.compute_unit_price, 1000);
    assert_eq!(facts.compute_unit_limit, 200_000);
    assert_eq!(facts.priority_fee_lamports, 200); // 1000 * 200_000 / 1_000_000
    assert_eq!(facts.total_fee_lamports, 5200); // 5000 * 1 + 200

    // instruction facts: transfer ix accounts resolved + writable
    let tr = &facts.instructions[1];
    assert!(matches!(tr.program_id, MaybeKey::Known(p) if p == system));
    assert_eq!(tr.accounts.len(), 2);
    assert_eq!(tr.writable, vec![true, true]);
}

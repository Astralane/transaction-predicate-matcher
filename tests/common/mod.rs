//! Test helpers: build serialized transactions so tests can construct a `SanitizedTransactionView`.
#![allow(dead_code)] // shared across test binaries; not every binary uses every helper

use agave_transaction_view::sanitize::SanitizeConfig;
use solana_message::compiled_instruction::CompiledInstruction;
use solana_message::v0::{Message as V0Message, MessageAddressTableLookup};
use solana_message::{Message as LegacyMessage, MessageHeader, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use std::str::FromStr;

// Current protocol values, mirroring agave-transaction-view's own tests.
pub const SANITIZE_CONFIG: SanitizeConfig = SanitizeConfig {
    min_requested_heap_size: 32 * 1024,
    max_requested_heap_size: 256 * 1024,
    max_instructions: 64,
    max_accounts_per_instruction: 255,
};

pub fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
pub fn system() -> Pubkey {
    Pubkey::from_str("11111111111111111111111111111111").unwrap()
}
pub fn compute_budget() -> Pubkey {
    Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap()
}

pub fn ci(program_id_index: u8, accounts: Vec<u8>, data: Vec<u8>) -> CompiledInstruction {
    CompiledInstruction::new_from_raw_parts(program_id_index, data, accounts)
}

/// SetComputeUnitLimit instruction data.
pub fn cb_limit(limit: u32) -> Vec<u8> {
    let mut d = vec![0x02u8];
    d.extend_from_slice(&limit.to_le_bytes());
    d
}
/// SetComputeUnitPrice instruction data.
pub fn cb_price(price: u64) -> Vec<u8> {
    let mut d = vec![0x03u8];
    d.extend_from_slice(&price.to_le_bytes());
    d
}

#[allow(clippy::too_many_arguments)]
pub fn legacy_bytes(
    account_keys: Vec<Pubkey>,
    num_required_signatures: u8,
    num_readonly_signed: u8,
    num_readonly_unsigned: u8,
    instructions: Vec<CompiledInstruction>,
) -> Vec<u8> {
    let msg = LegacyMessage {
        header: MessageHeader {
            num_required_signatures,
            num_readonly_signed_accounts: num_readonly_signed,
            num_readonly_unsigned_accounts: num_readonly_unsigned,
        },
        account_keys,
        recent_blockhash: Default::default(),
        instructions,
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); num_required_signatures as usize],
        message: VersionedMessage::Legacy(msg),
    };
    bincode::serialize(&tx).unwrap()
}

#[allow(clippy::too_many_arguments)]
pub fn v0_bytes(
    account_keys: Vec<Pubkey>,
    num_required_signatures: u8,
    num_readonly_signed: u8,
    num_readonly_unsigned: u8,
    instructions: Vec<CompiledInstruction>,
    lookups: Vec<MessageAddressTableLookup>,
) -> Vec<u8> {
    let msg = V0Message {
        header: MessageHeader {
            num_required_signatures,
            num_readonly_signed_accounts: num_readonly_signed,
            num_readonly_unsigned_accounts: num_readonly_unsigned,
        },
        account_keys,
        recent_blockhash: Default::default(),
        instructions,
        address_table_lookups: lookups,
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); num_required_signatures as usize],
        message: VersionedMessage::V0(msg),
    };
    bincode::serialize(&tx).unwrap()
}

/// A legacy tx that produces `priority_fee_lamports == fee` (limit = 1e6, price = fee).
pub fn fee_tx_bytes(fee: u64) -> Vec<u8> {
    let payer = pk(200);
    let cb = compute_budget();
    legacy_bytes(
        vec![payer, cb],
        1,
        0,
        1, // cb readonly
        vec![
            ci(1, vec![], cb_limit(1_000_000)),
            ci(1, vec![], cb_price(fee)),
        ],
    )
}

//! Optional fact-building: turn a `VersionedTransaction` + a consumer ALT resolver into `TxFacts`.
//!
//! Feature-gated behind `build-facts`. The canonical account ordering and writability rules here
//! must match the Solana runtime, or every index-based atom is wrong (spec §10.2).

pub mod compute_budget;

use crate::facts::*;
use crate::value::TxVer;
use solana_message::VersionedMessage;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

/// Consumer-supplied. Returns the full address list of a lookup table, or None if not cached.
pub trait AltResolver {
    fn addresses(&self, table: &Pubkey) -> Option<Vec<Pubkey>>;
}

/// Optional per-instruction decoder. Given the (resolved) program id and instruction data,
/// return a human name from the consumer's registry, if any.
pub trait IxDecoder {
    fn decode(&self, program_id: &MaybeKey, data: &[u8]) -> Option<String>;
}

impl IxDecoder for () {
    fn decode(&self, _: &MaybeKey, _: &[u8]) -> Option<String> {
        None
    }
}

pub struct FactBuilder;

impl FactBuilder {
    /// Build `TxFacts` from a wire transaction with no instruction decoding.
    pub fn build(tx: &VersionedTransaction, resolver: &impl AltResolver) -> TxFacts {
        Self::build_with_decoder(tx, resolver, &())
    }

    pub fn build_with_decoder(
        tx: &VersionedTransaction,
        resolver: &impl AltResolver,
        decoder: &impl IxDecoder,
    ) -> TxFacts {
        let msg = &tx.message;
        let header = msg.header();
        let nrs = header.num_required_signatures as usize;
        let nrss = header.num_readonly_signed_accounts as usize;
        let nru = header.num_readonly_unsigned_accounts as usize;

        let static_keys = msg.static_account_keys();
        let total_static = static_keys.len();

        let version = match msg {
            VersionedMessage::Legacy(_) => TxVer::Legacy,
            VersionedMessage::V0(_) => TxVer::V0,
        };

        // --- full ordered account list + writability + signer flags ---
        let mut keys: Vec<MaybeKey> = Vec::new();
        let mut writable: Vec<bool> = Vec::new();
        let mut signer: Vec<bool> = Vec::new();
        let mut has_unresolved = false;

        // static keys
        let writable_static = |i: usize| -> bool {
            if i < nrs {
                i < nrs - nrss // writable signers
            } else {
                i < total_static - nru // writable non-signers
            }
        };
        for (i, k) in static_keys.iter().enumerate() {
            keys.push(MaybeKey::Known(*k));
            writable.push(writable_static(i));
            signer.push(i < nrs);
        }

        // ALT-loaded keys: all writable (across tables, in lookup order), then all readonly.
        let lookups = msg.address_table_lookups().unwrap_or(&[]);
        let push_alt = |table: &Pubkey, indexes: &[u8], wr: bool, out_keys: &mut Vec<MaybeKey>,
                            out_wr: &mut Vec<bool>, out_sg: &mut Vec<bool>, unres: &mut bool| {
            match resolver.addresses(table) {
                Some(addrs) => {
                    for &idx in indexes {
                        match addrs.get(idx as usize) {
                            Some(p) => out_keys.push(MaybeKey::Known(*p)),
                            None => {
                                out_keys.push(MaybeKey::Unresolved);
                                *unres = true;
                            }
                        }
                        out_wr.push(wr);
                        out_sg.push(false);
                    }
                }
                None => {
                    for _ in indexes {
                        out_keys.push(MaybeKey::Unresolved);
                        out_wr.push(wr);
                        out_sg.push(false);
                        *unres = true;
                    }
                }
            }
        };
        for l in lookups {
            push_alt(&l.account_key, &l.writable_indexes, true, &mut keys, &mut writable,
                     &mut signer, &mut has_unresolved);
        }
        for l in lookups {
            push_alt(&l.account_key, &l.readonly_indexes, false, &mut keys, &mut writable,
                     &mut signer, &mut has_unresolved);
        }

        let at = |i: usize| -> MaybeKey { keys.get(i).copied().unwrap_or(MaybeKey::Unresolved) };
        let wr_at = |i: usize| -> bool { writable.get(i).copied().unwrap_or(false) };
        let sg_at = |i: usize| -> bool { signer.get(i).copied().unwrap_or(false) };

        // --- per-instruction facts ---
        let cb = compute_budget::compute_budget_program_id();
        let mut instructions = Vec::with_capacity(msg.instructions().len());
        let mut non_cb_ix_count = 0u64;
        for ci in msg.instructions() {
            let program_id = at(ci.program_id_index as usize);
            if !matches!(program_id, MaybeKey::Known(p) if p == cb) {
                non_cb_ix_count += 1;
            }
            let accounts: Vec<MaybeKey> = ci.accounts.iter().map(|&i| at(i as usize)).collect();
            let ix_wr: Vec<bool> = ci.accounts.iter().map(|&i| wr_at(i as usize)).collect();
            let ix_sg: Vec<bool> = ci.accounts.iter().map(|&i| sg_at(i as usize)).collect();
            let decoded_name = decoder.decode(&program_id, &ci.data);
            instructions.push(IxFacts {
                program_id,
                accounts,
                writable: ix_wr,
                signer: ix_sg,
                data: ci.data.clone(),
                decoded_name,
            });
        }

        // --- derived fee facts ---
        let scan = compute_budget::scan(
            instructions.iter().map(|i| (&i.program_id, i.data.as_slice())),
        );
        let f = compute_budget::fees(scan, nrs as u64, non_cb_ix_count);

        let signers: Vec<Pubkey> = static_keys.iter().take(nrs).copied().collect();
        let fee_payer = static_keys.first().copied().unwrap_or_default();

        TxFacts {
            version,
            uses_alt: !lookups.is_empty(),
            signers,
            fee_payer,
            account_keys: keys,
            writable,
            has_unresolved,
            instructions,
            compute_unit_price: f.compute_unit_price,
            compute_unit_limit: f.compute_unit_limit,
            priority_fee_lamports: f.priority_fee_lamports,
            total_fee_lamports: f.total_fee_lamports,
        }
    }
}

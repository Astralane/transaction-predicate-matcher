//! The evaluation input contract. Produced by the consumer (or the optional builder in `build/`).

use crate::value::TxVer;
use solana_pubkey::Pubkey;

/// A resolved account key, or a marker that it sits behind an unresolved ALT.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaybeKey {
    Known(Pubkey),
    Unresolved,
}

#[derive(Clone, Debug)]
pub struct TxFacts {
    pub version: TxVer,
    pub uses_alt: bool,
    /// First `num_required_signatures` keys. Signers are ALWAYS Known (never from ALT).
    pub signers: Vec<Pubkey>,
    /// `account_keys[0]`; always Known.
    pub fee_payer: Pubkey,
    /// FULL account list in Solana canonical order:
    /// `[ static keys ] ++ [ ALT writable ] ++ [ ALT readonly ]`.
    pub account_keys: Vec<MaybeKey>,
    /// Parallel to `account_keys`: is this slot writable? Known for ALL slots (incl. Unresolved),
    /// because writability is structural (header + which lookup list the entry came from).
    pub writable: Vec<bool>,
    /// True iff at least one entry in `account_keys` is Unresolved. Cached for fast index decisions.
    pub has_unresolved: bool,
    pub instructions: Vec<IxFacts>,

    // derived numeric facts. All concrete u64; absence encoded as 0 where noted.
    pub compute_unit_price: u64,
    pub compute_unit_limit: u64,
    pub priority_fee_lamports: u64,
    pub total_fee_lamports: u64,
}

#[derive(Clone, Debug)]
pub struct IxFacts {
    pub program_id: MaybeKey,
    pub accounts: Vec<MaybeKey>,
    pub writable: Vec<bool>,
    pub signer: Vec<bool>,
    pub data: Vec<u8>,
    pub decoded_name: Option<String>,
}

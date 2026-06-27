//! ComputeBudget instruction decoding + fee math.

use crate::facts::MaybeKey;
use solana_pubkey::Pubkey;
use std::str::FromStr;

/// `ComputeBudget111111111111111111111111111111`.
pub fn compute_budget_program_id() -> Pubkey {
    Pubkey::from_str("ComputeBudget111111111111111111111111111111").unwrap()
}

/// Runtime default per non-ComputeBudget instruction, and the global cap.
pub const DEFAULT_CU_PER_IX: u64 = 200_000;
pub const MAX_CU_LIMIT: u64 = 1_400_000;
pub const BASE_FEE_PER_SIG: u64 = 5000;

#[derive(Default, Clone, Copy, Debug)]
pub struct ComputeBudgetScan {
    pub price_micro_lamports: Option<u64>,
    pub unit_limit: Option<u32>,
}

/// Scan top-level instructions for ComputeBudget SetComputeUnitLimit/Price.
/// `ixs` yields `(program_id, data)` per top-level instruction.
pub fn scan<'a>(ixs: impl Iterator<Item = (&'a MaybeKey, &'a [u8])>) -> ComputeBudgetScan {
    let cb = compute_budget_program_id();
    let mut out = ComputeBudgetScan::default();
    for (prog, data) in ixs {
        if !matches!(prog, MaybeKey::Known(p) if *p == cb) {
            continue;
        }
        match data.first() {
            // SetComputeUnitLimit(u32 LE)
            Some(0x02) => {
                if let Some(b) = data.get(1..5) {
                    out.unit_limit = Some(u32::from_le_bytes(b.try_into().unwrap()));
                }
            }
            // SetComputeUnitPrice(u64 LE)
            Some(0x03) => {
                if let Some(b) = data.get(1..9) {
                    out.price_micro_lamports = Some(u64::from_le_bytes(b.try_into().unwrap()));
                }
            }
            // 0x00 RequestUnits (legacy), 0x01 RequestHeapFrame -> ignore for fees
            _ => {}
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct Fees {
    pub compute_unit_price: u64,
    pub compute_unit_limit: u64,
    pub priority_fee_lamports: u64,
    pub total_fee_lamports: u64,
}

/// Resolve fee facts. `non_cb_ix_count` is the number of top-level instructions whose program is
/// NOT the ComputeBudget program (used for the default CU limit when none is set explicitly).
pub fn fees(scan: ComputeBudgetScan, num_required_signatures: u64, non_cb_ix_count: u64) -> Fees {
    let price = scan.price_micro_lamports.unwrap_or(0);
    let limit = scan.unit_limit.map(u64::from).unwrap_or_else(|| {
        (DEFAULT_CU_PER_IX.saturating_mul(non_cb_ix_count)).min(MAX_CU_LIMIT)
    });
    let priority = ((price as u128) * (limit as u128) / 1_000_000) as u64;
    let base = BASE_FEE_PER_SIG.saturating_mul(num_required_signatures);
    Fees {
        compute_unit_price: price,
        compute_unit_limit: limit,
        priority_fee_lamports: priority,
        total_fee_lamports: base.saturating_add(priority),
    }
}

//! The evaluation input: a borrowing, zero-allocation view over a `SanitizedTransactionView`
//! plus an optional Address-Lookup-Table cache.
//!
//! Account keys, writability, and fees are computed on the fly from the view rather than
//! materialized into owned `Vec`s, so matching a transaction allocates nothing here.

use crate::budget::{self, Fees};
use crate::value::TxVer;
use agave_transaction_view::transaction_data::TransactionData;
use agave_transaction_view::transaction_version::TransactionVersion;
use agave_transaction_view::transaction_view::SanitizedTransactionView;
use rustc_hash::FxHashMap;
use solana_pubkey::Pubkey;

/// A resolved account key, or a marker that it sits behind an unresolved ALT.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MaybeKey {
    Known(Pubkey),
    Unresolved,
}

/// Consumer-supplied resolver for Address Lookup Tables. Implement it over whatever store you
/// already keep ALT addresses in — a `HashMap`, `DashMap`, `Arc<RwLock<HashMap<…>>>`, etc. — and
/// pass it by reference. The matcher only ever needs one address at a time, so `resolve` returns a
/// single (`Copy`) `Pubkey`: an impl can take and drop a lock guard within the call and copy the
/// address out, without cloning the table's address list.
///
/// Return `None` when the table isn't available or the index is out of range; any unresolved ALT
/// slot makes the matcher defer (or stay `Unknown`).
///
/// ```ignore
/// impl AltLookup for dashmap::DashMap<Pubkey, Vec<Pubkey>> {
///     fn resolve(&self, table: &Pubkey, index: u8) -> Option<Pubkey> {
///         self.get(table).and_then(|v| v.get(index as usize).copied())
///     }
/// }
/// ```
pub trait AltLookup {
    /// The address at `index` within lookup table `table`, or `None` if unavailable.
    fn resolve(&self, table: &Pubkey, index: u8) -> Option<Pubkey>;
}

/// A simple in-memory [`AltLookup`] backed by a `HashMap`, for consumers that don't already have a
/// store. `table pubkey -> full address list`.
#[derive(Clone, Debug, Default)]
pub struct AccountLookupTableCache {
    tables: FxHashMap<Pubkey, Vec<Pubkey>>,
}

impl AccountLookupTableCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_table(mut self, table: Pubkey, addresses: Vec<Pubkey>) -> Self {
        self.tables.insert(table, addresses);
        self
    }

    pub fn insert(&mut self, table: Pubkey, addresses: Vec<Pubkey>) -> Option<Vec<Pubkey>> {
        self.tables.insert(table, addresses)
    }

    pub fn len(&self) -> usize {
        self.tables.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

impl AltLookup for AccountLookupTableCache {
    fn resolve(&self, table: &Pubkey, index: u8) -> Option<Pubkey> {
        self.tables.get(table).and_then(|v| v.get(index as usize).copied())
    }
}

impl FromIterator<(Pubkey, Vec<Pubkey>)> for AccountLookupTableCache {
    fn from_iter<I: IntoIterator<Item = (Pubkey, Vec<Pubkey>)>>(it: I) -> Self {
        Self {
            tables: it.into_iter().collect(),
        }
    }
}

/// Borrowing facts over a sanitized transaction view + optional ALT cache. Cheap to construct
/// (precomputes only scalars); all account/key lookups are computed on demand.
pub struct ViewFacts<'a, D: TransactionData> {
    view: &'a SanitizedTransactionView<D>,
    alt: Option<&'a dyn AltLookup>,
    nrs: usize,
    nrss: usize,
    nru: usize,
    total_static: usize,
    n_writable_alt: usize,
    n_readonly_alt: usize,
    has_unresolved: bool,
    uses_alt: bool,
    fees: Fees,
}

impl<'a, D: TransactionData> ViewFacts<'a, D> {
    pub fn new(view: &'a SanitizedTransactionView<D>, alt: Option<&'a dyn AltLookup>) -> Self {
        let nrs = view.num_required_signatures() as usize;
        let nrss = view.num_readonly_signed_static_accounts() as usize;
        let nru = view.num_readonly_unsigned_static_accounts() as usize;
        let total_static = view.static_account_keys().len();
        let n_writable_alt = view.total_writable_lookup_accounts() as usize;
        let n_readonly_alt = view.total_readonly_lookup_accounts() as usize;
        let uses_alt = view.num_address_table_lookups() > 0;

        // has_unresolved: any ALT index that doesn't resolve (uncached table or out-of-range).
        let has_unresolved = view.address_table_lookup_iter().any(|l| {
            l.writable_indexes
                .iter()
                .chain(l.readonly_indexes)
                .any(|&i| alt.and_then(|c| c.resolve(l.account_key, i)).is_none())
        });

        // fees: scan ComputeBudget instructions once, count non-CB instructions.
        let cb = budget::compute_budget_program_id();
        let mut scan = budget::ComputeBudgetScan::default();
        let mut non_cb = 0u64;
        for ix in view.instructions_iter() {
            let prog = resolve_index(view, alt, total_static, n_writable_alt, ix.program_id_index as usize);
            if matches!(prog, MaybeKey::Known(p) if p == cb) {
                scan.apply(ix.data);
            } else {
                non_cb += 1;
            }
        }
        let fees = budget::fees(scan, nrs as u64, non_cb);

        Self {
            view,
            alt,
            nrs,
            nrss,
            nru,
            total_static,
            n_writable_alt,
            n_readonly_alt,
            has_unresolved,
            uses_alt,
            fees,
        }
    }

    pub fn version(&self) -> TxVer {
        match self.view.version() {
            TransactionVersion::Legacy => TxVer::Legacy,
            TransactionVersion::V0 => TxVer::V0,
            TransactionVersion::V1 => TxVer::V1,
        }
    }

    pub fn uses_alt(&self) -> bool {
        self.uses_alt
    }

    pub fn has_unresolved(&self) -> bool {
        self.has_unresolved
    }

    pub fn num_signers(&self) -> usize {
        self.nrs
    }

    pub fn signer_contains(&self, pk: &Pubkey) -> bool {
        self.view.static_account_keys()[..self.nrs].contains(pk)
    }

    pub fn fee_payer_is(&self, pk: &Pubkey) -> bool {
        self.view.static_account_keys().first() == Some(pk)
    }

    pub fn num_accounts(&self) -> usize {
        self.total_static + self.n_writable_alt + self.n_readonly_alt
    }

    pub fn account(&self, i: usize) -> MaybeKey {
        resolve_index(self.view, self.alt, self.total_static, self.n_writable_alt, i)
    }

    /// Structural writability of slot `i` (known even when the key itself is unresolved).
    pub fn writable(&self, i: usize) -> bool {
        if i < self.total_static {
            if i < self.nrs {
                i < self.nrs - self.nrss
            } else {
                i < self.total_static - self.nru
            }
        } else {
            i - self.total_static < self.n_writable_alt
        }
    }

    pub fn signer(&self, i: usize) -> bool {
        i < self.nrs
    }

    pub fn num_instructions(&self) -> usize {
        self.view.num_instructions() as usize
    }

    pub fn instruction(&self, idx: usize) -> Option<IxView<'a, '_, D>> {
        self.view.instructions_iter().nth(idx).map(|ix| IxView {
            facts: self,
            program_id_index: ix.program_id_index,
            accounts: ix.accounts,
            data: ix.data,
        })
    }

    pub fn compute_unit_price(&self) -> u64 {
        self.fees.compute_unit_price
    }
    pub fn compute_unit_limit(&self) -> u64 {
        self.fees.compute_unit_limit
    }
    pub fn priority_fee_lamports(&self) -> u64 {
        self.fees.priority_fee_lamports
    }
    pub fn total_fee_lamports(&self) -> u64 {
        self.fees.total_fee_lamports
    }
}

/// A single instruction's view: its raw data plus account resolution through the parent facts.
pub struct IxView<'a, 'f, D: TransactionData> {
    facts: &'f ViewFacts<'a, D>,
    program_id_index: u8,
    accounts: &'f [u8],
    data: &'f [u8],
}

impl<D: TransactionData> IxView<'_, '_, D> {
    pub fn program_id(&self) -> MaybeKey {
        self.facts.account(self.program_id_index as usize)
    }
    pub fn data(&self) -> &[u8] {
        self.data
    }
    pub fn num_accounts(&self) -> usize {
        self.accounts.len()
    }
    /// Account at this instruction's local index `k`, or `None` if out of range.
    pub fn account(&self, k: usize) -> Option<MaybeKey> {
        self.accounts.get(k).map(|&g| self.facts.account(g as usize))
    }
    pub fn writable(&self, k: usize) -> Option<bool> {
        self.accounts.get(k).map(|&g| self.facts.writable(g as usize))
    }
    pub fn signer(&self, k: usize) -> Option<bool> {
        self.accounts.get(k).map(|&g| self.facts.signer(g as usize))
    }
    /// No decoder registry in the view path; always `None`.
    pub fn decoded_name(&self) -> Option<&str> {
        None
    }
}

fn resolve_lookup(alt: Option<&dyn AltLookup>, table: &Pubkey, idx: u8) -> MaybeKey {
    match alt.and_then(|c| c.resolve(table, idx)) {
        Some(p) => MaybeKey::Known(p),
        None => MaybeKey::Unresolved,
    }
}

/// Resolve a global account index into a `MaybeKey`, in Solana canonical order:
/// `[ static keys ] ++ [ ALT writable ] ++ [ ALT readonly ]`.
fn resolve_index<D: TransactionData>(
    view: &SanitizedTransactionView<D>,
    alt: Option<&dyn AltLookup>,
    total_static: usize,
    n_writable_alt: usize,
    i: usize,
) -> MaybeKey {
    if i < total_static {
        return MaybeKey::Known(view.static_account_keys()[i]);
    }
    let off = i - total_static;
    if off < n_writable_alt {
        let mut acc = 0usize;
        for l in view.address_table_lookup_iter() {
            let len = l.writable_indexes.len();
            if off < acc + len {
                return resolve_lookup(alt, l.account_key, l.writable_indexes[off - acc]);
            }
            acc += len;
        }
    } else {
        let mut off = off - n_writable_alt;
        let mut acc = 0usize;
        for l in view.address_table_lookup_iter() {
            let len = l.readonly_indexes.len();
            if off < acc + len {
                return resolve_lookup(alt, l.account_key, l.readonly_indexes[off - acc]);
            }
            acc += len;
        }
        let _ = &mut off;
    }
    MaybeKey::Unresolved
}

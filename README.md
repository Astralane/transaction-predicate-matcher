# transaction-predicate-matcher

Match rules against Solana transactions.

A rule is a predicate plus a little metadata (id, name, enabled). It describes what to match and
nothing else, order is priority (the rule's position in the list).
When it matches you get its `id` back and map that to an action on your side.

Solana transactions often reference accounts behind Address Lookup Tables you haven't resolved
yet. Rather than guess and risk a wrong `false`, the matcher answers `True`, `False`, or `Unknown`,
and `Unknown` carries through the logic. If it can't tell, it says so instead of routing on a
guess.

## Atoms

Predicates are built from small atoms: signer and fee-payer checks, account membership,
instruction shape (program id, discriminator, data slices), fees. There's no `transfer_to_X` atom;
compose one from `program_id_is` + `discriminator` + `account_at`. Atoms come in two scopes,
whole-transaction and single-instruction, so "program X and instruction Y" always means the same
instruction.

First-match routing (what a "switch" used to do) is just ordering: list the rules from most to
least specific and the matcher returns the first that matches.

## Quick look

A rule that flags high-priority-fee transactions:

```json
{ "priority_fee_lamports": { "op": "ge", "n": 1000000 } }
```

Loading and running it:

```rust
use transaction_predicate_matcher::{AccountLookupTableCache, RuleSet, MatchResult, OnUnknown, Rule};

let rule = Rule::load_from_json(
    7,                // id (you map this id -> action)
    true,             // enabled
    OnUnknown::Skip,  // what to do on an Unknown result
    1,                // schema_version
    &predicate_json,
)?;

// Order = priority. The rule at index 0 is tried first.
let rules = RuleSet::new(vec![rule]);

// `view` is a SanitizedTransactionView; `cache` resolves any Address Lookup Tables it uses.
let cache = AccountLookupTableCache::new().with_table(table_pubkey, addresses);
match rules.match_view(&view, Some(&cache)) {
    MatchResult::Matched(id) => { /* rule `id` matched; look up its action */ }
    MatchResult::NoMatch     => { /* nothing matched */ }
    MatchResult::Deferred    => { /* a table wasn't in the cache, resolve and retry */ }
}
```

The matcher evaluates straight off the view — no per-transaction allocation. Pass `None` (or a
cache missing some tables) and any ALT-loaded accounts stay unresolved, so `match_view` returns
`Deferred` rather than guess.

`AccountLookupTableCache` is just a convenience. To resolve tables out of a store you already keep
(a `DashMap`, an `Arc<RwLock<HashMap<…>>>`, …) without cloning anything, implement `AltLookup` on
it and pass `Some(&store)`:

```rust
use transaction_predicate_matcher::AltLookup;
use solana_pubkey::Pubkey;

impl AltLookup for MyAltStore {
    fn resolve(&self, table: &Pubkey, index: u8) -> Option<Pubkey> {
        // take a read guard, copy out one 32-byte pubkey, drop the guard
        self.read().get(table).and_then(|addrs| addrs.get(index as usize).copied())
    }
}
```

`resolve` hands back a single `Copy` `Pubkey`, so the lock guard never has to outlive the call and
the address list is never cloned.

## Example rules

Each block is the `predicate` for one rule. Full schema in [`docs/schema_v1.md`](docs/schema_v1.md);
all four parse and validate in `tests/example_rules.rs`.

Durable-nonce transaction (its first instruction is System `AdvanceNonceAccount`, tag `04000000`):

```json
{ "instruction_at": { "index": 0, "pred": { "and": [
  { "program_id_is": "11111111111111111111111111111111" },
  { "discriminator": { "offset": 0, "bytes": "04000000" } }
] } } }
```

Raydium AMM v4 swap (`swapBaseIn` tag `09` or `swapBaseOut` tag `0b`):

```json
{ "any_instruction": { "and": [
  { "program_id_is": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8" },
  { "or": [
    { "discriminator": { "offset": 0, "bytes": "09" } },
    { "discriminator": { "offset": 0, "bytes": "0b" } }
  ] }
] } }
```

Pump.fun mint (`create`, Anchor discriminator `181ec828051c0777`):

```json
{ "any_instruction": { "and": [
  { "program_id_is": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P" },
  { "discriminator": { "offset": 0, "bytes": "181ec828051c0777" } }
] } }
```

Transfer of 0.001 SOL to an Astralane tip account (System `Transfer`, destination at index 1,
1,000,000 lamports at offset 4):

```json
{ "any_instruction": { "and": [
  { "program_id_is": "11111111111111111111111111111111" },
  { "discriminator": { "offset": 0, "bytes": "02000000" } },
  { "account_at": { "index": 1, "pk": "<ASTRALANE_TIP_ACCOUNT>" } },
  { "data_slice": { "offset": 4, "kind": "u64_le", "op": "eq", "value": 1000000 } }
] } }
```

Fill in `<ASTRALANE_TIP_ACCOUNT>` with their published tip account. There's usually more than one,
so `or` over an `account_at` per address, and use `op: "ge"` for a minimum tip instead of an exact
amount. Only top-level transfers are visible before a transaction lands; CPI transfers aren't.

## Good to know

- An inverted index narrows candidates so you don't evaluate every rule on every transaction.
- If a transaction has unresolved lookup tables, `match_view` returns `Deferred`. Resolve and retry,
  or use `match_view_scan` to evaluate anyway.
- Bad rules are rejected at load (bad base58/hex, type mismatches, unknown atoms), not silently
  treated as `false`. `load_rules` skips the bad ones and keeps the rest.
- Rule sets hot-reload at runtime.

## Versioning

The matcher has a schema version (`ENGINE_SCHEMA_VERSION`). When you add a new atom, bump it and tag
rules that use it. An older build skips a too-new rule instead of choking on a keyword it doesn't
know, so new rules don't break deployed matchers. Rule types are `#[non_exhaustive]`, so adding
atoms stays backwards-compatible for code that depends on this crate.

## Not included

Networking, RPC, fetching lookup tables, storing rules, mapping ids to actions, anything spanning
multiple transactions ("5 txs in 100ms"), or ML. Feed those in as facts and map the matched id to
an action yourself.

## Build & test

```bash
cargo test
```

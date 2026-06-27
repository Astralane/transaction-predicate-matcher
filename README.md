# transaction-predicate-matcher

Match rules against Solana transactions.

You write rules as JSON. The matcher loads them once, then tells you which rules a transaction
matches by returning their ids. What to do with a match is up to you.

Pending transactions often reference accounts behind Address Lookup Tables you haven't resolved
yet. Rather than guess and risk a wrong `false`, the matcher answers `True`, `False`, or `Unknown`,
and `Unknown` carries through the logic. If it can't tell, it says so instead of routing on a
guess.

## Rules are matching logic, not actions

A rule is a predicate plus some metadata (id, priority, enabled). It describes what to match and
nothing else. When it matches you get its id back, and you keep the id-to-action mapping on your
side. Change what a match does without touching the rules.

Predicates are built from small atoms: signer and fee-payer checks, account membership,
instruction shape (program id, discriminator, data slices), fees. There's no `transfer_to_X` atom;
compose one from `program_id_is` + `discriminator` + `account_at`. Atoms come in two scopes,
whole-transaction and single-instruction, so "program X and instruction Y" always means the same
instruction.

For first-match routing (what a "switch" used to do), give rules descending `priority` and set
`stop_after_match`. The first match wins and evaluation stops.

## Quick look

A rule that flags high-priority-fee transactions:

```json
{ "priority_fee_lamports": { "op": "ge", "n": 1000000 } }
```

Loading and running it:

```rust
use transaction_predicate_matcher::{Engine, MatchResult, Rule, OnUnknown};

let rule = Rule::load_from_json(
    7,                       // id  (you map this id -> action)
    "high-priority".into(),  // name
    true,                    // enabled
    0,                       // priority
    OnUnknown::Skip,         // what to do on an Unknown result
    false,                   // stop_after_match
    1,                       // schema_version
    &predicate_json,
)?;

let engine = Engine::new(vec![rule]);

match engine.match_tx(&facts) {
    MatchResult::Matched(ids) => for id in ids { /* look up the action for `id` */ },
    MatchResult::Deferred     => { /* lookup tables unresolved, resolve and retry */ }
}
```

`facts` is a `TxFacts`. Build it yourself, or turn on the `build-facts` feature and let
`FactBuilder` build one from a `VersionedTransaction` plus your ALT cache.

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
- If a transaction has unresolved lookup tables, `match_tx` returns `Deferred`. Resolve and retry,
  or use `match_tx_scan` to evaluate anyway.
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
multiple transactions ("5 txs in 100ms"), or ML. Feed those in as facts and map matched ids to
actions yourself.

## Build & test

```bash
cargo test                        # core
cargo test --features build-facts # core + transaction -> facts builder
```

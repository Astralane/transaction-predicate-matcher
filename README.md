# transaction-predicate-matcher

Match rules against **pending** (not-yet-landed) Solana transactions.

You write rules as JSON, the engine loads and checks them once, then tells you which rules a
transaction matches. It's a pure function: no network, no clocks, no randomness. You hand it the
transaction facts, it hands you the **ids of the rules that matched** — what to *do* about each
match is yours to decide.

The interesting bit: a pending transaction may reference accounts hidden behind Address Lookup
Tables you haven't resolved yet. Instead of guessing (and risking a wrong `false`), the engine
reasons in `True` / `False` / **`Unknown`**, and `Unknown` propagates honestly. So you never route
real money on a maybe.

## Rules describe matching, not outcomes

A rule is just a **predicate** with some metadata (id, priority, enabled, …). It says *what to
match* and nothing about *what happens next*. When it matches, the engine reports its id; you keep
the id → action mapping on your side. This keeps actions out of the rule data, so you can change
what a match *does* without touching (or migrating) the rules themselves.

Predicates are built from small, composable atoms — signer/fee-payer checks, account membership,
instruction shape (`program_id`, discriminator, data slices), fees, and so on. There's no
`transfer_to_X` atom; you compose one from `program_id_is` + `discriminator` + `account_at`. Atoms
come in two scopes — whole-transaction and single-instruction — so "program X **and** instruction
Y" always means *the same* instruction, never an accidental match across two.

Need first-match-exclusive routing (the old "switch")? Give the rules descending `priority` and set
`stop_after_match` — the first match wins and evaluation stops.

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
    MatchResult::Deferred     => { /* lookup tables unresolved — resolve and retry */ }
}
```

`facts` is a `TxFacts`. You can build it yourself, or enable the `build-facts` feature and let
`FactBuilder` turn a `VersionedTransaction` + your ALT cache into one.

## Example rules

Each block below is the `predicate` JSONB for one rule. The full schema is in
[`docs/schema_v1.md`](docs/schema_v1.md). All four parse and validate in
`tests/example_rules.rs`.

**Durable-nonce transaction** — a tx uses a durable nonce iff its **first** instruction is System
`AdvanceNonceAccount` (instruction index `4`, u32-LE → `04000000`):

```json
{ "instruction_at": { "index": 0, "pred": { "and": [
  { "program_id_is": "11111111111111111111111111111111" },
  { "discriminator": { "offset": 0, "bytes": "04000000" } }
] } } }
```

**Raydium AMM v4 swap** — program `675kPX9…`, single-byte tag `09` (`swapBaseIn`) or `0b`
(`swapBaseOut`):

```json
{ "any_instruction": { "and": [
  { "program_id_is": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8" },
  { "or": [
    { "discriminator": { "offset": 0, "bytes": "09" } },
    { "discriminator": { "offset": 0, "bytes": "0b" } }
  ] }
] } }
```

**Pump.fun mint** — program `6EF8rre…`, Anchor 8-byte discriminator for `create`
(`181ec828051c0777`):

```json
{ "any_instruction": { "and": [
  { "program_id_is": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P" },
  { "discriminator": { "offset": 0, "bytes": "181ec828051c0777" } }
] } }
```

**Transfer of exactly 0.001 SOL to an Astralane tip account** — System `Transfer` (tag `02000000`),
destination at account index `1`, amount `1_000_000` lamports (= 0.001 SOL) at data offset `4`:

```json
{ "any_instruction": { "and": [
  { "program_id_is": "11111111111111111111111111111111" },
  { "discriminator": { "offset": 0, "bytes": "02000000" } },
  { "account_at": { "index": 1, "pk": "<ASTRALANE_TIP_ACCOUNT>" } },
  { "data_slice": { "offset": 4, "kind": "u64_le", "op": "eq", "value": 1000000 } }
] } }
```

> Substitute `<ASTRALANE_TIP_ACCOUNT>` with Astralane's published tip account (a base58 pubkey).
> Providers usually expose several rotating tip accounts — `or` over an `account_at` per address,
> and switch `op` to `ge` if you want to match a *minimum* tip rather than an exact amount. Only
> **top-level** transfers are visible pre-land; CPI transfers are not.

## A few things worth knowing

- **Fast matching.** An inverted index keeps lookups sub-linear in the rule count instead of
  scanning every rule on every transaction.
- **Unresolved lookup tables.** When tables are missing, `match_tx` returns `Deferred` rather than
  risk a wrong answer. Resolve the tables and re-submit. (`match_tx_scan` evaluates everything
  anyway if you'd rather degrade than defer.)
- **Bad rules fail loud, at load.** Malformed base58/hex, type mismatches, unknown atoms — all
  rejected when you load them, never silently treated as `false`. One bad rule is skipped via
  `load_rules`; the rest load fine.
- **Hot reload.** Swap the rule set at runtime without downtime.

## Adding new keywords later (versioning)

The engine carries a schema version (`ENGINE_SCHEMA_VERSION`). When you add a new atom, bump it and
tag rules that use the new feature with that version. An older engine that sees a too-new rule
**skips it cleanly** instead of choking on a keyword it doesn't recognize — so rolling out new
rules never breaks already-deployed engines. The public rule types are also `#[non_exhaustive]`,
so adding atoms is a backwards-compatible change for Rust code depending on this crate.

## Not included (by design)

Networking, RPC, ALT fetching, storing rules, mapping ids to actions, and anything that spans
multiple transactions ("5 txs in 100ms") or needs an ML model. Those live upstream or downstream;
pass results in as facts, map matched ids to actions on your side.

## Build & test

```bash
cargo test                        # core
cargo test --features build-facts # core + transaction → facts builder
```

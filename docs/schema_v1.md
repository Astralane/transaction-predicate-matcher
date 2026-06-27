# Rule predicate schema — v1

This document defines the JSON(B) schema for a rule's `predicate` in
`transaction-predicate-matcher`, schema version **1** (`ENGINE_SCHEMA_VERSION = 1`).

A rule is **matching logic only** — it carries no decision/action. When the predicate matches a
transaction, the engine reports the rule's `id`; the consumer maps that id to an action.

## Storage model

A rule is a row; only the `predicate` column is the JSON described here. Everything else is rule
metadata read independently of (and before) parsing the predicate:

| Column | Type | Notes |
|---|---|---|
| `id` | `i64` | reported on match; you map it → action |
| `name` | `text` | human label |
| `enabled` | `bool` | disabled rules are not compiled |
| `priority` | `i32` | evaluation order, **descending**; ties broken by `id` ascending |
| `on_unknown` | `text` | `skip` (default) \| `fail_closed` \| `treat_true` — see [Tri-state](#tri-state-and-on_unknown) |
| `stop_after_match` | `bool` | if this rule matches, stop evaluating lower-priority rules (first-match-exclusive routing) |
| `schema_version` | `u32` | the schema version this predicate targets; rules with `schema_version > ENGINE_SCHEMA_VERSION` are skipped at load |
| `predicate` | `jsonb` | a [`Pred`](#transaction-scope--pred) value |

Loading is **fail-closed**: a malformed predicate, a value/kind mismatch, an unknown atom, or a
`schema_version` newer than the engine supports rejects *that* rule (logged & skipped). It never
degrades to `false` and never poisons the rest of the set.

## Encoding conventions

Predicates are a Rust enum serialized **externally tagged** with `snake_case` variant names, so
each node is a single-key JSON object whose key is the atom name:

- **Newtype atoms** take the value directly: `{ "signer_contains": "<base58>" }`.
- **Struct atoms** take an object: `{ "num_signers": { "op": "ge", "n": 2 } }`.
- **List atoms** take an array: `{ "and": [ <Pred>, … ] }`.

Struct atoms use `deny_unknown_fields` — an unexpected key is a load error.

### Scalar types

| Type | JSON | Meaning |
|---|---|---|
| `Pk` | base58 string | a 32-byte pubkey; parsed at load (bad base58 → error) |
| `Bytes` | hex string (lowercase, no `0x`) | raw bytes; any length unless noted (e.g. discriminator ≥ 1) |
| `Cmp` | `"eq"`,`"ne"`,`"lt"`,`"le"`,`"gt"`,`"ge"` | integer comparison operator |
| `TxVer` | `"legacy"`, `"v0"` | transaction version |
| `SliceKind` | see [data_slice](#data_slice) | how to read bytes at an offset |
| `SliceVal` | number **or** hex string | compare target for `data_slice` |
| index / count / offset / `n` | non-negative integer | `usize`/`u64` as noted |

Numeric comparisons are evaluated in `i128`, so any `u64`/`i64` value compares correctly.

## Transaction scope — `Pred`

The predicate at the root of a rule is a `Pred`. Atoms marked **tri** can return `Unknown` under
partial ALT resolution; all others are strictly two-valued.

### Logic

```json
{ "and": [ <Pred>, … ] }      // non-empty; Unknown-aware AND
{ "or":  [ <Pred>, … ] }      // non-empty; Unknown-aware OR
{ "not": <Pred> }
{ "const": true }             // or false
```

`and`/`or` operand lists **must be non-empty** (an empty list is a load error).

### Quantifiers (the only bridge into instruction scope)

```json
{ "any_instruction": <IxPred> }                       // ∃ instruction matching IxPred
{ "all_instructions": <IxPred> }                      // ∀ instructions
{ "instruction_at": { "index": 0, "pred": <IxPred> } }// the instruction at this index (out of range → false)
{ "instruction_count": { "op": "ge", "n": 2 } }
```

The atoms inside an `IxPred` all bind to the **same** instruction — there is no way to write a
transaction-scope test that matches "program X in one instruction and data Y in another" by
accident.

### Account / signer atoms

```json
{ "signer_contains": "<Pk>" }              // a required signer (always Known)
{ "fee_payer_is": "<Pk>" }                 // account_keys[0]
{ "num_signers": { "op": "ge", "n": 1 } }
{ "tx_version": "v0" }
{ "uses_alt": true }
{ "account_contains": "<Pk>" }             // tri — any account in the full list
{ "writable_account_contains": "<Pk>" }    // tri — writable slot
{ "readonly_account_contains": "<Pk>" }    // tri — readonly slot
```

### Derived numeric facts

All are concrete `u64`; **absence is modeled as 0** (e.g. no `SetComputeUnitPrice` ⇒
`compute_unit_price = 0` ⇒ `priority_fee_lamports = 0`).

```json
{ "compute_unit_price":    { "op": "ge", "n": 1000 } }     // micro-lamports per CU
{ "compute_unit_limit":    { "op": "le", "n": 200000 } }   // CU
{ "priority_fee_lamports": { "op": "ge", "n": 100000 } }   // price * limit / 1e6
{ "total_fee_lamports":    { "op": "ge", "n": 105000 } }   // 5000*signers + priority
```

## Instruction scope — `IxPred`

Used only inside a quantifier. All atoms bind to one instruction.

### Logic

```json
{ "and": [ <IxPred>, … ] }    // non-empty
{ "or":  [ <IxPred>, … ] }    // non-empty
{ "not": <IxPred> }
{ "const": true }
```

### Atoms

```json
{ "program_id_is": "<Pk>" }                              // tri — program id can be ALT-loaded in v0
{ "discriminator": { "offset": 0, "bytes": "02000000" } }// bytes length ≥ 1; exact byte match at offset
{ "data_len": { "op": "ge", "n": 9 } }
{ "account_at": { "index": 1, "pk": "<Pk>" } }           // tri (out of range → false)
{ "account_props_at": { "index": 0, "is_writable": true, "is_signer": true } } // both fields optional
{ "ix_account_contains": "<Pk>" }                        // tri
{ "decoded_name": "transfer" }                           // from the consumer's decoder registry
```

`account_props_at` omits `is_writable`/`is_signer` to leave that aspect unconstrained; an omitted
field always passes. Slot writability/signer-ness is structural, so this atom is two-valued even
under partial ALT.

#### data_slice

Read bytes at `offset` per `kind`, compare with `op` against `value`. Out-of-bounds → no match.

```json
{ "data_slice": { "offset": 4, "kind": "u64_le", "op": "ge", "value": 1000000 } }
{ "data_slice": { "offset": 0, "kind": "bytes",  "op": "eq", "value": "deadbeef" } }
```

| `kind` | width | reads as |
|---|---|---|
| `u8` | 1 | unsigned |
| `u16_le` / `u16_be` | 2 | unsigned, little/big-endian |
| `u32_le` / `u32_be` | 4 | unsigned |
| `u64_le` / `u64_be` | 8 | unsigned |
| `i64_le` | 8 | **signed** |
| `bytes` | len of `value` | raw byte compare |

Rules:
- numeric kinds require `value` to be a **JSON number**;
- `bytes` requires `value` to be a **hex string** and `op` to be `eq` or `ne` only.

Violations are load errors (`SliceValKindMismatch`, `BytesOpUnsupported`).

## Tri-state and `on_unknown`

The engine reasons in `True` / `False` / `Unknown` (Kleene K3). An atom that reads a key sitting
behind an **unresolved** Address Lookup Table returns `Unknown` rather than `False` — it cannot
prove falsity. `Unknown` propagates: `True ∧ Unknown = Unknown`, `True ∨ Unknown = True`,
`¬Unknown = Unknown`.

When a whole rule evaluates to `Unknown`, the rule's `on_unknown` decides:

| `on_unknown` | result |
|---|---|
| `skip` (default) | rule does not match |
| `fail_closed` | rule does not match |
| `treat_true` | rule matches |

Note that at the **engine** level, if *any* account in the transaction is unresolved
(`has_unresolved == true`), `Engine::match_tx` returns `Deferred` instead of evaluating — resolve
the tables and re-submit. `on_unknown` matters for direct `eval_rule`/`match_tx_scan` use.

## Validation summary (load-time)

A predicate is rejected (and its rule skipped) if any of these hold:

- malformed base58 `Pk` or malformed hex `Bytes`;
- an `and`/`or` with an empty operand list;
- a `discriminator` with empty `bytes`;
- a `data_slice` whose `value` type disagrees with its `kind`, or `bytes` kind with an ordering op;
- an unknown atom name or an unexpected field in a struct atom;
- `schema_version` greater than `ENGINE_SCHEMA_VERSION`.

## Versioning

This is schema **v1**. New predicate atoms are added in later versions; a rule that uses them sets
its `schema_version` accordingly. Engines built against an older schema skip such rules at load
rather than failing, so new keywords never break a deployed engine. See the README's *Adding new
keywords later* section.

## Worked examples

See the README's *Example rules* section (durable-nonce detection, Raydium AMM v4 swap, Pump.fun
mint, Astralane tip transfer). They are exercised in `tests/example_rules.rs`.

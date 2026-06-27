//! The README / docs example rules must parse and validate as schema-v1 predicates.

use serde_json::json;
use solana_pubkey::Pubkey;
use transaction_predicate_matcher::{OnUnknown, Rule};

fn load(predicate: serde_json::Value) -> Rule {
    Rule::load_from_json("example".into(), true, OnUnknown::Skip, 1, &predicate)
        .expect("example predicate must load")
}

#[test]
fn durable_nonce_detection() {
    load(json!({ "instruction_at": { "index": 0, "pred": { "and": [
        { "program_id_is": "11111111111111111111111111111111" },
        { "discriminator": { "offset": 0, "bytes": "04000000" } }
    ] } } }));
}

#[test]
fn raydium_amm_v4_swap() {
    load(json!({ "any_instruction": { "and": [
        { "program_id_is": "675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8" },
        { "or": [
            { "discriminator": { "offset": 0, "bytes": "09" } },
            { "discriminator": { "offset": 0, "bytes": "0b" } }
        ] }
    ] } }));
}

#[test]
fn pumpfun_mint() {
    load(json!({ "any_instruction": { "and": [
        { "program_id_is": "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P" },
        { "discriminator": { "offset": 0, "bytes": "181ec828051c0777" } }
    ] } }));
}

#[test]
fn astralane_tip_transfer_0_001_sol() {
    // The tip account is operator config; use a valid placeholder pubkey for the test.
    let tip = Pubkey::new_from_array([7u8; 32]).to_string();
    load(json!({ "any_instruction": { "and": [
        { "program_id_is": "11111111111111111111111111111111" },
        { "discriminator": { "offset": 0, "bytes": "02000000" } },
        { "account_at": { "index": 1, "pk": tip } },
        { "data_slice": { "offset": 4, "kind": "u64_le", "op": "eq", "value": 1000000 } }
    ] } }));
}

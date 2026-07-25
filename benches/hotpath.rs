//! Hotpath benchmarks: how `match_view` scales with instruction count and rule count, and what the
//! index buys over the brute-force scan. `instr_scaling` exposes the O(n^2) instruction walk
//! (`instruction(idx)` is `instructions_iter().nth(idx)`).

use agave_transaction_view::transaction_view::SanitizedTransactionView;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use solana_message::compiled_instruction::CompiledInstruction;
use solana_message::{Message as LegacyMessage, MessageHeader, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signature::Signature;
use solana_transaction::versioned::VersionedTransaction;
use std::str::FromStr;
use transaction_predicate_matcher::ast::*;
use transaction_predicate_matcher::value::*;
use transaction_predicate_matcher::RuleSet;

fn pk(n: u8) -> Pubkey {
    Pubkey::new_from_array([n; 32])
}
fn raydium() -> Pubkey {
    Pubkey::from_str("675kPX9MHTjS2zt1qfr1NYHuzeLXfQM9H24wFSUt1Mp8").unwrap()
}

/// A legacy tx with `n_ix` instructions; the LAST one is the Raydium swapBaseIn we search for, so
/// `any_instruction` has to walk the whole list (worst case for the nth()-based iteration).
fn tx_with_n_instructions(n_ix: usize) -> Vec<u8> {
    let prog = raydium();
    let keys = vec![pk(200), prog];
    let mut ixs: Vec<CompiledInstruction> = Vec::with_capacity(n_ix);
    for i in 0..n_ix {
        // all but the last are a no-op discriminator; the last is 0x09 (swapBaseIn).
        let disc = if i == n_ix - 1 { 0x09u8 } else { 0x00u8 };
        ixs.push(CompiledInstruction::new_from_raw_parts(1, vec![disc, 1, 2, 3], vec![]));
    }
    let msg = LegacyMessage {
        header: MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 1,
        },
        account_keys: keys,
        recent_blockhash: Default::default(),
        instructions: ixs,
    };
    let tx = VersionedTransaction {
        signatures: vec![Signature::default(); 1],
        message: VersionedMessage::Legacy(msg),
    };
    bincode::serialize(&tx).unwrap()
}

/// The canonical Raydium swapBaseIn rule (any_instruction { program_id_is && discriminator 09 }).
fn raydium_rule(id: i64) -> Rule {
    let pred = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(raydium())),
        IxPred::Discriminator { offset: 0, bytes: Bytes(vec![0x09]) },
    ])));
    Rule::from_row(id, true, OnUnknown::Skip, 1, pred).unwrap()
}

/// A non-matching rule keyed on a distinct program, to pad the rule set without ever matching.
fn filler_rule(id: i64) -> Rule {
    let pred = Pred::AnyInstruction(Box::new(IxPred::And(vec![
        IxPred::ProgramIdIs(Pk(pk((id % 200) as u8))),
        IxPred::Discriminator { offset: 0, bytes: Bytes(vec![0xEE]) },
    ])));
    Rule::from_row(id, true, OnUnknown::Skip, 1, pred).unwrap()
}

fn instr_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("instr_scaling");
    let rs = RuleSet::new(vec![raydium_rule(1)]);
    for &n in &[1usize, 4, 16, 64] {
        let bytes = tx_with_n_instructions(n);
        let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), true).unwrap();
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &view, |b, view| {
            b.iter(|| black_box(rs.match_view(black_box(view), None)));
        });
    }
    g.finish();
}

fn rule_scaling(c: &mut Criterion) {
    let mut g = c.benchmark_group("rule_scaling");
    // realistic-ish tx: 6 instructions, the last is the Raydium swap.
    let bytes = tx_with_n_instructions(6);
    let view = SanitizedTransactionView::try_new_sanitized(bytes.as_slice(), true).unwrap();

    for &n_rules in &[1usize, 10, 100, 1000] {
        // 1 matching rule at the END, n_rules-1 fillers first => index must skip the fillers.
        let mut rules: Vec<Rule> = (0..n_rules - 1).map(|i| filler_rule(i as i64 + 2)).collect();
        rules.push(raydium_rule(1));
        let indexed = RuleSet::new(rules.clone());
        let scan = RuleSet::new(rules);

        g.bench_with_input(BenchmarkId::new("indexed", n_rules), &view, |b, view| {
            b.iter(|| black_box(indexed.match_view(black_box(view), None)));
        });
        g.bench_with_input(BenchmarkId::new("brute_scan", n_rules), &view, |b, view| {
            b.iter(|| black_box(scan.match_view_scan(black_box(view), None)));
        });
    }
    g.finish();
}

criterion_group!(benches, instr_scaling, rule_scaling);
criterion_main!(benches);

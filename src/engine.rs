//! The top-level engine: rules + candidate index + hot-reload via `ArcSwap`.

use crate::{ast::*, eval::*, facts::*, index::*};
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RuleSet {
    pub rules: HashMap<i64, Rule>,
    /// rule ids sorted by priority DESC, then id ASC.
    pub order: Vec<i64>,
    pub index: CandidateIndex,
}

pub struct Engine {
    inner: ArcSwap<RuleSet>,
}

pub enum MatchResult {
    /// Ids of the rules that matched, in priority order (highest first). Map each id to an action.
    Matched(Vec<i64>),
    /// Partial ALT (§7.4 policy A): consumer should resolve tables and re-submit.
    Deferred,
}

impl Engine {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self {
            inner: ArcSwap::from_pointee(Self::compile(rules)),
        }
    }

    pub fn reload(&self, rules: Vec<Rule>) {
        self.inner.store(Arc::new(Self::compile(rules)));
    }

    fn compile(mut rules: Vec<Rule>) -> RuleSet {
        rules.retain(|r| r.enabled);
        let index = CandidateIndex::build(&rules);
        let mut order: Vec<i64> = rules.iter().map(|r| r.id).collect();
        let by_id: HashMap<i64, Rule> = rules.into_iter().map(|r| (r.id, r)).collect();
        order.sort_by(|a, b| {
            let (ra, rb) = (&by_id[a], &by_id[b]);
            rb.priority.cmp(&ra.priority).then(a.cmp(b))
        });
        RuleSet {
            rules: by_id,
            order,
            index,
        }
    }

    pub fn match_tx(&self, tx: &TxFacts) -> MatchResult {
        let rs = self.inner.load();

        // §7.4 partial-ALT policy (A): defer.
        if tx.has_unresolved {
            return MatchResult::Deferred;
        }

        let cands = rs.index.candidates(tx);
        let mut matched = Vec::new();
        for id in &rs.order {
            if !cands.contains(id) {
                continue;
            }
            let out = eval_rule(&rs.rules[id], tx);
            if out.matched {
                matched.push(*id);
            }
            if out.stop {
                break;
            }
        }
        MatchResult::Matched(matched)
    }

    /// Brute-force evaluation over all enabled rules in priority order, ignoring the index.
    /// Used by §7.4 policy (B) and as the property-test oracle.
    pub fn match_tx_scan(&self, tx: &TxFacts) -> Vec<i64> {
        let rs = self.inner.load();
        let mut matched = Vec::new();
        for id in &rs.order {
            let out = eval_rule(&rs.rules[id], tx);
            if out.matched {
                matched.push(*id);
            }
            if out.stop {
                break;
            }
        }
        matched
    }
}

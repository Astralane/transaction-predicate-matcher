//! The top-level rule set: rules + candidate index + hot-reload via `ArcSwap`.

use crate::{ast::*, eval::*, facts::*, index::*};
use agave_transaction_view::transaction_data::TransactionData;
use agave_transaction_view::transaction_view::SanitizedTransactionView;
use arc_swap::ArcSwap;
use std::collections::HashMap;
use std::sync::Arc;

/// The compiled form swapped in atomically on reload: rules by id, priority order, and the index.
pub struct Compiled {
    pub rules: HashMap<i64, Rule>,
    /// rule ids sorted by priority DESC, then id ASC.
    pub order: Vec<i64>,
    pub index: CandidateIndex,
}

/// A live, hot-reloadable set of rules to match transactions against.
pub struct RuleSet {
    inner: ArcSwap<Compiled>,
}

pub enum MatchResult {
    /// Ids of the rules that matched, in priority order (highest first). Map each id to an action.
    Matched(Vec<i64>),
    /// Partial ALT (policy A): consumer should resolve tables and re-submit.
    Deferred,
}

impl RuleSet {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self {
            inner: ArcSwap::from_pointee(Self::compile(rules)),
        }
    }

    pub fn reload(&self, rules: Vec<Rule>) {
        self.inner.store(Arc::new(Self::compile(rules)));
    }

    fn compile(mut rules: Vec<Rule>) -> Compiled {
        rules.retain(|r| r.enabled);
        let index = CandidateIndex::build(&rules);
        let mut order: Vec<i64> = rules.iter().map(|r| r.id).collect();
        let by_id: HashMap<i64, Rule> = rules.into_iter().map(|r| (r.id, r)).collect();
        order.sort_by(|a, b| {
            let (ra, rb) = (&by_id[a], &by_id[b]);
            rb.priority.cmp(&ra.priority).then(a.cmp(b))
        });
        Compiled {
            rules: by_id,
            order,
            index,
        }
    }

    /// Match the rules against a sanitized transaction view + optional ALT cache.
    /// Returns `Deferred` if any account is unresolved (partial-ALT policy A).
    pub fn match_view<D: TransactionData>(
        &self,
        view: &SanitizedTransactionView<D>,
        alt: Option<&AccountLookupTableCache>,
    ) -> MatchResult {
        let facts = ViewFacts::new(view, alt);
        let rs = self.inner.load();

        if facts.has_unresolved() {
            return MatchResult::Deferred;
        }

        let cands = rs.index.candidates(&facts);
        let mut matched = Vec::new();
        for id in &rs.order {
            if !cands.contains(id) {
                continue;
            }
            let out = eval_rule(&rs.rules[id], &facts);
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
    /// Used by partial-ALT policy (B) and as the property-test oracle. Does not defer.
    pub fn match_view_scan<D: TransactionData>(
        &self,
        view: &SanitizedTransactionView<D>,
        alt: Option<&AccountLookupTableCache>,
    ) -> Vec<i64> {
        let facts = ViewFacts::new(view, alt);
        let rs = self.inner.load();
        let mut matched = Vec::new();
        for id in &rs.order {
            let out = eval_rule(&rs.rules[id], &facts);
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

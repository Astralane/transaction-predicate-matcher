//! The top-level rule set: rules (in priority order) + candidate index + hot-reload via `ArcSwap`.

use crate::{ast::*, eval::*, facts::*, index::*};
use agave_transaction_view::transaction_data::TransactionData;
use agave_transaction_view::transaction_view::SanitizedTransactionView;
use arc_swap::ArcSwap;
use std::sync::Arc;

/// The compiled form swapped in atomically on reload: rules in priority order + the index.
pub struct Compiled {
    /// Rules in priority order (index 0 = highest priority), exactly as supplied.
    pub rules: Vec<Rule>,
    pub index: CandidateIndex,
}

/// A live, hot-reloadable set of rules. Rules are matched in `Vec` order; index 0 wins ties.
pub struct RuleSet {
    inner: ArcSwap<Compiled>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MatchResult {
    /// The `id` of the first matching rule (rules are tried in `Vec` order). Map it to an action.
    Matched(i64),
    /// No rule matched.
    NoMatch,
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

    fn compile(rules: Vec<Rule>) -> Compiled {
        let index = CandidateIndex::build(&rules);
        Compiled { rules, index }
    }

    /// Match against a sanitized transaction view + optional ALT cache, returning the first
    /// matching rule's position. Returns `Deferred` if any account is unresolved (policy A).
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
        for (i, rule) in rs.rules.iter().enumerate() {
            if !rule.enabled || !cands.contains(&i) {
                continue;
            }
            if eval_rule(rule, &facts) {
                return MatchResult::Matched(rule.id);
            }
        }
        MatchResult::NoMatch
    }

    /// First match by brute force over all enabled rules in order, ignoring the index.
    /// Used by partial-ALT policy (B) and as the property-test oracle. Never returns `Deferred`
    /// (it evaluates even when accounts are unresolved, per `on_unknown`).
    pub fn match_view_scan<D: TransactionData>(
        &self,
        view: &SanitizedTransactionView<D>,
        alt: Option<&AccountLookupTableCache>,
    ) -> MatchResult {
        let facts = ViewFacts::new(view, alt);
        let rs = self.inner.load();
        match rs.rules.iter().find(|r| r.enabled && eval_rule(r, &facts)) {
            Some(r) => MatchResult::Matched(r.id),
            None => MatchResult::NoMatch,
        }
    }
}

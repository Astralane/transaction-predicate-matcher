//! `transaction-predicate-matcher` — evaluate configurable boolean/routing rules against pending
//! Solana transactions, using three-valued logic so partial Address-Lookup-Table knowledge never
//! silently collapses to `false`.
//!
//! Matching runs directly against a borrowing [`facts::ViewFacts`] over a
//! `SanitizedTransactionView` plus an optional [`AccountLookupTableCache`], so it allocates nothing
//! per transaction. Top-level entry point: [`RuleSet`].

pub mod ast;
pub mod budget;
pub mod engine;
pub mod error;
pub mod eval;
pub mod facts;
pub mod index;
pub mod tri;
pub mod value;

pub use ast::{load_rules, IxPred, OnUnknown, Pred, RawRule, Rule, ENGINE_SCHEMA_VERSION};
pub use engine::{Compiled, MatchResult, RuleSet};
pub use error::{EngineError, LoadError};
pub use eval::{eval_ix, eval_pred, eval_rule, eval_slice};
pub use facts::{AccountLookupTableCache, IxView, MaybeKey, ViewFacts};
pub use index::{CandidateIndex, Trig, TrigKey};
pub use tri::Tri;
pub use value::{Bytes, Cmp, Pk, SliceKind, SliceVal, TxVer};

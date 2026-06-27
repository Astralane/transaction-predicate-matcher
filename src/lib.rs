//! `transaction-predicate-matcher` — evaluate configurable boolean/routing rules against pending
//! (unlanded) Solana transactions, using three-valued logic so partial Address-Lookup-Table
//! knowledge never silently collapses to `false`.
//!
//! See the crate's implementation spec for the full contract. Top-level entry point: [`Engine`].

pub mod ast;
pub mod engine;
pub mod error;
pub mod eval;
pub mod facts;
pub mod index;
pub mod tri;
pub mod value;

#[cfg(feature = "build-facts")]
pub mod build;

pub use ast::{
    load_rules, IxPred, OnUnknown, Pred, RawRule, Rule, ENGINE_SCHEMA_VERSION,
};
pub use engine::{Engine, MatchResult, RuleSet};
pub use error::{EngineError, LoadError};
pub use eval::{eval_ix, eval_pred, eval_rule, eval_slice, RuleOutput};
pub use facts::{IxFacts, MaybeKey, TxFacts};
pub use index::{CandidateIndex, Trig, TrigKey};
pub use tri::Tri;
pub use value::{Bytes, Cmp, Pk, SliceKind, SliceVal, TxVer};

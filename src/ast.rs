//! The rule AST. Deserializes from JSON; `validate` enforces load-time invariants.
//!
//! A rule is pure matching logic — a single predicate. It carries **no outcome/decision**: the
//! engine reports which rule ids matched, and the consumer owns the rule-id → action mapping.

use crate::error::LoadError;
use crate::value::*;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Pred {
    // TRANSACTION scope
    And(Vec<Pred>),
    Or(Vec<Pred>),
    Not(Box<Pred>),
    Const(bool),

    // quantifiers — the ONLY bridge into instruction scope
    AnyInstruction(Box<IxPred>),
    AllInstructions(Box<IxPred>),
    InstructionAt { index: usize, pred: Box<IxPred> },
    InstructionCount { op: Cmp, n: usize },

    // tx-level atoms — ALWAYS two-valued (never depend on ALTs)
    SignerContains(Pk),
    FeePayerIs(Pk),
    NumSigners { op: Cmp, n: usize },
    TxVersion(TxVer),
    UsesAlt(bool),

    // tx-level atoms — may be Unknown under partial ALT
    AccountContains(Pk),
    WritableAccountContains(Pk),
    ReadonlyAccountContains(Pk),

    // derived numeric facts
    ComputeUnitPrice { op: Cmp, n: u64 },
    ComputeUnitLimit { op: Cmp, n: u64 },
    PriorityFeeLamports { op: Cmp, n: u64 },
    TotalFeeLamports { op: Cmp, n: u64 },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum IxPred {
    // SINGLE-INSTRUCTION scope
    And(Vec<IxPred>),
    Or(Vec<IxPred>),
    Not(Box<IxPred>),
    Const(bool),

    ProgramIdIs(Pk),
    Discriminator { offset: usize, bytes: Bytes },
    DataSlice {
        offset: usize,
        kind: SliceKind,
        op: Cmp,
        value: SliceVal,
    },
    DataLen { op: Cmp, n: usize },
    AccountAt { index: usize, pk: Pk },
    AccountPropsAt {
        index: usize,
        #[serde(default)]
        is_writable: Option<bool>,
        #[serde(default)]
        is_signer: Option<bool>,
    },
    IxAccountContains(Pk),
    DecodedName(String),
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OnUnknown {
    #[default]
    Skip,
    FailClosed,
    TreatTrue,
}

/// The rule-schema version this engine build understands. Bump it whenever you add a new
/// predicate keyword. A rule authored for a newer schema version than the running engine
/// supports is skipped at load (see [`Rule::load_from_json`]) instead of breaking the whole rule
/// set — so adding keywords never breaks an older deployed engine.
pub const ENGINE_SCHEMA_VERSION: u32 = 2;

/// A single matching rule. The `predicate` is the only thing authored in JSON; everything else is
/// rule metadata. There is no priority field — a rule's position in the [`crate::RuleSet`]'s `Vec`
/// is its priority (index 0 is highest). On a match, the matcher reports the rule's `id`; the
/// action it triggers is resolved by the consumer, not encoded here.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Rule {
    pub id: i64,
    pub enabled: bool,
    pub on_unknown: OnUnknown,
    /// Schema version this rule was authored against. The engine accepts rules with
    /// `schema_version <= ENGINE_SCHEMA_VERSION` and rejects newer ones.
    pub schema_version: u32,
    pub predicate: Pred,
}

impl Rule {
    /// Assemble a `Rule` from an already-deserialized `Pred`. Rejects the rule if it targets a
    /// newer schema version than this engine supports, then runs `validate` (rejecting on the
    /// first failure).
    pub fn from_row(
        id: i64,
        enabled: bool,
        on_unknown: OnUnknown,
        schema_version: u32,
        predicate: Pred,
    ) -> Result<Rule, LoadError> {
        check_schema_version(schema_version)?;
        validate(&predicate)?;
        Ok(Rule {
            id,
            enabled,
            on_unknown,
            schema_version,
            predicate,
        })
    }

    /// Versioned load path for persisted rules: gate on `schema_version` *before* parsing, so an
    /// older engine never trips over keywords it doesn't know — it skips the future rule cleanly.
    pub fn load_from_json(
        id: i64,
        enabled: bool,
        on_unknown: OnUnknown,
        schema_version: u32,
        predicate_json: &serde_json::Value,
    ) -> Result<Rule, LoadError> {
        check_schema_version(schema_version)?;
        let predicate: Pred = serde_json::from_value(predicate_json.clone())?;
        Self::from_row(id, enabled, on_unknown, schema_version, predicate)
    }
}

fn check_schema_version(schema_version: u32) -> Result<(), LoadError> {
    if schema_version > ENGINE_SCHEMA_VERSION {
        return Err(LoadError::UnsupportedSchemaVersion {
            rule: schema_version,
            engine: ENGINE_SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// A raw rule row, as read from storage (the `predicate` column is still un-parsed JSON).
/// Order matters: the position in the input `Vec` is the rule's priority.
pub struct RawRule {
    pub id: i64,
    pub enabled: bool,
    pub on_unknown: OnUnknown,
    pub schema_version: u32,
    pub predicate: serde_json::Value,
}

/// Load a batch of raw rules, isolating failures: returns the rules that loaded (in input order)
/// plus the `(id, error)` pairs for those that didn't. One bad or too-new rule never poisons the
/// set.
pub fn load_rules(raw: Vec<RawRule>) -> (Vec<Rule>, Vec<(i64, LoadError)>) {
    let mut ok = Vec::new();
    let mut errs = Vec::new();
    for r in raw {
        match Rule::load_from_json(r.id, r.enabled, r.on_unknown, r.schema_version, &r.predicate) {
            Ok(rule) => ok.push(rule),
            Err(e) => errs.push((r.id, e)),
        }
    }
    (ok, errs)
}

// ---- validation ----

pub fn validate(p: &Pred) -> Result<(), LoadError> {
    use Pred::*;
    match p {
        And(v) | Or(v) => {
            if v.is_empty() {
                return Err(LoadError::EmptyOperands);
            }
            for c in v {
                validate(c)?;
            }
            Ok(())
        }
        Not(c) => validate(c),
        AnyInstruction(ix) | AllInstructions(ix) => validate_ix(ix),
        InstructionAt { pred, .. } => validate_ix(pred),
        _ => Ok(()),
    }
}

pub fn validate_ix(p: &IxPred) -> Result<(), LoadError> {
    use IxPred::*;
    match p {
        And(v) | Or(v) => {
            if v.is_empty() {
                return Err(LoadError::EmptyOperands);
            }
            for c in v {
                validate_ix(c)?;
            }
            Ok(())
        }
        Not(c) => validate_ix(c),
        Discriminator { bytes, .. } => {
            if bytes.0.is_empty() {
                return Err(LoadError::EmptyDiscriminator);
            }
            Ok(())
        }
        DataSlice { kind, op, value, .. } => {
            match (kind.is_numeric(), value) {
                (true, SliceVal::Num(_)) => {}
                (false, SliceVal::Bytes(_)) => {}
                _ => return Err(LoadError::SliceValKindMismatch),
            }
            if !kind.is_numeric() && !matches!(op, Cmp::Eq | Cmp::Ne) {
                return Err(LoadError::BytesOpUnsupported);
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

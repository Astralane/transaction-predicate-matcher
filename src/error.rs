#[derive(thiserror::Error, Debug)]
pub enum LoadError {
    #[error("bad base58 pubkey: {0}")]
    BadPubkey(String),
    #[error("bad hex: {0}")]
    BadHex(String),
    #[error("data_slice value/kind mismatch")]
    SliceValKindMismatch,
    #[error("bytes kind supports only eq/ne")]
    BytesOpUnsupported,
    #[error("empty and/or operand list")]
    EmptyOperands,
    #[error("rule schema version {rule} exceeds engine schema version {engine}")]
    UnsupportedSchemaVersion { rule: u32, engine: u32 },
    #[error("discriminator bytes must be non-empty")]
    EmptyDiscriminator,
    #[error("data_contains bytes must be non-empty")]
    EmptyDataContains,
    #[error("signer_in must contain at least one pubkey")]
    EmptySignerIn,
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum EngineError {
    #[error("load: {0}")]
    Load(#[from] LoadError),
}

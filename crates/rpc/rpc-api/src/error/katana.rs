use jsonrpsee::types::ErrorObjectOwned;

#[derive(thiserror::Error, Clone, Debug)]
#[allow(clippy::enum_variant_names)]
pub enum KatanaApiError {
    #[error("Failed to change next block timestamp.")]
    FailedToChangeNextBlockTimestamp,
    #[error("Failed to dump state.")]
    FailedToDumpState,
    #[error("Failed to update storage.")]
    FailedToUpdateStorage,
    #[error("Invalid block range: from_block must be less than to_block.")]
    InvalidBlockRange,
    #[error("Block not found.")]
    BlockNotFound,
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl From<KatanaApiError> for ErrorObjectOwned {
    fn from(err: KatanaApiError) -> Self {
        let code = match &err {
            KatanaApiError::FailedToChangeNextBlockTimestamp => 1,
            KatanaApiError::FailedToDumpState => 2,
            KatanaApiError::FailedToUpdateStorage => 3,
            KatanaApiError::InvalidBlockRange => 4,
            KatanaApiError::BlockNotFound => 5,
            KatanaApiError::InternalError(_) => 6,
        };
        ErrorObjectOwned::owned(code, err.to_string(), None::<()>)
    }
}

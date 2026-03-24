use katana_tee::TeeProviderType;

/// TEE configuration options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeeConfig {
    /// The type of TEE provider to use for attestation.
    pub provider_type: TeeProviderType,
    /// Hash of security-critical runtime args attested in report_data[32..64].
    /// In stable forked mode this includes `fork.block`.
    pub args_hash: [u8; 32],
    /// The block number Katana forked from (resolved at startup).
    /// Included in TEE report_data for fork freshness verification.
    pub fork_block_number: Option<u64>,
}

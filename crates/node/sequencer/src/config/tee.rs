use katana_tee::TeeProviderType;

/// TEE configuration options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeeConfig {
    /// The type of TEE provider to use for attestation.
    pub provider_type: TeeProviderType,
    /// The block number Katana forked from (resolved at startup).
    /// Included in TEE report_data for fork freshness verification.
    pub fork_block_number: Option<u64>,
    /// SHA-256 hash of security-critical runtime arguments.
    /// Embedded in report_data[32..64] for attestation binding.
    pub args_hash: Option<[u8; 32]>,
}

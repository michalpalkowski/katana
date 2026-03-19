use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
use katana_primitives::block::BlockNumber;
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::ContractAddress;
use katana_rpc_types::broadcasted::{
    BroadcastedDeclareTx, BroadcastedDeployAccountTx, BroadcastedInvokeTx,
};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use serde::{Deserialize, Serialize};

/// A single storage key-value change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageDiffEntry {
    pub key: StorageKey,
    pub value: StorageValue,
}

/// Katana-specific JSON-RPC methods.
#[cfg_attr(not(feature = "client"), rpc(server, namespace = "katana"))]
#[cfg_attr(feature = "client", rpc(client, server, namespace = "katana"))]
pub trait KatanaApi {
    /// Submit a new invoke transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addInvokeTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addInvokeTransactionSync")]
    async fn add_invoke_transaction_sync(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new declare transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addDeclareTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addDeclareTransactionSync")]
    async fn add_declare_transaction_sync(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Submit a new deploy account transaction and wait until the receipt is available.
    ///
    /// This is a synchronous version of the `starknet_addDeployAccountTransaction` method where the
    /// request's response is the actual receipt of the transaction's execution - the receipt is
    /// returned immediately once it becomes available.
    #[method(name = "addDeployAccountTransactionSync")]
    async fn add_deploy_account_transaction_sync(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<TxReceiptWithBlockInfo>;

    /// Returns the accumulated storage diff for a single contract across a range of blocks.
    ///
    /// For each block in `(from_block, to_block]`, the storage changes for `contract_address`
    /// are merged. Later blocks overwrite earlier values for the same key. The result is the
    /// net set of changed keys with their final values at `to_block`.
    #[method(name = "getStorageDiff")]
    async fn get_storage_diff(
        &self,
        contract_address: ContractAddress,
        from_block: BlockNumber,
        to_block: BlockNumber,
    ) -> RpcResult<Vec<StorageDiffEntry>>;
}

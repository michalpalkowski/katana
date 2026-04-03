use std::collections::BTreeMap;
use std::time::Duration;

use jsonrpsee::core::{async_trait, RpcResult};
use katana_pool::TransactionPool;
use katana_primitives::block::{BlockHashOrNumber, BlockNumber};
use katana_primitives::contract::{StorageKey, StorageValue};
use katana_primitives::transaction::TxHash;
use katana_primitives::ContractAddress;
use katana_provider::api::state_update::StateUpdateProvider;
use katana_provider::{ProviderFactory, ProviderRO};
use katana_rpc_api::error::katana::KatanaApiError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_api::katana::{KatanaApiServer, PrefetchResult, StorageDiffEntry};
use katana_rpc_api::starknet::StarknetWriteApiServer;
use katana_rpc_types::broadcasted::{
    AddDeclareTransactionResponse, AddDeployAccountTransactionResponse,
    AddInvokeTransactionResponse, BroadcastedDeclareTx, BroadcastedDeployAccountTx,
    BroadcastedInvokeTx,
};
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::{BroadcastedTx, BroadcastedTxWithChainId};

use super::StarknetApi;
use crate::starknet::pending::PendingBlockProvider;

const TX_RECEIPT_POLL_INTERVAL: Duration = Duration::from_millis(100);

impl<Pool, PoolTx, Pending, PF> StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
{
    pub async fn add_invoke_tx(
        &self,
        tx: BroadcastedInvokeTx,
    ) -> Result<AddInvokeTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let tx = BroadcastedTxWithChainId { tx: BroadcastedTx::Invoke(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddInvokeTransactionResponse { transaction_hash })
        })
        .await?
    }

    pub async fn add_declare_tx(
        &self,
        tx: BroadcastedDeclareTx,
    ) -> Result<AddDeclareTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let class_hash = tx.contract_class.hash();

            let tx = BroadcastedTxWithChainId { tx: BroadcastedTx::Declare(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddDeclareTransactionResponse { transaction_hash, class_hash })
        })
        .await?
    }

    pub async fn add_deploy_account_tx(
        &self,
        tx: BroadcastedDeployAccountTx,
    ) -> Result<AddDeployAccountTransactionResponse, StarknetApiError> {
        self.on_cpu_blocking_task(|this| async move {
            if tx.is_query() {
                return Err(StarknetApiError::UnsupportedTransactionVersion);
            }

            let chain_id = this.inner.chain_spec.id();
            let contract_address = tx.contract_address();
            let tx =
                BroadcastedTxWithChainId { tx: BroadcastedTx::DeployAccount(tx), chain: chain_id };

            let transaction_hash = this.inner.pool.add_transaction(tx.into()).await?;
            Ok(AddDeployAccountTransactionResponse { transaction_hash, contract_address })
        })
        .await?
    }
}

impl<Pool, PoolTx, Pending, PF> StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    pub(super) async fn wait_for_tx_receipt(
        &self,
        transaction_hash: TxHash,
    ) -> Result<TxReceiptWithBlockInfo, StarknetApiError> {
        loop {
            match self.receipt(transaction_hash).await {
                Ok(receipt) => return Ok(receipt),
                Err(StarknetApiError::TxnHashNotFound) => {
                    tokio::time::sleep(TX_RECEIPT_POLL_INTERVAL).await;
                }
                Err(error) => return Err(error),
            }
        }
    }

    pub(super) async fn storage_diff(
        &self,
        contract_address: ContractAddress,
        from_block: BlockNumber,
        to_block: BlockNumber,
    ) -> Result<Vec<StorageDiffEntry>, KatanaApiError> {
        if from_block >= to_block {
            return Err(KatanaApiError::InvalidBlockRange);
        }

        self.on_io_blocking_task(move |this| {
            let provider = this.storage().provider();
            let mut accumulated: BTreeMap<StorageKey, StorageValue> = BTreeMap::new();

            for block_num in (from_block + 1)..=to_block {
                let block_id = BlockHashOrNumber::Num(block_num);
                let state_updates = StateUpdateProvider::state_update(&provider, block_id)
                    .map_err(|e| KatanaApiError::InternalError(e.to_string()))?
                    .ok_or(KatanaApiError::BlockNotFound)?;

                if let Some(contract_updates) = state_updates.storage_updates.get(&contract_address)
                {
                    for (key, value) in contract_updates {
                        accumulated.insert(*key, *value);
                    }
                }
            }

            Ok(accumulated
                .into_iter()
                .map(|(key, value)| StorageDiffEntry { key, value })
                .collect())
        })
        .await
        .map_err(|e| KatanaApiError::InternalError(e.to_string()))?
    }

    pub(super) async fn prefetch_storage_slots(
        &self,
        contract_address: ContractAddress,
        keys: Vec<StorageKey>,
    ) -> Result<PrefetchResult, KatanaApiError> {
        use katana_rpc_types::BlockIdOrTag;

        let start = std::time::Instant::now();
        let total = keys.len() as u32;

        if keys.is_empty() {
            return Ok(PrefetchResult { total: 0, fetched: 0, failed: 0, elapsed_ms: 0 });
        }

        // Split keys into chunks — each chunk runs as one blocking task that
        // sequentially reads its keys. The fork BackendWorker handles up to 200
        // concurrent RPC requests across all tasks.
        const CHUNK_SIZE: usize = 50;
        let chunks: Vec<Vec<StorageKey>> = keys.chunks(CHUNK_SIZE).map(|c| c.to_vec()).collect();

        let mut join_set = tokio::task::JoinSet::new();
        for chunk in chunks {
            let this = self.clone();
            join_set.spawn(async move {
                this.on_io_blocking_task(move |api| {
                    let state = api.state(&BlockIdOrTag::Latest)?;
                    let mut fetched = 0u32;
                    let mut failed = 0u32;
                    for key in &chunk {
                        match state.storage(contract_address, *key) {
                            Ok(_) => fetched += 1,
                            Err(e) => {
                                tracing::warn!(
                                    key = %format!("{key:#x}"),
                                    error = %e,
                                    "prefetch: slot fetch failed"
                                );
                                failed += 1;
                            }
                        }
                    }
                    Ok::<_, StarknetApiError>((fetched, failed))
                })
                .await
            });
        }

        let mut fetched = 0u32;
        let mut failed = 0u32;
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(Ok(Ok((f, fail)))) => {
                    fetched += f;
                    failed += fail;
                }
                Ok(Ok(Err(e))) => {
                    tracing::warn!(error = %e, "prefetch: chunk state error");
                    failed += CHUNK_SIZE as u32;
                }
                Ok(Err(e)) => {
                    tracing::warn!(error = %e, "prefetch: chunk task error");
                    failed += CHUNK_SIZE as u32;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "prefetch: chunk join error");
                    failed += CHUNK_SIZE as u32;
                }
            }
        }

        let elapsed_ms = start.elapsed().as_millis() as u64;
        tracing::info!(
            %contract_address,
            total,
            fetched,
            failed,
            elapsed_ms,
            "storage prefetch complete"
        );

        Ok(PrefetchResult { total, fetched, failed, elapsed_ms })
    }
}

#[async_trait]
impl<Pool, PoolTx, Pending, PF> StarknetWriteApiServer for StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
{
    async fn add_invoke_transaction(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<AddInvokeTransactionResponse> {
        Ok(self.add_invoke_tx(invoke_transaction).await?)
    }

    async fn add_declare_transaction(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<AddDeclareTransactionResponse> {
        Ok(self.add_declare_tx(declare_transaction).await?)
    }

    async fn add_deploy_account_transaction(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<AddDeployAccountTransactionResponse> {
        Ok(self.add_deploy_account_tx(deploy_account_transaction).await?)
    }
}

#[async_trait]
impl<Pool, PoolTx, Pending, PF> KatanaApiServer for StarknetApi<Pool, Pending, PF>
where
    Pool: TransactionPool<Transaction = PoolTx> + Send + Sync + 'static,
    PoolTx: From<BroadcastedTxWithChainId>,
    Pending: PendingBlockProvider,
    PF: ProviderFactory,
    <PF as ProviderFactory>::Provider: ProviderRO,
{
    async fn add_invoke_transaction_sync(
        &self,
        invoke_transaction: BroadcastedInvokeTx,
    ) -> RpcResult<TxReceiptWithBlockInfo> {
        let response = self.add_invoke_tx(invoke_transaction).await?;
        Ok(self.wait_for_tx_receipt(response.transaction_hash).await?)
    }

    async fn add_declare_transaction_sync(
        &self,
        declare_transaction: BroadcastedDeclareTx,
    ) -> RpcResult<TxReceiptWithBlockInfo> {
        let response = self.add_declare_tx(declare_transaction).await?;
        Ok(self.wait_for_tx_receipt(response.transaction_hash).await?)
    }

    async fn add_deploy_account_transaction_sync(
        &self,
        deploy_account_transaction: BroadcastedDeployAccountTx,
    ) -> RpcResult<TxReceiptWithBlockInfo> {
        let response = self.add_deploy_account_tx(deploy_account_transaction).await?;
        Ok(self.wait_for_tx_receipt(response.transaction_hash).await?)
    }

    async fn get_storage_diff(
        &self,
        contract_address: ContractAddress,
        from_block: BlockNumber,
        to_block: BlockNumber,
    ) -> RpcResult<Vec<StorageDiffEntry>> {
        Ok(self.storage_diff(contract_address, from_block, to_block).await?)
    }

    async fn prefetch_storage(
        &self,
        contract_address: ContractAddress,
        keys: Vec<StorageKey>,
    ) -> RpcResult<PrefetchResult> {
        Ok(self.prefetch_storage_slots(contract_address, keys).await?)
    }
}

use jsonrpsee::http_client::HttpClientBuilder;
use katana_primitives::block::{BlockHash, BlockIdOrTag, BlockNumber};
use katana_primitives::class::ClassHash;
use katana_primitives::contract::ContractAddress;
use katana_primitives::transaction::TxHash;
use katana_primitives::Felt;
use katana_provider::error::ProviderError;
use katana_rpc_api::error::starknet::StarknetApiError;
use katana_rpc_types::block::{
    MaybePendingBlockWithReceipts, MaybePendingBlockWithTxHashes, MaybePendingBlockWithTxs,
};
use katana_rpc_types::event::EventsPage;
use katana_rpc_types::receipt::TxReceiptWithBlockInfo;
use katana_rpc_types::state_update::MaybePendingStateUpdate;
use katana_rpc_types::transaction::Tx;
use katana_rpc_types::trie::{ContractStorageKeys, GetStorageProofResponse};
use starknet::core::types::{EventFilter, TransactionStatus};
use starknet::providers::jsonrpc::HttpTransport;
use starknet::providers::ProviderError as StarknetProviderError;
use starknet::providers::{JsonRpcClient, Provider};
use url::Url;
// use katana_rpc_api::starknet::StarknetApiClient;
use jsonrpsee::core::Error as JsonRpcseError;
use starknet::core::types::{
    BlockId, BlockTag, MaybePendingStateUpdate as StarknetRsMaybePendingStateUpdate,
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Error originating from the underlying [`Provider`] implementation.
    #[error("Provider error: {0}")]
    Provider(#[from] StarknetProviderError),

    #[error("Block out of range")]
    BlockOutOfRange,

    #[error("Not allowed to use block tag as a block identifier")]
    BlockTagNotAllowed,

    #[error("Unexpected pending data")]
    UnexpectedPendingData,

    /// Error from jsonrpsee client
    #[error("JsonRPC client error: {0}")]
    JsonRpc(#[from] JsonRpcseError),

    /// Error from katana provider
    #[error("Katana provider error: {0}")]
    KatanaProvider(#[from] ProviderError),
}

#[derive(Debug)]
pub struct ForkedClient<P: Provider = JsonRpcClient<HttpTransport>> {
    /// The block number where the node is forked from.
    block: BlockNumber,
    /// The Starknet Json RPC provider client for doing the request to the forked network.
    provider: P,
    /// The URL of the forked network (only set when using JsonRpcClient<HttpTransport>).
    url: Option<Url>,
}

impl<P: Provider> ForkedClient<P> {
    /// Creates a new forked client from the given [`Provider`] and block number.
    pub fn new(provider: P, block: BlockNumber) -> Self {
        Self { provider, block, url: None }
    }

    /// Returns the block number of the forked client.
    pub fn block(&self) -> &BlockNumber {
        &self.block
    }
}

impl ForkedClient {
    /// Creates a new forked client from the given HTTP URL and block number.
    pub fn new_http(url: Url, block: BlockNumber) -> Self {
        Self {
            provider: JsonRpcClient::new(HttpTransport::new(url.clone())),
            block,
            url: Some(url),
        }
    }
}

impl<P: Provider> ForkedClient<P> {
    pub async fn get_block_number_by_hash(&self, hash: BlockHash) -> Result<BlockNumber, Error> {
        use starknet::core::types::MaybePendingBlockWithTxHashes as StarknetRsMaybePendingBlockWithTxHashes;

        let block = self.provider.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
        // Pending block doesn't have a hash yet, so if we get a pending block, we return an error.
        let StarknetRsMaybePendingBlockWithTxHashes::Block(block) = block else {
            return Err(Error::UnexpectedPendingData);
        };

        if block.block_number > self.block {
            Err(Error::BlockOutOfRange)
        } else {
            Ok(block.block_number)
        }
    }

    pub async fn get_transaction_by_hash(&self, hash: TxHash) -> Result<Tx, Error> {
        let tx = self.provider.get_transaction_by_hash(hash).await?;
        Ok(tx.into())
    }

    pub async fn get_transaction_receipt(
        &self,
        hash: TxHash,
    ) -> Result<TxReceiptWithBlockInfo, Error> {
        let receipt = self.provider.get_transaction_receipt(hash).await?;

        if let starknet::core::types::ReceiptBlock::Block { block_number, .. } = receipt.block {
            if block_number > self.block {
                return Err(Error::BlockOutOfRange);
            }
        }

        Ok(receipt.into())
    }

    pub async fn get_transaction_status(&self, hash: TxHash) -> Result<TransactionStatus, Error> {
        let (receipt, status) = tokio::join!(
            self.get_transaction_receipt(hash),
            self.provider.get_transaction_status(hash)
        );

        // We get the receipt first to check if the block number is within the forked range.
        let _ = receipt?;

        Ok(status?)
    }

    pub async fn get_transaction_by_block_id_and_index(
        &self,
        block_id: BlockIdOrTag,
        idx: u64,
    ) -> Result<Tx, Error> {
        match block_id {
            BlockIdOrTag::Number(num) => {
                if num > self.block {
                    return Err(Error::BlockOutOfRange);
                }

                let tx = self.provider.get_transaction_by_block_id_and_index(block_id, idx).await?;
                Ok(tx.into())
            }

            BlockIdOrTag::Hash(hash) => {
                let (block, tx) = tokio::join!(
                    self.provider.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)),
                    self.provider.get_transaction_by_block_id_and_index(block_id, idx)
                );

                let number = match block? {
                    starknet::core::types::MaybePendingBlockWithTxHashes::Block(block) => {
                        block.block_number
                    }
                    starknet::core::types::MaybePendingBlockWithTxHashes::PendingBlock(_) => {
                        return Err(Error::UnexpectedPendingData);
                    }
                };

                if number > self.block {
                    return Err(Error::BlockOutOfRange);
                }

                Ok(tx?.into())
            }

            BlockIdOrTag::Tag(_) => Err(Error::BlockTagNotAllowed),
        }
    }

    pub async fn get_block_with_txs(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePendingBlockWithTxs, Error> {
        let block = self.provider.get_block_with_txs(block_id).await?;

        match block {
            starknet::core::types::MaybePendingBlockWithTxs::Block(ref b) => {
                if b.block_number > self.block {
                    Err(Error::BlockOutOfRange)
                } else {
                    Ok(block.into())
                }
            }

            starknet::core::types::MaybePendingBlockWithTxs::PendingBlock(_) => {
                Err(Error::UnexpectedPendingData)
            }
        }
    }

    pub async fn get_block_with_receipts(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePendingBlockWithReceipts, Error> {
        let block = self.provider.get_block_with_receipts(block_id).await?;

        match block {
            starknet::core::types::MaybePendingBlockWithReceipts::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            starknet::core::types::MaybePendingBlockWithReceipts::PendingBlock(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block.into())
    }

    pub async fn get_block_with_tx_hashes(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePendingBlockWithTxHashes, Error> {
        let block = self.provider.get_block_with_tx_hashes(block_id).await?;

        match block {
            starknet::core::types::MaybePendingBlockWithTxHashes::Block(ref b) => {
                if b.block_number > self.block {
                    return Err(Error::BlockOutOfRange);
                }
            }
            starknet::core::types::MaybePendingBlockWithTxHashes::PendingBlock(_) => {
                return Err(Error::UnexpectedPendingData);
            }
        }

        Ok(block.into())
    }

    pub async fn get_block_transaction_count(&self, block_id: BlockIdOrTag) -> Result<u64, Error> {
        match block_id {
            BlockIdOrTag::Number(num) if num > self.block => {
                return Err(Error::BlockOutOfRange);
            }
            BlockIdOrTag::Hash(hash) => {
                let block =
                    self.provider.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
                if let starknet::core::types::MaybePendingBlockWithTxHashes::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::Tag(_) => {
                return Err(Error::BlockTagNotAllowed);
            }
            _ => {}
        }

        let status = self.provider.get_block_transaction_count(block_id).await?;
        Ok(status)
    }

    pub async fn get_state_update(
        &self,
        block_id: BlockIdOrTag,
    ) -> Result<MaybePendingStateUpdate, Error> {
        match block_id {
            BlockIdOrTag::Number(num) if num > self.block => {
                return Err(Error::BlockOutOfRange);
            }
            BlockIdOrTag::Hash(hash) => {
                let block =
                    self.provider.get_block_with_tx_hashes(BlockIdOrTag::Hash(hash)).await?;
                if let starknet::core::types::MaybePendingBlockWithTxHashes::Block(b) = block {
                    if b.block_number > self.block {
                        return Err(Error::BlockOutOfRange);
                    }
                }
            }
            BlockIdOrTag::Tag(_) => {
                return Err(Error::BlockTagNotAllowed);
            }
            _ => {}
        }

        let state_update = self.provider.get_state_update(block_id).await?;
        Ok(state_update.into())
    }

    // NOTE(kariy): The reason why I don't just use EventFilter as a param, bcs i wanna make sure
    // the from/to blocks are not None. maybe should do the same for other methods that accept a
    // BlockId in some way?
    pub async fn get_events(
        &self,
        from: BlockNumber,
        to: BlockNumber,
        address: Option<ContractAddress>,
        keys: Option<Vec<Vec<Felt>>>,
        continuation_token: Option<String>,
        chunk_size: u64,
    ) -> Result<EventsPage, Error> {
        if from > self.block || to > self.block {
            return Err(Error::BlockOutOfRange);
        }

        let from_block = Some(BlockIdOrTag::Number(from));
        let to_block = Some(BlockIdOrTag::Number(to));
        let address = address.map(Felt::from);
        let filter = EventFilter { from_block, to_block, address, keys };

        let events = self.provider.get_events(filter, continuation_token, chunk_size).await?;

        Ok(events)
    }
}

impl From<Error> for StarknetApiError {
    fn from(value: Error) -> Self {
        match value {
            Error::Provider(provider_error) => provider_error.into(),
            Error::BlockOutOfRange => StarknetApiError::BlockNotFound,
            Error::BlockTagNotAllowed | Error::UnexpectedPendingData => {
                StarknetApiError::UnexpectedError { reason: value.to_string() }
            }
            Error::JsonRpc(json_rpc_error) => {
                StarknetApiError::UnexpectedError { reason: json_rpc_error.to_string() }
            }
            Error::KatanaProvider(provider_error) => provider_error.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use katana_primitives::felt;
    use katana_rpc_types::trie::ContractStorageKeys;
    use url::Url;

    use super::*;

    // const SEPOLIA_URL: &str = "https://api.cartridge.gg/x/starknet/sepolia/rpc/v0_8";
    const SEPOLIA_URL: &str = "https://rpc.starknet-testnet.lava.build:443";

    const FORK_BLOCK_NUMBER: BlockNumber = 268_471;

    #[tokio::test]
    async fn get_block_hash() {
        let url = Url::parse(SEPOLIA_URL).unwrap();
        let client = ForkedClient::new_http(url, FORK_BLOCK_NUMBER);

        // -----------------------------------------------------------------------
        // Block before the forked block

        // https://sepolia.voyager.online/block/0x4dfd88ba652622450c7758b49ac4a2f23b1fa8e6676297333ea9c97d0756c7a
        let hash = felt!("0x4dfd88ba652622450c7758b49ac4a2f23b1fa8e6676297333ea9c97d0756c7a");
        let number =
            client.get_block_number_by_hash(hash).await.expect("failed to get block number");
        assert_eq!(number, 268469);

        // -----------------------------------------------------------------------
        // Block after the forked block (exists only in the forked chain)

        // https://sepolia.voyager.online/block/0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e
        let hash = felt!("0x335a605f2c91873f8f830a6e5285e704caec18503ca28c18485ea6f682eb65e");
        let err = client.get_block_number_by_hash(hash).await.expect_err("should return an error");
        assert!(matches!(err, Error::BlockOutOfRange));
    }

    #[tokio::test]
    async fn test_get_storage_proof() {
        let external_client = JsonRpcClient::new(HttpTransport::new(
            Url::parse("https://api.cartridge.gg/x/starknet/sepolia").unwrap(),
        ));
        let block_number = external_client.block_number().await.unwrap();
        println!("Block number: {:?}", block_number);

        let url = Url::parse(SEPOLIA_URL).unwrap();
        let client = ForkedClient::new_http(url.clone(), block_number);

        // Create a simple StateUpdates object for testing
        use katana_primitives::state::StateUpdates;
        use katana_provider::providers::fork::ForkedProvider;
        use katana_provider::traits::trie::TrieWriter;
        use starknet::providers::jsonrpc::HttpTransport;
        use starknet::providers::JsonRpcClient;
        use std::collections::BTreeMap;

        let contract_address =
            felt!("0x06a4d4e8c1cc9785e125195a2f8bd4e5b0c7510b19f3e2dd63533524f5687e41").into();
        let class_hash =
            felt!("0x03d5de568b28042464214dfbe2ea0d7e22d162986bcdb9f56d691d22955a4c23");
        let storage_key = felt!("0x1").into();
        let storage_value = felt!("0x1").into();

        let mut state_updates = StateUpdates::default();

        // Add some sample data
        state_updates.deployed_contracts.insert(contract_address, class_hash);
        state_updates
            .storage_updates
            .insert(contract_address, BTreeMap::from([(storage_key, storage_value)]));
        state_updates.declared_classes.insert(class_hash, felt!("0x123").into());

        // Create the provider
        let rpc_provider = Arc::new(JsonRpcClient::new(HttpTransport::new(url.clone())));
        let forked_provider = ForkedProvider::new_ephemeral(
            katana_primitives::block::BlockHashOrNumber::Num(block_number),
            rpc_provider,
            url.clone(),
        );

        let state_root = forked_provider.compute_state_root(block_number, &state_updates).unwrap();
        println!("State root: {:?}", state_root);

        let external_state_update =
            external_client.get_state_update(BlockId::Tag(BlockTag::Latest)).await.unwrap();

        let external_state_root = match external_state_update {
            StarknetRsMaybePendingStateUpdate::Update(state_update) => {
                println!("External new_root: {:#x}", state_update.new_root);
                println!("External old_root: {:#x}", state_update.old_root);
                println!("External block_hash: {:#x}", state_update.block_hash);
                state_update.new_root
            }
            StarknetRsMaybePendingStateUpdate::PendingUpdate(pending) => {
                println!("Pending state update - no state root available");
                return;
            }
        };
    }
}

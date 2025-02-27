use std::{net::SocketAddr, sync::Arc};

use katana_chain_spec::{dev, ChainSpec};
use katana_core::backend::{storage::Database, Backend};
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_node::{
    config::{
        dev::DevConfig,
        rpc::{RpcConfig, RpcModulesList, DEFAULT_RPC_ADDR},
        sequencing::SequencingConfig,
        Config,
    },
    LaunchedNode,
};
use katana_primitives::{address, chain::ChainId, ContractAddress};
use katana_provider::BlockchainProvider;
pub use starknet::core::types::StarknetError;
use starknet::providers::{jsonrpc::HttpTransport, JsonRpcClient, Url};
pub use starknet::providers::{Provider, ProviderError};

pub struct TestNode {
    node: LaunchedNode,
}

impl TestNode {
    pub async fn new() -> Self {
        Self::new_with_config(test_config()).await
    }

    pub async fn new_with_block_time(block_time: u64) -> Self {
        let mut config = test_config();
        config.sequencing.block_time = Some(block_time);
        Self::new_with_config(config).await
    }

    pub async fn new_with_config(config: Config) -> Self {
        Self {
            node: katana_node::build(config)
                .await
                .expect("failed to build node")
                .launch()
                .await
                .expect("failed to launch node"),
        }
    }

    /// Returns the address of the node's RPC server.
    pub fn rpc_addr(&self) -> &SocketAddr {
        self.node.rpc.addr()
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory>> {
        &self.node.node.backend
    }

    /// Returns a reference to the blockchain provider.
    pub fn blockchain(&self) -> &BlockchainProvider<Box<dyn Database>> {
        self.node.node.backend.blockchain.provider()
    }

    /// Returns a reference to the launched node handle.
    pub fn handle(&self) -> &LaunchedNode {
        &self.node
    }

    pub fn starknet_provider(&self) -> JsonRpcClient<HttpTransport> {
        let url = Url::parse(&format!("http://{}", self.rpc_addr())).expect("failed to parse url");
        JsonRpcClient::new(HttpTransport::new(url))
    }
}

pub fn test_config() -> Config {
    let sequencing = SequencingConfig::default();
    let dev = DevConfig { fee: false, account_validation: true, fixed_gas_prices: None };

    let mut chain = dev::ChainSpec { id: ChainId::SEPOLIA, ..Default::default() };
    chain.genesis.sequencer_address = address!("0x1");

    let rpc = RpcConfig {
        port: 0,
        addr: DEFAULT_RPC_ADDR,
        apis: RpcModulesList::all(),
        max_proof_keys: Some(100),
        max_event_page_size: Some(100),
        ..Default::default()
    };

    Config { sequencing, rpc, dev, chain: ChainSpec::Dev(chain).into(), ..Default::default() }
}

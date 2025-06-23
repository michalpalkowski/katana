// #![cfg_attr(not(test), warn(unused_crate_dependencies))]

#[cfg(feature = "full-node")]
pub mod full;

pub mod config;
pub mod exit;

use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::{Context, Result};
use config::rpc::RpcModuleKind;
use config::Config;
use http::header::CONTENT_TYPE;
use http::Method;
use jsonrpsee::RpcModule;
use katana_chain_spec::{ChainSpec, SettlementLayer};
use katana_core::backend::gas_oracle::GasOracle;
use katana_core::backend::storage::Blockchain;
use katana_core::backend::Backend;
use katana_core::constants::{
    DEFAULT_ETH_L1_DATA_GAS_PRICE, DEFAULT_ETH_L1_GAS_PRICE, DEFAULT_STRK_L1_DATA_GAS_PRICE,
    DEFAULT_STRK_L1_GAS_PRICE,
};
use katana_core::env::BlockContextGenerator;
use katana_core::service::block_producer::BlockProducer;
use katana_db::mdbx::DbEnv;
use katana_executor::implementation::blockifier::cache::ClassCache;
use katana_executor::implementation::blockifier::BlockifierFactory;
use katana_executor::ExecutionFlags;
use katana_metrics::exporters::prometheus::PrometheusRecorder;
use katana_metrics::sys::DiskReporter;
use katana_metrics::{Report, Server as MetricsServer};
use katana_pool::ordering::FiFo;
use katana_pool::TxPool;
use katana_primitives::block::GasPrice;
use katana_primitives::env::{CfgEnv, FeeTokenAddressses};
#[cfg(feature = "cartridge")]
use katana_rpc::cartridge::CartridgeApi;
use katana_rpc::cors::Cors;
use katana_rpc::dev::DevApi;
use katana_rpc::starknet::forking::ForkedClient;
#[cfg(feature = "cartridge")]
use katana_rpc::starknet::PaymasterConfig;
use katana_rpc::starknet::{StarknetApi, StarknetApiConfig};
use katana_rpc::{RpcServer, RpcServerHandle};
#[cfg(feature = "cartridge")]
use katana_rpc_api::cartridge::CartridgeApiServer;
use katana_rpc_api::dev::DevApiServer;
use katana_rpc_api::starknet::{StarknetApiServer, StarknetTraceApiServer, StarknetWriteApiServer};
use katana_stage::Sequencing;
use katana_tasks::TaskManager;
use tracing::info;

use crate::exit::NodeStoppedFuture;

/// A node instance.
///
/// The struct contains the handle to all the components of the node.
#[must_use = "Node does nothing unless launched."]
#[derive(Debug)]
pub struct Node {
    config: Arc<Config>,
    pool: TxPool,
    db: DbEnv,
    rpc_server: RpcServer,
    task_manager: TaskManager,
    backend: Arc<Backend<BlockifierFactory>>,
    block_producer: BlockProducer<BlockifierFactory>,
}

impl Node {
    /// Build the node components from the given [`Config`].
    ///
    /// This returns a [`Node`] instance which can be launched with the all the necessary components
    /// configured.
    pub async fn build(config: Config) -> Result<Node> {
        let mut config = config;

        if config.metrics.is_some() {
            // Metrics recorder must be initialized before calling any of the metrics macros, in
            // order for it to be registered.
            let _ = PrometheusRecorder::install("katana")?;
        }

        // --- build executor factory

        let fee_token_addresses = match config.chain.as_ref() {
            ChainSpec::Dev(cs) => {
                FeeTokenAddressses { eth: cs.fee_contracts.eth, strk: cs.fee_contracts.strk }
            }
            ChainSpec::Rollup(cs) => {
                FeeTokenAddressses { eth: cs.fee_contract.strk, strk: cs.fee_contract.strk }
            }
        };

        let cfg_env = CfgEnv {
            fee_token_addresses,
            chain_id: config.chain.id(),
            invoke_tx_max_n_steps: config.execution.invocation_max_steps,
            validate_max_n_steps: config.execution.validation_max_steps,
            max_recursion_depth: config.execution.max_recursion_depth,
        };

        let execution_flags = ExecutionFlags::new()
            .with_account_validation(config.dev.account_validation)
            .with_fee(config.dev.fee);

        let executor_factory = {
            #[allow(unused_mut)]
            let mut class_cache = ClassCache::builder();

            #[cfg(feature = "native")]
            {
                info!(enabled = config.execution.compile_native, "Cairo native compilation");
                class_cache = class_cache.compile_native(config.execution.compile_native);
            }

            let global_class_cache = class_cache.build_global()?;
            // let global_class_cache = ClassCache::new()?;

            let factory = BlockifierFactory::new(
                cfg_env,
                execution_flags,
                config.sequencing.block_limits(),
                global_class_cache,
            );

            Arc::new(factory)
        };

        // --- build backend

        let (blockchain, db, forked_client) = if let Some(cfg) = &config.forking {
            let chain_spec = Arc::get_mut(&mut config.chain).expect("get mut Arc");

            let ChainSpec::Dev(chain_spec) = chain_spec else {
                return Err(anyhow::anyhow!("Forking is only supported in dev mode for now"));
            };

            let db = katana_db::init_ephemeral_db()?;
            let (bc, block_num) =
                Blockchain::new_from_forked(db.clone(), cfg.url.clone(), cfg.block, chain_spec)
                    .await?;

            // TODO: it'd bee nice if the client can be shared on both the rpc and forked backend
            // side
            let forked_client = ForkedClient::new_http(cfg.url.clone(), block_num);

            (bc, db, Some(forked_client))
        } else if let Some(db_path) = &config.db.dir {
            let db = katana_db::init_db(db_path)?;
            (Blockchain::new_with_db(db.clone()), db, None)
        } else {
            let db = katana_db::init_ephemeral_db()?;
            (Blockchain::new_with_db(db.clone()), db, None)
        };

        // --- build l1 gas oracle

        // Check if the user specify a fixed gas price in the dev config.
        let gas_oracle = if let Some(fixed_prices) = &config.dev.fixed_gas_prices {
            // Use fixed gas prices if provided in the configuration
            GasOracle::fixed(fixed_prices.gas_price.clone(), fixed_prices.data_gas_price.clone())
        } else if let Some(settlement) = config.chain.settlement() {
            match settlement {
                SettlementLayer::Starknet { .. } => GasOracle::sampled_starknet(),
                SettlementLayer::Ethereum { rpc_url, .. } => {
                    GasOracle::sampled_ethereum(rpc_url.clone())
                }
                SettlementLayer::Sovereign { .. } => GasOracle::fixed(
                    GasPrice { eth: DEFAULT_ETH_L1_GAS_PRICE, strk: DEFAULT_STRK_L1_GAS_PRICE },
                    GasPrice {
                        eth: DEFAULT_ETH_L1_DATA_GAS_PRICE,
                        strk: DEFAULT_STRK_L1_DATA_GAS_PRICE,
                    },
                ),
            }
        } else {
            // Use default fixed gas prices if no url and if no fixed prices are provided
            GasOracle::fixed(
                GasPrice { eth: DEFAULT_ETH_L1_GAS_PRICE, strk: DEFAULT_STRK_L1_GAS_PRICE },
                GasPrice {
                    eth: DEFAULT_ETH_L1_DATA_GAS_PRICE,
                    strk: DEFAULT_STRK_L1_DATA_GAS_PRICE,
                },
            )
        };

        let block_context_generator = BlockContextGenerator::default().into();
        let backend = Arc::new(Backend {
            gas_oracle,
            blockchain,
            executor_factory,
            block_context_generator,
            chain_spec: config.chain.clone(),
        });

        backend.init_genesis(config.forking.is_some()).context("failed to initialize genesis")?;

        // --- build block producer

        let block_producer =
            if config.sequencing.block_time.is_some() || config.sequencing.no_mining {
                if let Some(interval) = config.sequencing.block_time {
                    BlockProducer::interval(Arc::clone(&backend), interval)
                } else {
                    BlockProducer::on_demand(Arc::clone(&backend))
                }
            } else {
                BlockProducer::instant(Arc::clone(&backend))
            };

        // --- build transaction pool

        let validator = block_producer.validator();
        let pool = TxPool::new(validator.clone(), FiFo::new());

        // --- build rpc server

        let mut rpc_modules = RpcModule::new(());

        let cors = Cors::new()
        .allow_origins(config.rpc.cors_origins.clone())
        // Allow `POST` when accessing the resource
        .allow_methods([Method::POST, Method::GET])
        .allow_headers([CONTENT_TYPE, "argent-client".parse().unwrap(), "argent-version".parse().unwrap()]);

        #[cfg(feature = "cartridge")]
        let paymaster = if let Some(paymaster) = &config.paymaster {
            anyhow::ensure!(
                config.rpc.apis.contains(&RpcModuleKind::Cartridge),
                "Cartridge API should be enabled when paymaster is set"
            );

            let api = CartridgeApi::new(
                backend.clone(),
                block_producer.clone(),
                pool.clone(),
                paymaster.cartridge_api_url.clone(),
            );

            rpc_modules.merge(CartridgeApiServer::into_rpc(api))?;

            Some(PaymasterConfig { cartridge_api_url: paymaster.cartridge_api_url.clone() })
        } else {
            None
        };

        if config.rpc.apis.contains(&RpcModuleKind::Starknet) {
            let cfg = StarknetApiConfig {
                max_event_page_size: config.rpc.max_event_page_size,
                max_proof_keys: config.rpc.max_proof_keys,
                max_call_gas: config.rpc.max_call_gas,
                max_concurrent_estimate_fee_requests: config
                    .rpc
                    .max_concurrent_estimate_fee_requests,
                #[cfg(feature = "cartridge")]
                paymaster,
            };

            let api = if let Some(client) = forked_client {
                StarknetApi::new_forked(
                    backend.clone(),
                    pool.clone(),
                    block_producer.clone(),
                    client,
                    cfg,
                )
            } else {
                StarknetApi::new(backend.clone(), pool.clone(), Some(block_producer.clone()), cfg)
            };

            rpc_modules.merge(StarknetApiServer::into_rpc(api.clone()))?;
            rpc_modules.merge(StarknetWriteApiServer::into_rpc(api.clone()))?;
            rpc_modules.merge(StarknetTraceApiServer::into_rpc(api))?;
        }

        if config.rpc.apis.contains(&RpcModuleKind::Dev) {
            let api = DevApi::new(backend.clone(), block_producer.clone());
            rpc_modules.merge(DevApiServer::into_rpc(api))?;
        }

        #[allow(unused_mut)]
        let mut rpc_server =
            RpcServer::new().metrics(true).health_check(true).cors(cors).module(rpc_modules)?;

        #[cfg(feature = "explorer")]
        {
            rpc_server = rpc_server.explorer(config.rpc.explorer);
        }

        if let Some(timeout) = config.rpc.timeout {
            rpc_server = rpc_server.timeout(timeout);
        };

        if let Some(max_connections) = config.rpc.max_connections {
            rpc_server = rpc_server.max_connections(max_connections);
        }

        if let Some(max_request_body_size) = config.rpc.max_request_body_size {
            rpc_server = rpc_server.max_request_body_size(max_request_body_size);
        }

        if let Some(max_response_body_size) = config.rpc.max_response_body_size {
            rpc_server = rpc_server.max_response_body_size(max_response_body_size);
        }

        Ok(Node {
            db,
            pool,
            backend,
            rpc_server,
            block_producer,
            config: Arc::new(config),
            task_manager: TaskManager::current(),
        })
    }

    /// Start the node.
    ///
    /// This method will start all the node process, running them until the node is stopped.
    pub async fn launch(self) -> Result<LaunchedNode> {
        let chain = self.backend.chain_spec.id();
        info!(%chain, "Starting node.");

        // TODO: maybe move this to the build stage
        if let Some(ref cfg) = self.config.metrics {
            let db_metrics = Box::new(self.db.clone()) as Box<dyn Report>;
            let disk_metrics = Box::new(DiskReporter::new(self.db.path())?) as Box<dyn Report>;
            let reports: Vec<Box<dyn Report>> = vec![db_metrics, disk_metrics];

            let exporter = PrometheusRecorder::current().expect("qed; should exist at this point");
            let server = MetricsServer::new(exporter).with_process_metrics().with_reports(reports);

            let addr = cfg.socket_addr();
            self.task_manager.task_spawner().build_task().spawn(server.start(addr));
            info!(%addr, "Metrics server started.");
        }

        let pool = self.pool.clone();
        let backend = self.backend.clone();
        let block_producer = self.block_producer.clone();

        // --- build and run sequencing task

        let sequencing = Sequencing::new(
            pool.clone(),
            backend.clone(),
            self.task_manager.task_spawner(),
            block_producer.clone(),
            self.config.messaging.clone(),
        );

        self.task_manager
            .task_spawner()
            .build_task()
            .critical()
            .name("Sequencing")
            .spawn(sequencing.into_future());

        // --- start the rpc server

        let rpc_handle = self.rpc_server.start(self.config.rpc.socket_addr()).await?;

        // --- start the gas oracle worker task
        self.backend.gas_oracle.run_worker(self.task_manager.task_spawner());
        info!(target: "node", "Gas price oracle worker started.");

        Ok(LaunchedNode { node: self, rpc: rpc_handle })
    }

    /// Returns a reference to the node's database environment (if any).
    pub fn db(&self) -> &DbEnv {
        &self.db
    }

    pub fn backend(&self) -> &Arc<Backend<BlockifierFactory>> {
        &self.backend
    }

    /// Returns a reference to the node's transaction pool.
    pub fn pool(&self) -> &TxPool {
        &self.pool
    }

    /// Returns a reference to the node's JSON-RPC server.
    pub fn rpc(&self) -> &RpcServer {
        &self.rpc_server
    }

    /// Returns a reference to the node's configuration.
    pub fn config(&self) -> &Config {
        &self.config
    }
}

/// A handle to the launched node.
#[derive(Debug)]
pub struct LaunchedNode {
    node: Node,
    /// Handle to the rpc server.
    rpc: RpcServerHandle,
}

impl LaunchedNode {
    /// Returns a reference to the [`Node`] handle.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// Returns a reference to the rpc server handle.
    pub fn rpc(&self) -> &RpcServerHandle {
        &self.rpc
    }

    /// Stops the node.
    ///
    /// This will instruct the node to stop and wait until it has actually stop.
    pub async fn stop(&self) -> Result<()> {
        // TODO: wait for the rpc server to stop instead of just stopping it.
        self.rpc.stop()?;
        self.node.task_manager.shutdown().await;
        Ok(())
    }

    /// Returns a future which resolves only when the node has stopped.
    pub fn stopped(&self) -> NodeStoppedFuture<'_> {
        NodeStoppedFuture::new(self)
    }
}

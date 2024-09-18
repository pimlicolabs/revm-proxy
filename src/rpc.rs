use std::{collections::BTreeSet, sync::Arc};

use async_trait::async_trait;
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_evm::backend::{BlockchainDb, BlockchainDbMeta, SharedBackend};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use reth_primitives::{BlockId, Bytes, U256};
use reth_rpc_eth_types::{error::ensure_success, EthApiError};
use reth_rpc_types::{state::StateOverride, TransactionRequest};
use revm::{db::CacheDB, Evm};

#[rpc(server, namespace = "eth")]
pub trait PassthroughApi {
    #[method(name = "estimateGas")]
    async fn estimate_gas(
        &self,
        request: TransactionRequest,
        block_number: Option<BlockId>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<U256>;

    #[method(name = "call")]
    async fn call(
        &self,
        request: TransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
    ) -> RpcResult<Bytes>;
}

#[derive(Clone, Debug)]
pub struct PassthroughProxy {
    provider: Arc<RetryProvider>,
}

impl PassthroughProxy {
    pub fn init(endpoint: &str) -> eyre::Result<Self> {
        let provider = Arc::new(ProviderBuilder::new(endpoint).build()?);

        Ok(Self { provider })
    }

    async fn forward_request(&self, method: &str, params: Value) -> RpcResult<Value> {
        let request_json = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        let response = self
            .client
            .post(&self.provider_url)
            .json(&request_json)
            .send()
            .await
            .map_err(|e| JsonRpseeError::Custom(e.to_string()))?;

        let response_json: Value = response
            .json()
            .await
            .map_err(|e| JsonRpseeError::Custom(e.to_string()))?;

        if let Some(error) = response_json.get("error") {
            Err(JsonRpseeError::Custom(error.to_string()))
        } else if let Some(result) = response_json.get("result") {
            Ok(result.clone())
        } else {
            Err(JsonRpseeError::Custom(
                "Invalid response from provider".to_string(),
            ))
        }
    }
}

#[async_trait]
impl PassthroughApiServer for PassthroughProxy {
    async fn estimate_gas(
        &self,
        request: TransactionRequest,
        block_number: Option<BlockId>,
        state_override: Option<StateOverride>,
    ) -> RpcResult<U256> {
        let shared_backend = SharedBackend::spawn_backend_thread(
            self.provider.clone(),
            BlockchainDb::new(
                BlockchainDbMeta {
                    cfg_env: Default::default(),
                    block_env: Default::default(),
                    hosts: BTreeSet::from(["".to_string()]),
                },
                None,
            ),
            block_number,
        );

        let db = CacheDB::new(shared_backend);
        let mut evm = Evm::builder()
            .with_db(db)
            .modify_tx_env(|tx| {
                tx.caller = request.from.unwrap_or(tx.caller);
                tx.data = request.input.input.unwrap_or_default();
                tx.value = request.value.unwrap_or_default();
                tx.nonce = request.nonce;
                tx.transact_to = request.to.unwrap_or(tx.transact_to);
                tx.gas_limit = request
                    .gas
                    .map(|g| g.try_into().unwrap_or(u64::MAX))
                    .unwrap_or(tx.gas_limit);
                tx.gas_price = request
                    .gas_price
                    .map(|g| U256::from(g))
                    .unwrap_or(tx.gas_price);
            })
            .build();

        // Apply state overrides if provided
        if let Some(overrides) = state_override {
            for (address, account) in overrides.iter() {
                if let Some(balance) = account.balance {
                    let mut account_info = evm
                        .db_mut()
                        .load_account(*address)
                        .map_err(|e| EthApiError::EvmCustom(e.to_string()))?
                        .clone();
                    account_info.info.balance = balance;
                    evm.db_mut()
                        .insert_account_info(*address, account_info.info);
                }
                if let Some(storage) = &account.state_diff {
                    for (key, value) in storage {
                        let _ = evm.db_mut().insert_account_storage(
                            *address,
                            (*key).into(),
                            (*value).into(),
                        );
                    }
                }
            }
        }

        let res = evm
            .transact()
            .map_err(|e| EthApiError::EvmCustom(e.to_string()))?;

        let gas_used = res.result.gas_used();
        let _ = ensure_success(res.result)?;

        Ok(U256::from(gas_used))
    }

    async fn call(
        &self,
        request: TransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
    ) -> RpcResult<Bytes> {
        let shared_backend = SharedBackend::spawn_backend_thread(
            self.provider.clone(),
            BlockchainDb::new(
                BlockchainDbMeta {
                    cfg_env: Default::default(),
                    block_env: Default::default(),
                    hosts: BTreeSet::from(["".to_string()]),
                },
                None,
            ),
            block_number,
        );

        let db = CacheDB::new(shared_backend);
        let mut evm = Evm::builder()
            .with_db(db)
            .modify_tx_env(|tx| {
                tx.caller = request.from.unwrap_or(tx.caller);
                tx.data = request.input.data.unwrap_or_default();
                tx.value = request.value.unwrap_or_default();
                tx.nonce = request.nonce;
                tx.transact_to = request.to.unwrap_or(tx.transact_to);
                tx.gas_limit = request
                    .gas
                    .map(|g| g.try_into().unwrap_or(u64::MAX))
                    .unwrap_or(tx.gas_limit);
                tx.gas_price = request
                    .gas_price
                    .map(|g| U256::from(g))
                    .unwrap_or(tx.gas_price);
            })
            .build();

        // Apply state overrides if provided
        if let Some(overrides) = state_overrides {
            for (address, account) in overrides.iter() {
                if let Some(balance) = account.balance {
                    let mut account_info = evm
                        .db_mut()
                        .load_account(*address)
                        .map_err(|e| EthApiError::EvmCustom(e.to_string()))?
                        .clone();
                    account_info.info.balance = balance;
                    evm.db_mut()
                        .insert_account_info(*address, account_info.info);
                }
                if let Some(storage) = &account.state_diff {
                    for (key, value) in storage {
                        let _ = evm.db_mut().insert_account_storage(
                            *address,
                            (*key).into(),
                            (*value).into(),
                        );
                    }
                }
            }
        }

        let res = evm
            .transact()
            .map_err(|e| EthApiError::EvmCustom(e.to_string()))?;
        Ok(ensure_success(res.result)?)
    }
}

use std::{collections::BTreeSet, sync::Arc};

use alloy_provider::Provider;
use async_trait::async_trait;
use foundry_common::provider::{ProviderBuilder, RetryProvider};
use foundry_evm::backend::{BlockchainDb, BlockchainDbMeta, SharedBackend};
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use reth_primitives::{Address, BlockId, Bytes, B256, U256, U64};
use reth_rpc_eth_types::{error::ensure_success, EthApiError};
use reth_rpc_types::{
    state::StateOverride, AnyNetworkBlock, AnyTransactionReceipt, Block, BlockNumberOrTag,
    BlockTransactionsKind, Filter, Log, Transaction, TransactionRequest, WithOtherFields,
};
use revm::{db::CacheDB, Evm};

#[derive(Clone, Debug)]
pub struct PassthroughProxy {
    provider: Arc<RetryProvider>,
}

impl PassthroughProxy {
    pub fn init(endpoint: &str) -> eyre::Result<Self> {
        let provider = Arc::new(ProviderBuilder::new(endpoint).build()?);

        Ok(Self { provider })
    }
}

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

    /* Endpoints that will fallthrough and call using provider */
    #[method(name = "blockNumber")]
    async fn block_number(&self) -> RpcResult<U256>;

    #[method(name = "getBalance")]
    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256>;

    #[method(name = "maxPriorityFeePerGas")]
    async fn max_priority_fee_per_gas(&self) -> RpcResult<U256>;

    #[method(name = "chainId")]
    async fn chain_id(&self) -> RpcResult<U64>;

    #[method(name = "getTransactionCount")]
    async fn transaction_count(
        &self,
        address: Address,
        block_number: Option<BlockId>,
    ) -> RpcResult<U256>;

    #[method(name = "getLogs")]
    async fn logs(&self, filter: Filter) -> RpcResult<Vec<Log>>;

    #[method(name = "getBlockByNumber")]
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> RpcResult<Option<AnyNetworkBlock>>;

    #[method(name = "getTransactionReceipt")]
    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<AnyTransactionReceipt>>;

    #[method(name = "gasPrice")]
    async fn gas_price(&self) -> RpcResult<U256>;
}

#[async_trait]
impl PassthroughApiServer for PassthroughProxy {
    /* Fallthrough methods */
    async fn gas_price(&self) -> RpcResult<U256> {
        let gas_price = self
            .provider
            .get_gas_price()
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(U256::from(gas_price))
    }

    async fn transaction_receipt(&self, hash: B256) -> RpcResult<Option<AnyTransactionReceipt>> {
        let receipt = self
            .provider
            .get_transaction_receipt(hash)
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(receipt)
    }

    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> RpcResult<Option<AnyNetworkBlock>> {
        let block_tx_kind = if full {
            BlockTransactionsKind::Full
        } else {
            BlockTransactionsKind::Hashes
        };

        let block: Option<WithOtherFields<Block<WithOtherFields<Transaction>>>> = self
            .provider
            .get_block(number.into(), block_tx_kind)
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(block)
    }

    async fn logs(&self, filter: Filter) -> RpcResult<Vec<Log>> {
        let logs = self
            .provider
            .get_logs(&filter)
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(logs)
    }

    async fn transaction_count(
        &self,
        address: Address,
        block_number: Option<BlockId>,
    ) -> RpcResult<U256> {
        let nonce = self
            .provider
            .get_transaction_count(address)
            .block_id(block_number.unwrap_or_default())
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(U256::from(nonce))
    }

    async fn chain_id(&self) -> RpcResult<U64> {
        let chain_id = self
            .provider
            .get_chain_id()
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(U64::from(chain_id))
    }

    async fn max_priority_fee_per_gas(&self) -> RpcResult<U256> {
        let mpfpg = self
            .provider
            .get_max_priority_fee_per_gas()
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(U256::from(mpfpg))
    }

    async fn block_number(&self) -> RpcResult<U256> {
        let block_num = self
            .provider
            .get_block_number()
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(U256::from(block_num))
    }

    async fn balance(&self, address: Address, block_number: Option<BlockId>) -> RpcResult<U256> {
        let bal = self
            .provider
            .get_balance(address)
            .block_id(block_number.unwrap_or_default())
            .await
            .map_err(|e| EthApiError::InvalidParams(e.to_string()))?;

        Ok(bal)
    }

    /* Methods using REVM */

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

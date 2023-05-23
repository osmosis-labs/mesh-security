use cosmwasm_std::{ensure_eq, from_slice, Binary, Decimal, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::must_pay;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::local_staking_api::{self, LocalStakingApi, MaxSlashResponse};
use mesh_native_staking_proxy::native_staking_callback::{self, NativeStakingCallback};

use crate::error::ContractError;
use crate::types::{Config, ConfigResponse, OwnerByProxyResponse, ProxyByOwnerResponse, StakeMsg};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

// TODO: Hardcoded for now. Revisit for v1.
pub const MAX_SLASH_PERCENTAGE: u64 = 10;

pub struct NativeStakingContract<'a> {
    // TODO
    config: Item<'a, Config>,
}

#[contract]
#[error(ContractError)]
#[messages(local_staking_api as LocalStakingApi)]
#[messages(native_staking_callback as NativeStakingCallback)]
impl NativeStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// The caller of the instantiation will be the vault contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        proxy_code_id: u64,
    ) -> Result<Response, ContractError> {
        let config = Config {
            denom,
            proxy_code_id,
            vault: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        Ok(self.config.load(ctx.deps.storage)?)
    }

    #[msg(query)]
    fn proxy_by_owner(
        &self,
        _ctx: QueryCtx,
        owner: String,
    ) -> Result<ProxyByOwnerResponse, ContractError> {
        let _ = owner;
        todo!()
    }

    #[msg(query)]
    fn owner_by_proxy(
        &self,
        _ctx: QueryCtx,
        proxy: String,
    ) -> Result<OwnerByProxyResponse, ContractError> {
        let _ = proxy;
        todo!()
    }
}

#[contract]
impl LocalStakingApi for NativeStakingContract<'_> {
    type Error = ContractError;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        msg: Binary,
    ) -> Result<Response, Self::Error> {
        // only can be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.vault, ctx.info.sender, ContractError::Unauthorized {});

        // assert funds passed in
        let _paid = must_pay(&ctx.info, &cfg.denom)?;

        // parse message to find validator to stake on
        let StakeMsg { validator } = from_slice(&msg)?;
        let _ = validator;

        let _ = owner;

        // look up if there is a proxy to match
        // instantiate or call stake on existing
        todo!();
    }

    /// Returns the maximum percentage that can be slashed
    /// TODO: Any way to query this from the chain? Or we just pass in InstantiateMsg?
    #[msg(query)]
    fn max_slash(&self, _ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error> {
        Ok(MaxSlashResponse {
            max_slash: Decimal::percent(MAX_SLASH_PERCENTAGE),
        })
    }
}

#[contract]
impl NativeStakingCallback for NativeStakingContract<'_> {
    type Error = ContractError;

    /// This sends tokens back from the proxy to native-staking. (See info.funds)
    /// The native-staking contract can determine which user it belongs to via an internal Map.
    /// The native-staking contract will then send those tokens back to vault and release the claim.
    #[msg(exec)]
    fn release_proxy_stake(&self, _ctx: ExecCtx) -> Result<Response, Self::Error> {
        // ensure proper denom in info.funds
        // look up proxy address (info.sender) to account owner
        // send these tokens to vault contract, using release_local_stake method
        todo!()
    }
}

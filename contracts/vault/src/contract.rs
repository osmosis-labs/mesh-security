use cosmwasm_std::{Binary, Response, Uint128};
use cw2::set_contract_version;
use cw_storage_plus::Item;

use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::types::{BalanceResponse, Config, ConfigResponse};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VaultContract<'a> {
    // TODO
    config: Item<'a, Config>,
}

#[contract(error=ContractError)]
impl VaultContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        local_staking: String,
    ) -> Result<Response, ContractError> {
        let config = Config {
            denom,
            local_staking: ctx.deps.api.addr_validate(&local_staking)?,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[msg(exec)]
    fn bond(&self, _ctx: ExecCtx) -> Result<Response, ContractError> {
        todo!()
    }

    #[msg(exec)]
    fn unbond(&self, _ctx: ExecCtx, _amount: Uint128) -> Result<Response, ContractError> {
        todo!()
    }

    /// This assigns a claim of amount tokens to the remote contract, which can take some action with it
    #[msg(exec)]
    fn stake_remote(
        &self,
        _ctx: ExecCtx,
        // address of the contract to virtually stake on
        _contract: String,
        // amount to stake on that contract
        _amount: Uint128,
        // action to take with that stake
        _msg: Binary,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_remote(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        _owner: String,
        // amount to unstake on that contract
        _amount: Uint128,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    /// This sends actual tokens to the local staking contract
    #[msg(exec)]
    fn stake_local(
        &self,
        _ctx: ExecCtx,
        // amount to stake on that contract
        _amount: Uint128,
        // action to take with that stake
        _msg: Binary,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        _owner: String,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    #[msg(query)]
    fn balance(&self, _ctx: QueryCtx, _account: String) -> Result<BalanceResponse, ContractError> {
        todo!()
    }

    #[msg(query)]
    fn config(&self, _ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        todo!()
    }
}

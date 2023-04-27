use cosmwasm_std::{Binary, Response, Uint128};
use cw2::set_contract_version;
use cw_storage_plus::Item;

use mesh_apis::{LocalStakingApi, MaxSlashResponse};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::types::{ClaimsResponse, Config, ConfigResponse};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct NativeStakingContract<'a> {
    // TODO
    config: Item<'a, Config>,
}

#[contract(error=ContractError)]
impl NativeStakingContract<'_> {
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
        // Question: how to register vault_contract??
    ) -> Result<Response, ContractError> {
        let config = Config { denom, vault: None };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    /// unstakes the given amount from the given validator on behalf of the calling user.
    /// returns an error if the user doesn't have such stake.
    /// after unbonding period, it will allow the user to claim the tokens (returning to vault)
    #[msg(exec)]
    fn unstake(
        &self,
        _ctx: ExecCtx,
        _validator: String,
        _amount: Uint128,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    /// restakes the given amount from the one validator to another on behalf of the calling user.
    /// returns an error if the user doesn't have such stake.
    #[msg(exec)]
    fn restake(
        &self,
        _ctx: ExecCtx,
        _from_validator: String,
        _to_validator: String,
        _amount: Uint128,
    ) -> Result<Response, ContractError> {
        todo!()
    }

    /// If the caller has any delegations, withdraw all rewards from those delegations and
    /// send the tokens to the caller.
    #[msg(exec)]
    fn claim_rewards(&self, _ctx: ExecCtx) -> Result<Response, ContractError> {
        todo!()
    }

    // TODO: maybe a better name to differentiate from claim_rewards?
    // This is about finishing the unbonding process....

    /// releases any mature claims this user has from a previous unstake.
    /// this will go back to the vault via release_local
    /// error if the user doesn't have any mature claims
    #[msg(exec)]
    fn claim(&self, _ctx: ExecCtx) -> Result<Response, ContractError> {
        todo!()
    }

    #[msg(query)]
    fn config(&self, _ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        todo!()
    }

    /// Returns all open claims for this account, both mature and pending
    #[msg(query)]
    fn claims(&self, _ctx: QueryCtx, _account: String) -> Result<ClaimsResponse, ContractError> {
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
        _ctx: ExecCtx,
        _owner: String,
        // TODO: we parse this into
        _msg: Binary,
    ) -> Result<Response, Self::Error> {
        todo!();
    }

    /// Returns the maximum percentage that can be slashed
    #[msg(query)]
    fn max_slash(&self, _ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error> {
        todo!();
    }
}

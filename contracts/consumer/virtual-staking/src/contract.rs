use cosmwasm_std::{ensure_eq, entry_point, Coin, DepsMut, Env, Response, Uint128};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::nonpayable;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::virtual_staking_api::{self, SudoMsg, VirtualStakingApi};

use crate::error::ContractError;
use crate::types::{Config, ConfigResponse};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VirtualStakingContract<'a> {
    config: Item<'a, Config>,
    // Amount of tokens that have been requested to bond to a validator
    // (Sum of bond minus unbond requests). This will only update actual bond on epoch changes.
    bond_requests: Map<'a, &'a str, Uint128>,
    // TODO: current bond
}

#[contract]
#[error(ContractError)]
#[messages(virtual_staking_api as VirtualStakingApi)]
impl VirtualStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            bond_requests: Map::new("bond_requests"),
        }
    }

    /// The caller of the instantiation will be the converter contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let config = Config {
            denom,
            converter: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        Ok(self.config.load(ctx.deps.storage)?.into())
    }

    fn handle_epoch(&self, _deps: DepsMut, _env: Env) -> Result<Response, ContractError> {
        todo!();
    }
}

#[contract]
impl VirtualStakingApi for VirtualStakingContract<'_> {
    type Error = ContractError;

    /// Requests to bond tokens to a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance.
    /// If the max cap is 0, then this will immediately return an error.
    #[msg(exec)]
    fn bond(&self, ctx: ExecCtx, validator: String, amount: Coin) -> Result<Response, Self::Error> {
        nonpayable(&ctx.info)?;
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.converter, ContractError::Unauthorized); // only the converter can call this
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::WrongDenom(cfg.denom)
        );

        // Update the amount requested
        let mut bonded = self.bond_requests.load(ctx.deps.storage, &validator)?;
        bonded += amount.amount;
        self.bond_requests
            .save(ctx.deps.storage, &validator, &bonded)?;

        Ok(Response::new())
    }

    /// Requests to unbond tokens from a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance in addition to unbond.
    /// If the virtual staking contract doesn't have at least amont tokens staked to the given validator, this will return an error.
    #[msg(exec)]
    fn unbond(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, Self::Error> {
        nonpayable(&ctx.info)?;
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.converter, ContractError::Unauthorized); // only the converter can call this
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::WrongDenom(cfg.denom)
        );

        // Update the amount requested
        let bonded = self.bond_requests.load(ctx.deps.storage, &validator)?;
        let bonded = bonded
            .checked_sub(amount.amount)
            .map_err(|_| ContractError::InsufficientBond(validator.clone(), amount.amount))?;
        self.bond_requests
            .save(ctx.deps.storage, &validator, &bonded)?;

        Ok(Response::new())
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::Rebalance {} => VirtualStakingContract::new().handle_epoch(deps, env),
    }
}

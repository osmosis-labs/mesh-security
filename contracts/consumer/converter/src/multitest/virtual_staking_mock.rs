use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Addr, Coin, Response, StdError, StdResult, Uint128};

use cw_storage_plus::{Item, Map};
use cw_utils::{nonpayable, PaymentError};
use mesh_apis::virtual_staking_api::{self, VirtualStakingApi};
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,
    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: Addr,
}

#[derive(thiserror::Error, Debug)]
pub enum ContractError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("Unauthorized")]
    Unauthorized,

    #[error("Wrong denom. Cannot stake {0}")]
    WrongDenom(String),
}

/// This is a stub implementation of the local staking proxy contract, for test purposes only.
/// When proper local staking proxy contract is available, this should be replaced in multitests
pub struct VirtualStakingMock<'a> {
    config: Item<'a, Config>,
    stake: Map<'a, &'a str, Uint128>,
}

#[contract]
#[error(ContractError)]
#[messages(virtual_staking_api as VirtualStakingApi)]
impl VirtualStakingMock<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            stake: Map::new("stake"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(&self, ctx: InstantiateCtx) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let denom = ctx.deps.querier.query_bonded_denom()?;
        let config = Config {
            denom,
            converter: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        let denom = cfg.denom;
        let converter = cfg.converter.into_string();
        Ok(ConfigResponse { denom, converter })
    }

    #[msg(query)]
    fn stake(&self, ctx: QueryCtx, validator: String) -> Result<StakeResponse, ContractError> {
        let stake = self
            .stake
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();
        Ok(StakeResponse { stake })
    }

    #[msg(query)]
    fn all_stake(&self, ctx: QueryCtx) -> Result<AllStakeResponse, ContractError> {
        let stakes = self
            .stake
            .range(ctx.deps.storage, None, None, cosmwasm_std::Order::Ascending)
            .collect::<StdResult<_>>()?;
        Ok(AllStakeResponse { stakes })
    }
}

#[cw_serde]
pub struct StakeResponse {
    pub stake: Uint128,
}

#[cw_serde]
pub struct AllStakeResponse {
    pub stakes: Vec<(String, Uint128)>,
}

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,
    pub converter: String,
}

#[contract]
#[messages(virtual_staking_api as VirtualStakingApi)]
impl VirtualStakingApi for VirtualStakingMock<'_> {
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
        self.stake
            .update::<_, ContractError>(ctx.deps.storage, &validator, |old| {
                Ok(old.unwrap_or_default() + amount.amount)
            })?;

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
        self.stake
            .update::<_, ContractError>(ctx.deps.storage, &validator, |old| {
                Ok(old.unwrap_or_default() - amount.amount)
            })?;

        Ok(Response::new())
    }
}

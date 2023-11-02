use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Addr, Coin, Response, StdError, StdResult, Uint128};
use std::cmp::min;

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

    #[error("Empty validators list")]
    NoValidators {},

    #[error("Virtual staking {0} has not enough delegated funds: {1}")]
    InsufficientDelegations(String, Uint128),
}

/// This is a stub implementation of the virtual staking contract, for test purposes only.
/// When proper virtual staking contract is available, this should be replaced in multitests
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
    /// If the virtual staking contract doesn't have at least amount tokens staked to the given validator, this will return an error.
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

    /// Requests to unbond and burn tokens from a lists of validators (one or more). This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance in addition to unbond.
    /// If the virtual staking contract doesn't have at least amount tokens staked over the given validators, this will return an error.
    #[msg(exec)]
    fn burn(
        &self,
        ctx: ExecCtx,
        validators: Vec<String>,
        amount: Coin,
    ) -> Result<Response, Self::Error> {
        nonpayable(&ctx.info)?;
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.converter, ContractError::Unauthorized);
        // only the converter can call this
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::WrongDenom(cfg.denom)
        );

        // Error if no validators
        if validators.is_empty() {
            return Err(ContractError::NoValidators {});
        }

        let mut unstaked = 0;
        let proportional_amount = Uint128::new(amount.amount.u128() / validators.len() as u128);
        for validator in &validators {
            // Checks that validator has `proportional_amount` delegated. Adjust accordingly if not.
            self.stake
                .update::<_, ContractError>(ctx.deps.storage, &validator, |old| {
                    let delegated_amount = old.unwrap_or_default();
                    let unstake_amount = min(delegated_amount, proportional_amount);
                    unstaked += unstake_amount.u128();
                    Ok(delegated_amount - unstake_amount)
                })?;
        }
        // Adjust possible rounding issues
        if unstaked < amount.amount.u128() {
            // Look for the first validator that has enough stake, and unstake it from there
            let unstake_amount = Uint128::new(amount.amount.u128() - unstaked);
            for validator in &validators {
                let delegated_amount = self
                    .stake
                    .may_load(ctx.deps.storage, &validator)?
                    .unwrap_or_default();
                if delegated_amount >= unstake_amount {
                    self.stake.save(
                        ctx.deps.storage,
                        &validator,
                        &(delegated_amount - unstake_amount),
                    )?;
                    unstaked += unstake_amount.u128();
                    break;
                }
            }
        }
        // Bail if we still don't have enough stake
        if unstaked < amount.amount.u128() {
            return Err(ContractError::InsufficientDelegations(
                ctx.env.contract.address.to_string(),
                amount.amount,
            ));
        }

        Ok(Response::new())
    }
}

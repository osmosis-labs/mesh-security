use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, Addr, Coin, Response, StdError, StdResult, Uint128, Validator};

use cw_storage_plus::{Item, Map};
use cw_utils::{nonpayable, PaymentError};
use mesh_apis::virtual_staking_api::{self, ValidatorSlash, VirtualStakingApi};
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, SudoCtx};

use crate::contract::custom;

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
#[sv::error(ContractError)]
#[sv::messages(virtual_staking_api as VirtualStakingApi)]
#[sv::custom(query=custom::ConverterQuery, msg=custom::ConverterMsg)]
impl VirtualStakingMock<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            stake: Map::new("stake"),
        }
    }

    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx<custom::ConverterQuery>,
    ) -> Result<custom::Response, ContractError> {
        nonpayable(&ctx.info)?;
        let denom = ctx.deps.querier.query_bonded_denom()?;
        let config = Config {
            denom,
            converter: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[sv::msg(query)]
    fn config(
        &self,
        ctx: QueryCtx<custom::ConverterQuery>,
    ) -> Result<ConfigResponse, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        let denom = cfg.denom;
        let converter = cfg.converter.into_string();
        Ok(ConfigResponse { denom, converter })
    }

    #[sv::msg(query)]
    fn stake(
        &self,
        ctx: QueryCtx<custom::ConverterQuery>,
        validator: String,
    ) -> Result<StakeResponse, ContractError> {
        let stake = self
            .stake
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();
        Ok(StakeResponse { stake })
    }

    #[sv::msg(query)]
    fn all_stake(
        &self,
        ctx: QueryCtx<custom::ConverterQuery>,
    ) -> Result<AllStakeResponse, ContractError> {
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

impl VirtualStakingApi for VirtualStakingMock<'_> {
    type Error = ContractError;
    type ExecC = custom::ConverterMsg;
    type QueryC = custom::ConverterQuery;

    /// Requests to bond tokens to a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance.
    /// If the max cap is 0, then this will immediately return an error.
    fn bond(
        &self,
        ctx: ExecCtx<Self::QueryC>,
        _delegator: String,
        validator: String,
        amount: Coin,
    ) -> Result<Response<Self::ExecC>, Self::Error> {
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
    fn unbond(
        &self,
        ctx: ExecCtx<Self::QueryC>,
        _delegator: String,
        validator: String,
        amount: Coin,
    ) -> Result<Response<Self::ExecC>, Self::Error> {
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
    fn burn(
        &self,
        ctx: ExecCtx<Self::QueryC>,
        validators: Vec<String>,
        amount: Coin,
    ) -> Result<Response<Self::ExecC>, Self::Error> {
        nonpayable(&ctx.info)?;
        let cfg = self.config.load(ctx.deps.storage)?;
        // only the converter can call this
        ensure_eq!(ctx.info.sender, cfg.converter, ContractError::Unauthorized);
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::WrongDenom(cfg.denom)
        );

        let mut stakes = vec![];
        for validator in validators {
            let stake = self
                .stake
                .may_load(ctx.deps.storage, &validator)?
                .unwrap_or_default()
                .u128();
            if stake != 0 {
                stakes.push((validator, stake));
            }
        }

        // Error if no delegations
        if stakes.is_empty() {
            return Err(ContractError::InsufficientDelegations(
                ctx.env.contract.address.to_string(),
                amount.amount,
            ));
        }

        let (burned, burns) = mesh_burn::distribute_burn(stakes.as_slice(), amount.amount.u128());

        // Bail if we still don't have enough stake
        if burned < amount.amount.u128() {
            return Err(ContractError::InsufficientDelegations(
                ctx.env.contract.address.to_string(),
                amount.amount,
            ));
        }

        // Update stake
        for (validator, burn_amount) in burns {
            self.stake
                .update::<_, ContractError>(ctx.deps.storage, validator, |old| {
                    Ok(old.unwrap_or_default() - Uint128::new(burn_amount))
                })?;
        }

        Ok(Response::new())
    }

    fn internal_unbond(
        &self,
        _ctx:ExecCtx<Self::QueryC>,
        _delegator:String,
        _validator:String,
        _amount:Coin
    ) -> Result<Response<Self::ExecC> ,Self::Error> {
        unimplemented!()
    }

    /// SudoMsg::HandleEpoch{} should be called once per epoch by the sdk (in EndBlock).
    /// It allows the virtual staking contract to bond or unbond any pending requests, as well
    /// as to perform a rebalance if needed (over the max cap).
    ///
    /// It should also withdraw all pending rewards here, and send them to the converter contract.
    fn handle_epoch(
        &self,
        _ctx: SudoCtx<Self::QueryC>,
    ) -> Result<Response<Self::ExecC>, Self::Error> {
        unimplemented!()
    }

    /// SudoMsg::ValsetUpdate{} should be called every time there's a validator set update:
    ///  - Addition of a new validator to the active validator set.
    ///  - Temporary removal of a validator from the active set. (i.e. `unbonded` state).
    ///  - Update of validator data.
    ///  - Temporary removal of a validator from the active set due to jailing. Implies slashing.
    ///  - Addition of an existing validator to the active validator set.
    ///  - Permanent removal (i.e. tombstoning) of a validator from the active set. Implies slashing
    fn handle_valset_update(
        &self,
        _ctx: SudoCtx<Self::QueryC>,
        _additions: Option<Vec<Validator>>,
        _removals: Option<Vec<String>>,
        _updated: Option<Vec<Validator>>,
        _jailed: Option<Vec<String>>,
        _unjailed: Option<Vec<String>>,
        _tombstoned: Option<Vec<String>>,
        _slashed: Option<Vec<ValidatorSlash>>,
    ) -> Result<Response<Self::ExecC>, Self::Error> {
        unimplemented!()
    }
}

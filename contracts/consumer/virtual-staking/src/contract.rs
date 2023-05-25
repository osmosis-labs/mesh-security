use cosmwasm_std::{ensure_eq, entry_point, Coin, DepsMut, Env, Response, Uint128};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::nonpayable;
use mesh_bindings::{TokenQuerier, VirtualStakeCustomMsg, VirtualStakeCustomQuery};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::virtual_staking_api::{self, SudoMsg, VirtualStakingApi};

use crate::error::ContractError;
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct VirtualStakingContract<'a> {
    pub config: Item<'a, Config>,
    /// Amount of tokens that have been requested to bond to a validator
    /// (Sum of bond minus unbond requests). This will only update actual bond on epoch changes.
    /// Note: Validator addresses are stored as strings, as they are different format than Addr
    //
    // Optimization: I keep bond_requests as a Map, as most interactions are bond/unbond requests
    // (from IBC) which touch one. And then we only range over it once per epoch (in handle_epoch).
    pub bond_requests: Map<'a, &'a str, Uint128>,
    /// This is how much was bonded last time (validator, amount) pairs
    // `bonded` could be a Map like `bond_requests`, but the only time we use it is to read / write the entire list in bulk (in handle_epoch),
    // never accessing one element. Reading 100 elements in an Item is much cheaper than ranging over a Map with 100 entries.
    pub bonded: Item<'a, Vec<(String, Uint128)>>,
}

#[contract]
#[error(ContractError)]
#[messages(virtual_staking_api as VirtualStakingApi)]
impl VirtualStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            bond_requests: Map::new("bond_requests"),
            bonded: Item::new("bonded"),
        }
    }

    /// The caller of the instantiation will be the converter contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
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

    fn handle_epoch(
        &self,
        deps: DepsMut<VirtualStakeCustomQuery>,
        env: Env,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        let bond =
            TokenQuerier::new(&deps.querier).bond_status(env.contract.address.to_string())?;
        // TODO: return error if it is 0 (or maybe just return early?)

        // withdraw rewards - or make that a separate message?
        let _bonded = self.bonded.load(deps.storage)?;
        // TODO: also make this configurable compile time for easier system tests

        // calculate what the delegations should be when we are done
        let requests: Vec<(String, Uint128)> = self
            .bond_requests
            .range(deps.storage, None, None, cosmwasm_std::Order::Ascending)
            .collect::<Result<_, _>>()?;
        let total_request: Uint128 = requests.iter().map(|(_, v)| v).sum();
        if total_request > bond.cap.amount {
            // TODO: normalize the list of requests to match the cap
            todo!();
        }

        // TODO: Look at existing list and calculate differences for bond/unbond calls

        // Save the new values
        self.bonded.save(deps.storage, &requests)?;
        Ok(Response::new())
    }
}

#[contract]
#[messages(virtual_staking_api as VirtualStakingApi)]
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
pub fn sudo(
    deps: DepsMut<VirtualStakeCustomQuery>,
    env: Env,
    msg: SudoMsg,
) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
    match msg {
        SudoMsg::Rebalance {} => VirtualStakingContract::new().handle_epoch(deps, env),
    }
}

use std::cmp::Ordering;
use std::collections::HashMap;

use cosmwasm_std::{
    coin, ensure_eq, entry_point, Coin, CosmosMsg, DepsMut, DistributionMsg, Env, Response,
    StakingMsg, SubMsg, Uint128,
};
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

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
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
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let config = Config {
            denom,
            converter: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        // initialize this to no one, so no issue when reading for the first time
        self.bonded.save(ctx.deps.storage, &vec![])?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        Ok(self.config.load(ctx.deps.storage)?.into())
    }

    /**
     * This is called once per epoch to withdraw all rewards and rebalance the bonded tokens.
     * Note: the current implementation may (repeatedly) fail if any validator was slashed or fell out
     * of the active set.
     *
     * The basic logic for calculating rebalance is:
     * 1. Get all bond requests
     * 2. Sum the total amount
     * 3. If the sum <= max_cap then use collected requests as is
     * 4. If the sum > max_cap,
     *   a. calculate multiplier Decimal(max_cap / sum)
     *   b. multiply every element of the collected requests in place.
     * 5. Find diff between collected (normalized) requests and last bonding amounts (which go up, which down).
     * 6. Transform diff into unbond and bond requests, sorting so all unbond happen first
     */
    fn handle_epoch(
        &self,
        deps: DepsMut<VirtualStakeCustomQuery>,
        env: Env,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        // withdraw rewards
        let bonded = self.bonded.load(deps.storage)?;
        let withdraw = withdraw_reward_msgs(&bonded);
        let resp = Response::new().add_submessages(withdraw);

        let bond =
            TokenQuerier::new(&deps.querier).bond_status(env.contract.address.to_string())?;
        let max_cap = bond.cap.amount;
        // If 0 max cap, then we assume all tokens were force unbonded already, and just return the withdraw rewards
        // call and set bonded to empty
        // TODO: verify this behavior with SDK module (otherwise we send unbond message)
        if max_cap.is_zero() {
            self.bonded.save(deps.storage, &Vec::new())?;
            return Ok(resp);
        }

        // calculate what the delegations should be when we are done
        let mut requests: Vec<(String, Uint128)> = self
            .bond_requests
            .range(deps.storage, None, None, cosmwasm_std::Order::Ascending)
            .collect::<Result<_, _>>()?;
        let total_requested: Uint128 = requests.iter().map(|(_, v)| v).sum();
        if total_requested > max_cap {
            for (_, v) in requests.iter_mut() {
                *v = (*v * max_cap) / total_requested;
            }
        }

        // Load current bonded and save the future values
        let current = self.bonded.load(deps.storage)?;
        self.bonded.save(deps.storage, &requests)?;

        // Compare these two to make bond/unbond calls as needed
        let config = self.config.load(deps.storage)?;
        let rebalance = calculate_rebalance(current, requests, &config.denom);
        let resp = resp.add_messages(rebalance);

        Ok(resp)
    }
}

fn calculate_rebalance(
    current: Vec<(String, Uint128)>,
    desired: Vec<(String, Uint128)>,
    denom: &str,
) -> Vec<CosmosMsg<VirtualStakeCustomMsg>> {
    let mut desired: HashMap<_, _> = desired.into_iter().collect();

    // this will handle adjustments to all current validators
    let mut msgs = vec![];
    for (validator, prev) in current {
        let next = desired.remove(&validator).unwrap_or_else(Uint128::zero);
        match next.cmp(&prev) {
            Ordering::Less => {
                let unbond = prev - next;
                let amount = coin(unbond.u128(), denom);
                msgs.push(StakingMsg::Undelegate { validator, amount }.into())
            }
            Ordering::Greater => {
                let bond = next - prev;
                let amount = coin(bond.u128(), denom);
                msgs.push(StakingMsg::Delegate { validator, amount }.into())
            }
            Ordering::Equal => {}
        }
    }

    // any new validators in the desired list need to be bonded
    for (validator, bond) in desired {
        let amount = coin(bond.u128(), denom);
        msgs.push(StakingMsg::Delegate { validator, amount }.into())
    }

    msgs
}

// TODO: each submsg should have a callback to this contract to trigger sending the rewards to converter
// This will be done in a future PR
fn withdraw_reward_msgs(bonded: &[(String, Uint128)]) -> Vec<SubMsg<VirtualStakeCustomMsg>> {
    bonded
        .iter()
        .map(|(validator, _)| {
            SubMsg::new(DistributionMsg::WithdrawDelegatorReward {
                validator: validator.clone(),
            })
        })
        .collect()
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
        let mut bonded = self
            .bond_requests
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();
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

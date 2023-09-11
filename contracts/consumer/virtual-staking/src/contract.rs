use std::cmp::Ordering;
use std::collections::BTreeMap;

use cosmwasm_std::{
    coin, ensure_eq, entry_point, to_binary, Coin, CosmosMsg, CustomQuery, DepsMut,
    DistributionMsg, Env, Event, Reply, Response, StdResult, SubMsg, SubMsgResponse, Uint128,
    Validator, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::nonpayable;
use mesh_apis::converter_api;
use mesh_bindings::{
    TokenQuerier, VirtualStakeCustomMsg, VirtualStakeCustomQuery, VirtualStakeMsg,
};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
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
// #[sv::override_entry_point(sudo=sudo(SudoMsg))] // Disabled because lack of custom query support
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
    pub fn instantiate(&self, ctx: InstantiateCtx) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let denom = ctx.deps.querier.query_bonded_denom()?;
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
        mut deps: DepsMut<VirtualStakeCustomQuery>,
        env: Env,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        // withdraw rewards
        let bonded = self.bonded.load(deps.storage)?;
        let withdraw = withdraw_reward_msgs(deps.branch(), &bonded);
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

    /**
     * This is called every time there's a change of the active validator set.
     *
     */
    fn handle_valset_update(
        &self,
        deps: DepsMut<VirtualStakeCustomQuery>,
        additions: &[Validator],
        removals: &[Validator],
        tombstones: &[Validator],
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        // TODO: Store/process removals (and additions) locally, so that they are filtered out from
        // the `bonded` list
        let _ = removals;

        // Send additions to the converter. Removals are considered non-permanent and ignored
        let cfg = self.config.load(deps.storage)?;
        let msg = converter_api::ExecMsg::ValsetUpdate {
            additions: additions.to_vec(),
            tombstones: tombstones.to_vec(),
        };
        let msg = WasmMsg::Execute {
            contract_addr: cfg.converter.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };
        let resp = Response::new().add_message(msg);
        Ok(resp)
    }

    #[msg(reply)]
    fn reply(&self, ctx: ReplyCtx, reply: Reply) -> Result<Response, ContractError> {
        match (reply.id, reply.result.into_result()) {
            (REPLY_REWARDS_ID, Ok(result)) => self.reply_rewards(ctx.deps, ctx.env, result),
            (REPLY_REWARDS_ID, Err(e)) => {
                // We need to pop the REWARD_TARGETS so it doesn't get out of sync
                let target = pop_target(ctx.deps)?;
                // Ignore errors, so the rest doesn't fail, but report them.
                let evt = Event::new("rewards_error")
                    .add_attribute("error", e)
                    .add_attribute("target", target);
                Ok(Response::new().add_event(evt))
            }
            (id, _) => Err(ContractError::InvalidReplyId(id)),
        }
    }

    /// This is called on each successful withdrawal
    fn reply_rewards(
        &self,
        mut deps: DepsMut,
        env: Env,
        _reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        // Find the validator to assign the rewards to
        let target = pop_target(deps.branch())?;

        // Find all the tokens received here (consider it rewards to that validator)
        let cfg = self.config.load(deps.storage)?;
        let reward = deps
            .querier
            .query_balance(env.contract.address, cfg.denom)?;
        if reward.amount.is_zero() {
            return Ok(Response::new());
        }

        // Send them to the converter, assigned to proper validator
        let msg = converter_api::ExecMsg::DistributeReward { validator: target };
        let msg = WasmMsg::Execute {
            contract_addr: cfg.converter.into_string(),
            msg: to_binary(&msg)?,
            funds: vec![reward],
        };
        let resp = Response::new().add_message(msg);
        Ok(resp)
    }
}

fn pop_target(deps: DepsMut) -> StdResult<String> {
    let mut targets = REWARD_TARGETS.load(deps.storage)?;
    let target = targets.pop().unwrap();
    REWARD_TARGETS.save(deps.storage, &targets)?;
    Ok(target)
}

fn calculate_rebalance(
    current: Vec<(String, Uint128)>,
    desired: Vec<(String, Uint128)>,
    denom: &str,
) -> Vec<CosmosMsg<VirtualStakeCustomMsg>> {
    let mut desired: BTreeMap<_, _> = desired.into_iter().collect();

    // this will handle adjustments to all current validators
    let mut msgs = vec![];
    for (validator, prev) in current {
        let next = desired.remove(&validator).unwrap_or_else(Uint128::zero);
        match next.cmp(&prev) {
            Ordering::Less => {
                let unbond = prev - next;
                let amount = coin(unbond.u128(), denom);
                msgs.push(VirtualStakeMsg::Unbond { validator, amount }.into())
            }
            Ordering::Greater => {
                let bond = next - prev;
                let amount = coin(bond.u128(), denom);
                msgs.push(VirtualStakeMsg::Bond { validator, amount }.into())
            }
            Ordering::Equal => {}
        }
    }

    // any new validators in the desired list need to be bonded
    for (validator, bond) in desired {
        let amount = coin(bond.u128(), denom);
        msgs.push(VirtualStakeMsg::Bond { validator, amount }.into())
    }

    msgs
}

const REWARD_TARGETS: Item<Vec<String>> = Item::new("reward_targets");
const REPLY_REWARDS_ID: u64 = 1;

/// Each of these messages will need to get a callback to distribute received rewards to the proper validator
/// To manage that, we store a queue of validators in an Item, one item for each SubMsg, and read them in reply.
/// Look at reply implementation that uses the value set here.
fn withdraw_reward_msgs<T: CustomQuery>(
    deps: DepsMut<T>,
    bonded: &[(String, Uint128)],
) -> Vec<SubMsg<VirtualStakeCustomMsg>> {
    // We need to make a list, so we know where to send the rewards later (reversed, so we can pop off the top)
    let targets = bonded
        .iter()
        .map(|(v, _)| v.clone())
        .rev()
        .collect::<Vec<_>>();
    REWARD_TARGETS.save(deps.storage, &targets).unwrap();

    bonded
        .iter()
        .map(|(validator, _)| {
            SubMsg::reply_always(
                DistributionMsg::WithdrawDelegatorReward {
                    validator: validator.clone(),
                },
                REPLY_REWARDS_ID,
            )
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
        SudoMsg::ValsetUpdate {
            additions,
            removals,
            tombstones,
        } => VirtualStakingContract::new().handle_valset_update(
            deps,
            &additions,
            &removals,
            &tombstones,
        ),
    }
}

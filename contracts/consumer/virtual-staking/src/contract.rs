use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use cosmwasm_std::{
    coin, ensure_eq, to_json_binary, Coin, CosmosMsg, CustomQuery, DepsMut, DistributionMsg, Env,
    Event, Reply, Response, StdResult, Storage, SubMsg, Uint128, Validator, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::nonpayable;
use mesh_apis::converter_api::{self, RewardInfo, ValidatorSlashInfo};
use mesh_bindings::{
    TokenQuerier, VirtualStakeCustomMsg, VirtualStakeCustomQuery, VirtualStakeMsg,
};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx, SudoCtx};
use sylvia::{contract, schemars};

use mesh_apis::virtual_staking_api::{self, ValidatorSlash, VirtualStakingApi};

use crate::error::ContractError;
use crate::msg::{AllStakeResponse, ConfigResponse, StakeResponse};
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
    /// This is what validators have been requested to be slashed.
    // The list will be cleared after processing in `handle_epoch`.
    pub slash_requests: Item<'a, Vec<ValidatorSlash>>,
    /// This is what validators are inactive because of tombstoning, jailing or removal (unbonded).
    // `inactive` could be a Map like `bond_requests`, but the only time we use it is to read / write the entire list in bulk (in handle_epoch),
    // never accessing one element. Reading 100 elements in an Item is much cheaper than ranging over a Map with 100 entries.
    pub inactive: Item<'a, Vec<String>>,
    /// Amount of tokens that have been burned from a validator.
    /// This is just for accounting / tracking reasons, as token "burning" is being implemented as unbonding,
    /// and there's no real need to discount the burned amount in this contract.
    burned: Map<'a, &'a str, u128>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
#[sv::messages(virtual_staking_api as VirtualStakingApi)]
// FIXME: how to handle custom messages for sudo?
#[sv::custom(query=VirtualStakeCustomQuery, msg=VirtualStakeCustomMsg)]
// #[sv::override_entry_point(sudo=sudo(SudoMsg))] // Disabled because lack of custom query support
impl VirtualStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            bond_requests: Map::new("bond_requests"),
            bonded: Item::new("bonded"),
            slash_requests: Item::new("slashed"),
            inactive: Item::new("inactive"),
            burned: Map::new("burned"),
        }
    }

    /// The caller of the instantiation will be the converter contract
    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx<VirtualStakeCustomQuery>,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        nonpayable(&ctx.info)?;
        let denom = ctx.deps.querier.query_bonded_denom()?;
        let config = Config {
            denom,
            converter: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        // initialize these to no one, so no issue when reading for the first time
        self.bonded.save(ctx.deps.storage, &vec![])?;
        self.slash_requests.save(ctx.deps.storage, &vec![])?;
        self.inactive.save(ctx.deps.storage, &vec![])?;
        VALIDATOR_REWARDS_BATCH.init(ctx.deps.storage)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[sv::msg(query)]
    fn config(
        &self,
        ctx: QueryCtx<VirtualStakeCustomQuery>,
    ) -> Result<ConfigResponse, ContractError> {
        Ok(self.config.load(ctx.deps.storage)?.into())
    }

    fn adjust_slashings(
        &self,
        deps: DepsMut<VirtualStakeCustomQuery>,
        current: &mut [(String, Uint128)],
        slash: &[ValidatorSlash],
    ) -> StdResult<()> {
        let slashes: HashMap<String, ValidatorSlash> =
            HashMap::from_iter(slash.iter().map(|s| (s.address.clone(), s.clone())));

        // this is linear over current, but better than turn it in to a map
        for (validator, prev) in current {
            match slashes.get(validator) {
                None => continue,
                Some(s) => {
                    // Just deduct the slash amount passed by the chain
                    *prev -= s.slash_amount;
                    // Apply to request as well (to avoid unbonding msg)
                    let mut request = self
                        .bond_requests
                        .may_load(deps.storage, validator)?
                        .unwrap_or_default();
                    request = request.saturating_sub(s.slash_amount);
                    self.bond_requests.save(deps.storage, validator, &request)?;
                }
            }
        }
        Ok(())
    }

    #[sv::msg(reply)]
    fn reply(
        &self,
        ctx: ReplyCtx<VirtualStakeCustomQuery>,
        reply: Reply,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        match (reply.id, reply.result.into_result()) {
            (REPLY_REWARDS_ID, Ok(_)) => self.reply_rewards(ctx.deps, ctx.env),
            (REPLY_REWARDS_ID, Err(e)) => {
                // We need to pop the REWARD_TARGETS so it doesn't get out of sync
                let (target, _) = pop_target(ctx.deps)?;
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
        mut deps: DepsMut<VirtualStakeCustomQuery>,
        env: Env,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        const BATCH: ValidatorRewardsBatch = VALIDATOR_REWARDS_BATCH;

        // Find the validator to assign the new reward to
        let (target, finished) = pop_target(deps.branch())?;

        // Find all the tokens received until now
        let cfg = self.config.load(deps.storage)?;
        let total = deps
            .querier
            .query_balance(env.contract.address, &cfg.denom)?
            .amount;

        if total.is_zero() {
            // This helps avoid unnecessary store read/writes and messages sent
            // when there's definitely nothing to do
            return Ok(Response::new());
        }

        let new_reward_amount = total - BATCH.total(deps.storage)?;

        let all_rewards = |storage| {
            if !new_reward_amount.is_zero() {
                let reward_info = RewardInfo {
                    validator: target,
                    reward: new_reward_amount,
                };

                if total == new_reward_amount {
                    // we don't always have to read from the store to know what the
                    // rewards queue looks like
                    Ok(vec![reward_info])
                } else {
                    let mut rewards = BATCH.rewards(storage)?;
                    rewards.push(reward_info);
                    Ok(rewards)
                }
            } else {
                BATCH.rewards(storage)
            }
        };

        if finished {
            let all_rewards = all_rewards(deps.storage)?;
            BATCH.wipe(deps.storage)?;

            let msg = converter_api::sv::ExecMsg::DistributeRewards {
                payments: all_rewards,
            };
            let msg = WasmMsg::Execute {
                contract_addr: cfg.converter.into_string(),
                msg: to_json_binary(&msg)?,
                funds: vec![coin(total.into(), cfg.denom)],
            };
            Ok(Response::new().add_message(msg))
        } else if !new_reward_amount.is_zero() {
            // since we're not sending out the rewards batch yet, we need to persist
            // the update for future calls
            let all_rewards = all_rewards(deps.storage)?;
            BATCH.set_rewards(deps.storage, &all_rewards)?;
            BATCH.set_total(deps.storage, &total)?;
            Ok(Response::new())
        } else {
            Ok(Response::new())
        }
    }

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[sv::msg(exec)]
    pub fn test_handle_epoch(
        &self,
        ctx: ExecCtx<VirtualStakeCustomQuery>,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            let ExecCtx { mut deps, .. } = ctx;
            let requests: Vec<(String, Uint128)> = self
                .bond_requests
                .range(
                    deps.as_ref().storage,
                    None,
                    None,
                    cosmwasm_std::Order::Ascending,
                )
                .collect::<Result<_, _>>()?;

            // Save the future values
            self.bonded.save(deps.branch().storage, &requests)?;
            Ok(Response::new())
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            Err(ContractError::Unauthorized)
        }
    }

    #[sv::msg(query)]
    fn get_stake(
        &self,
        ctx: QueryCtx<VirtualStakeCustomQuery>,
        validator: String,
    ) -> Result<StakeResponse, ContractError> {
        let bonded = self.bonded.load(ctx.deps.storage)?;

        if let Some(stake) = bonded.iter().find(|&x| x.0 == validator) {
            Ok(StakeResponse { stake: stake.1 })
        } else {
            Ok(StakeResponse {
                stake: Uint128::zero(),
            })
        }
    }

    #[sv::msg(query)]
    fn get_all_stake(
        &self,
        ctx: QueryCtx<VirtualStakeCustomQuery>,
    ) -> Result<AllStakeResponse, ContractError> {
        let stakes = self.bonded.load(ctx.deps.storage)?;
        Ok(AllStakeResponse { stakes })
    }
}

/// Returns a tuple containing the reward target and a boolean value
/// specifying if we've exhausted the list.
fn pop_target(deps: DepsMut<VirtualStakeCustomQuery>) -> StdResult<(String, bool)> {
    let mut targets = REWARD_TARGETS.load(deps.storage)?;
    let target = targets.pop().unwrap();
    REWARD_TARGETS.save(deps.storage, &targets)?;
    Ok((target, targets.is_empty()))
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
const VALIDATOR_REWARDS_BATCH: ValidatorRewardsBatch = ValidatorRewardsBatch::new();
const REPLY_REWARDS_ID: u64 = 1;

struct ValidatorRewardsBatch<'a> {
    rewards: Item<'a, Vec<RewardInfo>>,
    total: Item<'a, Uint128>,
}

impl<'a> ValidatorRewardsBatch<'a> {
    const fn new() -> Self {
        Self {
            rewards: Item::new("validator_rewards_batch"),
            total: Item::new("validator_rewards_batch_total"),
        }
    }

    fn init(&self, store: &mut dyn Storage) -> StdResult<()> {
        self.rewards.save(store, &vec![])?;
        self.total.save(store, &Uint128::zero())?;

        Ok(())
    }

    fn rewards(&self, store: &mut dyn Storage) -> StdResult<Vec<RewardInfo>> {
        self.rewards.load(store)
    }

    /// The total of all rewards currently in the batch.
    fn total(&self, store: &mut dyn Storage) -> StdResult<Uint128> {
        self.total.load(store)
    }

    fn set_rewards(&self, store: &mut dyn Storage, rewards: &Vec<RewardInfo>) -> StdResult<()> {
        self.rewards.save(store, rewards)
    }

    /// The total of all rewards currently in the batch.
    fn set_total(&self, store: &mut dyn Storage, total: &Uint128) -> StdResult<()> {
        self.total.save(store, total)
    }

    fn wipe(&self, store: &mut dyn Storage) -> StdResult<()> {
        self.init(store)
    }
}

/// Each of these messages will need to get a callback to distribute received rewards to the proper validator
/// To manage that, we store a queue of validators in an Item, one item for each SubMsg, and read them in reply.
/// Look at reply implementation that uses the value set here.
fn withdraw_reward_msgs<T: CustomQuery>(
    deps: DepsMut<T>,
    bonded: &[(String, Uint128)],
    inactive: &[String],
) -> Vec<SubMsg<VirtualStakeCustomMsg>> {
    // Filter out inactive validators
    let inactive = inactive.iter().collect::<HashSet<_>>();
    let bonded = bonded
        .iter()
        .filter(|(validator, amount)| !amount.is_zero() && !inactive.contains(validator))
        .collect::<Vec<_>>();
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

impl VirtualStakingApi for VirtualStakingContract<'_> {
    type Error = ContractError;
    type QueryC = VirtualStakeCustomQuery;
    type ExecC = VirtualStakeCustomMsg;

    /// Requests to bond tokens to a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance.
    /// If the max cap is 0, then this will immediately return an error.
    fn bond(
        &self,
        ctx: ExecCtx<VirtualStakeCustomQuery>,
        validator: String,
        amount: Coin,
    ) -> Result<Response<VirtualStakeCustomMsg>, Self::Error> {
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
    fn unbond(
        &self,
        ctx: ExecCtx<VirtualStakeCustomQuery>,
        validator: String,
        amount: Coin,
    ) -> Result<Response<VirtualStakeCustomMsg>, Self::Error> {
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

    /// Requests to unbond and burn tokens from a list of validators. Unbonding will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance in addition to unbond.
    /// If the virtual staking contract doesn't have at least amount tokens staked over the given validators, this will return an error.
    fn burn(
        &self,
        ctx: ExecCtx<VirtualStakeCustomQuery>,
        validators: Vec<String>,
        amount: Coin,
    ) -> Result<Response<VirtualStakeCustomMsg>, Self::Error> {
        nonpayable(&ctx.info)?;
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.converter, ContractError::Unauthorized); // only the converter can call this
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::WrongDenom(cfg.denom)
        );
        let mut bonds = vec![];
        for validator in validators {
            let stake = self
                .bond_requests
                .may_load(ctx.deps.storage, &validator)?
                .unwrap_or_default()
                .u128();
            if stake != 0 {
                bonds.push((validator, stake));
            }
        }

        // Error if no delegations
        if bonds.is_empty() {
            return Err(ContractError::InsufficientDelegations(
                ctx.env.contract.address.to_string(),
                amount.amount,
            ));
        }

        let (burned, burns) = mesh_burn::distribute_burn(bonds.as_slice(), amount.amount.u128());

        for (validator, burn_amount) in burns {
            // Update bond requests
            self.bond_requests
                .update::<_, ContractError>(ctx.deps.storage, validator, |old| {
                    Ok(old.unwrap_or_default() - Uint128::new(burn_amount))
                })?;
            // Accounting trick to avoid burning stake
            self.burned.update(ctx.deps.storage, validator, |old| {
                Ok::<_, ContractError>(old.unwrap_or_default() + burn_amount)
            })?;
        }

        // Bail if we still don't have enough stake
        if burned < amount.amount.u128() {
            return Err(ContractError::InsufficientDelegations(
                ctx.env.contract.address.to_string(),
                amount.amount,
            ));
        }

        Ok(Response::new())
    }

    // FIXME: need to handle custom message types and queries
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
        ctx: SudoCtx<VirtualStakeCustomQuery>,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        let SudoCtx { mut deps, env, .. } = ctx;

        // withdraw rewards
        let bonded = self.bonded.load(deps.storage)?;
        let inactive = self.inactive.load(deps.storage)?;
        let withdraw = withdraw_reward_msgs(deps.branch(), &bonded, &inactive);
        let mut resp = Response::new().add_submessages(withdraw);

        let bond =
            TokenQuerier::new(&deps.querier).bond_status(env.contract.address.to_string())?;
        let max_cap = bond.cap.amount;
        // If 0 max cap, then we assume all tokens were force unbonded already, and just return the withdraw rewards
        // call and set bonded to empty
        // TODO: verify this behavior with SDK module (otherwise we send unbond message)
        if max_cap.is_zero() {
            self.bonded.save(deps.storage, &vec![])?;
            return Ok(resp);
        }

        let config = self.config.load(deps.storage)?;
        // Make current bonded mutable
        let mut current = bonded;
        // Process slashes due to tombstoning (unbonded) or jailing, over bond_requests and current
        let slash = self.slash_requests.load(deps.storage)?;
        if !slash.is_empty() {
            self.adjust_slashings(deps.branch(), &mut current, &slash)?;
            // Update inactive list. Defensive, as it should already been updated in handle_valset_update, due to removals
            self.inactive.update(deps.branch().storage, |mut old| {
                old.extend_from_slice(&slash.iter().map(|v| v.address.clone()).collect::<Vec<_>>());
                old.dedup();
                Ok::<_, ContractError>(old)
            })?;
            // Clear up slash requests
            self.slash_requests.save(deps.storage, &vec![])?;
        }

        // calculate what the delegations should be when we are done
        let mut requests: Vec<(String, Uint128)> = self
            .bond_requests
            .range(
                deps.as_ref().storage,
                None,
                None,
                cosmwasm_std::Order::Ascending,
            )
            .collect::<Result<_, _>>()?;
        let total_requested: Uint128 = requests.iter().map(|(_, v)| v).sum();
        if total_requested > max_cap {
            for (_, v) in requests.iter_mut() {
                *v = (*v * max_cap) / total_requested;
            }
        }

        // Save the future values
        self.bonded.save(deps.branch().storage, &requests)?;

        // Compare these two to make bond/unbond calls as needed
        let rebalance = calculate_rebalance(current, requests, &config.denom);
        resp = resp.add_messages(rebalance);

        Ok(resp)
    }

    // FIXME: need to handle custom message types and queries
    /**
     * This is called every time there's a change of the active validator set.
     *
     */
    #[allow(clippy::too_many_arguments)]
    fn handle_valset_update(
        &self,
        ctx: SudoCtx<VirtualStakeCustomQuery>,
        additions: Option<Vec<Validator>>,
        removals: Option<Vec<String>>,
        updated: Option<Vec<Validator>>,
        jailed: Option<Vec<String>>,
        unjailed: Option<Vec<String>>,
        tombstoned: Option<Vec<String>>,
        slashed: Option<Vec<ValidatorSlash>>,
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        let SudoCtx { deps, .. } = ctx;

        let additions = &additions.unwrap_or_default();
        let removals = &removals.unwrap_or_default();
        let updated = &updated.unwrap_or_default();
        let jailed = &jailed.unwrap_or_default();
        let unjailed = &unjailed.unwrap_or_default();
        let tombstoned = &tombstoned.unwrap_or_default();
        let slashed = &slashed.unwrap_or_default();

        // Account for slashed validators. Will be processed in handle_epoch
        if !slashed.is_empty() {
            self.slash_requests.update(deps.storage, |mut old| {
                old.extend_from_slice(slashed);
                Ok::<_, ContractError>(old)
            })?;
        }

        // Update inactive list.
        // We ignore `unjailed` as it's not clear they make the validator active again or not.
        if !removals.is_empty() || !additions.is_empty() {
            self.inactive.update(deps.storage, |mut old| {
                // Add removals
                old.extend_from_slice(removals);
                // Filter additions
                old.retain(|v| !additions.iter().any(|a| a.address == *v));
                old.dedup();
                Ok::<_, ContractError>(old)
            })?;
        }
        // Send all updates to the converter.
        let cfg = self.config.load(deps.storage)?;
        let msg = converter_api::sv::ExecMsg::ValsetUpdate {
            additions: additions.to_vec(),
            removals: removals.to_vec(),
            updated: updated.to_vec(),
            jailed: jailed.to_vec(),
            unjailed: unjailed.to_vec(),
            tombstoned: tombstoned.to_vec(),
            slashed: slashed
                .iter()
                .map(|s| ValidatorSlashInfo {
                    address: s.address.clone(),
                    infraction_height: s.infraction_height,
                    infraction_time: s.infraction_time,
                    power: s.power,
                    slash_amount: coin(s.slash_amount.u128(), cfg.denom.clone()),
                    slash_ratio: s.slash_ratio.clone(),
                })
                .collect(),
        };
        let msg = WasmMsg::Execute {
            contract_addr: cfg.converter.to_string(),
            msg: to_json_binary(&msg)?,
            funds: vec![],
        };
        let resp = Response::new().add_message(msg);
        Ok(resp)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Ref, RefCell},
        marker::PhantomData,
        rc::Rc,
    };

    use cosmwasm_std::{
        coins, from_json,
        testing::{mock_env, mock_info, MockApi, MockQuerier, MockStorage},
        Decimal,
    };
    use mesh_bindings::{BondStatusResponse, SlashRatioResponse};

    use super::*;

    type OwnedDeps<C = VirtualStakeCustomQuery> =
        cosmwasm_std::OwnedDeps<MockStorage, MockApi, MockQuerier<C>, C>;
    type DepsMut<'a, C = VirtualStakeCustomQuery> = cosmwasm_std::DepsMut<'a, C>;

    #[test]
    fn no_bond_with_zero_cap() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());

        knobs.bond_status.update_cap(0u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_no_bonding()
            .assert_rewards(&[]);
    }

    #[test]
    fn simple_bond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(10u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom))])
            .assert_rewards(&[]);
    }

    #[test]
    fn simple_bond2() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(10u128);
        contract.quick_bond(deps.as_mut(), "val1", 6);
        contract.quick_bond(deps.as_mut(), "val2", 4);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (6u128, &denom)), ("val2", (4u128, &denom))])
            .assert_rewards(&[]);
    }

    /// If there isn't enough cap, bonds get proportionally rebalanced so that their sum
    /// doesn't exceed the cap.
    #[test]
    fn bond_rebalance() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(5u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract.quick_bond(deps.as_mut(), "val2", 40);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (1u128, &denom)), ("val2", (4u128, &denom))])
            .assert_rewards(&[]);
    }

    #[test]
    fn unbond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(10u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom))])
            .assert_rewards(&[]);

        contract.quick_unbond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_unbond(&[("val1", (5u128, &denom))])
            .assert_rewards(&["val1"]);
    }

    #[test]
    fn burn() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(10u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom))])
            .assert_rewards(&[]);

        contract.quick_burn(deps.as_mut(), &["val1"], 5).unwrap();
        contract
            .hit_epoch(deps.as_mut())
            .assert_unbond(&[("val1", (5u128, &denom))])
            .assert_rewards(&["val1"]);
    }

    #[test]
    fn multiple_validators_burn() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract.quick_bond(deps.as_mut(), "val2", 20);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom)), ("val2", (20u128, &denom))])
            .assert_rewards(&[]);

        contract
            .quick_burn(deps.as_mut(), &["val1", "val2"], 5)
            .unwrap();
        contract
            .hit_epoch(deps.as_mut())
            .assert_unbond(&[("val1", (3u128, &denom)), ("val2", (2u128, &denom))])
            .assert_rewards(&["val1", "val2"]);
    }

    #[test]
    fn some_unbonded_validators_burn() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract.quick_bond(deps.as_mut(), "val2", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom)), ("val2", (10u128, &denom))])
            .assert_rewards(&[]);

        contract
            .quick_burn(deps.as_mut(), &["val1", "val2"], 15)
            .unwrap();
        contract
            .hit_epoch(deps.as_mut())
            .assert_unbond(&[("val1", (5u128, &denom)), ("val2", (10u128, &denom))])
            .assert_rewards(&["val1", "val2"]);
    }

    #[test]
    fn unbonded_validators_burn() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract.quick_bond(deps.as_mut(), "val2", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom)), ("val2", (10u128, &denom))])
            .assert_rewards(&[]);

        let res = contract.quick_burn(deps.as_mut(), &["val1", "val2"], 20);
        assert!(matches!(
            res.unwrap_err(),
            ContractError::InsufficientDelegations { .. }
        ));
    }

    #[test]
    fn validator_jail_unjail() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract.quick_bond(deps.as_mut(), "val2", 20);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (10u128, &denom)), ("val2", (20u128, &denom))])
            .assert_rewards(&[]);

        // val1 is being jailed and slashed for being offline
        contract.jail(deps.as_mut(), "val1", Decimal::percent(10), Uint128::one());

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after jailing
            .assert_unbond(&[]) // No unbond msgs after jailing
            .assert_rewards(&["val2"]); // Rewards are not gathered anymore because of the removal

        // Check that the bonded amounts of val1 have been slashed for being offline (10%)
        // Val2 is unaffected.
        // TODO: Check that the amounts have been slashed for being offline on-chain (needs mt slashing support)
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(
            bonded,
            [
                ("val1".to_string(), Uint128::new(9)),
                ("val2".to_string(), Uint128::new(20))
            ]
        );

        // Subsequent rewards msgs are removed while validator is jailed / inactive
        contract.hit_epoch(deps.as_mut()).assert_rewards(&["val2"]);

        // Unjail does nothing
        contract.unjail(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&["val2"]);
        // Removal does nothing (already removed)
        contract.remove_val(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&["val2"]);

        // Addition restores the validator to the active set
        contract.add_val(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&["val1", "val2"]);
    }

    #[test]
    fn validator_jail_pending_bond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (10u128, &denom))])
            .assert_rewards(&[]);

        // Val1 is bonding some more
        contract.quick_bond(deps.as_mut(), "val1", 20);

        // And it's being jailed at the same time
        contract.jail(deps.as_mut(), "val1", Decimal::percent(10), Uint128::one());

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (20u128, &denom))]) // Tombstoned validators can still bond
            .assert_unbond(&[]) // No unbond msgs after jailing
            .assert_rewards(&[]); // Rewards are not gathered anymore because of jailing implying removal

        // Check that the non-slashed amounts of val1 have been bonded
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(bonded, [("val1".to_string(), Uint128::new(29)),]);
    }

    #[test]
    fn validator_jail_pending_unbond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (10u128, &denom))])
            .assert_rewards(&[]);

        // Val1 is unbonding
        contract.quick_unbond(deps.as_mut(), "val1", 10);

        // And it's being jailed at the same time
        contract.jail(deps.as_mut(), "val1", Decimal::percent(10), Uint128::one());

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after jailing
            .assert_unbond(&[("val1", (9u128, &denom))]) // Only unbond non-slashed amount
            .assert_rewards(&[]); // Rewards are not gathered anymore because of the removal

        // Check that the non-slashed amounts of val1 have been unbonded
        // FIXME: Remove / filter zero amounts
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(bonded, [("val1".to_string(), Uint128::new(0)),]);

        // No rewards gatherings after the first one for the jailed validator
        contract.hit_epoch(deps.as_mut()).assert_rewards(&[]);

        // Unjail over unbonded has no effect
        contract.unjail(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&[]);

        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(bonded, [("val1".to_string(), Uint128::new(0)),]);
    }

    #[test]
    fn validator_remove() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(10u128);
        contract.quick_bond(deps.as_mut(), "val1", 5);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (5u128, &denom))])
            .assert_rewards(&[]);

        contract.remove_val(deps.as_mut(), "val1");
        // Subsequent rewards msgs are removed while validator is inactive
        contract.hit_epoch(deps.as_mut()).assert_rewards(&[]);

        // Addition restores the validator to the active set
        contract.add_val(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&["val1"]); // Rewards are being gathered again
    }

    #[test]
    fn validator_tombstoning() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 20);
        contract.quick_bond(deps.as_mut(), "val2", 20);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (20u128, &denom)), ("val2", (20u128, &denom))])
            .assert_rewards(&[]);

        // Val1 is being tombstoned
        contract.tombstone(deps.as_mut(), "val1", Decimal::percent(25), Uint128::new(5));
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after tombstoning
            .assert_unbond(&[]) // No unbond msgs after tombstoning
            .assert_rewards(&["val1", "val2"]); // Last rewards msgs after tombstoning

        // Check that the bonded amounts of val1 have been slashed for double sign (25%)
        // Val2 is unaffected.
        // TODO: Check that the amounts have been slashed for double sign on-chain (needs mt slashing / tombstoning support)
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(
            bonded,
            [
                ("val1".to_string(), Uint128::new(15)),
                ("val2".to_string(), Uint128::new(20))
            ]
        );

        // Subsequent rewards msgs are removed after validator is tombstoned
        contract.hit_epoch(deps.as_mut()).assert_rewards(&["val2"]);
    }

    #[test]
    fn validator_tombstoning_pending_bond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (10u128, &denom))])
            .assert_rewards(&[]);

        // Val1 is bonding some more
        contract.quick_bond(deps.as_mut(), "val1", 20);

        // And it's being tombstoned at the same time
        contract.tombstone(deps.as_mut(), "val1", Decimal::percent(25), Uint128::new(2));

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (20, &denom))]) // FIXME?: Tombstoned validators can still bond
            .assert_unbond(&[])
            .assert_rewards(&["val1"]); // Rewards are still being gathered

        // Check that the previously bonded amounts of val1 have been slashed for double sign (25%)
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(
            bonded,
            [
                ("val1".to_string(), Uint128::new(8 + 20)), // Due to rounding up
            ]
        );

        // Subsequent rewards msgs are removed after validator is tombstoned
        contract.hit_epoch(deps.as_mut()).assert_rewards(&[]);
    }

    #[test]
    fn validator_tombstoning_pending_unbond() {
        let (mut deps, knobs) = mock_dependencies();

        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());
        let denom = contract.config.load(&deps.storage).unwrap().denom;

        knobs.bond_status.update_cap(100u128);
        contract.quick_bond(deps.as_mut(), "val1", 10);
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[("val1", (10u128, &denom))])
            .assert_rewards(&[]);

        // Val1 is unbonding
        contract.quick_unbond(deps.as_mut(), "val1", 10);

        // And it's being tombstoned at the same time
        contract.tombstone(deps.as_mut(), "val1", Decimal::percent(25), Uint128::new(2));

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after jailing
            .assert_unbond(&[("val1", (8u128, &denom))]) // Unbond adjusted for double sign slashing
            .assert_rewards(&["val1"]); // Rewards are still being gathered

        // Check that bonded accounting has been adjusted
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        //  FIXME: Remove zero amounts
        assert_eq!(bonded, [("val1".to_string(), Uint128::new(0)),]);

        // Subsequent rewards msgs are removed after validator is tombstoned
        contract.hit_epoch(deps.as_mut()).assert_rewards(&[]);
    }

    #[test]
    fn reply_rewards() {
        let (mut deps, _) = mock_dependencies();
        let contract = VirtualStakingContract::new();

        contract.quick_inst(deps.as_mut());
        set_reward_targets(&mut deps.storage, &["val3", "val2", "val1"]);

        contract.push_rewards(&mut deps, 10).assert_empty();
        contract.push_rewards(&mut deps, 20).assert_empty();
        contract
            .push_rewards(&mut deps, 30)
            .assert_eq(&[("val1", 10), ("val2", 20), ("val3", 30)]);
    }

    #[test]
    fn reply_rewards_twice() {
        let (mut deps, _) = mock_dependencies();
        let contract = VirtualStakingContract::new();
        contract.quick_inst(deps.as_mut());

        set_reward_targets(&mut deps.storage, &["val2", "val1"]);
        contract.push_rewards(&mut deps, 20).assert_empty();
        contract
            .push_rewards(&mut deps, 30)
            .assert_eq(&[("val1", 20), ("val2", 30)]);

        set_reward_targets(&mut deps.storage, &["val"]);
        contract
            .push_rewards(&mut deps, 30)
            .assert_eq(&[("val", 30)]);
    }

    #[test]
    fn reply_rewards_all_zero() {
        let (mut deps, _) = mock_dependencies();
        let contract = VirtualStakingContract::new();

        contract.quick_inst(deps.as_mut());
        set_reward_targets(&mut deps.storage, &["val3", "val2", "val1"]);

        for _ in 0..3 {
            contract.push_rewards(&mut deps, 0).assert_empty();
        }
    }

    #[test]
    fn reply_rewards_mid_push_is_zero() {
        let (mut deps, _) = mock_dependencies();
        let contract = VirtualStakingContract::new();

        contract.quick_inst(deps.as_mut());
        set_reward_targets(&mut deps.storage, &["val3", "val2", "val1"]);

        contract.push_rewards(&mut deps, 20).assert_empty();
        contract.push_rewards(&mut deps, 0).assert_empty();
        contract
            .push_rewards(&mut deps, 10)
            .assert_eq(&[("val1", 20), ("val3", 10)]);
    }

    #[test]
    fn reply_rewards_last_push_is_zero() {
        let (mut deps, _) = mock_dependencies();
        let contract = VirtualStakingContract::new();

        contract.quick_inst(deps.as_mut());
        set_reward_targets(&mut deps.storage, &["val3", "val2", "val1"]);

        contract.push_rewards(&mut deps, 20).assert_empty();
        contract.push_rewards(&mut deps, 10).assert_empty();
        contract
            .push_rewards(&mut deps, 0)
            .assert_eq(&[("val1", 20), ("val2", 10)]);
    }

    fn mock_dependencies() -> (OwnedDeps, StakingKnobs) {
        let bond_status = MockBondStatus::new(BondStatusResponse {
            cap: coin(0, "DOES NOT MATTER"),
            delegated: coin(0, "DOES NOT MATTER"),
        });
        let slash_ratio = MockSlashRatio::new(SlashRatioResponse {
            slash_fraction_downtime: "0.1".to_string(),
            slash_fraction_double_sign: "0.25".to_string(),
        });

        let handler = {
            let bs_copy = bond_status.clone();
            move |msg: &_| {
                let VirtualStakeCustomQuery::VirtualStake(msg) = msg;
                match msg {
                    mesh_bindings::VirtualStakeQuery::BondStatus { .. } => {
                        cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(
                            to_json_binary(&*bs_copy.borrow()).unwrap(),
                        ))
                    }
                    mesh_bindings::VirtualStakeQuery::SlashRatio {} => {
                        cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(
                            to_json_binary(&*slash_ratio.borrow()).unwrap(),
                        ))
                    }
                }
            }
        };

        (
            OwnedDeps {
                storage: MockStorage::default(),
                api: MockApi::default(),
                querier: MockQuerier::new(&[]).with_custom_handler(handler),
                custom_query_type: PhantomData,
            },
            StakingKnobs { bond_status },
        )
    }

    struct StakingKnobs {
        bond_status: MockBondStatus,
    }

    #[derive(Clone)]
    struct MockBondStatus(Rc<RefCell<BondStatusResponse>>);

    impl MockBondStatus {
        fn new(res: BondStatusResponse) -> Self {
            Self(Rc::new(RefCell::new(res)))
        }

        fn borrow(&self) -> Ref<'_, BondStatusResponse> {
            self.0.borrow()
        }

        fn update_cap(&self, cap: impl Into<Uint128>) {
            let mut mut_obj = self.0.borrow_mut();
            mut_obj.cap.amount = cap.into();
        }
    }

    #[derive(Clone)]
    struct MockSlashRatio(Rc<RefCell<SlashRatioResponse>>);

    impl MockSlashRatio {
        fn new(res: SlashRatioResponse) -> Self {
            Self(Rc::new(RefCell::new(res)))
        }

        fn borrow(&self) -> Ref<'_, SlashRatioResponse> {
            self.0.borrow()
        }
    }

    fn set_reward_targets(storage: &mut dyn Storage, targets: &[&str]) {
        REWARD_TARGETS
            .save(
                storage,
                &targets.iter().map(<&str>::to_string).collect::<Vec<_>>(),
            )
            .unwrap();
    }

    trait VirtualStakingExt {
        fn quick_inst(&self, deps: DepsMut);
        fn push_rewards(&self, deps: &mut OwnedDeps, amount: u128) -> PushRewardsResult;
        fn hit_epoch(&self, deps: DepsMut) -> HitEpochResult;
        fn quick_bond(&self, deps: DepsMut, validator: &str, amount: u128);
        fn quick_unbond(&self, deps: DepsMut, validator: &str, amount: u128);
        fn quick_burn(
            &self,
            deps: DepsMut,
            validator: &[&str],
            amount: u128,
        ) -> Result<Response<VirtualStakeCustomMsg>, ContractError>;
        fn jail(
            &self,
            deps: DepsMut,
            val: &str,
            nominal_slash_ratio: Decimal,
            slash_amount: Uint128,
        );
        fn unjail(&self, deps: DepsMut, val: &str);
        fn tombstone(
            &self,
            deps: DepsMut,
            val: &str,
            nominal_slash_ratio: Decimal,
            slash_amount: Uint128,
        );
        fn add_val(&self, deps: DepsMut, val: &str);
        fn remove_val(&self, deps: DepsMut, val: &str);
    }

    impl VirtualStakingExt for VirtualStakingContract<'_> {
        fn quick_inst(&self, deps: DepsMut) {
            self.instantiate(InstantiateCtx {
                deps,
                env: mock_env(),
                info: mock_info("me", &[]),
            })
            .unwrap();
        }

        fn push_rewards(&self, deps: &mut OwnedDeps, amount: u128) -> PushRewardsResult {
            let denom = self.config.load(&deps.storage).unwrap().denom;
            let old_amount = deps
                .as_ref()
                .querier
                .query_balance(mock_env().contract.address, &denom)
                .unwrap()
                .amount
                .u128();
            deps.querier = MockQuerier::new(&[(
                mock_env().contract.address.as_str(),
                &coins(old_amount + amount, &denom),
            )]);

            let result = PushRewardsResult::new(
                self.reply_rewards(deps.as_mut(), mock_env())
                    .unwrap()
                    .messages,
            );

            if let PushRewardsResult::Batch(_) = result {
                deps.querier =
                    MockQuerier::new(&[(mock_env().contract.address.as_str(), &coins(0, denom))]);
            }

            result
        }

        #[track_caller]
        fn hit_epoch(&self, deps: DepsMut) -> HitEpochResult {
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            HitEpochResult::new(self.handle_epoch(deps).unwrap())
        }

        fn quick_bond(&self, deps: DepsMut, validator: &str, amount: u128) {
            let denom = self.config.load(deps.storage).unwrap().denom;

            self.bond(
                ExecCtx {
                    deps,
                    env: mock_env(),
                    info: mock_info("me", &[]),
                },
                validator.to_string(),
                coin(amount, denom),
            )
            .unwrap();
        }

        fn quick_unbond(&self, deps: DepsMut, validator: &str, amount: u128) {
            let denom = self.config.load(deps.storage).unwrap().denom;

            self.unbond(
                ExecCtx {
                    deps,
                    env: mock_env(),
                    info: mock_info("me", &[]),
                },
                validator.to_string(),
                coin(amount, denom),
            )
            .unwrap();
        }

        fn quick_burn(
            &self,
            deps: DepsMut,
            validators: &[&str],
            amount: u128,
        ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
            let denom = self.config.load(deps.storage).unwrap().denom;

            self.burn(
                ExecCtx {
                    deps,
                    env: mock_env(),
                    info: mock_info("me", &[]),
                },
                validators.iter().map(<&str>::to_string).collect(),
                coin(amount, denom),
            )
        }

        fn jail(
            &self,
            deps: DepsMut,
            val: &str,
            nominal_slash_ratio: Decimal,
            slash_amount: Uint128,
        ) {
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            // We sent a removal and a slash along with the jail, as this is what the blockchain does
            self.handle_valset_update(
                deps,
                None,
                Some(vec![val.to_string()]),
                None,
                Some(vec![val.to_string()]),
                None,
                None,
                Some(vec![ValidatorSlash {
                    address: val.to_string(),
                    height: 0,
                    time: 0,
                    infraction_height: 0,
                    infraction_time: 0,
                    power: 0,
                    slash_amount,
                    slash_ratio: nominal_slash_ratio.to_string(),
                }]),
            )
            .unwrap();
        }

        fn unjail(&self, deps: DepsMut, val: &str) {
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            self.handle_valset_update(
                deps,
                None,
                None,
                None,
                None,
                Some(vec![val.to_string()]),
                None,
                None,
            )
            .unwrap();
        }

        fn tombstone(
            &self,
            deps: DepsMut,
            val: &str,
            nominal_slash_ratio: Decimal,
            slash_amount: Uint128,
        ) {
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            // We sent a slash along with the tombstone, as this is what the blockchain does
            self.handle_valset_update(
                deps,
                None,
                None,
                None,
                None,
                None,
                Some(vec![val.to_string()]),
                Some(vec![ValidatorSlash {
                    address: val.to_string(),
                    height: 0,
                    time: 0,
                    infraction_height: 0,
                    infraction_time: 0,
                    power: 0,
                    slash_amount,
                    slash_ratio: nominal_slash_ratio.to_string(),
                }]),
            )
            .unwrap();
        }

        fn add_val(&self, deps: DepsMut, val: &str) {
            let val = cosmwasm_std::Validator {
                address: val.to_string(),
                commission: Default::default(),
                max_commission: Default::default(),
                max_change_rate: Default::default(),
            };
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            self.handle_valset_update(deps, Some(vec![val]), None, None, None, None, None, None)
                .unwrap();
        }

        fn remove_val(&self, deps: DepsMut, val: &str) {
            let deps = SudoCtx {
                deps,
                env: mock_env(),
            };
            self.handle_valset_update(
                deps,
                None,
                Some(vec![val.to_string()]),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        }
    }

    enum PushRewardsResult {
        Empty,
        Batch(Vec<RewardInfo>),
    }

    impl PushRewardsResult {
        fn new<C: cosmwasm_std::CustomMsg>(data: Vec<SubMsg<C>>) -> Self {
            match &data[..] {
                [] => Self::Empty,
                [SubMsg {
                    msg: CosmosMsg::Wasm(WasmMsg::Execute { msg: bin_msg, .. }),
                    ..
                }] => {
                    if let converter_api::sv::ExecMsg::DistributeRewards { mut payments } =
                        from_json(bin_msg).unwrap()
                    {
                        payments.sort();
                        Self::Batch(payments)
                    } else {
                        panic!("failed to deserialize DistributeRewards msg")
                    }
                }
                _ => panic!("invalid response"),
            }
        }

        #[track_caller]
        fn assert_empty(&self) {
            if let Self::Empty = self {
            } else {
                panic!("not empty");
            }
        }

        #[track_caller]
        fn assert_eq(&self, expected: &[(&str, u128)]) {
            if expected.is_empty() {
                self.assert_empty();
            } else if let Self::Batch(rewards) = self {
                let mut expected = expected
                    .iter()
                    .map(|(val, reward)| RewardInfo {
                        validator: val.to_string(),
                        reward: (*reward).into(),
                    })
                    .collect::<Vec<_>>();
                expected.sort();
                assert_eq!(rewards, &expected);
            } else {
                panic!("empty result")
            }
        }
    }

    struct HitEpochResult {
        virtual_stake_msgs: Vec<VirtualStakeMsg>,
        withdraw_reward_msgs: Vec<String>,
    }

    impl HitEpochResult {
        fn new(data: Response<VirtualStakeCustomMsg>) -> Self {
            use itertools::Either;
            use itertools::Itertools as _;

            let (virtual_stake_msgs, withdraw_reward_msgs) = data
                .messages
                .into_iter()
                .partition_map(|SubMsg { msg, .. }| {
                    if let CosmosMsg::Custom(VirtualStakeCustomMsg::VirtualStake(msg)) = msg {
                        Either::Left(msg)
                    } else if let CosmosMsg::Distribution(
                        DistributionMsg::WithdrawDelegatorReward { validator },
                    ) = msg
                    {
                        Either::Right(validator)
                    } else {
                        panic!("invalid message: {:?}", msg)
                    }
                });

            Self {
                virtual_stake_msgs,
                withdraw_reward_msgs,
            }
        }

        #[track_caller]
        fn assert_no_bonding(&self) -> &Self {
            if !self.virtual_stake_msgs.is_empty() {
                panic!(
                    "hit_epoch result was expected to be a noop, but has these: {:?}",
                    self.virtual_stake_msgs
                );
            }

            self
        }

        fn bond_msgs(&self) -> Vec<(&str, (u128, &str))> {
            self.virtual_stake_msgs
                .iter()
                .filter_map(|msg| {
                    if let VirtualStakeMsg::Bond { amount, validator } = msg {
                        Some((
                            validator.as_str(),
                            (amount.amount.u128(), amount.denom.as_str()),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        }

        fn unbond_msgs(&self) -> Vec<(&str, (u128, &str))> {
            self.virtual_stake_msgs
                .iter()
                .filter_map(|msg| {
                    if let VirtualStakeMsg::Unbond { amount, validator } = msg {
                        Some((
                            validator.as_str(),
                            (amount.amount.u128(), amount.denom.as_str()),
                        ))
                    } else {
                        None
                    }
                })
                .collect()
        }

        #[track_caller]
        fn assert_bond(&self, expected: &[(&str, (u128, &str))]) -> &Self {
            let mut expected = expected.to_vec();
            let mut actual = self.bond_msgs();
            expected.sort();
            actual.sort();

            assert_eq!(expected, actual);

            self
        }

        #[track_caller]
        fn assert_unbond(&self, expected: &[(&str, (u128, &str))]) -> &Self {
            let mut expected = expected.to_vec();
            let mut actual = self.unbond_msgs();
            expected.sort();
            actual.sort();

            assert_eq!(expected, actual);

            self
        }

        #[track_caller]
        fn assert_rewards(&self, expected: &[&str]) -> &Self {
            let mut expected = expected.to_vec();
            let mut actual = self.withdraw_reward_msgs.clone();
            expected.sort();
            actual.sort();

            assert_eq!(expected, actual);

            self
        }
    }
}

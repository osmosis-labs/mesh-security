use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use cosmwasm_std::{
    coin, ensure_eq, entry_point, to_binary, Coin, CosmosMsg, CustomQuery, Decimal, DepsMut,
    DistributionMsg, Env, Event, Reply, Response, StdResult, Storage, SubMsg, Uint128, Validator,
    WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::nonpayable;
use mesh_apis::converter_api::{self, RewardInfo};
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
    /// This is what validators have been fully unbonded due to tombstoning
    // The list will be cleared after processing in handle_epoch.
    pub tombstoned: Item<'a, Vec<String>>,
    /// This is what validators have been slashed due to jailing.
    // The list will be cleared after processing in handle_epoch.
    pub jailed: Item<'a, Vec<String>>,
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
            tombstoned: Item::new("tombstoned"),
            jailed: Item::new("jailed"),
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
        // initialize these to no one, so no issue when reading for the first time
        self.bonded.save(ctx.deps.storage, &vec![])?;
        self.tombstoned.save(ctx.deps.storage, &vec![])?;
        self.jailed.save(ctx.deps.storage, &vec![])?;
        VALIDATOR_REWARDS_BATCH.init(ctx.deps.storage)?;

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
            self.bonded.save(deps.storage, &vec![])?;
            return Ok(resp);
        }

        // Make current bonded mutable
        let mut current = bonded;
        // Process tombstoning (unbonded) and jailing (slashed) over bond_requests and current
        let tombstoned = self.tombstoned.load(deps.storage)?;
        let jailed = self.jailed.load(deps.storage)?;
        if !tombstoned.is_empty() || !jailed.is_empty() {
            let slash_ratio = if !jailed.is_empty() {
                // Only query if needed
                let ratios = TokenQuerier::new(&deps.querier).slash_ratio()?;
                Decimal::from_str(&ratios.slash_fraction_downtime)?
            } else {
                Decimal::zero()
            };
            self.adjust_slashings(deps.storage, &mut current, tombstoned, jailed, slash_ratio)?;
            // Clear up both lists
            self.tombstoned.save(deps.storage, &vec![])?;
            self.jailed.save(deps.storage, &vec![])?;
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

        // Save the future values
        self.bonded.save(deps.storage, &requests)?;

        // Compare these two to make bond/unbond calls as needed
        let config = self.config.load(deps.storage)?;
        let rebalance = calculate_rebalance(current, requests, &config.denom);
        let resp = resp.add_messages(rebalance);

        Ok(resp)
    }

    fn adjust_slashings(
        &self,
        storage: &mut dyn Storage,
        current: &mut [(String, Uint128)],
        tombstones: Vec<String>,
        jailing: Vec<String>,
        slash_ratio: Decimal,
    ) -> StdResult<()> {
        let tombstones: BTreeSet<_> = tombstones.into_iter().collect();
        let jailing: BTreeSet<_> = jailing.into_iter().collect();

        // this is linear over current, but better than turn it in to a map
        for (validator, prev) in current {
            let tombstoned = tombstones.contains(validator);
            let jailed = jailing.contains(validator);
            if tombstoned {
                // Set current to zero
                *prev = Uint128::zero();
                // Remove request as well, to avoid unbonding msg (auto unbonded when tombstoned)
                self.bond_requests.remove(storage, validator);
            } else if jailed {
                // Apply slash ratio to current
                *prev -= *prev * slash_ratio;
                // Apply to request as well, to avoid unbonding msg
                let mut request = self
                    .bond_requests
                    .may_load(storage, validator)?
                    .unwrap_or_default();
                request -= request * slash_ratio;
                self.bond_requests.save(storage, validator, &request)?;
            }
        }
        Ok(())
    }

    /**
     * This is called every time there's a change of the active validator set.
     *
     */
    #[allow(clippy::too_many_arguments)]
    fn handle_valset_update(
        &self,
        deps: DepsMut<VirtualStakeCustomQuery>,
        additions: &[Validator],
        removals: &[String],
        updated: &[Validator],
        jailed: &[String],
        unjailed: &[String],
        tombstoned: &[String],
    ) -> Result<Response<VirtualStakeCustomMsg>, ContractError> {
        let _ = (removals, updated, unjailed);

        // Account for tombstoned validators. Will be processed in handle_epoch
        self.tombstoned.update(deps.storage, |mut old| {
            old.extend_from_slice(tombstoned);
            Ok::<_, ContractError>(old)
        })?;

        // Account for jailed validators. Will be processed in handle_epoch
        self.jailed.update(deps.storage, |mut old| {
            old.extend_from_slice(jailed);
            Ok::<_, ContractError>(old)
        })?;

        // Send additions and tombstones to the Converter. Removals are non-permanent and ignored.
        // Send jailed even when they are non-permanent, for slashing.
        let cfg = self.config.load(deps.storage)?;
        let msg = converter_api::ExecMsg::ValsetUpdate {
            additions: additions.to_vec(),
            tombstoned: tombstoned.to_vec(),
            jailed: jailed.to_vec(),
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
    fn reply_rewards(&self, mut deps: DepsMut, env: Env) -> Result<Response, ContractError> {
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

            let msg = converter_api::ExecMsg::DistributeRewards {
                payments: all_rewards,
            };
            let msg = WasmMsg::Execute {
                contract_addr: cfg.converter.into_string(),
                msg: to_binary(&msg)?,
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
}

/// Returns a tuple containing the reward target and a boolean value
/// specifying if we've exhausted the list.
fn pop_target(deps: DepsMut) -> StdResult<(String, bool)> {
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
            updated,
            jailed,
            unjailed,
            tombstoned,
        } => VirtualStakingContract::new().handle_valset_update(
            deps,
            &additions,
            &removals,
            &updated,
            &jailed,
            &unjailed,
            &tombstoned,
        ),
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
        coins, from_binary,
        testing::{mock_env, mock_info, MockApi, MockQuerier, MockStorage},
    };
    use mesh_bindings::{BondStatusResponse, SlashRatioResponse};
    use serde::de::DeserializeOwned;

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

        // val1 is being jailed
        contract.jail(deps.as_mut(), "val1");

        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after jailing
            .assert_unbond(&[]) // No unbond msgs after jailing
            .assert_rewards(&["val1", "val2"]); // But rewards can still be gathered

        // Check that the bonded amounts of val1 have been slashed for being offline (10%)
        // Val2 is unaffected.
        let bonded = contract.bonded.load(deps.as_ref().storage).unwrap();
        assert_eq!(
            bonded,
            [
                ("val1".to_string(), Uint128::new(9)),
                ("val2".to_string(), Uint128::new(20))
            ]
        );

        // FIXME: Subsequent rewards msgs could be removed while validator is jailed / inactive
        contract
            .hit_epoch(deps.as_mut())
            .assert_rewards(&["val1", "val2"]); // But rewards can still be gathered

        contract.unjail(deps.as_mut(), "val1");
        contract
            .hit_epoch(deps.as_mut())
            .assert_bond(&[]) // No bond msgs after unjailing
            .assert_unbond(&[]) // No unbond msgs after unjailing
            .assert_rewards(&["val1", "val2"]);
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
        // FIXME: Subsequent rewards msgs could be removed while validator is inactive
        contract.hit_epoch(deps.as_mut()).assert_rewards(&["val1"]);
    }

    #[test]
    #[ignore]
    fn validator_tombstone() {
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

        contract.tombstone(deps.as_mut(), "val1");
        contract.hit_epoch(deps.as_mut()).assert_rewards(&[]);
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
            slash_fraction_double_sign: "0.2".to_string(),
        });

        let handler = {
            let bs_copy = bond_status.clone();
            move |msg: &_| {
                let VirtualStakeCustomQuery::VirtualStake(msg) = msg;
                match msg {
                    mesh_bindings::VirtualStakeQuery::BondStatus { .. } => {
                        cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(
                            to_binary(&*bs_copy.borrow()).unwrap(),
                        ))
                    }
                    mesh_bindings::VirtualStakeQuery::SlashRatio {} => {
                        cosmwasm_std::SystemResult::Ok(cosmwasm_std::ContractResult::Ok(
                            to_binary(&*slash_ratio.borrow()).unwrap(),
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
            StakingKnobs {
                bond_status,
            },
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
        fn quick_inst<C: CustomQuery>(&self, deps: DepsMut<C>);
        fn push_rewards<C: CustomQuery + DeserializeOwned>(
            &self,
            deps: &mut OwnedDeps<C>,
            amount: u128,
        ) -> PushRewardsResult;
        fn hit_epoch(&self, deps: DepsMut) -> HitEpochResult;
        fn quick_bond(&self, deps: DepsMut, validator: &str, amount: u128);
        fn quick_unbond(&self, deps: DepsMut, validator: &str, amount: u128);
        fn jail(&self, deps: DepsMut, val: &str);
        fn unjail(&self, deps: DepsMut, val: &str);
        fn tombstone(&self, deps: DepsMut, val: &str);
        fn remove_val(&self, deps: DepsMut, val: &str);
    }

    impl VirtualStakingExt for VirtualStakingContract<'_> {
        fn quick_inst<C: CustomQuery>(&self, deps: DepsMut<C>) {
            self.instantiate(InstantiateCtx {
                deps: deps.into_empty(),
                env: mock_env(),
                info: mock_info("me", &[]),
            })
            .unwrap();
        }

        fn push_rewards<C: CustomQuery + DeserializeOwned>(
            &self,
            deps: &mut OwnedDeps<C>,
            amount: u128,
        ) -> PushRewardsResult {
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
                self.reply_rewards(deps.as_mut().into_empty(), mock_env())
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
            HitEpochResult::new(self.handle_epoch(deps, mock_env()).unwrap())
        }

        fn quick_bond(&self, deps: DepsMut, validator: &str, amount: u128) {
            let denom = self.config.load(deps.storage).unwrap().denom;

            self.bond(
                ExecCtx {
                    deps: deps.into_empty(),
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
                    deps: deps.into_empty(),
                    env: mock_env(),
                    info: mock_info("me", &[]),
                },
                validator.to_string(),
                coin(amount, denom),
            )
            .unwrap();
        }

        fn jail(&self, deps: DepsMut, val: &str) {
            self.handle_valset_update(deps, &[], &[], &[], &[val.to_string()], &[], &[])
                .unwrap();
        }

        fn unjail(&self, deps: DepsMut, val: &str) {
            self.handle_valset_update(deps, &[], &[], &[], &[], &[val.to_string()], &[])
                .unwrap();
        }

        fn tombstone(&self, deps: DepsMut, val: &str) {
            self.handle_valset_update(deps, &[], &[], &[], &[], &[], &[val.to_string()])
                .unwrap();
        }

        fn remove_val(&self, deps: DepsMut, val: &str) {
            self.handle_valset_update(deps, &[], &[val.to_string()], &[], &[], &[], &[])
                .unwrap();
        }
    }

    enum PushRewardsResult {
        Empty,
        Batch(Vec<RewardInfo>),
    }

    impl PushRewardsResult {
        fn new(data: Vec<SubMsg>) -> Self {
            match &data[..] {
                [] => Self::Empty,
                [SubMsg {
                    msg: CosmosMsg::Wasm(WasmMsg::Execute { msg: bin_msg, .. }),
                    ..
                }] => {
                    if let converter_api::ExecMsg::DistributeRewards { mut payments } =
                        from_binary(bin_msg).unwrap()
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

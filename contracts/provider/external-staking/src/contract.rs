use cosmwasm_std::{
    coin, ensure, ensure_eq, to_binary, Coin, Decimal, DepsMut, Env, Event, IbcMsg, Order,
    Response, StdResult, Storage, Uint128, Uint256, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use cw_utils::{nonpayable, PaymentError};
use std::cmp::min;
use std::collections::HashSet;

use mesh_apis::converter_api::RewardInfo;
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

use mesh_apis::cross_staking_api::{self};
use mesh_apis::ibc::{AddValidator, ProviderPacket};
use mesh_apis::vault_api::{SlashInfo, VaultApiHelper};
use mesh_sync::{Tx, ValueRange};

use crate::crdt::{CrdtState, State};
use crate::error::ContractError;
use crate::ibc::{packet_timeout, IBC_CHANNEL};
use crate::msg::{
    AllPendingRewards, AllTxsResponse, AuthorizedEndpointResponse, ConfigResponse,
    IbcChannelResponse, ListActiveValidatorsResponse, ListValidatorsResponse, PendingRewards,
    StakeInfo, StakesResponse, TxResponse, ValidatorPendingRewards,
};
use crate::stakes::Stakes;
use crate::state::{Config, Distribution, Stake};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_PAGE_LIMIT: u32 = 10;
pub const MAX_PAGE_LIMIT: u32 = 30;

pub const DISTRIBUTION_POINTS_SCALE: Uint256 = Uint256::from_u128(1_000_000_000);

/// Aligns pagination limit
fn clamp_page_limit(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).max(MAX_PAGE_LIMIT) as usize
}

pub struct ExternalStakingContract<'a> {
    pub config: Item<'a, Config>,
    /// Stakes indexed by `(owner, validator)` pair
    pub stakes: Stakes<'a>,
    /// Per-validator distribution information
    pub distribution: Map<'a, &'a str, Distribution>,
    /// Pending txs information
    pub tx_count: Item<'a, u64>,
    pub pending_txs: Map<'a, u64, Tx>,
    /// Valset CRDT
    pub val_set: CrdtState<'a>,
}

impl Default for ExternalStakingContract<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(cross_staking_api as CrossStakingApi)]
#[messages(crate::test_methods as TestMethods)]
impl ExternalStakingContract<'_> {
    pub fn new() -> Self {
        Self {
            config: Item::new("config"),
            stakes: Stakes::new("stakes", "vals"),
            distribution: Map::new("distribution"),
            pending_txs: Map::new("pending_txs"),
            tx_count: Item::new("tx_count"),
            val_set: CrdtState::new(),
        }
    }

    pub fn next_tx_id(&self, store: &mut dyn Storage) -> StdResult<u64> {
        // `vault` and `external-staking` transaction ids are in different ranges for clarity.
        // The second (`vault`'s) transaction's commit or rollback cannot fail.
        let id: u64 = self.tx_count.may_load(store)?.unwrap_or(u64::MAX >> 1) + 1;
        self.tx_count.save(store, &id)?;
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        rewards_denom: String,
        vault: String,
        unbonding_period: u64,
        remote_contact: crate::msg::AuthorizedEndpoint,
        max_slashing: Decimal,
    ) -> Result<Response, ContractError> {
        let vault = ctx.deps.api.addr_validate(&vault)?;
        let vault = VaultApiHelper(vault);

        if max_slashing > Decimal::one() {
            return Err(ContractError::InvalidMaxSlashing);
        }

        let config = Config {
            denom,
            rewards_denom,
            vault,
            unbonding_period,
            max_slashing,
        };

        self.config.save(ctx.deps.storage, &config)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        remote_contact.validate()?;
        crate::ibc::AUTH_ENDPOINT.save(ctx.deps.storage, &remote_contact)?;

        // test code sets a channel, so we can closer approximate ibc in test code
        #[cfg(any(feature = "mt", test))]
        {
            let channel = cosmwasm_std::testing::mock_ibc_channel(
                "channel-172",
                cosmwasm_std::IbcOrder::Unordered,
                "mesh-security",
            );
            crate::ibc::IBC_CHANNEL.save(ctx.deps.storage, &channel)?;
        }

        Ok(Response::new())
    }

    /// In test code, this is called from `test_commit_stake`.
    /// In non-test code, this is called from `ibc_packet_ack`
    pub(crate) fn commit_stake(&self, deps: DepsMut, tx_id: u64) -> Result<WasmMsg, ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // Verify tx is of the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteStaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteStaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        // Load stake
        let mut stake = self
            .stakes
            .stake
            .load(deps.storage, (&tx_user, &tx_validator))?;

        // Load distribution
        let mut distribution = self
            .distribution
            .may_load(deps.storage, &tx_validator)?
            .unwrap_or_default();

        // Commit stake (saturating up if slashed)
        stake.stake.commit_add_saturating(tx_amount);

        // Distribution alignment
        stake
            .points_alignment
            .stake_increased(tx_amount, distribution.points_per_stake);
        distribution.total_stake += tx_amount;

        // Save stake
        self.stakes
            .stake
            .save(deps.storage, (&tx_user, &tx_validator), &stake)?;

        // Save distribution
        self.distribution
            .save(deps.storage, &tx_validator, &distribution)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);

        // Call commit hook on vault
        let cfg = self.config.load(deps.storage)?;
        let msg = cfg.vault.commit_tx(tx_id)?;
        Ok(msg)
    }

    /// In test code, this is called from `test_rollback_stake`.
    /// In non-test code, this is called from `ibc_packet_ack` or `ibc_packet_timeout`
    pub(crate) fn rollback_stake(
        &self,
        deps: DepsMut,
        tx_id: u64,
    ) -> Result<WasmMsg, ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // Verify tx is of the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteStaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteStaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        // Load stake
        let mut stake = self
            .stakes
            .stake
            .load(deps.storage, (&tx_user, &tx_validator))?;

        // Rollback add amount (saturating up if slashed)
        stake.stake.rollback_add_saturating(tx_amount);

        // Save stake
        self.stakes
            .stake
            .save(deps.storage, (&tx_user, &tx_validator), &stake)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);

        // Call rollback hook on vault
        let cfg = self.config.load(deps.storage)?;
        let msg = cfg.vault.rollback_tx(tx_id)?;
        Ok(msg)
    }

    /// Schedules tokens for release, adding them to the pending unbonds. After the unbonding period
    /// passes, funds are ready to be released through a `withdraw_unbonded` call by the user.
    #[msg(exec)]
    pub fn unstake(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, ContractError> {
        let ExecCtx { info, deps, env } = ctx;
        nonpayable(&info)?;

        let config = self.config.load(deps.storage)?;

        ensure_eq!(
            amount.denom,
            config.denom,
            ContractError::InvalidDenom(config.denom)
        );

        let mut stake = self
            .stakes
            .stake
            .may_load(deps.storage, (&info.sender, &validator))?
            .unwrap_or_default();

        ensure!(
            stake.stake.low() >= amount.amount,
            ContractError::NotEnoughStake(stake.stake.low())
        );

        stake.stake.prepare_sub(amount.amount, Uint128::zero())?;

        self.stakes
            .stake
            .save(deps.storage, (&info.sender, &validator), &stake)?;

        // Create new tx
        let tx_id = self.next_tx_id(deps.storage)?;

        // Save tx
        let new_tx = Tx::InFlightRemoteUnstaking {
            id: tx_id,
            amount: amount.amount,
            user: info.sender.clone(),
            validator: validator.clone(),
        };
        self.pending_txs.save(deps.storage, tx_id, &new_tx)?;

        #[allow(unused_mut)]
        let mut resp = Response::new()
            .add_attribute("action", "unstake")
            .add_attribute("amount", amount.amount.to_string())
            .add_attribute("owner", info.sender);

        let channel = IBC_CHANNEL.load(deps.storage)?;
        let packet = ProviderPacket::Unstake {
            validator,
            unstake: amount,
            tx_id,
        };
        let msg = IbcMsg::SendPacket {
            channel_id: channel.endpoint.channel_id,
            data: to_binary(&packet)?,
            timeout: packet_timeout(&env),
        };
        // send packet if we are ibc enabled
        // TODO: send in test code when we can handle it
        #[cfg(not(any(test, feature = "mt")))]
        {
            resp = resp.add_message(msg);
        }
        #[cfg(any(test, feature = "mt"))]
        {
            let _ = msg;
        }

        Ok(resp)
    }

    /// In test code, this is called from `test_commit_unstake`.
    /// In non-test code, this is called from `ibc_packet_ack`
    pub(crate) fn commit_unstake(
        &self,
        deps: DepsMut,
        env: Env,
        tx_id: u64,
    ) -> Result<(), ContractError> {
        use crate::state::PendingUnbond;

        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // Verify tx is of the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteUnstaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteUnstaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        let config = self.config.load(deps.storage)?;

        // Load stake
        let mut stake = self
            .stakes
            .stake
            .load(deps.storage, (&tx_user, &tx_validator))?;

        // Load distribution
        let mut distribution = self
            .distribution
            .may_load(deps.storage, &tx_validator)?
            .unwrap_or_default();

        // Commit sub amount, saturating if slashed
        let amount = min(tx_amount, stake.stake.high());
        stake.stake.commit_sub(amount);

        let immediate_release = matches!(
            self.val_set.validator_state(deps.storage, &tx_validator)?,
            State::Unbonded {} | State::Tombstoned {}
        );

        // FIXME? Release period being computed after successful IBC tx
        // (Note: this is good for now, but can be revisited in v1 design)
        let release_at = if immediate_release {
            env.block.time
        } else {
            env.block.time.plus_seconds(config.unbonding_period)
        };
        let unbond = PendingUnbond { amount, release_at };
        stake.pending_unbonds.push(unbond);

        // Distribution alignment
        stake
            .points_alignment
            .stake_decreased(amount, distribution.points_per_stake);
        distribution.total_stake -= amount;

        // Save stake
        self.stakes
            .stake
            .save(deps.storage, (&tx_user, &tx_validator), &stake)?;

        // Save distribution
        self.distribution
            .save(deps.storage, &tx_validator, &distribution)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);
        Ok(())
    }

    /// In test code, this is called from `test_rollback_unstake`.
    /// In non-test code, this is called from `ibc_packet_ack` or `ibc_packet_timeout`
    pub(crate) fn rollback_unstake(&self, deps: DepsMut, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // Verify tx is of the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteUnstaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );
        let (tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteUnstaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        // Load stake
        let mut stake = self
            .stakes
            .stake
            .load(deps.storage, (&tx_user, &tx_validator))?;

        // Rollback sub amount
        stake.stake.rollback_sub_saturating(tx_amount);

        // Save stake
        self.stakes
            .stake
            .save(deps.storage, (&tx_user, &tx_validator), &stake)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);
        Ok(())
    }

    /// In non-test code, this is called from `ibc_packet_ack`
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn valset_update(
        &self,
        deps: DepsMut,
        env: Env,
        height: u64,
        time: u64,
        additions: &[AddValidator],
        removals: &[String],
        updated: &[AddValidator],
        jailed: &[String],
        unjailed: &[String],
        tombstoned: &[String],
    ) -> Result<(Event, Vec<WasmMsg>), ContractError> {
        let cfg = self.config.load(deps.storage)?;
        let mut msgs = vec![];
        let mut valopers: HashSet<String> = HashSet::new();
        // Process tombstoning events first. Once tombstoned, a validator cannot be changed anymore.
        for valoper in tombstoned {
            // Check that the validator is active at height and slash it if that is the case
            let active =
                self.val_set
                    .is_active_validator_at_height(deps.storage, valoper, height)?;
            self.val_set
                .tombstone_validator(deps.storage, valoper, height, time)?;
            if active {
                // Slash the validator (if bonded)
                let slash_msg = self.handle_slashing(&env, deps.storage, &cfg, valoper)?;
                if let Some(msg) = slash_msg {
                    msgs.push(msg)
                }
            }
            // Maintenance
            valopers.insert(valoper.clone());
        }
        // Process additions. Already existing validators will be updated and set to active.
        // If the validator is tombstoned, this will be ignored.
        for AddValidator { valoper, pub_key } in additions {
            self.val_set
                .add_validator(deps.storage, valoper, pub_key, height, time)?;
            // Maintenance
            valopers.insert(valoper.clone());
        }
        // Process removals. Non-existent validators will be ignored.
        for valoper in removals {
            self.val_set
                .remove_validator(deps.storage, valoper, height, time)?;
            // Maintenance
            valopers.insert(valoper.clone());
        }
        // Process jailings. Non-existent validators will be ignored.
        for valoper in jailed {
            // Check that the validator is active at height and slash it if that is the case
            let active =
                self.val_set
                    .is_active_validator_at_height(deps.storage, valoper, height)?;
            self.val_set
                .jail_validator(deps.storage, valoper, height, time)?;
            if active {
                // Slash the validator, if bonded
                // TODO: Slash with a different slash ratio! (downtime / offline slash ratio)
                let slash_msg = self.handle_slashing(&env, deps.storage, &cfg, valoper)?;
                if let Some(msg) = slash_msg {
                    msgs.push(msg)
                }
            }
            // Maintenance
            valopers.insert(valoper.clone());
        }
        // Process unjailings. Does nothing at the moment, as we don't have a way to know if we the
        // validator must go to the active or the unbonded state.

        // Process updates. Non-existent and tombstoned validators will be ignored.
        for AddValidator { valoper, pub_key } in updated {
            self.val_set
                .update_validator(deps.storage, valoper, pub_key, height, time)?;
            // Maintenance
            valopers.insert(valoper.clone());
        }
        // Maintenance. Drain events that are older than unbonding period from now
        // Assumes time keeping is the same in both chains
        let max_time = env
            .block
            .time
            .seconds()
            .saturating_sub(cfg.unbonding_period);
        for valoper in valopers {
            self.val_set.drain_older(deps.storage, &valoper, max_time)?;
        }
        let mut event = Event::new("valset_update");
        if !additions.is_empty() {
            event = event.add_attribute(
                "additions",
                additions
                    .iter()
                    .map(|v| v.valoper.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
        }
        if !removals.is_empty() {
            event = event.add_attribute("removals", removals.join(","));
        }
        if !updated.is_empty() {
            event = event.add_attribute(
                "updated",
                updated
                    .iter()
                    .map(|v| v.valoper.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
        }
        if !jailed.is_empty() {
            event = event.add_attribute("jailed", jailed.join(","));
        }
        if !unjailed.is_empty() {
            event = event.add_attribute("unjailed", unjailed.join(","));
        }
        if !tombstoned.is_empty() {
            event = event.add_attribute("tombstoned", tombstoned.join(","));
        }
        Ok((event, msgs))
    }

    /// Withdraws all of their released tokens to the calling user.
    ///
    /// Tokens to be claimed have to be unbond before by calling the `unbond` message, and
    /// their unbonding period must have passed.
    #[msg(exec)]
    pub fn withdraw_unbonded(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let config = self.config.load(ctx.deps.storage)?;

        let stakes: Vec<_> = self
            .stakes
            .stake
            .prefix(&ctx.info.sender)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .collect::<Result<_, _>>()?;

        let released: Uint128 = stakes
            .into_iter()
            .map(|(validator, mut stake)| -> Result<_, ContractError> {
                let released = stake.release_pending(&ctx.env.block);

                if !released.is_zero() {
                    self.stakes.stake.save(
                        ctx.deps.storage,
                        (&ctx.info.sender, &validator),
                        &stake,
                    )?
                }

                Ok(released)
            })
            .try_fold(Uint128::zero(), |acc, released| {
                released.map(|released| released + acc)
            })?;

        let mut resp = Response::new()
            .add_attribute("action", "withdraw_unbonded")
            .add_attribute("owner", ctx.info.sender.to_string())
            .add_attribute("amount", released.to_string());

        if !released.is_zero() {
            let release_msg = config.vault.release_cross_stake(
                ctx.info.sender.into_string(),
                coin(released.u128(), &config.denom),
                vec![],
            )?;

            resp = resp.add_message(release_msg);
        }

        Ok(resp)
    }

    /// Distributes reward among users staking via particular validator. Distribution is performed
    /// proportionally to amount of tokens staked by user.
    /// In test code, this is called from `test_distribute_rewards`.
    /// In non-test code, this is called from `ibc_packet_receive`
    pub(crate) fn distribute_rewards(
        &self,
        mut deps: DepsMut,
        validator: &str,
        rewards: Coin,
    ) -> Result<Event, ContractError> {
        // check we have the proper denom
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            rewards.denom,
            config.rewards_denom,
            PaymentError::MissingDenom(rewards.denom)
        );

        self.distribute_rewards_unchecked(&mut deps, validator, rewards.amount)
    }

    fn distribute_rewards_unchecked(
        &self,
        deps: &mut DepsMut,
        validator: &str,
        amount: Uint128,
    ) -> Result<Event, ContractError> {
        let mut distribution = self
            .distribution
            .may_load(deps.storage, validator)?
            .unwrap_or_default();

        let total_stake = Uint256::from(distribution.total_stake);
        let points_distributed =
            Uint256::from(amount) * DISTRIBUTION_POINTS_SCALE + distribution.points_leftover;
        let points_per_stake = points_distributed / total_stake;

        distribution.points_leftover = points_distributed - points_per_stake * total_stake;
        distribution.points_per_stake += points_per_stake;

        self.distribution
            .save(deps.storage, validator, &distribution)?;

        let event = Event::new("distribute_rewards")
            .add_attribute("validator", validator)
            .add_attribute("amount", amount.to_string());

        Ok(event)
    }

    pub(crate) fn distribute_rewards_batch(
        &self,
        mut deps: DepsMut,
        rewards: &[RewardInfo],
        denom: &str,
    ) -> Result<Vec<Event>, ContractError> {
        // check we have the proper denom
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            denom,
            config.rewards_denom,
            ContractError::InvalidDenom(config.rewards_denom)
        );

        rewards
            .iter()
            .map(|reward_info| {
                self.distribute_rewards_unchecked(
                    &mut deps,
                    &reward_info.validator,
                    reward_info.reward,
                )
            })
            .collect()
    }

    /// Withdraw rewards from staking via given validator
    #[msg(exec)]
    pub fn withdraw_rewards(
        &self,
        ctx: ExecCtx,
        validator: String,
        /// Address on the consumer side to receive the rewards
        remote_recipient: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let stake = self
            .stakes
            .stake
            .may_load(ctx.deps.storage, (&ctx.info.sender, &validator))?
            .unwrap_or_default();

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(&stake, &distribution)?;

        if amount.is_zero() {
            return Err(ContractError::NoRewards);
        }

        #[allow(unused_mut)]
        let mut resp = Response::new()
            .add_attribute("action", "withdraw_rewards")
            .add_attribute("owner", ctx.info.sender.to_string())
            .add_attribute("validator", &validator)
            .add_attribute("recipient", &remote_recipient)
            .add_attribute("amount", amount.to_string());

        // prepare the pending tx
        let tx_id = self.next_tx_id(ctx.deps.storage)?;
        let new_tx = Tx::InFlightTransferFunds {
            id: tx_id,
            amount,
            staker: ctx.info.sender,
            validator,
        };
        self.pending_txs.save(ctx.deps.storage, tx_id, &new_tx)?;

        // Crate the IBC packet
        let config = self.config.load(ctx.deps.storage)?;
        let rewards = coin(amount.u128(), config.rewards_denom);
        let packet = ProviderPacket::TransferRewards {
            rewards,
            recipient: remote_recipient,
            tx_id,
        };
        let channel_id = IBC_CHANNEL.load(ctx.deps.storage)?.endpoint.channel_id;
        let send_msg = IbcMsg::SendPacket {
            channel_id,
            data: to_binary(&packet)?,
            timeout: packet_timeout(&ctx.env),
        };

        // TODO: send in test code when we can handle it
        #[cfg(not(any(test, feature = "mt")))]
        {
            resp = resp.add_message(send_msg);
        }
        #[cfg(any(test, feature = "mt"))]
        {
            let _ = send_msg;
        }

        Ok(resp)
    }

    /// In test code, this is called from `test_rollback_withdraw_rewards`.
    /// In non-test code, this is called from `ibc_packet_ack` or `ibc_packet_timeout`
    pub(crate) fn rollback_withdraw_rewards(
        &self,
        deps: DepsMut,
        tx_id: u64,
    ) -> Result<(), ContractError> {
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // Verify tx is of the right type and remove it from the map
        match tx {
            Tx::InFlightTransferFunds { .. } => {
                self.pending_txs.remove(deps.storage, tx_id);
            }
            _ => {
                return Err(ContractError::WrongTypeTx(tx_id, tx));
            }
        };

        Ok(())
    }

    /// In test code, this is called from `test_commit_withdraw_rewards`.
    /// In non-test code, this is called from `ibc_packet_ack`
    pub(crate) fn commit_withdraw_rewards(
        &self,
        deps: DepsMut,
        tx_id: u64,
    ) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;
        self.pending_txs.remove(deps.storage, tx_id);

        // Verify tx is of the right type and get data
        let (amount, staker, validator) = match tx {
            Tx::InFlightTransferFunds {
                amount,
                staker,
                validator,
                ..
            } => (amount, staker, validator),
            _ => {
                return Err(ContractError::WrongTypeTx(tx_id, tx));
            }
        };

        // Update withdrawn_funds to hold this transfer
        let mut stake = self
            .stakes
            .stake
            .load(deps.storage, (&staker, &validator))?;
        stake.withdrawn_funds += amount;

        self.stakes
            .stake
            .save(deps.storage, (&staker, &validator), &stake)?;

        Ok(())
    }

    /// Slashes a validator.
    ///
    /// In test code, this is called from `test_handle_slashing`.
    /// In non-test code, this is being called from `ibc_packet_receive` (in the `ConsumerPacket::RemoveValidators`
    /// handler)
    pub(crate) fn handle_slashing(
        &self,
        env: &Env,
        storage: &mut dyn Storage,
        config: &Config,
        validator: &str,
    ) -> Result<Option<WasmMsg>, ContractError> {
        // Get the list of users staking via this validator
        let users = self
            .stakes
            .stake
            .idx
            .rev
            .sub_prefix(validator.to_string())
            .range(storage, None, None, Order::Ascending)
            .map(|item| {
                let ((user, _), stake) = item?;
                Ok::<_, ContractError>((user, stake))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if users.is_empty() {
            return Ok(None);
        }

        // Slash their stake in passing
        let mut slash_infos = vec![];
        for (user, ref mut stake) in users {
            let stake_low = stake.stake.low();
            let stake_high = stake.stake.high();
            // Calculating slashing with always the `high` value of the range goes against the user
            // in some scenario (pending stakes while slashing); but the scenario is relatively
            // unlikely.
            let stake_slash = stake_high * config.max_slashing;
            // Requires proper saturating methods in commit/rollback_stake/unstake
            stake.stake = ValueRange::new(
                stake_low.saturating_sub(stake_slash),
                stake_high - stake_slash,
            );

            // Distribution alignment
            let mut distribution = self
                .distribution
                .may_load(storage, validator)?
                .unwrap_or_default();
            stake
                .points_alignment
                .stake_decreased(stake_slash, distribution.points_per_stake);
            distribution.total_stake = distribution.total_stake.saturating_sub(stake_slash); // Don't fail if pending bond tx
            self.distribution.save(storage, validator, &distribution)?;

            // Slash the unbondings
            let pending_slashed = stake.slash_pending(&env.block, config.max_slashing);

            self.stakes.stake.save(storage, (&user, validator), stake)?;

            slash_infos.push(SlashInfo {
                user: user.to_string(),
                slash: stake_slash + pending_slashed,
            });
        }

        // Route associated users to vault for slashing of their collateral
        let msg = config
            .vault
            .process_cross_slashing(slash_infos, validator)?;
        Ok(Some(msg))
    }

    /// Queries for contract configuration
    #[msg(query)]
    pub fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let resp = self.config.load(ctx.deps.storage)?.into();
        Ok(resp)
    }

    /// Query for the endpoint that can connect
    #[msg(query)]
    pub fn authorized_endpoint(
        &self,
        ctx: QueryCtx,
    ) -> Result<AuthorizedEndpointResponse, ContractError> {
        let resp = crate::ibc::AUTH_ENDPOINT.load(ctx.deps.storage)?;
        Ok(resp)
    }

    /// Query for the endpoint that can connect
    #[msg(query)]
    pub fn ibc_channel(&self, ctx: QueryCtx) -> Result<IbcChannelResponse, ContractError> {
        let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
        Ok(IbcChannelResponse { channel })
    }

    /// Show all external validators that we know to be active (and can delegate to)
    #[msg(query)]
    pub fn list_active_validators(
        &self,
        ctx: QueryCtx,
        start_after: Option<String>,
        limit: Option<u64>,
    ) -> Result<ListActiveValidatorsResponse, ContractError> {
        let limit = limit.unwrap_or(100) as usize;
        let validators =
            self.val_set
                .list_active_validators(ctx.deps.storage, start_after.as_deref(), limit)?;
        Ok(ListActiveValidatorsResponse { validators })
    }

    /// Show all external validators that we know about, along with their state.
    #[msg(query)]
    pub fn list_validators(
        &self,
        ctx: QueryCtx,
        start_after: Option<String>,
        limit: Option<u64>,
    ) -> Result<ListValidatorsResponse, ContractError> {
        let limit = limit.unwrap_or(100) as usize;
        let validators = self
            .val_set
            .list_validators(ctx.deps.storage, start_after.as_deref(), limit)?
            .into_iter()
            .map(|(valoper, state)| {
                Ok(crate::msg::ValidatorState {
                    validator: valoper,
                    state,
                })
            })
            .collect::<StdResult<Vec<_>>>()?;
        Ok(ListValidatorsResponse { validators })
    }

    /// Queries for stake info
    ///
    /// If stake does not exist for (user, validator) pair, the zero-stake is returned
    #[msg(query)]
    pub fn stake(
        &self,
        ctx: QueryCtx,
        user: String,
        validator: String,
    ) -> Result<Stake, ContractError> {
        let user = ctx.deps.api.addr_validate(&user)?;
        let stake = self
            .stakes
            .stake
            .may_load(ctx.deps.storage, (&user, &validator))?
            .unwrap_or_default();

        Ok(stake)
    }

    /// Paginated list of user stakes.
    ///
    /// `start_after` is the last validator of previous page
    #[msg(query)]
    pub fn stakes(
        &self,
        ctx: QueryCtx,
        user: String,
        start_after: Option<String>,
        limit: Option<u32>,
    ) -> Result<StakesResponse, ContractError> {
        let limit = clamp_page_limit(limit);
        let user = ctx.deps.api.addr_validate(&user)?;

        let bound = start_after.as_deref().and_then(Bounder::exclusive_bound);

        let stakes = self
            .stakes
            .stake
            .prefix(&user)
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .map(|item| {
                item.map(|(validator, stake)| {
                    Ok::<StakeInfo, ContractError>(StakeInfo {
                        owner: user.to_string(),
                        validator,
                        stake,
                    })
                })?
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = StakesResponse { stakes };

        Ok(resp)
    }

    /// Queries a pending tx.
    #[msg(query)]
    fn pending_tx(&self, ctx: QueryCtx, tx_id: u64) -> Result<TxResponse, ContractError> {
        let resp = self.pending_txs.load(ctx.deps.storage, tx_id)?;
        Ok(resp)
    }

    /// Queries for all pending txs.
    /// Reports txs in descending order (newest first).
    /// `start_after` is the last tx id included in previous page
    #[msg(query)]
    fn all_pending_txs_desc(
        &self,
        ctx: QueryCtx,
        start_after: Option<u64>,
        limit: Option<u32>,
    ) -> Result<AllTxsResponse, ContractError> {
        let limit = clamp_page_limit(limit);
        let bound = start_after.and_then(Bounder::exclusive_bound);

        let txs = self
            .pending_txs
            .range(ctx.deps.storage, None, bound, Order::Descending)
            .map(|item| {
                let (_id, tx) = item?;
                Ok::<TxResponse, ContractError>(tx)
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = AllTxsResponse { txs };

        Ok(resp)
    }

    /// Returns how much rewards are to be withdrawn by particular user, from the particular
    /// validator staking
    #[msg(query)]
    pub fn pending_rewards(
        &self,
        ctx: QueryCtx,
        user: String,
        validator: String,
    ) -> Result<PendingRewards, ContractError> {
        let user = ctx.deps.api.addr_validate(&user)?;

        let stake = self
            .stakes
            .stake
            .may_load(ctx.deps.storage, (&user, &validator))?
            .unwrap_or_default();

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(&stake, &distribution)?;
        let config = self.config.load(ctx.deps.storage)?;

        Ok(PendingRewards {
            rewards: coin(amount.u128(), config.rewards_denom),
        })
    }

    /// Returns how much rewards are to be withdrawn by particular user, iterating over all validators.
    /// This is like stakes is to stake query, but for rewards.
    #[msg(query)]
    pub fn all_pending_rewards(
        &self,
        ctx: QueryCtx,
        user: String,
        start_after: Option<String>,
        limit: Option<u32>,
    ) -> Result<AllPendingRewards, ContractError> {
        let limit: usize = clamp_page_limit(limit);
        let user = ctx.deps.api.addr_validate(&user)?;

        let bound = start_after.as_deref().and_then(Bounder::exclusive_bound);

        let config = self.config.load(ctx.deps.storage)?;

        let rewards: Vec<_> = self
            .stakes
            .stake
            .prefix(&user)
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .take(limit)
            .map(|item| {
                let (validator, stake) = item?;
                let distribution = self
                    .distribution
                    .may_load(ctx.deps.storage, &validator)?
                    .unwrap_or_default();
                let amount = Self::calculate_reward(&stake, &distribution)?;
                Ok::<_, ContractError>(ValidatorPendingRewards::new(
                    validator,
                    amount.u128(),
                    &config.rewards_denom,
                ))
            })
            .collect::<Result<_, _>>()?;

        Ok(AllPendingRewards { rewards })
    }

    /// Calculates reward for the user basing on the `Stake` he want to withdraw rewards from, and
    /// the corresponding validator `Distribution`.
    //
    // It is important to make sure the distribution passed matches the validator for stake. It
    // could be enforced by taking user and validator in arguments, then fetching data, but
    // sometimes data are used also for different calculations so we want to avoid double
    // fetching.
    fn calculate_reward(
        stake: &Stake,
        distribution: &Distribution,
    ) -> Result<Uint128, ContractError> {
        // Calculating rewards with always the `low` value of the range goes against the user in some
        // scenario (pending unstakes), but the possible errors are small and temporary.
        let points = distribution.points_per_stake * Uint256::from(stake.stake.low());

        let points = stake.points_alignment.align(points);
        let total = Uint128::try_from(points / DISTRIBUTION_POINTS_SCALE)?;

        Ok(total - stake.withdrawn_funds)
    }
}

pub mod cross_staking {
    use crate::msg::ReceiveVirtualStake;

    use super::*;
    use cosmwasm_std::{from_binary, Binary};
    use mesh_apis::{cross_staking_api::CrossStakingApi, local_staking_api::MaxSlashResponse};

    #[contract(module=crate::contract)]
    #[messages(mesh_apis::cross_staking_api as CrossStakingApi)]
    impl CrossStakingApi for ExternalStakingContract<'_> {
        type Error = ContractError;

        #[msg(exec)]
        fn receive_virtual_stake(
            &self,
            ctx: ExecCtx,
            owner: String,
            amount: Coin,
            tx_id: u64,
            msg: Binary,
        ) -> Result<Response, Self::Error> {
            let config = self.config.load(ctx.deps.storage)?;
            ensure_eq!(ctx.info.sender, config.vault.0, ContractError::Unauthorized);

            // sending proper denom
            ensure_eq!(
                amount.denom,
                config.denom,
                ContractError::InvalidDenom(config.denom)
            );

            let owner = ctx.deps.api.addr_validate(&owner)?;

            // parse and validate message
            let msg: ReceiveVirtualStake = from_binary(&msg)?;
            if !self
                .val_set
                .is_active_validator(ctx.deps.storage, &msg.validator)?
            {
                return Err(ContractError::ValidatorNotActive(msg.validator));
            }
            let mut stake = self
                .stakes
                .stake
                .may_load(ctx.deps.storage, (&owner, &msg.validator))?
                .unwrap_or_default();

            // Prepare stake addition and save stake.
            // We don't check for max here, as this call can only come from the `vault` contract, which already
            // performed the proper check.
            stake.stake.prepare_add(amount.amount, None)?;
            self.stakes
                .stake
                .save(ctx.deps.storage, (&owner, &msg.validator), &stake)?;

            // Save tx
            let new_tx = Tx::InFlightRemoteStaking {
                id: tx_id,
                amount: amount.amount,
                user: owner.clone(),
                validator: msg.validator.clone(),
            };
            self.pending_txs.save(ctx.deps.storage, tx_id, &new_tx)?;

            let mut resp = Response::new();

            let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
            let packet = ProviderPacket::Stake {
                validator: msg.validator,
                stake: amount.clone(),
                tx_id,
            };
            let msg = IbcMsg::SendPacket {
                channel_id: channel.endpoint.channel_id,
                data: to_binary(&packet)?,
                timeout: packet_timeout(&ctx.env),
            };
            // add ibc packet if we are ibc enabled (skip in tests)
            #[cfg(not(any(feature = "mt", test)))]
            {
                resp = resp.add_message(msg);
            }
            #[cfg(any(feature = "mt", test))]
            {
                let _ = msg;
            }

            resp = resp
                .add_attribute("action", "receive_virtual_stake")
                .add_attribute("owner", owner)
                .add_attribute("amount", amount.amount.to_string())
                .add_attribute("tx_id", tx_id.to_string());

            Ok(resp)
        }

        #[msg(exec)]
        fn burn_virtual_stake(
            &self,
            ctx: ExecCtx,
            owner: String,
            amount: Coin,
            validator: Option<String>,
        ) -> Result<Response, Self::Error> {
            let config = self.config.load(ctx.deps.storage)?;
            ensure_eq!(ctx.info.sender, config.vault.0, ContractError::Unauthorized);

            // sending proper denom
            ensure_eq!(
                amount.denom,
                config.denom,
                ContractError::InvalidDenom(config.denom)
            );

            let owner = ctx.deps.api.addr_validate(&owner)?;

            let stakes: Vec<_> = match validator {
                Some(validator) => {
                    // Burn from validator
                    // TODO: Preferentially, i.e. burn remaining amount, if any, from other validators
                    let stake = self
                        .stakes
                        .stake
                        .may_load(ctx.deps.storage, (&owner, &validator))?
                        .unwrap_or_default();
                    vec![(validator, stake.stake.high().u128())] // Burn takes precedence over any pending txs
                }
                None => {
                    // Burn proportionally from all validators associated to the user
                    self.stakes
                        .stake
                        .prefix(&owner)
                        .range(ctx.deps.storage, None, None, Order::Ascending)
                        .map(|item| {
                            let (validator, stake) = item?;
                            Ok::<_, Self::Error>((validator, stake.stake.high().u128()))
                            // Burn takes precedence over any pending txs
                        })
                        .collect::<Result<_, _>>()?
                }
            };

            // Error if no stakes
            if stakes.is_empty() {
                return Err(ContractError::InsufficientDelegations(
                    owner.to_string(),
                    amount.amount,
                ));
            }

            let (burned, burns) = mesh_burn::distribute_burn(&stakes, amount.amount.u128());

            // Bail if we don't have enough stake
            if burned < amount.amount.u128() {
                return Err(ContractError::InsufficientDelegations(
                    owner.to_string(),
                    amount.amount,
                ));
            }

            for (validator, burn_amount) in &burns {
                let burn_amount = Uint128::new(*burn_amount);
                // Perform stake subtraction
                let mut stake = self
                    .stakes
                    .stake
                    .load(ctx.deps.storage, (&owner, validator))?;
                // We don't check for min here, as this call can only come from the `vault` contract, which already
                // performed the proper check.
                stake.stake.sub(burn_amount, None)?;

                // Load distribution
                let mut distribution = self
                    .distribution
                    .may_load(ctx.deps.storage, validator)?
                    .unwrap_or_default();

                // Distribution alignment
                stake
                    .points_alignment
                    .stake_decreased(burn_amount, distribution.points_per_stake);
                distribution.total_stake -= burn_amount;

                // Save stake
                self.stakes
                    .stake
                    .save(ctx.deps.storage, (&owner, validator), &stake)?;

                // Save distribution
                self.distribution
                    .save(ctx.deps.storage, validator, &distribution)?;
            }

            let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
            let packet = ProviderPacket::Burn {
                validators: burns.iter().map(|v| v.0.to_string()).collect(),
                burn: amount.clone(),
            };
            let msg = IbcMsg::SendPacket {
                channel_id: channel.endpoint.channel_id,
                data: to_binary(&packet)?,
                timeout: packet_timeout(&ctx.env),
            };
            let mut resp = Response::new();
            // add ibc packet if we are ibc enabled (skip in tests)
            #[cfg(not(any(feature = "mt", test)))]
            {
                resp = resp.add_message(msg);
            }
            #[cfg(any(feature = "mt", test))]
            {
                let _ = msg;
            }

            resp = resp
                .add_attribute("action", "burn_virtual_stake")
                .add_attribute("owner", owner)
                .add_attribute(
                    "validators",
                    stakes
                        .into_iter()
                        .map(|s| s.0)
                        .collect::<Vec<_>>()
                        .join(", "),
                )
                .add_attribute("amount", amount.to_string());

            Ok(resp)
        }

        #[msg(query)]
        fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, ContractError> {
            let Config { max_slashing, .. } = self.config.load(ctx.deps.storage)?;
            Ok(MaxSlashResponse {
                max_slash: max_slashing,
            })
        }
    }
}

// Some unit tests, to test valset updates and slashing side effects in isolation
#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::{Attribute, Decimal, DepsMut};

    use crate::crdt::State;
    use crate::msg::{AuthorizedEndpoint, ReceiveVirtualStake, ValidatorState};
    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use mesh_apis::cross_staking_api::CrossStakingApi;
    use mesh_apis::vault_api::VaultApiExecMsg::CrossSlash;

    static OSMO: &str = "uosmo";
    static CREATOR: &str = "staking"; // The creator of the proxy contract(s) is the staking contract
    static OWNER: &str = "user";

    fn do_instantiate(deps: DepsMut) -> (ExecCtx, ExternalStakingContract) {
        let contract = ExternalStakingContract::new();
        let mut ctx = InstantiateCtx {
            deps,
            env: mock_env(),
            info: mock_info(CREATOR, &[coin(100, OSMO)]),
        };
        contract
            .instantiate(
                ctx.branch(),
                OSMO.to_owned(),
                "ujuno".to_string(),
                "vault_addr".to_string(),
                100,
                AuthorizedEndpoint {
                    connection_id: "connection_id_1".to_string(),
                    port_id: "port_id_1".to_string(),
                },
                Decimal::percent(10),
            )
            .unwrap();
        let exec_ctx = ExecCtx {
            deps: ctx.deps,
            info: mock_info(OWNER, &[]),
            env: ctx.env,
        };
        (exec_ctx, contract)
    }

    #[test]
    fn instantiating() {
        let mut deps = mock_dependencies();
        let _ = do_instantiate(deps.as_mut());
    }

    #[test]
    fn valset_update_happy_path() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add three new validators, and tombstone one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
            AddValidator {
                valoper: "carl".to_string(),
                pub_key: "carl_pub_key".to_string(),
            },
        ];
        let tombs = vec!["bob".to_string()];

        let (evt, msgs) = contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env,
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(
            evt.attributes,
            vec![
                Attribute::new("additions", "alice,bob,carl"),
                Attribute::new("tombstoned", "bob"),
            ]
        );

        // Check there are no messages (edge case of tombstoning a validator that is not yet active)
        assert!(msgs.is_empty());

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Tombstoned {}
                },
                ValidatorState {
                    validator: "carl".to_string(),
                    state: State::Active {}
                }
            ]
        );
    }

    #[test]
    fn valset_update_tombstoning_slashes() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, tombstone one and check slashing
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Cross stake with bob
        let mut stake_deps = ctx.deps.branch();
        let vault_info = mock_info("vault_addr", &[]);
        let stake_ctx = ExecCtx {
            deps: stake_deps.branch(),
            env: mock_env(),
            info: vault_info,
        };
        contract
            .receive_virtual_stake(
                stake_ctx,
                OWNER.to_string(),
                coin(100, "uosmo"),
                1,
                to_binary(&ReceiveVirtualStake {
                    validator: "bob".to_string(),
                })
                .unwrap(),
            )
            .unwrap();
        // Commit stake
        contract.commit_stake(stake_deps, 1).unwrap();

        // Bob is tombstoned next
        let update_ctx = ctx.branch();
        let tombs = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("tombstoned", "bob"),]);

        // Check the slashing message
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0],
            WasmMsg::Execute {
                contract_addr: "vault_addr".to_string(),
                msg: to_binary(&CrossSlash {
                    slashes: vec![SlashInfo {
                        user: OWNER.to_string(),
                        slash: Uint128::new(10),
                    }],
                    validator: "bob".to_string(),
                })
                .unwrap(),
                funds: vec![],
            }
        );

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Tombstoned {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_tombstoning_slashes_pending_bond() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and tombstone one with a pending bond
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Cross stake with bob
        let mut stake_deps = ctx.deps.branch();
        let vault_info = mock_info("vault_addr", &[]);
        let stake_ctx = ExecCtx {
            deps: stake_deps.branch(),
            env: mock_env(),
            info: vault_info,
        };
        contract
            .receive_virtual_stake(
                stake_ctx,
                OWNER.to_string(),
                coin(100, "uosmo"),
                1,
                to_binary(&ReceiveVirtualStake {
                    validator: "bob".to_string(),
                })
                .unwrap(),
            )
            .unwrap();
        // Stake tx is pending

        // Bob is tombstoned next
        let update_ctx = ctx.branch();
        let tombs = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("tombstoned", "bob"),]);

        // Check the slashing message
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0],
            WasmMsg::Execute {
                contract_addr: "vault_addr".to_string(),
                msg: to_binary(&CrossSlash {
                    slashes: vec![SlashInfo {
                        user: OWNER.to_string(),
                        slash: Uint128::new(10),
                    }],
                    validator: "bob".to_string(),
                })
                .unwrap(),
                funds: vec![],
            }
        );

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Tombstoned {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_tombstoning_slashes_pending_unbond() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and tombstone one with a pending unbond
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Cross stake with bob
        let mut stake_deps = ctx.deps.branch();
        let vault_info = mock_info("vault_addr", &[]);
        let stake_ctx = ExecCtx {
            deps: stake_deps.branch(),
            env: mock_env(),
            info: vault_info,
        };
        contract
            .receive_virtual_stake(
                stake_ctx,
                OWNER.to_string(),
                coin(100, "uosmo"),
                1,
                to_binary(&ReceiveVirtualStake {
                    validator: "bob".to_string(),
                })
                .unwrap(),
            )
            .unwrap();
        // Commit stake
        contract.commit_stake(stake_deps, 1).unwrap();

        // OWNER then cross-unstakes half of the stake
        let mut stake_deps = ctx.deps.branch();
        let owner_info = mock_info(OWNER, &[]);
        let stake_ctx = ExecCtx {
            deps: stake_deps.branch(),
            env: mock_env(),
            info: owner_info,
        };
        contract
            .unstake(stake_ctx, "bob".to_string(), coin(50, "uosmo"))
            .unwrap();
        // Unstake tx is pending

        // Bob is tombstoned next
        let update_ctx = ctx.branch();
        let tombs = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("tombstoned", "bob"),]);

        // Check the slashing message
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0],
            WasmMsg::Execute {
                contract_addr: "vault_addr".to_string(),
                msg: to_binary(&CrossSlash {
                    slashes: vec![SlashInfo {
                        user: OWNER.to_string(),
                        slash: Uint128::new(10), // Owner is slashed over the full stake, including pending
                    }],
                    validator: "bob".to_string(),
                })
                .unwrap(),
                funds: vec![],
            }
        );

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Tombstoned {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_tombstoning_slashes_no_stake() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and tombstone one unbonded one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();
        // Bob has no cross-delegations (which can be possible)

        // Bob is tombstoned next
        let update_ctx = ctx.branch();
        let tombs = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("tombstoned", "bob"),]);

        // Check that no slashing message is emitted
        assert_eq!(msgs.len(), 0);

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Tombstoned {}
                },
            ]
        );
    }

    /// Tombstoning of a nonexistent validator is supported for robustness.
    /// No slashing is performed (non-existent validator assumed to be inactive).
    #[test]
    fn valset_update_tombstoning_nonexistent_validator() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and tombstone no-one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Carl is tombstoned next
        let update_ctx = ctx.branch();
        let tombs = vec!["carl".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &[],
                &tombs,
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("tombstoned", "carl"),]);

        // Check that no slashing message is emitted
        assert_eq!(msgs.len(), 0);

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "carl".to_string(),
                    state: State::Tombstoned {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_unjailing_noop() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and jail/unjail one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Bob is jailed next
        let update_ctx = ctx.branch();
        let jails = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &jails,
                &[],
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("jailed", "bob"),]);

        // Check there's no message
        assert_eq!(msgs.len(), 0);

        // Bob is unjailed next
        let update_ctx = ctx.branch();
        let unjails = vec!["bob".to_string()];
        let (evt, _msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &[],
                &unjails,
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("unjailed", "bob"),]);

        // Check that unjail is a no-op
        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Jailed {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_jailing_slashes() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and jail one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Cross stake with bob
        let mut stake_deps = ctx.deps.branch();
        let vault_info = mock_info("vault_addr", &[]);
        let stake_ctx = ExecCtx {
            deps: stake_deps.branch(),
            env: mock_env(),
            info: vault_info,
        };
        contract
            .receive_virtual_stake(
                stake_ctx,
                OWNER.to_string(),
                coin(100, "uosmo"),
                1,
                to_binary(&ReceiveVirtualStake {
                    validator: "bob".to_string(),
                })
                .unwrap(),
            )
            .unwrap();
        // Commit stake
        contract.commit_stake(stake_deps, 1).unwrap();

        // Bob is jailed next
        let update_ctx = ctx.branch();
        let jails = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &[],
                &[],
                &jails,
                &[],
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("jailed", "bob"),]);

        // Check the slashing message
        assert_eq!(msgs.len(), 1);
        assert_eq!(
            msgs[0],
            WasmMsg::Execute {
                contract_addr: "vault_addr".to_string(),
                msg: to_binary(&CrossSlash {
                    slashes: vec![SlashInfo {
                        user: OWNER.to_string(),
                        slash: Uint128::new(10),
                    }],
                    validator: "bob".to_string(),
                })
                .unwrap(),
                funds: vec![],
            }
        );

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Jailed {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_removal_works() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and remove one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Bob is removed next
        let update_ctx = ctx.branch();
        let rems = vec!["bob".to_string()];
        let (evt, msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &rems,
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("removals", "bob"),]);

        // Check there's no slashing message
        assert_eq!(msgs.len(), 0);

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Unbonded {}
                },
            ]
        );
    }

    #[test]
    fn valset_update_updates_keep_state() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // We add two new validators, and remove/update one
        let adds = vec![
            AddValidator {
                valoper: "alice".to_string(),
                pub_key: "alice_pub_key".to_string(),
            },
            AddValidator {
                valoper: "bob".to_string(),
                pub_key: "bob_pub_key".to_string(),
            },
        ];

        contract
            .valset_update(
                ctx.deps.branch(),
                ctx.env.clone(),
                100,
                1234,
                &adds,
                &[],
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Bob is removed next
        let update_ctx = ctx.branch();
        let rems = vec!["bob".to_string()];
        let (evt, _msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                200,
                2345,
                &[],
                &rems,
                &[],
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("removals", "bob"),]);

        // Bob is updated next
        let update_ctx = ctx.branch();
        let upds = vec![AddValidator {
            valoper: "bob".to_string(),
            pub_key: "bob_pub_key_updated".to_string(),
        }];
        let (evt, _msgs) = contract
            .valset_update(
                update_ctx.deps,
                update_ctx.env,
                300,
                3456,
                &[],
                &[],
                &upds,
                &[],
                &[],
                &[],
            )
            .unwrap();

        // Check the event
        assert_eq!(evt.attributes, vec![Attribute::new("updated", "bob"),]);

        let query_deps = ctx.deps;
        let query_ctx = QueryCtx {
            deps: query_deps.as_ref(),
            env: mock_env(),
        };

        // Check that bob is still unbonded
        let vals = contract.list_validators(query_ctx, None, None).unwrap();
        assert_eq!(
            vals.validators,
            vec![
                ValidatorState {
                    validator: "alice".to_string(),
                    state: State::Active {}
                },
                ValidatorState {
                    validator: "bob".to_string(),
                    state: State::Unbonded {}
                },
            ]
        );
    }
}

use cosmwasm_std::{
    coin, coins, ensure, ensure_eq, from_binary, Addr, BankMsg, Binary, Coin, Decimal, DepsMut,
    Env, Order, Response, StdResult, Storage, Uint128, Uint256, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use cw_utils::must_pay;
use mesh_apis::cross_staking_api::{self, CrossStakingApi};
use mesh_apis::local_staking_api::MaxSlashResponse;
use mesh_apis::vault_api::VaultApiHelper;
use mesh_sync::Lockable;

// IBC sending is disabled in tests...
#[cfg(not(test))]
use crate::ibc::{packet_timeout, IBC_CHANNEL};
#[cfg(not(test))]
use cosmwasm_std::{to_binary, IbcMsg};
#[cfg(not(test))]
use mesh_apis::ibc::ProviderPacket;

use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

use crate::error::ContractError;
use crate::ibc::VAL_CRDT;
use crate::msg::{
    AllTxsResponse, AuthorizedEndpointResponse, ConfigResponse, IbcChannelResponse,
    ListRemoteValidatorsResponse, PendingRewards, ReceiveVirtualStake, StakeInfo, StakesResponse,
    TxResponse,
};
use crate::state::{Config, Distribution, Stake};
use mesh_sync::Tx;

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
    pub stakes: Map<'a, (&'a Addr, &'a str), Lockable<Stake>>,
    /// Per-validator distribution information
    pub distribution: Map<'a, &'a str, Distribution>,
    /// Pending txs information
    pub tx_count: Item<'a, u64>,
    pub pending_txs: Map<'a, u64, Tx>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(cross_staking_api as CrossStakingApi)]
impl ExternalStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            stakes: Map::new("stakes"),
            distribution: Map::new("distribution"),
            pending_txs: Map::new("pending_txs"),
            tx_count: Item::new("tx_count"),
        }
    }

    pub fn next_tx_id(&self, store: &mut dyn Storage) -> StdResult<u64> {
        let id: u64 = self.tx_count.may_load(store)?.unwrap_or(u64::MAX >> 1) + 1;
        self.tx_count.save(store, &id)?;
        Ok(id)
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        rewards_denom: String,
        vault: String,
        unbonding_period: u64,
        remote_contact: crate::msg::AuthorizedEndpoint,
    ) -> Result<Response, ContractError> {
        let vault = ctx.deps.api.addr_validate(&vault)?;
        let vault = VaultApiHelper(vault);

        let config = Config {
            denom,
            rewards_denom,
            vault,
            unbonding_period,
        };

        self.config.save(ctx.deps.storage, &config)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        remote_contact.validate()?;
        crate::ibc::AUTH_ENDPOINT.save(ctx.deps.storage, &remote_contact)?;

        Ok(Response::new())
    }

    pub(crate) fn commit_stake(&self, deps: DepsMut, tx_id: u64) -> Result<WasmMsg, ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;
        println!("tx: {:?}", tx);

        // TODO: Verify tx comes from the right context
        // Verify tx is the right type
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
        let mut stake_lock = self.stakes.load(deps.storage, (&tx_user, &tx_validator))?;

        // Load distribution
        let mut distribution = self
            .distribution
            .may_load(deps.storage, &tx_validator)?
            .unwrap_or_default();

        // Commit amount (need to unlock it first)
        stake_lock.unlock_write()?;
        let stake = stake_lock.write()?;
        stake.stake += tx_amount;

        // Commit distribution (need to unlock it first)
        // Distribution alignment
        stake
            .points_alignment
            .stake_increased(tx_amount, distribution.points_per_stake);
        distribution.total_stake += tx_amount;

        // Save stake
        self.stakes
            .save(deps.storage, (&tx_user, &tx_validator), &stake_lock)?;

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

    /// In test code, this is called from test_rollback_stake.
    /// In non-test code, this is called from ibc_packet_ack or ibc_packet_timeout
    pub(crate) fn rollback_stake(
        &self,
        deps: DepsMut,
        tx_id: u64,
    ) -> Result<WasmMsg, ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // TODO: Verify tx comes from the right context
        // Verify tx is the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteStaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (_tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteStaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        // Load stake
        let mut stake_lock = self.stakes.load(deps.storage, (&tx_user, &tx_validator))?;

        // Release stake lock
        stake_lock.unlock_write()?;

        // Save stake
        self.stakes
            .save(deps.storage, (&tx_user, &tx_validator), &stake_lock)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);

        // Call rollback hook on vault
        let cfg = self.config.load(deps.storage)?;
        let msg = cfg.vault.rollback_tx(tx_id)?;
        Ok(msg)
    }

    /// Commits a pending stake.
    /// Must be called by the IBC callback handler on successful remote staking.
    #[msg(exec)]
    fn test_commit_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(test)]
        {
            let msg = self.commit_stake(ctx.deps, tx_id)?;
            Ok(Response::new().add_message(msg))
        }
        #[cfg(not(test))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Rollbacks a pending stake.
    /// Must be called by the IBC callback handler on failed remote staking.
    #[msg(exec)]
    fn test_rollback_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(test)]
        {
            let msg = self.rollback_stake(ctx.deps, tx_id)?;
            Ok(Response::new().add_message(msg))
        }
        #[cfg(not(test))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Schedules tokens for release, adding them to the pending unbonds. After `unbonding_period`
    /// passes, funds are ready to be released with `withdraw_unbonded` call by the user
    #[msg(exec)]
    pub fn unstake(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;

        ensure_eq!(
            amount.denom,
            config.denom,
            ContractError::InvalidDenom(config.denom)
        );

        let mut stake_lock = self
            .stakes
            .may_load(ctx.deps.storage, (&ctx.info.sender, &validator))?
            .unwrap_or_default();
        let stake = stake_lock.read()?;

        ensure!(
            stake.stake >= amount.amount,
            ContractError::NotEnoughStake(stake.stake)
        );

        stake_lock.lock_write()?;
        self.stakes.save(
            ctx.deps.storage,
            (&ctx.info.sender, &validator),
            &stake_lock,
        )?;

        // Create new tx
        let tx_id = self.next_tx_id(ctx.deps.storage)?;

        // Save tx
        #[allow(clippy::redundant_clone)]
        let new_tx = Tx::InFlightRemoteUnstaking {
            id: tx_id,
            amount: amount.amount,
            user: ctx.info.sender.clone(),
            validator: validator.clone(),
        };
        self.pending_txs.save(ctx.deps.storage, tx_id, &new_tx)?;

        let mut resp = Response::new()
            .add_attribute("action", "unstake")
            .add_attribute("amount", amount.amount.to_string());

        // add ibc packet if we are ibc enabled (skip in tests)
        #[cfg(not(test))]
        {
            let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
            let packet = ProviderPacket::Unstake {
                validator,
                unstake: amount,
                tx_id,
            };
            let msg = IbcMsg::SendPacket {
                channel_id: channel.endpoint.channel_id,
                data: to_binary(&packet)?,
                timeout: packet_timeout(&ctx.env),
            };
            resp = resp.add_message(msg);
        }

        // put this later so compiler doens't complain about mut in test mode
        resp = resp.add_attribute("owner", ctx.info.sender);

        Ok(resp)
    }

    pub(crate) fn commit_unstake(
        &self,
        deps: DepsMut,
        env: Env,
        tx_id: u64,
    ) -> Result<(), ContractError> {
        use crate::state::PendingUnbond;

        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // TODO: Verify tx comes from the right context
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
        let mut stake_lock = self.stakes.load(deps.storage, (&tx_user, &tx_validator))?;

        // Load distribution
        let mut distribution = self
            .distribution
            .may_load(deps.storage, &tx_validator)?
            .unwrap_or_default();

        // Commit amount (need to unlock it first)
        stake_lock.unlock_write()?;
        let stake = stake_lock.write()?;
        stake.stake -= tx_amount;

        // FIXME? Release period being computed after successful IBC tx
        let release_at = env.block.time.plus_seconds(config.unbonding_period);
        let unbond = PendingUnbond {
            amount: tx_amount,
            release_at,
        };
        stake.pending_unbonds.push(unbond);

        // Distribution alignment
        stake
            .points_alignment
            .stake_decreased(tx_amount, distribution.points_per_stake);
        distribution.total_stake -= tx_amount;

        // Save stake
        self.stakes
            .save(deps.storage, (&tx_user, &tx_validator), &stake_lock)?;

        // Save distribution
        self.distribution
            .save(deps.storage, &tx_validator, &distribution)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);
        Ok(())
    }

    /// In test code, this is called from test_rollback_unstake.
    /// In non-test code, this is called from ibc_packet_ack or ibc_packet_timeout
    pub(crate) fn rollback_unstake(&self, deps: DepsMut, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending_txs.load(deps.storage, tx_id)?;

        // TODO: Verify tx comes from the right context
        // Verify tx is of the right type
        ensure!(
            matches!(tx, Tx::InFlightRemoteUnstaking { .. }),
            ContractError::WrongTypeTx(tx_id, tx)
        );
        let (_tx_amount, tx_user, tx_validator) = match tx {
            Tx::InFlightRemoteUnstaking {
                amount,
                user,
                validator,
                ..
            } => (amount, user, validator),
            _ => unreachable!(),
        };

        // Load stake
        let mut stake_lock = self.stakes.load(deps.storage, (&tx_user, &tx_validator))?;

        // Release stake lock
        stake_lock.unlock_write()?;

        // Save stake
        self.stakes
            .save(deps.storage, (&tx_user, &tx_validator), &stake_lock)?;

        // Remove tx
        self.pending_txs.remove(deps.storage, tx_id);
        Ok(())
    }

    /// Commits a pending unstake.
    /// Must be called by the IBC callback handler on successful remote unstaking.
    #[msg(exec)]
    fn test_commit_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(test)]
        {
            self.commit_unstake(ctx.deps, ctx.env, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(test))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Rollbacks a pending unstake.
    /// Must be called by the IBC callback handler on failed remote unstaking.
    #[msg(exec)]
    fn test_rollback_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(test)]
        {
            self.rollback_unstake(ctx.deps, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(test))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Withdraws all released tokens to the sender.
    ///
    /// Tokens to be claimed has to be unbond before by calling the `unbond` message and
    /// waiting the `unbond_period`
    #[msg(exec)]
    pub fn withdraw_unbonded(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;

        let stake_locks: Vec<_> = self
            .stakes
            .prefix(&ctx.info.sender)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .collect::<Result<_, _>>()?;

        let released: Uint128 = stake_locks
            .into_iter()
            .map(|(validator, mut stake_lock)| -> Result<_, ContractError> {
                let stake = stake_lock.write()?;
                let released = stake.release_pending(&ctx.env.block);

                if !released.is_zero() {
                    self.stakes.save(
                        ctx.deps.storage,
                        (&ctx.info.sender, &validator),
                        &stake_lock,
                    )?
                }

                Ok(released)
            })
            .fold(Ok(Uint128::zero()), |acc, released| {
                let acc = acc?;
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

    /// Distributes reward among users staking via particular validator. Distribution is performend
    /// proportionally to amount of tokens staken by user.
    #[msg(exec)]
    pub fn distribute_rewards(
        &self,
        ctx: ExecCtx,
        validator: String,
    ) -> Result<Response, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        let amount = must_pay(&ctx.info, &config.rewards_denom)?;

        let mut distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let total_stake = Uint256::from(distribution.total_stake);
        let points_distributed =
            Uint256::from(amount) * DISTRIBUTION_POINTS_SCALE + distribution.points_leftover;
        let points_per_stake = points_distributed / total_stake;

        distribution.points_leftover = points_distributed - points_per_stake * total_stake;
        distribution.points_per_stake += points_per_stake;

        self.distribution
            .save(ctx.deps.storage, &validator, &distribution)?;

        let resp = Response::new()
            .add_attribute("action", "distribute_rewards")
            .add_attribute("sender", ctx.info.sender.into_string())
            .add_attribute("validator", validator)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    /// Withdraw rewards from staking via given validator
    #[msg(exec)]
    pub fn withdraw_rewards(
        &self,
        ctx: ExecCtx,
        validator: String,
    ) -> Result<Response, ContractError> {
        let mut stake_lock = self
            .stakes
            .may_load(ctx.deps.storage, (&ctx.info.sender, &validator))?
            .unwrap_or_default();

        let stake = stake_lock.write()?;

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(stake, &distribution)?;

        let mut resp = Response::new()
            .add_attribute("action", "withdraw_rewards")
            .add_attribute("owner", ctx.info.sender.to_string())
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.to_string());

        if !amount.is_zero() {
            stake.withdrawn_funds += amount;

            self.stakes.save(
                ctx.deps.storage,
                (&ctx.info.sender, &validator),
                &stake_lock,
            )?;

            let config = self.config.load(ctx.deps.storage)?;
            let send_msg = BankMsg::Send {
                to_address: ctx.info.sender.into_string(),
                amount: coins(amount.u128(), config.rewards_denom),
            };

            resp = resp.add_message(send_msg);
        }

        Ok(resp)
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
        let channel = crate::ibc::IBC_CHANNEL.load(ctx.deps.storage)?;
        Ok(IbcChannelResponse { channel })
    }

    /// Show all external validators that we know to be active (and can delegate to)
    #[msg(query)]
    pub fn list_remote_validators(
        &self,
        ctx: QueryCtx,
        start_after: Option<String>,
        limit: Option<u64>,
    ) -> Result<ListRemoteValidatorsResponse, ContractError> {
        let limit = limit.unwrap_or(100) as usize;
        let validators =
            VAL_CRDT.list_active_validators(ctx.deps.storage, start_after.as_deref(), limit)?;
        Ok(ListRemoteValidatorsResponse { validators })
    }

    /// Queries for stake info
    ///
    /// If stake is not existing in the system is queried, the zero-stake is returned
    #[msg(query)]
    pub fn stake(
        &self,
        ctx: QueryCtx,
        user: String,
        validator: String,
    ) -> Result<Stake, ContractError> {
        let user = ctx.deps.api.addr_validate(&user)?;
        let stake_lock = self
            .stakes
            .may_load(ctx.deps.storage, (&user, &validator))?
            .unwrap_or_default();
        let stake = stake_lock.read()?;
        Ok(stake.clone())
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
            .prefix(&user)
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .map(|item| {
                item.map(|(validator, stake_lock)| {
                    Ok::<StakeInfo, ContractError>(StakeInfo {
                        owner: user.to_string(),
                        validator,
                        stake: stake_lock.read()?.stake,
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

        let stake_lock = self
            .stakes
            .may_load(ctx.deps.storage, (&user, &validator))?
            .unwrap_or_default();
        let stake = stake_lock.read()?;

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(stake, &distribution)?;
        let config = self.config.load(ctx.deps.storage)?;

        let resp = PendingRewards {
            amount: coin(amount.u128(), config.rewards_denom),
        };

        Ok(resp)
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
        let points = distribution.points_per_stake * Uint256::from(stake.stake);

        let points = stake.points_alignment.align(points);
        let total = Uint128::try_from(points / DISTRIBUTION_POINTS_SCALE)?;

        Ok(total - stake.withdrawn_funds)
    }
}

pub mod cross_staking {
    use super::*;

    #[contract]
    #[messages(cross_staking_api as CrossStakingApi)]
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

            ensure_eq!(
                amount.denom,
                config.denom,
                ContractError::InvalidDenom(config.denom)
            );

            let owner = ctx.deps.api.addr_validate(&owner)?;

            let msg: ReceiveVirtualStake = from_binary(&msg)?;
            let mut stake_lock = self
                .stakes
                .may_load(ctx.deps.storage, (&owner, &msg.validator))?
                .unwrap_or_default();

            // Write lock and save stake and distribution
            stake_lock.lock_write()?;
            self.stakes
                .save(ctx.deps.storage, (&owner, &msg.validator), &stake_lock)?;

            // Save tx
            #[allow(clippy::redundant_clone)]
            let new_tx = Tx::InFlightRemoteStaking {
                id: tx_id,
                amount: amount.amount,
                user: owner.clone(),
                validator: msg.validator.clone(),
            };
            self.pending_txs.save(ctx.deps.storage, tx_id, &new_tx)?;

            let mut resp = Response::new()
                .add_attribute("action", "receive_virtual_stake")
                .add_attribute("owner", owner)
                .add_attribute("amount", amount.amount.to_string());

            // add ibc packet if we are ibc enabled (skip in tests)
            #[cfg(not(test))]
            {
                let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
                let packet = ProviderPacket::Stake {
                    validator: msg.validator,
                    stake: amount,
                    tx_id,
                };
                let msg = IbcMsg::SendPacket {
                    channel_id: channel.endpoint.channel_id,
                    data: to_binary(&packet)?,
                    timeout: packet_timeout(&ctx.env),
                };
                resp = resp.add_message(msg);
            }

            // This is later so `mut resp` is valid in test and non-test code
            resp = resp.add_attribute("tx_id", tx_id.to_string());

            Ok(resp)
        }

        #[msg(query)]
        fn max_slash(&self, _ctx: QueryCtx) -> Result<MaxSlashResponse, ContractError> {
            // TODO: Properly set this value
            // Arbitrary value - only to make some testing possible
            //
            // Probably should be queried from remote chain
            let resp = MaxSlashResponse {
                max_slash: Decimal::percent(5),
            };

            Ok(resp)
        }
    }
}

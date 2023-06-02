use std::cmp::Ordering;

use cosmwasm_std::{
    coin, coins, ensure, ensure_eq, from_binary, Addr, BankMsg, Binary, Coin, Decimal, Order,
    Response, StdResult, Uint128, Uint256,
};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use cw_utils::must_pay;
use mesh_apis::cross_staking_api::{self, CrossStakingApi};
use mesh_apis::local_staking_api::MaxSlashResponse;
use mesh_apis::vault_api::VaultApiHelper;

use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, PendingRewards, ReceiveVirtualStake, StakeInfo, StakesResponse};
use crate::state::{Config, Distribution, PendingUnbond, Stake};

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
    // Stakes indexed by `(owner, validator)` pair
    pub stakes: Map<'a, (&'a Addr, &'a str), Stake>,
    // Per-validator distribution information
    pub distribution: Map<'a, &'a str, Distribution>,
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
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        rewards_denom: String,
        vault: String,
        unbonding_period: u64,
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

        Ok(Response::new())
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

        let mut stake = self
            .stakes
            .may_load(ctx.deps.storage, (&ctx.info.sender, &validator))?
            .unwrap_or_default();

        let mut distribution = self.distribution.load(ctx.deps.storage, &validator)?;

        ensure!(
            stake.stake >= amount.amount,
            ContractError::NotEnoughStake(stake.stake)
        );

        stake.stake -= amount.amount;

        let release_at = ctx.env.block.time.plus_seconds(config.unbonding_period);
        let unbond = PendingUnbond {
            amount: amount.amount,
            release_at,
        };
        stake.pending_unbonds.push(unbond);

        // Distribution alignment
        stake.points_alignment += Uint256::from(amount.amount) * distribution.points_per_stake;
        distribution.total_stake -= amount.amount;

        self.stakes
            .save(ctx.deps.storage, (&ctx.info.sender, &validator), &stake)?;

        self.distribution
            .save(ctx.deps.storage, &validator, &distribution)?;

        // TODO:
        //
        // Probably some more communication with remote via IBC should happen here?
        // Or maybe this contract should be called via IBC here? To be specified
        let resp = Response::new()
            .add_attribute("action", "unstake")
            .add_attribute("owner", ctx.info.sender.into_string())
            .add_attribute("amount", amount.amount.to_string());

        Ok(resp)
    }

    /// Withdraws all released tokens to the sender.
    ///
    /// Tokens to be claimed has to be unbond before by calling the `unbond` message and
    /// waiting the `unbond_period`
    #[msg(exec)]
    pub fn withdraw_unbonded(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;

        let stakes: Vec<_> = self
            .stakes
            .prefix(&ctx.info.sender)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .collect::<Result<_, _>>()?;

        let released: Uint128 = stakes
            .into_iter()
            .map(|(validator, mut stake)| -> StdResult<_> {
                let released = stake.release_pending(&ctx.env.block);

                if !released.is_zero() {
                    self.stakes
                        .save(ctx.deps.storage, (&ctx.info.sender, &validator), &stake)?
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
        let mut stake = self
            .stakes
            .may_load(ctx.deps.storage, (&ctx.info.sender, &validator))?
            .unwrap_or_default();

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(&stake, &distribution)?;

        let mut resp = Response::new()
            .add_attribute("action", "withdraw_rewards")
            .add_attribute("owner", ctx.info.sender.to_string())
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.to_string());

        if !amount.is_zero() {
            stake.withdrawn_funds += amount;

            self.stakes
                .save(ctx.deps.storage, (&ctx.info.sender, &validator), &stake)?;

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
        let stake = self
            .stakes
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
            .prefix(&user)
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .map(|item| {
                item.map(|(validator, stake)| StakeInfo {
                    owner: user.to_string(),
                    validator,
                    stake: stake.stake,
                })
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = StakesResponse { stakes };

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
            .may_load(ctx.deps.storage, (&user, &validator))?
            .unwrap_or_default();

        let distribution = self
            .distribution
            .may_load(ctx.deps.storage, &validator)?
            .unwrap_or_default();

        let amount = Self::calculate_reward(&stake, &distribution)?;
        let config = self.config.load(ctx.deps.storage)?;

        let resp = PendingRewards {
            amount: coin(amount.u128(), &config.rewards_denom),
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

        // Points alignment application - way very much around because of offset of `Uint256` we
        // used
        let points = match stake
            .points_alignment
            .cmp(&(Uint256::MAX / Uint256::from_u128(2)))
        {
            // Points alignemnt negative - first we need to add alignment and then add offset
            // to avoid exceeding limit
            Ordering::Less => {
                points + stake.points_alignment - Uint256::MAX / Uint256::from_u128(2)
            }
            // Points alignment is positive - first we reduce it by offset and then add to the
            // poits
            Ordering::Greater => {
                points + (stake.points_alignment - Uint256::MAX / Uint256::from_u128(2))
            }
            // Alignment is `0`, no math to be done
            Ordering::Equal => points,
        };

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
            let mut stake = self
                .stakes
                .may_load(ctx.deps.storage, (&owner, &msg.validator))?
                .unwrap_or_default();

            let mut distribution = self
                .distribution
                .may_load(ctx.deps.storage, &msg.validator)?
                .unwrap_or_default();

            stake.stake += amount.amount;

            // Distribution alignment
            stake.points_alignment -= Uint256::from(amount.amount) * distribution.points_per_stake;
            distribution.total_stake += amount.amount;

            self.stakes
                .save(ctx.deps.storage, (&owner, &msg.validator), &stake)?;

            self.distribution
                .save(ctx.deps.storage, &msg.validator, &distribution)?;

            // TODO: Send proper IBC message to remote staking contract

            let resp = Response::new()
                .add_attribute("action", "receive_virtual_stake")
                .add_attribute("owner", owner)
                .add_attribute("amount", amount.amount.to_string());

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

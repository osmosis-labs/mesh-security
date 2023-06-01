use cosmwasm_std::{coin, ensure, ensure_eq, Addr, Coin, Order, Response};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use mesh_apis::cross_staking_api;
use mesh_apis::vault_api::VaultApiHelper;
use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, UserInfo, UsersResponse};
use crate::state::{Config, PendingUnbond, User};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const DEFAULT_PAGE_LIMIT: u32 = 10;
pub const MAX_PAGE_LIMIT: u32 = 30;

/// Aligns pagination limit
fn clamp_page_limit(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).max(MAX_PAGE_LIMIT) as usize
}

pub struct ExternalStakingContract<'a> {
    pub config: Item<'a, Config>,
    pub users: Map<'a, &'a Addr, User>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(cross_staking_api as CrossStakingApi)]
impl ExternalStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            users: Map::new("users"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        vault: String,
        unbonding_period: u64,
    ) -> Result<Response, ContractError> {
        let vault = ctx.deps.api.addr_validate(&vault)?;
        let vault = VaultApiHelper(vault);

        let config = Config {
            denom,
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
    pub fn unstake(&self, ctx: ExecCtx, amount: Coin) -> Result<Response, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;

        ensure_eq!(
            amount.denom,
            config.denom,
            ContractError::InvalidDenom(config.denom)
        );

        let mut user = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();

        ensure!(
            user.stake >= amount.amount,
            ContractError::NotEnoughStake(user.stake)
        );

        user.stake -= amount.amount;

        let release_at = ctx.env.block.time.plus_seconds(config.unbonding_period);
        let unbond = PendingUnbond {
            amount: amount.amount,
            release_at,
        };
        user.pending_unbonds.push(unbond);

        self.users.save(ctx.deps.storage, &ctx.info.sender, &user)?;

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

        let mut user = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();

        let released = user.release_pending(&ctx.env.block);
        self.users.save(ctx.deps.storage, &ctx.info.sender, &user)?;

        let release_msg = config.vault.release_cross_stake(
            ctx.info.sender.to_string(),
            coin(released.u128(), &config.denom),
            vec![],
        )?;

        let resp = Response::new()
            .add_message(release_msg)
            .add_attribute("action", "withdraw_unbonded")
            .add_attribute("owner", ctx.info.sender.into_string())
            .add_attribute("amount", released.to_string());

        Ok(resp)
    }

    /// Queries for contract configuration
    #[msg(query)]
    pub fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let resp = self.config.load(ctx.deps.storage)?.into();
        Ok(resp)
    }

    /// Queries for user-related info
    ///
    /// If user not existin in the system is queried, the default "nothing staken" user
    /// is returned
    #[msg(query)]
    pub fn user(&self, ctx: QueryCtx, user: String) -> Result<User, ContractError> {
        let user = ctx.deps.api.addr_validate(&user)?;
        let user = self
            .users
            .may_load(ctx.deps.storage, &user)?
            .unwrap_or_default();
        Ok(user)
    }

    #[msg(query)]
    pub fn users(
        &self,
        ctx: QueryCtx,
        start_after: Option<String>,
        limit: Option<u32>,
    ) -> Result<UsersResponse, ContractError> {
        let limit = clamp_page_limit(limit);

        let start_after = start_after.map(Addr::unchecked);
        let bound = start_after.as_ref().and_then(Bounder::exclusive_bound);

        let users = self
            .users
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .map(|item| {
                item.map(|(addr, user)| UserInfo {
                    addr: addr.into(),
                    stake: user.stake,
                    pending_unbonds: user.pending_unbonds,
                })
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = UsersResponse { users };

        Ok(resp)
    }
}

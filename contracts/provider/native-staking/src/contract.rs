use cosmwasm_std::Order::Ascending;
use cosmwasm_std::{
    from_json, Addr, Binary, Decimal, DepsMut, Event, Reply, Response, StdResult, SubMsgResponse, SubMsgResult, WasmMsg
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::parse_instantiate_response_data;
use sylvia::ctx::{ExecCtx, InstantiateCtx, QueryCtx, SudoCtx};
#[allow(deprecated)]
use sylvia::types::ReplyCtx;
use sylvia::{contract, schemars};

use mesh_apis::local_staking_api;
use mesh_apis::vault_api::{SlashInfo, VaultApiHelper};
use mesh_native_staking_proxy::msg::OwnerMsg;
use mesh_native_staking_proxy::native_staking_callback;

use crate::error::ContractError;
use crate::msg::{ConfigResponse, OwnerByProxyResponse, ProxyByOwnerResponse};
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 2;

pub struct NativeStakingContract {
    pub config: Item<Config>,
    /// Map of proxy contract address by owner address
    pub proxy_by_owner: Map<Addr, Addr>,
    /// Reverse map of owner address by proxy contract address
    pub owner_by_proxy: Map<Addr, Addr>,
    /// Map of delegators per validator
    // This is used for prefixing and ranging during slashing
    pub delegators: Map<(String, Addr), bool>,
}

pub(crate) enum SlashingReason {
    Offline,
    DoubleSign,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
#[sv::messages(local_staking_api as LocalStakingApi)]
#[sv::messages(native_staking_callback as NativeStakingCallback)]
impl NativeStakingContract {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            proxy_by_owner: Map::new("proxies"),
            owner_by_proxy: Map::new("owners"),
            delegators: Map::new("delegators"),
        }
    }

    /// The caller of the instantiation will be the vault contract
    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        proxy_code_id: u64,
        slash_ratio_dsign: Decimal,
        slash_ratio_offline: Decimal,
    ) -> Result<Response, ContractError> {
        if slash_ratio_dsign > Decimal::one() || slash_ratio_offline > Decimal::one() {
            return Err(ContractError::InvalidSlashRatio);
        }

        let config = Config {
            denom,
            proxy_code_id,
            vault: VaultApiHelper(ctx.info.sender),
            slash_ratio_dsign,
            slash_ratio_offline,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    /// This is called every time there's a change of the active validator set that implies slashing.
    /// In test code, this is called from `test_handle_jailing`.
    /// In non-test code, this is called from `sudo`.
    fn handle_jailing(
        &self,
        mut deps: DepsMut,
        jailed: Option<Vec<String>>,
        tombstoned: Option<Vec<String>>,
    ) -> Result<Response, ContractError> {
        let jailed = &jailed.unwrap_or_default();
        let tombstoned = &tombstoned.unwrap_or_default();

        let cfg = self.config.load(deps.storage)?;
        let mut msgs = vec![];
        for validator in tombstoned {
            // Slash the validator (if bonded)
            let slash_msg =
                self.handle_slashing(&mut deps, &cfg, validator, SlashingReason::DoubleSign)?;
            if let Some(msg) = slash_msg {
                msgs.push(msg)
            }
        }
        for validator in jailed {
            // Slash the validator (if bonded)
            let slash_msg =
                self.handle_slashing(&mut deps, &cfg, validator, SlashingReason::Offline)?;
            if let Some(msg) = slash_msg {
                msgs.push(msg)
            }
        }
        let mut evt = Event::new("jailing");
        if !jailed.is_empty() {
            evt = evt.add_attribute("jailed", jailed.join(","));
        }
        if !tombstoned.is_empty() {
            evt = evt.add_attribute("tombstoned", tombstoned.join(","));
        }
        Ok(Response::new().add_event(evt).add_messages(msgs))
    }

    pub(crate) fn handle_slashing(
        &self,
        deps: &mut DepsMut,
        config: &Config,
        validator: &str,
        reason: SlashingReason,
    ) -> Result<Option<WasmMsg>, ContractError> {
        let slash_ratio = match reason {
            SlashingReason::Offline => config.slash_ratio_offline,
            SlashingReason::DoubleSign => config.slash_ratio_dsign,
        };
        // Get all mesh delegators to this validator
        let owners = self
            .delegators
            .prefix(validator.to_string())
            .range(deps.storage, None, None, Ascending)
            .collect::<StdResult<Vec<_>>>()?;
        if owners.is_empty() {
            return Ok(None);
        }

        let mut slash_infos = vec![];
        for (owner, _) in &owners {
            // Get owner proxy address
            let proxy = self.proxy_by_owner.load(deps.storage, owner.clone())?;
            // Get proxy's delegation (pre-slashing?) amount over validator
            // TODO: Confirm queried delegation amounts are pre- or post-slashing
            let delegation = deps
                .querier
                .query_delegation(proxy, validator)?
                .map(|full_delegation| full_delegation.amount.amount)
                .unwrap_or_default();

            if delegation.is_zero() {
                // Maintenance: Remove delegator from map in passing
                self.delegators.remove(deps.storage, (validator.to_string(), owner.clone()));
                continue;
            }

            let slash_amount = delegation.mul_floor(slash_ratio);

            slash_infos.push(SlashInfo {
                user: owner.to_string(),
                slash: slash_amount,
            });
        }
        if slash_infos.is_empty() {
            return Ok(None);
        }
        // Route associated users to vault for slashing of their collateral
        let msg = config
            .vault
            .process_local_slashing(slash_infos, validator)?;
        Ok(Some(msg))
    }

    #[sv::msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        self.config.load(ctx.deps.storage).map_err(Into::into)
    }

    #[sv::msg(reply)]
    #[allow(deprecated)]
    fn reply(&self, ctx: ReplyCtx, reply: Reply) -> Result<Response, ContractError> {
        match reply.id {
            REPLY_ID_INSTANTIATE => self.reply_init_callback(ctx.deps, reply.result.unwrap()),
            _ => Err(ContractError::InvalidReplyId(reply.id)),
        }
    }

    #[allow(deprecated)]
    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;

        // Associate staking proxy with owner address
        let proxy_addr = Addr::unchecked(init_data.contract_address);
        let owner_data: OwnerMsg =
            from_json(init_data.data.ok_or(ContractError::NoInstantiateData {})?)?;
        let owner_addr = deps.api.addr_validate(&owner_data.owner)?;
        self.proxy_by_owner
            .save(deps.storage, owner_addr.clone(), &proxy_addr)?;
        self.owner_by_proxy
            .save(deps.storage, proxy_addr, &owner_addr)?;

        Ok(Response::new())
    }

    #[sv::msg(query)]
    fn proxy_by_owner(
        &self,
        ctx: QueryCtx,
        owner: String,
    ) -> Result<ProxyByOwnerResponse, ContractError> {
        let owner_addr = ctx.deps.api.addr_validate(&owner)?;
        let proxy_addr = self.proxy_by_owner.load(ctx.deps.storage, owner_addr)?;
        Ok(ProxyByOwnerResponse {
            proxy: proxy_addr.to_string(),
        })
    }

    #[sv::msg(query)]
    fn owner_by_proxy(
        &self,
        ctx: QueryCtx,
        proxy: String,
    ) -> Result<OwnerByProxyResponse, ContractError> {
        let proxy_addr = ctx.deps.api.addr_validate(&proxy)?;
        let owner_addr = self.owner_by_proxy.load(ctx.deps.storage, proxy_addr)?;
        Ok(OwnerByProxyResponse {
            owner: owner_addr.to_string(),
        })
    }

    /// Jails validators temporarily or permanently.
    /// Method used for test only.
    #[sv::msg(exec)]
    fn test_handle_jailing(
        &self,
        ctx: ExecCtx,
        jailed: Vec<String>,
        tombstoned: Vec<String>,
    ) -> Result<Response, ContractError> {
        #[cfg(any(feature = "mt", test))]
        {
            let jailed = if jailed.is_empty() {
                None
            } else {
                Some(jailed)
            };
            let tombstoned = if tombstoned.is_empty() {
                None
            } else {
                Some(tombstoned)
            };
            NativeStakingContract::new().handle_jailing(ctx.deps, jailed, tombstoned)
        }
        #[cfg(not(any(feature = "mt", test)))]
        {
            let _ = (ctx, jailed, tombstoned);
            Err(ContractError::Unauthorized {})
        }
    }

    /// `SudoMsg::Jailing` should be called every time there's a validator set update that implies
    /// slashing.
    ///  - Temporary removal of a validator from the active set due to jailing.
    ///  - Permanent removal (i.e. tombstoning) of a validator from the active set.
    #[sv::msg(sudo)]
    fn jailing(
        &self,
        ctx: SudoCtx,
        jailed: Option<Vec<String>>,
        tombstoned: Option<Vec<String>>,
    ) -> Result<Response, ContractError> {
        self.handle_jailing(ctx.deps, jailed, tombstoned)
    }
}

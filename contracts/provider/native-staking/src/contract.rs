#[cfg(not(feature = "library"))]
use cosmwasm_std::entry_point;

use cosmwasm_std::{from_slice, Addr, Decimal, DepsMut, Env, Reply, Response, SubMsgResponse};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::parse_instantiate_response_data;
use sylvia::types::{InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use mesh_apis::local_staking_api;
use mesh_apis::local_staking_api::SudoMsg;
use mesh_apis::vault_api::VaultApiHelper;
use mesh_native_staking_proxy::msg::OwnerMsg;
use mesh_native_staking_proxy::native_staking_callback;

use crate::error::ContractError;
use crate::msg::{ConfigResponse, OwnerByProxyResponse, ProxyByOwnerResponse};
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 2;

pub struct NativeStakingContract<'a> {
    pub config: Item<'a, Config>,
    /// Map of proxy contract address by owner address
    pub proxy_by_owner: Map<'a, &'a Addr, Addr>,
    /// Reverse map of owner address by proxy contract address
    pub owner_by_proxy: Map<'a, &'a Addr, Addr>,
    /// Map of delegators per validator
    // This is used for prefixing and ranging during slashing
    pub delegators: Map<'a, (&'a str, &'a Addr), bool>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(local_staking_api as LocalStakingApi)]
#[messages(native_staking_callback as NativeStakingCallback)]
impl NativeStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            proxy_by_owner: Map::new("proxies"),
            owner_by_proxy: Map::new("owners"),
            delegators: Map::new("delegators"),
        }
    }

    /// The caller of the instantiation will be the vault contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        proxy_code_id: u64,
        max_slashing: Decimal,
    ) -> Result<Response, ContractError> {
        if max_slashing > Decimal::one() {
            return Err(ContractError::InvalidMaxSlashing);
        }

        let config = Config {
            denom,
            proxy_code_id,
            vault: VaultApiHelper(ctx.info.sender),
            max_slashing,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    /**
     * This is called every time there's a change of the active validator set that implies slashing.
     *
     */
    fn handle_jailing(
        &self,
        deps: DepsMut,
        jailed: &[String],
        tombstoned: &[String],
    ) -> Result<Response, ContractError> {
        let _ = (deps, jailed, tombstoned);
        todo!()
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        self.config.load(ctx.deps.storage).map_err(Into::into)
    }

    #[msg(reply)]
    fn reply(&self, ctx: ReplyCtx, reply: Reply) -> Result<Response, ContractError> {
        match reply.id {
            REPLY_ID_INSTANTIATE => self.reply_init_callback(ctx.deps, reply.result.unwrap()),
            _ => Err(ContractError::InvalidReplyId(reply.id)),
        }
    }

    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;

        // Associate staking proxy with owner address
        let proxy_addr = Addr::unchecked(init_data.contract_address);
        let owner_data: OwnerMsg =
            from_slice(&init_data.data.ok_or(ContractError::NoInstantiateData {})?)?;
        let owner_addr = deps.api.addr_validate(&owner_data.owner)?;
        self.proxy_by_owner
            .save(deps.storage, &owner_addr, &proxy_addr)?;
        self.owner_by_proxy
            .save(deps.storage, &proxy_addr, &owner_addr)?;

        Ok(Response::new())
    }

    #[msg(query)]
    fn proxy_by_owner(
        &self,
        ctx: QueryCtx,
        owner: String,
    ) -> Result<ProxyByOwnerResponse, ContractError> {
        let owner_addr = ctx.deps.api.addr_validate(&owner)?;
        let proxy_addr = self.proxy_by_owner.load(ctx.deps.storage, &owner_addr)?;
        Ok(ProxyByOwnerResponse {
            proxy: proxy_addr.to_string(),
        })
    }

    #[msg(query)]
    fn owner_by_proxy(
        &self,
        ctx: QueryCtx,
        proxy: String,
    ) -> Result<OwnerByProxyResponse, ContractError> {
        let proxy_addr = ctx.deps.api.addr_validate(&proxy)?;
        let owner_addr = self.owner_by_proxy.load(ctx.deps.storage, &proxy_addr)?;
        Ok(OwnerByProxyResponse {
            owner: owner_addr.to_string(),
        })
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(deps: DepsMut, _env: Env, msg: SudoMsg) -> Result<Response, ContractError> {
    match msg {
        SudoMsg::Jailing { jailed, tombstoned } => NativeStakingContract::new().handle_jailing(
            deps,
            &jailed.unwrap_or_default(),
            &tombstoned.unwrap_or_default(),
        ),
    }
}

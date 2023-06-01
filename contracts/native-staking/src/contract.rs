use cosmwasm_std::{from_slice, Addr, DepsMut, Reply, Response, SubMsgResponse};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::parse_instantiate_response_data;
use sylvia::types::{InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use mesh_apis::local_staking_api;
use mesh_native_staking_proxy::msg::OwnerMsg;
use mesh_native_staking_proxy::native_staking_callback;

use crate::error::ContractError;
use crate::msg::{ConfigResponse, OwnerByProxyResponse, ProxyByOwnerResponse};
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 2;

// TODO: Hardcoded for now. Revisit for v1.
pub const MAX_SLASH_PERCENTAGE: u64 = 10;

pub struct NativeStakingContract<'a> {
    pub config: Item<'a, Config>,
    /// Map of proxy contract address by owner address
    pub proxy_by_owner: Map<'a, &'a Addr, Addr>,
    /// Reverse map of owner address by proxy contract address
    pub owner_by_proxy: Map<'a, &'a Addr, Addr>,
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
        }
    }

    /// The caller of the instantiation will be the vault contract
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        proxy_code_id: u64,
    ) -> Result<Response, ContractError> {
        let config = Config {
            denom,
            proxy_code_id,
            vault: ctx.info.sender,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
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

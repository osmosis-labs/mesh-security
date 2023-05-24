use cosmwasm_std::{
    ensure_eq, entry_point, from_slice, to_binary, Addr, Binary, Decimal, DepsMut, Env, Reply,
    Response, SubMsg, SubMsgResponse, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::{must_pay, parse_instantiate_response_data};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::local_staking_api::{self, LocalStakingApi, MaxSlashResponse};
use mesh_native_staking_proxy::native_staking_callback::{self, NativeStakingCallback};

use crate::error::ContractError;
use crate::msg::{ConfigResponse, OwnerByProxyResponse, OwnerMsg, ProxyByOwnerResponse, StakeMsg};
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 2;

// TODO: Hardcoded for now. Revisit for v1.
pub const MAX_SLASH_PERCENTAGE: u64 = 10;

pub struct NativeStakingContract<'a> {
    // TODO
    config: Item<'a, Config>,
    proxies: Map<'a, &'a Addr, Addr>,
}

#[contract]
#[error(ContractError)]
#[messages(local_staking_api as LocalStakingApi)]
#[messages(native_staking_callback as NativeStakingCallback)]
impl NativeStakingContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            proxies: Map::new("proxies"),
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
        self.proxies.save(deps.storage, &owner_addr, &proxy_addr)?;

        Ok(Response::new())
    }

    #[msg(query)]
    fn proxy_by_owner(
        &self,
        _ctx: QueryCtx,
        owner: String,
    ) -> Result<ProxyByOwnerResponse, ContractError> {
        let _ = owner;
        todo!()
    }

    #[msg(query)]
    fn owner_by_proxy(
        &self,
        _ctx: QueryCtx,
        proxy: String,
    ) -> Result<OwnerByProxyResponse, ContractError> {
        let _ = proxy;
        todo!()
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, reply: Reply) -> Result<Response, ContractError> {
    match reply.id {
        REPLY_ID_INSTANTIATE => {
            NativeStakingContract::new().reply_init_callback(deps, reply.result.unwrap())
        }
        _ => Err(ContractError::InvalidReplyId(reply.id)),
    }
}

#[contract]
impl LocalStakingApi for NativeStakingContract<'_> {
    type Error = ContractError;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        msg: Binary,
    ) -> Result<Response, Self::Error> {
        // Can only be called by the vault
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.vault, ctx.info.sender, ContractError::Unauthorized {});

        // Assert funds are passed in
        let _paid = must_pay(&ctx.info, &cfg.denom)?;

        // Parse message to find validator to stake on
        let StakeMsg { validator } = from_slice(&msg)?;
        // TODO?: Validate validator address
        let _ = validator;

        let owner_addr = ctx.deps.api.addr_validate(&owner)?;

        // Look up if there is a proxy to match. Instantiate or call stake on existing
        match self.proxies.may_load(ctx.deps.storage, &owner_addr)? {
            None => {
                // Instantiate proxy contract and send stake message, with reply handling on success
                let msg = WasmMsg::Instantiate {
                    admin: Some(ctx.env.contract.address.into()),
                    code_id: cfg.proxy_code_id,
                    msg,
                    funds: ctx.info.funds,
                    label: format!("LSP for {owner}"), // FIXME: Check / cap label length
                };
                let sub_msg = SubMsg::reply_on_success(msg, REPLY_ID_INSTANTIATE);
                let owner_data = to_binary(&OwnerMsg { owner })?;
                Ok(Response::new().add_submessage(sub_msg).set_data(owner_data))
            }
            Some(proxy_addr) => {
                // Send stake message to the proxy contract
                let msg = WasmMsg::Execute {
                    contract_addr: proxy_addr.into(),
                    msg,
                    funds: ctx.info.funds,
                };
                let sub_msg = SubMsg::new(msg);
                Ok(Response::new().add_submessage(sub_msg))
            }
        }
    }

    /// Returns the maximum percentage that can be slashed
    /// TODO: Any way to query this from the chain? Or we just pass in InstantiateMsg?
    #[msg(query)]
    fn max_slash(&self, _ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error> {
        Ok(MaxSlashResponse {
            max_slash: Decimal::percent(MAX_SLASH_PERCENTAGE),
        })
    }
}

#[contract]
impl NativeStakingCallback for NativeStakingContract<'_> {
    type Error = ContractError;

    /// This sends tokens back from the proxy to native-staking. (See info.funds)
    /// The native-staking contract can determine which user it belongs to via an internal Map.
    /// The native-staking contract will then send those tokens back to vault and release the claim.
    #[msg(exec)]
    fn release_proxy_stake(&self, _ctx: ExecCtx) -> Result<Response, Self::Error> {
        // ensure proper denom in info.funds
        // look up proxy address (info.sender) to account owner
        // send these tokens to vault contract, using release_local_stake method
        todo!()
    }
}

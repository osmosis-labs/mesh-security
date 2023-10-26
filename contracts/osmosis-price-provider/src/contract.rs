use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, entry_point, DepsMut, Env, IbcChannel, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use mesh_bindings::PriceFeedProviderSudoMsg;
use sylvia::types::{ExecCtx, InstantiateCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::state::{Config, Subscription};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct OsmosisPriceProvider {
    config: Item<'static, Config>,
    subscriptions: Item<'static, Vec<Subscription>>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
impl OsmosisPriceProvider {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            subscriptions: Item::new("subscriptions"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        admin: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let admin = ctx.deps.api.addr_validate(&admin)?;
        let config = Config { admin };
        self.config.save(ctx.deps.storage, &config)?;

        self.subscriptions.save(ctx.deps.storage, &vec![])?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        Ok(Response::new())
    }

    #[msg(exec)]
    pub fn subscribe(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.admin, ContractError::Unauthorized {});

        todo!("implement subscribing")
    }

    #[msg(exec)]
    pub fn unsubscribe(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.admin, ContractError::Unauthorized {});

        todo!("implement unsubscribing")
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(
    _deps: DepsMut,
    _env: Env,
    msg: PriceFeedProviderSudoMsg,
) -> Result<Response, ContractError> {
    match msg {
        PriceFeedProviderSudoMsg::EndBlock {} => {
            todo!("periodically send out updates over IBC")
        }
    }
}

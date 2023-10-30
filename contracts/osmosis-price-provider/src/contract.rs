use std::collections::HashMap;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{ensure_eq, entry_point, DepsMut, Env, IbcChannel, Response, Timestamp, Uint64};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::{nonpayable, Duration};
use mesh_bindings::PriceFeedProviderSudoMsg;
use sylvia::types::{ExecCtx, InstantiateCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const EPOCH_IN_SECS: u64 = 120;
const LAST_EPOCH: Item<'static, Timestamp> = Item::new("last_epoch");

pub struct OsmosisPriceProvider {
    config: Item<'static, Config>,
    subscriptions: Item<'static, HashMap<String, IbcChannel>>,
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
        LAST_EPOCH.save(ctx.deps.storage, &Timestamp::from_seconds(0))?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        Ok(Response::new())
    }

    #[msg(exec)]
    pub fn subscribe(
        &self,
        ctx: ExecCtx,
        denom: String,
        channel: IbcChannel,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.admin, ContractError::Unauthorized {});

        let mut subs = self.subscriptions.load(ctx.deps.storage)?;

        if subs.contains_key(&denom) {
            Err(ContractError::SubscriptionAlreadyExists)
        } else {
            subs.insert(denom, channel);
            self.subscriptions.save(ctx.deps.storage, &subs)?;
            Ok(Response::new())
        }
    }

    #[msg(exec)]
    pub fn unsubscribe(&self, ctx: ExecCtx, denom: String) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.admin, ContractError::Unauthorized {});

        let mut subs = self.subscriptions.load(ctx.deps.storage)?;

        if !subs.contains_key(&denom) {
            Err(ContractError::SubscriptionDoesNotExist)
        } else {
            subs.remove(&denom);
            self.subscriptions.save(ctx.deps.storage, &subs)?;
            Ok(Response::new())
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    msg: PriceFeedProviderSudoMsg,
) -> Result<Response, ContractError> {
    match msg {
        PriceFeedProviderSudoMsg::EndBlock {} => {
            let last_epoch = LAST_EPOCH.load(deps.storage)?;
            let secs_since_last_epoch = env.block.time.seconds() - last_epoch.seconds();
            if secs_since_last_epoch >= EPOCH_IN_SECS {
                todo!("send out updates over IBC")
            } else {
                Ok(Response::new())
            }
        }
    }
}

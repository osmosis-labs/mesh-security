use std::str::FromStr;

use cosmwasm_std::{
    ensure_eq, entry_point, Decimal, DepsMut, Env, IbcChannel, Response, Timestamp,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use mesh_bindings::PriceFeedProviderSudoMsg;
use osmosis_std::types::osmosis::twap::v1beta1::TwapQuerier;
use sylvia::types::{ExecCtx, InstantiateCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::ibc::make_ibc_packet;
use crate::state::{Config, Subscription, Subscriptions};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const BASE_ASSET: &str = "OSMO";

const EPOCH_IN_SECS: u64 = 120;
const LAST_EPOCH: Item<'static, Timestamp> = Item::new("last_epoch");

pub struct OsmosisPriceProvider {
    config: Item<'static, Config>,
    pub(crate) subscriptions: Subscriptions,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
impl OsmosisPriceProvider {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            subscriptions: Subscriptions::new(),
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
        self.subscriptions.init(ctx.deps.storage)?;
        LAST_EPOCH.save(ctx.deps.storage, &Timestamp::from_seconds(0))?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        Ok(Response::new())
    }

    #[msg(exec)]
    pub fn bind(
        &self,
        ctx: ExecCtx,
        denom: String,
        channel: IbcChannel,
        pool_id: u64,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, cfg.admin, ContractError::Unauthorized {});

        self.subscriptions
            .bind_channel(ctx.deps.storage, channel, denom, pool_id)?;

        Ok(Response::new())
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
                let contract = OsmosisPriceProvider::new();

                let subs = contract.subscriptions.subs(deps.storage)?;
                let querier = TwapQuerier::new(&deps.querier);

                let msgs = subs
                    .map(|(denom, Subscription { channel, pool_id })| {
                        let twap = querier
                            .arithmetic_twap_to_now(pool_id, BASE_ASSET.to_string(), denom, None)?
                            .arithmetic_twap;
                        let twap = Decimal::from_str(&twap)?;
                        let packet = mesh_apis::ibc::PriceFeedProviderPacket::Update { twap };
                        make_ibc_packet(&env.block.time, channel, packet)
                    })
                    .filter_map(Result::ok); // silently ignore failures - TODO: logging of some kind?

                Ok(Response::new().add_messages(msgs))
            } else {
                Ok(Response::new())
            }
        }
    }
}

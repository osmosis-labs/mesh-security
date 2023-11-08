use cosmwasm_std::{entry_point, DepsMut, Env, IbcChannel, Response, Timestamp};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use mesh_bindings::RemotePriceFeedSudoMsg;
use sylvia::types::{InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::price_feed_api::{self, PriceFeedApi, PriceResponse};

use crate::error::ContractError;
use crate::ibc::{make_ibc_packet, AUTH_ENDPOINT};
use crate::msg::AuthorizedEndpoint;
use crate::state::{PriceInfo, TradingPair};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct RemotePriceFeedContract {
    pub channel: Item<'static, IbcChannel>,
    pub trading_pair: Item<'static, TradingPair>,
    pub price_info: Item<'static, PriceInfo>,
    pub last_epoch: Item<'static, Timestamp>,
    pub epoch_in_secs: Item<'static, u64>,
    pub price_info_ttl_in_secs: Item<'static, u64>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(price_feed_api as PriceFeedApi)]
impl RemotePriceFeedContract {
    pub const fn new() -> Self {
        Self {
            channel: Item::new("channel"),
            trading_pair: Item::new("tpair"),
            price_info: Item::new("price"),
            last_epoch: Item::new("last_epoch"),
            epoch_in_secs: Item::new("epoch"),
            price_info_ttl_in_secs: Item::new("price_ttl"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        trading_pair: TradingPair,
        auth_endpoint: AuthorizedEndpoint,
        epoch_in_secs: u64,
        price_info_ttl_in_secs: u64,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        self.last_epoch
            .save(ctx.deps.storage, &Timestamp::from_seconds(0))?;
        self.trading_pair.save(ctx.deps.storage, &trading_pair)?;
        self.epoch_in_secs.save(ctx.deps.storage, &epoch_in_secs)?;
        self.price_info_ttl_in_secs
            .save(ctx.deps.storage, &price_info_ttl_in_secs)?;

        AUTH_ENDPOINT.save(ctx.deps.storage, &auth_endpoint)?;

        Ok(Response::new())
    }
}

#[contract]
#[messages(price_feed_api as PriceFeedApi)]
impl PriceFeedApi for RemotePriceFeedContract {
    type Error = ContractError;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    #[msg(query)]
    fn price(&self, ctx: QueryCtx) -> Result<PriceResponse, Self::Error> {
        let price_info_ttl = self.price_info_ttl_in_secs.load(ctx.deps.storage)?;
        let price_info = self
            .price_info
            .may_load(ctx.deps.storage)?
            .ok_or(ContractError::NoPriceData)?;

        if ctx.env.block.time.minus_seconds(price_info_ttl) < price_info.time {
            Ok(PriceResponse {
                native_per_foreign: price_info.native_per_foreign,
            })
        } else {
            Err(ContractError::OutdatedPriceData)
        }
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn sudo(
    deps: DepsMut,
    env: Env,
    msg: RemotePriceFeedSudoMsg,
) -> Result<Response, ContractError> {
    match msg {
        RemotePriceFeedSudoMsg::EndBlock {} => {
            let contract = RemotePriceFeedContract::new();
            let TradingPair {
                pool_id,
                base_asset,
                quote_asset,
            } = contract.trading_pair.load(deps.storage)?;
            let channel = contract
                .channel
                .may_load(deps.storage)?
                .ok_or(ContractError::IbcChannelNotOpen)?;

            let last_epoch = contract.last_epoch.load(deps.storage)?;
            let epoch_duration = contract.epoch_in_secs.load(deps.storage)?;
            let secs_since_last_epoch = env.block.time.seconds() - last_epoch.seconds();
            if secs_since_last_epoch >= epoch_duration {
                let packet = mesh_apis::ibc::RemotePriceFeedPacket::QueryTwap {
                    pool_id,
                    base_asset,
                    quote_asset,
                };
                let msg = make_ibc_packet(&env.block.time, channel, packet)?;

                Ok(Response::new().add_message(msg))
            } else {
                Ok(Response::new())
            }
        }
    }
}

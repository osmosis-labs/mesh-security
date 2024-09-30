use std::vec;

use cosmwasm_std::{Decimal, DepsMut, Env, IbcChannel, Response, Timestamp};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use mesh_apis::ibc::{encode_request, ibc_query_packet, ArithmeticTwapToNowRequest, CosmosQuery};
use osmosis_std::shim::Timestamp as OsmosisTimestamp;
use osmosis_std::types::tendermint::abci::RequestQuery;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, SudoCtx};
use sylvia::{contract, schemars};

use mesh_apis::price_feed_api::{self, PriceFeedApi, PriceResponse};

use crate::error::ContractError;
use crate::ibc::make_ibc_packet;
use crate::state::TradingPair;
use mesh_price_feed::{Action, PriceKeeper, Scheduler};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const OSMOSIS_QUERY_TWAP_PATH: &str = "/osmosis.twap.v1beta1.Query/ArithmeticTwapToNow";

pub struct RemotePriceFeedContract {
    pub channel: Item<'static, IbcChannel>,
    pub trading_pair: Item<'static, TradingPair>,
    pub price_keeper: PriceKeeper,
    pub scheduler: Scheduler<Box<dyn Action<ContractError>>, ContractError>,
}

impl Default for RemotePriceFeedContract {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
#[sv::messages(price_feed_api as PriceFeedApi)]
impl RemotePriceFeedContract {
    pub fn new() -> Self {
        Self {
            channel: Item::new("channel"),
            trading_pair: Item::new("tpair"),
            price_keeper: PriceKeeper::new(),
            // TODO: the indirection can be removed once Sylvia supports
            // generics. The constructor can then probably be constant.
            //
            // Stable existential types would be even better!
            // https://github.com/rust-lang/rust/issues/63063
            scheduler: Scheduler::new(Box::new(query_twap)),
        }
    }

    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        mut ctx: InstantiateCtx,
        trading_pair: TradingPair,
        epoch_in_secs: u64,
        price_info_ttl_in_secs: u64,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        self.trading_pair.save(ctx.deps.storage, &trading_pair)?;

        self.price_keeper
            .init(&mut ctx.deps, price_info_ttl_in_secs)?;
        self.scheduler.init(&mut ctx.deps, epoch_in_secs)?;
        Ok(Response::new())
    }

    #[sv::msg(exec)]
    pub fn request(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let ExecCtx { deps, env, info: _ } = ctx;
        query_twap(deps, &env)
    }

    pub(crate) fn update_twap(
        &self,
        deps: DepsMut,
        time: Timestamp,
        twap: Decimal,
    ) -> Result<(), ContractError> {
        Ok(self.price_keeper.update(deps, time, twap)?)
    }
}

impl PriceFeedApi for RemotePriceFeedContract {
    type Error = ContractError;
    // FIXME: make these under a feature flag if we need virtual-staking multitest compatibility
    type ExecC = cosmwasm_std::Empty;
    type QueryC = cosmwasm_std::Empty;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    fn price(&self, ctx: QueryCtx) -> Result<PriceResponse, Self::Error> {
        Ok(self
            .price_keeper
            .price(ctx.deps, &ctx.env)
            .map(|rate| PriceResponse {
                native_per_foreign: rate,
            })?)
    }

    fn handle_epoch(&self, ctx: SudoCtx) -> Result<Response, Self::Error> {
        self.scheduler.trigger(ctx.deps, &ctx.env)
    }
}

pub fn query_twap(deps: DepsMut, env: &Env) -> Result<Response, ContractError> {
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

    let request = ArithmeticTwapToNowRequest {
        pool_id,
        base_asset,
        quote_asset,
        start_time: Some(OsmosisTimestamp {
            seconds: env.block.time.seconds() as i64,
            nanos: 0,
        }),
    };
    let packet = CosmosQuery {
        requests: vec![RequestQuery {
            path: OSMOSIS_QUERY_TWAP_PATH.to_string(),
            data: encode_request(&request),
            height: 0,
            prove: false,
        }],
    };

    let msg = make_ibc_packet(&env.block.time, channel, ibc_query_packet(packet))?;

    Ok(Response::new().add_message(msg))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{
        from_json,
        testing::{mock_dependencies, mock_env, mock_info},
        Binary,
    };
    use mesh_apis::ibc::{
        decode_response, AcknowledgementResult, CosmosResponse, InterchainQueryPacketAck,
    };

    use super::*;

    #[test]
    fn instantiation() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let info = mock_info("sender", &[]);
        let contract = RemotePriceFeedContract::new();

        let trading_pair = TradingPair {
            pool_id: 1,
            base_asset: "base".to_string(),
            quote_asset: "quote".to_string(),
        };

        contract
            .instantiate(
                InstantiateCtx {
                    deps: deps.as_mut(),
                    env,
                    info,
                },
                trading_pair,
                10,
                50,
            )
            .unwrap();
    }

    #[test]
    fn json_binary() {
        let resp = Binary::from_base64("eyJyZXN1bHQiOiJleUprWVhSaElqb2lRMmhqTmtaUmIxUk5WRUYzVFVSQmQwMUVRWGROUkVGM1RVUkJkMDFFUVhkTlFUMDlJbjA9In0=").unwrap();

        let ack_result: AcknowledgementResult = from_json(resp).unwrap();
        assert_eq!(
            ack_result.result.to_string(),
            String::from("eyJkYXRhIjoiQ2hjNkZRb1RNVEF3TURBd01EQXdNREF3TURBd01EQXdNQT09In0=")
        );

        let packet_ack: InterchainQueryPacketAck = from_json(&ack_result.result).unwrap();
        assert_eq!(
            packet_ack.data.to_string(),
            String::from("Chc6FQoTMTAwMDAwMDAwMDAwMDAwMDAwMA==")
        );

        let response: CosmosResponse = decode_response(&packet_ack.data).unwrap();
        assert_eq!(response.responses.len(), 1);
    }
}

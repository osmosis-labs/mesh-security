use cosmwasm_std::{
    to_json_binary, Binary, Coin, DepsMut, Env, IbcChannel, IbcMsg, IbcTimeout,
    Response, Uint64,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use mesh_apis::price_feed_api::{PriceFeedApi, PriceResponse};

use crate::error::ContractError;
use crate::state::{Config, TradingPair};

use sylvia::ctx::{ExecCtx, InstantiateCtx, QueryCtx, SudoCtx};
use sylvia::contract;

use cw_band::oracle::oracle_script::std_crypto::Input;
use cw_band::oracle::packet::OracleRequestPacketData;
use mesh_price_feed::{Action, PriceKeeper, Scheduler};
use obi::enc::OBIEncode;

// Version info for migration
const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct RemotePriceFeedContract {
    pub channel: Item<IbcChannel>,
    pub config: Item<Config>,
    pub trading_pair: Item<TradingPair>,
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
#[sv::messages(mesh_apis::price_feed_api as PriceFeedApi)]
impl RemotePriceFeedContract {
    pub fn new() -> Self {
        Self {
            channel: Item::new("channel"),
            config: Item::new("config"),
            trading_pair: Item::new("tpair"),
            price_keeper: PriceKeeper::new(),
            // TODO: the indirection can be removed once Sylvia supports
            // generics. The constructor can then probably be constant.
            //
            // Stable existential types would be even better!
            // https://github.com/rust-lang/rust/issues/63063
            scheduler: Scheduler::new(Box::new(try_request)),
        }
    }

    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        mut ctx: InstantiateCtx,
        trading_pair: TradingPair,
        client_id: String,
        oracle_script_id: Uint64,
        ask_count: Uint64,
        min_count: Uint64,
        fee_limit: Vec<Coin>,
        prepare_gas: Uint64,
        execute_gas: Uint64,
        minimum_sources: u8,
        epoch_in_secs: u64,
        price_info_ttl_in_secs: u64,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        self.trading_pair.save(ctx.deps.storage, &trading_pair)?;
        self.config.save(
            ctx.deps.storage,
            &Config {
                client_id,
                oracle_script_id,
                ask_count,
                min_count,
                fee_limit,
                prepare_gas,
                execute_gas,
                minimum_sources,
            },
        )?;
        self.scheduler.init(&mut ctx.deps, epoch_in_secs)?;
        self.price_keeper
            .init(&mut ctx.deps, price_info_ttl_in_secs)?;
        Ok(Response::new())
    }

    #[sv::msg(exec)]
    pub fn request(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let ExecCtx { deps, env, .. } = ctx;
        try_request(deps, &env)
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

// TODO: Possible features
// - Request fee + Bounty logic to prevent request spam and incentivize relayer
// - Whitelist who can call update price
pub fn try_request(deps: DepsMut, env: &Env) -> Result<Response, ContractError> {
    let contract = RemotePriceFeedContract::new();
    let TradingPair {
        base_asset,
        quote_asset,
    } = contract.trading_pair.load(deps.storage)?;
    let config = contract.config.load(deps.storage)?;
    let channel = contract
        .channel
        .may_load(deps.storage)?
        .ok_or(ContractError::IbcChannelNotOpen)?;

    let raw_calldata = Input {
        symbols: vec![base_asset, quote_asset],
        minimum_sources: config.minimum_sources,
    }
    .try_to_vec()
    .map(|bytes| Binary::from(bytes))
    .map_err(|err| ContractError::CustomError {
        val: err.to_string(),
    })?;

    let packet = OracleRequestPacketData {
        client_id: config.client_id,
        oracle_script_id: config.oracle_script_id,
        calldata: raw_calldata,
        ask_count: config.ask_count,
        min_count: config.min_count,
        prepare_gas: config.prepare_gas,
        execute_gas: config.execute_gas,
        fee_limit: config.fee_limit,
    };

    Ok(Response::new().add_message(IbcMsg::SendPacket {
        channel_id: channel.endpoint.channel_id,
        data: to_json_binary(&packet)?,
        timeout: IbcTimeout::with_timestamp(env.block.time.plus_seconds(60)),
    }))
}

#[cfg(test)]
mod tests {
    use cosmwasm_std::{
        testing::{message_info, mock_dependencies, mock_env}, Uint128, Uint64,
    };
    use cw_multi_test::IntoBech32;

    use super::*;

    #[test]
    fn instantiation() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let sender = "sender".into_bech32();
        let info = message_info(&sender, &[]);
        let contract = RemotePriceFeedContract::new();

        let trading_pair = TradingPair {
            base_asset: "base".to_string(),
            quote_asset: "quote".to_string(),
        };

        contract
            .instantiate(
                InstantiateCtx::from((deps.as_mut(), env, info)),
                trading_pair,
                "07-tendermint-0".to_string(),
                Uint64::new(1),
                Uint64::new(10),
                Uint64::new(50),
                vec![Coin {
                    denom: "uband".to_string(),
                    amount: Uint128::new(1),
                }],
                Uint64::new(100000),
                Uint64::new(200000),
                1,
                60,
                60,
            )
            .unwrap();
    }
}

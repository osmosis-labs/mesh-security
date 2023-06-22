use cosmwasm_std::{
    ensure_eq, to_binary, Addr, Coin, Decimal, Deps, DepsMut, Event, Reply, Response, SubMsg,
    SubMsgResponse, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::{nonpayable, parse_instantiate_response_data};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use mesh_apis::converter_api::{self, ConverterApi, RewardInfo};
use mesh_apis::price_feed_api;
use mesh_apis::virtual_staking_api;

use crate::error::ContractError;
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const REPLY_ID_INSTANTIATE: u64 = 1;

pub struct ConverterContract<'a> {
    pub config: Item<'a, Config>,
    pub virtual_stake: Item<'a, Addr>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(converter_api as ConverterApi)]
impl ConverterContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            virtual_stake: Item::new("bonded"),
        }
    }

    // TODO: there is a chicken and egg problem here.
    // converter needs fixed address for virtual stake contract
    // virtual stake contract needs fixed address for converter
    // (Price feed can be made first and then converter second, that is no issue)
    /// The caller of the instantiation will be the converter contract
    /// Discount is applied to foreign tokens after adjusting foreign/native price,
    /// such that 0.3 discount means foreign assets have 70% of their value
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        price_feed: String,
        discount: Decimal,
        remote_denom: String,
        virtual_staking_code_id: u64,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let config = Config {
            price_feed: ctx.deps.api.addr_validate(&price_feed)?,
            // TODO: better error if discount greater than 1 (this will panic)
            adjustment: Decimal::one() - discount,
            local_denom: ctx.deps.querier.query_bonded_denom()?,
            // TODO: validation here? Just that it is non-empty?
            remote_denom,
        };
        self.config.save(ctx.deps.storage, &config)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        // instantiate virtual staking contract here
        let info = ctx
            .deps
            .querier
            .query_wasm_contract_info(ctx.env.contract.address)?;
        let init_msg = WasmMsg::Instantiate {
            admin: info.admin,
            code_id: virtual_staking_code_id,
            msg: b"{}".into(),
            funds: vec![],
            label: format!("Virtual Staking: {}", &config.remote_denom),
        };
        let init_msg = SubMsg::reply_on_success(init_msg, REPLY_ID_INSTANTIATE);

        Ok(Response::new().add_submessage(init_msg))
    }

    #[msg(reply)]
    fn reply(&self, ctx: ReplyCtx, reply: Reply) -> Result<Response, ContractError> {
        match reply.id {
            REPLY_ID_INSTANTIATE => self.reply_init_callback(ctx.deps, reply.result.unwrap()),
            _ => Err(ContractError::InvalidReplyId(reply.id)),
        }
    }

    /// Store virtual staking address
    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;
        let virtual_staking = Addr::unchecked(init_data.contract_address);
        self.virtual_stake.save(deps.storage, &virtual_staking)?;
        Ok(Response::new())
    }

    /// This is only used for tests
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[msg(exec)]
    fn demo_stake(
        &self,
        ctx: ExecCtx,
        validator: String,
        stake: Coin,
    ) -> Result<Response, ContractError> {
        // This can only ever be valid in tests
        ensure_eq!(ctx.info.sender, "ADMIN", ContractError::Unauthorized);
        self.stake(ctx.deps, validator, stake)
    }

    /// This is only used for tests
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[msg(exec)]
    fn demo_unstake(
        &self,
        ctx: ExecCtx,
        validator: String,
        unstake: Coin,
    ) -> Result<Response, ContractError> {
        // This can only ever be valid in tests
        ensure_eq!(ctx.info.sender, "ADMIN", ContractError::Unauthorized);
        self.unstake(ctx.deps, validator, unstake)
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        let virtual_staking = self.virtual_stake.load(ctx.deps.storage)?.into_string();
        Ok(ConfigResponse {
            price_feed: config.price_feed.into_string(),
            adjustment: config.adjustment,
            virtual_staking,
        })
    }

    /// This is called by ibc_packet_receive.
    /// It is pulled out into a method, so it can also be called by sudo for testing
    pub(crate) fn stake(
        &self,
        deps: DepsMut,
        validator: String,
        stake: Coin,
    ) -> Result<Response, ContractError> {
        let amount = self.normalize_price(deps.as_ref(), stake)?;

        let event = Event::new("mesh-bond")
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.amount.to_string());

        let msg = virtual_staking_api::ExecMsg::Bond { validator, amount };
        let msg = WasmMsg::Execute {
            contract_addr: self.virtual_stake.load(deps.storage)?.into(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(msg).add_event(event))
    }

    /// This is called by ibc_packet_receive.
    /// It is pulled out into a method, so it can also be called by sudo for testing
    pub(crate) fn unstake(
        &self,
        deps: DepsMut,
        validator: String,
        unstake: Coin,
    ) -> Result<Response, ContractError> {
        let amount = self.normalize_price(deps.as_ref(), unstake)?;

        let event = Event::new("mesh-unbond")
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.amount.to_string());

        let msg = virtual_staking_api::ExecMsg::Unbond { validator, amount };
        let msg = WasmMsg::Execute {
            contract_addr: self.virtual_stake.load(deps.storage)?.into(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(msg).add_event(event))
    }

    fn normalize_price(&self, deps: Deps, amount: Coin) -> Result<Coin, ContractError> {
        // TODO: ensure the proper remote denom - set this in the instantiate
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            config.remote_denom,
            amount.denom,
            ContractError::WrongDenom {
                sent: amount.denom,
                expected: config.remote_denom
            }
        );

        // get the price value (usage is a bit clunky, need use trait and cannot chain Remote::new() with .querier())
        use price_feed_api::Querier;
        let remote = price_feed_api::Remote::new(config.price_feed);
        let price_feed = remote.querier(&deps.querier);
        let price = price_feed.price()?.native_per_foreign;
        let converted = (amount.amount * price) * config.adjustment;

        Ok(Coin {
            denom: config.local_denom,
            amount: converted,
        })
    }
}

#[contract]
#[messages(converter_api as ConverterApi)]
impl ConverterApi for ConverterContract<'_> {
    type Error = ContractError;

    /// Rewards tokens (in native staking denom) are sent alongside the message, and should be distributed to all
    /// stakers who staked on this validator.
    #[msg(exec)]
    fn distribute_reward(&self, ctx: ExecCtx, validator: String) -> Result<Response, Self::Error> {
        let _ = (ctx, validator);
        todo!();
    }

    /// This is a batch for of distribute_reward, including the payment for multiple validators.
    /// This is more efficient than calling distribute_reward multiple times, but also more complex.
    ///
    /// info.funds sent along with the message should be the sum of all rewards for all validators,
    /// in the native staking denom.
    #[msg(exec)]
    fn distribute_rewards(
        &self,
        ctx: ExecCtx,
        payments: Vec<RewardInfo>,
    ) -> Result<Response, Self::Error> {
        let _ = (ctx, payments);
        todo!();
    }
}

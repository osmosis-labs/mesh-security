use cosmwasm_std::{
    ensure_eq, to_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, Deps, DepsMut, Event, Reply,
    Response, SubMsg, SubMsgResponse, Uint128, Validator, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::{must_pay, nonpayable, parse_instantiate_response_data};
use mesh_apis::ibc::ConsumerPacket;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use mesh_apis::converter_api::{self, ConverterApi, RewardInfo};
use mesh_apis::price_feed_api;
use mesh_apis::virtual_staking_api;

use crate::error::ContractError;
use crate::ibc::{add_validators_msg, make_ibc_packet, tombstone_validators_msg, IBC_CHANNEL};
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
            virtual_stake: Item::new("virtual_stake"),
        }
    }

    /// We must first instantiate the price feed contract, then the converter contract.
    /// The converter will then instantiate a virtual staking contract to work with it,
    /// as they both need references to each other. The admin of the virtual staking
    /// contract is taken as an explicit argument.
    ///
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
        admin: Option<String>,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        // validate args
        if discount > Decimal::one() {
            return Err(ContractError::InvalidDiscount);
        }
        if remote_denom.is_empty() {
            return Err(ContractError::InvalidDenom(remote_denom));
        }
        let config = Config {
            price_feed: ctx.deps.api.addr_validate(&price_feed)?,
            price_adjustment: Decimal::one() - discount,
            local_denom: ctx.deps.querier.query_bonded_denom()?,
            remote_denom,
        };
        self.config.save(ctx.deps.storage, &config)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        if let Some(admin) = &admin {
            ctx.deps.api.addr_validate(admin)?;
        }

        // Instantiate virtual staking contract
        let init_msg = WasmMsg::Instantiate {
            admin,
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

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[msg(exec)]
    fn test_stake(
        &self,
        ctx: ExecCtx,
        validator: String,
        stake: Coin,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            // This can only ever be called in tests
            self.stake(ctx.deps, validator, stake)
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, validator, stake);
            Err(ContractError::Unauthorized)
        }
    }

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[msg(exec)]
    fn test_unstake(
        &self,
        ctx: ExecCtx,
        validator: String,
        unstake: Coin,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            // This can only ever be called in tests
            self.unstake(ctx.deps, validator, unstake)
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, validator, unstake);
            Err(ContractError::Unauthorized)
        }
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        let virtual_staking = self.virtual_stake.load(ctx.deps.storage)?.into_string();
        Ok(ConfigResponse {
            price_feed: config.price_feed.into_string(),
            adjustment: config.price_adjustment,
            virtual_staking,
        })
    }

    /// This is called by ibc_packet_receive.
    /// It is pulled out into a method, so it can also be called by test_stake for testing
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
    /// It is pulled out into a method, so it can also be called by test_unstake for testing
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
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            config.remote_denom,
            amount.denom,
            ContractError::WrongDenom {
                sent: amount.denom,
                expected: config.remote_denom
            }
        );

        // get the price value (usage is a bit clunky, need to use trait and cannot chain Remote::new() with .querier())
        // also see https://github.com/CosmWasm/sylvia/issues/181 to just store Remote in state
        use price_feed_api::Querier;
        let remote = price_feed_api::Remote::new(config.price_feed);
        let price = remote.querier(&deps.querier).price()?.native_per_foreign;
        let converted = (amount.amount * price) * config.price_adjustment;

        Ok(Coin {
            denom: config.local_denom,
            amount: converted,
        })
    }

    pub(crate) fn transfer_rewards(
        &self,
        deps: Deps,
        recipient: String,
        rewards: Coin,
    ) -> Result<CosmosMsg, ContractError> {
        // ensure the address is proper
        let recipient = deps.api.addr_validate(&recipient)?;

        // ensure this is the reward denom (same as staking denom)
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            config.local_denom,
            rewards.denom,
            ContractError::WrongDenom {
                sent: rewards.denom,
                expected: config.local_denom
            }
        );

        // send the coins
        let msg = BankMsg::Send {
            to_address: recipient.into(),
            amount: vec![rewards],
        };
        Ok(msg.into())
    }
}

#[contract]
#[messages(converter_api as ConverterApi)]
impl ConverterApi for ConverterContract<'_> {
    type Error = ContractError;

    /// Rewards tokens (in native staking denom) are sent alongside the message, and should be distributed to all
    /// stakers who staked on this validator. This is tracked on the provider, so we send an IBC packet there.
    #[msg(exec)]
    fn distribute_reward(
        &self,
        mut ctx: ExecCtx,
        validator: String,
    ) -> Result<Response, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        let denom = config.local_denom;
        must_pay(&ctx.info, &denom)?;
        let rewards = ctx.info.funds.remove(0);

        let event = Event::new("distribute_reward")
            .add_attribute("validator", &validator)
            .add_attribute("amount", rewards.amount.to_string());

        let msg = make_ibc_packet(&mut ctx, ConsumerPacket::Distribute { validator, rewards })?;
        Ok(Response::new().add_message(msg).add_event(event))
    }

    /// This is a batch form of distribute_reward, including the payment for multiple validators.
    /// This is more efficient than calling distribute_reward multiple times, but also more complex.
    ///
    /// info.funds sent along with the message should be the sum of all rewards for all validators,
    /// in the native staking denom.
    #[msg(exec)]
    fn distribute_rewards(
        &self,
        mut ctx: ExecCtx,
        payments: Vec<RewardInfo>,
    ) -> Result<Response, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        let denom = config.local_denom;

        let summed_rewards: Uint128 = payments.iter().map(|reward_info| reward_info.reward).sum();
        let sent = must_pay(&ctx.info, &denom)?;

        if summed_rewards != sent {
            return Err(ContractError::DistributeRewardsInvalidAmount {
                sum: summed_rewards,
                sent,
            });
        }

        Ok(Response::new()
            .add_events(payments.iter().map(|reward_info| {
                Event::new("distribute_reward")
                    .add_attribute("validator", &reward_info.validator)
                    .add_attribute("amount", reward_info.reward)
            }))
            .add_message(make_ibc_packet(
                &mut ctx,
                ConsumerPacket::DistributeBatch {
                    rewards: payments,
                    denom,
                },
            )?))
    }

    /// Valset updates.
    ///
    /// Sent validator set additions (entering the active validator set) to the external staking
    /// contract on the Consumer via IBC.
    #[msg(exec)]
    fn valset_update(
        &self,
        ctx: ExecCtx,
        additions: Vec<Validator>,
        tombstones: Vec<Validator>,
    ) -> Result<Response, Self::Error> {
        let virtual_stake = self.virtual_stake.load(ctx.deps.storage)?;
        ensure_eq!(
            ctx.info.sender,
            virtual_stake,
            ContractError::Unauthorized {}
        );

        // Send over IBC to the Consumer
        let channel = IBC_CHANNEL.load(ctx.deps.storage)?;
        let add_msg = add_validators_msg(&ctx.env, &channel, &additions)?;
        let tomb_msg = tombstone_validators_msg(&ctx.env, &channel, &tombstones)?;

        let event = Event::new("valset_update").add_attribute(
            "additions",
            additions
                .iter()
                .map(|v| v.address.clone())
                .collect::<Vec<String>>()
                .join(","),
        );
        let event = event.add_attribute(
            "tombstones",
            tombstones
                .iter()
                .map(|v| v.address.clone())
                .collect::<Vec<String>>()
                .join(","),
        );
        let resp = Response::new()
            .add_event(event)
            .add_message(add_msg)
            .add_message(tomb_msg);

        Ok(resp)
    }
}

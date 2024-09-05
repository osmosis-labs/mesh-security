use cosmwasm_std::{
    ensure_eq, to_json_binary, Addr, BankMsg, Coin, CosmosMsg, Decimal, Deps, DepsMut, Event,
    Fraction, IbcMsg, MessageInfo, Reply, Response, StdError, SubMsg, SubMsgResponse, Uint128,
    Validator, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::{must_pay, nonpayable, parse_instantiate_response_data};
use mesh_apis::ibc::ConsumerPacket;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use mesh_apis::converter_api::{self, ConverterApi, RewardInfo, ValidatorSlashInfo};
use mesh_apis::price_feed_api;
use mesh_apis::virtual_staking_api;

use crate::error::ContractError;
use crate::ibc::{
    make_ibc_packet, packet_timeout_internal_unstake, valset_update_msg, IBC_CHANNEL,
};
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

const REPLY_ID_INSTANTIATE: u64 = 1;

#[cfg(not(feature = "fake-custom"))]
pub mod custom {
    pub type ConverterMsg = cosmwasm_std::Empty;
    pub type ConverterQuery = cosmwasm_std::Empty;
    pub type Response = cosmwasm_std::Response<cosmwasm_std::Empty>;
}
#[cfg(feature = "fake-custom")]
pub mod custom {
    pub type ConverterMsg = mesh_bindings::VirtualStakeCustomMsg;
    pub type ConverterQuery = mesh_bindings::VirtualStakeCustomQuery;
    pub type Response = cosmwasm_std::Response<ConverterMsg>;
}

pub struct ConverterContract<'a> {
    pub config: Item<'a, Config>,
    pub virtual_stake: Item<'a, Addr>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
#[sv::messages(converter_api as ConverterApi)]
/// Workaround for lack of support in communication `Empty` <-> `Custom` Contracts.
#[sv::custom(query=custom::ConverterQuery, msg=custom::ConverterMsg)]
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
    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx<custom::ConverterQuery>,
        price_feed: String,
        discount: Decimal,
        remote_denom: String,
        virtual_staking_code_id: u64,
        tombstoned_unbond_enable: bool,
        admin: Option<String>,
        max_retrieve: u16,
    ) -> Result<custom::Response, ContractError> {
        nonpayable(&ctx.info)?;
        // validate args
        if discount >= Decimal::one() {
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

        let msg = to_json_binary(&mesh_virtual_staking::contract::sv::InstantiateMsg {
            max_retrieve,
            tombstoned_unbond_enable,
        })?;
        
        // Instantiate virtual staking contract
        let init_msg = WasmMsg::Instantiate {
            admin,
            code_id: virtual_staking_code_id,
            msg,
            funds: vec![],
            label: format!("Virtual Staking: {}", &config.remote_denom),
        };
        let init_msg = SubMsg::reply_on_success(init_msg, REPLY_ID_INSTANTIATE);

        Ok(Response::new().add_submessage(init_msg))
    }

    #[sv::msg(reply)]
    fn reply(
        &self,
        ctx: ReplyCtx<custom::ConverterQuery>,
        reply: Reply,
    ) -> Result<custom::Response, ContractError> {
        match reply.id {
            REPLY_ID_INSTANTIATE => self.reply_init_callback(ctx.deps, reply.result.unwrap()),
            _ => Err(ContractError::InvalidReplyId(reply.id)),
        }
    }

    /// Store virtual staking address
    fn reply_init_callback(
        &self,
        deps: DepsMut<custom::ConverterQuery>,
        reply: SubMsgResponse,
    ) -> Result<custom::Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;
        let virtual_staking = Addr::unchecked(init_data.contract_address);
        self.virtual_stake.save(deps.storage, &virtual_staking)?;
        Ok(Response::new())
    }

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[sv::msg(exec)]
    fn test_stake(
        &self,
        ctx: ExecCtx<custom::ConverterQuery>,
        delegator: String,
        validator: String,
        stake: Coin,
    ) -> Result<custom::Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            // This can only ever be called in tests
            self.stake(ctx.deps, delegator, validator, stake)
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, delegator, validator, stake);
            Err(ContractError::Unauthorized)
        }
    }

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[sv::msg(exec)]
    fn test_unstake(
        &self,
        ctx: ExecCtx<custom::ConverterQuery>,
        delegator: String,
        validator: String,
        unstake: Coin,
    ) -> Result<custom::Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            // This can only ever be called in tests
            self.unstake(ctx.deps, delegator, validator, unstake)
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, delegator, validator, unstake);
            Err(ContractError::Unauthorized)
        }
    }

    /// This is only used for tests.
    /// Ideally we want conditional compilation of these whole methods and the enum variants
    #[sv::msg(exec)]
    fn test_burn(
        &self,
        ctx: ExecCtx<custom::ConverterQuery>,
        validators: Vec<String>,
        burn: Coin,
    ) -> Result<custom::Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            // This can only ever be called in tests
            self.burn(ctx.deps, &validators, burn)
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, validators, burn);
            Err(ContractError::Unauthorized)
        }
    }

    #[sv::msg(query)]
    fn config(
        &self,
        ctx: QueryCtx<custom::ConverterQuery>,
    ) -> Result<ConfigResponse, ContractError> {
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
        deps: DepsMut<custom::ConverterQuery>,
        delegator: String,
        validator: String,
        stake: Coin,
    ) -> Result<custom::Response, ContractError> {
        let amount = self.normalize_price(deps.as_ref(), stake)?;

        let event = Event::new("mesh-bond")
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.amount.to_string());

        let msg = virtual_staking_api::sv::ExecMsg::Bond {
            delegator,
            validator,
            amount,
        };
        let msg = WasmMsg::Execute {
            contract_addr: self.virtual_stake.load(deps.storage)?.into(),
            msg: to_json_binary(&msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(msg).add_event(event))
    }

    /// This is called by ibc_packet_receive.
    /// It is pulled out into a method, so it can also be called by test_unstake for testing
    pub(crate) fn unstake(
        &self,
        deps: DepsMut<custom::ConverterQuery>,
        delegator: String,
        validator: String,
        unstake: Coin,
    ) -> Result<custom::Response, ContractError> {
        let amount = self.normalize_price(deps.as_ref(), unstake)?;

        let event = Event::new("mesh-unbond")
            .add_attribute("validator", &validator)
            .add_attribute("amount", amount.amount.to_string());

        let msg = virtual_staking_api::sv::ExecMsg::Unbond {
            delegator,
            validator,
            amount,
        };
        let msg = WasmMsg::Execute {
            contract_addr: self.virtual_stake.load(deps.storage)?.into(),
            msg: to_json_binary(&msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(msg).add_event(event))
    }

    /// This is called by ibc_packet_receive.
    /// It is pulled out into a method, so it can also be called by test_burn for testing
    pub(crate) fn burn(
        &self,
        deps: DepsMut<custom::ConverterQuery>,
        validators: &[String],
        burn: Coin,
    ) -> Result<custom::Response, ContractError> {
        let amount = self.normalize_price(deps.as_ref(), burn)?;

        let event = Event::new("mesh-burn")
            .add_attribute("validators", validators.join(","))
            .add_attribute("amount", amount.amount.to_string());

        let msg = virtual_staking_api::sv::ExecMsg::Burn {
            validators: validators.to_vec(),
            amount,
        };
        let msg = WasmMsg::Execute {
            contract_addr: self.virtual_stake.load(deps.storage)?.into(),
            msg: to_json_binary(&msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(msg).add_event(event))
    }

    fn normalize_price(
        &self,
        deps: Deps<custom::ConverterQuery>,
        amount: Coin,
    ) -> Result<Coin, ContractError> {
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            config.remote_denom,
            amount.denom,
            ContractError::WrongDenom {
                sent: amount.denom,
                expected: config.remote_denom
            }
        );

        // FIXME not sure how to get this to compile with latest sylvia
        // get the price value (usage is a bit clunky, need to use trait and cannot chain Remote::new() with .querier())
        // also see https://github.com/CosmWasm/sylvia/issues/181 to just store Remote in state
        use price_feed_api::sv::Querier;
        use sylvia::types::Remote;
        let remote = Remote::<
            &dyn price_feed_api::PriceFeedApi<
                Error = StdError,
                ExecC = custom::ConverterMsg,
                QueryC = custom::ConverterQuery,
            >,
        >::new(config.price_feed);
        let price = remote.querier(&deps.querier).price()?.native_per_foreign;
        let converted = (amount.amount * price) * config.price_adjustment;

        Ok(Coin {
            denom: config.local_denom,
            amount: converted,
        })
    }

    fn invert_price(
        &self,
        deps: Deps<custom::ConverterQuery>,
        amount: Coin,
    ) -> Result<Coin, ContractError> {
        let config = self.config.load(deps.storage)?;
        ensure_eq!(
            config.local_denom,
            amount.denom,
            ContractError::WrongDenom {
                sent: amount.denom,
                expected: config.local_denom
            }
        );

        // get the price value (usage is a bit clunky, need to use trait and cannot chain Remote::new() with .querier())
        // also see https://github.com/CosmWasm/sylvia/issues/181 to just store Remote in state
        use price_feed_api::sv::Querier;
        use sylvia::types::Remote;
        // Note: it doesn't seem to matter which error type goes here...
        let remote = Remote::<
            &dyn price_feed_api::PriceFeedApi<
                Error = StdError,
                ExecC = custom::ConverterMsg,
                QueryC = custom::ConverterQuery,
            >,
        >::new(config.price_feed);
        let price = remote.querier(&deps.querier).price()?.native_per_foreign;
        let converted = (amount.amount * price.inv().ok_or(ContractError::InvalidPrice {})?)
            * config
                .price_adjustment
                .inv()
                .ok_or(ContractError::InvalidDiscount {})?;

        Ok(Coin {
            denom: config.remote_denom,
            amount: converted,
        })
    }

    pub(crate) fn transfer_rewards(
        &self,
        deps: Deps<custom::ConverterQuery>,
        recipient: String,
        rewards: Coin,
    ) -> Result<CosmosMsg<custom::ConverterMsg>, ContractError> {
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

    fn ensure_authorized(
        &self,
        deps: &DepsMut<custom::ConverterQuery>,
        info: &MessageInfo,
    ) -> Result<(), ContractError> {
        let virtual_stake = self.virtual_stake.load(deps.storage)?;
        ensure_eq!(info.sender, virtual_stake, ContractError::Unauthorized {});

        Ok(())
    }
}

impl ConverterApi for ConverterContract<'_> {
    type Error = ContractError;
    type ExecC = custom::ConverterMsg;
    type QueryC = custom::ConverterQuery;

    /// Rewards tokens (in native staking denom) are sent alongside the message, and should be distributed to all
    /// stakers who staked on this validator. This is tracked on the provider, so we send an IBC packet there.
    fn distribute_reward(
        &self,
        mut ctx: ExecCtx<custom::ConverterQuery>,
        validator: String,
    ) -> Result<custom::Response, Self::Error> {
        self.ensure_authorized(&ctx.deps, &ctx.info)?;

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
    fn distribute_rewards(
        &self,
        mut ctx: ExecCtx<custom::ConverterQuery>,
        payments: Vec<RewardInfo>,
    ) -> Result<custom::Response, Self::Error> {
        self.ensure_authorized(&ctx.deps, &ctx.info)?;

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
    /// Send validator set additions (entering the active validator set), jailings and tombstonings
    /// to the external staking contract on the Consumer via IBC.
    #[allow(clippy::too_many_arguments)]
    fn valset_update(
        &self,
        ctx: ExecCtx<custom::ConverterQuery>,
        additions: Vec<Validator>,
        removals: Vec<String>,
        updated: Vec<Validator>,
        jailed: Vec<String>,
        unjailed: Vec<String>,
        tombstoned: Vec<String>,
        mut slashed: Vec<ValidatorSlashInfo>,
    ) -> Result<custom::Response, Self::Error> {
        self.ensure_authorized(&ctx.deps, &ctx.info)?;

        // Send over IBC to the Consumer
        let channel = IBC_CHANNEL.load(ctx.deps.storage)?;

        let mut event = Event::new("valset_update");
        let mut is_empty = true;

        if !additions.is_empty() {
            event = event.add_attribute(
                "additions",
                additions
                    .iter()
                    .map(|v| v.address.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
            is_empty = false;
        }
        if !removals.is_empty() {
            event = event.add_attribute("removals", removals.join(","));
            is_empty = false;
        }
        if !updated.is_empty() {
            event = event.add_attribute(
                "updated",
                updated
                    .iter()
                    .map(|v| v.address.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
            is_empty = false;
        }
        if !jailed.is_empty() {
            event = event.add_attribute("jailed", jailed.join(","));
            is_empty = false;
        }
        if !unjailed.is_empty() {
            event = event.add_attribute("unjailed", unjailed.join(","));
            is_empty = false;
        }
        if !tombstoned.is_empty() {
            event = event.add_attribute("tombstoned", tombstoned.join(","));
            is_empty = false;
        }
        if !slashed.is_empty() {
            event = event.add_attribute(
                "slashed",
                slashed
                    .iter()
                    .map(|v| v.address.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
            event = event.add_attribute(
                "ratios",
                slashed
                    .iter()
                    .map(|v| v.slash_ratio.clone())
                    .collect::<Vec<String>>()
                    .join(","),
            );
            event = event.add_attribute(
                "amounts",
                slashed
                    .iter()
                    .map(|v| {
                        [
                            v.slash_amount.amount.to_string(),
                            v.slash_amount.denom.clone(),
                        ]
                        .concat()
                    })
                    .collect::<Vec<String>>()
                    .join(","),
            );
            // Convert slash amounts to Provider's coin
            slashed
                .iter_mut()
                .map(|v| {
                    v.slash_amount =
                        self.invert_price(ctx.deps.as_ref(), v.slash_amount.clone())?;
                    Ok(v)
                })
                .collect::<Result<Vec<_>, ContractError>>()?;
            event = event.add_attribute(
                "provider_amounts",
                slashed
                    .iter()
                    .map(|v| {
                        [
                            v.slash_amount.amount.to_string(),
                            v.slash_amount.denom.clone(),
                        ]
                        .concat()
                    })
                    .collect::<Vec<String>>()
                    .join(","),
            );
            is_empty = false;
        }
        let mut resp = Response::new();
        if !is_empty {
            let valset_msg = valset_update_msg(
                &ctx.env,
                &channel,
                &additions,
                &removals,
                &updated,
                &jailed,
                &unjailed,
                &tombstoned,
                &slashed,
            )?;
            resp = resp.add_message(valset_msg);
        }
        resp = resp.add_event(event);
        Ok(resp)
    }

    fn internal_unstake(
        &self,
        ctx: ExecCtx<custom::ConverterQuery>,
        delegator: String,
        validator: String,
        amount: Coin,
    ) -> Result<custom::Response, Self::Error> {
        let virtual_stake = self.virtual_stake.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, virtual_stake, ContractError::Unauthorized);

        #[allow(unused_mut)]
        let mut resp = Response::new()
            .add_attribute("action", "internal_unstake")
            .add_attribute("amount", amount.amount.to_string())
            .add_attribute("owner", delegator.clone());

        let channel = IBC_CHANNEL.load(ctx.deps.storage)?;

        // Recalculate the price when unbond
        let inverted_amount = self.invert_price(ctx.deps.as_ref(), amount.clone())?;
        let packet = ConsumerPacket::InternalUnstake {
            delegator,
            validator,
            normalize_amount: amount,
            inverted_amount,
        };
        let msg = IbcMsg::SendPacket {
            channel_id: channel.endpoint.channel_id,
            data: to_json_binary(&packet)?,
            timeout: packet_timeout_internal_unstake(&ctx.env),
        };
        // send packet if we are ibc enabled
        resp = resp.add_message(msg);
        Ok(resp)
    }
}

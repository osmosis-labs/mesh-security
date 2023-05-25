use cosmwasm_std::{Addr, Decimal, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::converter_api::{self, ConverterApi, RewardInfo};

use crate::error::ContractError;
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct ConverterContract<'a> {
    pub config: Item<'a, Config>,
    pub virtual_stake: Item<'a, Addr>,
}

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
        virtual_staking: String, // TODO: figure out to pass this in later
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let config = Config {
            price_feed: ctx.deps.api.addr_validate(&price_feed)?,
            // TODO: better error if discount greater than 1 (this will panic)
            adjustment: Decimal::one() - discount,
        };
        self.config.save(ctx.deps.storage, &config)?;

        let virtual_staking = ctx.deps.api.addr_validate(&virtual_staking)?;
        self.virtual_stake
            .save(ctx.deps.storage, &virtual_staking)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
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

use cosmwasm_std::{Decimal, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::price_feed_api::{self, PriceFeedApi, PriceResponse};

use crate::error::ContractError;
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct SimplePriceFeedContract<'a> {
    pub config: Item<'a, Config>,
}

#[contract]
#[error(ContractError)]
#[messages(price_feed_api as PriceFeedApi)]
impl SimplePriceFeedContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// Sets up the contract with an initial price.
    /// If the owner is not set in the message, it defaults to info.sender.
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        native_per_foreign: Decimal,
        owner: Option<String>,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;
        let owner = match owner {
            Some(owner) => ctx.deps.api.addr_validate(&owner)?,
            None => ctx.info.sender,
        };
        let config = Config {
            native_per_foreign,
            owner,
        };
        self.config.save(ctx.deps.storage, &config)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
        Ok(Response::new())
    }

    #[msg(exec)]
    fn update_price(
        &self,
        ctx: ExecCtx,
        native_per_foreign: Decimal,
    ) -> Result<Response, ContractError> {
        let mut config = self.config.load(ctx.deps.storage)?;
        config.native_per_foreign = native_per_foreign;
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        Ok(ConfigResponse {
            owner: config.owner.into_string(),
            native_per_foreign: config.native_per_foreign,
        })
    }
}

#[contract]
#[messages(converter_api as PriceFeedApi)]
impl PriceFeedApi for SimplePriceFeedContract<'_> {
    type Error = ContractError;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    #[msg(query)]
    fn price(&self, ctx: QueryCtx) -> Result<PriceResponse, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        Ok(PriceResponse {
            native_per_foreign: config.native_per_foreign,
        })
    }
}

use cosmwasm_std::{Decimal, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use sylvia::types::{InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use mesh_apis::price_feed_api::{self, PriceFeedApi, PriceResponse};

use crate::error::ContractError;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct RemotePriceFeedContract {
    pub native_per_foreign: Item<'static, Decimal>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(price_feed_api as PriceFeedApi)]
impl RemotePriceFeedContract {
    pub const fn new() -> Self {
        Self {
            native_per_foreign: Item::new("price"),
        }
    }

    /// Sets up the contract with an initial price.
    /// If the owner is not set in the message, it defaults to info.sender.
    #[msg(instantiate)]
    pub fn instantiate(&self, ctx: InstantiateCtx) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;
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
        let native_per_foreign = self.native_per_foreign.may_load(ctx.deps.storage)?;
        native_per_foreign
            .map(|price| PriceResponse {
                native_per_foreign: price,
            })
            .ok_or(ContractError::NoPriceData)
    }
}

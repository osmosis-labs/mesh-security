use cosmwasm_std::{ensure_eq, Decimal, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, SudoCtx};
use sylvia::{contract, schemars};

use mesh_apis::price_feed_api::{self, PriceFeedApi, PriceResponse};

use crate::error::ContractError;
use crate::msg::ConfigResponse;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(not(any(test, feature = "mt")))]
pub type PriceFeedCustomMsg = cosmwasm_std::Empty;
#[cfg(any(test, feature = "mt"))]
pub type PriceFeedCustomMsg = mesh_bindings::VirtualStakeCustomMsg;

#[cfg(not(any(test, feature = "mt")))]
pub type PriceFeedCustomQuery = cosmwasm_std::Empty;
#[cfg(any(test, feature = "mt"))]
pub type PriceFeedCustomQuery = mesh_bindings::VirtualStakeCustomQuery;

pub struct SimplePriceFeedContract<'a> {
    pub config: Item<'a, Config>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
#[sv::messages(price_feed_api as PriceFeedApi: custom(msg, query))]
// #[cfg_attr(any(test, feature = "mt"), sv::messages(price_feed_api as PriceFeedApi: custom(msg, query)))]
// #[cfg_attr(not(any(test, feature = "mt")), sv::messages(price_feed_api as PriceFeedApi))]
/// Workaround for lack of support in communication `Empty` <-> `Custom` Contracts.
#[sv::custom(query=PriceFeedCustomQuery, msg=PriceFeedCustomMsg)]
impl SimplePriceFeedContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// Sets up the contract with an initial price.
    /// If the owner is not set in the message, it defaults to info.sender.
    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx<PriceFeedCustomQuery>,
        native_per_foreign: Decimal,
        owner: Option<String>,
    ) -> Result<Response<PriceFeedCustomMsg>, ContractError> {
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

    #[sv::msg(exec)]
    fn update_price(
        &self,
        ctx: ExecCtx<PriceFeedCustomQuery>,
        native_per_foreign: Decimal,
    ) -> Result<Response<PriceFeedCustomMsg>, ContractError> {
        nonpayable(&ctx.info)?;

        let mut config = self.config.load(ctx.deps.storage)?;

        // Only allow owner to call this
        ensure_eq!(
            ctx.info.sender,
            config.owner,
            ContractError::Unauthorized {}
        );

        config.native_per_foreign = native_per_foreign;
        self.config.save(ctx.deps.storage, &config)?;
        Ok(Response::new())
    }

    #[sv::msg(query)]
    fn config(&self, ctx: QueryCtx<PriceFeedCustomQuery>) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        Ok(ConfigResponse {
            owner: config.owner.into_string(),
            native_per_foreign: config.native_per_foreign,
        })
    }
}

impl PriceFeedApi for SimplePriceFeedContract<'_> {
    type Error = ContractError;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    fn price(&self, ctx: QueryCtx) -> Result<PriceResponse, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        Ok(PriceResponse {
            native_per_foreign: config.native_per_foreign,
        })
    }

    /// Nothing needs to be done on the epoch
    fn handle_epoch(&self, _ctx: SudoCtx) -> Result<Response, Self::Error> {
        Ok(Response::new())
    }
}

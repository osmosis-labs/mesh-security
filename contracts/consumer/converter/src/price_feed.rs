use mesh_apis::price_feed_api::PriceResponse;
use crate::contract::custom;
use cosmwasm_std::{Response, StdError};
use sylvia::ctx::{QueryCtx, SudoCtx};
use sylvia::interface;

// Custom price feed api from price_feed_api::PriceFeedApi
// Due to the fact that the price_feed_api::PriceFeedApi can not implement custom trait
// we need to redefine the structure
#[interface]
#[sv::custom(query=custom::ConverterQuery, msg=custom::ConverterMsg)]
pub trait CustomPriceFeedApi {
    type Error: From<StdError>;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    #[sv::msg(query)]
    fn price(&self, ctx: QueryCtx<custom::ConverterQuery>) -> Result<PriceResponse, Self::Error>;

    #[sv::msg(sudo)]
    fn handle_epoch(
        &self,
        ctx: SudoCtx<custom::ConverterQuery>,
    ) -> Result<Response<custom::ConverterMsg>, Self::Error>;
}

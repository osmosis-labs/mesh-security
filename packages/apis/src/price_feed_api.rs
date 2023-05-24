use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, StdError};
use sylvia::types::QueryCtx;
use sylvia::{interface, schemars};

/// This is a common interface to any price feed provider.
/// It may be a minimal example with a single price set by an governance vote,
/// pull data from TWAP of an on-chain DEX, get remote TWAP data via IBC,
/// or use some off-chain oracle system.
///
/// It only has one pair of tokens and returns a single price.
#[interface]
pub trait PriceFeedApi {
    type Error: From<StdError>;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    #[msg(query)]
    fn price(&self, ctx: QueryCtx) -> Result<PriceResponse, Self::Error>;
}

#[cw_serde]
pub struct PriceResponse {
    pub native_per_foreign: Decimal,
}

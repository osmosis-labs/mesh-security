use cosmwasm_schema::cw_serde;
use cosmwasm_std::{CustomMsg, CustomQuery, Decimal, Response, StdError};
use sylvia::types::{QueryCtx, SudoCtx};
use sylvia::{interface, schemars};

/// This is a common interface to any price feed provider.
/// It may be a minimal example with a single price set by a governance vote,
/// pull data from TWAP of an on-chain DEX, get remote TWAP data via IBC,
/// or use some off-chain oracle system.
///
/// It only has one pair of tokens and returns a single price.
#[interface]
pub trait PriceFeedApi {
    type Error: From<StdError>;
    type ExecC: CustomMsg;
    type QueryC: CustomQuery;

    /// Return the price of the foreign token. That is, how many native tokens
    /// are needed to buy one foreign token.
    #[sv::msg(query)]
    fn price(&self, ctx: QueryCtx<Self::QueryC>) -> Result<PriceResponse, Self::Error>;

    #[sv::msg(sudo)]
    fn handle_epoch(
        &self,
        ctx: SudoCtx<Self::QueryC>,
    ) -> Result<Response<Self::ExecC>, Self::Error>;
}

#[cw_serde]
pub struct PriceResponse {
    pub native_per_foreign: Decimal,
}

use cosmwasm_schema::cw_serde;
use cosmwasm_std::Decimal;

#[cw_serde]
pub struct ConfigResponse {
    pub adjustment: Decimal,

    /// Address of the contract we query for the price feed to normalize the foreign asset into native tokens.
    pub price_feed: String,

    /// Address of the virtual staking contract.
    pub virtual_staking: String,
}

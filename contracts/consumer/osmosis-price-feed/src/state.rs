use cosmwasm_schema::cw_serde;

#[cw_serde]
pub struct TradingPair {
    pub pool_id: u64,
    pub base_asset: String,
    pub quote_asset: String,
}

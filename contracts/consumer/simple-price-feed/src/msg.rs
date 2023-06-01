use cosmwasm_schema::cw_serde;
use cosmwasm_std::Decimal;

#[cw_serde]
pub struct ConfigResponse {
    /// Owner who can update price
    pub owner: String,

    /// The current set price
    pub native_per_foreign: Decimal,
}

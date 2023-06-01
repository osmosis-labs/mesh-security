use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: Addr,
}

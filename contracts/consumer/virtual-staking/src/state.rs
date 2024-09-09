use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: Addr,

    /// Maximum delegations per query
    pub max_retrieve: u32,

    /// If it enable, tombstoned validators will be unbond automatically
    pub tombstoned_unbond_enable: bool,
}

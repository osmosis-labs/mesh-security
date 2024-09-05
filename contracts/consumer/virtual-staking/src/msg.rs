use cosmwasm_schema::cw_serde;
use cosmwasm_std::Uint128;

use crate::state::Config;

#[cw_serde]
pub struct ConfigResponse {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: String,
}

#[cw_serde]
pub struct StakeResponse {
    pub stake: Uint128,
}

#[cw_serde]
pub struct AllStakeResponse {
    pub stakes: Vec<(String, Uint128)>,
}

impl From<Config> for ConfigResponse {
    fn from(config: Config) -> Self {
        Self {
            denom: config.denom,
            converter: config.converter.into(),
        }
    }
}

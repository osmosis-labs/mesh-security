use cosmwasm_schema::cw_serde;

use crate::state::Config;

#[cw_serde]
pub struct ConfigResponse {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: String,
}

impl From<Config> for ConfigResponse {
    fn from(config: Config) -> Self {
        Self {
            denom: config.denom,
            converter: config.converter.into(),
        }
    }
}

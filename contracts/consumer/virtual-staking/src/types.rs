use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the converter contract (that is authorized to bond/unbond and will receive rewards)
    pub converter: Addr,
}

/**** api ****/

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

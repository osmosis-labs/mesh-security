use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The code id for the `native-staking-proxy` contracts we will be managing
    pub proxy_code_id: u64,

    /// The address of the vault contract (where we get and return stake)
    pub vault: Addr,
}

/**** api ****/

pub type ConfigResponse = Config;

#[cw_serde]
pub struct ProxyByOwnerResponse {
    pub proxy: String,
}

#[cw_serde]
pub struct OwnerByProxyResponse {
    pub owner: String,
}

/// The message that is binary encoded in `receive_stake(..msg)`
#[cw_serde]
pub struct StakeMsg {
    pub validator: String,
}

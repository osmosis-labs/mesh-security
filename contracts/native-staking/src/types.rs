use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// This is the code id for native-staking-proxy contract we will be managing
    pub proxy_code_id: u64,

    /// The address of the vault contract (where we get and return stake)
    pub vault: Addr,
}

/**** api ****/

#[cw_serde]
pub struct ConfigResponse {
    /// The denom we accept for staking
    pub denom: String,

    /// This is the code id for native-staking-proxy contract we will be managing
    pub proxy_code_id: u64,

    /// The address of the vault contract (where we get and return stake)
    pub vault: String,
}

#[cw_serde]
pub struct ProxyByOwnerResponse {
    pub proxy: String,
}

#[cw_serde]
pub struct OwnerByProxyResponse {
    pub owner: String,
}

/// This is the message that is binary encoded in receive_stake(..msg)
#[cw_serde]
pub struct StakeMsg {
    pub validator: String,
}

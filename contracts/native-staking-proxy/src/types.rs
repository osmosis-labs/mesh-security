use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

/*** state ***/

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the vault contract (where we get and return stake)
    pub vault: Addr,
}

/**** api ****/

#[cw_serde]
pub struct ConfigResponse {
    pub denom: String,

    /// The address of the vault contract (where we get and return stake)
    pub vault: String,
}

#[cw_serde]
pub struct ClaimsResponse {
    // TODO
}

/// This is the message that is binary encoded in receive_stake(..msg)
#[cw_serde]
pub struct StakeMsg {
    validator: String,
}

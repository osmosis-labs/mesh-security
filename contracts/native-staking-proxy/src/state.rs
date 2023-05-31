use cosmwasm_schema::cw_serde;
use cosmwasm_std::Addr;

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking
    pub denom: String,

    /// The address of the users who controls restaking, voting, unbonding
    pub owner: Addr,

    /// The address of the parent contract (where we get and return stake)
    pub parent: Addr,
}

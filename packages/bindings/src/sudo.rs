use cosmwasm_schema::cw_serde;

#[cw_serde]
pub enum SudoMsg {
    Rebalance {},
}

#[cw_serde]
pub enum RemotePriceFeedSudoMsg {
    EndBlock {},
}

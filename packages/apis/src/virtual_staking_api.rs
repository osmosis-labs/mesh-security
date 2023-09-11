use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Coin, Response, StdError, Validator};
use sylvia::types::ExecCtx;
use sylvia::{interface, schemars};

/// The Virtual Staking API is called from the converter contract to bond and (instantly) unbond tokens.
/// The Virtual Staking contract is responsible for interfacing with the native SDK module, while the converter
/// manages the IBC connection.
#[interface]
pub trait VirtualStakingApi {
    type Error: From<StdError>;

    /// Requests to bond tokens to a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance.
    /// If the max cap is 0, then this will immediately return an error.
    #[msg(exec)]
    fn bond(&self, ctx: ExecCtx, validator: String, amount: Coin) -> Result<Response, Self::Error>;

    /// Requests to unbond tokens from a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance in addition to unbond.
    /// If the virtual staking contract doesn't have at least amount tokens staked to the given validator, this will return an error.
    #[msg(exec)]
    fn unbond(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, Self::Error>;
}

#[cw_serde]
pub enum SudoMsg {
    /// SudoMsg::Rebalance{} should be called once per epoch by the sdk (in EndBlock).
    /// It allows the virtual staking contract to bond or unbond any pending requests, as well
    /// as to perform a rebalance if needed (over the max cap).
    ///
    /// It should also withdraw all pending rewards here, and send them to the converter contract.
    Rebalance {},
    /// SudoMsg::ValsetUpdate{} should be called every time there's a validator set update: addition
    /// of a new validator to the active validator set, removal of a validator from the
    /// active validator set, or permanent removal (i.e. tombstoning) of a validator from the active
    /// validator set.
    ValsetUpdate {
        additions: Vec<Validator>,
        removals: Vec<Validator>,
        tombstones: Vec<Validator>,
    },
}

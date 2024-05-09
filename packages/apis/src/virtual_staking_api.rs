use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Coin, Response, StdError, Uint128, Validator};
use sylvia::types::{ExecCtx, SudoCtx};
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
    #[sv::msg(exec)]
    fn bond(&self, ctx: ExecCtx, validator: String, amount: Coin) -> Result<Response, Self::Error>;

    /// Requests to unbond tokens from a validator. This will be actually handled at the next epoch.
    /// If the virtual staking module is over the max cap, it will trigger a rebalance in addition to unbond.
    /// If the virtual staking contract doesn't have at least amount tokens staked to the given validator, this will return an error.
    #[sv::msg(exec)]
    fn unbond(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, Self::Error>;

    /// Burns stake. This is called when the user's collateral is slashed and, as part of slashing
    /// propagation, the virtual staking contract needs to burn / discount the indicated slashing amount.
    /// Undelegates evenly from all `validators`.
    #[sv::msg(exec)]
    fn burn(
        &self,
        ctx: ExecCtx,
        validators: Vec<String>,
        amount: Coin,
    ) -> Result<Response, Self::Error>;

    /// SudoMsg::HandleEpoch{} should be called once per epoch by the sdk (in EndBlock).
    /// It allows the virtual staking contract to bond or unbond any pending requests, as well
    /// as to perform a rebalance if needed (over the max cap).
    ///
    /// It should also withdraw all pending rewards here, and send them to the converter contract.
    #[sv::msg(sudo)]
    fn handle_epoch(&self, ctx: SudoCtx) -> Result<Response, Self::Error>;

    /// SudoMsg::ValsetUpdate{} should be called every time there's a validator set update:
    ///  - Addition of a new validator to the active validator set.
    ///  - Temporary removal of a validator from the active set. (i.e. `unbonded` state).
    ///  - Update of validator data.
    ///  - Temporary removal of a validator from the active set due to jailing. Implies slashing.
    ///  - Addition of an existing validator to the active validator set.
    ///  - Permanent removal (i.e. tombstoning) of a validator from the active set. Implies slashing
    #[allow(clippy::too_many_arguments)]
    #[sv::msg(sudo)]
    fn handle_valset_update(
        &self,
        ctx: SudoCtx,
        additions: Option<Vec<Validator>>,
        removals: Option<Vec<String>>,
        updated: Option<Vec<Validator>>,
        jailed: Option<Vec<String>>,
        unjailed: Option<Vec<String>>,
        tombstoned: Option<Vec<String>>,
        slashed: Option<Vec<ValidatorSlash>>,
    ) -> Result<Response, Self::Error>;
}

#[cw_serde]
pub struct ValidatorSlash {
    /// The address of the validator.
    pub address: String,
    /// The height at which the slash is being processed.
    pub height: u64,
    /// The time at which the slash is being processed, in seconds.
    pub time: u64,
    /// The height at which the misbehaviour occurred.
    pub infraction_height: u64,
    /// The time at which the misbehaviour occurred, in seconds.
    pub infraction_time: u64,
    /// The validator power when the misbehaviour occurred.
    pub power: u64,
    /// The slashed amount over the virtual-staking contract.
    pub slash_amount: Uint128,
    /// The (nominal) slash ratio for the validator.
    /// Useful in case we don't know if it's a double sign or downtime slash.
    pub slash_ratio: String,
}

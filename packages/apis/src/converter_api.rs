use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Coin, Response, StdError, Uint128, Validator};
use sylvia::types::ExecCtx;
use sylvia::{interface, schemars};

/// The converter API is all calls that can be made from the virtual staking contract on this contract.
/// Updating the discount rate may be a custom API (such as SudoMsg), and all interactions with the
/// provider occur over IBC, so this is rather minimal
#[interface]
pub trait ConverterApi {
    type Error: From<StdError>;

    /// Rewards tokens (in native staking denom) are sent alongside the message, and should be distributed to all
    /// stakers who staked on this validator.
    #[msg(exec)]
    fn distribute_reward(&self, ctx: ExecCtx, validator: String) -> Result<Response, Self::Error>;

    /// This is a batch for of distribute_reward, including the payment for multiple validators.
    /// This is more efficient than calling distribute_reward multiple times, but also more complex.
    ///
    /// info.funds sent along with the message should be the sum of all rewards for all validators,
    /// in the native staking denom.
    #[msg(exec)]
    fn distribute_rewards(
        &self,
        ctx: ExecCtx,
        payments: Vec<RewardInfo>,
    ) -> Result<Response, Self::Error>;

    /// Valset updates.
    ///
    /// TODO: pubkeys need to be part of the Validator struct (requires CosmWasm support).
    #[allow(clippy::too_many_arguments)]
    #[msg(exec)]
    fn valset_update(
        &self,
        ctx: ExecCtx,
        additions: Vec<Validator>,
        removals: Vec<String>,
        updated: Vec<Validator>,
        jailed: Vec<String>,
        unjailed: Vec<String>,
        tombstoned: Vec<String>,
        slashed: Vec<ValidatorSlashInfo>,
    ) -> Result<Response, Self::Error>;
}

#[cw_serde]
#[derive(PartialOrd, Eq, Ord)]
pub struct RewardInfo {
    pub validator: String,
    pub reward: Uint128,
}

#[cw_serde]
pub struct ValidatorSlashInfo {
    /// The address of the validator.
    pub address: String,
    /// The height at which the misbehaviour occurred.
    pub infraction_height: u64,
    /// The time at which the misbehaviour occurred, in seconds.
    pub infraction_time: u64,
    /// The validator power when the misbehaviour occurred.
    pub power: u64,
    /// The slash amount over the amount delegated by virtual-staking for the validator.
    pub slash_amount: Coin,
    /// The (nominal) slash ratio for the validator.
    /// Useful in case we don't know if it's a double sign or downtime slash.
    pub slash_ratio: String,
}

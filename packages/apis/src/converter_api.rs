use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Response, StdError, Uint128, Validator};
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
    /// Only additions and permanent removals are accepted, as removals (leaving the active
    /// validator set) are non-permanent and ignored on the Provider (CRDTs only support permanent
    /// removals).
    ///
    /// If a validator that already exists in the list is re-sent for addition, its pubkey
    /// will be updated.
    /// TODO: pubkeys need to be part of the Validator struct (requires CosmWasm support).
    #[msg(exec)]
    fn valset_update(
        &self,
        ctx: ExecCtx,
        additions: Vec<Validator>,
        tombstoned: Vec<String>,
    ) -> Result<Response, Self::Error>;

    /// Slashing routing.
    /// To be sent to the Provider for processing
    #[msg(exec)]
    fn slash(
        &self,
        ctx: ExecCtx,
        validator: String,
        height: u64,
        time: u64,
        tombstone: bool,
    ) -> Result<Response, Self::Error>;
}

#[cw_serde]
#[derive(PartialOrd, Eq, Ord)]
pub struct RewardInfo {
    pub validator: String,
    pub reward: Uint128,
}

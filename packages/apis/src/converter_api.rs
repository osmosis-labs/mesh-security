use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Response, StdError, Uint128};
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
}

#[cw_serde]
pub struct RewardInfo {
    pub validator: String,
    pub reward: Uint128,
}

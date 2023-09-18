use cosmwasm_std::{Coin, Response, StdError};
use mesh_apis::converter_api::RewardInfo;
use mesh_apis::ibc::AddValidator;
use sylvia::interface;
use sylvia::types::ExecCtx;

/// Interface to work around lack of support for IBC in `cw-multi-test`
/// This interface is for test usage only
#[interface]
pub trait TestMethods {
    type Error: From<StdError>;

    /// Commits a pending stake.
    #[msg(exec)]
    fn test_commit_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// Rollbacks a pending stake.
    #[msg(exec)]
    fn test_rollback_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// Updates the active validator set.
    #[msg(exec)]
    fn test_set_active_validator(
        &self,
        ctx: ExecCtx,
        validator: AddValidator,
    ) -> Result<Response, Self::Error>;

    /// Commits a pending unstake.
    #[msg(exec)]
    fn test_commit_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// Rollbacks a pending unstake.
    #[msg(exec)]
    fn test_rollback_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// Distribute rewards.
    #[msg(exec)]
    fn test_distribute_rewards(
        &self,
        ctx: ExecCtx,
        validator: String,
        rewards: Coin,
    ) -> Result<Response, Self::Error>;

    /// Batch distribute rewards.
    #[msg(exec)]
    fn test_distribute_rewards_batch(
        &self,
        ctx: ExecCtx,
        denom: String,
        rewards: Vec<RewardInfo>,
    ) -> Result<Response, Self::Error>;

    /// Commits a withdraw rewards transaction.
    #[msg(exec)]
    fn test_commit_withdraw_rewards(
        &self,
        ctx: ExecCtx,
        tx_id: u64,
    ) -> Result<Response, Self::Error>;

    /// Rollbacks a withdraw rewards transaction.
    #[msg(exec)]
    fn test_rollback_withdraw_rewards(
        &self,
        ctx: ExecCtx,
        tx_id: u64,
    ) -> Result<Response, Self::Error>;
}

use crate::contract::ExternalStakingContract;
use crate::error::ContractError;
use crate::test_methods::TestMethods;

use cosmwasm_std::{Coin, Response};
use mesh_apis::converter_api::RewardInfo;
use mesh_apis::ibc::AddValidator;
use sylvia::contract;
use sylvia::types::ExecCtx;

/// These methods are for test usage only
#[contract(module=crate::contract)]
#[messages(crate::test_methods as TestMethods)]
impl TestMethods for ExternalStakingContract<'_> {
    type Error = ContractError;

    /// Commits a pending stake.
    #[msg(exec)]
    fn test_commit_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(any(feature = "mt", test))]
        {
            let msg = self.commit_stake(ctx.deps, tx_id)?;
            Ok(Response::new().add_message(msg))
        }
        #[cfg(not(any(feature = "mt", test)))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Rollbacks a pending stake.
    #[msg(exec)]
    fn test_rollback_stake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            let msg = self.rollback_stake(ctx.deps, tx_id)?;
            Ok(Response::new().add_message(msg))
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Updates the active validator set.
    #[msg(exec)]
    fn test_set_active_validator(
        &self,
        ctx: ExecCtx,
        validator: AddValidator,
        height: u64,
        time: u64,
    ) -> Result<Response, ContractError> {
        #[cfg(any(feature = "mt", test))]
        {
            let AddValidator { valoper, pub_key } = validator;
            self.val_set
                .add_validator(ctx.deps.storage, &valoper, &pub_key, height, time)?;
            Ok(Response::new())
        }
        #[cfg(not(any(feature = "mt", test)))]
        {
            let _ = (ctx, validator, height, time);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Sets validator as `unbonded`.
    #[msg(exec)]
    fn test_remove_validator(
        &self,
        ctx: ExecCtx,
        valoper: String,
        height: u64,
        time: u64,
    ) -> Result<Response, ContractError> {
        #[cfg(any(feature = "mt", test))]
        {
            self.val_set
                .remove_validator(ctx.deps.storage, &valoper, height, time)?;
            Ok(Response::new())
        }
        #[cfg(not(any(feature = "mt", test)))]
        {
            let _ = (ctx, valoper, height, time);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Sets validator as `unbonded`.
    #[msg(exec)]
    fn test_tombstone_validator(
        &self,
        ctx: ExecCtx,
        valoper: String,
        height: u64,
        time: u64,
    ) -> Result<Response, ContractError> {
        #[cfg(any(feature = "mt", test))]
        {
            self.val_set
                .tombstone_validator(ctx.deps.storage, &valoper, height, time)?;
            Ok(Response::new())
        }
        #[cfg(not(any(feature = "mt", test)))]
        {
            let _ = (ctx, valoper, height, time);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Commits a pending unstake.
    #[msg(exec)]
    fn test_commit_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            self.commit_unstake(ctx.deps, ctx.env, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Rollbacks a pending unstake.
    #[msg(exec)]
    fn test_rollback_unstake(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            self.rollback_unstake(ctx.deps, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Distribute rewards.
    #[msg(exec)]
    fn test_distribute_rewards(
        &self,
        ctx: ExecCtx,
        validator: String,
        rewards: Coin,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            let event = self.distribute_rewards(ctx.deps, &validator, rewards)?;
            Ok(Response::new().add_event(event))
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, validator, rewards);
            Err(ContractError::Unauthorized)
        }
    }

    /// Batch distribute rewards.
    #[msg(exec)]
    fn test_distribute_rewards_batch(
        &self,
        ctx: ExecCtx,
        denom: String,
        rewards: Vec<RewardInfo>,
    ) -> Result<Response, Self::Error> {
        #[cfg(any(test, feature = "mt"))]
        {
            let events = self.distribute_rewards_batch(ctx.deps, &rewards, &denom)?;
            Ok(Response::new().add_events(events))
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, denom, rewards);
            Err(ContractError::Unauthorized)
        }
    }

    /// Commits a withdraw rewards transaction.
    #[msg(exec)]
    fn test_commit_withdraw_rewards(
        &self,
        ctx: ExecCtx,
        tx_id: u64,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            self.commit_withdraw_rewards(ctx.deps, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Rollbacks a withdraw rewards transaction.
    #[msg(exec)]
    fn test_rollback_withdraw_rewards(
        &self,
        ctx: ExecCtx,
        tx_id: u64,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            self.rollback_withdraw_rewards(ctx.deps, tx_id)?;
            Ok(Response::new())
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, tx_id);
            Err(ContractError::Unauthorized {})
        }
    }

    /// Slashes a validator
    #[msg(exec)]
    fn test_handle_slashing(
        &self,
        ctx: ExecCtx,
        validator: String,
    ) -> Result<Response, ContractError> {
        #[cfg(any(test, feature = "mt"))]
        {
            let cfg = self.config.load(ctx.deps.storage)?;
            let slash_msg = self.handle_slashing(&ctx.env, ctx.deps.storage, &cfg, &validator)?;
            match slash_msg {
                Some(msg) => Ok(Response::new().add_message(msg)),
                None => Ok(Response::new()),
            }
        }
        #[cfg(not(any(test, feature = "mt")))]
        {
            let _ = (ctx, validator);
            Err(ContractError::Unauthorized {})
        }
    }
}

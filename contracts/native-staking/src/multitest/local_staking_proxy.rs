use cosmwasm_std::{Coin, Response, StdResult, VoteOption, WeightedVoteOption};

use sylvia::contract;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};

/// This is a stub implementation of the local staking proxy contract, for test purposes only.
/// When proper local staking proxy contract is available, this should be replaced in multitests
pub struct LocalStakingProxy;

#[contract]
impl LocalStakingProxy {
    pub const fn new() -> Self {
        Self
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        _ctx: InstantiateCtx,
        _denom: String,
        _owner: String,
        _validator: String,
    ) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn stake(&self, _ctx: ExecCtx, _validator: String) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn restake(
        &self,
        _ctx: ExecCtx,
        _from_validator: String,
        _to_validator: String,
        _amount: Coin,
    ) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn vote(&self, _ctx: ExecCtx, _proposal_id: String, _vote: VoteOption) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn vote_weighted(
        &self,
        _ctx: ExecCtx,
        _proposal_id: String,
        _vote: Vec<WeightedVoteOption>,
    ) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn withdraw_rewards(&self, _ctx: ExecCtx) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn unstake(&self, _ctx: ExecCtx, _validator: String, _amount: Coin) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(exec)]
    fn release_unbonded(&self, _ctx: ExecCtx) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(query)]
    fn config(&self, _ctx: QueryCtx) -> StdResult<Response> {
        Ok(Response::new())
    }

    #[msg(query)]
    fn unbonding(&self, _ctx: QueryCtx) -> StdResult<Response> {
        Ok(Response::new())
    }
}

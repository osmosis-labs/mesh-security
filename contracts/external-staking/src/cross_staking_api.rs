use cosmwasm_std::{ensure_eq, Binary, Coin, Decimal, Response};
#[allow(unused_imports)]
use mesh_apis::cross_staking_api::{self, CrossStakingApi};
use mesh_apis::local_staking_api::MaxSlashResponse;
use sylvia::contract;
use sylvia::types::{ExecCtx, QueryCtx};

use crate::contract::ExternalStakingContract;
use crate::error::ContractError;

#[contract]
#[messages(cross_staking_api as CrossStakingApi)]
impl CrossStakingApi for ExternalStakingContract<'_> {
    type Error = ContractError;

    #[msg(exec)]
    fn receive_virtual_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        _msg: Binary,
    ) -> Result<Response, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, config.vault.0, ContractError::Unauthorized);

        ensure_eq!(
            amount.denom,
            config.denom,
            ContractError::InvalidDenom(config.denom)
        );

        let owner = ctx.deps.api.addr_validate(&owner)?;

        let mut user = self
            .users
            .may_load(ctx.deps.storage, &owner)?
            .unwrap_or_default();

        user.stake += amount.amount;

        self.users.save(ctx.deps.storage, &owner, &user)?;

        // TODO: Send proper IBC message to remote staking contract

        let resp = Response::new()
            .add_attribute("action", "receive_virtual_stake")
            .add_attribute("owner", owner)
            .add_attribute("amount", amount.amount.to_string());

        Ok(resp)
    }

    #[msg(query)]
    fn max_slash(&self, _ctx: QueryCtx) -> Result<MaxSlashResponse, ContractError> {
        // TODO: Properly set this value
        // Arbitrary value - only to make some testing possible
        //
        // Probably should be queried from remote chain
        let resp = MaxSlashResponse {
            max_slash: Decimal::percent(5),
        };

        Ok(resp)
    }
}

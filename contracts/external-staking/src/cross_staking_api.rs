use cosmwasm_std::{ensure_eq, from_binary, Binary, Coin, Decimal, Order, Response};
#[allow(unused_imports)]
use mesh_apis::cross_staking_api::{self, CrossStakingApi};
use mesh_apis::local_staking_api::MaxSlashResponse;
use sylvia::contract;
use sylvia::types::{ExecCtx, QueryCtx};

use crate::contract::ExternalStakingContract;
use crate::error::ContractError;
use crate::msg::ReceiveVirtualStake;

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
        msg: Binary,
    ) -> Result<Response, Self::Error> {
        let config = self.config.load(ctx.deps.storage)?;
        ensure_eq!(ctx.info.sender, config.vault.0, ContractError::Unauthorized);

        ensure_eq!(
            amount.denom,
            config.denom,
            ContractError::InvalidDenom(config.denom)
        );

        let owner = ctx.deps.api.addr_validate(&owner)?;

        let msg: ReceiveVirtualStake = from_binary(&msg)?;

        // For now we assume there is only single validator per user, however we want to maintain
        // proper structure keeping `(user, validator)` as bound index

        let stakes: Vec<_> = self
            .stakes
            .prefix(&owner)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .collect();

        ensure_eq!(stakes.len(), 1, ContractError::InvariantsNotMet);

        let (validator, mut stake) = stakes.into_iter().next().transpose()?.unwrap_or_else(|| {
            let validator = msg.validator.clone();
            let stake = Default::default();
            (validator, stake)
        });

        ensure_eq!(
            validator,
            msg.validator,
            ContractError::InvalidValidator(msg.validator)
        );

        stake.stake += amount.amount;

        self.stakes
            .save(ctx.deps.storage, (&owner, &validator), &stake)?;

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

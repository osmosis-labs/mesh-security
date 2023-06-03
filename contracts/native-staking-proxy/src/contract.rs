use cosmwasm_std::WasmMsg::Execute;
use cosmwasm_std::{
    coin, ensure_eq, to_binary, Coin, DistributionMsg, GovMsg, Response, StakingMsg, VoteOption,
    WeightedVoteOption,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;

use cw_utils::must_pay;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::msg::{ClaimsResponse, ConfigResponse, OwnerMsg};
use crate::native_staking_callback;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct NativeStakingProxyContract<'a> {
    config: Item<'a, Config>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
impl NativeStakingProxyContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    /// The caller of the instantiation will be the native-staking contract.
    /// We stake `funds.info` on the given validator
    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        owner: String,
        validator: String,
    ) -> Result<Response, ContractError> {
        let config = Config {
            denom,
            parent: ctx.info.sender.clone(),
            owner: ctx.deps.api.addr_validate(&owner)?,
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        // Stake info.funds on validator
        let res = self.stake(ctx, validator)?;

        // Set owner as recipient of future withdrawals
        let set_withdrawal = DistributionMsg::SetWithdrawAddress {
            address: config.owner.into_string(),
        };

        // Pass owner to caller's reply handler
        let owner_msg = to_binary(&OwnerMsg { owner })?;
        Ok(res.add_message(set_withdrawal).set_data(owner_msg))
    }

    /// Stakes the tokens from `info.funds` to the given validator.
    /// Can only be called by the parent contract
    #[msg(exec)]
    fn stake(&self, ctx: ExecCtx, validator: String) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.parent, ctx.info.sender, ContractError::Unauthorized {});

        let amount = must_pay(&ctx.info, &cfg.denom)?;

        let amount = coin(amount.u128(), cfg.denom);
        let msg = StakingMsg::Delegate { validator, amount };

        Ok(Response::new().add_message(msg))
    }

    /// Re-stakes the given amount from the one validator to another on behalf of the calling user.
    /// Returns an error if the user doesn't have such stake
    #[msg(exec)]
    fn restake(
        &self,
        ctx: ExecCtx,
        src_validator: String,
        dst_validator: String,
        amount: Coin,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::InvalidDenom(amount.denom)
        );

        let msg = StakingMsg::Redelegate {
            src_validator,
            dst_validator,
            amount,
        };
        Ok(Response::new().add_message(msg))
    }

    /// Vote with the user's stake (over all delegations)
    #[msg(exec)]
    fn vote(
        &self,
        ctx: ExecCtx,
        proposal_id: u64,
        vote: VoteOption,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});

        let msg = GovMsg::Vote { proposal_id, vote };
        Ok(Response::new().add_message(msg))
    }

    /// Vote with the user's stake (over all delegations)
    #[msg(exec)]
    fn vote_weighted(
        &self,
        ctx: ExecCtx,
        proposal_id: u64,
        vote: Vec<WeightedVoteOption>,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});

        let msg = GovMsg::VoteWeighted {
            proposal_id,
            options: vote,
        };
        Ok(Response::new().add_message(msg))
    }

    /// If the caller has any delegations, withdraw all rewards from those delegations and
    /// send the tokens to the caller.
    /// NOTE: must make sure not to release unbonded tokens
    #[msg(exec)]
    fn withdraw_rewards(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});

        // Withdraw all delegations to the owner (already set as withdrawal address in instantiate)
        let msgs = ctx
            .deps
            .querier
            .query_all_delegations(ctx.env.contract.address)?
            .into_iter()
            .map(|delegation| DistributionMsg::WithdrawDelegatorReward {
                validator: delegation.validator,
            });
        let res = Response::new().add_messages(msgs);
        Ok(res)
    }

    /// Unstakes the given amount from the given validator on behalf of the calling user.
    /// Returns an error if the user doesn't have such stake.
    /// After the unbonding period, it will allow the user to claim the tokens (returning to vault)
    #[msg(exec)]
    fn unstake(
        &self,
        ctx: ExecCtx,
        validator: String,
        amount: Coin,
    ) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});
        ensure_eq!(
            amount.denom,
            cfg.denom,
            ContractError::InvalidDenom(amount.denom)
        );

        // TODO?: Register unbonding as pending (needs unbonding period)

        let msg = StakingMsg::Undelegate { validator, amount };
        Ok(Response::new().add_message(msg))
    }

    /// Releases any tokens that have fully unbonded from a previous unstake.
    /// This will go back to the parent via `release_proxy_stake`.
    /// Errors if the proxy doesn't have any liquid tokens
    #[msg(exec)]
    fn release_unbonded(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        ensure_eq!(cfg.owner, ctx.info.sender, ContractError::Unauthorized {});

        // Simply assume all of our liquid assets are from unbondings
        // TODO?: Get the list of all the completed unbondings
        let balance = ctx
            .deps
            .querier
            .query_balance(ctx.env.contract.address, cfg.denom)?;

        // Send them to the parent contract via `release_proxy_stake`
        let msg = to_binary(&native_staking_callback::ExecMsg::ReleaseProxyStake {})?;

        let wasm_msg = Execute {
            contract_addr: cfg.parent.to_string(),
            msg,
            funds: vec![balance],
        };
        Ok(Response::new().add_message(wasm_msg))
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        Ok(self.config.load(ctx.deps.storage)?)
    }

    /// Returns all pending unbonding.
    /// TODO: can we do that with contract API?
    /// Or better they use CosmJS native delegation queries with this proxy address
    #[msg(query)]
    fn unbonding(&self, _ctx: QueryCtx) -> Result<ClaimsResponse, ContractError> {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmwasm_std::GovMsg::{Vote, VoteWeighted};
    use cosmwasm_std::{Decimal, DepsMut};

    use cosmwasm_std::testing::{mock_dependencies, mock_env, mock_info};
    use cosmwasm_std::VoteOption::Yes;

    static OSMO: &str = "uosmo";
    static CREATOR: &str = "staking"; // The creator of the proxy contract(s) is the staking contract
    static OWNER: &str = "user";
    static VALIDATOR: &str = "validator";

    fn do_instantiate(deps: DepsMut) -> (ExecCtx, NativeStakingProxyContract) {
        let contract = NativeStakingProxyContract::new();
        let mut ctx = InstantiateCtx {
            deps,
            env: mock_env(),
            info: mock_info(CREATOR, &[coin(100, OSMO)]),
        };
        contract
            .instantiate(
                ctx.branch(),
                OSMO.to_owned(),
                OWNER.to_owned(),
                VALIDATOR.to_owned(),
            )
            .unwrap();
        (ctx, contract)
    }

    #[test]
    fn voting() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // the owner can vote
        ctx.info = mock_info(OWNER, &[]);
        let proposal_id = 1;
        let vote = Yes;
        let res = contract
            .vote(ctx.branch(), proposal_id, vote.clone())
            .unwrap();
        assert_eq!(1, res.messages.len());
        // assert it's a governance vote
        assert_eq!(
            res.messages[0].msg,
            cosmwasm_std::CosmosMsg::Gov(Vote {
                proposal_id,
                vote: vote.clone()
            })
        );

        // Nobody else can vote
        ctx.info = mock_info("somebody", &[]);
        let res = contract.vote(ctx.branch(), proposal_id, vote.clone());
        assert!(matches!(res.unwrap_err(), ContractError::Unauthorized {}));

        // Not even the creator
        ctx.info = mock_info(CREATOR, &[]);
        let res = contract.vote(ctx, proposal_id, vote);
        assert!(matches!(res.unwrap_err(), ContractError::Unauthorized {}));
    }

    #[test]
    fn weighted_voting() {
        let mut deps = mock_dependencies();
        let (mut ctx, contract) = do_instantiate(deps.as_mut());

        // The owner can weighted vote
        ctx.info = mock_info(OWNER, &[]);
        let proposal_id = 2;
        let vote = vec![WeightedVoteOption {
            option: Yes,
            weight: Decimal::percent(50),
        }];
        let res = contract
            .vote_weighted(ctx.branch(), proposal_id, vote.clone())
            .unwrap();
        assert_eq!(1, res.messages.len());
        // Assert it's a weighted governance vote
        assert_eq!(
            res.messages[0].msg,
            cosmwasm_std::CosmosMsg::Gov(VoteWeighted {
                proposal_id,
                options: vote.clone()
            })
        );

        // Nobody else can vote
        ctx.info = mock_info("somebody", &[]);
        let res = contract.vote_weighted(ctx.branch(), proposal_id, vote.clone());
        assert!(matches!(res.unwrap_err(), ContractError::Unauthorized {}));

        // Not even the creator
        ctx.info = mock_info(CREATOR, &[]);
        let res = contract.vote_weighted(ctx, proposal_id, vote);
        assert!(matches!(res.unwrap_err(), ContractError::Unauthorized {}));
    }
}

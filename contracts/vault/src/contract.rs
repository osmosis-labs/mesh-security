use cosmwasm_std::{
    entry_point, Addr, Binary, DepsMut, Env, Reply, Response, SubMsg, SubMsgResponse, Uint128,
    WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::parse_instantiate_response_data;

use mesh_apis::local_staking_api::{LocalStakingApiQueryMsg, MaxSlashResponse};
use mesh_apis::vault_api::{self, VaultApi};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::types::{BalanceResponse, Config, ConfigResponse, StakingInitInfo};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 1;

pub struct VaultContract<'a> {
    // TODO
    config: Item<'a, Config>,
}

#[contract(error=ContractError)]
#[messages(vault_api as VaultApi)]
impl VaultContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
        }
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        local_staking: StakingInitInfo,
    ) -> Result<Response, ContractError> {
        let config = Config {
            denom,
            // We set this in reply, so proper once the reply message completes successfully
            local_staking: Addr::unchecked(""),
        };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        // instantiate local_staking and handle reply
        let msg = WasmMsg::Instantiate {
            admin: local_staking.admin,
            code_id: local_staking.code_id,
            msg: local_staking.msg,
            funds: vec![],
            label: local_staking
                .label
                .unwrap_or_else(|| "Mesh Security Local Staking".to_string()),
        };
        // TODO: how to handle reply in sylvia?
        let sub_msg = SubMsg::reply_on_success(msg, REPLY_ID_INSTANTIATE);
        Ok(Response::new().add_submessage(sub_msg))
    }

    #[msg(exec)]
    fn bond(&self, _ctx: ExecCtx) -> Result<Response, ContractError> {
        todo!()
    }

    #[msg(exec)]
    fn unbond(&self, _ctx: ExecCtx, amount: Uint128) -> Result<Response, ContractError> {
        let _ = amount;
        todo!()
    }

    /// This assigns a claim of amount tokens to the remote contract, which can take some action with it
    #[msg(exec)]
    fn stake_remote(
        &self,
        _ctx: ExecCtx,
        // address of the contract to virtually stake on
        contract: String,
        // amount to stake on that contract
        amount: Uint128,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, ContractError> {
        let _ = (contract, amount, msg);
        todo!()
    }

    /// This sends actual tokens to the local staking contract
    #[msg(exec)]
    fn stake_local(
        &self,
        _ctx: ExecCtx,
        // amount to stake on that contract
        amount: Uint128,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, ContractError> {
        let _ = (amount, msg);
        todo!()
    }

    #[msg(query)]
    fn balance(&self, _ctx: QueryCtx, account: String) -> Result<BalanceResponse, ContractError> {
        let _ = account;
        todo!()
    }

    #[msg(query)]
    fn config(&self, _ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        todo!()
    }

    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;
        let local_staking = Addr::unchecked(init_data.contract_address);

        // we want to calculate the slashing rate on this contract and store it locally...
        let query = LocalStakingApiQueryMsg::MaxSlash {};
        let MaxSlashResponse { max_slash } =
            deps.querier.query_wasm_smart(&local_staking, &query)?;
        // TODO: store this when we actually implement the other logic
        let _ = max_slash;

        let mut cfg = self.config.load(deps.storage)?;
        cfg.local_staking = local_staking;
        self.config.save(deps.storage, &cfg)?;

        Ok(Response::new())
    }
}

#[cfg_attr(not(feature = "library"), entry_point)]
pub fn reply(deps: DepsMut, _env: Env, reply: Reply) -> Result<Response, ContractError> {
    match reply.id {
        REPLY_ID_INSTANTIATE => {
            VaultContract::new().reply_init_callback(deps, reply.result.unwrap())
        }
        _ => Err(ContractError::InvalidReplyId(reply.id)),
    }
}

#[contract]
impl VaultApi for VaultContract<'_> {
    type Error = ContractError;

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_cross_stake(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Uint128,
    ) -> Result<Response, ContractError> {
        let _ = (owner, amount);
        todo!()
    }

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local_stake(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
    ) -> Result<Response, ContractError> {
        let _ = owner;
        todo!()
    }
}

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    to_binary, Addr, BankMsg, Binary, Coin, DepsMut, Env, Reply, Response, StdError, SubMsg,
    SubMsgResponse, Uint128, WasmMsg,
};
use cw_utils::{must_pay, nonpayable, ParseReplyError, PaymentError};
use thiserror::Error;

use cw_storage_plus::Item;
use cw_utils::parse_instantiate_response_data;

use mesh_apis::cross_staking_api::CrossStakingApiExecMsg;
use mesh_apis::local_staking_api::LocalStakingApiExecMsg;
use mesh_apis::vault_api::{self, VaultApi};
use sylvia::types::{ExecCtx, InstantiateCtx};
use sylvia::{contract, schemars};

pub const REPLY_ID_INSTANTIATE: u64 = 1;

#[derive(Error, Debug)]
pub enum VaultError {
    #[error("{0}")]
    Std(#[from] StdError),

    #[error("{0}")]
    Payment(#[from] PaymentError),

    #[error("{0}")]
    ParseReply(#[from] ParseReplyError),

    #[error("Invalid reply id: {0}")]
    InvalidReplyId(u64),
}

#[cw_serde]
pub struct Config {
    /// The denom we accept for staking (only native tokens)
    pub denom: String,

    /// The address of the local staking contract (where actual tokens go)
    pub local_staking: Addr,
}

/// This is the info used to construct the native staking contract
#[cw_serde]
pub struct StakingInitInfo {
    /// Admin for the local staking contract. If empty, it is immutable
    pub admin: Option<String>,
    /// Code id used to instantiate the local staking contract
    pub code_id: u64,
    /// JSON-encoded local staking `InstantiateMsg` struct (as raw `Binary`)
    pub msg: Binary,
    /// A human-readable label for the local staking contract (will use a default if not provided)
    pub label: Option<String>,
}

pub struct MockVaultContract<'a> {
    config: Item<'a, Config>,
}

#[contract(error=VaultError)]
#[messages(vault_api as VaultApi)]
impl MockVaultContract<'_> {
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
    ) -> Result<Response, VaultError> {
        let config = Config {
            denom,
            // We set this in reply, so proper once the reply message completes successfully
            local_staking: Addr::unchecked(""),
        };
        self.config.save(ctx.deps.storage, &config)?;

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
        let sub_msg = SubMsg::reply_on_success(msg, REPLY_ID_INSTANTIATE);
        Ok(Response::new().add_submessage(sub_msg))
    }

    // mock so no tracking, just ensure proper denom
    #[msg(exec)]
    fn bond(&self, ctx: ExecCtx) -> Result<Response, VaultError> {
        let cfg = self.config.load(ctx.deps.storage)?;
        let _ = must_pay(&ctx.info, &cfg.denom)?;
        Ok(Response::new())
    }

    // mock so no checks
    #[msg(exec)]
    fn unbond(&self, ctx: ExecCtx, amount: Uint128) -> Result<Response, VaultError> {
        nonpayable(&ctx.info)?;
        let Config { denom, .. } = self.config.load(ctx.deps.storage)?;
        let msg = BankMsg::Send {
            to_address: ctx.info.sender.to_string(),
            amount: vec![Coin { amount, denom }],
        };
        Ok(Response::new().add_message(msg))
    }

    /// This assigns a claim of amount tokens to the remote contract, which can take some action with it
    #[msg(exec)]
    fn stake_remote(
        &self,
        ctx: ExecCtx,
        // address of the contract to virtually stake on
        contract: String,
        // amount to stake on that contract
        amount: Uint128,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, VaultError> {
        nonpayable(&ctx.info)?;

        // embed user-message in the actual message we want
        let stake_msg = CrossStakingApiExecMsg::ReceiveVirtualStake {
            owner: ctx.info.sender.into_string(),
            amount,
            msg,
        };
        let wasm_msg = WasmMsg::Execute {
            contract_addr: contract,
            msg: to_binary(&stake_msg)?,
            funds: vec![],
        };

        Ok(Response::new().add_message(wasm_msg))
    }

    /// This sends actual tokens to the local staking contract
    #[msg(exec)]
    fn stake_local(
        &self,
        ctx: ExecCtx,
        // amount to stake on that contract
        amount: Uint128,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, VaultError> {
        nonpayable(&ctx.info)?;
        let Config {
            denom,
            local_staking,
        } = self.config.load(ctx.deps.storage)?;

        // embed user-message in the actual message we want
        let stake_msg = LocalStakingApiExecMsg::ReceiveStake {
            owner: ctx.info.sender.into_string(),
            msg,
        };
        let wasm_msg = WasmMsg::Execute {
            contract_addr: local_staking.into_string(),
            msg: to_binary(&stake_msg)?,
            funds: vec![Coin { denom, amount }],
        };

        Ok(Response::new().add_message(wasm_msg))
    }

    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, VaultError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;
        let local_staking = Addr::unchecked(init_data.contract_address);

        let mut cfg = self.config.load(deps.storage)?;
        cfg.local_staking = local_staking;
        self.config.save(deps.storage, &cfg)?;

        Ok(Response::new())
    }
}

pub fn reply(deps: DepsMut, _env: Env, reply: Reply) -> Result<Response, VaultError> {
    match reply.id {
        REPLY_ID_INSTANTIATE => {
            MockVaultContract::new().reply_init_callback(deps, reply.result.unwrap())
        }
        _ => Err(VaultError::InvalidReplyId(reply.id)),
    }
}

#[contract]
impl VaultApi for MockVaultContract<'_> {
    type Error = VaultError;

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_cross_stake(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Uint128,
    ) -> Result<Response, VaultError> {
        let _ = (owner, amount);
        // we don't track liens so no-op
        Ok(Response::new())
    }

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local_stake(
        &self,
        _ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
    ) -> Result<Response, VaultError> {
        let _ = owner;
        // we don't track liens so no-op
        Ok(Response::new())
    }
}

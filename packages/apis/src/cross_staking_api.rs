use cosmwasm_schema::cw_serde;
use cosmwasm_std::{to_binary, Addr, Binary, Coin, Deps, Response, StdError, WasmMsg};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

pub use crate::local_staking_api::MaxSlashResponse;

/// This is the interface to any cross staking contract needed by the vault contract.
/// That is, using the vault collateral to stake on a system that doesn't use the collateral
/// as the native staking token. This involves the concept of "virtual stake"
///
/// Users will need to use implementation-specific methods to actually manage funds,
/// this just clarifies the interaction with the Vault contract
#[interface]
pub trait CrossStakingApi {
    type Error: From<StdError>;

    /// Receives stake from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_virtual_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        tx_id: u64,
        msg: Binary,
    ) -> Result<Response, Self::Error>;

    /// Burns stake. This is called when the user's collateral is slashed and, as part of slashing
    /// propagation, the virtual staking contract needs to burn / discount the indicated slashing amount.
    /// If `validator` is set, undelegate preferentially from it first.
    /// If it is not set, undelegate evenly from all validators the user has stake in.
    /// This should be transactional, but it's not. If the transaction fails there isn't much we can
    /// do, besides logging the failure.
    #[msg(exec)]
    fn burn_virtual_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        validator: Option<String>,
    ) -> Result<Response, Self::Error>;

    /// Returns the maximum percentage that can be slashed
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error>;
}

#[cw_serde]
pub struct CrossStakingApiHelper(pub Addr);

impl CrossStakingApiHelper {
    pub fn addr(&self) -> &Addr {
        &self.0
    }

    pub fn receive_virtual_stake(
        &self,
        owner: String,
        amount: Coin,
        tx_id: u64,
        msg: Binary,
        funds: Vec<Coin>,
    ) -> Result<WasmMsg, StdError> {
        let msg = CrossStakingApiExecMsg::ReceiveVirtualStake {
            owner,
            msg,
            amount,
            tx_id,
        };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds,
        };
        Ok(wasm)
    }

    pub fn burn_virtual_stake(
        &self,
        owner: &Addr,
        amount: Coin,
        validator: Option<String>,
    ) -> Result<WasmMsg, StdError> {
        let msg = CrossStakingApiExecMsg::BurnVirtualStake {
            owner: owner.to_string(),
            validator,
            amount,
        };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };
        Ok(wasm)
    }

    pub fn max_slash(&self, deps: Deps) -> Result<MaxSlashResponse, StdError> {
        let query = CrossStakingApiQueryMsg::MaxSlash {};
        deps.querier.query_wasm_smart(&self.0, &query)
    }
}

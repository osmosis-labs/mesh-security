use cosmwasm_schema::cw_serde;
use cosmwasm_std::{to_binary, Addr, Coin, Response, StdError, Uint128, WasmMsg};
use sylvia::types::ExecCtx;
use sylvia::{interface, schemars};

/// This is the interface to the vault contract needed by staking contracts to release funds.
/// Users will need to use the other contract methods to actually manage funds
#[interface]
pub trait VaultApi {
    type Error: From<StdError>;

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_cross_stake(
        &self,
        ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Coin,
    ) -> Result<Response, Self::Error>;

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local_stake(
        &self,
        ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
    ) -> Result<Response, Self::Error>;

    /// This must be called by the remote staking contract to commit the remote staking call on success.
    /// Transaction ID is used to identify the original (vault contract originated) transaction.
    #[msg(exec)]
    fn commit_tx(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// This must be called by the remote staking contract to rollback the remote staking call on failure.
    /// Transaction ID is used to identify the original (vault contract originated) transaction.
    #[msg(exec)]
    fn rollback_tx(&self, ctx: ExecCtx, tx_id: u64) -> Result<Response, Self::Error>;

    /// This must be called by the external staking contract to process a slashing event
    /// because of a misbehaviour on the Consumer chain
    #[msg(exec)]
    fn cross_slash(&self, ctx: ExecCtx, slashes: Vec<SlashInfo>) -> Result<Response, Self::Error>;
}

#[cw_serde]
pub struct SlashInfo {
    pub user: String,
    pub stake: Uint128,
}

#[cw_serde]
pub struct VaultApiHelper(pub Addr);

impl VaultApiHelper {
    pub fn addr(&self) -> &Addr {
        &self.0
    }

    pub fn release_cross_stake(
        &self,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Coin,
        funds: Vec<Coin>,
    ) -> Result<WasmMsg, StdError> {
        let msg = VaultApiExecMsg::ReleaseCrossStake { owner, amount };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds,
        };
        Ok(wasm)
    }

    pub fn release_local_stake(
        &self,
        // address of the user who originally called stake_remote
        owner: String,
        // tokens to send along with this
        funds: Vec<Coin>,
    ) -> Result<WasmMsg, StdError> {
        let msg = VaultApiExecMsg::ReleaseLocalStake { owner };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds,
        };
        Ok(wasm)
    }

    pub fn process_cross_slashing(&self, slashes: Vec<SlashInfo>) -> Result<WasmMsg, StdError> {
        let msg = VaultApiExecMsg::CrossSlash { slashes };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };
        Ok(wasm)
    }

    pub fn commit_tx(&self, tx_id: u64) -> Result<WasmMsg, StdError> {
        let msg = VaultApiExecMsg::CommitTx { tx_id };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };
        Ok(wasm)
    }

    pub fn rollback_tx(&self, tx_id: u64) -> Result<WasmMsg, StdError> {
        let msg = VaultApiExecMsg::RollbackTx { tx_id };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds: vec![],
        };
        Ok(wasm)
    }
}

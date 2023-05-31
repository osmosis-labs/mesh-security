use cosmwasm_schema::cw_serde;
use cosmwasm_std::{to_binary, Addr, Binary, Coin, Decimal, Deps, Response, StdError, WasmMsg};
use sylvia::types::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

#[cw_serde]
pub struct MaxSlashResponse {
    pub max_slash: Decimal,
}

/// This is the interface to any local staking contract needed by the vault contract.
/// Users will need to use the custom methods to actually manage funds
#[interface]
pub trait LocalStakingApi {
    type Error: From<StdError>;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[msg(exec)]
    fn receive_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        // Q: Why is this Binary and not just `validator: String` like before?
        // A: To make it more flexible. Maybe "local" staking is staking a cw20 collateral in the local DAO is belongs to
        // and said DAO requires unbonding period as staking argument and not a validator address.
        //
        // Basically, it allows iterations on various staking designs without touching Vault
        msg: Binary,
    ) -> Result<Response, Self::Error>;

    /// Returns the maximum percentage that can be slashed
    #[msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<MaxSlashResponse, Self::Error>;
}

#[cw_serde]
pub struct LocalStakingApiHelper(pub Addr);

impl LocalStakingApiHelper {
    pub fn addr(&self) -> &Addr {
        &self.0
    }

    pub fn receive_stake(
        &self,
        // address of the user who originally called stake_local
        owner: String,
        // custom to each implementation and opaque to the vault
        msg: Binary,
        // amount to stake on that contract
        funds: Vec<Coin>,
    ) -> Result<WasmMsg, StdError> {
        let msg = LocalStakingApiExecMsg::ReceiveStake { owner, msg };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_binary(&msg)?,
            funds,
        };
        Ok(wasm)
    }

    pub fn max_slash(&self, deps: Deps) -> Result<MaxSlashResponse, StdError> {
        let query = LocalStakingApiQueryMsg::MaxSlash {};
        deps.querier.query_wasm_smart(&self.0, &query)
    }
}

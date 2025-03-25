use cosmwasm_schema::cw_serde;
use cosmwasm_std::{
    to_json_binary, Addr, Binary, Coin, Decimal, Deps, Response, StdError, WasmMsg,
};
use sylvia::ctx::{ExecCtx, QueryCtx};
use sylvia::{interface, schemars};

#[cw_serde]
pub struct SlashRatioResponse {
    pub slash_ratio_dsign: Decimal,
    pub slash_ratio_offline: Decimal,
}

/// This is the interface to any local staking contract needed by the vault contract.
/// Users will need to use the custom methods to actually manage funds
#[interface]
pub trait LocalStakingApi {
    type Error: From<StdError>;

    /// Receives stake (info.funds) from vault contract on behalf of owner and performs the action
    /// specified in msg with it.
    /// Msg is custom to each implementation of the staking contract and opaque to the vault
    #[sv::msg(exec)]
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

    /// Burns stake. This is called when the user's collateral is slashed and, as part of slashing
    /// propagation, the native staking contract needs to burn / discount the indicated slashing amount.
    /// If `validator` is set, undelegate preferentially from it first.
    /// If it is not set, undelegate evenly from all validators the user has stake in.
    #[sv::msg(exec)]
    fn burn_stake(
        &self,
        ctx: ExecCtx,
        owner: String,
        amount: Coin,
        validator: Option<String>,
    ) -> Result<Response, Self::Error>;

    /// Returns the maximum percentage that can be slashed
    #[sv::msg(query)]
    fn max_slash(&self, ctx: QueryCtx) -> Result<SlashRatioResponse, Self::Error>;
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
        let msg = sv::LocalStakingApiExecMsg::ReceiveStake { owner, msg };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_json_binary(&msg)?,
            funds,
        };
        Ok(wasm)
    }

    pub fn burn_stake(
        &self,
        owner: &Addr,
        amount: Coin,
        validator: Option<String>,
    ) -> Result<WasmMsg, StdError> {
        let msg = sv::LocalStakingApiExecMsg::BurnStake {
            owner: owner.to_string(),
            validator,
            amount,
        };
        let wasm = WasmMsg::Execute {
            contract_addr: self.0.to_string(),
            msg: to_json_binary(&msg)?,
            funds: vec![],
        };
        Ok(wasm)
    }

    pub fn max_slash(&self, deps: Deps) -> Result<SlashRatioResponse, StdError> {
        let query = sv::LocalStakingApiQueryMsg::MaxSlash {};
        deps.querier.query_wasm_smart(&self.0, &query)
    }
}

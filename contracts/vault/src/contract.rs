use cosmwasm_std::{
    ensure, entry_point, Addr, Binary, Coin, DepsMut, Env, Order, Reply, Response, StdResult,
    Storage, SubMsg, SubMsgResponse, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Item, Map};
use cw_utils::{must_pay, parse_instantiate_response_data};

use mesh_apis::local_staking_api::{LocalStakingApiQueryMsg, MaxSlashResponse};
use mesh_apis::vault_api::{self, VaultApi};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::msg::{AccountResponse, StakingInitInfo};
use crate::state::{Config, Lien};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 1;

pub struct VaultContract<'a> {
    /// General contract configuration
    config: Item<'a, Config>,
    /// Collateral amount of all users
    collateral: Map<'a, &'a Addr, Uint128>,
    /// All liens in the protocol
    ///
    /// Liens are indexed with (user, creditor), as this pair has to be unique
    liens: Map<'a, (&'a Addr, &'a Addr), Lien>,
}

#[contract]
#[error(ContractError)]
#[messages(vault_api as VaultApi)]
impl VaultContract<'_> {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            collateral: Map::new("collateral"),
            liens: Map::new("liens"),
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
        let sub_msg = SubMsg::reply_on_success(msg, REPLY_ID_INSTANTIATE);
        Ok(Response::new().add_submessage(sub_msg))
    }

    #[msg(exec)]
    fn bond(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let amount = must_pay(&ctx.info, &denom)?;

        self.collateral.update(
            ctx.deps.storage,
            &ctx.info.sender,
            |collat| -> StdResult<_> { Ok(collat.unwrap_or_default() + amount) },
        )?;

        let resp = Response::new()
            .add_attribute("action", "bond")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    #[msg(exec)]
    fn unbond(&self, ctx: ExecCtx, amount: Uint128) -> Result<Response, ContractError> {
        let collateral = self.collateral.load(ctx.deps.storage, &ctx.info.sender)?;

        let free_collateral =
            self.free_collateral(ctx.deps.storage, &ctx.info.sender, collateral)?;
        ensure!(
            free_collateral >= amount,
            ContractError::ClaimsLocked(free_collateral)
        );

        self.collateral
            .save(ctx.deps.storage, &ctx.info.sender, &(collateral - amount))?;

        let resp = Response::new()
            .add_attribute("action", "unbond")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    /// This assigns a claim of amount tokens to the remote contract, which can take some action with it
    #[msg(exec)]
    fn stake_remote(
        &self,
        _ctx: ExecCtx,
        // address of the contract to virtually stake on
        contract: String,
        // amount to stake on that contract
        amount: Coin,
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
        amount: Coin,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, ContractError> {
        let _ = (amount, msg);
        todo!()
    }

    #[msg(query)]
    fn account(&self, _ctx: QueryCtx, account: String) -> Result<AccountResponse, ContractError> {
        let _ = account;
        todo!()
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<Config, ContractError> {
        self.config.load(ctx.deps.storage).map_err(Into::into)
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

    /// Calculates free collateral for an user
    ///
    /// Collateral amount is provided, so when it hs to be used int he contract
    /// itself, it is not read twice from the storage
    fn free_collateral(
        &self,
        storage: &dyn Storage,
        user: &Addr,
        collateral: Uint128,
    ) -> StdResult<Uint128> {
        // Calculating both maximum lien and the slashable collateral in the single
        // range pass to avoid collecting the data, or even worse = multiple state
        // reading
        let (max_lien, total_slashable) = self
            .liens
            .prefix(user)
            .range(storage, None, None, Order::Ascending)
            // (amount, slashable) per lien
            .map(|lien| lien.map(|(_, lien)| (lien.amount, lien.slashable_collateral())))
            // (max_amount, total_slashable) per user
            .try_fold(
                (Uint128::zero(), Uint128::zero()),
                |(max_amount, total_slashable), lien| -> StdResult<_> {
                    let (amount, slashable) = lien?;
                    let max_amount = max_amount.max(amount);
                    let total_slashable = total_slashable + slashable;
                    Ok((max_amount, total_slashable))
                },
            )?;

        Ok(collateral - max_lien.max(total_slashable))
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
        amount: Coin,
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

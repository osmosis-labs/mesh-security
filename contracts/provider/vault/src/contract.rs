use cosmwasm_std::{
    coin, ensure, Addr, BankMsg, Binary, Coin, Decimal, DepsMut, Order, Reply, Response, StdResult,
    Storage, SubMsg, SubMsgResponse, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use cw_utils::{must_pay, nonpayable, parse_instantiate_response_data};

use mesh_apis::cross_staking_api::CrossStakingApiHelper;
use mesh_apis::local_staking_api::{
    LocalStakingApiHelper, LocalStakingApiQueryMsg, MaxSlashResponse,
};
use mesh_apis::vault_api::{self, VaultApi};
use mesh_sync::Lockable;
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::msg::{
    AccountClaimsResponse, AccountResponse, AllAccountsResponse, AllAccountsResponseItem,
    AllTxsResponse, AllTxsResponseItem, ConfigResponse, LienInfo, StakingInitInfo,
};
use crate::state::{Config, Lien, LocalStaking, UserInfo};
use crate::txs::{Tx, TxType, Txs};

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub const REPLY_ID_INSTANTIATE: u64 = 1;

pub const DEFAULT_PAGE_LIMIT: u32 = 10;
pub const MAX_PAGE_LIMIT: u32 = 30;

/// Aligns pagination limit
fn clamp_page_limit(limit: Option<u32>) -> usize {
    limit.unwrap_or(DEFAULT_PAGE_LIMIT).max(MAX_PAGE_LIMIT) as usize
}

/// Default falseness for serde
fn def_false() -> bool {
    false
}

pub struct VaultContract<'a> {
    /// General contract configuration
    pub config: Item<'a, Config>,
    /// Local staking info
    pub local_staking: Item<'a, LocalStaking>,
    /// All liens in the protocol
    ///
    /// Liens are indexed with (user, creditor), as this pair has to be unique
    pub liens: Map<'a, (&'a Addr, &'a Addr), Lockable<Lien>>,
    /// Per-user information
    pub users: Map<'a, &'a Addr, Lockable<UserInfo>>,
    /// Pending txs information
    pub tx_count: Item<'a, u64>,
    pub pending: Txs<'a>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[error(ContractError)]
#[messages(vault_api as VaultApi)]
impl VaultContract<'_> {
    pub fn new() -> Self {
        Self {
            config: Item::new("config"),
            local_staking: Item::new("local_staking"),
            liens: Map::new("liens"),
            users: Map::new("users"),
            pending: Txs::new("pending_txs", "users"),
            tx_count: Item::new("tx_count"),
        }
    }

    pub fn next_tx_id(&self, store: &mut dyn Storage) -> StdResult<u64> {
        let id: u64 = self.tx_count.may_load(store)?.unwrap_or_default() + 1;
        self.tx_count.save(store, &id)?;
        Ok(id)
    }

    #[msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        denom: String,
        local_staking: StakingInitInfo,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let config = Config { denom };
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

        let mut user_lock = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();
        let user = user_lock.write()?;
        user.collateral += amount;
        self.users
            .save(ctx.deps.storage, &ctx.info.sender, &user_lock)?;

        let resp = Response::new()
            .add_attribute("action", "bond")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    #[msg(exec)]
    fn unbond(&self, ctx: ExecCtx, amount: Coin) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let denom = self.config.load(ctx.deps.storage)?.denom;

        ensure!(denom == amount.denom, ContractError::UnexpectedDenom(denom));

        let mut user_lock = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();
        let user = user_lock.read()?;

        let free_collateral = user.free_collateral();
        ensure!(
            user.free_collateral() >= amount.amount,
            ContractError::ClaimsLocked(free_collateral)
        );

        let user = user_lock.write()?;
        user.collateral -= amount.amount;
        self.users
            .save(ctx.deps.storage, &ctx.info.sender, &user_lock)?;

        let msg = BankMsg::Send {
            to_address: ctx.info.sender.to_string(),
            amount: vec![amount.clone()],
        };

        let resp = Response::new()
            .add_message(msg)
            .add_attribute("action", "unbond")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    /// This assigns a claim of amount tokens to the remote contract, which can take some action with it
    #[msg(exec)]
    fn stake_remote(
        &self,
        mut ctx: ExecCtx,
        // address of the contract to virtually stake on
        contract: String,
        // amount to stake on that contract
        amount: Coin,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let config = self.config.load(ctx.deps.storage)?;
        let contract = ctx.deps.api.addr_validate(&contract)?;
        let contract = CrossStakingApiHelper(contract);
        let slashable = contract.max_slash(ctx.deps.as_ref())?;

        let tx_id = self.stake(
            &mut ctx,
            &config,
            &contract.0,
            slashable.max_slash,
            amount.clone(),
            true,
        )?;

        let stake_msg = contract.receive_virtual_stake(
            ctx.info.sender.to_string(),
            amount.clone(),
            tx_id,
            msg,
            vec![],
        )?;

        let resp = Response::new()
            .add_message(stake_msg)
            .add_attribute("action", "stake_remote")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.amount.to_string())
            .add_attribute("tx_id", tx_id.to_string());

        Ok(resp)
    }

    /// This sends actual tokens to the local staking contract
    #[msg(exec)]
    fn stake_local(
        &self,
        mut ctx: ExecCtx,
        // amount to stake on that contract
        amount: Coin,
        // action to take with that stake
        msg: Binary,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let config = self.config.load(ctx.deps.storage)?;
        let local_staking = self.local_staking.load(ctx.deps.storage)?;

        self.stake(
            &mut ctx,
            &config,
            &local_staking.contract.0,
            local_staking.max_slash,
            amount.clone(),
            false,
        )?;

        let stake_msg = local_staking.contract.receive_stake(
            ctx.info.sender.to_string(),
            msg,
            vec![amount.clone()],
        )?;

        let resp = Response::new()
            .add_message(stake_msg)
            .add_attribute("action", "stake_local")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("amount", amount.amount.to_string());

        Ok(resp)
    }

    #[msg(query)]
    fn account(&self, ctx: QueryCtx, account: String) -> Result<AccountResponse, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let account = ctx.deps.api.addr_validate(&account)?;

        let user_lock = self
            .users
            .may_load(ctx.deps.storage, &account)?
            .unwrap_or_default();
        let user = user_lock.read()?;

        let resp = AccountResponse {
            denom,
            bonded: user.collateral,
            free: user.free_collateral(),
        };

        Ok(resp)
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        let local_staking = self.local_staking.load(ctx.deps.storage)?;

        let resp = ConfigResponse {
            denom: config.denom,
            local_staking: local_staking.contract.0.into(),
        };

        Ok(resp)
    }

    /// Returns a single claim between the user and lienholder
    #[msg(query)]
    fn claim(
        &self,
        ctx: QueryCtx,
        account: String,
        lienholder: String,
    ) -> Result<Lien, ContractError> {
        let account = ctx.deps.api.addr_validate(&account)?;
        let lienholder = ctx.deps.api.addr_validate(&lienholder)?;

        self.liens
            .load(ctx.deps.storage, (&account, &lienholder))?
            .read()
            .map_err(Into::into)
            .cloned()
    }

    /// Returns paginated claims list for an user
    ///
    /// `start_after` is a last lienholder of the previous page, and it will not be included
    #[msg(query)]
    fn account_claims(
        &self,
        ctx: QueryCtx,
        account: String,
        start_after: Option<String>,
        limit: Option<u32>,
    ) -> Result<AccountClaimsResponse, ContractError> {
        let limit = clamp_page_limit(limit);
        let start_after = start_after.map(Addr::unchecked);
        let bound = start_after.as_ref().and_then(Bounder::exclusive_bound);

        let account = Addr::unchecked(account);
        let claims = self
            .liens
            .prefix(&account)
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .map(|item| {
                let (lienholder, lien) = item?;
                let lien = lien.read()?;
                Ok::<LienInfo, ContractError>(LienInfo {
                    lienholder: lienholder.into(),
                    amount: lien.amount,
                })
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = AccountClaimsResponse { claims };

        Ok(resp)
    }

    /// Queries for all users ever performing action in the system, paginating over
    /// them.
    ///
    /// `start_after` is the last account included in previous page
    ///
    /// `with_collateral` flag filters out users with no collateral, defaulted to false
    #[msg(query)]
    fn all_accounts(
        &self,
        ctx: QueryCtx,
        #[serde(default = "def_false")] with_collateral: bool,
        start_after: Option<String>,
        limit: Option<u32>,
    ) -> Result<AllAccountsResponse, ContractError> {
        let limit = clamp_page_limit(limit);
        let start_after = start_after.map(Addr::unchecked);
        let bound = start_after.as_ref().and_then(Bounder::exclusive_bound);

        let denom = self.config.load(ctx.deps.storage)?.denom;

        let accounts = self
            .users
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .filter(|account| {
                !(with_collateral
                    && account
                        .as_ref()
                        .map(|(_, account_lock)| {
                            // FIXME: This skips write-locked accounts
                            match account_lock.read() {
                                Ok(account) => account.collateral.is_zero(),
                                Err(_) => true,
                            }
                        })
                        .unwrap_or(false))
            })
            .map(|account| {
                let (addr, account_lock) = account?;
                let account = account_lock.read()?;
                Ok::<AllAccountsResponseItem, ContractError>(AllAccountsResponseItem {
                    account: addr.into(),
                    denom: denom.clone(),
                    bonded: account.collateral,
                    free: account.free_collateral(),
                })
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = AllAccountsResponse { accounts };

        Ok(resp)
    }

    /// Queries for all pending txs.
    /// Reports txs in descending order (newest first).
    /// `start_after` is the last tx id included in previous page
    #[msg(query)]
    fn all_pending_txs(
        &self,
        ctx: QueryCtx,
        start_after: Option<u64>,
        limit: Option<u32>,
    ) -> Result<AllTxsResponse, ContractError> {
        let limit = clamp_page_limit(limit);
        let bound = start_after.and_then(Bounder::exclusive_bound);

        let txs = self
            .pending
            .txs
            .range(ctx.deps.storage, None, bound, Order::Descending)
            .map(|item| {
                let (_id, tx) = item?;
                Ok::<AllTxsResponseItem, ContractError>(tx)
            })
            .take(limit)
            .collect::<Result<_, _>>()?;

        let resp = AllTxsResponse { txs };

        Ok(resp)
    }

    #[msg(reply)]
    fn reply(&self, ctx: ReplyCtx, reply: Reply) -> Result<Response, ContractError> {
        match reply.id {
            REPLY_ID_INSTANTIATE => self.reply_init_callback(ctx.deps, reply.result.unwrap()),
            _ => Err(ContractError::InvalidReplyId(reply.id)),
        }
    }

    fn reply_init_callback(
        &self,
        deps: DepsMut,
        reply: SubMsgResponse,
    ) -> Result<Response, ContractError> {
        let init_data = parse_instantiate_response_data(&reply.data.unwrap())?;
        let local_staking = Addr::unchecked(init_data.contract_address);

        // As we control the local staking contract it might be better to just raw-query it
        // on demand instead of duplicating the data.
        let query = LocalStakingApiQueryMsg::MaxSlash {};
        let MaxSlashResponse { max_slash } =
            deps.querier.query_wasm_smart(&local_staking, &query)?;

        let local_staking = LocalStaking {
            contract: LocalStakingApiHelper(local_staking),
            max_slash,
        };

        self.local_staking.save(deps.storage, &local_staking)?;

        Ok(Response::new())
    }

    /// Updates the local stake for staking on any contract
    ///
    /// Stake (both local and remote) is always called by the tokens owner, so the `sender` is
    /// ued as an owner address.
    ///
    /// Config is taken in argument as it sometimes is used outside of this function, so
    /// we want to avoid double-fetching it
    fn stake(
        &self,
        ctx: &mut ExecCtx,
        config: &Config,
        lienholder: &Addr,
        slashable: Decimal,
        amount: Coin,
        remote: bool,
    ) -> Result<u64, ContractError> {
        ensure!(
            amount.denom == config.denom,
            ContractError::UnexpectedDenom(config.denom.clone())
        );

        let amount = amount.amount;
        let mut lien_lock = self
            .liens
            .may_load(ctx.deps.storage, (&ctx.info.sender, lienholder))?
            .unwrap_or_else(|| {
                Lockable::new(Lien {
                    amount: Uint128::zero(),
                    slashable,
                })
            });
        let lien = lien_lock.write()?;
        lien.amount += amount;

        let mut user_lock = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();
        let mut user = user_lock.write()?;
        user.max_lien = user.max_lien.max(lien.amount);
        user.total_slashable += amount * lien.slashable;

        ensure!(user.verify_collateral(), ContractError::InsufficentBalance);

        // Write lock it if remote stake
        if remote {
            lien_lock.lock_write()?;
            user_lock.lock_write()?;
        }
        self.liens
            .save(ctx.deps.storage, (&ctx.info.sender, lienholder), &lien_lock)?;

        self.users
            .save(ctx.deps.storage, &ctx.info.sender, &user_lock)?;

        if remote {
            // Create new tx
            let tx_id = self.next_tx_id(ctx.deps.storage)?;

            let new_tx = Tx {
                id: tx_id,
                ty: TxType::InFlightStaking,
                amount,
                slashable,
                user: ctx.info.sender.clone(),
                lienholder: lienholder.clone(),
            };
            self.pending.txs.save(ctx.deps.storage, tx_id, &new_tx)?;
            Ok(tx_id)
        } else {
            Ok(0)
        }
    }

    /// Commits a pending stake
    fn commit_stake(&self, ctx: &mut ExecCtx, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending.txs.load(ctx.deps.storage, tx_id)?;

        // Verify tx comes from the right contract
        ensure!(
            tx.lienholder == ctx.info.sender,
            ContractError::WrongContractTx(tx_id, ctx.info.sender.clone())
        );

        // Load lien
        let mut lien_lock = self
            .liens
            .load(ctx.deps.storage, (&tx.user, &tx.lienholder))?;
        // Unlock it
        lien_lock.unlock_write()?;
        // Save it
        self.liens
            .save(ctx.deps.storage, (&tx.user, &tx.lienholder), &lien_lock)?;
        // Load user
        let mut user_lock = self.users.load(ctx.deps.storage, &tx.user)?;
        // Unlock it
        user_lock.unlock_write()?;
        // Save it
        self.users.save(ctx.deps.storage, &tx.user, &user_lock)?;

        // Remove tx
        self.pending.txs.remove(ctx.deps.storage, tx_id)?;

        Ok(())
    }

    /// Rollbacks a pending tx
    fn rollback_stake(&self, ctx: &mut ExecCtx, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending.txs.load(ctx.deps.storage, tx_id)?;
        // Verify tx comes from the right contract
        ensure!(
            tx.lienholder == ctx.info.sender,
            ContractError::WrongContractTx(tx_id, ctx.info.sender.clone())
        );

        // Load lien
        let mut lien_lock = self
            .liens
            .load(ctx.deps.storage, (&tx.user, &tx.lienholder))?;
        // Rollback amount (need to unlock it first)
        lien_lock.unlock_write()?;
        let lien = lien_lock.write()?;
        lien.amount -= tx.amount;
        // Save it unlocked
        self.liens
            .save(ctx.deps.storage, (&tx.user, &tx.lienholder), &lien_lock)?;

        // Load user
        let mut user_lock = self.users.load(ctx.deps.storage, &tx.user)?;
        // Rollback user's max_lien (need to unlock it first)
        lien_lock.unlock_write()?;
        let mut user = user_lock.write()?;

        // Max lien has to be recalculated from scratch; the just rolled back lien
        // is already written to storage
        user.max_lien = self
            .liens
            .prefix(&tx.user)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .try_fold(Uint128::zero(), |max_lien, item| {
                let (_, lien_lock) = item?;
                // Shouldn't fail, because unlocked already
                let lien = lien_lock.read().map_err(Into::<ContractError>::into);
                lien.map(|lien| max_lien.max(lien.amount))
            })?;

        user.total_slashable -= tx.amount * tx.slashable;
        self.users.save(ctx.deps.storage, &tx.user, &user_lock)?;

        // Remove tx
        self.pending.txs.remove(ctx.deps.storage, tx_id)?;
        Ok(())
    }

    /// Updates the local stake for unstaking from any contract
    ///
    /// The unstake (both local and remote) is always called by the staking contract
    /// (aka lienholder), so the `sender` address is used for that.
    fn unstake(&self, ctx: &mut ExecCtx, owner: String, amount: Coin) -> Result<(), ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        ensure!(amount.denom == denom, ContractError::UnexpectedDenom(denom));
        let amount = amount.amount;

        let owner = Addr::unchecked(owner);
        let mut lien_lock = self
            .liens
            .may_load(ctx.deps.storage, (&owner, &ctx.info.sender))?
            .ok_or(ContractError::UnknownLienholder)?;
        let lien = lien_lock.read()?;

        ensure!(lien.amount >= amount, ContractError::InsufficientLien);
        let slashable = lien.slashable;
        let lien = lien_lock.write()?;
        lien.amount -= amount;

        self.liens
            .save(ctx.deps.storage, (&owner, &ctx.info.sender), &lien_lock)?;

        let mut user_lock = self.users.load(ctx.deps.storage, &owner)?;
        let mut user = user_lock.write()?;

        // Max lien has to be recalculated from scratch; the just saved lien
        // is already written to storage
        user.max_lien = self
            .liens
            .prefix(&owner)
            .range(ctx.deps.storage, None, None, Order::Ascending)
            .try_fold(Uint128::zero(), |max_lien, item| {
                let (_, lien_lock) = item?;
                let lien = lien_lock.read().map_err(Into::<ContractError>::into);
                lien.map(|lien| max_lien.max(lien.amount))
            })?;

        user.total_slashable -= amount * slashable;
        self.users.save(ctx.deps.storage, &owner, &user_lock)?;

        Ok(())
    }
}

impl Default for VaultContract<'_> {
    fn default() -> Self {
        Self::new()
    }
}

#[contract]
#[messages(vault_api as VaultApi)]
impl VaultApi for VaultContract<'_> {
    type Error = ContractError;

    /// This must be called by the remote staking contract to release this claim
    #[msg(exec)]
    fn release_cross_stake(
        &self,
        mut ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
        // amount to unstake on that contract
        amount: Coin,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        self.unstake(&mut ctx, owner.clone(), amount.clone())?;

        let resp = Response::new()
            .add_attribute("action", "release_cross_stake")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("owner", owner)
            .add_attribute("amount", amount.amount.to_string());

        Ok(resp)
    }

    /// This must be called by the local staking contract to release this claim
    /// Amount of tokens unstaked are those included in ctx.info.funds
    #[msg(exec)]
    fn release_local_stake(
        &self,
        mut ctx: ExecCtx,
        // address of the user who originally called stake_remote
        owner: String,
    ) -> Result<Response, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let amount = must_pay(&ctx.info, &denom)?;

        self.unstake(&mut ctx, owner.clone(), coin(amount.u128(), denom))?;

        let resp = Response::new()
            .add_attribute("action", "release_cross_stake")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("owner", owner)
            .add_attribute("amount", amount.to_string());

        Ok(resp)
    }

    #[msg(exec)]
    fn commit_tx(&self, mut ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        self.commit_stake(&mut ctx, tx_id)?;

        let resp = Response::new()
            .add_attribute("action", "commit_tx")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("tx_id", tx_id.to_string());

        Ok(resp)
    }

    #[msg(exec)]
    fn rollback_tx(&self, mut ctx: ExecCtx, tx_id: u64) -> Result<Response, ContractError> {
        self.rollback_stake(&mut ctx, tx_id)?;

        let resp = Response::new()
            .add_attribute("action", "rollback_tx")
            .add_attribute("sender", ctx.info.sender)
            .add_attribute("tx_id", tx_id.to_string());
        Ok(resp)
    }
}

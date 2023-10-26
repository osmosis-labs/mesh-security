use cosmwasm_std::{
    coin, ensure, Addr, BankMsg, Binary, Coin, Decimal, DepsMut, Fraction, Order, Reply, Response,
    StdResult, Storage, SubMsg, SubMsgResponse, Uint128, WasmMsg,
};
use cw2::set_contract_version;
use cw_storage_plus::{Bounder, Item, Map};
use cw_utils::{must_pay, nonpayable, parse_instantiate_response_data};
use std::cmp::min;

use mesh_apis::cross_staking_api::CrossStakingApiHelper;
use mesh_apis::local_staking_api::{
    LocalStakingApiHelper, LocalStakingApiQueryMsg, MaxSlashResponse,
};
use mesh_apis::vault_api::{self, SlashInfo, VaultApi};
use mesh_sync::Tx::InFlightStaking;
use mesh_sync::{max_range, ValueRange};
use sylvia::types::{ExecCtx, InstantiateCtx, QueryCtx, ReplyCtx};
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::msg::{
    AccountClaimsResponse, AccountDetailsResponse, AccountResponse, AllAccountsResponse,
    AllAccountsResponseItem, AllTxsResponse, AllTxsResponseItem, ConfigResponse, LienResponse,
    StakingInitInfo, TxResponse,
};
use crate::state::{Config, Lien, LocalStaking, UserInfo};
use crate::txs::Txs;

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
    pub local_staking: Item<'a, Option<LocalStaking>>,
    /// All liens in the protocol
    ///
    /// Liens are indexed with (user, lien_holder), as this pair has to be unique
    pub liens: Map<'a, (&'a Addr, &'a Addr), Lien>,
    /// Per-user information
    pub users: Map<'a, &'a Addr, UserInfo>,
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
        local_staking: Option<StakingInitInfo>,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let config = Config { denom };
        self.config.save(ctx.deps.storage, &config)?;
        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        if let Some(local_staking) = local_staking {
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
        } else {
            self.local_staking.save(ctx.deps.storage, &None)?;
            Ok(Response::new())
        }
    }

    #[msg(exec)]
    fn bond(&self, ctx: ExecCtx) -> Result<Response, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let amount = must_pay(&ctx.info, &denom)?;

        let mut user = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();
        user.collateral += amount;
        self.users.save(ctx.deps.storage, &ctx.info.sender, &user)?;

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

        let mut user = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();

        let free_collateral = user.free_collateral();
        ensure!(
            free_collateral.low() >= amount.amount,
            ContractError::ClaimsLocked(free_collateral)
        );

        user.collateral -= amount.amount;
        self.users.save(ctx.deps.storage, &ctx.info.sender, &user)?;

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
        if let Some(local_staking) = self.local_staking.load(ctx.deps.storage)? {
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
        } else {
            Err(ContractError::NoLocalStaking)
        }
    }

    #[msg(query)]
    fn account(&self, ctx: QueryCtx, account: String) -> Result<AccountResponse, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let account = ctx.deps.api.addr_validate(&account)?;

        let user = self
            .users
            .may_load(ctx.deps.storage, &account)?
            .unwrap_or_default();
        Ok(AccountResponse {
            denom,
            bonded: user.collateral,
            free: user.free_collateral(),
        })
    }

    #[msg(query)]
    fn account_details(
        &self,
        ctx: QueryCtx,
        account: String,
    ) -> Result<AccountDetailsResponse, ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        let account = ctx.deps.api.addr_validate(&account)?;

        let user = self
            .users
            .may_load(ctx.deps.storage, &account)?
            .unwrap_or_default();
        Ok(AccountDetailsResponse {
            denom,
            bonded: user.collateral,
            free: user.free_collateral(),
            max_lien: user.max_lien,
            total_slashable: user.total_slashable,
        })
    }

    #[msg(query)]
    fn config(&self, ctx: QueryCtx) -> Result<ConfigResponse, ContractError> {
        let config = self.config.load(ctx.deps.storage)?;
        let local_staking = self.local_staking.load(ctx.deps.storage)?;

        let resp = ConfigResponse {
            denom: config.denom,
            local_staking: local_staking.map(|ls| ls.contract.0.into()),
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

        Ok(self.liens.load(ctx.deps.storage, (&account, &lienholder))?)
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
                Ok::<_, ContractError>(LienResponse {
                    lienholder: lienholder.to_string(),
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

        let accounts: Vec<_> = self
            .users
            .range(ctx.deps.storage, bound, None, Order::Ascending)
            .filter(|account| {
                account
                    .as_ref()
                    .map(|(_, account)| {
                        !with_collateral || !account.collateral.is_zero() // Skip zero collateral
                    })
                    .unwrap_or(false) // Skip other errors
            })
            .map(|account| {
                account.map(|(addr, account)| AllAccountsResponseItem {
                    user: addr.to_string(),
                    account: AccountResponse {
                        denom: denom.clone(),
                        bonded: account.collateral,
                        free: account.free_collateral(),
                    },
                })
            })
            .take(limit)
            .collect::<StdResult<_>>()?;

        let resp = AllAccountsResponse { accounts };

        Ok(resp)
    }

    /// Queries a pending tx.
    #[msg(query)]
    fn pending_tx(&self, ctx: QueryCtx, tx_id: u64) -> Result<TxResponse, ContractError> {
        let resp = self.pending.txs.load(ctx.deps.storage, tx_id)?;
        Ok(resp)
    }

    /// Queries for all pending txs.
    /// Reports txs in descending order (newest first).
    /// `start_after` is the last tx id included in previous page
    #[msg(query)]
    fn all_pending_txs_desc(
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

        self.local_staking
            .save(deps.storage, &Some(local_staking))?;

        Ok(Response::new())
    }

    /// Updates the local stake for staking on any contract
    ///
    /// Stake (both local and remote) is always called by the tokens owner, so the `sender` is
    /// ued as an owner address.
    ///
    /// Config is taken in argument as it sometimes is used outside of this function, so
    /// we want to avoid double-fetching it
    ///
    /// Remote indicates if the stake is remote or local. Remote staking involves transaction
    /// processing.
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
        let mut lien = self
            .liens
            .may_load(ctx.deps.storage, (&ctx.info.sender, lienholder))?
            .unwrap_or_else(|| Lien {
                amount: ValueRange::new_val(Uint128::zero()),
                slashable,
            });
        let mut user = self
            .users
            .may_load(ctx.deps.storage, &ctx.info.sender)?
            .unwrap_or_default();
        if remote {
            lien.amount
                .prepare_add(amount, user.collateral)
                .map_err(|_| ContractError::InsufficentBalance)?;
            // Tentative value
            user.max_lien = max_range(user.max_lien, lien.amount);
            user.total_slashable
                .prepare_add(amount * lien.slashable, user.collateral)
                .map_err(|_| ContractError::InsufficentBalance)?;
        } else {
            // Update lien immediately
            lien.amount
                .add(amount, user.collateral)
                .map_err(|_| ContractError::InsufficentBalance)?;
            // Update max lien and total slashable immediately
            user.max_lien = max_range(user.max_lien, lien.amount);
            user.total_slashable
                .add(amount * lien.slashable, user.collateral)
                .map_err(|_| ContractError::InsufficentBalance)?;
        }

        ensure!(user.verify_collateral(), ContractError::InsufficentBalance);

        self.liens
            .save(ctx.deps.storage, (&ctx.info.sender, lienholder), &lien)?;
        self.users.save(ctx.deps.storage, &ctx.info.sender, &user)?;
        let tx_id = if remote {
            // Create new tx
            let tx_id = self.next_tx_id(ctx.deps.storage)?;

            let new_tx = InFlightStaking {
                id: tx_id,
                amount,
                slashable,
                user: ctx.info.sender.clone(),
                lienholder: lienholder.clone(),
            };
            self.pending.txs.save(ctx.deps.storage, tx_id, &new_tx)?;
            tx_id
        } else {
            0
        };
        Ok(tx_id)
    }

    /// Commits a pending stake
    fn commit_stake(&self, ctx: &mut ExecCtx, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending.txs.load(ctx.deps.storage, tx_id)?;

        // Verify tx comes from the right contract, and is of the right type
        ensure!(
            match tx.clone() {
                InFlightStaking { lienholder, .. } => {
                    ensure!(
                        lienholder == ctx.info.sender,
                        ContractError::WrongContractTx(tx_id, ctx.info.sender.clone())
                    );
                    true
                }
                _ => false,
            },
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (tx_amount, tx_user, tx_lienholder) = match tx {
            InFlightStaking {
                amount,
                user,
                lienholder,
                ..
            } => (amount, user, lienholder),
            _ => unreachable!(),
        };

        // Load lien
        let mut lien = self
            .liens
            .load(ctx.deps.storage, (&tx_user, &tx_lienholder))?;
        // Commit it
        lien.amount.commit_add(tx_amount);
        // Save it
        self.liens
            .save(ctx.deps.storage, (&tx_user, &tx_lienholder), &lien)?;
        // Load user
        let mut user = self.users.load(ctx.deps.storage, &tx_user)?;
        // Update max lien definitive value (it depends on the lien's value range)
        user.max_lien = max_range(user.max_lien, lien.amount);
        // Commit total slashable
        user.total_slashable.commit_add(tx_amount * lien.slashable);
        // Save it
        self.users.save(ctx.deps.storage, &tx_user, &user)?;

        // Remove tx
        self.pending.txs.remove(ctx.deps.storage, tx_id)?;

        Ok(())
    }

    /// Rollbacks a pending tx
    fn rollback_stake(&self, ctx: &mut ExecCtx, tx_id: u64) -> Result<(), ContractError> {
        // Load tx
        let tx = self.pending.txs.load(ctx.deps.storage, tx_id)?;

        // Verify tx comes from the right contract, and is of the right type
        ensure!(
            match tx.clone() {
                InFlightStaking { lienholder, .. } => {
                    ensure!(
                        lienholder == ctx.info.sender,
                        ContractError::WrongContractTx(tx_id, ctx.info.sender.clone())
                    );
                    true
                }
                _ => false,
            },
            ContractError::WrongTypeTx(tx_id, tx)
        );

        let (tx_amount, tx_slashable, tx_user, tx_lienholder) = match tx {
            InFlightStaking {
                amount,
                slashable,
                user,
                lienholder,
                ..
            } => (amount, slashable, user, lienholder),
            _ => unreachable!(),
        };

        // Load lien
        let mut lien = self
            .liens
            .load(ctx.deps.storage, (&tx_user, &tx_lienholder))?;
        // Rollback amount
        lien.amount.rollback_add(tx_amount);
        // Save it
        self.liens
            .save(ctx.deps.storage, (&tx_user, &tx_lienholder), &lien)?;

        // Load user
        let mut user = self.users.load(ctx.deps.storage, &tx_user)?;
        // Rollback user's max_lien

        // Max lien has to be recalculated from scratch; the just rolled back lien
        // is already written to storage
        self.recalculate_max_lien(ctx.deps.storage, &tx_user, &mut user)?;

        user.total_slashable.rollback_add(tx_amount * tx_slashable);
        self.users.save(ctx.deps.storage, &tx_user, &user)?;

        // Remove tx
        self.pending.txs.remove(ctx.deps.storage, tx_id)?;
        Ok(())
    }

    /// Recalculates the max lien for the user
    fn recalculate_max_lien(
        &self,
        storage: &mut dyn Storage,
        user: &Addr,
        user_info: &mut UserInfo,
    ) -> Result<(), ContractError> {
        user_info.max_lien = self
            .liens
            .prefix(user)
            .range(storage, None, None, Order::Ascending)
            .try_fold(ValueRange::new_val(Uint128::zero()), |max_lien, item| {
                let (_, lien) = item?;
                Ok::<_, ContractError>(max_range(max_lien, lien.amount))
            })?;
        Ok(())
    }

    /// Updates the local stake for unstaking from any contract
    ///
    /// The unstake (both local and remote) is always called by the staking contract
    /// (aka lien_holder), so the `sender` address is used for that.
    fn unstake(&self, ctx: &mut ExecCtx, owner: String, amount: Coin) -> Result<(), ContractError> {
        let denom = self.config.load(ctx.deps.storage)?.denom;
        ensure!(amount.denom == denom, ContractError::UnexpectedDenom(denom));
        let amount = amount.amount;

        let owner = Addr::unchecked(owner);
        let mut lien = self
            .liens
            .may_load(ctx.deps.storage, (&owner, &ctx.info.sender))?
            .ok_or(ContractError::UnknownLienholder)?;

        let slashable = lien.slashable;
        lien.amount
            .sub(amount, Uint128::zero())
            .map_err(|_| ContractError::InsufficientLien)?;

        self.liens
            .save(ctx.deps.storage, (&owner, &ctx.info.sender), &lien)?;

        let mut user = self.users.load(ctx.deps.storage, &owner)?;

        // Max lien has to be recalculated from scratch; the just saved lien
        // is already written to storage
        self.recalculate_max_lien(ctx.deps.storage, &owner, &mut user)?;

        user.total_slashable
            .sub(amount * slashable, Uint128::zero())?;
        self.users.save(ctx.deps.storage, &owner, &user)?;

        Ok(())
    }

    /// Processes a (remote or local) slashing event.
    ///
    /// This slashes the users that have funds delegated to the validator involved in the
    /// misbehaviour.
    ///
    /// In case of remote slashing, it makes sure to unbond native user funds from the native
    /// staking contract, if they are needed for slashing.
    ///
    /// It also checks that the mesh security invariants are not violated after slashing,
    /// i.e. performs slashing propagation across lien holders, for all of the slashed users.
    fn slash(
        &self,
        ctx: &mut ExecCtx,
        slashes: &[SlashInfo],
    ) -> Result<Vec<WasmMsg>, ContractError> {
        // Process users that belong to lien_holder
        let lien_holder = ctx.info.sender.clone();
        let mut msgs = vec![];
        for slash in slashes {
            let slash_user = Addr::unchecked(slash.user.clone());
            // User must have a lien with this lien holder
            let mut lien = self
                .liens
                .load(ctx.deps.storage, (&slash_user, &lien_holder))?;
            let slash_amount = slash.slash;
            let mut user_info = self.users.load(ctx.deps.storage, &slash_user)?;
            let new_collateral = user_info.collateral - slash_amount;

            // Slash user
            lien.amount.sub(slash_amount, Uint128::zero())?;
            // Save lien
            self.liens
                .save(ctx.deps.storage, (&slash_user, &lien_holder), &lien)?;
            // Adjust total slashable and max lien
            user_info
                .total_slashable
                .sub(slash_amount * lien.slashable, Uint128::zero())?;
            self.recalculate_max_lien(ctx.deps.storage, &slash_user, &mut user_info)?;
            // Get free collateral before adjusting collateral, but after slashing
            let free_collateral = user_info.free_collateral().low(); // For simplicity
            if free_collateral < slash_amount {
                // Check / adjust mesh security invariants according to the new collateral
                let burn_msgs = self.propagate_slash(
                    ctx.deps.storage,
                    &slash_user,
                    &mut user_info,
                    new_collateral,
                    slash_amount - free_collateral,
                )?;
                msgs.extend_from_slice(&burn_msgs);
            }
            // Adjust collateral
            user_info.collateral = new_collateral;
            // Recompute max lien
            self.recalculate_max_lien(ctx.deps.storage, &slash_user, &mut user_info)?;
            // Save user info
            self.users.save(ctx.deps.storage, &slash_user, &user_info)?;
        }
        Ok(msgs)
    }

    fn propagate_slash(
        &self,
        storage: &mut dyn Storage,
        user: &Addr,
        user_info: &mut UserInfo,
        new_collateral: Uint128,
        claimed_collateral: Uint128,
    ) -> Result<Vec<WasmMsg>, ContractError> {
        let denom = self.config.load(storage)?.denom;
        let native_staking = self.local_staking.load(storage)?;
        let mut msgs = vec![];
        if user_info.max_lien.high() >= user_info.total_slashable.high() {
            // Liens adjustment
            let broken_liens = self
                .liens
                .prefix(user)
                .range(storage, None, None, Order::Ascending)
                .filter(|item| {
                    item.as_ref()
                        .map(|(_, lien)| lien.amount.high() > new_collateral) // Skip in range liens
                        .unwrap_or(false) // Skip other errors
                })
                .collect::<StdResult<Vec<_>>>()?;
            for (lien_holder, mut lien) in broken_liens {
                let new_low_amount = min(lien.amount.low(), new_collateral);
                let new_high_amount = min(lien.amount.high(), new_collateral);
                // Adjust the user's total slashable amount
                let adjust_amount_low = lien.amount.low() - new_low_amount;
                let adjust_amount_high = lien.amount.high() - new_high_amount;
                user_info.total_slashable = ValueRange::new(
                    user_info.total_slashable.low() - adjust_amount_low * lien.slashable,
                    user_info.total_slashable.high() - adjust_amount_high * lien.slashable,
                );
                // Keep the invariant over the lien
                lien.amount = ValueRange::new(new_low_amount, new_high_amount);
                self.liens.save(storage, (user, &lien_holder), &lien)?;
                // Remove the required amount from the user's stake
                let burn_msg = self.burn_stake(
                    user,
                    &denom,
                    &native_staking,
                    &lien_holder,
                    adjust_amount_high,
                )?; // High amount For simplicity
                msgs.push(burn_msg);
            }
        } else {
            // Total slashable adjustment
            let slash_ratio_sum = self
                .liens
                .prefix(user)
                .range(storage, None, None, Order::Ascending)
                .try_fold(Decimal::zero(), |sum, item| {
                    let (_, lien) = item?;
                    Ok::<_, ContractError>(sum + lien.slashable)
                })?;
            let round_up = if (claimed_collateral * slash_ratio_sum.inv().unwrap())
                * slash_ratio_sum
                != claimed_collateral
            {
                Uint128::one()
            } else {
                Uint128::zero()
            };
            let sub_amount = claimed_collateral * slash_ratio_sum.inv().unwrap() + round_up;
            let all_liens = self
                .liens
                .prefix(user)
                .range(storage, None, None, Order::Ascending)
                .collect::<StdResult<Vec<_>>>()?;
            for (lien_holder, mut lien) in all_liens {
                // Adjust the user's total slashable amount
                user_info
                    .total_slashable
                    .sub(sub_amount * lien.slashable, Uint128::zero())?;
                // Keep the invariant over the lien
                lien.amount.sub(sub_amount, Uint128::zero())?;
                self.liens.save(storage, (user, &lien_holder), &lien)?;
                // Remove the required amount from the user's stake
                let burn_msg =
                    self.burn_stake(user, &denom, &native_staking, &lien_holder, sub_amount)?;
                msgs.push(burn_msg);
            }
        }
        Ok(msgs)
    }

    fn burn_stake(
        &self,
        user: &Addr,
        denom: &String,
        native_staking: &Option<LocalStaking>,
        lien_holder: &Addr,
        amount: Uint128,
    ) -> Result<WasmMsg, ContractError> {
        // Native vs cross staking
        let msg = match &native_staking {
            Some(local_staking) if local_staking.contract.0 == lien_holder => {
                let contract = local_staking.contract.clone();
                contract.burn_stake(user, coin(amount.u128(), denom), None)? // TODO: Add validator when burning over the originally slashed lien holder
            }
            _ => {
                let contract = CrossStakingApiHelper(lien_holder.clone());
                contract.burn_virtual_stake(user, coin(amount.u128(), denom), None)?
                // TODO: Add validator when burning over the originally slashed lien holder
            }
        };
        Ok(msg)
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

    /// This must be called by the external staking contract to process a misbehaviour
    #[msg(exec)]
    fn cross_slash(
        &self,
        mut ctx: ExecCtx,
        slashes: Vec<SlashInfo>,
    ) -> Result<Response, Self::Error> {
        nonpayable(&ctx.info)?;

        let msgs = self.slash(&mut ctx, &slashes)?;

        let resp = Response::new()
            .add_messages(msgs)
            .add_attribute("action", "process_cross_slashing")
            .add_attribute("lien_holder", ctx.info.sender)
            .add_attribute(
                "users",
                slashes
                    .iter()
                    .map(|s| s.user.clone())
                    .collect::<Vec<_>>()
                    .join(", "),
            );

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

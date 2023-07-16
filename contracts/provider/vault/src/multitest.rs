mod cross_staking;
mod local_staking;

use crate::contract;
use crate::contract::multitest_utils::VaultContractProxy;
use crate::contract::test_utils::VaultApi;
use crate::error::ContractError;
use crate::msg::{
    AccountResponse, AllAccountsResponseItem, LienResponse, MaybeAccountResponse,
    MaybeLienResponse, StakingInitInfo,
};
use cosmwasm_std::StdError::GenericErr;
use cosmwasm_std::{coin, coins, to_binary, Addr, Binary, Decimal, Empty, Uint128};
use cw_multi_test::App as MtApp;
use mesh_sync::Tx::InFlightStaking;
use mesh_sync::{LockError, Tx};
use sylvia::multitest::App;

const OSMO: &str = "OSMO";

#[test]
fn instantiation() {
    let app = App::default();

    let owner = "onwer";

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    let config = vault.config().unwrap();
    assert_eq!(config.denom, OSMO);

    let users = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(users.accounts, []);
}

#[test]
fn bonding() {
    let owner = "owner";
    let user = "user1";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::zero(),
            free: Uint128::zero(),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);

    // Bond some tokens
    vault
        .bond()
        .with_funds(&coins(100, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(100),
            free: Uint128::new(100),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app().wrap().query_balance(user, OSMO).unwrap(),
        coin(200, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(100, OSMO)
    );

    vault
        .bond()
        .with_funds(&coins(150, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(250),
            free: Uint128::new(250),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app().wrap().query_balance(user, OSMO).unwrap(),
        coin(50, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(250, OSMO)
    );

    // Unbond some tokens

    vault.unbond(coin(200, OSMO)).call(user).unwrap();
    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(50),
            free: Uint128::new(50),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app().wrap().query_balance(user, OSMO).unwrap(),
        coin(250, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(50, OSMO)
    );

    vault.unbond(coin(20, OSMO)).call(user).unwrap();
    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(30),
            free: Uint128::new(30),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app().wrap().query_balance(user, OSMO).unwrap(),
        coin(270, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(30, OSMO)
    );

    // Unbounding over bounded fails

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(err, ContractError::ClaimsLocked(Uint128::new(30)));
}

#[test]
fn stake_local() {
    let owner = "owner";
    let user = "user1";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    let local_staking = Addr::unchecked(vault.config().unwrap().local_staking);
    let local_staking = local_staking::multitest_utils::LocalStakingProxy::new(local_staking, &app);

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Staking locally

    vault
        .stake_local(coin(100, OSMO), Binary::default())
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(200, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(100, OSMO)
    );

    vault
        .stake_local(coin(150, OSMO), Binary::default())
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(50),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: Uint128::new(250)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(50, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(250, OSMO)
    );

    // Cannot stake over collateral

    let err = vault
        .stake_local(coin(150, OSMO), Binary::default())
        .call(user)
        .unwrap_err();

    assert_eq!(err, ContractError::InsufficentBalance);

    // Cannot unbond used collateral

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(err, ContractError::ClaimsLocked(Uint128::new(50)));

    // Unstaking

    local_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(50, OSMO),
        )
        .call(owner)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(100),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: Uint128::new(200)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(100, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(200, OSMO)
    );

    local_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(100, OSMO),
        )
        .call(owner)
        .unwrap();
    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(200, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(100, OSMO)
    );

    // Cannot unstake over the lein
    // Error not verified as it is swallowed by intermediate contract
    // int this scenario
    local_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(200, OSMO),
        )
        .call(owner)
        .unwrap_err();
}

#[track_caller]
fn get_last_pending_tx_id(vault: &VaultContractProxy) -> Option<u64> {
    let txs = vault.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(Tx::id)
}

#[test]
fn stake_cross() {
    let owner = "owner";
    let user = "user1";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let cross_staking_code = cross_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let cross_staking = cross_staking_code
        .instantiate(Decimal::percent(10))
        .call(owner)
        .unwrap();

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Staking remotely

    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();

    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    // Hardcoded commit_tx call (lack of IBC support yet)
    vault
        .vault_api_proxy()
        .commit_tx(last_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(150, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(50),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(250)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Cannot stake over collateral

    let err = vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(150, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap_err();

    assert_eq!(err, ContractError::InsufficentBalance);

    // Cannot unbond used collateral

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(err, ContractError::ClaimsLocked(Uint128::new(50)));

    // Unstaking

    cross_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(50, OSMO),
        )
        .call(owner)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(100),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(200)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    cross_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(100, OSMO),
        )
        .call(owner)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );

    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Cannot unstake over the lein
    // Error not verified as it is swallowed by intermediate contract
    // int this scenario
    cross_staking
        .unstake(
            vault.contract_addr.to_string(),
            user.to_owned(),
            coin(300, OSMO),
        )
        .call(owner)
        .unwrap_err();
}

#[test]
fn stake_cross_txs() {
    let owner = "owner";
    let user = "user1";
    let user2 = "user2";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(300, OSMO))
            .unwrap();
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user2), coins(500, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let cross_staking_code = cross_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let cross_staking = cross_staking_code
        .instantiate(Decimal::percent(10))
        .call(owner)
        .unwrap();

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);

    vault
        .bond()
        .with_funds(&coins(500, OSMO))
        .call(user2)
        .unwrap();
    assert_eq!(
        vault.account(user2.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(500),
            free: Uint128::new(500),
        }
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(800, OSMO)
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&cross_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // No pending txs
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs, vec![]);
    // Can query all accounts
    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(accounts.accounts.len(), 2);

    // Staking remotely

    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();

    // One pending tx
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 1);

    // Same user cannot stake while pending tx
    assert_eq!(
        vault
            .stake_remote(
                cross_staking.contract_addr.to_string(),
                coin(100, OSMO),
                Binary::default(),
            )
            .call(user)
            .unwrap_err(),
        ContractError::Lock(LockError::WriteLocked)
    );
    // Store for later
    let first_tx = get_last_pending_tx_id(&vault).unwrap();

    // But other user can
    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            Binary::default(),
        )
        .call(user2)
        .unwrap();

    // Two pending txs
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 2);

    // Last tx commit_tx call
    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(last_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    // First tx is still pending
    let first_id = match vault.all_pending_txs_desc(None, None).unwrap().txs[0] {
        InFlightStaking { id, .. } => id,
        _ => panic!("unexpected tx type"),
    };
    assert_eq!(first_id, first_tx);

    // Cannot query account while pending
    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        MaybeAccountResponse::Locked {}
    ); // write locked
       // Cannot query claims while pending
       // TODO: locked enum not error
    assert!(matches!(
        vault
            .account_claims(user.to_owned(), None, None)
            .unwrap_err(),
        ContractError::Std(GenericErr { .. })
    )); // write locked
        // Can query vault's balance while pending
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(800, OSMO)
    );
    // Can query all accounts, and locked are reported
    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        vec![
            AllAccountsResponseItem {
                user: user.to_string(),
                account: MaybeAccountResponse::Locked {}
            },
            AllAccountsResponseItem {
                user: user2.to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(500),
                    Uint128::new(400)
                ),
            },
        ]
    );

    // Can query the other account
    let acc = vault.account(user2.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(500),
            free: Uint128::new(400),
        }
    );
    // Can query the other account claims
    let claims = vault.account_claims(user2.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );

    // Commit first tx
    vault
        .vault_api_proxy()
        .commit_tx(first_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    // Can query account now
    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
        }
    );
    // Can query claims
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(100)
        })]
    );
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(800, OSMO)
    );
}

#[test]
fn stake_cross_rollback_tx() {
    let owner = "owner";
    let user = "user1";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let cross_staking_code = cross_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let cross_staking = cross_staking_code
        .instantiate(Decimal::percent(10))
        .call(owner)
        .unwrap();

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
        }
    );

    // Staking remotely

    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();

    // One pending tx
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 1);

    // Rollback tx
    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    vault
        .vault_api_proxy()
        .rollback_tx(last_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    // No pending txs
    assert!(vault
        .all_pending_txs_desc(None, None)
        .unwrap()
        .txs
        .is_empty());

    // Funds are restored
    let acc = vault.account(user.to_owned()).unwrap().unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
        }
    );
    // No non-empty claims
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [MaybeLienResponse::Lien(LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: Uint128::new(0)
        })]
    );
    // Vault has the funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(300, OSMO)
    );
}

#[test]
fn multiple_stakes() {
    let owner = "owner";
    let user = "user1";

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(user), coins(1000, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let cross_staking_code = cross_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let cross_staking1 = cross_staking_code
        .instantiate(Decimal::percent(60))
        .call(owner)
        .unwrap();

    let cross_staking2 = cross_staking_code
        .instantiate(Decimal::percent(60))
        .call(owner)
        .unwrap();

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    let local_staking = Addr::unchecked(vault.config().unwrap().local_staking);
    let local_staking = local_staking::multitest_utils::LocalStakingProxy::new(local_staking, &app);

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(1000, OSMO))
        .call(user)
        .unwrap();

    // Stake properly, highest collateral usage is local staking (the 300 OSMO lein)
    //
    // When comparing `LienInfo`, for simplification assume their order to be in the order of
    // leinholder creation. It is guaranteed to work as MT assigns increasing names to the
    // contracts, and leins are queried ascending by the lienholder.

    vault
        .stake_local(coin(300, OSMO), Binary::default())
        .call(user)
        .unwrap();

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(200, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking1.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(100, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking2.contract_addr.as_str())
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(1000),
            free: Uint128::new(700),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            MaybeLienResponse::Lien(LienResponse {
                lienholder: cross_staking1.contract_addr.to_string(),
                amount: Uint128::new(200)
            }),
            MaybeLienResponse::Lien(LienResponse {
                lienholder: cross_staking2.contract_addr.to_string(),
                amount: Uint128::new(100)
            }),
            MaybeLienResponse::Lien(LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(300)
            }),
        ]
    );

    // Still staking properly, but lein on `cross_staking1` goes up to 400 OSMO, with `max_slash`
    // goes up to 240 OSMO, and lein on `cross_staking2` goes up to 500 OSMO, with `mas_slash`
    // goin up to 300 OSMO. `local_staking` lein is still on 300 OSMO with `max_slash` on
    // 30 OSMO. In total `max_slash` goes to 240 + 300 + 30 = 570 OSMO which becomes collateral
    // usage

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(200, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking1.contract_addr.as_str())
        .unwrap();

    vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(400, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking2.contract_addr.as_str())
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap().unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(1000),
            free: Uint128::new(430),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            MaybeLienResponse::Lien(LienResponse {
                lienholder: cross_staking1.contract_addr.to_string(),
                amount: Uint128::new(400)
            }),
            MaybeLienResponse::Lien(LienResponse {
                lienholder: cross_staking2.contract_addr.to_string(),
                amount: Uint128::new(500)
            }),
            MaybeLienResponse::Lien(LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(300)
            }),
        ]
    );

    // Now trying to add more staking to `cross_staking1` and `cross_staking2`, leaving them on 900 OSMO lein,
    // which is still in the range of collateral, but the `max_slash` on those lein goes up to 540. This makes
    // total `max_slash` on 540 + 540 + 30 = 1110 OSMO which exceeds collateral and fails

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(500, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(get_last_pending_tx_id(&vault).unwrap())
        .call(cross_staking1.contract_addr.as_str())
        .unwrap();

    let err = vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(400, OSMO),
            Binary::default(),
        )
        .call(user)
        .unwrap_err();

    assert_eq!(err, ContractError::InsufficentBalance);
}

#[test]
fn all_users_fetching() {
    let owner = "owner";
    let users = ["user1", "user2"];

    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[0]), coins(300, OSMO))
            .unwrap();

        router
            .bank
            .init_balance(storage, &Addr::unchecked(users[1]), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup

    let local_staking_code = local_staking::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking_init_info = StakingInitInfo {
        admin: None,
        code_id: local_staking_code.code_id(),
        msg: to_binary(&Empty {}).unwrap(),
        label: None,
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    // No users should show up no matter of collateral flag

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(accounts.accounts, []);

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(accounts.accounts, []);

    // When user bond some collateral, he should be visible

    vault
        .bond()
        .with_funds(&coins(100, OSMO))
        .call(users[0])
        .unwrap();

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [AllAccountsResponseItem {
            user: users[0].to_string(),
            account: MaybeAccountResponse::new_unlocked(OSMO, Uint128::new(100), Uint128::new(100)),
        }]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [AllAccountsResponseItem {
            user: users[0].to_string(),
            account: MaybeAccountResponse::new_unlocked(OSMO, Uint128::new(100), Uint128::new(100),)
        }]
    );

    // Second user bonds - we want to see him

    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(users[1])
        .unwrap();

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(100),
                    Uint128::new(100),
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(200),
                    Uint128::new(200),
                )
            }
        ]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(100),
                    Uint128::new(100),
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(200),
                    Uint128::new(200),
                )
            }
        ]
    );

    // After unbounding some, but not all collateral, user shall still be visible

    vault.unbond(coin(50, OSMO)).call(users[0]).unwrap();

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(50),
                    Uint128::new(50),
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(200),
                    Uint128::new(200),
                )
            }
        ]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(50),
                    Uint128::new(50),
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(200),
                    Uint128::new(200),
                )
            }
        ]
    );

    // Unbonding all the collateral hides the user when the collateral flag is set
    vault.unbond(coin(200, OSMO)).call(users[1]).unwrap();

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: MaybeAccountResponse::new_unlocked(
                    OSMO,
                    Uint128::new(50),
                    Uint128::new(50),
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: MaybeAccountResponse::new_unlocked(OSMO, Uint128::new(0), Uint128::new(0),)
            }
        ]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [AllAccountsResponseItem {
            user: users[0].to_string(),
            account: MaybeAccountResponse::new_unlocked(OSMO, Uint128::new(50), Uint128::new(50),)
        },]
    );
}

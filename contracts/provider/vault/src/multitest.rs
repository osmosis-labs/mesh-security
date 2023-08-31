mod local_staking;

use cosmwasm_std::{coin, coins, to_binary, Addr, Binary, Decimal, Empty, Uint128};
use cw_multi_test::App as MtApp;
use mesh_apis::ibc::AddValidator;
use mesh_external_staking::contract::multitest_utils::ExternalStakingContractProxy;
use mesh_external_staking::msg::{AuthorizedEndpoint, ReceiveVirtualStake};
use mesh_external_staking::test_methods_impl::test_utils::TestMethods;
use mesh_sync::Tx::InFlightStaking;
use mesh_sync::{Tx, ValueRange};
use sylvia::multitest::App;

use crate::contract;
use crate::contract::multitest_utils::VaultContractProxy;
use crate::contract::test_utils::VaultApi;
use crate::error::ContractError;
use crate::msg::{AccountResponse, AllAccountsResponseItem, LienResponse, StakingInitInfo};

const OSMO: &str = "OSMO";
const STAR: &str = "star";

/// 10% slashing on the remote chain
const SLASHING_PERCENTAGE: u64 = 10;

#[track_caller]
fn get_last_external_staking_pending_tx_id(
    contract: &ExternalStakingContractProxy<MtApp>,
) -> Option<u64> {
    let txs = contract.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(Tx::id)
}

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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::zero(),
            free: ValueRange::new_val(Uint128::zero()),
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(100),
            free: ValueRange::new_val(Uint128::new(100)),
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(250),
            free: ValueRange::new_val(Uint128::new(250)),
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(50),
            free: ValueRange::new_val(Uint128::new(50)),
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(30),
            free: ValueRange::new_val(Uint128::new(30)),
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

    // Unbonding over bounded fails

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(
        err,
        ContractError::ClaimsLocked(ValueRange::new_val(Uint128::new(30)))
    );
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(300)),
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(200)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(100))
        }]
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(50)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(250))
        }]
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
    assert_eq!(
        err,
        ContractError::ClaimsLocked(ValueRange::new_val(Uint128::new(50)))
    );

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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(100)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(200))
        }]
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
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(200)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: local_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(100))
        }]
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

    // Cannot unstake over the lien
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

// Add comment
// transaction ids are in different namespaces
// if one transaction passes the second one cannot fail
#[track_caller]
fn get_last_pending_tx_id(vault: &VaultContractProxy<MtApp>) -> Option<u64> {
    let txs = vault.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(Tx::id)
}

#[track_caller]
fn skip_time(app: &App<MtApp>, skip_time: u64) {
    let mut block_info = app.app().block_info();
    let ts = block_info.time.plus_seconds(skip_time);
    block_info.time = ts;
    app.app_mut().set_block(block_info);
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
    let cross_staking_code =
        mesh_external_staking::contract::multitest_utils::CodeId::store_code(&app);
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

    let unbond_period = 100;
    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

    let cross_staking = cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            Decimal::percent(SLASHING_PERCENTAGE),
        )
        .call(owner)
        .unwrap();

    // Set active validator
    let validator = "validator";

    let activate = AddValidator::mock(validator);
    cross_staking
        .test_methods_proxy()
        .test_set_active_validator(activate)
        .call("test")
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(300)),
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
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new(Uint128::new(200), Uint128::new(300)),
        }
    );

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx = get_last_external_staking_pending_tx_id(&cross_staking).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(200)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(100))
        }]
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

    // second stake remote
    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(150, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new(Uint128::new(50), Uint128::new(200)),
        }
    );

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx = get_last_external_staking_pending_tx_id(&cross_staking).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(50)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(250))
        }]
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
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap_err();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(50)),
        }
    );

    assert_eq!(err, ContractError::InsufficentBalance);

    // Cannot unbond used collateral

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(
        err,
        ContractError::ClaimsLocked(ValueRange::new_val(Uint128::new(50)))
    );

    // Unstake does not free collateral on vault right away
    cross_staking
        .unstake(validator.to_owned(), coin(50, OSMO))
        .call(user)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(50)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(250))
        }]
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

    // External staking contract will wait for `unbond_period` after receiving
    // confirmation through the IBC channel that `unstake` was successfully executed.
    skip_time(&app, unbond_period);
    let tx_id = get_last_external_staking_pending_tx_id(&cross_staking).unwrap();
    cross_staking
        .test_methods_proxy()
        .test_commit_unstake(tx_id)
        .call("test")
        .unwrap();

    // No tokens is withdrawn before unbonding period is over
    let insufficient_time = 99;
    skip_time(&app, insufficient_time);

    cross_staking.withdraw_unbonded().call(user).unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(50)),
        }
    );

    // After the unbonding period user can withdraw unbonded tokens
    let remaining_time = 1;
    skip_time(&app, remaining_time);

    cross_staking.withdraw_unbonded().call(user).unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(100)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(200))
        }]
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

    // Unstake further funds
    cross_staking
        .unstake(validator.to_owned(), coin(100, OSMO))
        .call(user)
        .unwrap();

    let tx_id = get_last_external_staking_pending_tx_id(&cross_staking).unwrap();
    cross_staking
        .test_methods_proxy()
        .test_commit_unstake(tx_id)
        .call("test")
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(100)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(200))
        }]
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

    // Cannot unstake over the lien
    // Error not verified as it is swallowed by intermediate contract
    // int this scenario
    cross_staking
        .unstake(user.to_owned(), coin(300, OSMO))
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
    let cross_staking_code =
        mesh_external_staking::contract::multitest_utils::CodeId::store_code(&app);
    let vault_code = contract::multitest_utils::CodeId::store_code(&app);

    let unbond_period = 100;
    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

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

    let cross_staking = cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            Decimal::percent(SLASHING_PERCENTAGE),
        )
        .call(owner)
        .unwrap();

    // Set active validator
    let validator = "validator";

    let activate = AddValidator::mock(validator);
    cross_staking
        .test_methods_proxy()
        .test_set_active_validator(activate)
        .call("test")
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(300)),
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
        vault.account(user2.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(500),
            free: ValueRange::new_val(Uint128::new(500)),
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
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // One pending tx
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 1);
    // Store for later
    let first_tx = get_last_pending_tx_id(&vault).unwrap();

    // Same user can stake while pending tx
    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(50, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();
    // Store for later
    let second_tx = get_last_pending_tx_id(&vault).unwrap();

    // Other user can as well
    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user2)
        .unwrap();

    // Three pending txs
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 3);

    // Last tx commit_tx call
    let last_tx = get_last_pending_tx_id(&vault).unwrap();
    vault
        .vault_api_proxy()
        .commit_tx(last_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    // Two pending txs now
    assert_eq!(vault.all_pending_txs_desc(None, None).unwrap().txs.len(), 2);

    // First tx (old one) is still pending
    let first_id = match vault.all_pending_txs_desc(None, None).unwrap().txs[1] {
        InFlightStaking { id, .. } => id,
        _ => panic!("unexpected tx type"),
    };
    assert_eq!(first_id, first_tx);
    // Second tx (recent one) is still pending
    let second_id = match vault.all_pending_txs_desc(None, None).unwrap().txs[0] {
        InFlightStaking { id, .. } => id,
        _ => panic!("unexpected tx type"),
    };
    assert_eq!(second_id, second_tx);

    // Can query account while pending
    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse::new(
            OSMO,
            Uint128::new(300),
            ValueRange::new(Uint128::new(150), Uint128::new(300))
        )
    );
    // Can query claims, and value ranges are reported
    assert_eq!(
        vault
            .account_claims(user.to_owned(), None, None)
            .unwrap()
            .claims,
        [LienResponse {
            lienholder: "contract2".to_string(),
            amount: ValueRange::new(Uint128::zero(), Uint128::new(150))
        }]
    );
    // Can query vault's balance while pending
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(&vault.contract_addr, OSMO)
            .unwrap(),
        coin(800, OSMO)
    );
    // Can query all accounts, and value ranges are reported
    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        vec![
            AllAccountsResponseItem {
                user: user.to_string(),
                account: AccountResponse {
                    denom: OSMO.to_owned(),
                    bonded: Uint128::new(300),
                    free: ValueRange::new(Uint128::new(150), Uint128::new(300)),
                },
            },
            AllAccountsResponseItem {
                user: user2.to_string(),
                account: AccountResponse {
                    denom: OSMO.to_owned(),
                    bonded: Uint128::new(500),
                    free: ValueRange::new_val(Uint128::new(400)),
                },
            },
        ]
    );

    // Can query the other account as well
    let acc = vault.account(user2.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(500),
            free: ValueRange::new_val(Uint128::new(400)),
        }
    );
    // Can query the other account claims
    let claims = vault.account_claims(user2.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::new(100))
        }]
    );

    // Commit first tx
    vault
        .vault_api_proxy()
        .commit_tx(first_tx)
        .call(cross_staking.contract_addr.as_str())
        .unwrap();

    // Can query account
    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new(Uint128::new(150), Uint128::new(200)),
        }
    );
    // Can query claims
    // The other tx is still pending, and that is reflected in the reported value range
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new(Uint128::new(100), Uint128::new(150))
        }]
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
    let cross_staking_code =
        mesh_external_staking::contract::multitest_utils::CodeId::store_code(&app);
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

    let unbond_period = 100;
    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

    let cross_staking = cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            Decimal::percent(SLASHING_PERCENTAGE),
        )
        .call(owner)
        .unwrap();

    // Set active validator
    let validator = "validator";

    let activate = AddValidator::mock(validator);
    cross_staking
        .test_methods_proxy()
        .test_set_active_validator(activate)
        .call("test")
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(300, OSMO))
        .call(user)
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(300)),
        }
    );

    // Staking remotely

    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
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
    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: ValueRange::new_val(Uint128::new(300)),
        }
    );
    // No non-empty claims
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [LienResponse {
            lienholder: cross_staking.contract_addr.to_string(),
            amount: ValueRange::new_val(Uint128::zero())
        }]
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
    let cross_staking_code =
        mesh_external_staking::contract::multitest_utils::CodeId::store_code(&app);
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

    let unbond_period = 100;
    let slashing_percentage: u64 = 60;
    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

    let cross_staking1 = cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact.clone(),
            Decimal::percent(slashing_percentage),
        )
        .call(owner)
        .unwrap();

    let cross_staking2 = cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            Decimal::percent(slashing_percentage),
        )
        .call(owner)
        .unwrap();

    let local_staking = Addr::unchecked(vault.config().unwrap().local_staking);
    let local_staking = local_staking::multitest_utils::LocalStakingProxy::new(local_staking, &app);

    // Set active validator
    let validator = "validator";

    let activate = AddValidator::mock(validator);
    cross_staking1
        .test_methods_proxy()
        .test_set_active_validator(activate.clone())
        .call("test")
        .unwrap();

    cross_staking2
        .test_methods_proxy()
        .test_set_active_validator(activate)
        .call("test")
        .unwrap();

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(1000, OSMO))
        .call(user)
        .unwrap();

    // Stake properly, highest collateral usage is local staking (the 300 OSMO lien)
    //
    // When comparing `LienInfo`, for simplification assume their order to be in the order of
    // lienholder creation. It is guaranteed to work as MT assigns increasing names to the
    // contracts, and liens are queried ascending by the lienholder.

    vault
        .stake_local(coin(300, OSMO), Binary::default())
        .call(user)
        .unwrap();

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(200, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx =
        get_last_external_staking_pending_tx_id(&cross_staking1).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking1
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx =
        get_last_external_staking_pending_tx_id(&cross_staking2).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking2
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(1000),
            free: ValueRange::new_val(Uint128::new(700)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(300))
            },
            LienResponse {
                lienholder: cross_staking1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(200))
            },
            LienResponse {
                lienholder: cross_staking2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(100))
            },
        ]
    );

    // Still staking properly, but lien on `cross_staking1` goes up to 400 OSMO, with `max_slash`
    // goes up to 240 OSMO, and lien on `cross_staking2` goes up to 500 OSMO, with `mas_slash`
    // goin up to 300 OSMO. `local_staking` lien is still on 300 OSMO with `max_slash` on
    // 30 OSMO. In total `max_slash` goes to 240 + 300 + 30 = 570 OSMO which becomes collateral
    // usage

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(200, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx =
        get_last_external_staking_pending_tx_id(&cross_staking1).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking1
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(400, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx =
        get_last_external_staking_pending_tx_id(&cross_staking2).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking2
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    assert_eq!(
        vault.account(user.to_owned()).unwrap(),
        AccountResponse::new(
            OSMO,
            Uint128::new(1000),
            ValueRange::new_val(Uint128::new(430))
        ),
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(300))
            },
            LienResponse {
                lienholder: cross_staking1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(400))
            },
            LienResponse {
                lienholder: cross_staking2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(500))
            },
        ]
    );

    // Now trying to add more staking to `cross_staking1` and `cross_staking2`, leaving them on 900 OSMO lien,
    // which is still in the range of collateral, but the `max_slash` on those lien goes up to 540. This makes
    // total `max_slash` on 540 + 540 + 30 = 1110 OSMO which exceeds collateral and fails

    vault
        .stake_remote(
            cross_staking1.contract_addr.to_string(),
            coin(500, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // TODO: Hardcoded external-staking's commit_stake call (lack of IBC support yet).
    // This should be through `IbcPacketAckMsg`
    let last_external_staking_tx =
        get_last_external_staking_pending_tx_id(&cross_staking1).unwrap();
    println!("last_external_staking_tx: {:?}", last_external_staking_tx);
    cross_staking1
        .test_methods_proxy()
        .test_commit_stake(last_external_staking_tx)
        .call("test")
        .unwrap();

    let err = vault
        .stake_remote(
            cross_staking2.contract_addr.to_string(),
            coin(400, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: validator.to_string(),
            })
            .unwrap(),
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
            account: AccountResponse::new(
                OSMO,
                Uint128::new(100),
                ValueRange::new_val(Uint128::new(100))
            ),
        }]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [AllAccountsResponseItem {
            user: users[0].to_string(),
            account: AccountResponse::new(
                OSMO,
                Uint128::new(100),
                ValueRange::new_val(Uint128::new(100),)
            )
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
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(100),
                    ValueRange::new_val(Uint128::new(100),)
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(200),
                    ValueRange::new_val(Uint128::new(200),)
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
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(100),
                    ValueRange::new_val(Uint128::new(100),)
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(200),
                    ValueRange::new_val(Uint128::new(200),)
                )
            }
        ]
    );

    // After unbonding some, but not all collateral, user shall still be visible

    vault.unbond(coin(50, OSMO)).call(users[0]).unwrap();

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [
            AllAccountsResponseItem {
                user: users[0].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(50),
                    ValueRange::new_val(Uint128::new(50),)
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(200),
                    ValueRange::new_val(Uint128::new(200),)
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
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(50),
                    ValueRange::new_val(Uint128::new(50),)
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(200),
                    ValueRange::new_val(Uint128::new(200),)
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
                account: AccountResponse::new(
                    OSMO,
                    Uint128::new(50),
                    ValueRange::new_val(Uint128::new(50),)
                )
            },
            AllAccountsResponseItem {
                user: users[1].to_string(),
                account: AccountResponse::new(
                    OSMO,
                    Uint128::zero(),
                    ValueRange::new_val(Uint128::zero(),)
                )
            }
        ]
    );

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(
        accounts.accounts,
        [AllAccountsResponseItem {
            user: users[0].to_string(),
            account: AccountResponse::new(
                OSMO,
                Uint128::new(50),
                ValueRange::new_val(Uint128::new(50),)
            )
        },]
    );
}

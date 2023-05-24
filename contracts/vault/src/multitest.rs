mod local_staking;
use crate::error::ContractError;
use crate::msg::{LienInfo, StakingInitInfo};
use crate::{contract, msg::AccountResponse};
use cosmwasm_std::{coin, coins, to_binary, Addr, Binary, Empty, StdError, Uint128};
use cw_multi_test::App as MtApp;
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
}

#[test]
fn binding() {
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::zero(),
            free: Uint128::zero(),
            claims: vec![],
        }
    );

    // Bond some tokens

    vault
        .bond()
        .with_funds(&coins(100, OSMO))
        .call(user)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(100),
            free: Uint128::new(100),
            claims: vec![],
        }
    );
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(250),
            free: Uint128::new(250),
            claims: vec![],
        }
    );
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(50),
            free: Uint128::new(50),
            claims: vec![],
        }
    );
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(30),
            free: Uint128::new(30),
            claims: vec![],
        }
    );
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(300),
            claims: vec![],
        }
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
            .query_balance(&local_staking.contract_addr, OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Stakeing localy

    vault
        .stake_local(coin(100, OSMO), Binary::default())
        .call(user)
        .unwrap();

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
            claims: vec![LienInfo {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(100)
            }],
        }
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(50),
            claims: vec![LienInfo {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(250)
            }],
        }
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(100),
            claims: vec![LienInfo {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(200)
            }],
        }
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

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(300),
            free: Uint128::new(200),
            claims: vec![LienInfo {
                lienholder: local_staking.contract_addr.to_string(),
                amount: Uint128::new(100)
            }],
        }
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

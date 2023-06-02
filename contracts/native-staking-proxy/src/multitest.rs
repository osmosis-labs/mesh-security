use cosmwasm_std::testing::mock_env;
use cosmwasm_std::{coin, coins, Addr, Decimal, Validator};
use cw_multi_test::App as MtApp;

use sylvia::multitest::App;

use crate::contract;
use crate::msg::ConfigResponse;

const DENOM: &str = "TOKEN"; // cw-multi-test v0.16.x does not support custom tokens yet

fn init_app(owner: &str, validators: &[&str]) -> App {
    // Fund the staking contract, and add validators to staking keeper
    let block = mock_env().block;
    let app = MtApp::new(|router, api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(owner), coins(1000, DENOM))
            .unwrap();

        for &validator in validators {
            let valoper1 = Validator {
                address: validator.to_owned(),
                commission: Decimal::percent(10),
                max_commission: Decimal::percent(20),
                max_change_rate: Decimal::percent(1),
            };
            router
                .staking
                .add_validator(api, storage, &block, valoper1)
                .unwrap();
        }
    });
    App::new(app)
}

#[test]
fn instantiation() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake

    let app = init_app(owner, &[validator]);

    // Contract setup, with funds transfer
    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy = staking_proxy_code
        .instantiate(DENOM.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(1000, DENOM))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    let config = staking_proxy.config().unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            denom: DENOM.to_owned(),
            parent: Addr::unchecked(owner), // parent is the staking contract
            owner: Addr::unchecked(user),   // owner is the user
        }
    );

    // Check that funds have been staked
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), DENOM)
            .unwrap(),
        coin(0, DENOM)
    );
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(1000, DENOM));

    // TODO: Check side effects: data payload, etc.
}

#[test]
fn staking() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake

    let app = init_app(owner, &[validator]);

    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy = staking_proxy_code
        .instantiate(DENOM.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(1, DENOM))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Stake some more on behalf of the user
    staking_proxy
        .stake(validator.to_owned())
        .with_funds(&coins(2, DENOM))
        .call(owner) // Staking has the funds at the time
        .unwrap();

    // Check that new funds have been staked as well
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), DENOM)
            .unwrap(),
        coin(0, DENOM)
    );
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(3, DENOM));
}

#[test]
fn restaking() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake
    let validator2 = "validator2"; // Where to re-stake

    let app = init_app(owner, &[validator, validator2]);

    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy = staking_proxy_code
        .instantiate(DENOM.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(10, DENOM))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Restake 30% to a different validator
    staking_proxy
        .restake(validator.to_owned(), validator2.to_owned(), coin(3, DENOM))
        .call(user)
        .unwrap();

    // Check that funds have been re-staked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(7, DENOM));
    let delegation2 = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator2.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation2.amount, coin(3, DENOM));
}

#[test]
fn unstaking() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(owner, &[validator]);

    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy = staking_proxy_code
        .instantiate(DENOM.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(10, DENOM))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Unstake 50%
    staking_proxy
        .unstake(validator.to_owned(), coin(5, DENOM))
        .call(user)
        .unwrap();

    // Check that funds have been unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(5, DENOM));

    // TODO: And that they are now held, until the unbonding period
}

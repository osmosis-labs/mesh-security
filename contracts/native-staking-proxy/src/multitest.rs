use anyhow::Result as AnyResult;

use cosmwasm_std::testing::mock_env;
use cosmwasm_std::{coin, coins, to_binary, Addr, Decimal, Validator};

use cw_multi_test::{App as MtApp, StakingInfo, StakingSudo, SudoMsg};

use sylvia::multitest::App;

use mesh_vault::contract::multitest_utils::VaultContractProxy;

use crate::contract;
use crate::msg::ConfigResponse;

const OSMO: &str = "uosmo";
const UNBONDING_PERIOD: u64 = 17 * 24 * 60 * 60; // 7 days

fn init_app(owner: &str, validators: &[&str]) -> App {
    // Fund the staking contract, and add validators to staking keeper
    let block = mock_env().block;
    let app = MtApp::new(|router, api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(owner), coins(1000, OSMO))
            .unwrap();
        router
            .staking
            .setup(
                storage,
                StakingInfo {
                    bonded_denom: OSMO.to_string(),
                    unbonding_time: UNBONDING_PERIOD,
                    apr: Decimal::percent(1),
                },
            )
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

fn setup<'app>(app: &'app App, owner: &str) -> AnyResult<VaultContractProxy<'app>> {
    let vault_code = mesh_vault::contract::multitest_utils::CodeId::store_code(app);
    let staking_code = mesh_native_staking::contract::multitest_utils::CodeId::store_code(app);
    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(app);

    // Instantiate vault msg
    let staking_init_info = mesh_vault::msg::StakingInitInfo {
        admin: None,
        code_id: staking_code.code_id(),
        msg: to_binary(&mesh_native_staking::contract::InstantiateMsg {
            denom: OSMO.to_owned(),
            proxy_code_id: staking_proxy_code.code_id(),
        })
        .unwrap(),
        label: None,
    };

    // Instantiates vault and staking
    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();
    Ok(vault)
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
        .instantiate(OSMO.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(1000, OSMO))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    let config = staking_proxy.config().unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            denom: OSMO.to_owned(),
            parent: Addr::unchecked(owner), // parent is the staking contract
            owner: Addr::unchecked(user),   // owner is the user
        }
    );

    // Check that funds have been staked
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(0, OSMO)
    );
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(1000, OSMO));

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
        .instantiate(OSMO.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(1, OSMO))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Stake some more on behalf of the user
    staking_proxy
        .stake(validator.to_owned())
        .with_funds(&coins(2, OSMO))
        .call(owner) // Staking has the funds at the time
        .unwrap();

    // Check that new funds have been staked as well
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(0, OSMO)
    );
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(3, OSMO));
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
        .instantiate(OSMO.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(10, OSMO))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Restake 30% to a different validator
    staking_proxy
        .restake(validator.to_owned(), validator2.to_owned(), coin(3, OSMO))
        .call(user)
        .unwrap();

    // Check that funds have been re-staked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(7, OSMO));
    let delegation2 = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator2.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation2.amount, coin(3, OSMO));
}

#[test]
fn unstaking() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(owner, &[validator]);

    let staking_proxy_code = contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy = staking_proxy_code
        .instantiate(OSMO.to_owned(), user.to_owned(), validator.to_owned())
        .with_label("Local Staking Proxy")
        .with_funds(&coins(10, OSMO))
        .call(owner) // Instantiated by the staking contract
        .unwrap();

    // Unstake 50%
    staking_proxy
        .unstake(validator.to_owned(), coin(5, OSMO))
        .call(user)
        .unwrap();

    // Check that funds have been unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(5, OSMO));

    // TODO: And that they are now held, until the unbonding period
}

#[test]
fn releasing_unbonded() {
    let owner = "vault_admin";

    let vault_addr = "contract0"; // First created contract
    let _staking_addr = "contract1"; // Second created contract. Created by vault contract on init
    let proxy_addr = "contract2"; // Third contract (instantiated by staking contract on stake)

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner).unwrap();

    // Bond some funds to the vault
    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();

    // Stakes some of it locally. This instantiates the staking proxy contract for user
    vault
        .stake_local(
            coin(100, OSMO),
            to_binary(&mesh_native_staking::msg::StakeMsg {
                validator: validator.to_owned(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // Access staking proxy instance
    let staking_proxy = contract::multitest_utils::NativeStakingProxyContractProxy::new(
        Addr::unchecked(proxy_addr),
        &app,
    );

    // Unstake 100%
    staking_proxy
        .unstake(validator.to_owned(), coin(100, OSMO))
        .call(user)
        .unwrap();

    // Check that funds have been fully unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap();
    assert!(delegation.is_none());

    // Advance time until the unbonding period is over
    app.update_block(|block| {
        block.height += 1234;
        block.time = block.time.plus_seconds(UNBONDING_PERIOD + 1);
    });
    // Manually cause queue to get processed. TODO: Handle automatically in sylvia mt or cw-mt
    app.app_mut()
        .sudo(SudoMsg::Staking(StakingSudo::ProcessQueue {}))
        .unwrap();

    // Release the unbonded funds
    staking_proxy.release_unbonded().call(user).unwrap();

    // Check that the vault has the funds again
    assert_eq!(
        app.app().wrap().query_balance(vault_addr, OSMO).unwrap(),
        coin(200, OSMO)
    );
}

#[test]
fn withdrawing_rewards() {
    let owner = "vault_admin";

    let _vault_addr = "contract0"; // First created contract
    let _staking_addr = "contract1"; // Second created contract. Created by vault contract on init
    let proxy_addr = "contract2"; // Third contract (instantiated by staking contract on stake)

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner).unwrap();

    // Bond some funds to the vault
    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();

    // Stakes some of it locally. This instantiates the staking proxy contract for user
    vault
        .stake_local(
            coin(100, OSMO),
            to_binary(&mesh_native_staking::msg::StakeMsg {
                validator: validator.to_owned(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // Record current user funds
    let original_funds = app.app().wrap().query_balance(user, OSMO).unwrap();

    // Access staking proxy instance
    let staking_proxy = contract::multitest_utils::NativeStakingProxyContractProxy::new(
        Addr::unchecked(proxy_addr),
        &app,
    );

    // Advance time enough for rewards to accrue
    app.update_block(|block| {
        block.height += 12345678;
        block.time = block.time.plus_seconds(123456789);
    });

    // Withdraw rewards
    staking_proxy.withdraw_rewards().call(user).unwrap();

    // User now has some rewards
    let current_funds = app.app().wrap().query_balance(user, OSMO).unwrap();
    assert!(current_funds.amount > original_funds.amount);
}

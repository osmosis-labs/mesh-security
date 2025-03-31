use anyhow::Result as AnyResult;

use cosmwasm_std::testing::mock_env;
use cosmwasm_std::{coin, coins, to_json_binary, Addr, Decimal, Validator};

use cw_multi_test::{App as MtApp, IntoBech32, StakingInfo};
use sylvia::multitest::{App, Proxy};

use mesh_vault::mock::sv::mt::VaultMockProxy;
use mesh_vault::mock::VaultMock;
use mesh_vault::msg::LocalStakingInfo;

use mesh_native_staking::contract::sv::mt::NativeStakingContractProxy;
use mesh_native_staking::contract::NativeStakingContract;

use crate::mock::sv::mt::NativeStakingProxyMockProxy;
use crate::mock::NativeStakingProxyMock;
use crate::msg::ConfigResponse;

const OSMO: &str = "uosmo";
const UNBONDING_PERIOD: u64 = 17 * 24 * 60 * 60; // 7 days

fn init_app(owner: &str, validators: &[&str]) -> App<MtApp> {
    // Fund the staking contract, and add validators to staking keeper
    let block = mock_env().block;
    let app = MtApp::new(|router, api, storage| {
        router
            .bank
            .init_balance(storage, &owner.into_bech32(), coins(1000, OSMO))
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
            let valoper1 = Validator::create(
                validator.to_owned(),
                Decimal::percent(10),
                Decimal::percent(20),
                Decimal::percent(1),
            );
            router
                .staking
                .add_validator(api, storage, &block, valoper1)
                .unwrap();
        }
    });
    App::new(app)
}

fn setup<'app>(
    app: &'app App<MtApp>,
    owner: &'app str,
    user: &str,
    validators: &[&str],
) -> AnyResult<Proxy<'app, MtApp, VaultMock>> {
    let vault_code = mesh_vault::mock::sv::mt::CodeId::store_code(app);
    let staking_code = mesh_native_staking::contract::sv::mt::CodeId::store_code(app);
    let staking_proxy_code = crate::mock::sv::mt::CodeId::store_code(app);

    // Instantiate vault msg
    let staking_init_info = mesh_vault::msg::StakingInitInfo {
        admin: None,
        code_id: staking_code.code_id(),
        msg: to_json_binary(&mesh_native_staking::contract::sv::InstantiateMsg {
            denom: OSMO.to_owned(),
            proxy_code_id: staking_proxy_code.code_id(),
            slash_ratio_dsign: Decimal::percent(5),
            slash_ratio_offline: Decimal::percent(5),
        })
        .unwrap(),
        label: None,
    };

    // Instantiates vault and staking
    let vault = vault_code
        .instantiate(
            OSMO.to_owned(),
            Some(LocalStakingInfo::New(staking_init_info)),
        )
        .with_label("Vault")
        .call(&owner.into_bech32())
        .unwrap();

    // Bond some funds to the vault
    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(&user.into_bech32())
        .unwrap();

    // Stakes some of it locally. This instantiates the staking proxy contract for user
    for &validator in validators {
        vault
            .stake_local(
                coin(100, OSMO),
                to_json_binary(&mesh_native_staking::msg::StakeMsg {
                    validator: validator.to_owned(),
                })
                .unwrap(),
            )
            .call(&user.into_bech32())
            .unwrap();
    }

    Ok(vault)
}

#[test]
fn instantiation() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;
    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Check config
    let config = staking_proxy.config().unwrap();
    assert_eq!(
        config,
        ConfigResponse {
            denom: OSMO.to_owned(),
            parent: Addr::unchecked(staking_addr), // parent is the staking contract
            owner: user.into_bech32(),             // owner is the user
        }
    );

    // Check that initial funds have been staked
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
    assert_eq!(delegation.amount, coin(100, OSMO));
}

#[test]
fn staking() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Stake some more
    vault
        .stake_local(
            coin(20, OSMO),
            to_json_binary(&mesh_native_staking::msg::StakeMsg {
                validator: validator.to_owned(),
            })
            .unwrap(),
        )
        .call(&user.into_bech32())
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
    assert_eq!(delegation.amount, coin(120, OSMO));
}

#[test]
fn restaking() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake
    let validator2 = "validator2"; // Where to re-stake

    let app = init_app(user, &[validator, validator2]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Restake 30% to a different validator
    staking_proxy
        .restake(validator.to_owned(), validator2.to_owned(), coin(30, OSMO))
        .call(&user.into_bech32())
        .unwrap();

    // Check that funds have been re-staked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(70, OSMO));
    let delegation2 = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr, validator2.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation2.amount, coin(30, OSMO));
}

#[test]
fn unstaking() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Unstake 50%
    staking_proxy
        .unstake(validator.to_owned(), coin(50, OSMO))
        .call(&user.into_bech32())
        .unwrap();

    // Check that funds have been unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(50, OSMO));

    // And that they are now held, until the unbonding period
    // First, check that the contract has no funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Advance time until the unbonding period is over
    process_staking_unbondings(&app);

    // Check that the contract now has the funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr, OSMO)
            .unwrap(),
        coin(50, OSMO)
    );
}

#[test]
fn burning() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Burn 10%, from validator
    staking_proxy
        .burn(Some(validator.to_owned()), coin(10, OSMO))
        .call(&Addr::unchecked(staking_addr))
        .unwrap();

    // Check that funds have been unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap()
        .unwrap();
    assert_eq!(delegation.amount, coin(90, OSMO));

    // And that they are now held, until the unbonding period
    // First, check that the contract has no funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Advance time until the unbonding period is over
    process_staking_unbondings(&app);

    // Check that the contract now has the funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(10, OSMO)
    );

    // But they cannot be released
    staking_proxy
        .release_unbonded()
        .call(&user.into_bech32())
        .unwrap();

    // Check that the contract still has the funds (they are being effectively "burned")
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr, OSMO)
            .unwrap(),
        coin(10, OSMO)
    );
}

#[test]
fn burning_multiple_delegations() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validators = ["validator1", "validator2"]; // Where to stake / unstake

    let app = init_app(user, &validators); // Fund user, create validator
    let vault = setup(&app, owner, user, &validators).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Burn 15%, no validator specified
    let burn_amount = 15;
    staking_proxy
        .burn(None, coin(burn_amount, OSMO))
        .call(&Addr::unchecked(staking_addr))
        .unwrap();

    // Check that funds have been unstaked (15 / 2 = 7.5, rounded down to 7, rounded up to 8)
    // First validator gets the round up
    let delegation1 = app
        .app()
        .wrap()
        .query_delegation(
            staking_proxy.contract_addr.clone(),
            validators[0].to_owned(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(delegation1.amount, coin(100 - (burn_amount / 2 + 1), OSMO));
    let delegation2 = app
        .app()
        .wrap()
        .query_delegation(
            staking_proxy.contract_addr.clone(),
            validators[1].to_owned(),
        )
        .unwrap()
        .unwrap();
    assert_eq!(delegation2.amount, coin(100 - burn_amount / 2, OSMO));

    // And that they are now held, until the unbonding period
    // First, check that the contract has no funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(0, OSMO)
    );

    // Advance time until the unbonding period is over
    process_staking_unbondings(&app);

    // Check that the contract now has the funds
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr.clone(), OSMO)
            .unwrap(),
        coin(15, OSMO)
    );

    // But they cannot be released
    staking_proxy
        .release_unbonded()
        .call(&user.into_bech32())
        .unwrap();

    // Check that the contract still has the funds (they are being effectively "burned")
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(staking_proxy.contract_addr, OSMO)
            .unwrap(),
        coin(15, OSMO)
    );
}

#[test]
fn releasing_unbonded() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Unstake 100%
    staking_proxy
        .unstake(validator.to_owned(), coin(100, OSMO))
        .call(&user.into_bech32())
        .unwrap();

    // Check that funds have been fully unstaked
    let delegation = app
        .app()
        .wrap()
        .query_delegation(staking_proxy.contract_addr.clone(), validator.to_owned())
        .unwrap();
    assert!(delegation.is_none());

    // Advance time until the unbonding period is over
    process_staking_unbondings(&app);

    // Release the unbonded funds
    staking_proxy
        .release_unbonded()
        .call(&user.into_bech32())
        .unwrap();

    // Check that the vault has the funds again
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(vault.contract_addr, OSMO)
            .unwrap(),
        coin(200, OSMO)
    );
}

#[test]
fn withdrawing_rewards() {
    let owner = "vault_admin";

    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake / unstake

    let app = init_app(user, &[validator]); // Fund user, create validator
    let vault = setup(&app, owner, user, &[validator]).unwrap();
    let staking_addr = vault.config().unwrap().local_staking.unwrap();
    let local_staking: Proxy<'_, MtApp, NativeStakingContract> =
        Proxy::new(Addr::unchecked(staking_addr.clone()), &app);

    let proxy_addr = local_staking
        .proxy_by_owner(user.into_bech32().to_string())
        .unwrap()
        .proxy;

    // Record current vault, staking and user funds
    let original_vault_funds = app
        .app()
        .wrap()
        .query_balance(vault.contract_addr.clone(), OSMO)
        .unwrap();
    let original_staking_funds = app
        .app()
        .wrap()
        .query_balance(staking_addr.clone(), OSMO)
        .unwrap();
    let original_user_funds = app
        .app()
        .wrap()
        .query_balance(user.into_bech32().to_string(), OSMO)
        .unwrap();

    // Access staking proxy instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyMock> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // Advance time enough for rewards to accrue
    app.update_block(|block| {
        block.height += 12345678;
        block.time = block.time.plus_seconds(123456789);
    });

    // Withdraw rewards
    staking_proxy
        .withdraw_rewards()
        .call(&user.into_bech32())
        .unwrap();

    // User now has some rewards
    let current_funds = app
        .app()
        .wrap()
        .query_balance(user.into_bech32().to_string(), OSMO)
        .unwrap();
    assert!(current_funds.amount > original_user_funds.amount);

    // Staking hasn't received any rewards
    let staking_funds = app.app().wrap().query_balance(staking_addr, OSMO).unwrap();
    assert_eq!(original_staking_funds, staking_funds);

    // Vault hasn't received any rewards
    let vault_funds = app
        .app()
        .wrap()
        .query_balance(vault.contract_addr, OSMO)
        .unwrap();
    assert_eq!(original_vault_funds, vault_funds);
}

fn process_staking_unbondings(app: &App<MtApp>) {
    // Advance unbonding period
    app.app_mut().update_block(|block| {
        block.time = block.time.plus_seconds(UNBONDING_PERIOD);
        block.height += UNBONDING_PERIOD / 5;
    });
    // // This is deprecated as unneeded, but tests fail if it isn't here. What's up???
    // app.app_mut()
    //     .sudo(cw_multi_test::SudoMsg::Staking(
    //         #[allow(deprecated)]
    //         cw_multi_test::StakingSudo::ProcessQueue {},
    //     ))
    //     .unwrap();
}

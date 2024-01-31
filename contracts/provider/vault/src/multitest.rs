use cosmwasm_std::{coin, coins, to_binary, Addr, Decimal, Uint128, Validator};
use cw_multi_test::{App as MtApp, StakingInfo};
use mesh_apis::ibc::AddValidator;
use mesh_external_staking::contract::multitest_utils::ExternalStakingContractProxy;
use mesh_external_staking::msg::{AuthorizedEndpoint, ReceiveVirtualStake, StakeInfo};
use mesh_external_staking::state::SlashRatio;
use mesh_external_staking::state::Stake;
use mesh_external_staking::test_methods_impl::test_utils::TestMethods;
use mesh_native_staking::contract::multitest_utils::NativeStakingContractProxy;
use mesh_native_staking_proxy::contract::multitest_utils::NativeStakingProxyContractProxy;
use mesh_sync::Tx::InFlightStaking;
use mesh_sync::{Tx, ValueRange};
use sylvia::multitest::App;

use crate::contract;
use crate::contract::multitest_utils::VaultContractProxy;
use crate::contract::test_utils::VaultApi;
use crate::error::ContractError;
use crate::msg::{
    AccountResponse, AllAccountsResponseItem, AllActiveExternalStakingResponse, LienResponse,
    StakingInitInfo,
};

const OSMO: &str = "OSMO";
const STAR: &str = "star";

/// 10% slashing on the remote chain
const SLASHING_PERCENTAGE: u64 = 10;

/// Test utils

/// App initialization
fn init_app(users: &[&str], amounts: &[u128]) -> App<MtApp> {
    let mut app = MtApp::new(|router, _api, storage| {
        for (&user, amount) in std::iter::zip(users, amounts) {
            router
                .bank
                .init_balance(storage, &Addr::unchecked(user), coins(*amount, OSMO))
                .unwrap();
        }
    });

    set_chain_native_denom(&mut app, OSMO);

    App::new(app)
}

fn add_local_validator(app: &mut App<MtApp>, validator: &str) {
    let block_info = app.block_info();
    app.app_mut()
        .init_modules(|router, api, storage| {
            router.staking.add_validator(
                api,
                storage,
                &block_info,
                Validator {
                    address: validator.to_string(),
                    commission: Decimal::zero(),
                    max_commission: Decimal::zero(),
                    max_change_rate: Decimal::zero(),
                },
            )
        })
        .unwrap();
}

fn set_chain_native_denom(app: &mut MtApp, denom: &str) {
    app.init_modules(|router, _, storage| {
        router.staking.setup(
            storage,
            StakingInfo {
                bonded_denom: denom.to_string(),
                ..Default::default()
            },
        )
    })
    .unwrap();
}

/// Contracts setup
fn setup<'app>(
    app: &'app App<MtApp>,
    owner: &str,
    slash_percent: u64,
    unbond_period: u64,
) -> (
    VaultContractProxy<'app, MtApp>,
    NativeStakingContractProxy<'app, MtApp>,
    ExternalStakingContractProxy<'app, MtApp>,
) {
    let (vault, native, external) = setup_inner(app, owner, slash_percent, unbond_period, true);
    (vault, native.unwrap(), external)
}

/// Contracts setup
fn setup_without_local_staking<'app>(
    app: &'app App<MtApp>,
    owner: &str,
    slash_percent: u64,
    unbond_period: u64,
) -> (
    VaultContractProxy<'app, MtApp>,
    ExternalStakingContractProxy<'app, MtApp>,
) {
    let (vault, _, external) = setup_inner(app, owner, slash_percent, unbond_period, false);
    (vault, external)
}

fn setup_inner<'app>(
    app: &'app App<MtApp>,
    owner: &str,
    slash_percent: u64,
    unbond_period: u64,
    local_staking: bool,
) -> (
    VaultContractProxy<'app, MtApp>,
    Option<NativeStakingContractProxy<'app, MtApp>>,
    ExternalStakingContractProxy<'app, MtApp>,
) {
    let vault_code = contract::multitest_utils::CodeId::store_code(app);

    let staking_init_info = if local_staking {
        let native_staking_code =
            mesh_native_staking::contract::multitest_utils::CodeId::store_code(app);
        let native_staking_proxy_code =
            mesh_native_staking_proxy::contract::multitest_utils::CodeId::store_code(app);

        let native_staking_inst_msg = mesh_native_staking::contract::InstantiateMsg {
            denom: OSMO.to_string(),
            slash_ratio_dsign: Decimal::percent(10),
            slash_ratio_offline: Decimal::percent(10),
            proxy_code_id: native_staking_proxy_code.code_id(),
        };

        Some(StakingInitInfo {
            admin: None,
            code_id: native_staking_code.code_id(),
            msg: to_binary(&native_staking_inst_msg).unwrap(),
            label: None,
        })
    } else {
        None
    };

    let vault = vault_code
        .instantiate(OSMO.to_owned(), staking_init_info)
        .with_label("Vault")
        .call(owner)
        .unwrap();

    let native_staking_addr = vault.config().unwrap().local_staking.map(Addr::unchecked);
    let native_staking = native_staking_addr.map(|addr| NativeStakingContractProxy::new(addr, app));

    let cross_staking = setup_cross_stake(app, owner, &vault, slash_percent, unbond_period);
    (vault, native_staking, cross_staking)
}

fn setup_cross_stake<'app>(
    app: &'app App<MtApp>,
    owner: &str,
    vault: &VaultContractProxy<'app, MtApp>,
    slash_percent: u64,
    unbond_period: u64,
) -> ExternalStakingContractProxy<'app, MtApp> {
    // FIXME: Code shouldn't be duplicated
    let cross_staking_code =
        mesh_external_staking::contract::multitest_utils::CodeId::store_code(app);
    // FIXME: Connection endpoint should be unique
    let remote_contact = AuthorizedEndpoint::new("connection-2", "wasm-osmo1foobarbaz");

    cross_staking_code
        .instantiate(
            OSMO.to_owned(),
            STAR.to_owned(),
            vault.contract_addr.to_string(),
            unbond_period,
            remote_contact,
            SlashRatio {
                double_sign: Decimal::percent(slash_percent),
                offline: Decimal::percent(slash_percent),
            },
        )
        .call(owner)
        .unwrap()
}

/// Set some active validators
fn set_active_validators(
    cross_staking: &ExternalStakingContractProxy<MtApp>,
    validators: &[&str],
) -> (u64, u64) {
    let update_valset_height = 100;
    let update_valset_time = 1234;

    for validator in validators {
        let activate = AddValidator::mock(validator);
        cross_staking
            .test_methods_proxy()
            .test_set_active_validator(activate.clone(), update_valset_height, update_valset_time)
            .call("test")
            .unwrap();
    }
    (update_valset_height, update_valset_time)
}

/// Bond some tokens
fn bond(vault: &VaultContractProxy<MtApp>, user: &str, amount: u128) {
    vault
        .bond()
        .with_funds(&coins(amount, OSMO))
        .call(user)
        .unwrap();
}

fn stake_locally(
    vault: &VaultContractProxy<MtApp>,
    user: &str,
    stake: u128,
    validator: &str,
) -> Result<cw_multi_test::AppResponse, ContractError> {
    let msg = mesh_native_staking::msg::StakeMsg {
        validator: validator.to_string(),
    };

    vault
        .stake_local(coin(stake, OSMO), to_binary(&msg).unwrap())
        .call(user)
}

fn stake_remotely(
    vault: &VaultContractProxy<MtApp>,
    cross_staking: &ExternalStakingContractProxy<MtApp>,
    user: &str,
    validators: &[&str],
    amounts: &[u128],
) {
    for (validator, amount) in std::iter::zip(validators, amounts) {
        vault
            .stake_remote(
                cross_staking.contract_addr.to_string(),
                coin(*amount, OSMO),
                to_binary(&ReceiveVirtualStake {
                    validator: validator.to_string(),
                })
                .unwrap(),
            )
            .call(user)
            .unwrap();

        // TODO: Hardcoded `external-staking`'s commit_stake call (lack of IBC support yet).
        // This should be through `IbcPacketAckMsg`
        let last_external_staking_tx =
            get_last_external_staking_pending_tx_id(cross_staking).unwrap();
        cross_staking
            .test_methods_proxy()
            .test_commit_stake(last_external_staking_tx)
            .call("test")
            .unwrap();
    }
}

fn proxy_for_user<'a>(
    local_staking: &NativeStakingContractProxy<MtApp>,
    user: &str,
    app: &'a App<MtApp>,
) -> NativeStakingProxyContractProxy<'a, MtApp> {
    let proxy_addr = local_staking
        .proxy_by_owner(user.to_string())
        .unwrap()
        .proxy;
    NativeStakingProxyContractProxy::new(Addr::unchecked(proxy_addr), app)
}

fn process_staking_unbondings(app: &App<MtApp>) {
    let mut block_info = app.block_info();
    block_info.time = block_info.time.plus_seconds(61);
    app.set_block(block_info);
    app.app_mut()
        .sudo(cw_multi_test::SudoMsg::Staking(
            cw_multi_test::StakingSudo::ProcessQueue {},
        ))
        .unwrap();
}

#[track_caller]
fn get_last_vault_pending_tx_id(contract: &VaultContractProxy<MtApp>) -> Option<u64> {
    let txs = contract.all_pending_txs_desc(None, None).unwrap().txs;
    txs.first().map(Tx::id)
}

#[track_caller]
fn get_last_external_staking_pending_tx_id(
    contract: &ExternalStakingContractProxy<MtApp>,
) -> Option<u64> {
    let txs = contract.all_pending_txs_desc(None, None).unwrap().txs;
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
fn instantiation() {
    let owner = "owner";

    let app = init_app(&[], &[]);
    let (vault, _, _) = setup(&app, owner, 0, 100);

    let config = vault.config().unwrap();
    assert_eq!(config.denom, OSMO);

    let users = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(users.accounts, []);
}

#[test]
fn bonding() {
    let owner = "owner";
    let user = "user1";

    let app = init_app(&[user], &[300]);

    let (vault, _local_staking, _cross_staking1) = setup(&app, owner, 0, 100);

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

    bond(&vault, user, 100);

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

    bond(&vault, user, 150);

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
fn local_staking_disabled() {
    let owner = "owner";
    let user = "user1";
    let local_val = "local";
    let remote_val = "remote";

    let mut app = init_app(&[user], &[300]);
    add_local_validator(&mut app, local_val);

    let (vault, cross_staking) = setup_without_local_staking(&app, owner, SLASHING_PERCENTAGE, 100);

    set_active_validators(&cross_staking, &[remote_val]);

    bond(&vault, user, 300);

    assert_eq!(
        stake_locally(&vault, user, 100, local_val).unwrap_err(),
        ContractError::NoLocalStaking
    );
    assert_eq!(vault.config().unwrap().local_staking, None);

    // cross-staking still works
    vault
        .stake_remote(
            cross_staking.contract_addr.to_string(),
            coin(100, OSMO),
            to_binary(&ReceiveVirtualStake {
                validator: remote_val.to_string(),
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
}

#[test]
fn stake_local() {
    let owner = "owner";
    let user = "user1";
    let val = "validator";

    let mut app = init_app(&[user], &[300]);
    add_local_validator(&mut app, val);

    let (vault, local_staking, _cross_staking1) = setup(&app, owner, SLASHING_PERCENTAGE, 100);

    bond(&vault, user, 300);

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
    stake_locally(&vault, user, 100, val).unwrap();

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

    stake_locally(&vault, user, 150, val).unwrap();

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

    // Cannot stake over collateral

    let err = stake_locally(&vault, user, 150, val).unwrap_err();

    assert_eq!(err, ContractError::InsufficentBalance);

    // Cannot unbond used collateral

    let err = vault.unbond(coin(100, OSMO)).call(user).unwrap_err();
    assert_eq!(
        err,
        ContractError::ClaimsLocked(ValueRange::new_val(Uint128::new(50)))
    );

    // Unstaking

    let proxy = proxy_for_user(&local_staking, user, &app);
    proxy
        .unstake(val.to_string(), coin(50, OSMO))
        .call(user)
        .unwrap();
    process_staking_unbondings(&app);
    proxy.release_unbonded().call(user).unwrap();

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

    proxy
        .unstake(val.to_string(), coin(100, OSMO))
        .call(user)
        .unwrap();
    process_staking_unbondings(&app);
    proxy.release_unbonded().call(user).unwrap();

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

    // Cannot unstake over the lien

    // TODO: catch subcall error here
    // let err = proxy
    //     .unstake(val.to_string(), coin(200, OSMO))
    //     .call(user)
    //     .unwrap_err();
    // assert_eq!(
    //     err,
    //     mesh_native_staking_proxy::error::ContractError::Unauthorized {}
    // );
}

#[test]
fn stake_cross() {
    let owner = "owner";
    let user = "user1";

    let app = init_app(&[user], &[300]);

    let unbond_period = 100;
    let (vault, _local_staking, cross_staking) =
        setup(&app, owner, SLASHING_PERCENTAGE, unbond_period);

    // Set active validator
    let validator = "validator";
    set_active_validators(&cross_staking, &[validator]);

    // Bond some tokens
    bond(&vault, user, 300);

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

    let res = vault.active_external_staking().unwrap();
    assert_eq!(res, AllActiveExternalStakingResponse { contracts: vec![] });

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

    let res = vault.active_external_staking().unwrap();
    assert_eq!(
        res,
        AllActiveExternalStakingResponse {
            contracts: vec![cross_staking.contract_addr.to_string()],
        }
    );

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

    // Unstake and receive callback through the IBC.
    // Wait for the unbonding period and withdraw unbonded tokens.
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

    skip_time(&app, unbond_period);

    cross_staking.withdraw_unbonded().call(user).unwrap();

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

    // Cannot unstake over the lien
    // Error not verified as it is swallowed by intermediate contract
    // in this scenario
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

    let app = init_app(&[user, user2], &[300, 500]);

    let unbond_period = 100;
    let (vault, _local_staking, cross_staking) =
        setup(&app, owner, SLASHING_PERCENTAGE, unbond_period);

    // Set active validator
    let validator = "validator";
    set_active_validators(&cross_staking, &[validator]);

    // Bond some tokens
    bond(&vault, user, 300);

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

    bond(&vault, user2, 500);
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
    let first_tx = get_last_vault_pending_tx_id(&vault).unwrap();

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
    let second_tx = get_last_vault_pending_tx_id(&vault).unwrap();

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
    let last_tx = get_last_vault_pending_tx_id(&vault).unwrap();
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

    let app = init_app(&[user], &[300]);

    let unbond_period = 100;
    let (vault, _local_staking, cross_staking) =
        setup(&app, owner, SLASHING_PERCENTAGE, unbond_period);

    // Set active validator
    let validator = "validator";
    set_active_validators(&cross_staking, &[validator]);

    // Bond some tokens
    bond(&vault, user, 300);

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
    let last_tx = get_last_vault_pending_tx_id(&vault).unwrap();
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
    assert_eq!(claims.claims, []);
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
    let local_validator = "local";

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let slashing_percentage: u64 = 60;
    let (vault, local_staking, cross_staking1) = setup(&app, owner, slashing_percentage, 100);

    let cross_staking2 = setup_cross_stake(&app, owner, &vault, slashing_percentage, 100);

    // Set active validator
    let validator = "validator";
    set_active_validators(&cross_staking1, &[validator]);
    set_active_validators(&cross_staking2, &[validator]);

    // Bond some tokens
    bond(&vault, user, 1000);

    // Stake properly, highest collateral usage is local staking (the 300 OSMO lien)
    //
    // When comparing `LienInfo`, for simplification assume their order to be in the order of
    // lienholder creation. It is guaranteed to work as MT assigns increasing names to the
    // contracts, and liens are queried ascending by the lienholder.

    stake_locally(&vault, user, 300, local_validator).unwrap();

    stake_remotely(&vault, &cross_staking1, user, &[validator], &[200]);

    stake_remotely(&vault, &cross_staking2, user, &[validator], &[100]);

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

    stake_remotely(&vault, &cross_staking1, user, &[validator], &[200]);

    stake_remotely(&vault, &cross_staking2, user, &[validator], &[400]);

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

    stake_remotely(&vault, &cross_staking1, user, &[validator], &[500]);

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
    let collaterals = [300, 300];

    let app = init_app(&users, &collaterals);

    let (vault, _, _) = setup(&app, owner, 0, 100);

    // No users should show up no matter of collateral flag

    let accounts = vault.all_accounts(false, None, None).unwrap();
    assert_eq!(accounts.accounts, []);

    let accounts = vault.all_accounts(true, None, None).unwrap();
    assert_eq!(accounts.accounts, []);

    // When user bond some collateral, he should be visible
    bond(&vault, users[0], 100);

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
    bond(&vault, users[1], 200);

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

/// Scenario 1:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/Slashing.md#scenario-1-slashed-delegator-has-free-collateral-on-the-vault
#[test]
fn cross_slash_scenario_1() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let local_validator = "local";
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];
    let validator2 = validators[1];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 190;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &validators, &[100, 50]);

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(collateral),
            free: ValueRange::new_val(Uint128::new(10)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(local_stake))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(34))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(collateral));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100)));
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50)));

    // Validator 1 is slashed
    cross_staking
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(10))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(local_stake))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(140))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(33))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(190));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stake
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(90))); // 10% slashed
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50))); // no slashing
}

/// Scenario 2:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/Slashing.md#scenario-2-slashed-delegator-has-no-free-collateral-on-the-vault
#[test]
fn cross_slash_scenario_2() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let local_validator = "local";
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 200;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &[validator1], &[200]);

    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(200))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(200))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(200)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(40))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(collateral));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(200)));

    // Validator 1 is slashed
    cross_staking
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(20))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(180))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(180))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(180)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(36))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(180));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(180))); // 10% slashed
}

/// Scenario 3:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/Slashing.md#scenario-3-slashed-delegator-has-some-free-collateral-on-the-vault
#[test]
fn cross_slash_scenario_3() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let local_validator = "local";
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 190;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &[validator1], &[150]);

    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(190))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(34))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(200));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(150)));

    // Validator 1 is slashed
    cross_staking
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(15))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(185))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(135))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(185)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(32 + 1)) // Because of rounding / truncation
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(185));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(135))); // 10% slashed
}

/// Scenario 4:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/Slashing.md#scenario-4-same-as-scenario-3-but-with-more-delegations
#[test]
fn cross_slash_scenario_4() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let local_validator = "local";
    let collateral = 200;
    let validators_1 = vec!["validator1", "validator2"];
    let validators_2 = vec!["validator3", "validator4"];
    let validator1 = validators_1[0];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking_1) = setup(&app, owner, slashing_percentage, 100);
    let cross_staking_2 = setup_cross_stake(&app, owner, &vault, slashing_percentage, 100);

    let (_, _) = set_active_validators(&cross_staking_1, &validators_1);
    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking_2, &validators_2);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 190;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking_1, user, &validators_1, &[140, 40]);
    stake_remotely(&vault, &cross_staking_2, user, &validators_2, &[100, 88]);

    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(190))
            },
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(180))
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(188))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(55)) // Because of truncation
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(200));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stake
    let cross_stake1 = cross_staking_1
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake1.stakes,
        [
            StakeInfo::new(user, validators_1[0], &Stake::from_amount(140u128.into())),
            StakeInfo::new(user, validators_1[1], &Stake::from_amount(40u128.into()))
        ]
    );

    let cross_stake2 = cross_staking_2
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake2.stakes,
        [
            StakeInfo::new(user, validators_2[0], &Stake::from_amount(100u128.into())),
            StakeInfo::new(user, validators_2[1], &Stake::from_amount(88u128.into()))
        ]
    );

    // Validator 1 is slashed
    cross_staking_1
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(14))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(186))
            },
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(166))
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(186))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(186)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(53 + 1)) // Because of rounding / truncation
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(186));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stake
    let cross_stake1 = cross_staking_1
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake1.stakes,
        [
            StakeInfo::new(user, validators_1[0], &Stake::from_amount(126u128.into())),
            StakeInfo::new(user, validators_1[1], &Stake::from_amount(40u128.into()))
        ]
    );

    // Considering external-staking slashing propagation
    let cross_stake2 = cross_staking_2
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake2.stakes,
        [
            StakeInfo::new(user, validators_2[0], &Stake::from_amount(99u128.into())),
            StakeInfo::new(user, validators_2[1], &Stake::from_amount(87u128.into()))
        ]
    );
}

/// Scenario 5:
/// https://github.com/osmosis-labs/mesh-security/blob/main/docs/ibc/Slashing.md#scenario-5-total-slashable-greater-than-max-lien
#[test]
fn cross_slash_scenario_5() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 50;
    let collateral = 200;
    let local_validator = "local";
    let validators = ["validator1", "validator2", "validator3"];
    let validator1 = validators[0];
    let validator2 = validators[1];
    let validator3 = validators[2];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking_1) = setup(&app, owner, slashing_percentage, 100);
    let cross_staking_2 = setup_cross_stake(&app, owner, &vault, slashing_percentage, 100);
    let cross_staking_3 = setup_cross_stake(&app, owner, &vault, slashing_percentage, 100);

    let (_, _) = set_active_validators(&cross_staking_1, &[validator1]);
    let (_, _) = set_active_validators(&cross_staking_2, &[validator2]);
    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking_3, &[validator3]);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 100;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking_1, user, &[validator1], &[180]);
    stake_remotely(&vault, &cross_staking_2, user, &[validator2], &[80]);
    stake_remotely(&vault, &cross_staking_3, user, &[validator3], &[100]);

    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(100))
            },
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(180))
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(80))
            },
            LienResponse {
                lienholder: cross_staking_3.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(100))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(180)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(190))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(200));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stake
    let cross_stake1 = cross_staking_1
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake1.stakes,
        [StakeInfo::new(
            user,
            validators[0],
            &Stake::from_amount(180u128.into())
        ),]
    );

    let cross_stake2 = cross_staking_2
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake2.stakes,
        [StakeInfo::new(
            user,
            validators[1],
            &Stake::from_amount(80u128.into())
        ),]
    );

    let cross_stake3 = cross_staking_3
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake3.stakes,
        [StakeInfo::new(
            user,
            validators[2],
            &Stake::from_amount(100u128.into())
        ),]
    );

    // Validator 1 is slashed
    cross_staking_1
        .test_methods_proxy()
        .test_handle_slashing(
            validator1.to_string(),
            Uint128::new(180) * Decimal::percent(slashing_percentage),
        )
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(78)) // Rounded down
            },
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(68)) // Rounded down
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(58)) // Rounded down
            },
            LienResponse {
                lienholder: cross_staking_3.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(78)) // Rounded down
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(78)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(110))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(110));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));

    // Cross stake
    // Considering external-staking slashing propagation
    let cross_stake1 = cross_staking_1
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake1.stakes,
        [StakeInfo::new(
            user,
            validators[0],
            &Stake::from_amount(68u128.into())
        ),]
    );

    let cross_stake2 = cross_staking_2
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake2.stakes,
        [StakeInfo::new(
            user,
            validators[1],
            &Stake::from_amount(58u128.into())
        ),]
    );

    let cross_stake3 = cross_staking_3
        .stakes(user.to_string(), None, None)
        .unwrap();
    assert_eq!(
        cross_stake3.stakes,
        [StakeInfo::new(
            user,
            validators[2],
            &Stake::from_amount(78u128.into())
        ),]
    );
}

/// Scenario 6:
/// Same as scenario 4 but with zero native staking
#[test]
fn cross_slash_no_native_staking() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let validators_1 = vec!["validator1", "validator2"];
    let validators_2 = vec!["validator3", "validator4"];
    let validator1 = validators_1[0];

    let app = init_app(&[user], &[collateral]);

    let (vault, _local_staking, cross_staking_1) = setup(&app, owner, slashing_percentage, 100);
    let cross_staking_2 = setup_cross_stake(&app, owner, &vault, slashing_percentage, 100);

    let (_, _) = set_active_validators(&cross_staking_1, &validators_1);
    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking_2, &validators_2);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking_1, user, &validators_1, &[140, 40]);
    stake_remotely(&vault, &cross_staking_2, user, &validators_2, &[100, 88]);

    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(180))
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(188))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(188)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(36)) // Because of truncation
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(200));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(12)));

    // Validator 1 is slashed
    cross_staking_1
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(14))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: cross_staking_1.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(166))
            },
            LienResponse {
                lienholder: cross_staking_2.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(186))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(186)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(35))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(186));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::zero()));
}

/// Checks that the slashing applies to unbonding amounts as well.
#[test]
fn cross_slash_pending_unbonding() {
    let owner = "owner";
    let user = "user1";
    // Remote slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];
    let validator2 = validators[1];
    let local_validator = "local";

    let mut app = init_app(&[user], &[collateral]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    let local_stake = 190;
    stake_locally(&vault, user, local_stake, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &validators, &[100, 50]);

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(collateral),
            free: ValueRange::new_val(Uint128::new(10)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(local_stake))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(34))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(collateral));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100)));
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50)));

    // Unbond half the stake of validator1
    cross_staking
        .unstake(validator1.to_owned(), coin(50, OSMO))
        .call(user)
        .unwrap();
    cross_staking
        .test_methods_proxy()
        .test_commit_unstake(get_last_external_staking_pending_tx_id(&cross_staking).unwrap())
        .call("test")
        .unwrap();
    // Cross stakes amount
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(50)));
    assert_eq!(cross_stake1.pending_unbonds[0].amount, Uint128::new(50));

    // Validator 1 is slashed, over the current bond
    cross_staking
        .test_methods_proxy()
        .test_handle_slashing(validator1.to_string(), Uint128::new(5))
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(local_stake))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(140)) // 10% slashed
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(33)) // Due to rounding
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(190));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(0)));

    // Cross stake
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(45))); // 10% slashed
                                                                           // Pending unbondings have been slashed
    assert_eq!(cross_stake1.pending_unbonds[0].amount, Uint128::new(45)); // 10% slashed
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50))); // no slashing
                                                                           // No pending unbondings
    assert!(cross_stake2.pending_unbonds.is_empty());
}

/// Scenario 7:
/// Same as scenario 1 but with native slashing, because of double-signing.
#[test]
fn native_slashing_tombstoning() {
    let owner = "owner";
    let user = "user1";
    // Native slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let local_validator = "local";
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];
    let validator2 = validators[1];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    stake_locally(&vault, user, 190, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &validators, &[100, 50]);

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(collateral),
            free: ValueRange::new_val(Uint128::new(10)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(190))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(34))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(collateral));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100)));
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50)));

    // Local validator is tombstoned (which implies slashing)
    local_staking
        .test_handle_jailing(vec![], vec![local_validator.to_string()])
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(171)) // 10% slashing
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(171))); // Adjusted
                                                                              // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(33))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(181));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stake
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100))); // no slashing
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50))); // no slashing
}

/// Scenario 8:
/// Same as scenario 1 but with native slashing, because of offline.
#[test]
fn native_slashing_jailing() {
    let owner = "owner";
    let user = "user1";
    // Native slashing percentage
    let slashing_percentage = 10;
    let collateral = 200;
    let local_validator = "local";
    let validators = vec!["validator1", "validator2"];
    let validator1 = validators[0];
    let validator2 = validators[1];

    let mut app = init_app(&[user], &[1000]);
    add_local_validator(&mut app, local_validator);

    let (vault, local_staking, cross_staking) = setup(&app, owner, slashing_percentage, 100);

    let (_update_valset_height, _update_valset_time) =
        set_active_validators(&cross_staking, &validators);

    // Bond some collateral
    bond(&vault, user, collateral);

    // Stake some tokens locally
    stake_locally(&vault, user, 190, local_validator).unwrap();

    // Stake some tokens remotely
    stake_remotely(&vault, &cross_staking, user, &validators, &[100, 50]);

    let acc = vault.account(user.to_owned()).unwrap();
    assert_eq!(
        acc,
        AccountResponse {
            denom: OSMO.to_owned(),
            bonded: Uint128::new(collateral),
            free: ValueRange::new_val(Uint128::new(10)),
        }
    );
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(190))
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(190)));
    // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(34))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(collateral));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stakes
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100)));
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50)));

    // Local validator is jailed (which implies slashing)
    local_staking
        .test_handle_jailing(vec![local_validator.to_string()], vec![])
        .call("test")
        .unwrap();

    // Liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [
            LienResponse {
                lienholder: local_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(171)) // 10% slashing
            },
            LienResponse {
                lienholder: cross_staking.contract_addr.to_string(),
                amount: ValueRange::new_val(Uint128::new(150))
            },
        ]
    );

    let acc_details = vault.account_details(user.to_owned()).unwrap();
    // Max lien
    assert_eq!(acc_details.max_lien, ValueRange::new_val(Uint128::new(171))); // Adjusted
                                                                              // Total slashable
    assert_eq!(
        acc_details.total_slashable,
        ValueRange::new_val(Uint128::new(33))
    );
    // Collateral
    assert_eq!(acc_details.bonded, Uint128::new(181));
    // Free collateral
    assert_eq!(acc_details.free, ValueRange::new_val(Uint128::new(10)));

    // Cross stake
    let cross_stake1 = cross_staking
        .stake(user.to_string(), validator1.to_string())
        .unwrap();
    assert_eq!(cross_stake1.stake, ValueRange::new_val(Uint128::new(100))); // no slashing
    let cross_stake2 = cross_staking
        .stake(user.to_string(), validator2.to_string())
        .unwrap();
    assert_eq!(cross_stake2.stake, ValueRange::new_val(Uint128::new(50))); // no slashing
}

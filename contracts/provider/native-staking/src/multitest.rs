use cosmwasm_std::{
    coin, coins, to_json_binary, Addr, Decimal, Delegation, StdError, Uint128, Validator,
};

use cw_multi_test::{App as MtApp, StakingInfo};
use sylvia::multitest::{App, Proxy};

use mesh_apis::local_staking_api::sv::mt::LocalStakingApiProxy;
use mesh_native_staking_proxy::contract::sv::mt::{
    CodeId as NativeStakingProxyCodeId, NativeStakingProxyContractProxy,
};
use mesh_native_staking_proxy::contract::NativeStakingProxyContract;
use mesh_sync::ValueRange;
use mesh_vault::mock::sv::mt::VaultMockProxy;
use mesh_vault::msg::LocalStakingInfo;

use crate::contract;
use crate::contract::sv::mt::NativeStakingContractProxy;
use crate::error::ContractError;
use crate::msg;
use crate::msg::{OwnerByProxyResponse, ProxyByOwnerResponse};

const OSMO: &str = "OSMO";

const SLASHING_PERCENTAGE_DSIGN: u64 = 15;
const SLASHING_PERCENTAGE_OFFLINE: u64 = 10;

fn slashing_rate_dsign() -> Decimal {
    Decimal::percent(SLASHING_PERCENTAGE_DSIGN)
}

fn slashing_rate_offline() -> Decimal {
    Decimal::percent(SLASHING_PERCENTAGE_OFFLINE)
}

fn app(balances: &[(&str, (u128, &str))], validators: &[&str]) -> App<MtApp> {
    let mut mt_app = MtApp::default();

    let block_info = mt_app.block_info();
    mt_app.init_modules(|router, api, storage| {
        for (account, (amount, denom)) in balances {
            router
                .bank
                .init_balance(storage, &Addr::unchecked(*account), coins(*amount, *denom))
                .unwrap();
        }

        router
            .staking
            .setup(
                storage,
                StakingInfo {
                    bonded_denom: OSMO.to_string(),
                    unbonding_time: 0,
                    ..Default::default()
                },
            )
            .unwrap();

        for validator in validators {
            router
                .staking
                .add_validator(
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
                .unwrap();
        }
    });

    App::new(mt_app)
}

#[track_caller]
fn assert_delegations(app: &App<MtApp>, delegator: impl Into<String>, expected: &[(&str, u128)]) {
    let mut expected = expected
        .iter()
        .map(|(val, amount)| (val.to_string(), *amount))
        .collect::<Vec<_>>();
    expected.sort();

    let mut queried = app
        .app()
        .wrap()
        .query_all_delegations(delegator)
        .unwrap()
        .into_iter()
        .map(
            |Delegation {
                 validator, amount, ..
             }| (validator, amount.amount.u128()),
        )
        .collect::<Vec<_>>();
    queried.sort();

    assert_eq!(expected, queried);
}

#[test]
fn instantiation() {
    let app = app(&[], &[]);

    let owner = "vault"; // Owner of the staking contract (i. e. the vault contract)

    let staking_proxy_code = NativeStakingProxyCodeId::store_code(&app);
    let staking_code = contract::sv::mt::CodeId::store_code(&app);

    let staking = staking_code
        .instantiate(
            OSMO.to_owned(),
            staking_proxy_code.code_id(),
            slashing_rate_dsign(),
            slashing_rate_offline(),
        )
        .with_label("Staking")
        .call(owner)
        .unwrap();

    let config = staking.config().unwrap();
    assert_eq!(config.denom, OSMO);

    let res = staking.max_slash().unwrap();
    assert_eq!(res.slash_ratio_dsign, slashing_rate_dsign());
}

#[test]
fn receiving_stake() {
    let owner = "vault"; // Owner of the staking contract (i. e. the vault contract)

    let user1 = "user1"; // One who wants to local stake
    let user2 = "user2"; // Another one who wants to local stake

    let validator = "validator1"; // Validator to stake on

    // Fund the vault
    let app = app(&[(owner, (300, OSMO))], &[validator]);

    // Contracts setup
    let staking_proxy_code = NativeStakingProxyCodeId::store_code(&app);
    let staking_code = contract::sv::mt::CodeId::store_code(&app);

    let staking = staking_code
        .instantiate(
            OSMO.to_owned(),
            staking_proxy_code.code_id(),
            slashing_rate_dsign(),
            slashing_rate_offline(),
        )
        .with_label("Staking")
        .call(owner)
        .unwrap();

    // Check that no proxy exists for user1 yet
    let err = staking.proxy_by_owner(user1.to_owned()).unwrap_err();
    assert!(matches!(
        err,
        ContractError::Std(StdError::GenericErr { .. }) // Addr not found
    ));

    // Receive some stake on behalf of user1 for validator
    let stake_msg = to_json_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .receive_stake(user1.to_owned(), stake_msg)
        .with_funds(&coins(100, OSMO))
        .call(owner) // called from vault
        .unwrap();

    let proxy1 = staking.proxy_by_owner(user1.to_owned()).unwrap().proxy;
    // Reverse query
    assert_eq!(
        staking.owner_by_proxy(proxy1.clone()).unwrap(),
        OwnerByProxyResponse {
            owner: user1.to_owned(),
        }
    );

    // Check that funds ended up with the staking module
    assert_delegations(&app, &proxy1, &[(validator, 100)]);

    // Stake some more
    let stake_msg = to_json_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .receive_stake(user1.to_owned(), stake_msg)
        .with_funds(&coins(50, OSMO))
        .call(owner) // called from vault
        .unwrap();

    // Check that same proxy is used
    assert_eq!(
        staking.proxy_by_owner(user1.to_owned()).unwrap(),
        ProxyByOwnerResponse {
            proxy: proxy1.clone(),
        }
    );

    // Reverse check
    assert_eq!(
        staking.owner_by_proxy(proxy1.clone()).unwrap(),
        OwnerByProxyResponse {
            owner: user1.to_owned(),
        }
    );

    // Check that funds are updated in the proxy contract
    assert_delegations(&app, &proxy1, &[(validator, 150)]);

    // Receive some stake on behalf of user2 for validator
    let stake_msg = to_json_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .receive_stake(user2.to_owned(), stake_msg)
        .with_funds(&coins(10, OSMO))
        .call(owner) // called from vault
        .unwrap();

    let proxy2 = staking.proxy_by_owner(user2.to_owned()).unwrap().proxy;
    // Reverse query
    assert_eq!(
        staking.owner_by_proxy(proxy2.to_string()).unwrap(),
        OwnerByProxyResponse {
            owner: user2.to_owned(),
        }
    );

    // Check that funds are in the corresponding proxy contract
    assert_delegations(&app, &proxy2, &[(validator, 10)]);
}

#[test]
fn releasing_proxy_stake() {
    let owner = "vault_admin"; // Owner of the vault contract

    let vault_addr = "contract0"; // First created contract
    let staking_addr = "contract1"; // Second contract (instantiated by vault)
    let proxy_addr = "contract2"; // Staking proxy contract for user1 (instantiated by staking contract on stake)

    let user = "user1"; // One who wants to release stake
    let validator = "validator1";

    // Fund the user
    let app = app(&[(user, (300, OSMO))], &[validator]);

    // Contracts setup
    let vault_code = mesh_vault::mock::sv::mt::CodeId::store_code(&app);
    let staking_code = contract::sv::mt::CodeId::store_code(&app);
    let staking_proxy_code = NativeStakingProxyCodeId::store_code(&app);

    // Instantiate vault msg
    let staking_init_info = mesh_vault::msg::StakingInitInfo {
        admin: None,
        code_id: staking_code.code_id(),
        msg: to_json_binary(&crate::contract::sv::InstantiateMsg {
            denom: OSMO.to_owned(),
            proxy_code_id: staking_proxy_code.code_id(),
            slash_ratio_dsign: slashing_rate_dsign(),
            slash_ratio_offline: slashing_rate_offline(),
        })
        .unwrap(),
        label: None,
    };

    // Instantiates vault and staking contracts
    let vault = vault_code
        .instantiate(
            OSMO.to_owned(),
            Some(LocalStakingInfo::New(staking_init_info)),
        )
        .with_label("Vault")
        .call(owner)
        .unwrap();

    // Vault is empty
    assert_eq!(
        app.app().wrap().query_balance(vault_addr, OSMO).unwrap(),
        coin(0, OSMO)
    );

    // Access staking instance
    let staking_proxy: Proxy<'_, MtApp, NativeStakingProxyContract<'_>> =
        Proxy::new(Addr::unchecked(proxy_addr), &app);

    // User bonds some funds to the vault
    vault
        .bond()
        .with_funds(&coins(200, OSMO))
        .call(user)
        .unwrap();

    // Vault has the funds
    assert_eq!(
        app.app().wrap().query_balance(vault_addr, OSMO).unwrap(),
        coin(200, OSMO)
    );

    // Stakes some of it locally, to validator. This instantiates the staking proxy contract for
    // user
    vault
        .stake_local(
            coin(100, OSMO),
            to_json_binary(&msg::StakeMsg {
                validator: validator.to_owned(),
            })
            .unwrap(),
        )
        .call(user)
        .unwrap();

    // Vault has half the funds
    assert_eq!(
        app.app().wrap().query_balance(vault_addr, OSMO).unwrap(),
        coin(100, OSMO)
    );

    // And a lien on the other half
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(
        claims.claims,
        [mesh_vault::msg::LienResponse {
            lienholder: staking_addr.to_owned(),
            amount: ValueRange::new_val(Uint128::new(100))
        }]
    );

    // The other half is delegated
    assert_delegations(&app, proxy_addr, &[(validator, 100)]);

    // Now release the funds
    staking_proxy
        .unstake(validator.to_string(), coin(100, OSMO))
        .call(user)
        .unwrap();
    // Important: we need to wait the unbonding period until this is released
    app.update_block(advance_unbonding_period);
    staking_proxy.release_unbonded().call(user).unwrap();

    // Check that the vault has the funds again
    assert_eq!(
        app.app().wrap().query_balance(vault_addr, OSMO).unwrap(),
        coin(200, OSMO)
    );
    // And there are no more liens
    let claims = vault.account_claims(user.to_owned(), None, None).unwrap();
    assert_eq!(claims.claims, []);
}

pub fn advance_unbonding_period(block: &mut cosmwasm_std::BlockInfo) {
    // Default unbonding time in cw_multi_test is 60, from looking at the code...
    // Wish I could find this somewhere in this setup somewhere.
    const UNBONDING_TIME: u64 = 60;
    block.time = block.time.plus_seconds(5 * UNBONDING_TIME);
    block.height += UNBONDING_TIME;
}

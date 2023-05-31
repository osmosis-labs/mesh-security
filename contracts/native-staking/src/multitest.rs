use cosmwasm_std::{coin, coins, to_binary, Addr, Decimal};

use cw_multi_test::App as MtApp;
use sylvia::multitest::App;

use local_staking_api::test_utils::LocalStakingApi;

mod local_staking_proxy;

use crate::contract;
use crate::local_staking_api;
use crate::msg;
use crate::msg::{OwnerByProxyResponse, ProxyByOwnerResponse};

const OSMO: &str = "OSMO";

#[test]
fn instantiation() {
    let app = App::default();

    let owner = "owner";

    let staking_proxy_code = local_staking_proxy::multitest_utils::CodeId::store_code(&app);
    let staking_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking = staking_code
        .instantiate(OSMO.to_owned(), staking_proxy_code.code_id())
        .with_label("Staking")
        .call(owner)
        .unwrap();

    let config = staking.config().unwrap();
    assert_eq!(config.denom, OSMO);

    let res = staking.local_staking_api_proxy().max_slash().unwrap();
    assert_eq!(res.max_slash, Decimal::percent(10));
}

#[test]
fn receiving_stake() {
    let owner = "vault"; // Owner of the staking contract (i. e. the vault contract)
    let user1 = "user1"; // One who wants to local stake
    let user2 = "user2"; // Another one who wants to local stake
    let validator = "validator1"; // Validator to stake on

    // Fund the vault
    let app = MtApp::new(|router, _api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(owner), coins(300, OSMO))
            .unwrap();
    });
    let app = App::new(app);

    // Contracts setup
    // let vault_code = mesh_vault::contract::multitest_utils::CodeId::store_code(&app);
    let staking_proxy_code = local_staking_proxy::multitest_utils::CodeId::store_code(&app);
    let staking_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking = staking_code
        .instantiate(OSMO.to_owned(), staking_proxy_code.code_id())
        .with_label("Staking")
        .call(owner)
        .unwrap();

    // Receive some stake on behalf of user1 for validator
    let stake_msg = to_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .local_staking_api_proxy()
        .receive_stake(user1.to_owned(), stake_msg)
        .with_funds(&coins(100, OSMO))
        .call(owner) // called from vault
        .unwrap();

    assert_eq!(
        staking.proxy_by_owner(user1.to_owned()).unwrap(),
        ProxyByOwnerResponse {
            proxy: "contract1".to_string(),
        }
    );

    assert_eq!(
        staking.owner_by_proxy("contract1".to_string()).unwrap(),
        OwnerByProxyResponse {
            owner: user1.to_owned(),
        }
    );

    // Check that funds are in the proxy contract
    assert_eq!(
        app.app().wrap().query_balance("contract1", OSMO).unwrap(),
        coin(100, OSMO)
    );

    // Stake some more
    let stake_msg = to_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .local_staking_api_proxy()
        .receive_stake(user1.to_owned(), stake_msg)
        .with_funds(&coins(50, OSMO))
        .call(owner) // called from vault
        .unwrap();

    // Check that same proxy is used
    assert_eq!(
        staking.proxy_by_owner(user1.to_owned()).unwrap(),
        ProxyByOwnerResponse {
            proxy: "contract1".to_string(),
        }
    );

    assert_eq!(
        staking.owner_by_proxy("contract1".to_string()).unwrap(),
        OwnerByProxyResponse {
            owner: user1.to_owned(),
        }
    );

    // Check that funds are updated in the proxy contract
    assert_eq!(
        app.app().wrap().query_balance("contract1", OSMO).unwrap(),
        coin(150, OSMO)
    );

    // Receive some stake on behalf of user2 for validator
    let stake_msg = to_binary(&msg::StakeMsg {
        validator: validator.to_owned(),
    })
    .unwrap();
    staking
        .local_staking_api_proxy()
        .receive_stake(user2.to_owned(), stake_msg)
        .with_funds(&coins(10, OSMO))
        .call(owner) // called from vault
        .unwrap();

    assert_eq!(
        staking.proxy_by_owner(user2.to_owned()).unwrap(),
        ProxyByOwnerResponse {
            proxy: "contract2".to_string(),
        }
    );

    assert_eq!(
        staking.owner_by_proxy("contract2".to_string()).unwrap(),
        OwnerByProxyResponse {
            owner: user2.to_owned(),
        }
    );

    // Check that funds are in the corresponding proxy contract
    assert_eq!(
        app.app().wrap().query_balance("contract2", OSMO).unwrap(),
        coin(10, OSMO)
    );
}

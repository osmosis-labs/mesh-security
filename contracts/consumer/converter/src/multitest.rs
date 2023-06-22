mod virtual_staking_mock;

use cosmwasm_std::{Addr, Decimal};

use sylvia::multitest::App;

use crate::contract;
// use crate::error::ContractError;
// use crate::msg;

const JUNO: &str = "ujuno";

#[test]
fn instantiation() {
    let app = App::default();

    let owner = "Sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "The man";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let price_feed_code =
        mesh_simple_price_feed::contract::multitest_utils::CodeId::store_code(&app);
    let virtual_staking_code = virtual_staking_mock::multitest_utils::CodeId::store_code(&app);
    let converter_code = contract::multitest_utils::CodeId::store_code(&app);

    let price_feed = price_feed_code
        .instantiate(native_per_foreign, None)
        .with_label("Price Feed")
        .call(owner)
        .unwrap();

    let converter = converter_code
        .instantiate(
            price_feed.contract_addr.to_string(),
            discount,
            JUNO.to_owned(),
            virtual_staking_code.code_id(),
        )
        .with_label("Juno Converter")
        .with_admin(admin)
        .call(owner)
        .unwrap();

    // check the config
    let config = converter.config().unwrap();
    assert_eq!(config.price_feed, price_feed.contract_addr.to_string());
    assert_eq!(config.adjustment, Decimal::percent(60));
    assert!(!config.virtual_staking.is_empty());

    // let's check we passed the admin here properly
    let vs_info = app
        .app()
        .wrap()
        .query_wasm_contract_info(&config.virtual_staking)
        .unwrap();
    assert_eq!(vs_info.admin, Some(admin.to_string()));

    // let's query virtual staking to find the owner
    let virtual_staking_addr = Addr::unchecked(&config.virtual_staking);
    let virtual_staking = virtual_staking_mock::multitest_utils::VirtualStakingMockProxy::new(
        virtual_staking_addr,
        &app,
    );
    let vs_config = virtual_staking.config().unwrap();
    assert_eq!(vs_config.converter, converter.contract_addr.to_string());
}

/*
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
    let staking_proxy_code = local_staking_proxy::multitest_utils::CodeId::store_code(&app);
    let staking_code = contract::multitest_utils::CodeId::store_code(&app);

    let staking = staking_code
        .instantiate(OSMO.to_owned(), staking_proxy_code.code_id())
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

    let proxy1 = staking.proxy_by_owner(user1.to_owned()).unwrap().proxy;
    // Reverse query
    assert_eq!(
        staking.owner_by_proxy(proxy1.clone()).unwrap(),
        OwnerByProxyResponse {
            owner: user1.to_owned(),
        }
    );

    // Check that funds are in the proxy contract
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(proxy1.clone(), OSMO)
            .unwrap(),
        coin(100, OSMO)
    );
}
*/

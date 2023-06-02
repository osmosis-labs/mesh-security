use cosmwasm_std::testing::mock_env;
use cosmwasm_std::{coin, coins, Addr, Decimal, Validator};
use cw_multi_test::App as MtApp;

use sylvia::multitest::App;

use crate::contract;
use crate::msg::ConfigResponse;

const DENOM: &str = "TOKEN"; // cw-multi-test v0.16.x does not support custom tokens yet

#[test]
fn instantiation() {
    let owner = "staking"; // The staking contract is the owner of the staking-proxy contracts
    let user = "user1"; // One who wants to local stake (uses the proxy)
    let validator = "validator1"; // Where to stake

    // Fund the staking contract, and add validator to staking keeper
    let block = mock_env().block;
    let app = MtApp::new(|router, api, storage| {
        router
            .bank
            .init_balance(storage, &Addr::unchecked(owner), coins(1000, DENOM))
            .unwrap();

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
    });
    let app = App::new(app);

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
    let proxy_addr = staking_proxy.contract_addr;
    assert_eq!(
        app.app()
            .wrap()
            .query_balance(proxy_addr.clone(), DENOM)
            .unwrap(),
        coin(0, DENOM)
    );

    // TODO: Check "side" effects: Staking msg emitted, data payload, etc.
}

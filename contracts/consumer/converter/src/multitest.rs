mod virtual_staking_mock;

use cosmwasm_std::{coin, Addr, Decimal, StdError, Uint128, Validator};
use cw_multi_test::App as MtApp;
use sylvia::multitest::App;

use crate::contract;
use crate::contract::test_utils::ConverterApi;
use crate::error::ContractError;
use crate::error::ContractError::Unauthorized;

const JUNO: &str = "ujuno";

struct SetupArgs<'a> {
    owner: &'a str,
    admin: &'a str,
    discount: Decimal,
    native_per_foreign: Decimal,
}

struct SetupResponse<'a> {
    price_feed:
        mesh_simple_price_feed::contract::multitest_utils::SimplePriceFeedContractProxy<'a, MtApp>,
    converter: contract::multitest_utils::ConverterContractProxy<'a, MtApp>,
    virtual_staking: virtual_staking_mock::multitest_utils::VirtualStakingMockProxy<'a, MtApp>,
}

fn setup<'a>(app: &'a App<MtApp>, args: SetupArgs<'a>) -> SetupResponse<'a> {
    let SetupArgs {
        owner,
        admin,
        discount,
        native_per_foreign,
    } = args;

    let price_feed_code =
        mesh_simple_price_feed::contract::multitest_utils::CodeId::store_code(app);
    let virtual_staking_code = virtual_staking_mock::multitest_utils::CodeId::store_code(app);
    let converter_code = contract::multitest_utils::CodeId::store_code(app);

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
            Some(admin.to_owned()),
        )
        .with_label("Juno Converter")
        .with_admin(admin)
        .call(owner)
        .unwrap();

    let config = converter.config().unwrap();
    let virtual_staking_addr = Addr::unchecked(config.virtual_staking);
    let virtual_staking = virtual_staking_mock::multitest_utils::VirtualStakingMockProxy::new(
        virtual_staking_addr,
        app,
    );

    SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    }
}

#[test]
fn instantiation() {
    let app = App::default();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

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
    let vs_config = virtual_staking.config().unwrap();
    assert_eq!(vs_config.converter, converter.contract_addr.to_string());
}

#[test]
fn ibc_stake_and_unstake() {
    let app = App::default();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // no one is staked
    let val1 = "Val Kilmer";
    let val2 = "Valley Girl";
    assert!(virtual_staking.all_stake().unwrap().stakes.is_empty());
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        0
    );

    // let's stake some
    converter
        .test_stake(val1.to_string(), coin(1000, JUNO))
        .call(owner)
        .unwrap();
    converter
        .test_stake(val2.to_string(), coin(4000, JUNO))
        .call(owner)
        .unwrap();

    // and unstake some
    converter
        .test_unstake(val2.to_string(), coin(2000, JUNO))
        .call(owner)
        .unwrap();

    // and check the stakes (1000 * 0.6 * 0.5 = 300) (2000 * 0.6 * 0.5 = 600)
    assert_eq!(
        virtual_staking
            .stake(val1.to_string())
            .unwrap()
            .stake
            .u128(),
        300
    );
    assert_eq!(
        virtual_staking
            .stake(val2.to_string())
            .unwrap()
            .stake
            .u128(),
        600
    );
    assert_eq!(
        virtual_staking.all_stake().unwrap().stakes,
        vec![
            (val1.to_string(), Uint128::new(300)),
            (val2.to_string(), Uint128::new(600)),
        ]
    );
}

#[test]
fn valset_update_works() {
    let app = App::default();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(10); // 1 OSMO worth of JUNO should give 0.9 OSMO of stake
    let native_per_foreign = Decimal::percent(40); // 1 JUNO is worth 0.4 OSMO

    let SetupResponse {
        price_feed: _,
        converter,
        virtual_staking,
    } = setup(
        &app,
        SetupArgs {
            owner,
            admin,
            discount,
            native_per_foreign,
        },
    );

    // Send a valset update
    let add_validators = vec![
        Validator {
            address: "validator1".to_string(),
            commission: Default::default(),
            max_commission: Default::default(),
            max_change_rate: Default::default(),
        },
        Validator {
            address: "validator3".to_string(),
            commission: Default::default(),
            max_commission: Default::default(),
            max_change_rate: Default::default(),
        },
    ];
    let rem_validators = vec!["validator3".to_string()];

    // Check that only the virtual staking contract can call this handler
    let res = converter
        .converter_api_proxy()
        .valset_update(vec![], vec![])
        .call(owner);
    assert_eq!(res.unwrap_err(), Unauthorized {});

    let res = converter
        .converter_api_proxy()
        .valset_update(add_validators, rem_validators)
        .call(virtual_staking.contract_addr.as_ref());

    // This fails because of lack of IBC support in mt now.
    // Cannot be tested further in this setup.
    // TODO: Change this when IBC support is there in mt.
    assert_eq!(
        res.unwrap_err(),
        ContractError::Std(StdError::NotFound {
            kind: "cosmwasm_std::ibc::IbcChannel".to_string()
        })
    );
}

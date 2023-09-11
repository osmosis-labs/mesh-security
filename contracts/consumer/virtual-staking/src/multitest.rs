use cosmwasm_std::{Addr, Decimal, Validator};
use cw_multi_test::App as MtApp;
use mesh_apis::virtual_staking_api::SudoMsg;
use sylvia::multitest::App;

use crate::contract;

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
    converter: mesh_converter::contract::multitest_utils::ConverterContractProxy<'a, MtApp>,
    virtual_staking: contract::multitest_utils::VirtualStakingContractProxy<'a, MtApp>,
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
    let virtual_staking_code = contract::multitest_utils::CodeId::store_code(app);
    let converter_code = mesh_converter::contract::multitest_utils::CodeId::store_code(app);

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
    let virtual_staking =
        contract::multitest_utils::VirtualStakingContractProxy::new(virtual_staking_addr, app);

    SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    }
}

// TODO: Redundant test. Remove it.
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
#[ignore] // FIXME: Enable / finish this test once custom query support is added to sylvia
fn valset_update_sudo() {
    let app = App::default();

    let owner = "sunny"; // Owner of the staking contract (i. e. the vault contract)
    let admin = "theman";
    let discount = Decimal::percent(40); // 1 OSMO worth of JUNO should give 0.6 OSMO of stake
    let native_per_foreign = Decimal::percent(50); // 1 JUNO is worth 0.5 OSMO

    let SetupResponse {
        price_feed: _,
        converter: _,
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

    // Send a valset update sudo message
    let adds = vec![
        Validator {
            address: "cosmosval3".to_string(),
            commission: Decimal::percent(3),
            max_commission: Decimal::percent(30),
            max_change_rate: Default::default(),
        },
        Validator {
            address: "cosmosval1".to_string(),
            commission: Decimal::percent(1),
            max_commission: Decimal::percent(10),
            max_change_rate: Default::default(),
        },
    ];
    let rems = vec![Validator {
        address: "cosmosval2".to_string(),
        commission: Decimal::percent(2),
        max_commission: Decimal::percent(20),
        max_change_rate: Default::default(),
    }];
    let tombs = vec![Validator {
        address: "cosmosval3".to_string(),
        commission: Decimal::percent(3),
        max_commission: Decimal::percent(30),
        max_change_rate: Default::default(),
    }];
    let msg = SudoMsg::ValsetUpdate {
        additions: adds,
        removals: rems,
        tombstones: tombs,
    };

    let res = app
        .app_mut()
        .wasm_sudo(virtual_staking.contract_addr, &msg)
        .unwrap();

    println!("res: {:?}", res);
}

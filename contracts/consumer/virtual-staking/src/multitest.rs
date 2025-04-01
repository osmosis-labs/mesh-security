use cosmwasm_std::{Addr, Decimal, Validator};
use cw_multi_test::{no_init, IntoBech32};
use mesh_apis::virtual_staking_api::sv::mt::VirtualStakingApiProxy;
use sylvia::multitest::Proxy;

use mesh_converter::contract::sv::mt::ConverterContractProxy;

use crate::contract;
use crate::contract::sv::mt::VirtualStakingContractProxy;

const JUNO: &str = "ujuno";

// Trying to figure out how to work with the generic types
type MtApp = cw_multi_test::BasicApp<
    mesh_bindings::VirtualStakeCustomMsg,
    mesh_bindings::VirtualStakeCustomQuery,
>;
type App = sylvia::multitest::App<MtApp>;

fn new_app() -> App {
    // Ideally there is a shorter way to do this
    App::new(cw_multi_test::custom_app(no_init))
}

struct SetupArgs<'a> {
    owner: &'a str,
    admin: &'a str,
    discount: Decimal,
    native_per_foreign: Decimal,
}

struct SetupResponse<'a> {
    price_feed: Proxy<'a, MtApp, mesh_simple_price_feed::contract::SimplePriceFeedContract>,
    converter: Proxy<'a, MtApp, mesh_converter::contract::ConverterContract>,
    virtual_staking: Proxy<'a, MtApp, contract::VirtualStakingContract>,
}

fn setup<'a>(app: &'a App, args: SetupArgs<'a>) -> SetupResponse<'a> {
    let SetupArgs {
        owner,
        admin,
        discount,
        native_per_foreign,
    } = args;

    let price_feed_code = mesh_simple_price_feed::contract::sv::mt::CodeId::store_code(app);
    let virtual_staking_code = contract::sv::mt::CodeId::store_code(app);
    let converter_code = mesh_converter::contract::sv::mt::CodeId::store_code(app);

    let price_feed = price_feed_code
        .instantiate(native_per_foreign, None)
        .with_label("Price Feed")
        .call(&owner.into_bech32())
        .unwrap();

    let converter = converter_code
        .instantiate(
            price_feed.contract_addr.to_string(),
            discount,
            JUNO.to_owned(),
            virtual_staking_code.code_id(),
            true,
            Some(admin.into_bech32().to_string()),
            50,
        )
        .with_label("Juno Converter")
        .with_admin(admin.into_bech32().as_str())
        .call(&owner.into_bech32())
        .unwrap();

    let config = converter.config().unwrap();
    let virtual_staking_addr = Addr::unchecked(config.virtual_staking);
    let virtual_staking = Proxy::new(virtual_staking_addr, app);

    SetupResponse {
        price_feed,
        converter,
        virtual_staking,
    }
}

#[test]
fn instantiation() {
    let app = new_app();

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
    assert_eq!(vs_info.admin, Some(admin.into_bech32()));

    // let's query virtual staking to find the owner
    let vs_config = virtual_staking.config().unwrap();
    assert_eq!(vs_config.converter, converter.contract_addr.to_string());
}

#[test]
// FIXME: Enable / finish this test once custom query support is added to sylvia
#[ignore = "IBC Messages not supported yet"]
fn valset_update_sudo() {
    let app = new_app();

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
        Validator::create(
            "cosmosval3".to_string(),
            Decimal::percent(3),
            Decimal::percent(30),
            Decimal::zero(),
        ),
        Validator::create(
            "cosmosval1".to_string(),
            Decimal::percent(1),
            Decimal::percent(10),
            Decimal::zero(),
        ),
    ];
    let rems = vec!["cosmosval2".to_string()];
    let tombs = vec!["cosmosval3".to_string()];

    let res = virtual_staking.handle_valset_update(
        Some(adds),
        Some(rems),
        None,
        None,
        None,
        Some(tombs),
        None,
    );
    println!("res: {:?}", res);
    res.unwrap();
}

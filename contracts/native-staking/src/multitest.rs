mod local_staking_proxy;

use crate::contract;

use sylvia::multitest::App;

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
}

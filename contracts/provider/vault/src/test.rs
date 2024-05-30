use cosmwasm_std::coin;
// use cw_orch::environment::IndexResponse;
use cw_orch::prelude::*;

use crate::orch::MeshVault;

use crate::contract::sv::{ExecMsgFns, InstantiateMsg, QueryMsgFns};

// TODO: shared variable
const BECH_PREFIX: &str = "osmo";

#[test]
fn happy_path_works() {
    let denom = "uosmo";
    let chain = MockBech32::new(BECH_PREFIX);
    chain
        .add_balance(&chain.sender(), vec![coin(1_000_000, denom)])
        .unwrap();

    let contract = MeshVault::new("vault", chain.clone());
    contract.upload().unwrap();
    let msg = InstantiateMsg {
        denom: denom.to_string(),
        local_staking: None,
    };
    contract.instantiate(&msg, None, None).unwrap();

    let cfg = contract.config().unwrap();
    println!("{:?}", cfg);

    let account = contract.account(chain.sender().into()).unwrap();
    assert_eq!(account.bonded.u128(), 0u128);

    contract.bond(&[coin(400_000, denom)]).unwrap();

    let account = contract.account(chain.sender().into()).unwrap();
    assert_eq!(account.bonded.u128(), 400_000u128);
}

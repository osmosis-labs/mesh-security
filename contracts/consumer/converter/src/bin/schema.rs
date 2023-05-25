use cosmwasm_schema::write_api;

use mesh_converter::contract::{ContractExecMsg, ContractQueryMsg, InstantiateMsg};

#[cfg(not(tarpaulin_include))]
fn main() {
    write_api! {
        instantiate: InstantiateMsg,
        execute: ContractExecMsg,
        query: ContractQueryMsg,
    }
}

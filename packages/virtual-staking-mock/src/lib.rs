use anyhow::Result as AnyResult;
use cosmwasm_std::{Addr, Api, Binary, BlockInfo, CustomQuery, Empty, Querier, Storage, Uint128};
use cw_multi_test::Module;
use cw_storage_plus::Map;
use mesh_bindings::{VirtualStakeCustomMsg, VirtualStakeCustomQuery};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

struct VirtualStakingModule<'a> {
    delegated: Map<'a, String, Uint128>,
}

impl Module for VirtualStakingModule<'_> {
    type ExecT = VirtualStakeCustomMsg;

    type QueryT = VirtualStakeCustomQuery;

    type SudoT = Empty;

    fn execute<ExecC, QueryC>(
        &self,
        api: &dyn Api,
        storage: &mut dyn Storage,
        router: &dyn cw_multi_test::CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
        block: &BlockInfo,
        sender: Addr,
        msg: Self::ExecT,
    ) -> AnyResult<cw_multi_test::AppResponse>
    where
        ExecC: std::fmt::Debug + Clone + PartialEq + JsonSchema + DeserializeOwned + 'static,
        QueryC: CustomQuery + DeserializeOwned + 'static,
    {
        todo!()
    }

    fn sudo<ExecC, QueryC>(
        &self,
        api: &dyn Api,
        storage: &mut dyn Storage,
        router: &dyn cw_multi_test::CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
        block: &BlockInfo,
        msg: Self::SudoT,
    ) -> AnyResult<cw_multi_test::AppResponse>
    where
        ExecC: std::fmt::Debug + Clone + PartialEq + JsonSchema + DeserializeOwned + 'static,
        QueryC: CustomQuery + DeserializeOwned + 'static,
    {
        panic!("sudo not implemented for")
    }

    fn query(
        &self,
        api: &dyn Api,
        storage: &dyn Storage,
        querier: &dyn Querier,
        block: &BlockInfo,
        request: Self::QueryT,
    ) -> AnyResult<Binary> {
        todo!()
    }
}

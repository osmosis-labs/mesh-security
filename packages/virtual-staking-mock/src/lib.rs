use anyhow::Result as AnyResult;
use cosmwasm_std::{
    coin, testing::{MockApi, MockStorage}, to_json_binary, Addr, AllDelegationsResponse, Api, Binary, BlockInfo, CustomQuery, Empty, Querier, QuerierWrapper, Storage, Uint128
};
use cw_multi_test::{AppResponse, BankKeeper, Module, WasmKeeper};
use cw_storage_plus::{Item, Map};
use mesh_bindings::{
    BondStatusResponse, SlashRatioResponse, VirtualStakeCustomMsg, VirtualStakeCustomQuery,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;

pub type App = cw_multi_test::App<
    BankKeeper,
    MockApi,
    MockStorage,
    VirtualStakingModule,
    WasmKeeper<VirtualStakeCustomMsg, VirtualStakeCustomQuery>,
>;

pub struct VirtualStakingModule {
    /// virtual-staking contract -> max cap
    caps: Map<'static, Addr, Uint128>,
    /// (virtual-staking contract, validator) -> bonded amount
    bonds: Map<'static, (Addr, Addr), Uint128>,
    slash_ratio: Item<'static, SlashRatioResponse>,
}

impl VirtualStakingModule {
    pub fn new() -> Self {
        Self {
            caps: Map::new("virtual_staking_caps"),
            bonds: Map::new("virtual_staking_bonds"),
            slash_ratio: Item::new("virtual_staking_slash_ratios"),
        }
    }

    pub fn init_slash_ratios(
        &self,
        storage: &mut dyn Storage,
        slash_for_downtime: impl Into<String>,
        slash_for_double_sign: impl Into<String>,
    ) -> AnyResult<()> {
        self.slash_ratio.save(
            storage,
            &SlashRatioResponse {
                slash_fraction_downtime: slash_for_downtime.into(),
                slash_fraction_double_sign: slash_for_double_sign.into(),
            },
        )?;

        Ok(())
    }

    fn bonded_for_contract(&self, storage: &dyn Storage, contract: Addr) -> AnyResult<Uint128> {
        Ok(self
            .bonds
            .range(storage, None, None, cosmwasm_std::Order::Ascending)
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|((c, _), amt)| if c == contract { Some(amt) } else { None })
            .sum())
    }
}

impl Default for VirtualStakingModule {
    fn default() -> Self {
        Self::new()
    }
}

impl Module for VirtualStakingModule {
    type ExecT = VirtualStakeCustomMsg;

    type QueryT = VirtualStakeCustomQuery;

    type SudoT = Empty;

    fn execute<ExecC, QueryC>(
        &self,
        _api: &dyn Api,
        storage: &mut dyn Storage,
        _router: &dyn cw_multi_test::CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
        _block: &BlockInfo,
        sender: Addr,
        msg: Self::ExecT,
    ) -> AnyResult<cw_multi_test::AppResponse>
    where
        ExecC: std::fmt::Debug + Clone + PartialEq + JsonSchema + DeserializeOwned + 'static,
        QueryC: CustomQuery + DeserializeOwned + 'static,
    {
        let VirtualStakeCustomMsg::VirtualStake(msg) = msg;

        let cap = self.caps.load(storage, sender.clone())?;

        match msg {
            mesh_bindings::VirtualStakeMsg::Bond { amount, validator } => {
                let all_bonded = self.bonded_for_contract(storage, sender.clone())?;

                if all_bonded + amount.amount <= cap {
                    let current_bonded = self
                        .bonds
                        .may_load(storage, (sender.clone(), Addr::unchecked(&validator)))?
                        .unwrap_or(Uint128::zero());

                    self.bonds.save(
                        storage,
                        (sender, Addr::unchecked(validator)),
                        &(current_bonded + amount.amount),
                    )?;

                    Ok(AppResponse::default())
                } else {
                    Err(anyhow::anyhow!("cap exceeded"))
                }
            }
            mesh_bindings::VirtualStakeMsg::Unbond { amount, validator } => {
                let current_bonded = self
                    .bonds
                    .may_load(storage, (sender.clone(), Addr::unchecked(&validator)))?
                    .unwrap_or(Uint128::zero());

                if current_bonded - amount.amount >= Uint128::zero() {
                    self.bonds.save(
                        storage,
                        (sender, Addr::unchecked(validator)),
                        &(current_bonded - amount.amount),
                    )?;

                    Ok(AppResponse::default())
                } else {
                    Err(anyhow::anyhow!("bonded amount exceeded"))
                }
            }
        }
    }

    fn sudo<ExecC, QueryC>(
        &self,
        _api: &dyn Api,
        _storage: &mut dyn Storage,
        _router: &dyn cw_multi_test::CosmosRouter<ExecC = ExecC, QueryC = QueryC>,
        _block: &BlockInfo,
        _msg: Self::SudoT,
    ) -> AnyResult<cw_multi_test::AppResponse>
    where
        ExecC: std::fmt::Debug + Clone + PartialEq + JsonSchema + DeserializeOwned + 'static,
        QueryC: CustomQuery + DeserializeOwned + 'static,
    {
        Err(anyhow::anyhow!(
            "sudo not implemented for the virtual staking module"
        ))
    }

    fn query(
        &self,
        _api: &dyn Api,
        storage: &dyn Storage,
        querier: &dyn Querier,
        _block: &BlockInfo,
        request: Self::QueryT,
    ) -> AnyResult<Binary> {
        let VirtualStakeCustomQuery::VirtualStake(query) = request;

        let result = match query {
            mesh_bindings::VirtualStakeQuery::BondStatus { contract } => {
                let denom =
                    QuerierWrapper::<VirtualStakeCustomQuery>::new(querier).query_bonded_denom()?;

                let cap = self.caps.load(storage, Addr::unchecked(&contract))?;
                let bonded = self.bonded_for_contract(storage, Addr::unchecked(contract))?;

                to_json_binary(&BondStatusResponse {
                    cap: coin(cap.u128(), &denom),
                    delegated: coin(bonded.u128(), denom),
                })?
            }
            mesh_bindings::VirtualStakeQuery::SlashRatio {} => {
                to_json_binary(&self.slash_ratio.load(storage)?)?
            }
            mesh_bindings::VirtualStakeQuery::AllDelegations { .. } => {
                to_json_binary(&AllDelegationsResponse {
                    delegations: vec![]
                })?
            }
        };

        Ok(to_json_binary(&result)?)
    }
}

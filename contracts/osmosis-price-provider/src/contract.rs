use std::str::FromStr;

use cosmwasm_std::{Decimal, DepsMut, IbcChannel, Response};
use cw2::set_contract_version;
use cw_storage_plus::Item;
use cw_utils::nonpayable;
use osmosis_std::types::osmosis::twap::v1beta1::TwapQuerier;
use sylvia::types::InstantiateCtx;
use sylvia::{contract, schemars};

use crate::error::ContractError;
use crate::state::Config;

pub const CONTRACT_NAME: &str = env!("CARGO_PKG_NAME");
pub const CONTRACT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub struct OsmosisPriceProvider {
    config: Item<'static, Config>,
    pub(crate) channels: Item<'static, Vec<IbcChannel>>,
}

#[cfg_attr(not(feature = "library"), sylvia::entry_points)]
#[contract]
#[sv::error(ContractError)]
impl OsmosisPriceProvider {
    pub const fn new() -> Self {
        Self {
            config: Item::new("config"),
            channels: Item::new("channels"),
        }
    }

    #[sv::msg(instantiate)]
    pub fn instantiate(
        &self,
        ctx: InstantiateCtx,
        admin: String,
    ) -> Result<Response, ContractError> {
        nonpayable(&ctx.info)?;

        let admin = ctx.deps.api.addr_validate(&admin)?;
        let config = Config { admin };
        self.config.save(ctx.deps.storage, &config)?;
        self.channels.save(ctx.deps.storage, &vec![])?;

        set_contract_version(ctx.deps.storage, CONTRACT_NAME, CONTRACT_VERSION)?;

        Ok(Response::new())
    }

    pub(crate) fn query_twap(
        &self,
        deps: DepsMut,
        pool_id: u64,
        base: impl Into<String>,
        quote: impl Into<String>,
    ) -> Result<Decimal, ContractError> {
        let querier = TwapQuerier::new(&deps.querier);

        Decimal::from_str(
            &querier
                .arithmetic_twap_to_now(pool_id, base.into(), quote.into(), None)?
                .arithmetic_twap,
        )
        .map_err(Into::into)
    }
}

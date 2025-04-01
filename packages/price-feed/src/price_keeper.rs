use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Decimal, Deps, DepsMut, Env, Timestamp};
use cw_storage_plus::Item;

#[cw_serde]
pub struct PriceInfo {
    pub time: Timestamp,
    pub native_per_foreign: Decimal,
}

/// A component that keeps track of the latest price info.
pub struct PriceKeeper {
    pub price_info: Item<PriceInfo>,
    pub price_info_ttl_in_secs: Item<u64>,
}

impl PriceKeeper {
    pub const fn new() -> Self {
        Self {
            price_info: Item::new("price"),
            price_info_ttl_in_secs: Item::new("price_ttl"),
        }
    }

    pub fn init(
        &self,
        deps: &mut DepsMut,
        price_info_ttl_in_secs: u64,
    ) -> Result<(), PriceKeeperError> {
        self.price_info_ttl_in_secs
            .save(deps.storage, &price_info_ttl_in_secs)?;
        Ok(())
    }

    pub fn update(
        &self,
        deps: DepsMut,
        time: Timestamp,
        twap: Decimal,
    ) -> Result<(), PriceKeeperError> {
        let old = self.price_info.may_load(deps.storage)?;
        match old {
            Some(old) if old.time > time => {
                // don't update if we have newer price info stored
            }
            _ => self.price_info.save(
                deps.storage,
                &PriceInfo {
                    time,
                    native_per_foreign: twap,
                },
            )?,
        }

        Ok(())
    }

    pub fn price(&self, deps: Deps, env: &Env) -> Result<Decimal, PriceKeeperError> {
        let price_info_ttl = self.price_info_ttl_in_secs.load(deps.storage)?;
        let price_info = self
            .price_info
            .may_load(deps.storage)?
            .ok_or(PriceKeeperError::NoPriceData)?;

        if env.block.time.minus_seconds(price_info_ttl) < price_info.time {
            Ok(price_info.native_per_foreign)
        } else {
            Err(PriceKeeperError::OutdatedPriceData)
        }
    }
}

impl Default for PriceKeeper {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(thiserror::Error, Debug, PartialEq)]
pub enum PriceKeeperError {
    #[error("StdError: {0}")]
    StdError(#[from] cosmwasm_std::StdError),

    #[error("Price data is outdated")]
    OutdatedPriceData,

    #[error("No price data available")]
    NoPriceData,
}

#[cfg(test)]
mod tests {
    use super::*;

    use cosmwasm_std::testing::{mock_dependencies, mock_env};

    #[test]
    fn happy_path() {
        let mut deps = mock_dependencies();
        let mut env = mock_env();
        let keeper = PriceKeeper::new();

        keeper.init(&mut deps.as_mut(), 600).unwrap();
        keeper
            .update(deps.as_mut(), env.block.time, Decimal::one())
            .unwrap();

        let price = keeper.price(deps.as_ref(), &env).unwrap();
        assert_eq!(price, Decimal::one());

        env.block.time = env.block.time.plus_seconds(559);
        let price = keeper.price(deps.as_ref(), &env).unwrap();
        assert_eq!(price, Decimal::one());
    }

    #[test]
    fn no_initial_price_info() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let keeper = PriceKeeper::new();

        keeper.init(&mut deps.as_mut(), 600).unwrap();

        let err = keeper.price(deps.as_ref(), &env).unwrap_err();
        assert_eq!(err, PriceKeeperError::NoPriceData);
    }

    #[test]
    fn outdated_price_info() {
        let mut deps = mock_dependencies();
        let mut env = mock_env();
        let keeper = PriceKeeper::new();

        keeper.init(&mut deps.as_mut(), 600).unwrap();
        keeper
            .update(deps.as_mut(), env.block.time, Decimal::one())
            .unwrap();

        env.block.time = env.block.time.plus_seconds(601);
        let err = keeper.price(deps.as_ref(), &env).unwrap_err();
        assert_eq!(err, PriceKeeperError::OutdatedPriceData);
    }

    #[test]
    fn update_with_older_price_info_is_ignored() {
        let mut deps = mock_dependencies();
        let env = mock_env();
        let keeper = PriceKeeper::new();

        keeper.init(&mut deps.as_mut(), 600).unwrap();
        keeper
            .update(deps.as_mut(), env.block.time, Decimal::one())
            .unwrap();
        keeper
            .update(
                deps.as_mut(),
                env.block.time.minus_seconds(1),
                Decimal::percent(50),
            )
            .unwrap();

        let price = keeper.price(deps.as_ref(), &env).unwrap();
        assert_eq!(price, Decimal::one());
    }
}

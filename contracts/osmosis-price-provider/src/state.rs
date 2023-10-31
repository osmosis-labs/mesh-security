use std::collections::HashMap;

use cosmwasm_schema::cw_serde;
use cosmwasm_std::{Addr, IbcChannel, StdError, Storage};
use cw_storage_plus::Item;

use crate::error::ContractError;

#[cw_serde]
pub struct Config {
    pub admin: Addr,
}

pub struct Subscriptions {
    by_denom: Item<'static, HashMap<String, Subscription>>,
    inactive: Item<'static, Vec<IbcChannel>>,
}

impl Subscriptions {
    pub(crate) const fn new() -> Self {
        Self {
            by_denom: Item::new("subscriptions_by_denom"),
            inactive: Item::new("subscriptions_inactive"),
        }
    }

    pub(crate) fn init(&self, storage: &mut dyn Storage) -> Result<(), StdError> {
        self.by_denom.save(storage, &HashMap::new())?;
        self.inactive.save(storage, &vec![])?;

        Ok(())
    }

    pub(crate) fn register_channel(
        &self,
        storage: &mut dyn Storage,
        channel: IbcChannel,
    ) -> Result<(), ContractError> {
        self.inactive.update(storage, |mut v| {
            if v.iter().find(|c| **c == channel).is_some() {
                Err(ContractError::IbcChannelAlreadyOpen)
            } else {
                v.push(channel);
                Ok(v)
            }
        })?;

        Ok(())
    }

    pub(crate) fn bind_channel(
        &self,
        storage: &mut dyn Storage,
        channel: IbcChannel,
        denom: String,
        pool_id: u64,
    ) -> Result<(), ContractError> {
        self.inactive.update(storage, |mut v| {
            if let Some((ix, _)) = v.iter().enumerate().find(|(_, c)| **c == channel) {
                v.remove(ix);
                Ok(v)
            } else {
                Err(ContractError::IbcChannelNotOpen)
            }
        })?;

        self.by_denom.update(storage, |mut map| {
            map.insert(denom, Subscription { channel, pool_id })
                .is_none()
                .then_some(map)
                .ok_or(ContractError::SubscriptionAlreadyExists)
        })?;

        Ok(())
    }

    pub(crate) fn subs(
        &self,
        storage: &mut dyn Storage,
    ) -> Result<impl Iterator<Item = (String, Subscription)>, ContractError> {
        let list = self.by_denom.load(storage)?;

        Ok(list.into_iter())
    }

    pub(crate) fn remove_channel(
        &self,
        _storage: &mut dyn Storage,
        _channel: &IbcChannel,
    ) -> Result<(), ContractError> {
        todo!()
    }
}

#[cw_serde]
pub struct Subscription {
    pub channel: IbcChannel,
    pub pool_id: u64,
}

use cosmwasm_schema::cw_serde;
use cosmwasm_std::IbcEndpoint;
use cosmwasm_std::{Coin, Uint256, Uint64};
use cw_storage_plus::{Item, Map};

#[cw_serde]
pub struct Config {
    pub client_id: String,
    pub oracle_script_id: Uint64,
    pub ask_count: Uint64,
    pub min_count: Uint64,
    pub fee_limit: Vec<Coin>,
    pub prepare_gas: Uint64,
    pub execute_gas: Uint64,
    pub minimum_sources: u8,
}

#[cw_serde]
pub struct Rate {
    // Rate of an asset relative to USD
    pub rate: Uint64,
    // The resolve time of the request ID
    pub resolve_time: Uint64,
    // The request ID where the rate was derived from
    pub request_id: Uint64,
}

impl Rate {
    pub fn new(rate: Uint64, resolve_time: Uint64, request_id: Uint64) -> Self {
        Rate {
            rate,
            resolve_time,
            request_id,
        }
    }
}

pub const RATES: Map<&str, Rate> = Map::new("rates");

pub const ENDPOINT: Item<IbcEndpoint> = Item::new("endpoint");

pub const BAND_CONFIG: Item<Config> = Item::new("config");

#[cw_serde]
pub struct ReferenceData {
    // Pair rate e.g. rate of BTC/USD
    pub rate: Uint256,
    // Unix time of when the base asset was last updated. e.g. Last update time of BTC in Unix time
    pub last_updated_base: Uint64,
    // Unix time of when the quote asset was last updated. e.g. Last update time of USD in Unix time
    pub last_updated_quote: Uint64,
}

impl ReferenceData {
    pub fn new(rate: Uint256, last_updated_base: Uint64, last_updated_quote: Uint64) -> Self {
        ReferenceData {
            rate,
            last_updated_base,
            last_updated_quote,
        }
    }
}

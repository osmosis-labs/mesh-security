# Price feed contract

## Overview

This contract is a demo implementation for how to request data on BandChain through IBC with CosmWasm.
The contract shown here is still under development and is **NOT** intended for production use.

## Build

### Contract

To compile all contracts, run the following script in the repo root: `/scripts/build_artifacts.sh` or the command below:
The optimized wasm code and its checksums can be found in the `/artifacts` directory

```
docker run --rm -v "$(pwd)":/code \
  --mount type=volume,source="$(basename "$(pwd)")_cache",target=/code/target \
  --mount type=volume,source=registry_cache,target=/usr/local/cargo/registry \
  cosmwasm/workspace-optimizer:0.12.7
```

### Schema

To generate the JSON schema files for the contract call, queries and query responses, run the following script in the
repo root: `/scripts/build_schemas.sh` or run `cargo schema` in the smart contract directory.

## Messages

### Instantiate message

This contract accepts the following during instantiation

```rust
#[cw_serde]
pub struct InstantiateMsg {
    // A unique ID for the oracle request
    pub client_id: String,
    // The oracle script ID to query
    pub oracle_script_id: Uint64,
    // The number of validators that are requested to respond
    pub ask_count: Uint64,
    // The minimum number of validators that need to respond
    pub min_count: Uint64,
    // The maximum amount of band in uband to be paid to the data source providers
    // e.g. vec![Coin::new(100, "uband")]
    pub fee_limit: Vec<Coin>,
    // Amount of gas to pay to prepare raw requests
    pub prepare_gas: Uint64,
    // Amount of gas reserved for execution
    pub execute_gas: Uint64,
    // Minimum number of sources required to return a successful response
    pub minimum_sources: u8,
}
```

### Execute message

The contract contains the following execute messages:

```rust
pub enum ExecuteMsg {
    Request { symbols: Vec<String> },
}
```

Where `request()` takes a set of symbols to request on BandChain using the specified parameters contained in `Config`and
upon receiving a packet from BandChain, will store the returned data which can be queried.

An example message can be seen below:

```json
{
    "request": {
        "symbol": [
            "BTC",
            "ETH",
            "BAND"
        ]
    }
}
```

### Query message

The contract contains the following query messages:

```rust
pub enum QueryMsg {
    // Returns the ReferenceData of a given asset pairing
    GetReferenceData {
        // Symbol pair to query where:
        // symbol_pair := (base_symbol, quote_symbol)
        // e.g. BTC/USD ≡ ("BTC", "USD")
        symbol_pair: (String, String),
    },
    // Returns the ReferenceDatas of the given asset pairings
    GetReferenceDataBulk {
        // Vector of Symbol pair to query
        // e.g. <BTC/USD ETH/USD, BAND/BTC> ≡ <("BTC", "USD"), ("ETH", "USD"), ("BAND", "BTC")>
        symbol_pairs: Vec<(String, String)>,
    },
}
```

All queries in the contract will retrieve return the stored data from the `request()` execute function.

### ReferenceData

`ReferenceData` is the struct that is returned when querying with `GetReferenceData` or `GetReferenceDataBulk` where the
bulk variant returns `Vec<ReferenceData>`

`ReferenceData` is defined as:

```rust
pub struct ReferenceData {
    // Pair rate e.g. rate of BTC/USD
    pub rate: Uint256,
    // Unix time of when the base asset was last updated. e.g. Last update time of BTC in Unix time
    pub last_updated_base: Uint64,
    // Unix time of when the quote asset was last updated. e.g. Last update time of USD in Unix time
    pub last_updated_quote: Uint64,
}
```

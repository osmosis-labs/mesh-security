# Remote Price Feed

This implements the [price feed API](../../../packages/apis/src/price_feed_api.rs).

This contract provides exchange rate data by contacting an [Osmosis Price Provider](../osmosis-price-provider).

A single trading pair has to be configured on instantiation, along with the IBC endpoint. In case multiple trading pairs need to be synced, multiple contracts must be deployed.

For this contract to work correctly:

- An IBC connection to [Osmosis Price Provider](../osmosis-price-provider) must be opened.
- A`SudoMsg::EndBlock {}` must be sent to the contract regularly. This will allow the contract to request regular updates of locally stored data.

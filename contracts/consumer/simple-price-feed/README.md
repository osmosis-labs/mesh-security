# Simple Price Feed

This is the simplest usable implementation of the [price feed API](../../../packages/apis/src/price_feed_api.rs). 
One address is set to be the owner of the price feed, and only the owner can update the price.

The main usage is for tests, where we can set arbitrary prices for assets with a simple call.
However, if we set the owner to the x/gov module address, this could be used for assets where no on-chain data
is available.

It is intended to be used as a reference implementation for other price feed implementations.
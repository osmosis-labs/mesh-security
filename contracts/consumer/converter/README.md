# Token Converter

This contract runs on the consumer side and will communicate with the 
external staker contract on the provider via IBC messages. It has
the following responsibilities:

* Receive packets on bonding/unbonding from the provider, normalize them
  to local tokens, and send the requests to the virtual staking contract.
* Receive reward payments from the virtual staking contract, and transmit
  that over IBC.

Basically it is the IBC communication component on the consumer side without
too much logic.

It is bound 1-to-1 with a virtual staking contract, and requires a price feed
contract to provide valid price data.

**TODO** More details on IBC comms
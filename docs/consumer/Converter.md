# Stake Converter

The Stake Converter is on the consumer side and is connected to an External Staker on the Provider side.
This handles the normalization of the external tokens and _converts_ them into "Virtual Stake".
There is a 1:1 connection between a Converter and a [Virtual Staking Contract](./VirtualStaking.md)
which handles the actual issuance.

The converter is connected to the Provider chain via IBC and handles the various packets coming from it.

## Setup

We must first instantiate a Price Feed contract(see [Price Normalization / Price Feeds](#price-feeds) below),
then the Converter contract.
The Converter will then instantiate a Virtual Staking contract to work with it,
as they both need references to each other.
In addition to a reference to the Price Feed contract and the code id of the Virtual Staking contract,
the Converter also needs the discount ratio, the remote denomination, and the admin of the Virtual Staking
contract, which is taken as an explicit argument, and normally will be the same admin of the Converter.

TODO: Update IBC Setup

When we [deploy the contracts](../ibc/Overview.md#deployment), we connect the Converter on the Consumer
chain with an [External Staking](../provider/ExternalStaking.md) contract on the Provider. Once this
connection is established, Consumer governance can authorize this Converter with some ability to mint
on the [Virtual Staking](./VirtualStaking.md) contract.

## Staking Flow

Once the connection is established, the Provider can send various "virtual stake" messages to the Converter,
which is responsible for processing them and normalizing amounts for the local "virtual staking" module. These
packets are sent via a dedicated channel between the Provider chain and the Consumer chain, to ensure
that there are no other security assumptions (3rd party modules) involved in sending this critical staking
info.

By itself, a Converter cannot impact the local staking system. It must connect to the [Virtual Staking](./VirtualStaking.md)
contract, which will convert the "virtual stake" into actual stake in the dPoS system, and return the rewards as well.
This document focuses on the flow from IBC packets to the virtual stake.

### Price normalization

When we receive a "virtual stake" message for 1 provider token, we need to perform a few steps to normalize it to the
local staking tokens.

The first step is simply doing a price conversion. This is done via a [Price Feed](#price-feeds), which is
defined on setup and can call into arbitrary logic depending on the chain. (For example,
if we are sent 1000 JUNO, we convert to 1200 OSMO based on some price feed).

The second step is to apply a discount. This discount reduces the value of the cross-stake to a value below what we would get from the pure
currency conversion above. This has two purposes: the first is to provide a margin of error when the price deviates far from the TWAP, so
that the cross-stake is not overvalued above native staking; the second is to encourage local staking over remote staking. Looking at the
asset's historical volatility can provide a good estimate for the first step, as a floor for minimum discount. Beyond that, consumer
chain tokenomics and governance design is free to increase the discount as they feel beneficial.

In this case, let's assume a discount of 40%. A user on the Provider chain cross-stakes 100 PROV. We end up with a weight of

`100 PROV * 18 CONS/PROV * (1 - 0.4) = 1080 CONS`

Thus, this cross-stake will trigger the Converter to request the virtual staking module to stake 1080 CONS.

The discount is stored in the Converter contract and can only be updated by the admin (on-chain governance).

**Important** When we calculate the virtual stake (e.g. 1080 CONS in the example above), those
tokens will be staked as if they were native CONS tokens. They have the same influence on the
validator's voting power, and will receive the same rewards. The only difference is that they
can never be withdrawn, and slashing is managed remotely on the Provider chain.

### Price Feeds

In order to perform the conversion of remote stake into local units, the Converter needs a
trustable price feed. Since this logic may be chain dependent, we don't want to define it in the Converter
contract, but rather allow chains to plug in their custom price feed without modifying any of
the complex logic required to properly stake.

There are many possible price feed implementations. A few of the main ones we consider are:

**Gov-defined feed.** This is a simple contract that stores a constant price value, which is always
returned when asked for the price. On-chain governance can send a vote to update this price
value when needed. **This is good for mocks, or a new chain with no solid price feed,
and wanting a stable peg**.

**Local Oracle** If there is a DEX on the Consumer chain with sufficient liquidity and volume
on this asset pair (local staking - remote staking), then we can use that for a price feed.
Assuming it keeps a proper TWAP oracle on the pair, we sample this every day and can get the average
price over the last day, which is quite hard to manipulate for such a long time.
**This is good for an established chain with solid DEX infrastructure, like Osmosis or Juno**.

**Remote Oracle** More dynamic than the gov-defined feed, but less secure than the local Oracle,
we can do an IBC query on a DEX on another chain to find the price feed. This works like the
Local Oracle, except the DEX being queried lives on e.g. Osmosis. Note that it introduces another
security dependency, as if the DEX chain goes Byzantine, it could impact the security of the Consumer
chain. **This is a better option if the local staking token has a liquid market, but there is
no established DEX on the chain itself (like Stargaze)**.

The actual logic giving the price feed is located in an Oracle contract (configured upon init).
We recommend using an (e.g. daily) TWAP on a DEX with good liquidity - ideally on the consumer chain,
but this implementation is left up to the particular chain it is being deployed on.

With this TWAP we convert e.g. 1 PROV to 18 CONS.

### Virtual Staking

Each Converter is connected 1:1 with a [Virtual Staking](./VirtualStaking.md) contract. This contract
manages the stake and has limited permissions to [call into a native SDK module](./GoModule.md)
to mint "virtual tokens" and stake them, as well as immediately unbonding them. The contract
ensures the delegations are properly distributed.

The Converter simply tells the Virtual Staking contract it wishes to bond/unbond N tokens
and that contract manages all minting of tokens and distribution among multiple validators.
We dig more into the mechanics of the Virtual Staking contract in the
[Virtual Staking](./VirtualStaking.md) document.

## Rewards Flow

Once per epoch, the Virtual Staking module will trigger rewards. This will send a number of
messages to the Converter, specifying which validators the rewards belong to, along with the
native reward tokens themselves.

TODO: Update this.
The Converter will then [transfer all these tokens via ICS20](../ibc/Overview.md) to the
corresponding `External Staking` contract on the Provider chain, and send a message over the
standard IBC channel to inform the `External Staking` contract how to distribute them.
(If we get callbacks on ICS20, we send the metadata only after tokens have arrived. Until then
(for MVP), we send them concurrently and hope).

## Unstaking Flow

The Converter can also unstake some tokens. These will be held in escrow on the Provider and
are susceptible to slashing upon proper evidence submission. Since the virtual stake is, well,
"virtual" and slashing has no impact, the delegation numbers can be immediately reduced
on the Consumer's native staking module.

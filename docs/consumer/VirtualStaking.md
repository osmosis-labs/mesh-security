# Virtual Staking

Virtual Staking is a permissioned contract on the Consumer side that can interact with the
native staking module in special ways. There are usually multiple Virtual Staking contracts
on one Consumer chain, one for each active Provider, linked 1-to-1 with a Converter.

## Previous Work

There is a lot of overlap with this design and Osmosis' [design of SuperFluid Staking](https://github.com/osmosis-labs/osmosis/tree/main/x/superfluid).
Before commenting on the technical feasibility or implementation on the SDK side, it would be good to review this in depth.
This is probably the biggest change made to staking functionality without forking `x/staking` and should be leveraged for the MVP at least.

## Interface

The entry point to the Virtual Staking system is a contract that can be called by one Converter, and which
has some special abilities to stake virtual tokens. We cannot let any receiver mint arbitrary tokens, or we lose all security,
so each "Converter / Virtual Staking Contract" pair has permission of a maximum amount of "virtual stake" that it can provide
to the system. Anything over that is ignored, after which point, the average rewards per cross-staker start to diminish as
they split a limited resource.

The `Converter` should be able to call into the `Virtual Staking` contract with the following:

```rust
pub enum VirtualStakeMsg {
  /// This mints "virtual stake" if possible and bonds to this validator.
  Bond {
    amount: Coin,
    validator: String,
  },
  /// This unbonds immediately, not like standard staking Undelegate
  Unbond {
    amount: Coin,
    validator: String,
  },
}
```

The `Converter` should be able to query the following info from the contract:

```rust
pub enum VirtualStakeQuery {
  #[returns(BondStatusResponse)]
  BondStatus {
    contract: String,
  },
}

pub struct BondStatusResponse {
  pub cap: Coin,
  pub delegated: Coin,
}
```

Finally, the virtual staking contract should make the following call into the `Converter`,
which is sent along with a number of `info.funds` in the native staking token:

```rust
pub enum ConverterExecMsg {
  /// This is required, one message per validator all info.funds go to those delegators
  DistributeRewards {
      validator: String,
  },
  /// Optional (in v1?) to optimize this, by sending multiple payments at once.
  /// info.funds should equal rewards.map(|x| x.reward).sum()
  DistributeRewardsMulti {
    rewards: Vec<RewardInfo>,
  },
}

pub struct RewardInfo {
  pub validator: String,
  pub reward: Coin,
}
```

## Extensibility

Virtual Staking is primarily defined by the interface exposed to the Convert contract, so we can use wildly
different implementations of it on different chains. Some possible implementations:

- A CosmWasm contract with much of the logic, calling into a few custom SDK functions.
- A CosmWasm contract that just calls into a native module for all logic.
- An entry point to a precompile (eg. Composable uses "magic addresses" to call into the system, rather than `CustomMsg`)
- A mock contract that doesn't even stake (for testing)

All these implementations would require a Virtual Staking contract that fulfills the interface above and is informed by the
specific design discussed here, but it can by creative in the implementation.

The rest of this document describes a "standard SDK implementation" that we will ship with Mesh Security.
Since we want this to be portable and likely to be integrated into as many chains as possible, we will look
for a design minimizing the changes needed in the Go layer, and focus on keeping most logic on CosmWasm.
We can add other implementations later as needed.

## Standard Implementation

Virtual Staking will be a mix of a Cosmos SDK module and a CosmWasm contract.
We need to expose some special functionality through the SDK Module and `CustomMsg` type.
The module will contain a list of which contract has what limit, which can only be updated
by the native governance. The interface is limited and can best be described as Rust types:

```rust
#[cw_serde]
pub enum CustomMsg {
  /// Embed one level here, so we are independent of other custom messages (like TokenFactory, etc)
  VirtualStake(VirtualStakeMsg),
}

#[cw_serde]
/// These are the functionality
pub enum VirtualStakeMsg {
  /// This mints "virtual stake" if possible and bonds to this validator.
  Bond {
    amount: Coin,
    validator: String,
  },
  /// This unbonds immediately, not like standard staking Undelegate
  Unbond {
    amount: Coin,
    validator: String,
  },
}
```

It will also need a query to get its max cap limit. Note that it can use standard `StakingQuery` types to query it's existing delegations
as they will appear as normal in the `x/staking` system.

```rust
#[cw_serde]
pub enum CustomQuery {
  /// Embed one level here, so we are independent of other custom messages (like TokenFactory, etc)
  VirtualStake(VirtualStakeQuery),
}

#[cw_serde]
/// These are the functionality
pub enum VirtualStakeQuery {
  #[returns(MaxCapResponse)]
  MaxCap { },
}

pub struct MaxCapResponse {
  pub cap: Coin,
}
```

### Contract

Each Virtual Staking contract is deployed with a Converter contract and is tied to it.
It only accepts messages from that Converter and sends all rewards to that Converter.
The general flows it provides are:

- Accept Stake message from Converter and execute custom Bond message
- Accept Unstake message from Converter and execute custom Unbond message
- Trigger Reward Withdrawals periodically and send to the Converter

It should keep track on how much it has delegated to each validator, and the total amount of delegations,
along with the max cap. It should not try to bond anything beyond the max cap, as that will error.
But then silently adjust internal bookkeeping, ignoring everything over max cap.

The simplest solution is:

- If total delegated is less than max cap, but new delegation will exceed it, just delegate the difference
- If total delegated is less than max cap and we try to stake more, update accounting, but don't send virtual stake messages
- If we unbond, but total delegated remains greater than max cap, update accounting, but don't send virtual stake messages

However, there is an issue here, as we are staking on different validators. Imagine I have a cap of 100. 80 is bonded to A.
I request to bond 120 more to B, which turns into 20 bond to B. There is an actual ration of 4:1 A:B bonded, but the remote
stakers have delegated at a ratio of 2:3. This gets worse if eg. someone unbonds all A. We end up with 120 requested for B,
but 80 delegated to A and 20 delegated to B.

To properly handle this, we should use Redelegations to rebalance when the amounts are updated.
This may be quite some messages (if we have 50 validators, each time we stake one more, we need to
add a bit to that one and decrease the other 50). We describe a solution to this in the next section.

### Epochs

For efficiency, rewards will be withdrawn for all cross-stakers at a regular rhythm, once per epoch.
This is done to reduce the number of IBC messages sent, and reduce computation (gas) overhead.
Note that one provider will likely have to withdraw from dozens of validators and then distribute
to the proper cross-stakers, so this is a significant amount of computation, and shouldn't be
triggered every few blocks.

While staking and unstaking can be quite cheap below the max cap, it becomes quite expensive once
the number of tokens exceeds the max cap, as it will require a rebalance. If a provider is
over the max cap and someone stakes 10 tokens on validator V, the provider will have to unstake
some tokens proportionally from every other validator. Likewise, if someone unstakes 10 tokens,
it will trigger a staking on the remaining validators to keep the total at max cap.

#### CosmWasm Contract

It makes sense to do this rebalance only once per epoch, as it is quite expensive. These could
be different epoch periods for two different purposes, but it is recommended to keep them
the same for simplicity. However, we do suggest that the epochs of each provider be offset,
so we don't clog up the network with a bunch of IBC messages at the same time.

Implementation of this will require another binding with the native Go module. Our proposed
mechanism is to add a special `SudoMsg` that is triggered once per epoch on each contract with
a max cap. Each contract will have a `next_epoch` value and they all will share an `epoch_length`
parameter.

Calling `sudo` will be done in EndBlock once per epoch, and will have a gas limit to prevent
any freezing of the chain. We suggest something like 50-100% of the normal block gas limit.
The calling code should also catch any error (including out of gas) and log it, but not fail.
It will revert all state changes made by the `sudo` message, mark it as ran and continue.
This provides a way to trigger the contract regularly with a large gas allowance, but not
be susceptible to any DoS attacks.

```rust
#[cw_serde]
pub enum SudoMsg {
  Rebalance{},
}
```

### SDK Module

The module maintains a list of addresses (for Virtual Staking contracts), along with a max cap of
virtual tokens for each. It's main purpose is to process `Bond` and `Unbond` messages from
any registered contract up to the max cap. Note that it mints "virtual tokens" that don't affect
max supply queries and can only be used for staking.

The permissions defined in the Virtual Staking module to cap the influence of the various provider is of cirtical importance
for the security design of Mesh Security. Not all remote chains are treated equally and we need to be selective
of how much security we allow to rest on any given token.

The Virtual Staking **Module** maintains an access map `Addr => StakePermission` which can only be updated by chain governance (param change).
The Virtual Staking **Module** also maintains the current state of each of the receivers.

```go
type StakePermission struct {
  // Limits the cap of the virtual stake of this converter.
  // Defined as a number of "virtual native tokens" the given contract can mint.
  MaxStakingRatio: sdk.Int,
  // Next time (unix seconds) we trigger the sudo message
  // When the contract is first registered, it is set to block time + epoch length
  NextEpoch: uint64,
}
```

Beyond MVP, we wish to add the following functionality:

- Provide configuration for optional governance multiplier (eg 1 virtual stake leads to 1 tendermint power, but may be 0 or 1 or even 0.5 gov voting power)

### Reward Withdrawals

The Virtual Staking Module uses an [EndBlock hook called once per epoch](#epochs)
(param setting, eg 1 day).
This will trigger a `SudoMsg` to each Virtual Staking contract, allowing them to both do a rebalancing
with any staking/unstaking in that period, as well as withdraw rewards from all validators.

When the epoch finishes for one Converter, the Virtual Staking Module will withdraw rewards from all delegations
that converter made, and send those tokens to the Converter along with the info of which
validator these are for.

The initial implementation will call the Converter eg 50 times, once for each validator.
If dev time permits, we can use a more optimized structure and call it once, with all the
info it needs to map which token corresponds to which validator. (See both [variants of
`ConverterExecMsg`](#interface))

The Converter in turn will make a number of IBC packets to send the tokens and this metadata back
to the External Staking module on the Provider chain.

## Roadmap

Define which pieces are implemented when:

MVP: We stake virtual tokens (like SuperFluid), can unbond rapdily, and these influence governance normally
(validators with delegations get more governance voting power)

V1: We can turn the governance influence on and off per converter. (Stake impacts Tendermint voting power,
but may or may not impact governance voting powerr). We also get native callbacks for triggering rewards
each epoch.

V2: Make improvements here as possible (rebonding, fractional governance multiplier)

# Control Channel

This document describes the "control channel", which is a direct IBC channel between the
`external-staking` contract on the Provider side and the `converter` contract
on the Consumer side. This is used to send messages about bonding and unbonding,
and any other metadata about the protocol (like validators).

It is also used to send reward amounts(./Rewards.md) that have been earned by delegators
from the Provider chain, and are redeemable on the Consumer chain.

It is **not** used to [handle slashing](./Slashing.md), as there are concerns
a malicious state machine would lie, so we demand original evidence of Tendermint
misbehavior on the provider chain.

This channel encompasses two sub-protocols, which are described in their own documents.
The establishment of the channel (including handshake and version negotiation)
[is described further below](#establishing-a-channel).

## Start with Theory

This is the core IBC protocol of Mesh Security and requires a solid design to guarantee
correct operation in face of asynchronous communication, reordering, and errors.

In order to make the protocol documents more compact,
[all theoretical foundations are described separately](./Serializability.md).
Please read through that document and have a decent grasp of those concepts before
digging into the sub-protocols below.

## Validator Metadata Syncing

The Consumer sends packets for the original validator set.

TODO: Validator set updates are not yet supported. Only the original validator set is sent once
after IBC connection establishment at the moment.

We use a CRDT-based algorithm to maintain a consistent view of the validator set regardless
of the order the packets received, such that any permutation of the same set of messages
will result in the same state on the provider. These operations are `commutative` and `idempotent`.

The [full description of the algorithm](./Validators.md) is quite lengthy and defined in its own page.

## Virtual Staking Protocol

The Provider sends messages to stake and unstake virtual tokens to various validators
on the Consumer chain. This must be done in such a way that it is robust in face
of reordering and errors upon an unordered channel.

The [full description of the algorithm](./Staking.md) is quite lengthy and defined in its own page.

## Establishing a Channel

As [discussed below](#channel-ordering), ordered channels are extremely fragile and
one packet that causes and error can shut down the channel forever.

Unordered channels make it harder to prove guarantees for the application in an asynchronous
environment, but we will use them here. Thus, all communication must have a proof that
it maintains correctness in face of arbitrary packet reordering and dropping (via error/timeout).

### Deployment

Before creating that channel, you must have created the `external-staking` and
`converter` contracts. The `external-staking` contract must be initialized with
the valid (connection, port) from which the converter will connect, which means that
the converter contract must be instantiated before the `external-staking` contract.

The channel is initiated by a relayer, with party A being the appropriate `converter`
contract on the consumer chain, and party B being the `external-staking` contract.

The general process (assuming a vault is already established on the Provider) is:

1. Instantiate price feed, converter, virtual staking contracts on the Consumer chain.
2. Instantiate external staking contract on the Provider chain (referencing IBC port of the converter).
3. Create IBC channel from Provider to Consumer.
4. Apply to Consumer governance to provide a virtual staking max cap to the associated virtual staking contract,
   so that this connection may have voting power.

### Handshake

Opening the channel is a 4-step process. It must be initiated by the Consumer side.

1. Start with `OpenInit` from converter to the (connection, port) of the external staking. The version 
   SHOULD be set to the highest mesh-security version it supports (see below), and the channel ordering
   MUST be "unordered". It MUST error if it has a previously established channel.
2. The external staking contract receives `OpenTry`. The channel ordering MUST be "unordered",
   and the version protocol MUST be `mesh-security`. It performs version negotiation as defined below.
   It MUST error if the (connection, port) being proposed is not the one it was initialized with.
   It MUST error if it has a previously established channel.
3. The converter receives `OpenAck` and MUST verify the version protocol is `mesh-security`.
   It MUST verify the version is not higher than the one it proposed, and not lower than the oldest version
   it supports. If successful, it stores the new channel details locally.
4. The external staking contract receives `OpenConfirm`. Everything has been verified on all sides,
   and there can be no errors here. It stores the new channel details locally.

Closing a channel is currently not well-defined. It is expected that the channel will remain open, as it is unordered.
If the channel is closed, both sides must mark the channel as closed locally, and error on any attempt to send IBC packets.
The channel may be re-opened by repeating the initial process, with both sides validating the re-open
was from the same (connection, port) as the original channel. When that handshake is completed, they can replace
the closed channel from storage with the new open channel.

### Version Negotiation

The channel version uses a JSON-encoded struct with the following fields:

```json
{
  "protocol": "mesh-security",
  "version": "1.0.0"
}
```

It is important that you **do not** use `#[cw_serde]` on the Rust struct as we explicitly
want to **allow unknown fields**. This allows us to add new fields in the future.
`#[cw_serde]` generates `#[serde(deny_unknown_fields)]` which would break this.

Both sides must error if the protocol is not `mesh-security`.

The version is used to allow for future upgrades. The provider should always send the
highest protocol version it supports to start the handshake. The consumer should
error if the major version is higher than its known versions (for now, anything besides `1`).
The consumer should respond with the highest version it supports, but no higher than
that proposed by the provider.

At the end, the version is the highest version supported by both sides, and they may freely make
use of any features added up to that version. This document describes version `1.0.0` of
the protocol, but additions may be added in the future (which must be linked to from this section).

### Channel Ordering

Note the entire protocol is designed around syncing an initial state and sending a stream
of diffs, such that `State = Initial + Sum(Diffs)`. This applies to both the validator set
and the total amounts staked.

If we reorder the diffs, it is possible to get a different result, so we need to be careful
about relying on Unordered channels. Imagine Stake 100 and Unstake 50. If Unstake goes first,
it would return an error, yet the Stake would apply properly, leaving a total of 100 not 50.
Furthermore, if a packet is dropped, and the diff is not applied, the two sides will
get out of sync, where e.g. the Provider believes there is 500k staked to a given validator,
while the Consumer believes there is 400k.

At the same time, Ordered channels are fragile, in that a single timed-out or undeliverable packet
will render the channel useless forever. We must make sure to use extremely large timeouts
(say 7 days) to handle the case of a prolonged chain halt. We must also ensure that the
receiving contract always returns ACKs with errors on failure, and never panics.
A contract panic will abort the tx containing the IbcPacketReceiveMsg, as of wasmd 0.40
(MSV for Mesh Security).

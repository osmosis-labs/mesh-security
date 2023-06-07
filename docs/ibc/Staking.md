# Cross-Chain Staking Protocol

This describes the "control channel", which is a direct IBC channel between the
`external-staking` contract on the provider side and the `converter` contract
on the consumer side. This is used to send messages about bonding and unbonding,
and any other metadata about the protocol (like validators).

It is **not** used to [send reward tokens](./Rewards.md), which must be sent over
the commonly accepted ICS20 interface, so they are fungible after receipt.

It is also **not** used to [handle slashing](./Slashing.md), as there are concerns
a malicious state machine would lie, so we demand original evidence of Tendermint
misbehavior on the provider chain.

## Establishing a Channel

As [discussed below](#channel-ordering), ordered channels are extremely fragile and
one packet that causes and error can shut down the channel forever.

Unordered channels make it harder to prove guarantees for the application in an asynchronous
environment, but we will use them here. Thus, all communication must have a proof that
it maintains correctness in face of arbitrary packet reordering and dropping (via error/timeout).

### Handshake

Before creating that channel, you must have created the `external-staking` and
`converter` contracts. The `external-staking` contract must be initialized with
the valid (connection, port) from which the converter will connect, which means that
the converter contract must be established before the `external-staking` contract.

The channel is initiated by a relayer, with party A being the appropriate `converter`
contract on the consumer chain, and party B being the `external-staking` contract.

The general process (assuming a vault is already established on the provider) is:

1. Instantiate price feed, converter, virtual staking contracts on the consumer chain
2. Instantiate external staking contract on the provider chain (referencing IBC port of the converter)
3. Create IBC channel from provider to consumer
4. Apply to consumer governance to provide a virtual staking max cap to the associated virtual staking contract, so that this connection may have voting power.

### Version Negotiation

The channel version uses a JSON-encoded struct with the following fields:

```json
{
    "protocol": "mesh-security",
    "version": "1.0.0",
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

At the end, the version is the highest version supported by both sides and they may freely make
use of any features added up to that version. This document describes version `1.0.0` of
the protocol, but additions may be added in the future (which must be linked to from this section).

## Validator Metadata Syncing

The provider sends packets for the original validator set and for every update.

We use a CRDT-based algorithm to maintain a consistent view of the validator set regardless
of the order the packets received, such that any permutation of the same set of messages
will result in the same state on the provider. These operations are `commutative` and `idempotent`.

The [full description of the algorithm](./ValidatorSet.md) is quite lengthy and defined in its own page.

## Staking Messages

**TODO**

Once it has the information of the valid set of validators, the main action taken 
by the provider is assigning virtual stake to particular validator and removing it.

It does so via `Stake` and `Unstake` messages, specifying the valoper address it wishes
to delegate to and the amount of local (provider-side, held in the vault) tokens that
it wishes to virtually stake.

The converter must track the total sum of these messages for each validator on the consumer
side, convert them to proper values of native tokens and "virtual stake" them.

## Channel Ordering

Note the entire protocol is designed around syncing an initial state and sending a stream
of diffs, such that `State = Initial + Sum(Diffs)`. This applies to both the validator set
as well as the total amounts staked. 

If we reorder the diffs, it is possible to get a different result, so we need to be careful
about relying on Unordered channels. Imagine Stake 100 and Unstake 50. If Unstake goes first,
it would return an error, yet the Stake would apply properly, leaving a total of 100 not 50.
Furthermore, if a packet is dropped, and the diff is not applied, the two sides will
get out of sync, where eg the Provider believes there is 500k staked to a given validator,
while the Consumer believes there is 400k. 

At the same time, Ordered channels are fragile, in that a single timed-out or undeliverable packet
will render the channel useless forever. We must make sure to use extremely large timeouts
(say 7 days) to handle the case of a prolonged chain halt. We must also ensure that the
receiving contract always returns Acks with errors on failure, and never panics.
A contract panic will abort the tx containing the IbcPacketReceiveMsg as of wasmd 0.40
(MSV for Mesh Security)

## Implementation Notes

In order to maintain state synchronized on both sides, we must ensure that the
sending side properly handle ACKs, for both the success and failure case. 

If there is a failure sending a validator set update, it should be retried later.
A safe way to do so would be to store these messages in some "queue" and trigger sending 
such `AddValidator` and `RemoveValidator` packets on the next incoming Stake/Unstake command
(when we know the provider side is working). They should be re-sent in the same order
they were originally sent.

If there is a failure sending a Stake / Unstake action, the safest response is to
undo the action locally and inform the user. We need a solid UI here, as suddenly having
a staking action disappear with no notification as to why will lead to serious confusion
and cries of "bugs", when it was just an invisible, delayed error.

An alternate approach here is to have some sort of "re-sync" design to re-synchronize the
state on the two sides, but I argue this is too fragile in an asynchrnous environment.
Much care must be taken to ensure that all application errors are handled well.

### Error handling

When staking, create a `Stake` packet, but do not update the user's / validator's state
in the `external-staking` contract. If an error ack comes back, then the
`external-staking` contract should immediately call back to the vault to release
the lien for the amount of that packet. If a success ack comes back, it should
actually apply the staking changes locally.

Likewise, when unstaking, we first create an `Unstake` packet. However, here 
we want to avoid the situation of `Unstaking` 100% of the tokens multiple times,
as this would apply invalid state changes to the consumer. We need to enforce this 
limit on the provider side (which is handled by the vault in the staking case).

For `Unstake`, we should update a local "unstaking" value on the `(user, validator)`
staking info, but not create a claim nor apply a diff to the validator. 
We ensure that this "unstaking" amount can never be larger than the properly staked
(and ack'ed) value. On an error ack, we simply reduce "unstaking" by that amount.
On a success ack, we commit these changes, reducing not only "unstaking", but
also applying the deduction to actual stake for the user as well as the validator,
updating the rewards claim table, and creating the claim, so the user can get their
tokens / lien back after the unbonding period.

### Re-syncing

We could invent some "re-sync" mechanism, but would have to be careful mixing this with
in-flight messages. For example, the provider sends a `Stake` message to the consumer,
which returns an error for whatever reason. Before the ack has arrived, the provider
sends a re-sync message with the entire content of its local state.

Upon receiving that "re-sync" message, the consumer updates all tables and triggers the
appropriate virtual staking commands. However, the error ack for the `Stake` packet
now lands on the provider and it "undoes" the Stake action. This will be applied
on top of the snapshot that it sent to the consumer, thus modifying it and the two sides
will have diverged again.

This whole issue becomes much more difficult to manage if we are relying on unordered channels.
For example, if there is an "in-flight" `Stake` message that has not been processed, and we 
"re-sync" by sending the provider's view of the entire staking assignment, a malicious 
relayer could post the re-sync packet first and the `Stake` message second, thus double applying it.

### Error correction

**TODO** Ideas about using values in success acks to double check the state matches expectations.
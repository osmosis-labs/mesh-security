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

## Validator Metadata

It is important for the provider to know the proper validators on the consumer chain.
Both in order to limit the delegations to valid targets *before* creating an IBC message,
but also in order to track tendermint public keys to be able to slash properly.

We define the "Validator Subprotocol" as a way to sync this information. It uses a CRDT-like
design to maintain consistency in the face of arbitrary reordering. And retries in order
to ensure dropped packets are eventually received.

### Message Types

All validator change messages are initiated from the consumer side. The provider is
responsible for guaranteeing any packet with a success ACK is written to state.
Given all successful acks have been committed, the consumer maintains enough
information to sync otustanding changes and guarantee the Provider will eventually
have a proper view of the dynamic validator set.

We make use of two messages. `AddValidators` and `RemoveValidators`. These are batch messages
due to the IBC packet overhead, but conceptually we can consider them as vectors of "AddValidator"
(`A(x)`) and "RemoveValidator" (`R(x)`) messages.

### Message Sending Strategy

Once the channel is established, the consumer will sync the current state via an `AddValidators`
message for all validators in the current active set. This is a one-time message, and
all future messages are diffs on top of this initial state. Future changes will be sent as a
stream of `AddValidators` and `RemoveValidators` messages.

As new validators are added to the active set, the consumer will send an `AddValidators`
message with their information. We do not signal when a validator is removed from the active
set as long as it is still valid to delegate to them (ie. they have not been tombstoned).

When a validator is tombstoned, the consumer will send a `RemoveValidators` message with
the address of that validator. Once it has been removed, it can never be added again.

_Note: sending these updates as a stream (rather than polling for the whole list every epoch) requires some custom sdk bindings. This should be done as part of the virtual staking module, but the implementation will target v1. For MVP, we can just do batches every epoch and ignore slashing._

### CRDT Design

As you see from the message types, we are using an operation-based CRDT design.
This requires that all operations are commutative. We also guarantee they are idempotent,
although that is not strictly required given IBC's "exactly once" delivery.

In this section, we consider the operations that compose IBC packets:

* `A(x, p)` - Add validator `x` with pubkey `p` to the validator set
* `R(x)` - Remove validator `x` from the validator set

We do not consider Tendermint key rotation in this section, but describe it as an addition
in the following section. This should support the basic operations available today.

We wish to maintain a validator set `V` on the Provider with the following properties:

* If no packet has been received for a given `x`, `x` is not in `V`
* If `R(x)` has been received, `x` is not in `V`
* If `A(x, p)` has been received, but no `R(x)`, `x` is in `V` with pubkey `p`

The naive implementation of add on A and remove on R would not work. `[A(x, p), R(x)]` would
be properly processed, but `[R(x), A(x, p)]` would leave A in the set.

Instead, we store an enum for each validator, with the following states: 

```rust
type Validator = Option<ValidatorState>

enum State {
    Active(Pubkey),
    Tombstoned,
}
```

The two states for a "seen" validator are embedded inside `Some(_)`, as any Rust Map implementation
will provide us with `None` for any unseen validator.

We define the state transitions as:

```rust
let x = match &op {
    R(x) => x,
    A(x, _) => x,
};
let old_state: Option<State> = VALIDATORS.may_load(storage, x)?;
let new_state: State = match (old_state, op) {
    (_, R(_)) => State::Tombstoned,
    (Some(State::Tombstoned), _) => State::Tombstoned,
    (None, A(_, p)) => Some(State::Active(p)),
    // Ignore A if we have received state already
    (Some(State::Active(p)), A(_, _)) => Some(State::Active(p)),
};
```

### Proof of correctness

The basic implementation without public key rotation is another expression of the same algorithm
used in [`2P-Set`](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#2P-Set_(Two-Phase_Set)). This is a proven CRDT. But we explain the three properties here for completeness.

We promise to hold 3 properties:

_If no packet has been received for a given `x`, `x` is not in `V`_ - This is trivially true as we start with an empty map and only add validators via operations that include their address.

_If `R(x)` has been received, `x` is not in `V`_ - Upon the first receipt of `R(x)`, `V(x)`
transitions to `Tombstoned` from any previous state. 
`(Some(State::Tombstoned), _) => State::Tombstoned` will guarantee that once a validator
is in the `Tombstoned` state, it will never leave that state.

We also see that multiple receipts of `R(x)` have the same effect as a single such message, making
this idempotent.

_If `A(x, p)` has been received, but no `R(x)`, `x` is in `V` with pubkey `p`_ - The first part
is set when `A(x, p)` is seen the first time, and maintained when seen a second time.
The "but no R(x)" clause is maintained by the proof above, which overrides any other state
and enforces a permanent tombstone.

### Validator Key Rotation

In addition to the basic operations described above, there is another operations we would like
to support. This protocol is designed to handle Tendermint key rotation, such that a validator address
may be associated with multiple public keys over the course of it's existence. 

This feature is not implemented in Cosmos SDK yet, but long-requested and the Mesh Security
protocol should be future proof. (Note: we do not handle changing the `valoper` address as
we use that as the unique identifier).

To support key rotation, we must clarify what is desired... we want to keep the "most recent"
pubkey for a given validator. We also want to maintain a list of "past pubkeys" alogn with their
last active date. This is to allow for slashing to be associated with them,
if a double sign occurs within the "unbonding period" (ie. 21 days) of them being rotated out
(based on timestamps of the consumer chain).

This is very much like the state above, but we expand the content stored in the `State::Active`
variant. We also add a new operation: 

* `K(x, p, po, h)` - Update validator `x` to have pubkey `p`. The previous pubkey was `po` and the update occurred on block height `h`.

We add the limitation that is is invalid behavior to produce two different `K(x, p, po, h)` and `K(x, p', po', h)`, with the same `x`
and `h`, but different public keys. Basically, a given validator may not rotate their keys more than once per block height (which is
a very reasonable limitation).

The state transitions is the same as above, except we handle `K` similar to `A` (in terms of `None` -> `Active` -> `Tombstoned`).
What we want to focus on is the merge function for the inner state. We add a list of all past pubkeys, with their last active time.

```rust
type Validator = Option<ValidatorState>

enum State {
    /// The first entry is the current key.
    /// The second entry is a list of past keys, with their last active time.
    /// This list is sorted in descending order, with the most recent key first.
    Active(Pubkey, Vec<(Pubkey, Timestamp)>),
    Tombstoned,
}
```

This adds three state transitions to consider:

* `None` + `K(x, p, po, t)` => `State::Active`
* `Some(State::Active(_))` + `K(x, p, po, t)` => `State::Active(_)`
* `Some(State::Active(_))` + `A(x, p)` => `State::Active(_)`

The last one is either a no-op or an error. `A(x, p)` should only show up for the latest pubkey,
and return an error if it differs. In no case does it change the state. Let's now consider what
happens when applying `K` to the state; in particular, what are the invariants we want to hold
when applying `permutation(A, K1, K2, ... Kn)`?

* If `R(x)` has been seen, the state is `Tombstoned`. Otherwise, it is `Active`. (Same as before)
* `Active(p, v)` should hold the following invariants for `p`:
    * If `A(x, p)` has been seen, amd no `K(x, ..)`, then `p` comes from `A(_, p)` (Same as before)
    * Otherwise, `p` comes from the most recent `K(x, p, _, h)` (Maximum h)
* `Active(p, v)` should hold the following invariants for `v`:
    * There is one element in `v` for each `K` operation seen (from `(po, h)`)
    * `v` is sorted by `h` in descending order
    * There are no duplicate `h` values in `v`

```rust
let new_state: State = match (old_state, op) {
    // .. other branches as defined above

    // if we see K without a previous state, we treat it like `A` happened with `po` before
    (None, K(_, p, po, h)) => Some(State::Active(p, vec![(po, h)])),

    // if there is a state, we must insert the transition properly into the list and only update
    // the pubkey if 
    (Some(State::Active(cur, mut v)), K(_, p, po, h)) => {
        // --- if this transition is more recent that the last one, we update the current pubkey
        let updated = match v.get(0) {
            Some((_, h0)) if *h0 > h => cur,
            _ => p.clone(),
        };
 
        // ---- now we add this transition to the list of past pubkeys
        // Add this transition
        v.push((po, h));
        // sort the array from largest to smallest.
        v.sort_by(|a, b| a.cmp(b).reverse());
        // remove all duplicates (handle if `K` was seen before)
        v.dedup();
        // assert invariant violation if there are now multiple items with tn
        assert_eq!(v.iter().filter(|(_, x)| x == &h).count(), 1);

        Some(State::Active(updated, v)),
    }
};
```

### Proof of correctness

We have to show that for a set of `A` and `K` operations, all permutations of the order
of those operations result in the same state. 

* `Active(p, v)` should hold the following invariants for `v`:
    * There is one element in `v` for each `K` operation seen (from `(po, h)`)
    * `v` is sorted by `h` in descending order
    * There are no duplicate `h` values in `v`

The previous invariants for `v` are covered quite straightforwardly, by always adding new entries,
sorting the array and removing duplicates. The implementation can be optimized from the above description
(using clever inserting), but the code is a straightforward enforcement of the invariants.

* `Active(p, v)` should hold the following invariants for `p`:
    * If `A(x, p)` has been seen, amd no `K(x, ..)`, then `p` comes from `A(_, p)` (Same as before)
    * Otherwise, `p` comes from the most recent `K(x, p, _, h)` (Maximum h)

First of all, the initial state from the first `A` or `K` operation sets the initial value of `p`,
which will hold the invariant, as the "most recently seen" value is the "only seen" value.

```rust
    (None, A(_, p)) => Some(State::Active(p, [])),
    (None, K(_, p, po, h)) => Some(State::Active(p, vec![(po, h)])),
    (Some(State::Active(p, v)), A(_, _)) => Some(State::Active(p, v)),
```

We consider the case for `A` like a transition at height 0, meaning any further transition would
be more recent and should update the current pubkey. If we receive `[A, K]`, then we hit the case for `K` below 
(`v.get(0) == None`) and use the pubkey from `K`. If we receive `[K, A]`, then we set the pubkey from `K` first,
and return an Error for `A` with no state change (as it asserts a different pubkey than the state).

```rust
        let updated = match v.get(0) {
            Some((_, h0)) if *h0 > h => cur,
            _ => p.clone(),
        };
```

The case for `K` is more interesting, as the messages could be out-of-order, and we can't simply ignore them like `A`.
Assume `K(x, p, po, h)` and `K'(x, p', po', h')` with `h < h'` (and likely `po' = p`).
They could be received `[K, K']` or `[K', K]`, and both orderings should give the same response for the final pubkey (`p'`).

We have shown the first message will hold the invariant. In order to keep the invariant, we must update the pubkey
in the case `[K, K']` and not update it in the case `[K', K]`. Let's verify the algorithm maintains that.

In `[K, K']`, we initially set `v` to `[(po, h)]`. When adding `K'`, we compare `h'` to `h` and find `h' > h`, so it doesn't match
the first branch, and we overwrite the pubkey with that contained in `K'`.

In `[K', K]`, we initially set `v` to `[(po', h')]`. When adding `K`, we compare `h` to `h'` and find `h < h'`, so it matches
the first branch, and keep the pubkey originally set by `K'`.

Both cases maintain the invariant. And by induction, we can assert any new `K''` added to the existing state will maintain said invariant. 
Thus, we have shown that the algorithm is correct.

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
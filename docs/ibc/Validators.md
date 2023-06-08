# Validator Metadata

It is important for the provider to know the proper validators on the consumer chain.
Both in order to limit the delegations to valid targets *before* creating an IBC message,
but also in order to track tendermint public keys to be able to slash properly.

We define the "Validator Subprotocol" as a way to sync this information. It uses a CRDT-like
design to maintain consistency in the face of arbitrary reordering. And retries in order
to ensure dropped packets are eventually received.

## Message Types

All validator change messages are initiated from the consumer side. The provider is
responsible for guaranteeing any packet with a success ACK is written to state.
Given all successful acks have been committed, the consumer maintains enough
information to sync otustanding changes and guarantee the Provider will eventually
have a proper view of the dynamic validator set.

We make use of two messages. `AddValidators` and `RemoveValidators`. These are batch messages
due to the IBC packet overhead, but conceptually we can consider them as vectors of "AddValidator"
(`A(x)`) and "RemoveValidator" (`R(x)`) messages.

## Message Sending Strategy

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

## Basic CRDT Design

As you see from the message types, we are using an operation-based CRDT design.
This requires that all operations are commutative. We also guarantee they are idempotent,
although that is not strictly required given IBC's "exactly once" delivery.

In this section, we consider the operations that compose IBC packets:

* `A(x, p)` - Add validator `x` with pubkey `p` to the validator set
* `R(x)` - Remove validator `x` from the validator set

We do not consider Tendermint key rotation in this section, but describe it as an addition
in [the following section](#validator-key-rotation). 
This section is sufficient to support the basic operations available today.

We wish to maintain a validator set `V` on the Provider with the following properties:

* If no packet has been received for a given `x`, `x` is not in `V`
* If `R(x)` has been received, `x` is not in `V`
* If `A(x, p)` has been received, but no `R(x)`, `x` is in `V` with pubkey `p`

### Basic Implementation

The naive implementation of add on A and remove on R would not work. `[A(x, p), R(x)]` would
be properly processed, but `[R(x), A(x, p)]` would leave A in the set.

Instead, we store an enum for each validator, with the following states: 

```rust
type Validator = Option<ValidatorState>

enum State {
    Active(Pubkey),
    Tombstoned,
}

enum Op {
    A(validator, pubkey),
    R(validator),
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
used in [`2P-Set`](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#2P-Set_(Two-Phase_Set)). This is a proven CRDT, and if we implement it to match the spec, we have a 
guarantee of commutability.

## Validator Key Rotation

In addition to the basic operations described above, there is another operations we would like
to support. This protocol is designed to handle Tendermint key rotation, such that a validator
address may be associated with multiple public keys over the course of it's existence. 

This feature is not implemented in Cosmos SDK yet, but long-requested and the Mesh Security
protocol should be future proof. (Note: we do not handle changing the `valoper` address as
we use that as the unique identifier).

We wish the materialized state to have the following properties:

We wish to maintain a validator set `V` on the Provider with the following properties:

* If no packet has been received for a given `x`, `x` is not in `V`
* If `R(x)` has been received, `x` is not in `V`
* If at least one `A(x, _, _)` has been received, but no `R(x)`, `x` is in `V` with:
    * A set of all pubkeys, along with the block height they were first active from.     
    (We may represent this as a sorted list without duplicates, but that is a mathematically
    equivalent optimization)

To ensure we can perform all this with the commutivity property, we look for a mapping
of our concepts to proven CRDT types. The top level set is, as in the last section,
a [`2P-Set`](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#2P-Set_(Two-Phase_Set)).

Inside each element that has not been removed, we store the set of pubkeys
as a [`G-Set`](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#G-Set_(Grow-only_Set)),
which grows when each pubkey is added.

### Rotation Implementation

As long as we can prove our implementation matches those three concepts above, this is
a valid implementation. We will use the same `State` enum as above, store more information in
the `State::Active` variant, and add the "first active height" to each `A` operation.

A sample implementation could look like:

```rust
type Validator = Option<State>

enum State {
    /// The first entry is the most recent pubkey seen.
    /// 
    /// The second entry is a list of past keys, with their last active time.
    /// This list is sorted in descending order, with the most recent key first.
    /// (We can consider this a serializable form of a set)
    Active(Vec<(Pubkey, Timestamp)>),
    Tombstoned,
}

enum Op {
    A(validator, pubkey, height),
    R(validator),
}
```

A simple implementation of the state transitions is:

```rust
let x = match &op {
    R(x) => x,
    A(x, _) => x,
};
let old_state: Option<State> = VALIDATORS.may_load(storage, x)?;
let new_state: State = match (old_state, op) {
    // This handles leaving the 2P-Set
    (_, R(_)) => State::Tombstoned,
    (Some(State::Tombstoned), _) => State::Tombstoned,

    // Joining active set for the first time
    (None, A(_, p, h)) => Some(State::Active([(p, h)])),
    // Rotating the pubkey, adds to the set
    (Some(State::Active(mut v)), A(_, p, h)) => {
        // Add this transition
        v.push((po, h));
        // We sort the array from largest to smallest for quick compares on slashing
        v.sort_by(|a, b| a.cmp(b).reverse());
        // Remove all duplicates (handle if `K` was seen before)
        v.dedup();
        // Assert invariant violation if there are now multiple items same height
        debug_assert_eq!(v.iter().filter(|(_, hx)| hx == &h).count(), 1);
        Some(State::Active(updated, v)),
    }

};
```

### Proof of correctness

Since the CRDTs have already been proven, we just need to prove that our algorithm of
adding `A` to an existing `State::Present` is equivalent to adding to a set.

`v.push()` and `v.sort()` will add to a list and guarantee it has the same ordering
regardless of the order of operations. `v.dedup()` will remove duplicates,
keeping the property of a set that each element is only present once.

With this, we have proven that our algorithm is equivalent to a CRDT, and thus
fully commutative and maintains the desired properties regardless of packet ordering.


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

## CRDT Design

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

## Proof of correctness

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

## Validator Key Rotation

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

## Proof of correctness

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

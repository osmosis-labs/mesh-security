# Cross-Chain Staking Protocol

The staking protocol has two basic operations:

- Stake X tokens on Validator V - `S(V, X)`
- Unstake X tokens from Validator V - `U(V, X)`

We want to ensure that at any point in time, when all in-flight messages would be resolved, both the
provider and the consumer chain have the same view of the staking state. (**TODO** Consider the
effects of slashing, not included in this document currently).

We also want to guarantee that the provider chain always maintains sufficient staked tokens
in the vault to cover all virtual staking actions currently outstanding on the provider chain.

As [mentioned before](./ControlChannel.md#channel-ordering), we wish to use an unordered channel,
and therefore must bring a degree of understanding of [Serializability](./Serializability.md)
to this protocol.

## Delegation Syncing

The consumer side wants to maintain the same counter per validator as the provider side.
It manages a delegation count per validator - `D[V]`, with the following rules:

- Uninitialized is equivalent to `D[V] = 0`
- `S(V, X)` => `D[V] += X`, return success ack
- `U(V, X)` =>
  - if `D[V] < X`, return error ack
  - else `D[V] -= X`, return success ack

This is a pretty straightforward counter with a lower bound of 0, along with an increment and decrement
counters.

The only requirements to ensure this stays in sync is that the provider can successfully commit the same
changes upon a success ack, and is able to revert them (unstake) on an error ack.

**TODO** Do we need text / code / explanation here?

## Provider Side Design

Beyond this delegation list, the provider must maintain much more information.
The delegation to a given validator must be mapped to a user on the provider chain.
This user must also have a sufficient lien on the vault to cover the delegation.
And the vault must have sufficient funds to cover the lien.

This means we must maintain invariants over at least two contracts - the `external-staking` contract
and the `vault`. It includes possible conflicts with non-IBC transactions, like a user withdrawing
collateral from the vault that would be needed to cover the lien for an in-flight delegation.

Let us start analysing the protocol, but viewing all the state transitions that must be made on proper
success acks. These same state transitions must always be reverted without problem upon receiving error acks.

### Identifying Potential Conflicts

A staking operation would have the following steps and checks:

- Send a lien from `vault` to `external-staking` contract
  - Ensure there is sufficient collateral to cover max(lien)
  - Ensure there is sufficient collateral in sum(potential slashes)
  - Increase the lien for that given user on the `external-staking` contract
- Add a delegation in `external-staking` contract
  - Increase stake count for (user, validator)
  - Increase total stake on the validator
  - Increase the user's shares in the reward distribution
- Send an IBC packet to inform the Consumer
  - Guarantee we can commit all above on success
  - Guarantee we can rollback all above on error

An unstaking operation would have the following steps and checks:

- Remove a delegation in `external-staking` contract
  - Ensure stake count (user, validator) is set and greater than desired unstake amount
    - Ensure total stake on the validator is set and greater than desired unstake amount (should always be true if above is true)
  - Decrease stake count for (user, validator)
  - Decrease total stake on the validator
  - Decrease the user's shares in the reward distribution
  - Add an entry to the (user, validator) "pending unbonding" to withdraw said tokens after the unbonding period.
- Send an IBC packet to inform the Consumer
  - Guarantee we can commit all above on success
  - Guarantee we can rollback all above on error

Possible keys with conflicts are:

- In `vault` - `collateral(user)` and `lein(user, external-staking)`
- In `external-staking` - `stake(user, validator)`, `total-stake(validator)`, `reward-shares(user)`, `pending-unbonding(user, validator)`

### Identifying Potential Commutatibility

The general design should be to write all changes only on a successful ACK, but hold any locks needed to ensure those
write will not fail in any condition. Using the apporach of [Value Ranges](./Serializability.md#value-ranges), let us analyze
what needs to be minimally enforced here.

For staking, we update:

- `vault::lien(user, external-staking)` => `Maybe(+X)`
- `external-staking::stake(user, validator)` => `Maybe(+X)`
- `external-staking::total-stake(validator)` => `Maybe(+X)`
- `external-staking::reward-shares(validator, user)` => `Maybe(+X)`

The lower three have no upper limit and thus can never fail, so those operations are always commutative with others valid operations.
`vault::lein` has an upper value, and thus applying `Commit(+X)` could have a conflict with another transaction, so we must "lock" that
value.

For unstaking, we update:

- `external-staking::stake(user, validator)` => `Maybe(-X)`
- `external-staking::total-stake(validator)` => `Maybe(-X)`
- `external-staking::reward-shares(validator, user)` => `Maybe(-X)`
- `external-staking::total-shares(validator)` => `Maybe(-X)`
- `external-staking::pending-unbonding(user, validator)` => `Append((X, T))`

We have already seen that appending to an ordered/sorted list is commutative with other valid operations, so we can consider that as
commutative. The other three are all decrements, and may well conflict with another concurrent operation, such as another unstake,
as `Commit(-X)` could potentially fail.

### Minimal Locking

From the above, we have a list of all the values that could possibly cause conflicts with other transactions and thus must be locked:

- `vault::lien(user, external-staking)`
- `external-staking::stake(user, validator)`
- `external-staking::total-stake(validator)`
- `external-staking::reward-shares(validator, user)`
- `external-staking::total-shares(validator)`

We can simplify this list by making use of our knowledge of how various states are derived from each other:

- `total-stake(validator)` is the sum of `stake(user, validator)` over all users.
- `reward-shares(validator, user)` is the same as `stake(user, validator)`.
- `total-shares(validator)` is the sum of `reward-shares(validator, user)` over all users.

By analyzing how these other values are derived, we can see that if `stake(user, validator)` is never negative, then
none of the other values can ever be negative either. This means we can simplify those four keys to simply lock `stake(user, validator)`,
which is sufficient to guarantee the invariants of the other staking values, without interfering with actions by other users.

We end up with two keys that need to write locks, both of which only affect a single user:

- `vault::lien(user, external-staking)`
- `external-staking::stake(user, validator)`

Note that a write-lock will prevent reading of the value from any other transaction. That means the same user offering a lien on another
validator, which iterates over all liens to find the sum of max slashing, will be blocked until the first transaction completes.
We cannot actually block (or wait) transactions in this way, so we must return an error for the second transaction.

## Protocol Implementation

The messages can be defined as follows:

```rust
enum Op {
    Stake { validator, user, amount },
    Unstake { validator, user, amount },
}
```

One issue here is that the Consumer doesn't care about the user who made the staking action, only the validator.
However, when processing the ack, the Provider will need to know which user performed the action.
We could store this as local state in the Provider, indexed by the packet sequence, but simply including an extra
field in the packet is much simpler.

### Consumer-Side Logic

This is quite simple, as the Consumer only needs to apply the staking action to its local state, and return an error
if this violates some invariant (goes below 0):

```rust
match op {
    Stake { validator, user, amount} => {
        ensure_eq!(amount.denom, cfg.staking_denom, IbcError::InvalidDenom);
        let delegated = DELEGATIONS.may_load(store, validator)?.unwrap_or_default();
        delegated += amount.amount;
        DELEGATIONS.save(store, validator, delegated)?;
    }
    Unstake { validator, user, amount} => {
        ensure_eq!(amount.denom, cfg.staking_denom, IbcError::InvalidDenom);
        /// They must have a previous delegation to unstake
        let delegated = DELEGATIONS.load(store, validator)?.checked_sub(amount.amount)?;
        DELEGATIONS.save(store, validator, delegated)?;
    }
}
```

### Provider-Side Logic

**TODO** Implementation for sending

---

**TODO** below here is old, can be removed

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

# Cross-Chain Staking Protocol

The staking protocol has two basic operations:

- Stake X tokens on Validator V - `S(V, X)`.
- Unstake X tokens from Validator V - `U(V, X)`.

We want to ensure that at any point in time, when all in-flight messages would be resolved, both the
Provider and the Consumer chain have the same view of the staking state. (**TODO** Consider the
effects of slashing, not included in this document currently).

We also want to guarantee that the Provider chain always maintains sufficient staked tokens
in the vault to cover all virtual staking actions currently outstanding on the Provider chain.

As [mentioned before](./ControlChannel.md#channel-ordering), we wish to use an unordered channel,
and therefore must bring a degree of understanding of [serializability](./Serializability.md)
to this protocol.

## Delegation Syncing

The Consumer side wants to maintain the same counter per validator as the Provider side.
It manages a delegation count per validator - `D[V]`, with the following rules:

- Uninitialized is equivalent to `D[V] = 0`.
- `S(V, X)` => `D[V] += X`, return success ACK.
- `U(V, X)` =>
  - if `D[V] < X`, return error ACK,
  - else `D[V] -= X`, return success ACK.

This is a pretty straightforward counter with a lower bound of 0, along with an increment and decrement
counters.

The only requirements to ensure this stays in sync is that the Provider can successfully commit the same
changes upon a success ACK, and is able to revert them (unstake) on an error ACK.

## Provider Side Design

Beyond this delegation list, the Provider must maintain much more information.
The delegation to a given validator must be mapped to a user on the Provider chain.
This user must also have a sufficient lien on the vault to cover the delegation.
And the vault must have sufficient funds to cover the lien.

This means we must maintain invariants over at least two contracts - the `external-staking` contract
and the `vault`. It includes possible conflicts with non-IBC transactions, like a user withdrawing
collateral from the vault that would be needed to cover the lien for an in-flight delegation.

Let us start analysing the protocol, but viewing all the state transitions that must be made on proper
success ACKs. These same state transitions must always be reverted without problem upon receiving error ACKs.

### Identifying Potential Conflicts

A staking operation would have the following steps and checks:

- Send a lien from `vault` to `external-staking` contract.
  - Ensure there is sufficient collateral to cover max(lien).
  - Ensure there is sufficient collateral in sum(potential slashes).
  - Increase the lien for that given user on the `external-staking` contract.
- Add a delegation in `external-staking` contract.
  - Increase stake count for (user, validator).
  - Increase total stake on the validator.
  - Increase the user's shares in the reward distribution.
- Send an IBC packet to inform the Consumer.
  - Guarantee we can commit all above on success.
  - Guarantee we can roll back all above on error.

An unstaking operation would have the following steps and checks:

- Remove a delegation in `external-staking` contract.
  - Ensure stake count (user, validator) is set and greater than desired unstake amount.
    - Ensure total stake on the validator is set and greater than desired unstake amount (should always be true if above is true).
  - Decrease stake count for (user, validator).
  - Decrease total stake on the validator.
  - Decrease the user's shares in the reward distribution.
  - Add an entry to the (user, validator) "pending unbonding" to withdraw said tokens after the unbonding period.
- Send an IBC packet to inform the Consumer.
  - Guarantee we can commit all above on success.
  - Guarantee we can roll back all above on error.

Possible keys with conflicts are:

- In `vault` - `collateral(user)` and `lien(user, external-staking)`.
- In `external-staking` - `stake(user, validator)`, `total-stake(validator)`, `reward-shares(user)`, `pending-unbonding(user, validator)`.

### Identifying Potential Commutability

The general design should be to write all changes only on a successful ACK, but hold any locks needed to ensure those
writes will not fail in any condition. Using the approach of [Value Ranges](./Serializability.md#value-range), let us analyze
what needs to be minimally enforced here.

For staking, we update:

- `vault::lien(user, external-staking)` => `Maybe(+X)`.
- `external-staking::stake(user, validator)` => `Maybe(+X)`.
- `external-staking::total-stake(validator)` => `Maybe(+X)`.
- `external-staking::reward-shares(validator, user)` => `Maybe(+X)`.

The lower three have no upper limit and thus can never fail, so those operations are always commutative with others valid operations.
`vault::lien` has an upper value, and thus applying `Commit(+X)` could have a conflict with another transaction, so we must "lock" that
value.

For unstaking, we update:

- `external-staking::stake(user, validator)` => `Maybe(-X)`.
- `external-staking::total-stake(validator)` => `Maybe(-X)`.
- `external-staking::reward-shares(validator, user)` => `Maybe(-X)`.
- `external-staking::total-shares(validator)` => `Maybe(-X)`.
- `external-staking::pending-unbonding(user, validator)` => `Append((X, T))`.

We have already seen that appending to an ordered/sorted list is commutative with other valid operations, so we can consider that as
commutative. The other three are all decrements, and may well conflict with another concurrent operation, such as another unstake,
as `Commit(-X)` could potentially fail.

### Minimal Locking

From the above, we have a list of all the values that could possibly cause conflicts with other transactions and thus must be locked:

- `vault::lien(user, external-staking)`.
- `external-staking::stake(user, validator)`.
- `external-staking::total-stake(validator)`.
- `external-staking::reward-shares(validator, user)`.
- `external-staking::total-shares(validator)`.

We can simplify this list by making use of our knowledge of how various states are derived from each other:

- `total-stake(validator)` is the sum of `stake(user, validator)` over all users.
- `reward-shares(validator, user)` is the same as `stake(user, validator)`.
- `total-shares(validator)` is the sum of `reward-shares(validator, user)` over all users.

By analyzing how these other values are derived, we can see that if `stake(user, validator)` is never negative, then
none of the other values can ever be negative either. This means we can simplify those four keys to simply lock `stake(user, validator)`,
which is sufficient to guarantee the invariants of the other staking values, without interfering with actions by other users.

We end up with two keys that need to write locks, both of which only affect a single user:

- `vault::lien(user, external-staking)`.
- `external-staking::stake(user, validator)`.

Note that a write-lock will prevent reading of the value from any other transaction. That means the same user offering a lien on another
validator, which iterates over all liens to find the sum of max slashing, will be blocked until the first transaction completes.
We cannot actually block (or wait) transactions in this way, so we must return an error for the second transaction.

### Idea: Approximating Locks

Holding an actual lock on `vault::lien(user, external-staking)` makes the inter-contract communication rather more complex.
This means, that in addition to the call from vault -> external-staking to stake virtual tokens, we would require that
the external-staking contract **always** calls this contract back either with a commit or rollback associated with that
exact lock (we must store the value pending in vault to not trust the external-staking contract too much).

In this case, we look to approximate the lock by a series of writes that guarantee the invariant is maintained.

- Preparing Phase: `vault::lien(user, external-staking) += X`.
- Commit: do nothing (no message).
- Rollback: (call release_lien) `vault::lien(user, external-staking) -= X`.

The principle questions we need to answer to prove this is safe is:

- Can we guarantee that the rollback will never fail?
- Is there any transaction that would be valid after the preparing phase, but not after the rollback?

For the first one, since the increment is held by our `external-staking` contract, by correct rules of not releasing until the IBC ACK,
we can guarantee that the lien is still held and able to be decremented.

For the second one, the case is some transition that would be effectively using a "Phantom Read" to be valid. I will review the possible
transactions on the vault:

- `bond` - increases collateral.
- `unbond` - decreases collateral after comparing to max(liens).
- `stake_remote` - sends call to stake after comparing to max(liens).
- `stake_local` - sends call to stake after comparing to max(liens).
- `release_local_stake` - reduces lien and adjusts max_lien, slashable.
- `release_remote_stake` - reduces lien and adjusts max_lien, slashable.

Many of these will return errors if the lien is too large to permit said operation. But via inspection, none
would succeed with a larger lien that would not with a smaller one. However, this is not a proof,
and based on the actual implementation of the vault contract rather than any protocol guarantees.

## Protocol Implementation

The messages can be defined as follows:

```rust
enum Op {
    Stake { validator, user, amount },
    Unstake { validator, user, amount },
}
```

One issue here is that the Consumer doesn't care about the user who made the staking action, only the validator.
However, when processing the ACK, the Provider will need to know which user performed the action.
We could store this as local state in the Provider, indexed by the packet sequence, but simply including an extra
field in the packet is much simpler. The Consumer should ignore the user field, but the Provider will receive this original
Packet along with the ACK and be able to properly commit/rollback the staking action.

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

### Provider-Side Staking

This is more complex logic over multiple contracts, so I will only give a high-level overview here rather than the Rust pseudocode:

**TODO**
See [open question above](#idea-approximating-locks) for a discussion on how to possibly avoid locking on staking.
I would like feedback there before defining the actual implementation (which has two possible routes).

### Provider-Side Unstaking

Preparation Phase:

```rust
let mut stake = STAKE.load(store, validator, user)?;
/// write_lock method marks a key as locked, and returns an error if it is already locked
let view = stake.write_lock()?;
/// Enforce the invariant, so we know we can commit later
if view.staked < amount.amount {
    return Err(IbcError::InsufficientFunds);
}
STAKE.save(store, validator, user, &stake)?;
```

Rollback:

```rust
/// Just remove the write_lock from the key
let mut stake = STAKE.load(store, validator, user)?;
stake.release_write_lock()?;
STAKE.save(store, validator, user, &stake)?;
```

Commit:

```rust
/// Remove the write_lock from the key
let mut stake = STAKE.load(store, validator, user)?;
stake.release_write_lock()?;
STAKE.save(store, validator, user, &stake)?;

/// Call the actual staking logic we currently perform
do_stake();
```

### Error correction

**TODO**: Ideas about using values in success ACKs to double check the state matches expectations and flag possible errors.

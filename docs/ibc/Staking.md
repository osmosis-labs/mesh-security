# Cross-Chain Staking Protocol

**TODO** Create this document for unordered conditions!!

## Staking Messages

**TODO**

Once it has the information of the valid set of validators, the main action taken 
by the provider is assigning virtual stake to particular validator and removing it.

It does so via `Stake` and `Unstake` messages, specifying the valoper address it wishes
to delegate to and the amount of local (provider-side, held in the vault) tokens that
it wishes to virtually stake.

The converter must track the total sum of these messages for each validator on the consumer
side, convert them to proper values of native tokens and "virtual stake" them.


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
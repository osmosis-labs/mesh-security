# Slashing Evidence Handling

Note: Slashing will not be part of the MVP rollout, and first implemented in V1. However, we define
proper slashing mechanics here.

## General Architecture

We are worried about a Byzantine consumer chain slashing arbitrary validators on the provider
chain by writing false IBC packets. This could also be done via a bug in the State Machine
and we in general don't want to force the Provider to trust the Consumer state machine. However,
we should trust the Tendermint headers, which is the original source of double-signing
evidence.

Rather than submit IBC packets for Slashing, we require submission of the duplicate
signatures from the Tendermint headers on the Consumer chain. These cannot be forged
unless the private key is compromised.

As of V2, the external staker must have a method to allow submitting such evidence of
double-signing which can be verified and immediately slash all delegators of that
validator.

## Detecting Byzantine Chains

The IBC light clients have a
[built-in mechanism to detect](https://github.com/cosmos/ibc-go/blob/v7.0.1/modules/light-clients/07-tendermint/misbehaviour_handle.go)
if the **entire chain** has gone Byzantine, which is to say that there are two valid
light client proofs for the same height, and over 1/3 of the validators have double-signed.

At such a point, the light client will halt and require governance intervention to
be restored. No packets or ACKs on any channel between those two chains will be
allowed until the governance intervention is complete.

Mesh Security should detect such a state and likewise freeze the relevant `external-staking`
contract, or alternatively slash all delegators to all validators on that chain.
The simple act of freezing the channel will prevent anyone from unstaking, but pending unbondings
will process as usual.

**TODO** decide on proper handling of this case, and see how to get updates from the ibc-go
modules to detect such state.

## Verifying Double Sign

The much more common case we should defend against is a single validator on the consumer
chain signing two different blocks at the same height. This has occurred a number of times
on Cosmos SDK chains and been properly slashed in-protocol by CometBFT.
We need to add identical detection and slashing to the `external-staking` contract.

We can base our checks on the path Comet BFT validates double signing evidence.
It verifies [general aspects of evidence](https://github.com/cometbft/cometbft/blob/v0.37.1/evidence/verify.go#L13-L58)
and then [specific properties of a duplicate vote](https://github.com/cometbft/cometbft/blob/v0.37.1/evidence/verify.go#L156-L222).

To validate, we need [the two votes](https://github.com/cometbft/cometbft/blob/v0.37.1/proto/tendermint/types/evidence.pb.go#L116-L122). Each vote contains many fields about the header,
as [well as the signature](https://github.com/cometbft/cometbft/blob/v0.37.1/proto/tendermint/types/types.pb.go#L468-L477).

The basic checks are:

1. Ensure both votes are by the same validator, same height, and same round, and same vote type (pre-commit).
2. Ensure the Block IDs of the two votes are different.
3. Look up the validator's public key from the validator address (stored in `external-staking`) and ensure this is a valid validator on the consumer chain.
4. Finally, [verify the signature on both votes](https://github.com/cometbft/cometbft/blob/v0.37.1/evidence/verify.go#L211-L219)
   using the public key and the chain-id of the consumer chain (this must be set up in the `external-staking` contract)

We can also add some consistency checks, like "this evidence has not been seen before", which is
equivalent to "this validator has not been tombstoned yet", and maybe some limit on age of
evidence. Or we just accept any age and just use the age of the evidence to decide what is slashed
(based on the unbonding period). Or just slash everyone bonded or unbonding, as the timestamps
of the two votes may be wildly different, and they really shouldn't have trusted this
cheating validator in the first place.

## Trust Assumptions

For the V1 implementation, we will assume not only that the Tendermint headers are valid and can be trusted, but
also that the Consumer chain is not and does not turn Byzantine. This is for simplicity reasons, and to avoid
having to implement a different / independent communication channel for misbehaviour evidence submission.
Once that mechanism is established and implemented, by example as part of ICS, we can revisit this and adapt our implementation
to receive and verify misbehaviour evidence from the Consumer chain on the Provider.

So, this is concerned with a malicious validator on the Consumer chain, double-signing to slash associated delegators
on the Provider chain.

In principle, nothing except for slashing prevents a malicious validator on the Consumer to **intentionally** double-sign.
A user delegating to a malicious validator and then getting slashed is part of the risk of delegation. In the end, this is why
the delegator is getting staking rewards.
As a mitigating factor, the amount of slashing for misbehaviour is defined by the slashing ratio.

Another possibility is, a malicious validator on the Consumer double signing for profit, and trying to **avoid** being slashed.
He could, by example, allocate all or most of his funds through cross-delegators on the Provider, and then tamper with
the validator set updates, so that his public key, or associated block height and times, are invalid. This would prevent the
Provider from slashing him, as the **provided evidence for misbehaviour would fail to verify**.
This last scenario is only possible if the entire Consumer chain goes Byzantine, and included here just for completeness.
It shows that the trust assumptions extend beyond the misbehaviour's evidence, and should include
the validator set updates as well. Along with the Consumer chain itself.

This indicates that, barring Byzantine Consumer chains, it makes sense to re-utilize the same infrastructure and mechanisms
that are used for communication between the Provider and the Consumer, for the specific case of slashing processing.
Both, slashing evidence handling and submission, and validator set updates, share similar trust assumptions and concerns.
They can and must then be part of the same security model.

Similarly, if, for complexity control and auditability, we decide to keep slashing evidence handling and submission
separate from the rest of the Mesh Security infrastructure (in their own smart contracts on Consumer and Provider, by example),
then we should make sure that slashing evidence handling and submission is done by the same entity that is responsible for
validator set updates. So that they can be audited together, and the same trust assumptions apply to both.

## Slashing Handling

As mentioned, for V1, and for simplicity reasons, we will implement a slashing mechanism as part of the existing infrastructure,
namely, the established IBC channel between the `converter` contract on the Consumer and the `external-staking` contract on the Provider.
This will be implemented as a new IBC packet type. Given that we've decided to trust the Consumer chain, the slashing evidence
is assumed to be valid, will not be verified, and therefore doesn't even need to be sent to the Provider. For V1, we simply
obey the slashing commands that come from the Consumer.

The slashing will be submitted by the blockchain from a hook on the Evidence module, through a privileged (`sudo`) message,
to the `virtual-staking` contract, which already has sudo privileges. It will then be routed to the `converter` contract
through a specific message, to be delivered to the `external-staking` contract on the Provider chain over IBC.
This is similar to the way validator set updates are currently being implemented.

The `external-staking` contract will then route the slashing to the `vault` contract, which will slash the associated delegators.

The `vault` contract, along with the `external-staking` contract, has the necessary information to slash the delegators
associated with the misbehaving validator.
The only verification that will be done on the Provider at this point is to check that the validator is not tombstoned at
the misbehaviour's height. This, to avoid processing a single slashing event multiple times.

## Natively Staked Funds

Mesh Security allows for simultaneous native- and cross-staking of funds. This is a powerful feature, and one of the main
appeals behind it. However, it introduces some complications. In the case of a cross-slashing event, by example,
funds may not be available on the vault for some or all of the impacted cross-delegators. For the simple (and legit) reason
that they are being natively staked on the Provider chain at the same time.

The `vault` contract will need to be able to handle this scenario.
In the case of a slashed cross-delegator that doesn't have enough liquidity on the vault, it will need to unbond the funds
from the Provider chain first, and then burn them to execute the slashing.

A kind of "immediate unbonding" mechanism (and associated permission) could be needed for this.
Alternatively, the vault can unbond the funds from the Provider chain and wait for the unbonding period to expire in order to slash them.
This will be effectively the same, as during unbonding those funds are both, blocked from withdrawal, and not providing rewards.

Another option would be for the vault to delegate the slashing in these cases to the blockchain itself.
That is, by using and interacting with the Provider chain's Staking / Slashing module.
This may be complicated to implement, as the slashing evidence and origin are effectively from another chain.

## Slashing on the Native Chain

The Provider side blockchain must inform the vault contract (or the local staking contract) of slashing events,
the same way the Consumer side blockchain(s) informs the Provider of slashing events.

From the point of view of slashing, both local and cross slashing events must be considered and processed almost similarly.
The only difference being that local slashing events don't need to be verified. But local slashing events
must be processed, and its effects on collateral must be updated in the vault for each of the affected delegators.

## Slashing Propagation

Slashing affects the amount of total collateral that every affected delegator has on the vault (or natively staked).
So, it affects the invariants that depend on that collateral. Namely, the maximum lien and / or slashable amount (See [Invariants](../provider/Vault.md#invariants)).

After validating the slashing evidence (in the case of cross-slashing) and executing the slashing,
that is, discounting the slashing amount from the total collateral each affected delegator has, the associated invariants
must be checked and adjusted if needed.
In the case of a broken invariant, a rebalance (unbonding of the now lacking or insufficient funds) must be done to restore it.
This must be checked and rebalanced if needed over all the chains that the affected delegator has funds on.

## Collateral Unbonding and Slashing Propagation Examples

Let's go over some examples to clarify the entire process.

### Scenario 1. Slashed delegator has free collateral on the vault

This is the simplest scenario, as the required funds are available to be slashed.

- `slashing ratio: 10%` (local and external)

 - `collateral: 200`
 - `native staking: 190` (local)
 - `lien holder 1 staking: 150` (external)
   - `validator 1: 100`
   - `validator 2: 50`

 - `max lien: 190` (native)
 - `slashable amount: 190 * 0.10 + 150 * 0.10 = 19 + 15 = 34`
 - `free collateral: 200 - max(34, 190) = 200 - 190 = 10`

`validator 1` is slashed.

 - `slashing amount: 100 * 0.10 = 10`
 - `new collateral: 200 - 10 = 190`
 - `new free collateral: 190 - max(34, 190) = 190 - 190 = 0`

As the invariants are preserved, no native collateral unbonding is needed.

 - `native staking: 190` (equals new collateral)
 - `lien holder 1 staking: 140` (slashing applied)
   - `validator 1: 90` (slashing applied here)
   - `validator 2: 50`

 - `max lien: 190` (recalculated)
 - `slashable amount: 190 * 0.10 + 140 * 0.10 = 19 + 14 = 33` (recalculated)

As the new max lien and slashable amounts are less than or equal to the collateral,
no slashing propagation is needed.

### Scenario 2. Slashed delegator has no free collateral on the vault

- `slashing ratio: 10%` (local and external)

- `collateral: 200`
- `native staking: 200` (local)
- `lien holder 1 staking: 200` (external)
  - `validator 1: 200`

- `max lien: 200` (native and external)
- `slashable amount: 200 * 0.10 + 200 * 0.10 = 20 + 20 = 40`
- `free collateral: 200 - max(40, 200) = 200 - 200 = 0`

`validator 1` is slashed.

- `slashing amount: 200 * 0.10 = 20`
- `new collateral: 200 - 20 = 180`
- `new free collateral: 180 - max(40, 200) = 180 - 200 = -20`

The free collateral is not enough, native collateral unbonding is needed.

- `native staking: 180` (20 are unbonded / burned)
- `lien holder 1 staking: 180` (slashing applied)
  - `validator 1: 130` (slashing applied here)
  - `validator 2: 50`

- `new max lien: 180` (recalculated)
- `new slashable amount: 180 * 0.10 + 180 * 0.10 = 18 + 18 = 36` (recalculated)
- `free collateral: 180 - max(36, 180) = 180 - 180 = 0` (invariants restored)

As the new max lien and slashable amounts are less than or equal to the new collateral,
no slashing propagation is needed.


### Scenario 3. Slashed delegator has some free collateral on the vault

- `slashing ratio: 10%` (local and external)
- `collateral: 200`
- `native staking: 190` (local)
- `lien holder 1 staking: 150` (external)
  - `validator 1: 150`

- `max lien: 190` (native)
- `slashable amount: 190 * 0.10 + 150 * 0.10 = 19 + 15 = 34`
- `free collateral: 200 - max(34, 190) = 200 - 190 = 10`

`validator 1` is slashed.

- `slashing amount: 150 * 0.10 = 15`
- `new collateral: 200 - 15 = 185`
- `new free collateral: 185 - max(34, 190) = 185 - 190 = -5`

The free collateral is not enough, native collateral unbonding is needed.

- `native staking: 185` (5 are unbonded / burned)
- `lien holder 1 staking: 135` (slashing applied)
  - `validator 1: 135` (slashing applied here)

- `new max lien: 185` (recalculated)
- `new slashable amount: 185 * 0.10 + 135 * 0.10 = 18.5 + 13.5 = 32` (recalculated)
- `free collateral: 185 - max(33.5, 185) = 185 - 185 = 0` (invariants restored)

As the new max lien and slashable amounts are less than or equal to the new collateral,
no slashing propagation is needed.


### Scenario 4. Same as Scenario 3, but with more delegations

- `slashing ratio: 10%` (local and external)
- `collateral: 200`
- `native staking: 190` (local)
- `lien holder 1 staking: 180` (external)
  - `validator 1: 140`
  - `validator 2: 40`
- `lien holder 2 staking: 188` (external)
  - `validator 3: 100`
  - `validator 4: 88`

- `max lien: 190` (native)
- `slashable amount: 190 * 0.10 + 180 * 0.10 + 188 * 0.10 = 19 + 18 + 18.8 = 55.8`
- `free collateral: 200 - max(55.8, 190) = 200 - 190 = 10`

`validator 1` is slashed.

- `slashing amount: 140 * 0.10 = 14`
- `new collateral: 200 - 14 = 186`
- `new free collateral: 186 - max(55.8, 190) = 186 - 190 = -4`

The free collateral is not enough, native collateral unbonding is needed.

- `native staking: 186` (4 are unbonded / burned)
- `lien holder 1 staking: 166` (slashing applied)
  - `validator 1: 126` (slashing applied here)
  - `validator 2: 40`
- `lien holder 2 staking: 188`
  - `validator 3: 100`
  - `validator 4: 88`

- `new max lien: 188` (recalculated)
- `new slashable amount: 186 * 0.10 + 166 * 0.10 + 188 * 0.10 = 18.6 + 16.6 + 18.8 = 54` (recalculated)
- `free collateral: 186 - max(54, 188) = 186 - 188 = -2` (invariants broken)

As the new max lien is greater than the new collateral, **slashing propagation is needed**.

- `lien holder 2 staking: 186` (slashing propagation. 2 are unbonded / burned)
  - `validator 3: 99` (slashing propagation (proportional) applied here)
  - `validator 4: 87` (slashing propagation (proportional) applied here)

- `new max lien: 186` (recalculated)
- `new slashable amount: 186 * 0.10 + 166 * 0.10 + 186 * 0.10 = 18.6 + 16.6 + 18.6 = 53.8` (recalculated)
- `free collateral: 186 - max(53.8, 186) = 186 - 186 = 0` (invariants restored)


### Scenario 5. Total slashable greater than max lien

- `slashing ratio: 50%` (local and external)
- `collateral: 200`
- `native staking: 100` (local)
- `lien holder 1 staking: 180` (external)
  - `validator 1: 180`
- `lien holder 2 staking: 50` (external)
  - `validator 2: 50`
- `lien holder 3 staking: 50` (external)
  - `validator 3: 50`

- `max lien: 180` (external 1)
- `slashable amount: 100 * 0.50 + 180 * 0.50 + 50 * 0.50 + 50 * 0.50 = 50 + 90 + 25 + 25 = 190`
- `free collateral: 200 - max(190, 180) = 200 - 190 = 10`

`validator 1` is slashed.

- `slashing amount: 180 * 0.50 = 90`
- `new collateral: 200 - 90 = 110`
- `new free collateral: 110 - max(190, 180) = 110 - 190 = -80`

The free collateral is not enough, native collateral unbonding is needed.

- `native staking: 20` (80 are unbonded / burned)
- `lien holder 1 staking: 90` (slashing applied)
  - `validator 1: 90` (slashing applied here)
- `lien holder 2 staking: 50`
  - `validator 2: 50`
- `lien holder 3 staking: 50`
  - `validator 3: 50`

- `new max lien: 90` (recalculated)
- `new slashable amount: 20 * 0.50 + 90 * 0.50 + 50 * 0.50 + 50 * 0.50 = 10 + 45 + 25 + 25 = 105` (recalculated)
- `free collateral: 110 - max(105, 90) = 110 - 105 = 5` (invariants restored)

As the new max lien and slashable amounts are less than or equal to the new collateral,
no slashing propagation is needed.

### Slashing Process Summary

We can see that there are **three** processes potentially at play during slashing:

1) Slashing itself, which is done by the `vault` contract over the users associated with the slashed validator,
on the corresponding lien holder.
2) Native collateral unbonding, which is done by the `vault` contract over the native staking contract, in case
there's not enough free collateral to cover the slashing amount or part of it.
3) Slashing propagation, which is done by the `vault` contract over the other lien holders associated with the delegator,
in case the slashed collateral is now less than the new max lien or the new slashable amount.

### Native vs. Cross Slashing Process Details

**TODO**

### Effects of Validator Tomstoning During Slashing

**TODO**

# Slashing Evidence Handling

**Note**: Slashing will not be part of the MVP rollout, and first implemented in V1. However, we define
proper slashing mechanics here.

**Note 2**: As of V1, we will assume that the Consumer chain is not Byzantine, and therefore
we will not implement a mechanism to verify slashing evidence. This will be implemented in V2.

## General Architecture

We are worried about a Byzantine consumer chain slashing arbitrary validators on the provider
chain by writing false IBC packets. This could also be done via a bug in the State Machine
and we in general don't want to force the Provider to trust the Consumer state machine. However,
we should trust the Tendermint headers, which is the original source of double-signing
evidence.

Rather than submit IBC packets for Slashing, we require submission of the duplicate
signatures from the Tendermint headers on the Consumer chain. These cannot be forged
unless the private key is compromised.

As of V2, the external staker (on the Provider side) must have a method to allow submitting such evidence of
double-signing which can be verified and immediately slash all delegators of that
validator.

This can be done by following the approach and implementation of InterChain Security (ICS) for
[slashing](https://cosmos.github.io/interchain-security/adrs/adr-013-equivocation-slashing). At the time of writing,
there's already an implementation of ICS misbehaviour handling on the [Hermes relayer](https://github.com/informalsystems/hermes).
We can therefore use that as a reference.

The general idea is that the Relayer will submit the misbehaviour evidence to the Provider chain,
which will then slash the associated delegators. The evidence will be verified by the
Provider chain, and only valid evidence will be accepted and processed.

The submission mechanism is straightforward: the Relayer submits the slashing evidence from the
Consumer to the Provider, and broadcasts it as or as part of a blockchain transaction. So, the Provider
will need to have a slashing module that monitors the chain for a specific "Slashing evidence"
transaction type. The slashing evidence can then be submitted to a smart contract (as a sudo message by example) for
verification and processing.

Please note that the actual slashing implementation will not change. Only the slashing evidence
handling, submission and verification will need to be implemented as part of V2.

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

**Note**: Here we can again refer to the ICS specs / impl for reference:
https://cosmos.github.io/interchain-security/adrs/adr-005-cryptographic-equivocation-verification

## Trust Assumptions

For the V1 implementation, we will assume not only that the Tendermint headers are valid and can be trusted, but
also that the Consumer chain is not and does not turn Byzantine. This is for simplicity reasons, and to avoid
having to implement a different / independent communication channel for misbehaviour evidence submission.
Once that mechanism is established and implemented, by example as part of ICS, we can revisit this and adapt our implementation
to receive and verify misbehaviour evidence from the Consumer chain on the Provider.

**Note**: As of this update (28/11/23) this mechanism has already been implemented as part of the Hermes relayer.

So, this is concerned with a malicious validator on the Consumer chain, double-signing to slash associated delegators
on the Provider chain.

In principle, nothing except for slashing prevents a malicious validator on the Consumer to **intentionally** double-sign.
A user delegating to a malicious validator and then getting slashed is part of the risk of delegation. In the end, this is why
the delegator is getting staking rewards.
As a mitigating factor, the amount of slashing for misbehaviour is defined by the slashing ratio.

Another possibility is, a malicious validator on the Consumer double sign intentionally, and try to **avoid** being slashed.
He could, by example, allocate all or most of his funds through cross-delegators on the Provider, and then tamper with
the validator set updates, so that his public key, or associated block height and times, are invalid. This would prevent the
Provider from slashing him, as the **provided evidence for misbehaviour would fail to verify**.
This last scenario is only possible if the entire Consumer chain goes Byzantine, and included here just for completeness.
It shows that the trust assumptions extend beyond the misbehaviour's evidence, and should include
the validator set updates as well. Along with the Consumer chain itself.

**Note**: As of this update (28/11/23) AFAIK this scenario is already being handled by the Hermes relayer.
The misbehaviour evidence is verified against the validator set at the time of the misbehaviour, and the
validator public key, which is needed for verification, is included in the evidence itself.

All this indicates that, barring Byzantine Consumer chains, it makes sense to re-utilize the same infrastructure and mechanisms
that are used for communication between the Provider and the Consumer, for the specific case of slashing processing.
Both, slashing evidence handling and submission, and validator set updates, share similar trust assumptions and concerns.
They can and must then be part of the same security model.

Similarly, if, for complexity control and auditability, we decide to keep slashing evidence handling and submission
separate from the rest of the Mesh Security infrastructure (in their own smart contracts on Consumer and Provider, by example),
then we should make sure that slashing evidence handling and submission is done by the same entity that is responsible for
validator set updates. So that they can be audited together, and the same trust assumptions apply to both.

**Note**: As of this update (28/11/23) this is already the case. The Hermes relayer is responsible for both, validator public key
and misbehaviour evidence handling and submission.

**Note 2**: Another possibility in this scenario is a malicious validator on the Consumer relaying and submitting false / forged misbehaviour evidence to the Provider,
through the Relayer. That is, tricking the Relayer into submitting false slashing evidence to the Provider, in order to slash associated Provider's delegators.
This is in principle possible, and called a "Nothing at Stake" attack (since the validator has in principle nothing to lose on the Consumer chain when doing this).
To prevent against this, the Relayer must broadcast / replay the slashing evidence it gets on the Consumer chain. This way, if a validator forged
the evidenced, it will be slashed on the Consumer side anyway; so that it's no longer a "nothing at stake" scenario.

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

**Note**: As of this update (28/11/23) this is the current setup. Instead of burning the funds, they are unbonded from the Provider chain,
and kept in the Native staking contract. An accounting trick is being done, to avoid the need to burn the funds
and to discount the slashing amount from the total collateral of the delegator.

A kind of "immediate unbonding" mechanism (and associated permission) could be needed for this.
Alternatively, the vault can unbond the funds from the Provider chain and wait for the unbonding period to expire in order to slash them.
This will be effectively the same, as during unbonding those funds are both, blocked from withdrawal, and not providing rewards.

**Note**: This is currently (V1) being done using a normal unbonding period, which is not ideal, as the funds are being doubly discounted
by the slashing amount during the unbonding period. This will be fixed in V2, by implementing "immediate unbonding" on the Provider chain.

Another option would be for the vault to delegate the slashing in these cases to the blockchain itself.
That is, by using and interacting with the Provider chain's Staking / Slashing module.
This may be complicated to implement, as the slashing evidence and origin are effectively from another chain.

## Slashing on the Native Chain

The Provider side blockchain must inform the vault contract (or the local staking contract) of slashing events,
the same way the Consumer side blockchain(s) informs the Provider of slashing events.

From the point of view of slashing, both local and cross slashing events must be considered and processed almost similarly.
The only difference being that local slashing events don't need to be verified. But local slashing events
must be processed, and its effects on collateral must be updated in the vault for each of the affected delegators.

**Note**: This is not yet implemented as of V1. It will be implemented as part of V2.

## Slashing Propagation

Slashing affects the amount of total collateral that every affected delegator has on the vault (or natively staked).
So, it affects the invariants that depend on that collateral. Namely, the maximum lien and / or slashable amount (See [Invariants](../provider/Vault.md#invariants)).

After validating the slashing evidence (in the case of cross-slashing) and executing the slashing,
that is, discounting the slashing amount from the total collateral each affected delegator has, the associated invariants
must be checked and adjusted if needed.
In the case of a broken invariant, a rebalance (unbonding of the now lacking or insufficient funds) must be done to restore it.
This must be checked and rebalanced if needed, over all the chains that the affected delegator has funds on.

**Note**: Both Slashing accounting and Slashing propagation accounting have been implemented as part of V1.

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

- `max lien: 190` (local)
- `slashable amount: 190 * 0.10 + 150 * 0.10 = 19 + 15 = 34`
- `free collateral: 200 - max(34, 190) = 200 - 190 = 10`

`validator 1` is slashed.

- `slashing amount: 100 * 0.10 = 10`
- `new collateral: 200 - 10 = 190`
- `new max lien: 190` (after slashing)
- `new slashable amount: 190 * 0.10 + 140 * 0.10 = 19 + 14 = 33` (after slashing)
- `new free collateral: 190 - max(33, 190) = 190 - 190 = 0` (invariants preserved)

As the invariants are preserved, no slashing propagation is needed.


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
- `new max lien: 200` (unchanged)
- `new slashable amount: 200 * 0.10 + 180 * 0.10 = 20 + 18 = 38` (after slashing)
- `new free collateral: 180 - max(38, 200) = 180 - 200 = -20`

Free collateral is not enough, slashing propagation is needed.
Since max lien is the reason of the broken invariant, liens adjustment will be done.

- `native staking: 180` (20 are unbonded / burned)
- `lien holder 1 staking: 180` (slashing applied)
  - `validator 1: 180` (slashing applied here)

- `new max lien: 180` (recalculated)
- `new slashable amount: 180 * 0.10 + 180 * 0.10 = 18 + 18 = 36` (recalculated)
- `free collateral: 180 - max(36, 180) = 180 - 180 = 0` (invariants restored)


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
- `new max lien: 190` (unchanged)
- `new slashable amount: 190 * 0.10 + 135 * 0.10 = 19 + 13.5 = 32.5` (after slashing)
- `new free collateral: 185 - max(32.5, 190) = 185 - 190 = -5`

Free collateral is not enough, slashing propagation is needed.
Since max lien is the reason of the broken invariant, liens adjustment will be done.

- `native staking: 185` (5 are unbonded / burned)
- `lien holder 1 staking: 135` (slashing applied)
  - `validator 1: 135` (slashing applied here)

- `new max lien: 185` (recalculated)
- `new slashable amount: 185 * 0.10 + 135 * 0.10 = 18.5 + 13.5 = 32` (recalculated)
- `free collateral: 185 - max(32, 185) = 185 - 185 = 0` (invariants restored)


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
- `new max lien: 190` (unchanged)
- `new slashable amount: 190 * 0.10 + 166 * 0.10 + 188 * 0.10 = 19 + 16.6 + 18.8 = 54.4` (after slashing)
- `new free collateral: 186 - max(54.4, 190) = 186 - 190 = -4`

Free collateral is not enough, slashing propagation is needed.
Since max lien is the reason of the broken invariant, liens adjustment will be done.

- `native staking: 186` (4 are unbonded / burned)
- `lien holder 1 staking: 166` (slashing applied)
  - `validator 1: 126` (slashing applied here)
  - `validator 2: 40`
- `lien holder 2 staking: 186` (2 are unbonded / burned)
  - `validator 3: 99` (proportional)
  - `validator 4: 87` (proportional)

- `new max lien: 186` (recalculated)
- `new slashable amount: 186 * 0.10 + 166 * 0.10 + 186 * 0.10 = 18.6 + 16.6 + 18.6 = 53.8` (recalculated)
- `free collateral: 186 - max(53.8, 186) = 186 - 186 = 0` (invariants restored)


### Scenario 5. Total slashable greater than max lien

- `native slashing ratio: 10%` (local)
- `cross slashing ratio: 50%` (external)
- `collateral: 200`
- `native staking: 100` (local)
- `lien holder 1 staking: 180` (external)
  - `validator 1: 180`
- `lien holder 2 staking: 80` (external)
  - `validator 2: 80`
- `lien holder 3 staking: 100` (external)
  - `validator 3: 100`

- `max lien: 180` (external 1)
- `slashable amount: 100 * 0.10 + 180 * 0.50 + 80 * 0.50 + 100 * 0.50 = 10 + 90 + 40 + 50 = 190`
- `free collateral: 200 - max(190, 180) = 200 - 190 = 10`

`validator 1` is slashed.

- `slashing amount: 180 * 0.50 = 90`
- `new collateral: 200 - 90 = 110`
- `new max lien: 100` (after slashing)
- `new slashable amount: 100 * 0.10 + 90 * 0.50 + 80 * 0.50 + 100 * 0.50 = 10 + 45 + 40 + 50 = 145` (after slashing)
- `new free collateral: 110 - max(145, 100) = 110 - 145 = -35` (invariants broken)

Invariants are broken, slashing propagation is needed.
Given that total slashable is greater than max lien, slashing propagation over the liens will be proportional
to the aggregated slash ratios of all the lien holders.

- `sum of slash ratios: 0.1 + 0.5 + 0.5 + 0.5 = 1.6`
- `native staking: 100 - 35 / 1.6 =  100 - 21.875 = 78.125 ` (proportionally adjusted)
- `lien holder 1 staking: 90 - 35 / 1.6 = 90 - 21.875 = 68.125 ` (slashed and proportionally adjusted)
  - `validator 1: 68.125` (slashed and adjusted here)
- `lien holder 2 staking: 80 - 35 / 1.6 = 80 - 21.875 - 58.125` (proportionally adjusted)
  - `validator 2: 58.125` (proportionally adjusted here)
- `lien holder 3 staking: 100 - 35 / 1.6 = 100 - 21.875 = 78.125` (proportionally adjusted)
  - `validator 3: 78.125`

- `new max lien: 78.125` (recalculated)
- `new slashable amount: 78.125 * 0.1 + 68.125 * 0.5 + 58.125 * 0.5 + 78.125 * 0.5 = 7.8125 + 34.0625 + 29.0625 + 39.0625 = 110.0` (recalculated)
- `new free collateral: 110 - max(110, 78.125) = 110 - 110 = 0` (invariants restored)

## Slashing Process Summary

We can see that there are two processes potentially at play, at the smart contracts level, during slashing:

1) Slashing accounting itself, which is done by the `vault` contract, over the users associated with
the slashed validator, on the corresponding lien holder.
Slashing accounting is also adjusted "in passing" (while forwarding the slashing to the `vault` contract),
in both the `virtual-staking` contract on the Consumer, and the `external-staking` contract on the Provider.

2) Slashing propagation, which is done by the `vault` contract over the other lien holders associated
with the delegator or delegators that are associated in turn to the slashed validator. This happens
in case the slashed collateral in point 1), is now less than the new max lien, or the new slashable amount.

This process starts at the `vault` contract, and is propagated to the `virtual-staking` contract on the Consumer.
It can also involve the native staking contracts on the Provider, if the slashed collateral is natively staked.
This in turn involves a kind of on-chain **burn** mechanism for the slashed funds; which is currently being done
by unbonding them, and discounting them from the total collateral of the delegator.

**Note**: This unbonding mechanism is already "immediate unbonding" on the Consumer side, and will be implemented
as such on the Provider side as well, as part of V2 (or earlier).

Depending on which is the reason for the broken invariant (either max lien greater than collateral, or total slashable amount greater than collateral),
slashing propagation will be done differently:
Either, by **adjusting the offending liens** to be below the collateral. Or, by **proportionally adjusting all the liens**, so that the sum of
the resulting sum of slashable amounts is below the collateral.

### Native vs. Cross Slashing

Native vs. Cross Slashing processing and effects are similar, and are being implemented in the same way.
The main difference is that as of V1, we currently lack a **native** `x/meshsecurity` module (that is, for the Provider blockchain),
and therefore cannot do immediate unbonding. This will be implemented as part of V2.

### Effects of Validator Tombstoning During Slashing

Validator tombstoning, when or as a consequence of double signing, permanently removes the validator from the validator set.
Validator jailing, when or as a consequence of offline detection, also temporarily removes the validator from the active validator set.

Both events also lead to slashing, with different slash ratios for each misbehaviour. Only active validators
can be slashed, and this is a check that is currently done as part of the slashing process.

This means that if a validator is tombstoned or jailed during slashing, it has to be slashed first, and then tombstoned or jailed.
This is because the slashing process is done over the active validator set, and the validator must be active at the time of slashing.

This is currently being implemented this way, for cross-validators, in both the `virtual-staking` contract on the Consumer,
and the `external-staking` contract on the Provider.

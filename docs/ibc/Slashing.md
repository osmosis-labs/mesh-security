# Slashing Evidence Handling

Note: Slashing will not be part of the MVP rollout, and first implemented in v1. However, we define
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

As of v1, the external staker must have a method to allow submitting such evidence of
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

For the v1 implementation, we will assume that the Consumer chain is not and does not turn Byzantine,
and that the Tendermint headers are valid and can be trusted.

So, this is concerned with a malicious validator on the Consumer chain, double-signing to slash associated delegators
on the Provider chain.
In principle, nothing prevents a malicious validator on the Consumer to **intentionally** double-sign. So, the only
thing we can do is limit the damage it can do.
The amount of slashing for misbehaviour is defined by the slashing ratio. Additionally, we can rate limit the amount
of slashing events, to one slashing event per day or similar. That will limit the damage a malicious validator
can do to Provider funds, and will give time to cross-delegators on the Provider to unbond from that malicious validator.

Another possibility is, a malicious validator on the Consumer double signing for profit, and trying to **avoid** being slashed.
He could, by example, allocate all or most of his funds through cross-delegators on the Provider, and then tamper with
the validator set updates, so that his public key, or associated block height and times, are invalid. This would prevent the
Provider from slashing him, as the **provided evidence for misbehaviour would fail to verify**.
This is a complex scenario, included here for completeness. It shows that the trust assumptions extend beyond the
misbehaviour's evidence, and should include the validator set updates as well. Along with the Consumer chain itself.

This indicates that it makes sense to re-utilize the same infrastructure and mechanisms that are used for
communication between the Provider and the Consumer, for the specific case of slashing evidence submission.
Both, slashing evidence handling and submission, and validator set updates, share similar trust assumptions and concerns.
They should then be part of the same security model.

Conversely, if, for complexity control and auditability, we decide to keep slashing evidence handling and submission
separate from the rest of the Mesh Security infrastructure (in their own smart contracts on Consumer and Provider, by example),
then we should make sure that slashing evidence handling and submission is done by the same entity that is responsible for
validator set updates. So that they can be audited together, and the same trust assumptions apply to both.

## Slashing Evidence Handling

For v1, and for simplicity reasons, we will implement a slashing evidence handling mechanism as part of the existing infrastructure,
namely, the established IBC channel between the `converter` contract on the Consumer and the `external-staking` contract on the Provider.
This will be implemented as a new IBC packet type.

The actual evidence will be submitted by the blockchain from a hook on the Evidence module, through a privileged (`sudo`) message
to the `virtual-staking` contract, which already has sudo privileges. It will then be routed to the `converter` contract through a specific message,
to be delivered to the `external-staking` contract on the Provider chain over IBC.
This is similar to the way validator set updates are currently being implemented.

The `external-staking` contract will then route the evidence to the `vault` contract, which will verify it and slash the associated delegators
if the evidence is valid.
The `vault` contract has the necessary information to verify the evidence, namely, the updated validator set.
It also has the mapping between the offending validator and the associated cross-delegators.

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

Finally, another option would be, for the vault to delegate the slashing in these cases to the blockchain itself.
That is, by using and interacting with the Provider chain's Staking / Slashing module.
This may be complicated to implement, as the slashing evidence and origin are effectively from another chain.

# Slashing Evidence Handling

Note: Slashing will not be part of the MVP rollout, and first implemented in v1. However, we define
proper slashing mechanics here.

## General Architecture

We are worried about a Byzantine consumer chain slashing arbitrary validators on the provider
chain by writing false IBC packets. This could also be done via a bug in the State Machine
and we in general don't want force the Provider to trust the Consumer state machine. However,
we should trust the Tendermint headers, which is the original source of double-signing
evidence.

Rather than submit IBC packets for Slashing, we require submission of the duplicate
signatures from the Tendermint headers on the Consumer chain. These cannot be forged
unless the private key is compromised.

As of V1, the external staker must have a method to allow submitting such evidence of
double-signing which can be verified and immediately slash all delegators of that
validator.

## Detecting Byzantine Chains

The IBC light clients have a
[built-in mechanism to detect](https://github.com/cosmos/ibc-go/blob/v7.0.1/modules/light-clients/07-tendermint/misbehaviour_handle.go)
if the **entire chain** has gone Byzantine, which is to say that there are two valid
light client proofs for the same height, and over 1/3 of the validators have double-signed.

At such a point, the light client will halt and require governance intervention to
be restored. No packets or acks on any channel between those two chains will be
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

1. Ensure both votes are by the same validator, same height, and same round, and same vote type (precommit)
2. Ensure the Block IDs of the two votes are different
3. Look up the validator's public key from the validator address (stored in `external-staking`) and ensure this is a valid validator on the consumer chain
4. Finally, [verify the signature on both votes](https://github.com/cometbft/cometbft/blob/v0.37.1/evidence/verify.go#L211-L219) using the public key and the chain-id of the consumer chain (this must be set up in the `external-staking` contract)

We can also add some consistency checks, like "this evidence has not been seen before", which is
equivalent to "this validator has not been tombstoned yet", and maybe some limit on age of
evidence. Or we just accept any age and just use the age of the evidence to decide what is slashed
(based on the unbonding period). Or just slash everyone bonded or unbonding, as the timestamps
of the two votes may be wildly different, and they really shouldn't have trusted this
cheating validator in the first place.

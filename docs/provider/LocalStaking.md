# Local Staking (i.e. Native Staking)

A local / native staking contract connects to a [Vault](./Vault.md). Unlike external stakers, it actually
accepts the native token along with a claim on it. It manages staking the vault tokens to the native
protocol and returns them when finished unbonding.

# Description

Native staking is composed of two contracts: a staking contract and a staking proxy contract,
which is instantiated on behalf of each user. This is so to give each user the ability to
manage their own funds, and perform actions associated with them (i.e. unstaking, voting, etc).

# Transitions

## Native Staking Contract

**Stake (i.e. `receive_stake`)**

Receives stake (`info.funds`) from the vault contract on behalf of the user, and performs the action
specified in the accompanying `msg`.
`msg` is custom to each implementation of the staking contract, and opaque to the vault.
Typically, it will be a `StakeMsg` message containing the validator to stake to.

This will be staked to the native protocol of the blockchain (through a native-staking-proxy contract),
and the vault will have a claim on the staked tokens.

**Unstake (i.e. `release_proxy_stake`)**

This accepts tokens sent back from the native-staking-proxy contract (through `info.funds`).
The native-staking contract can determine which user they belong to via an internal map.
It will then send those tokens back to the vault, and release the associated claim.

## Native Staking Proxy Contract

**Stake (i.e. `stake`)**

Stakes the tokens from `info.funds` to the given validator. Can only be called by the parent contract, 
i.e. the native-staking contract.

This performs the actual staking action, and holds the user's funds.

**Restake (i.e. `restake`)**

Re-stakes the given amount from the one validator to another on behalf of the calling user.
Returns an error if the user doesn't have enough stake.

**Vote (i.e. `vote`)**

Vote with the user's stake (over all delegations).

**Weighted Vote (i.e. `vote_weighted`)**

Vote with the user's stake (over all delegations),
with a given weight.

**Withdraw Rewards (i.e. `withdraw_rewards`)**

If the caller has any delegations, withdraw all rewards from those delegations and
send the tokens to the calling user.

**Unstake (i.e. `unstake`)**

Unstakes the given amount from the given validator on behalf of the calling user.
Returns an error if the user doesn't have such a stake.

After the unbonding period, it will allow the user to claim the tokens (returning
them to the vault).

**Release Unbonded (i.e. `release_unbonded`)**

Releases any tokens that have fully unbonded from a previous `unstake`.
The funds will go back to the parent (the native-staking contract) via `release_proxy_stake`.
Errors if the proxy doesn't have any liquid tokens.

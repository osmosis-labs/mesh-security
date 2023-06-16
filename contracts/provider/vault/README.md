# Vault

This is the central entrypoint to Mesh Security on the Provider side. Users can provide collateral to this contract,
which then allows you to use that collateral to both stake locally (see [mesh-native-staking](../native-staking)),
as well as cross-stake.

## Definitions

Vault Denom - The token we use as collateral. Generally the native staking token (but there might be
cases for IBC assets, eg. ETH, BTC, or USDC).

Collateral - Total amount of tokens deposited by an account.

Local Staking Contract - One contract per vault that can receive the collateral to be staked
on the native staking system. The tokens moved here can only be staked and count towards the
account's collateral.

Cross Staking Contract - Any number of contracts per vault that receive Liens on the collateral.

Lien - A claim on some collateral (from local- or cross-staking contract). Collateral with existing
liens may not be withdrawn.

## Workflow

Deposit - A user deposits the vault denom to provide some collateral to their account

Stake Locally - A user triggers a local staking action to a chosen validator. They then
can manage their delegation and vote via the local staking contract.

Cross Stake - A user pulls out additional liens on the same collateral "cross staking" it
on different chains.

Withdraw Rewards - A user can interact with the local- and cross-staking contracts to withdraw
their staking rewards

Release cross stake - Must be called by a cross-staking contract that holds an existing lien
on the collateral, when it is no longer being used for cross-staking. This reduces/removes
the existing lien

Release local stake - Must be called by the local-staking contract along with the original vault
tokens. It releases the lien held by the local staking contract.

Withdraw collateral - Once outstanding liens on the user's collateral have been released, the
user can withdraw collateral. If the user deposited say 100 tokens and there are liens out
for 60 still, then they can withdraw 40 tokens.

## Invariants

There are tokens in the system to match the collateral (accounting amount).
`sum(collateral) <= balance(vault) + balance(local staking)`, where balance includes delegations.
This implies a slashing event on local staking must reduce collateral.

For each user, they have collateral equal to or greater than every lien.
`max(liens(user)) <= collateral(user)`

For each user, the total amount of potential slashing over all liens is less than or
equal to their total collateral (important if doing many cross-stakes, or with high slashing rates):
`liens(user).map(|x| x.lien * x.max_slashing_rate).sum() <= collateral(user)`

## Future Work

Propagation of Slashing

**TODO**

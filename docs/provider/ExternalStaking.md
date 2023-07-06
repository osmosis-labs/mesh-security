# External Staking

An external staking contract connects to a [Vault](./Vault.md).
It manages staking and staking the vault's (virtual) tokens to the remote protocol,
and releases the virtual tokens when finished unbonding.

It also manages distribution of rewards coming from the remote (consumer) chain,
and transferring of those rewards to remote recipients on the consumer chain.

## Transitions

**Stake (i.e. `receive_virtual_stake`)**

Receives virtual stake from the vault contract on behalf of the user, and performs the action
specified in the accompanying `msg`.
`msg` is custom to each implementation of the external staking contract, and opaque to the vault.
Typically, it will be a `ReceiveVirtualStakeMsg` message containing the remote validator to stake to.

This will be staked to the native protocol of the remote/external blockchain.
The vault will hold a lien on the remotely staked tokens, which allows for
multiple remote staking of the same funds.

**Unstake (i.e. `unstake`)**

Schedules tokens for release, adding them to the pending unbonds. After the
unbonding period passes, funds are ready to be released, which is accomplished
with a `withdraw_unbonded` call by the user.

**Withdraw Unbonded (i.e. `withdraw_unbonded`)**

Withdraws all released tokens to the calling user.

Tokens to be claimed have to be unbond before, by calling the `unstake` message and
waiting for the unbonding period.

**Withdraw Rewards (i.e. `withdraw_rewards`)**

Withdraws the rewards that are the result of staking via a given external validator.

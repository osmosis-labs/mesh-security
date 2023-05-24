# Native Staking

This is the principle implementation of Local Staking in the Mesh Security design. It allows
using the collateral in the Vault to stake to validators of your choice on the local chain.
The only other implementation of Native Staking currently imagined is staking a DAO's cw20 token
in the DAO staking contract.

## Interaction with Vault

There are three main interactions here.

First, the native staking contract is designed to be instantiated directly by the Vault during the process
of creating the vault. Since we need a 1:1 relationship between the two and links in both directions,
the only secure way of doing that is to instantiate them both in one transaction. Upon creation, the vault
will query for the max_slashing value of the staking contract and store it. **max_slashing should not change during
the lifetime of the contract**.

Second, the vault can send funds to the native staking contract to manage on behalf of a user. This is done via the
`receive_stake` method on the native staking contract. The contract receives the actual staking tokens in `info.funds`
as part of that method call.

Third, the native staking contract can release said stake back to the vault. This is done by calling `release_local_stake`
on the Vault contract while sending the actual staking tokens back to the Vault in `info.funds`.

## Interaction with SDK Staking

A critical part of the design of Mesh Security involves ensuring the user can still interact with the local staking just as well as if they
staked directly. That means they should be able to select multiple validators, re-stake between them, withdraw rewards as they wish,
and vote in on-chain governance. We limit them to not pull out liquid staking tokens or unbond to anyone but the native staking
contract, as the actual collateral must be held here. Otherwise, they should have full autonomy over the use of that stake.

Such control precludes managing one large pool in the native staking contract on behalf of multiple users. Rather than that, this
will create virtual accounts (via [`native-staking-proxy`](../native-staking-proxy)) for each user that sent stake and allow the
user to interact with this virtual account to manage their stake, up until the point of unbonding and sending the collateral back to
the vault to release the claim.

This means when `native-staking` receives stake on behalf of a user, it looks up to see if there exists a proxy account for that user already.
If so, it will simply pass the funds to that contract along with the message of which validator to stake to. If not, it will instantiate
a new `native-staking-proxy` for that user and pass it the funds along with validator information. 

The user then interacts directly with the proxy up to the point of a complete unbonding, when they can send the now-liquid tokens
to `native-staking`, which will immediately release those on the vault. This makes `native-staking` more of a pass-through node and
"air traffic controller", while logic to interact with staking and governance is placed inside the `native-staking-proxy` contract.

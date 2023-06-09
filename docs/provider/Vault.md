# Vault

The entry point of Mesh Security is the **Vault**. This is where a potential
staker can provide collateral in the form of native tokens, with which he or she wants
to stake on multiple chains.

Connected to the _Vault_ contract, is exactly one [Local Staking](./LocalStaking.md) contract
which can delegate the actual token to the native staking module. It also can connect to an
arbitrary number of [External Staking](./ExternalStaking.md) contracts which can make use
of said collateral as "virtual stake" to use in an external staking system (one that doesn't
directly use the vault token as collateral).

The key here is that we can safely use the
same collateral for multiple protocols, as the maximum slashing penalty is significantly
below 100%. If double-signing is penalized by a 5% slash (typical in Cosmos SDK chains),
then one could safely use the same collateral to provide security to 20 chains, as even
if every validator that used that collateral double-signed, there would still be enough
stake to slash to cover that security promise.

Looking at [extending the concept of mesh security to local DAOs](./DAOs.md),
we see that there may be many different implementations of both the _Local Staking_ concept and
the _External Staking_ concept. However, we must define
standard interfaces here that can plug into the Vault.

We define this interface as a _Lien Holder_ (as it accepts liens, i.e. embargable amounts).

## Definitions

- **Vault** - The main contract that holds all the collateral and liens.
- **Native Token** - The native staking token of this blockchain. More specifically,
  the token in which all collateral is measured.
- **Local Staking** - A contract that can delegate the native token to the native staking module.
- **External Staking** - A contract that can use the collateral as "virtual stake" in an external staking system.
- **User** - A user of the vault that provides collateral, to either the local staking or an external staking system.
- **Lien** - A promise to provide some amount of collateral to a lien holder.
- **Lien Holder** - A contract that accepts liens and can slash the collateral.
- **Collateral** - The total amount of collateral, in the native token, for a given user.
- **Slashable Collateral** - The total amount of collateral that can be slashed, in the native token, for a given user.
  `Liens(user).map(|x| x.amount * x.slashable).sum()`.
- **Maximum Lien** - The maximum lien of all liens a user has.
  `Liens(user).map(|x| x.amount).max()`.
- **Free Collateral** - The total collateral a user has, minus either their slashable collateral or their maximum lien (whatever is bigger).
  `Collateral(user) - max(SlashableCollateral(user), MaximumLien(user))`.

## Design Decisions

The _vault_ contract requires one canonical _Local Staking_ contract to be defined when it is
created, and this contract address cannot be changed.

The _vault_ contract doesn't require the _External Stakers_ to be pre-registered. Each user can decide
which external staker it trusts with their tokens. (We will provide guidance in the UI to only
show "recommended" externals, but do not enforce at the contract level, if someone wants to build their own UI).

The _vault_ contract enforces the maximum amount a given Lien Holder can slash to whatever was
agreed upon when making the lien.

The _vault_ contract will only release a lien when requested by the lien holder. (No auto-release override).

The _vault_ contract may force-reduce the size of the lien only in the case of slashing by another lien holder.
The lien holders must be able to handle this case properly.

The _vault_ contract ensures the user's collateral is sufficient to service all the liens
made on said collateral.

The _vault_ contract may have a parameter to limit slashable collateral or maximum lien to less than
100% of the size of the collateral. This makes handling a small slash condition much simpler.

The _lien holder_ is informed of a new lien, and may reject it by returning an error
(e.g. if the slashing percentage is too small, or if the total credit would be too high).

The _vault_ may slash the collateral associated to a given lien holder, up to the agreed upon amount when it was lent out.

The _vault_ should release the lien once the lien holder terminates any agreement with the user.

## Implementation

- [Vault](../../contracts/provider/vault/src/contract.rs).
- [Local Staking](../../contracts/provider/native-staking/src/contract.rs).
- [External Staking](../../contracts/provider/external-staking/src/contract.rs).

### State

- Config: General contract configuration.
- LocalStaking: Local staking info.
- Liens: All liens in the protocol. Liens are indexed with (user, lien_holder), as this pair has to be unique.
- Users: Per-user information. Collateral, max lien, total slashable amount, total used collateral and free collateral.
- Txs: Pending txs information.

### Invariants

- `SlashableCollateral(user) <= Collateral(user)` - for all users.
- `MaximumLien(user) <= Collateral(user)` - for all users.
- `Liens(user).map(|x| x.lien_holder).isUnique()` - for all users.

### Transitions

**Provide Collateral (i.e. `bond`)**

Any user may deposit native tokens to the vault contract,
thus increasing their collateral as stored in this contract.

**Withdraw Collateral (i.e. `unbond`)**

Any user may withdraw any _Free Collateral_ credited to their account.
Their collateral is reduced by this amount and these native tokens are
immediately transferred to their account.

**Provide Lien (i.e. `stake_local`)**

This sends actual tokens to the local staking contract for native staking.
Native staking for a user is handled through a proxy contract (i.e. `native_staking_proxy`),
which is owned by the native staking contract, and instantiated for each user.

Available collateral and related quantities are updated locally to the vault.

**Provide Lien (i.e. `stake_remote`)**

Promises collateral as slashable to some lien holder. It is the responsibility of the vault to guarantee that
that promise can be fulfilled (i.e. that the associated slashable amount is always available for eventual slashing).

This is updated locally to the vault.

**Release Local Stake (i.e. `release_local_stake`)**

Local unstaking is initiated by the user on their corresponding native-staking-proxy contract (i.e. native-staking-proxy `unstake` handler).
This is because unstaking requires an unbonding period to be enforced. Only after that unbonding period is over, the user's native-staking-proxy
contract allows the user to withdraw the unbonded amounts (i.e. `withdraw_unbonded`). And only then it sends a message to the vault contract,
to release the associated "lien" (Properly, a claim. The implementation uses "lien" indistinctly, for simplicity) (i.e. `release_local_stake`).

**Release Lien (i.e. `release_cross_stake`)**

Remote unstaking is initiated by the user on the external-staking contract (i.e. external-staking `unstake` handler).
This is because unstaking requires an unbonding period to be enforced. Only after that unbonding period is over, the external-staking
contract allows the user to withdraw the unbonded amounts (i.e. `withdraw_unbonded`). And only then it sends a message to the vault contract,
to release the associated lien (i.e. `release_cross_stake`).

**Commit Tx (i.e. `commit_tx`)**
Though this is a public handler, it is only meant to be called by external-staking contracts. This finalises the remote staking process
successfully, and updates the vault state accordingly.

**Rollback Tx (i.e. `rollback_tx`)**
Though this is a public handler, it is only meant to be called by external-staking contracts. This aborts the remote staking process
in case of error, and rollbacks the vault state accordingly.

**Slash**

TODO: Slashing is not part of MVP, and will be implemented in a future version of mesh-security.

- Increase Slashing(user, lien_holder)?

TODO: Increase slashing is not part of MVP, and will be implemented in a future version of mesh-security.

## Footnotes

For MVP, Slashable Collateral and Maximum Lien can be up to 100% of total Collateral.

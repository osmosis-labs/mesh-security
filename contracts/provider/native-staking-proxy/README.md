# Native Staking Proxy

This can only be understood in the context of the [native-staking contract](../native-staking).
Basically, it allows a per-user proxy to hold that user's stake. It allows the user full control
of the stake, including re-staking and voting, and will trigger a release on the vault when the stake
is fully unbonded.

## Interaction with SDK Staking

The user should be able to do the following as if they were staking normally:

- re-stake
- withdraw rewards
- vote
- vote weighted

The staking process is triggered by the vault sending tokens to the native-staking contract that sends them to this one.

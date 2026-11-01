# Virtual Staking Contract

This contract is responsible for interfacing with the [virtual staking sdk module](../../../docs/consumer/GoModule.md)
and minting and burning and delegating actual tokens to match the requests of the provider.

It is defined in more detail [in the architecture documentation](../../../docs/consumer/VirtualStaking.md). The rest of this
README should focus on the usage.

## Usage

### Contract Initialization

The Virtual Staking contract is deployed and initialized via a governance proposal on the consumer chain. During initialization, it establishes a connection with a specific Converter contract and is configured with:

- **Denom**: The native staking token denomination
- **Converter**: The address of the associated Converter contract
- **Max Cap**: The maximum amount of virtual tokens the contract can mint (set by chain governance)

### Interacting with the Contract

#### For Converter Contracts

The Virtual Staking contract accepts the following execution messages from its paired Converter:

1. **Bond** - Request to bond tokens to a specific validator:
   ```rust
   Bond {
     delegator: String,
     validator: String,
     amount: Coin,
   }
   ```

2. **Unbond** - Request to unbond tokens from a specific validator:
   ```rust
   Unbond {
     delegator: String,
     validator: String,
     amount: Coin,
   }
   ```

3. **Burn** - Unbond proportionally from multiple validators:
   ```rust
   Burn {
     validators: Vec<String>,
     amount: Coin,
   }
   ```

4. **Internal Unbond** - Immediately unbond when max cap is zero (triggered during channel close):
   ```rust
   InternalUnbond {
     delegator: String,
     validator: String,
     amount: Coin,
   }
   ```

5. **Handle Close Channel** - Unbond all tokens and clean up when IBC channel is closed:
   ```rust
   HandleCloseChannel {}
   ```

#### Query Functions

The contract provides several query endpoints:

1. **Config** - Get contract configuration:
   ```rust
   Config {}
   ```

2. **Bond Status** - Query delegation status for specific validator:
   ```rust
   GetStake {
     validator: String,
   }
   ```

3. **All Bond Status** - Get all delegations:
   ```rust
   GetAllStake {}
   ```

### Epoch-Based Operations

The Virtual Staking contract operates on an epoch-based system for efficiency:

1. **Rebalancing**: Bond/unbond operations are batched and processed once per epoch to minimize gas costs.
2. **Reward Distribution**: Rewards are withdrawn from all validators and distributed to the Converter contract during each epoch.

The `handle_epoch` function is triggered via a `SudoMsg` by the native module at the end of each epoch and handles:

- Processing pending bond/unbond requests
- Rebalancing delegations if the total exceeds the max cap
- Withdrawing and distributing rewards
- Processing validator slash requests

### Max Cap Enforcement

If total delegations exceed the max cap (set by governance), the contract will automatically:

1. Calculate the ratio between max cap and total requested delegations
2. Scale down each delegation proportionally
3. Rebalance existing delegations during the next epoch

### Validator Set Updates

The contract receives validator set updates via `SudoMsg::ValsetUpdate` whenever:
- A new validator joins the active set
- A validator is removed or jailed
- Validator data is updated

This ensures the contract maintains accurate information about the validator set for delegations and slashing events.

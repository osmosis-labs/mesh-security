## Go Module designs

## Consumer - Virtual Staking Contract

### Bootstrapping the contract(s)
Contracts will be uploaded and instantiated via gov proposals on permissioned chains. The proposal(s) will be submitted manual.
Wasmd contains the required proposal types already. It may make sense to use gov v1 multi-message proposals. 

### Contract Authorization
In order to let a contract manage virtual stake, an authorization by the consumer chain must be given to ensure security.
This is usually done by a proposal and gov authority for a running chain.
As authorization rules can be very chain specific, this is going to be an extension point in the implementation with an interface.

For simplicity the contract authorization for virtual stake can be combined with the **max cap limit** configuration per contract.
This would be the default implementation. No max cap limit means no authorization.

Example for gov v1 message to add/ update the max cap limit for a contract:

```protobuf
message MsgSetVirtualStakingMaxCap {
  // Authority is the address that controls the module (defaults to x/gov unless
  // overwritten).
  string authority = 1;

  // Contract is the address of the smart contract that is given permission
  // do virtual staking which includes minting and burning staking tokens.
  string contract = 2;

  // MaxCap is the limit up this the virtual tokens can be minted.
  cosmos.base.v1beta1.Coin max_cap = 5;
}
```

### Contract messages
Example of the custom messages that can be sent by an authorized contract to mint/burn virtual tokens and un-/ delegate to a validator:   
```go
type CustomMsg struct {
	VirtualStake *VirtualStakeMsg `json:"virtual_stake,omitempty"`
}

type VirtualStakeMsg struct {
	Bond   *BondMsg   `json:"bond,omitempty"`
	Unbond *UnbondMsg `json:"unbond,omitempty"`
}

type BondMsg struct {
	Amount    wasmvmtypes.Coin `json:"amount"`
	Validator string           `json:"validator"`
}

type UnbondMsg struct {
  Amount    wasmvmtypes.Coin `json:"amount"`
  Validator string           `json:"validator"`
}
```

### Enforcing max cap limit and system integrity
It is important to reject `Delegate`, `Undelegate`, `Redelegate` messages from any contract that has a max cap limit set. 
Virtual tokens can only be burned. To enforce this behaviour an additional message handler can be chained before the default one that
checks messages for malicious behaviour. This is just a safety net.
The handler would reject all:
* [`wasm.Staking`](https://github.com/CosmWasm/wasmvm/blob/v1.2.3/types/msg.go#L226),
* [`wasm.Stargate`](https://github.com/CosmWasm/wasmvm/blob/v1.2.3/types/msg.go#L269)

Out of scope are [SDK `Authz`](https://github.com/cosmos/cosmos-sdk/tree/main/x/authz) permissions.


## Integration of Cosmos-SDK and Osmosis Fork
Osmosis was pioneering the superfluid staking module. With this work additional methods were added to the Osmosis fork that make sense for mesh-security, too.
The Go module should provide extension points and adapters provided so that both SDKs are supported without the need to fork and modify existing Cosmos-SDK code.
The additional logic introduced by Osmosis should be handled within the adapters and extension points.

### Bank
-	`AddSupplyOffset(ctx sdk.Context, denom string, offsetAmount sdk.Int)` - keeps track of the current value of virtual tokens; This can either be replicated
  in the Go module for Cosmos-SDK chains or just be ignored. 

### Staking
- `InstantUndelegate(ctx sdk.Context, delAddr sdk.AccAddress, valAddr sdk.ValAddress, sharesAmount sdk.Dec) (sdk.Coins, error)`- undelegate tokens 
   without the normal unbounding period; can be fully replicated in the Go module for Cosmos-SDK chains 
- Hook: `AfterValidatorSlashed(ctx sdk.Context, valAddr sdk.ValAddress, infractionHeight int64, slashFactor sdk.Dec, effectiveSlashFactor sdk.Dec)` - callback that
  triggers a refresh of the intermediary delegations; this can either be achieved by a decorator to the Cosmos-SDK `staking/keeper.go`  Slash + SlashWithInfractionReason methods or
  an async process that registers the action on the `BeforeValidatorSlashed` hook for non Cosmos-SDK chains

### Adapters
In order to not add switches for Cosmos-SDK or the Osmosis fork in the code, adapters can be used to provide the missing functionality.

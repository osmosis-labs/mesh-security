## Go Module designs

## Consumer - Virtual Staking Contract

### Bootstrapping the contract(s)
Bootstrapping of the mesh consumer side contracts requires some orchestration and configuration work that can be done by the Go module
as an alternative to a manual setup. This comes also with some benefits for system- / integration-tests. 
The challenge is to keep the contracts that are shipped for bootstrapping up to date with the version of their 
own repository.

### Contract Authorization
In order to let a contract manage virtual stake, an authorization by the consumer chain must be given to ensure security.
This can be done either by a the Go module on chain update with migration code where the source and integrity of the contract is 
ensured before or via proposal and gov authority on a running chain.

For simplicity the contract authorization for virtual stake can be combined with the **max cap limit** configuration per contract.
No max cap limit means no authorization.

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

### Bond Virtual Stake

#### Contract messages
Example of the custom messages that can be sent by an authorized contract to mint virtual tokens and delegate to a validator:   
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
```

#### Custom Message Handler
Example for an extension to wasmd custom message handler:

```go
func (h CustomMsgHandler) DispatchMsg(ctx sdk.Context, contractAddr sdk.AccAddress, _ string, msg wasmvmtypes.CosmosMsg) ([]sdk.Event, [][]byte, error) {
	// assert msg.Custom != nil 
	var customMsg contract.CustomMsg
  err := json.Unmarshal(msg.Custom, &customMsg)
	// assert no error
  // assert customMsg.VirtualStake == nil
  // assert IsAuthorized(ctx, contractAddr)
	switch {
	case customMsg.VirtualStake.Bond != nil:
		events, i, err := h.handleBondMsg(ctx, contractAddr, customMsg.VirtualStake.Bond)
		if err != nil {
			return events, i, err
		}
    }
    return nil, nil, wasmtypes.ErrUnknownMsg
}
```

#### Bond Logic
Add to `SupplyOffset` used by Osmosis
```go
func (k Keeper) Delegate(pCtx sdk.Context, actor sdk.AccAddress, valAddr sdk.ValAddress, amt sdk.Coin) (sdk.AccAddress, error) {
  // assert amount is > 0 
  // Ensure MS constraints:
  newTotalDelegatedAmount := k.getTotalDelegatedAmount(pCtx, actor).Add(amt.Amount)
  // assert newTotalDelegatedAmount >>getMaxCapLimit(pCtx, actor)  

  // Ensure staking constraints
  // asert amt.Denom == staking.BondDenom(ctx) 
  validator, found := k.staking.GetValidator(pCtx, valAddr)
  // assert found
  
  cacheCtx, done := pCtx.CacheContext() // work in a cached store as osmosis (safety net?)
  // get or create intermediary account from index
  imAddr := k.getOrCreateIntermediaryAccount(pCtx, actor, valAddr)
  
  // mint tokens as virtual coins that do not count to the total supply
  coins := sdk.NewCoins(amt)
  err := k.bank.MintCoins(cacheCtx, types.ModuleName, coins)
  // assert no error
  k.bank.AddSupplyOffset(cacheCtx, bondDenom, amt.Amount.Neg())
  err = k.bank.SendCoinsFromModuleToAccount(cacheCtx, types.ModuleName, imAddr, coins)
  // assert no error
	
  // delegate virtual coins to the validator
  _, err = k.staking.Delegate(
    cacheCtx,
    imAddr,
    amt.Amount,
    stakingtypes.Unbonded,
    validator,
  true,
  )
  
  // and update our records
  k.setTotalDelegatedAmount(cacheCtx, actor, newTotalDelegatedAmount)
  done()
  
  // TODO: emit events?
  // TODO: add to telemetry?
    return imAddr, err
}
```
### Unbond Virtual Stake

#### Contract messages
Example of the custom messages that can be sent by an authorized contract to unbond from a delegator and burn virtual tokens:
```go
type VirtualStakeMsg struct {
	Bond   *BondMsg   `json:"bond,omitempty"`
	Unbond *UnbondMsg `json:"unbond,omitempty"`
}

type UnbondMsg struct {
  Amount    wasmvmtypes.Coin `json:"amount"`
  Validator string           `json:"validator"`
}

```

#### Custom Message Handler
Example for an extension to wasmd custom message handler:

```go
	switch {
    case customMsg.VirtualStake.Bond != nil:
    ...
    case customMsg.VirtualStake.Unbond != nil:
        events, i, err := h.handleUnbondMsg(ctx, contractAddr, customMsg.VirtualStake.Unbond)
        if err != nil {
            return events, i, err
        }
}
```
#### Unbond Logic
Decreases `SupplyOffset` used by Osmosis
```go
func (k Keeper) Undelegate(pCtx sdk.Context, actor sdk.AccAddress, valAddr sdk.ValAddress, amt sdk.Coin) error {
	// assert amt.Amount > 0 
	// Ensure staking constraints
	// assert amt.Denom = bondDenom

	// get intermediary address for validator from index
	imAddr := k.getIntermediaryAccount(pCtx, actor, valAddr)

	cacheCtx, done := pCtx.CacheContext() // work in a cached store (safety net?)
	shares, err := k.staking.ValidateUnbondAmount(cacheCtx, imAddr, valAddr, amt.Amount)
	// abort on stakingtypes.ErrNoDelegation
	// assert no error

	undelegatedCoins, err := k.staking.InstantUndelegate(cacheCtx, imAddr, valAddr, shares)
  // assert no error
	
	err = k.bank.SendCoinsFromAccountToModule(cacheCtx, imAddr, types.ModuleName, undelegatedCoins)
  // assert no error

	err = k.bank.BurnCoins(cacheCtx, types.ModuleName, undelegatedCoins)
  // assert no error

	unbondedAmount := undelegatedCoins.AmountOf(bondDenom)
	k.bank.AddSupplyOffset(cacheCtx, bondDenom, unbondedAmount)
	newDelegatedAmt := k.getTotalDelegatedAmount(cacheCtx, actor).Sub(unbondedAmount)
	k.setTotalDelegatedAmount(cacheCtx, actor, newDelegatedAmt)

	done()
	return nil
}
```

## Integration of Cosmos-SDK and Osmosis Fork
Osmosis was pioneering the superfluid staking module. With this work additional methods were added to the Osmosis fork that make sense for mesh-security, too.
There should be extension points and adapters provided so that both SDKs are supported.

### Bank
-	`AddSupplyOffset(ctx sdk.Context, denom string, offsetAmount sdk.Int)` - keeps track of the current value of virtual tokens; This can either be replicated
  in the Go module for Cosmos-SDK chains 

### Staking
- `InstantUndelegate(ctx sdk.Context, delAddr sdk.AccAddress, valAddr sdk.ValAddress, sharesAmount sdk.Dec) (sdk.Coins, error)`- undelegate tokens 
   without the normal unbounding period; can be fully replicated in the Go module for Cosmos-SDK chains 
- Hook: `AfterValidatorSlashed(ctx sdk.Context, valAddr sdk.ValAddress, infractionHeight int64, slashFactor sdk.Dec, effectiveSlashFactor sdk.Dec)` - callback that
  triggers a refresh of the intermediary delegations; this can either be achieved by a decorator to the Cosmos-SDK `staking/keeper.go`  Slash + SlashWithInfractionReason methods or
  an async process that registers the action on the `BeforeValidatorSlashed` hook for non Cosmos-SDK chains

### Adapters
In order to not add switches for Cosmos-SDK or the Osmosis fork in the code, adapters can be used to provide the missing functionality.

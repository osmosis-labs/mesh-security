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
	if msg.Custom == nil {
		return nil, nil, wasmtypes.ErrUnknownMsg
	}
	var customMsg contract.CustomMsg
	if err := json.Unmarshal(msg.Custom, &customMsg); err != nil {
		return nil, nil, sdkerrors.ErrJSONUnmarshal.Wrap("custom message")
	}
	if customMsg.VirtualStake != nil {
		// not our message type
		return nil, nil, wasmtypes.ErrUnknownMsg
	}

	ok, err := h.k.IsAuthorized(ctx, contractAddr) // authorization should be an extension point: can be a check for an existing max cap limit
	if err != nil {
		return nil, nil, err
	}
	if !ok {
		return nil, nil, sdkerrors.ErrUnauthorized.Wrapf("contract has no permission for mesh security operations")
	}

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
  if amt.Amount.IsZero() || amt.Amount.IsNegative() {
    return nil, errors.ErrInvalidRequest.Wrap("amount")
  }

  // Ensure MS constraints:
  newTotalDelegatedAmount := k.getTotalDelegatedAmount(pCtx, actor).Add(amt.Amount)
  if newTotalDelegatedAmount.GT(k.getMaxCapLimit(pCtx, actor)) {
    return nil, types.ErrMaxCapExceeded
  }

  // Ensure staking constraints
  bondDenom := k.staking.BondDenom(pCtx)
  if amt.Denom != bondDenom {
    return nil, errors.ErrInvalidRequest.Wrapf("invalid coin denomination: got %s, expected %s", amt.Denom, bondDenom)
  }
  validator, found := k.staking.GetValidator(pCtx, valAddr)
  if !found {
    return nil, stakingtypes.ErrNoValidatorFound
  }
  
  cacheCtx, done := pCtx.CacheContext() // work in a cached store as osmosis (safety net?)
  // get or create intermediary account from index
  imAddr := k.getOrCreateIntermediaryAccount(pCtx, actor, valAddr)
  
  // mint tokens as virtual coins that do not count to the total supply
  coins := sdk.NewCoins(amt)
  err := k.bank.MintCoins(cacheCtx, types.ModuleName, coins)
  if err != nil {
    return nil, err
  }
  k.bank.AddSupplyOffset(cacheCtx, bondDenom, amt.Amount.Neg())
  err = k.bank.SendCoinsFromModuleToAccount(cacheCtx, types.ModuleName, imAddr, coins)
  if err != nil {
      return nil, err
  }
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
	if amt.Amount.IsZero() || amt.Amount.IsNegative() {
		return errors.ErrInvalidRequest.Wrap("amount")
	}

	// Ensure staking constraints
	bondDenom := k.staking.BondDenom(pCtx)
	if amt.Denom != bondDenom {
		return errors.ErrInvalidRequest.Wrapf("invalid coin denomination: got %s, expected %s", amt.Denom, bondDenom)
	}

	// get intermediary address for validator from index
	imAddr := k.getIntermediaryAccount(pCtx, actor, valAddr)

	cacheCtx, done := pCtx.CacheContext() // work in a cached store (safety net?)
	shares, err := k.staking.ValidateUnbondAmount(cacheCtx, imAddr, valAddr, amt.Amount)
	if err == stakingtypes.ErrNoDelegation {
		return nil
	} else if err != nil {
		return err
	}

	undelegatedCoins, err := k.staking.InstantUndelegate(cacheCtx, imAddr, valAddr, shares)
	if err != nil {
		return err
	}
	err = k.bank.SendCoinsFromAccountToModule(cacheCtx, imAddr, types.ModuleName, undelegatedCoins)
	if err != nil {
		return err
	}

	err = k.bank.BurnCoins(cacheCtx, types.ModuleName, undelegatedCoins)
	if err != nil {
		return err
	}

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
#### Bank
For example:
```go
// BankKeeperAdapter adapter to vanilla SDK bank keeper
type BankKeeperAdapter struct {
	types.SDKBankKeeper
}

// NewBankKeeperAdapter constructor
func NewBankKeeperAdapter(k types.SDKBankKeeper) *BankKeeperAdapter {
	return &BankKeeperAdapter{SDKBankKeeper: k}
}

// AddSupplyOffset noop
func (b BankKeeperAdapter) AddSupplyOffset(ctx sdk.Context, denom string, offsetAmount sdk.Int) {
}
```
#### Staking
For example:
```go
type StakingKeeperAdapter struct {
	types.SDKStakingKeeper
	bank types.SDKBankKeeper
}

// NewStakingKeeperAdapter constructor
func NewStakingKeeperAdapter(k types.SDKStakingKeeper, b types.SDKBankKeeper) *StakingKeeperAdapter {
	return &StakingKeeperAdapter{SDKStakingKeeper: k, bank: b}
}

// InstantUndelegate allows another module account to undelegate while bypassing unbonding time.
// This function is a combination of Undelegate and CompleteUnbonding,
// but skips the creation and deletion of UnbondingDelegationEntry
//
// This is copied from the Osmosis sdk fork
func (s StakingKeeperAdapter) InstantUndelegate(ctx sdk.Context, delAddr sdk.AccAddress, valAddr sdk.ValAddress, sharesAmount sdk.Dec) (sdk.Coins, error) {
	validator, found := s.GetValidator(ctx, valAddr)
	if !found {
		return nil, stakingtypes.ErrNoDelegatorForAddress
	}

	returnAmount, err := s.Unbond(ctx, delAddr, valAddr, sharesAmount)
	if err != nil {
		return nil, err
	}

	bondDenom := s.BondDenom(ctx)

	amt := sdk.NewCoin(bondDenom, returnAmount)
	res := sdk.NewCoins(amt)

	moduleName := stakingtypes.NotBondedPoolName
	if validator.IsBonded() {
		moduleName = stakingtypes.BondedPoolName
	}
	err = s.bank.UndelegateCoinsFromModuleToAccount(ctx, moduleName, delAddr, res)
	if err != nil {
		return nil, err
	}
	return res, nil
}
```

### AfterValidatorSlashed Hook
TBD

# 0.10.0

- Slash amounts support.
- Update cosmwasm.
- Slashing docs update.

# 0.9.0

- Osmosis price oracle.
- Native slashing.
- Multiple slash ratios.
- Clean up empty liens in vault.
- Specify precise deps / set minimum cw version.

# 0.8.1

- Modify jailing test helper to better reflect blockchain's behaviour.
- Fix: Filter out jailed validators from removed set.

# 0.8.1-alpha.1

- Slashing propagation accounting.
- Improve cross-bond/unbond process.

# 0.8.0-alpha.1

- Add code coverage.
- Disable native staking in vault.
- Improve rewards withdrawal process.
- Valset updates for external-staking.

# 0.7.0-alpha.2

- Remove empty messages / events.
- Fix virtual-staking slashing accounting.

# 0.7.0-alpha.1

- Cross-slashing implementation.
- Batch distribute rewards.
- Valset updates.
- Slashing accounting.
- Slashing propagation at the `vault` contract level.

# 0.3.0-beta

- IBC specification is added to the documents.
- IBC types and logic added to `mesh-api::ibc`
- `converter` and `external-staking` support IBC
  - Handshake and channel creation
  - Validator sync protocol (Consumer -> Provider)
    TODO: Dynamic updates
  - Staking protocol (Provider -> Consumer)
  - Rewards protocol (Consumer -> Provider -> Consumer)

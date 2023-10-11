# 0.7.0-alhpa.1

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

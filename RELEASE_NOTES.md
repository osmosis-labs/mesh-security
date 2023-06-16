# Release 0.2.0

This has a complete implementation of the Provider side, except the external staking
contracts don't send any IBC packets. The main purpose of these contracts is to allow
development and testing of the frontend, as all user interactions will be with the
provider contracts. Please create issues for any missing queries, so we can add them.

We has some stub contracts for the consumer side. They are incomplete and not tested,
but at least a rough draft of the required functionality for the virtual-staking
contract which can be used to test the [virtual staking sdk module](https://github.com/osmosis-labs/mesh-security-sdk)

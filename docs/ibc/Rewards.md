# Cross-Chain Rewards Protocol

The cross-chain rewards protocol is used to send staking rewards from the consumer chain
to the provider chain, where it will be distributed among the virtual delegators.

## Channel Requirements

The original reward token is the native staking token of the consumer chain,
and it should be transferred to the provider chain using the most commonly accepted
token bridge, to ensure fungibility. Generally this will be the ICS20 channel
between `transfer` ports on a standard accepted channel. However, if there is
another bridge that is more commonly used, it may be used instead.

The consumer side should take care of both the _transfer_ of the reward token
as well as _distributing_ the tokens to the `external-staking` contract.
This means, the provider side just sees it as a local method execution
along with a whitelisted IBC token denom.

## Reward Distribution

The reward distribution is done by the `external-staking` contract on the provider chain, and not part of the
IBC protocol. However, it does touch on timing issues and I want to clarify how some of that is handled.

First, rewards are send from consumer -> provider (by one of the methods described below) on infrequent epochs.
Whoever is recorded as staked to that validator at the end of the epoch will receive a share of the rewards.
We don't track when during the epoch they staked, so the one who staked 1 minute after epoch N and the other
who staked 1 minute before epoch N+1 will both receive the same share of rewards.

While this looks like a possible place for arbitrage and attacks, you must remember there is an unbonding period of around
21 days. So if you stake at the end of the epoch, you will not be able to withdraw for 21 days. Getting a full days of rewards
for one minute of staking sounds nice. But getting one day of rewards for 21 days of unbonding is not such a good deal.
Thus, the approximation of treating everyone who staked any time during the epoch equally is fair enough.

The further question arises about how to count people with in-flight staking (or unstaking) transactions at the time of the
epoch reward packet arriving. This is a bit more tricky, as we don't know if they will succeed or not. If we were very strict
about [serializability](./Serializability.md) in this case, we wouldn't allow any reward distribution until all in-flight
packets (from all users) and finalized. This is unrealistic, so we can relax this a bit to "read committed", which is close
enough as there are no invariants to enforce here, and we already accepted an approximation above.

This means, that we don't update a user's reward shares for any in-flight IBC transaction until we have received a success ack.
This is such an edge case on which side of the epoch they fall, and both decisions are equally valid from an economic perspective.
However, in order to favor the approach of database consistency, we will use the "read committed" approach, and only count rewards
after a successful ACK.

## Proposed Implementations

As mentioned above, the general protocol design supports multiple bridges,
and the external staking contract need not be aware of the bridge used.
However, it is essential that the converter contract select an adequate process
that allows it to (a) send the reward tokens to the provider chain,
(b) execute the `external-staking` contract to distribute the tokens, and
(c) handle any IBC errors (timeout, etc) that may occur.

Below we discuss some proposed implementations:

### ICS20 plus ICA

**Not Recommended**

In this case, there must be an established ics20-v1 channel between both chains,
and a ICA channel between the converter contract and the provider chain.

The converter contract will send the reward tokens to the provider chain via ICS20,
and when the ack is received, it will send an ICA message to execute `distribute_rewards`
on the `external-staking` contract, along with the previously transferred funds.

**Problem**: There are currently no contract callbacks on ICS20 nor ICA. This means we cannot reliably
orchestrate these two events. Furthermore, the ICA message may fail, and the reward tokens will
be stranded, as there is no callback to inform them.

**This approach should only be considered if there is no other option.**

### ICS20 with Memo Field

**Recommended with hesitation**

Osmosis developed a nice addition to the ICS20 standard, which allows the sender
to specify a memo field. This is used to specify a contract execution to be performed
with the transferred tokens. It explicitly doesn't allow associating those tokens
with a particular address which is fine for this use case, as `distribute_rewards`
is permissionless and accepts any payment in the proper denom.

This is a nice solution, as it allows the converter to perform all actions in a single
IBC packet call and not worry about orchestrating multiple packets. However, as of the
writing of this document, there is **no callback** on failed packet receive
(that is left for a future version of the protocol). This means that if the `distribute_rewards`
call fails for whatever reason, the reward tokens will be effectively lost.

This is a good solution for the MVP, but we should consider moving to a more robust
solution when one exists. Hopefully this will spur deployment of such methods.

### ICS20 with Custom IBC Middleware

With the consumer chain including the mesh-security-sdk, we have the opportunity to add a custom IBC-middleware
to the IBC-stack for ICS-20. This middleware can be used to do callbacks to contracts on packet ack/timeout.
The process would be:

- Contract sends ICS-20 message and register for a callback on the IBC packet ID (via custom message)
- All metadata is stored on chain only and not relayed
- When the packet is ACKed/timeout, the contract receives the callback from the middleware (after the ICS-20 module) with the ack/timeout data
- When there is confidence that the ICS-20 operation succeeded, the contract can trigger the reward distribution work with the callback

Note: the callback execution must not fail to not interfere with the ack/timeout process

The benefit of this solution is that it is not depending on other technology. IBC-middleware and callback registration would
be provided and maintained by the mesh-security-sdk project.
We also have a [vertical spike](https://github.com/CosmWasm/wasmd/pull/1368) for this use case.

This can be added to both of the above approaches in order to remove some of the issues.
In particular, the Custom Middleware with callbacks along with the memo field can work quite well, as long as
the Provider chain has enabled the memo field extension.

### No ICS20

In the end, ICS20 is just two trusted contracts passing some numbers back and forth with a bit of accounting.
No tokens ever move. Just a promise to release the token on the source chain. So we can do this without ICS20
if we just want the reward tokens on the consumer chain.

Imagine the OSMO-JUNO scenario. The Provider on OSMO may want to get their reward tokens in IBC-JUNO on Osmosis if they wish
to sell it immediately. However, if they want to stake it (or compound on Juno), the will have to first withdraw on Osmosis,
then IBC Transfer it back to JUNO, then (mesh-)stake it there.

If we assume the typical use case is not selling the reward token on the provider chain, but rather re-investing it (or using it)
on the Consumer chain, there is another approach that works without ICS20. The consumer chain doesn't send any tokens, but
rather it holds the reward tokens and sends a packet over the control channel to the provider chain to execute
`distribute_rewards` with this amount. Later, when a user wants to withdraw their rewards on the provider chain,
it will send a packet to the consumer chain over the control channel to release the tokens to any address on the consumer side.

This approach is basically like an embedded ICS20 inside the mesh protocol, but it doesn't issue any tokens or provide fungibility
on the provider chain. This reduces the complexity as we only have one channel and packet and don't need to orchestrate multiple
packets over multiple channels.

The biggest question is whether the functionality change is acceptable (or even desirable). Note that this will make restaking the
reward tokens easier, while making selling them on the provider chain harder. This may be a good thing (in the eyes of the
consumer chain at least), as it encourages longer-term holders.

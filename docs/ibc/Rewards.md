# Cross-Chain Rewards Protocol

The cross-chain rewards protocol is used to send staking rewards from the consumer chain
to the provider chain, where it will be distributed among the virtual delegators.

## Channel Requirements

The original reward token is the native staking token of the consumer chain,
and it should be transfered to the provider chain using the most commonly accepted
token bridge, to ensure fungibility. Generally this will be the ICS20 channel 
between `transfer` ports on a standard accepted channel. However, if there is
another bridge that is more commonly used, it may be used instead.

The consumer side should take care of both the _transfer_ of the reward token
as well as _distributing_ the tokens to the `external-staking` contract.
This means, the provider side just sees it as a local method execution
along with a whitelisted IBC token denom.

## Proposed Implementations

As mentioned above, the general protocol design supports multiple bridges,
and the external staking contract need not be aware of the bridge used.
However, it is essential that the converter contract select an adequate process
that allows it to (a) send the reward tokens to the provider chain, 
(b) execute the `external-staking` contract to distribute the tokens, and 
(c) handle any IBC errors (timeout, etc) that may occur.

Below we discuss some proposed implementations:

## ICS20 plus ICA

**Not Recommended**

In this case, there must be an established ics20-v1 channel between both chains,
and a ICA channel between the converter contract and the provider chain.

The converter contract will send the reward tokens to the provider chain via ICS20,
and when the ack is received, it will send an ICA message to execute `distribute_rewards`
on the `external-staking` contract, along with the previously transfered funds.

**Problem**: There are currently no contract callbacks on ICS20 not ICA. This means we cannot reliably
orchestrate these two events. Furthermore, the ICA message may fail, and the reward tokens will 
be stranded, as there is no callback to inform them.

This approach should only be considered 


### ICS20 with Memo Field

**Recommended with hesitation**

Osmosis developed a nice addition to the ICS20 standard, which allows the sender
to specify a memo field. This is used to specify a contract execution to be performed
with the transferred tokens. It explicitly doesn't allow associating those tokens
with a particular address which is fine for this use case, as `distribute_rewards`
is permissionless and accepts any payment in the proper denom.

This is a nice solution, as it allows the converter to perform all actions in a single
IBC packet call and not worry about orchestrating multiple packets. However, as of the
writing of this document, there is no callback on failed packet receive
(that is left for a future version of the protocol). This means that if the `distribute_rewards`
call fails for whatever reason, the reward tokens will be effectively lost.

This is a good solution for the MVP, but we should consider moving to a more robust
solution when one exists. Hopefully this will spur deployment of such methods.

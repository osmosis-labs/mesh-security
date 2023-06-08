# Serializability

**Serializability** is a key property of blockchains, and a well-defined property in
standard databases that can be accepted when certain guarantees are more important
than raw throughput. The process of consensus and creating and ordered block of
transactions provides the basis for this guarantee. If the state machine then
executes them sequentially, we have the _serializable_ property, because we actually
execute them serially.

However, in a multi-chain (IBC) system, we have to be careful about how we implement this,
as the state transitions in the receiving chain are no longer atomic with the state transitions
on the sending chain, and we no longer have a guarantee of sequential order. Also, if there
are attempts to speed up performance by optimistically executing transactions in parallel,
the designer must be very careful of this implementation and understand the concept
of _serializability_ very well to avoid unexpected behavior.

## In ACID Databases

Let's start with a more practical definition, as a user of a system that supports this.
The **I** in ACID-complaint database stands for **Isolation**. PostgreSQL has a solid
implementation of this, and defines the concept of "Transaction Isolation", which you
may well have encountered and used:

> The SQL standard defines four levels of transaction isolation. The most strict is Serializable, which is defined by the standard in a paragraph which says that any concurrent execution of a set of Serializable transactions is guaranteed to produce the same effect as running them one at a time in some order. The other three levels are defined in terms of phenomena, resulting from interaction between concurrent transactions, which must not occur at each level. The standard notes that due to the definition of Serializable, none of these phenomena are possible at that level.

([PostgreSQL docs: Transaction Isolation](https://www.postgresql.org/docs/current/transaction-iso.html))

It goes on and explains

> The _Serializable_ isolation level provides the strictest transaction isolation. This level emulates serial transaction execution for all committed transactions; as if transactions had been executed one after another, serially, rather than concurrently. However, like the Repeatable Read level, applications using this level must be prepared to retry transactions due to serialization failures. In fact, this isolation level works exactly the same as Repeatable Read except that it also monitors for conditions which could make execution of a concurrent set of serializable transactions behave in a manner inconsistent with all possible serial (one at a time) executions of those transactions. This monitoring does not introduce any blocking beyond that present in repeatable read, but there is some overhead to the monitoring, and detection of the conditions which could cause a _serialization anomaly_ will trigger a _serialization failure_.

Continuing with an example of [such a case that you can read](https://www.postgresql.org/docs/current/transaction-iso.html#XACT-SERIALIZABLE).

The key note here is that, in the interest of performance, the database _does not_ actually run them
serially, not use locking to ensure key portions are run serially. Instead, it detects any conflict, and
if present, aborts the transaction with an error (to be retried later). Such a definition may be useful
when speeding up local changes, but it is not possible when the transaction consists of sub-transactions on different blockchains, as we cannot roll them both back on failure.

Example: I start (but don't commit) a transaction on chain A, and send a message to chain B.
Chain B starts a transaction, which succeeds. While committing on chain A, I detect a conflict
and abort it (roll it back). However, I can no longer safely roll back the state on B. If you attempt
to add more phases, you end up with a game of ping-pong, where one chain is unable to rollback
if the other fails.

## General Approaches

In distributed systems, we need to be more careful, and go back to the general defintion of
Serialiazable systems, to determine which primitives we need to implement it. And from there
determine which technique will be most effective in our system. Let's start with
[Wikipedia (which has quite good references in computer science)](https://en.m.wikipedia.org/wiki/Serializability#View_and_conflict_serializability). (All quotes below are from this page, unless otherwise noted)

### Locking Data

> Operations upon data are _read_ or _write_ (a write: insert, update, or delete). Two operations are conflicting if they are of different transactions, upon the same datum (data item), and at least one of them is write. Each such pair of conflicting operations has a conflict type: it is either a read–write, or write–read, or a write–write conflict. The transaction of the second operation in the pair is said to be in conflict with the transaction of the first operation.

So far, we can start thinking of Read-Write Locks to avoid said conflicts.
[Two-Phase Locking](https://en.m.wikipedia.org/wiki/Two-phase_locking) explains how to handle these
this approach without deadlocks, and would be the first approach we could consider. In fact,
I would consider it the simplest general-case solution to the problem, without understanding
the details of the actual transactions being executed.

### Commumtative Operations

> A more general definition of conflicting operations (also for complex operations, which may each consist of several "simple" read/write operations) requires that they are noncommutative (changing their order also changes their combined result). Each such operation needs to be atomic by itself (using proper system support) in order to be considered an operation for a commutativity check. For example, read–read operations are commutative (unlike read–write and the other possibilities) and thus read–read is not a conflict. Another more complex example: the operations increment and decrement of a counter are both write operations (both modify the counter), but do not need to be considered conflicting (write-write conflict type) since they are commutative (thus increment–decrement is not a conflict; e.g., already has been supported in the old IBM's IMS "fast path").

The idea of "commutative operations" gets very attractive for our case. In this case, we need no
locks, or special handling of the packet and ack ordering. One way to guarantee communativity
is to create a [data structure that is a CRDT](#crdts) (link to section below). Often that is
not possible (we have some global invariants that may never be broken), and we need to
look further. But if it is possible to use CRDTs, you can always guarantee serializability, without
the need for locks or any other book-keeping.

Note that Incrementing and Decrementing a counter is a good example of a commutative operation,
but once we start adding some global invariants, like "counter must never go below 0", this is
no longer commutative. `+10`, `-5` would equal `5`, while `-5`, `+10` would error on the first
operation and leave the counter at `10` after the second operation.

### Operation Ordering

> Only precedence (time order) in pairs of conflicting (non-commutative) operations is important when checking equivalence to a serial schedule, since different schedules consisting of the same transactions can be transformed from one to another by changing orders between different transactions' operations (different transactions' interleaving), and since changing orders of commutative operations (non-conflicting) does not change an overall operation sequence result, i.e., a schedule outcome (the outcome is preserved through order change between non-conflicting operations, but typically not when conflicting operations change order). This means that if a schedule can be transformed to any serial schedule without changing orders of conflicting operations (but changing orders of non-conflicting, while preserving operation order inside each transaction), then the outcome of both schedules is the same, and the schedule is conflict-serializable by definition.

Note that the ordering described here refers to not just the order of starting the individual
transactions, but the order of **committing** them. That is, if A, B, and C all decrement a counter
by 1, which starts at 2, then one will fail. We need to commit A and B before C to ensure that
C will fail if we wish to maintain the serializability property. Technically, the commit
doesn't happen until the response from the remote chain, but we need to enforce such
invariants on the sending chain before we send the IBC packet.

Furthermore, this suggests that by inspecting the actual contents of the transactions,
we can determine exactly which sections could conflict and design an algorithm to only focus
on ensuring those sections are executed serially. This allows the rest of the transactional logic
to be processed normally and limits the number of locks we need to take.

### Distributed Serializability and Atomic Commit

Most of the above discussion has been focused on the concurrent processing on one machine,
where the transactions are processed in parallel, but atomic commits are (usually) granted by
shared memory. However, in a distributed system, we need to consider the actual
process of committing.

> Distributed serializability is the serializability of a schedule of a transactional distributed system (e.g., a distributed database system). Such a system is characterized by distributed transactions (also called global transactions), i.e., transactions that span computer processes (a process abstraction in a general sense, depending on computing environment; e.g., operating system's thread) and possibly network nodes. A distributed transaction comprises more than one of several local sub-transactions that each has states as described above for a database transaction. A local sub-transaction comprises a single process, or more processes that typically fail together (e.g., in a single processor core). Distributed transactions imply a need for an atomic commit protocol to reach consensus among its local sub-transactions on whether to commit or abort. Such protocols can vary from a simple (one-phase) handshake among processes that fail together to more sophisticated protocols, like two-phase commit, to handle more complicated cases of failure (e.g., process, node, communication, etc. failure)

(From [Wikipedia - Distributed Serializability](https://en.m.wikipedia.org/wiki/Serializability#Distributed_serializability))

A [Two-Phase Commit](https://en.m.wikipedia.org/wiki/Two-phase_commit_protocol) requires
significant communication between the nodes, and is not suitable for an IBC system.
For our purposed, we can look at the basic requirements of
[Atomic Commits](https://en.m.wikipedia.org/wiki/Atomic_commit):

> In the field of computer science, an atomic commit is an operation that applies a set of distinct changes as a single operation. If the changes are applied, then the atomic commit is said to have succeeded. If there is a failure before the atomic commit can be completed, then all of the changes completed in the atomic commit are reversed. This ensures that the system is always left in a consistent state

This can be implemented in a two-chain IBC protocol with the following approach:

1. The sending chain A completes its sub-transaction and maintains all needed locks
   (or other structs) to guarantee it can commit or rollback the state transitions as needed.
2. The receiving chain B processes its sub-transaction and returns a success or error ack.
3. If the ack is a success, chain A commits its state transitions. If it is an error, chain A
   rolls back its state transitions.

This builds on the existing IBC infrastructure and is the reason why ACKs were introduced
into the IBC protocol in the first place.

## CRDTs

CRDTs are "magical" beasts. Since they are fully commutative, there is no more concern about ordering or conflicts.
If we can express our data in terms of commutative operations and a manifested state of them, this allows unordered channels
without any more concerns about data consistency.

> **Commutative** replicated data types (`CmRDTs`) replicas propagate state by transmitting only the update operation. For example, a CmRDT of a single
> integer might broadcast the operations (+10) or (−20). Replicas receive the updates and apply them locally. The operations are commutative. However, they are
> not necessarily idempotent. The communications infrastructure must therefore ensure that all operations on a replica are delivered to the other replicas,
> without duplication, but in any order.

(From [Wikipedia - CRDT](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#Known_CRDTs))

The key point here is every operation is commutative. This works great for a counter (increment and decrement) **over all integer values**.
However, if we could ever hit a limit (`i64::MAX` or simply `0`), then one of the the operations would fail. And the particular operation
that fails depends on the ordering of the operations, thus rendering it no longer commutative. If the limits are `i128::MAX` and `i128::MIN`
and there is a limit to the value of the counter (not used defined), then we can prove we will never hit the said limits and this
would be commutative. However, since we usually enforce `value > 0` on blockchains, this would rarely work.

Other types that are well defined and may be useful to IBC protocols are "grow-only set", "two-phase-set" (once removed, it can never enter),
"last write wins" (based on some trusted timestamp). These are mathematical definitions and can be implemented in a variety of ways.
For example "grow-only set" could be an "append-only vector" where we keep it sorted and remove duplicates.

You can read more on some [standard defined CRDT types](https://en.wikipedia.org/wiki/Conflict-free_replicated_data_type#Known_CRDTs)

## Locking

CRDTs are extremelely flexible and resillient, but it is very difficult to map most business logic to such types.
On the opposite extreme, we can use locking to ensure that only one transaction is processed at a time. This is very limiting,
but it provably correct over any business logic.

Note that is **requires ordered channels**, as commit ordering is essential here. If we start transaction A, B, C on the sending chain
in that order, then A', B', C' must be process on the receiving chain in that order, and the commit/rollback order (based on getting
acks on the sending chain must be also be A, B, C). This is exactly the guarantee that ordered channels provide.

On top of this, we have to ensure that no other transactions conflict with any open IBC transactions. Transaction A is "open" from the
time the first logic is run on the sending chain (which will send the IBC message) until the ack is fully process and committed on
the sending chain. This will span several blocks, possibly hours in the case of timeouts.

We can model this with [Two-phase locking](https://en.wikipedia.org/wiki/Two-phase_locking#Two-phase_locking_and_its_special_cases)
, which defines a "growing phase" of acquiring locks, followed by a "shrinking phase" of releasing locks. This is done to be
resistent to deadlock. We would do the following:s

1. Start Tx on Sending Chain: Acquire all read/write locks on all data that will be touched. This is the "growing" phase of the lock.
2. Process Packet on Receiving Chain: Aquire all read/write locks on all data, process data, release all locks. This goes from the "growing" phase to the "shrinking" phase.
3. Process Ack on the Sending Chain: Process ack, and release all locks. This is the "shrinking" phase.

If we guarantee that we never read any data that is not under a write lock, we can release the read locks at end of "start tx", as they are not needed.
Furthermore, the "process packet" phase just defines the normal sematics of blockchain processing. With that, we can simplify to:

1. Start Tx on Sending Chain: Acquire all write locks on all data that will be committed or rolled-back.
2. Process Packet on Receiving Chain: Process data, and return success or error ack.
3. Process Ack on the Sending Chain: Commit or rollback based on ack. Only can read/write data held under write lock from step 1

If we modelled ICS20 like this, it would require us to hold a lock on the account balance of the sender (at least the keys holding the
denom being sent) from the original send until the ack. This would not interfere with any other processes,
but that user could not send tokens locally, stake those tokens, receive local tokens, or even properly query his balance (as it is undefined).

In some cases where the business logic is very complex and hard to model commutatively, and the keys under lock are only used by this one
sub-system (nothing universal like bank), this may be the best approach. However, it is very limiting and should be avoided if possible.

## Forcing Commutativity

Here we go beyond the well-defined theories presented above, and to my own suggestions to how to safely convert many types of business logic
into something commutative. If a reader knows of some theory that explains the (in)correctness of such a scheme, it would be helpful
to expand the basis of this knowledge and a PR would be most welcome.

The basic concept is that to avoid conflicts, we do not need to make all possible operations commutative, but rather just guarantee that
**all concurrent operations** are commutative. This is a much weaker requirement, and can be achieved in many cases not by modifying the
data structures, but by aborting any transaction in the first phase (before IBC packet send), if it is not fully commutative with
all operations that are currently in-flight. That means, other operations that have previously sent an IBC packet and not yet received an ACK.
Furthermore, if the state transitions involded are local to one or two contracts, we only have to provide checks on those contracts,
which makes this a manageable task.

We can say that ICS20 implementation does something like this. As the only invairant is that the value never goes below zero,
it pre-emptively moves the tokens out of the users account to an escrow. This doesn't require any further lock on the user's account,
but ensures no other transaction will be possible to execute that would render the user's balance below 0 if this was committed.
Thus, any other transaction that would be commutative with this (commit or rollback) can be executed concurrently, but any other
transaction that would conflict with this (eg. spend the same tokens, and only be valid if the ICS20 transfer gets an error ack),
will immediately fail.

ICS20 is an extremely simple case and you don't need such theory to describe decrementing one counter. However, assume there was not
only a min token balance (0) but some max, say 1 million. Then ICS20 would not work, as you could excrow 500k tokens, then receive
600k tokens, and if the ICS20 received an error ACK, it would be impossible to validly roll this back (another example of non-commutative,
or conflicting, operations).

### Value range

One idea to implement this would be to not just store one value (the balance), but a range of values, covering possible values if
all in-flight transactions were to succeed or fail. Normal transactions (send 100 tokens) would update both values and error if either
one violated some invariant (too high or too low). When an IBC transaction is initiated, it would execute eg. "maybe 200", which would
attempt to decrease the min by 200 but leave the max unchanged (requiring this range to remain valid).

This approach would only work by values not used by complex forumulas, such as balances of an AMM (we can't calculate prices off ranges),
or paying out staking rewards (the value received by all other users depends on your stake, and we can't propogate this to all those accounts,
as it would be prohibitively expensive to collapse them all when the transaction is committed or rolled-back).

But for counters with comparisons, increments, and decrements, it could work well. Even enforcing logic like
"total amount of collateral is greater than the max lien amount" could be enforced if collateral and lien amounts
were value ranges. In this case, the max of (`lien.max`) would be compared against `collateral.min`.
With some clever reasoning, we could possibly enforce such value ranges without actually storing multiple fields.

## Next Steps

With this understanding, we can now start to design IBC applications that are safe and correct.
I will reference these theoretical concpets while defining a safe implementation of the Mesh Security control
protocol in face of unordered channels.

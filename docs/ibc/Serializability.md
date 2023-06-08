# Serializability

**Serializability** is a key property of blockchains, and a well-defined property in
standard databases that can be accepted when certain guarantees are more important
than raw throughput. The process of consensus and creating and ordered block of
transactions provides the basis for this guarantee. If the state machine then
executes them sequentially, we have the *serializable* property, because we actually
execute them serially.

However, in a multi-chain (IBC) system, we have to be careful about how we implement this,
as the state transitions in the receiving chain are no longer atomic with the state transitions
on the sending chain, and we no longer have a guarantee of sequential order. Also, if there
are attempts to speed up performance by optimistically executing transactions in parallel,
the designer must be very careful of this implementation and understand the concept
of *serializability* very well to avoid unexpected behavior.

## In ACID Databases

Let's start with a more practical definition, as a user of a system that supports this.
The **I** in ACID-complaint database stands for **Isolation**. PostgreSQL has a solid
implementation of this, and defines the concept of "Transaction Isolation", which you
may well have encountered and used:

> The SQL standard defines four levels of transaction isolation. The most strict is Serializable, which is defined by the standard in a paragraph which says that any concurrent execution of a set of Serializable transactions is guaranteed to produce the same effect as running them one at a time in some order. The other three levels are defined in terms of phenomena, resulting from interaction between concurrent transactions, which must not occur at each level. The standard notes that due to the definition of Serializable, none of these phenomena are possible at that level.

([PostgreSQL docs: Transaction Isolation](https://www.postgresql.org/docs/current/transaction-iso.html))

It goes on and explains

> The *Serializable* isolation level provides the strictest transaction isolation. This level emulates serial transaction execution for all committed transactions; as if transactions had been executed one after another, serially, rather than concurrently. However, like the Repeatable Read level, applications using this level must be prepared to retry transactions due to serialization failures. In fact, this isolation level works exactly the same as Repeatable Read except that it also monitors for conditions which could make execution of a concurrent set of serializable transactions behave in a manner inconsistent with all possible serial (one at a time) executions of those transactions. This monitoring does not introduce any blocking beyond that present in repeatable read, but there is some overhead to the monitoring, and detection of the conditions which could cause a *serialization anomaly* will trigger a *serialization failure*.

Continuing with an example of [such a case that you can read](https://www.postgresql.org/docs/current/transaction-iso.html#XACT-SERIALIZABLE).

The key note here is that, in the interest of performance, the database *does not* actually run them
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

> Operations upon data are *read* or *write* (a write: insert, update, or delete). Two operations are conflicting if they are of different transactions, upon the same datum (data item), and at least one of them is write. Each such pair of conflicting operations has a conflict type: it is either a read–write, or write–read, or a write–write conflict. The transaction of the second operation in the pair is said to be in conflict with the transaction of the first operation. 

So far, we can start thinking of Read-Write Locks to avoid said conflicts.
[Two-Phase Locking](https://en.m.wikipedia.org/wiki/Two-phase_locking) explains how to handle these
this approach without deadlocks, and would be the first approach we could consider. In fact,
I would consider it the simplest general-case solution to the problem, without understanding
the details of the actual transactions being executed.

### Commumtative Operations

> A more general definition of conflicting operations (also for complex operations, which may each consist of several "simple" read/write operations) requires that they are noncommutative (changing their order also changes their combined result). Each such operation needs to be atomic by itself (using proper system support) in order to be considered an operation for a commutativity check. For example, read–read operations are commutative (unlike read–write and the other possibilities) and thus read–read is not a conflict. Another more complex example: the operations increment and decrement of a counter are both write operations (both modify the counter), but do not need to be considered conflicting (write-write conflict type) since they are commutative (thus increment–decrement is not a conflict; e.g., already has been supported in the old IBM's IMS "fast path"). 

The idea of "commutative operations" gets very attractive for our case. In this case, we need no
locks, or special handling of the packet and ack ordering. One way to guarantee communativity
is to create a [data structure that is a CRDT](#crdt) (link to section below). Often that is
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

## CRDT

**TODO**

Explain **commutative** replicated data types.

Allows unordered channels without any more concerns about data consistency.

## Locking

**TODO** Brief example and explain why it is not a good solution for IBC.

Note: requires **ordered** channels, as commit ordering is essential here.

## Vector Clocks

**TODO**

Show how we maintain multiple possible states, such that any merge order is valid.

This requires careful construction of the merge function, but can allow unordered channels

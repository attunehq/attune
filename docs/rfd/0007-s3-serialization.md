# Serialization with S3

## Background

Today we use transactions and retries for publish serialization. This works for database state, because we can rely on Postgres's transaction guarantees. However, S3 does not provide transaction semantics, so S3 state can become inconsistent.

For example, imagine the following sequence of events:

1. Index change C1 commits its database transaction, writing index state I1 to the database.
2. Index change C2 commits its database transaction, writing index state I2 to the database.
3. Index change C2 uploads index I2 to S3.
4. Index change C1 uploads index I1 to S3.

Since indexes will override each other, S3 will contain index state I1 instead of I2, even though I2 is the correct state. However, the database _is_ in the correct state (I2), so we can trivially restore the correct state by re-uploading the current database state to S3.

This scenario is the _out-of-order upload_ problem.

There are other ways you can imagine this problem occurring:

- The _interleaved uploads_ problem: changes might interleave their uploads to S3, leaving S3 in an inconsistent state that is neither I1 nor I2.
- The _partial upload_ problem: handlers might crash while uploading to S3, leaving a partially updated state in between I1 and I2.

This RFD discusses ways to mitigate this inconsistency problem.

## Proposals

### Implementing `by-hash` retrieval

The canonical solution to this is to implement `Acquire-By-Hash`, which is an APT application-level solution to race conditions resulting from non-transactional repositories. This involves uploading package indexes to routes based on their content address (rather than their index name), so that only the top-level `Release` files need to be updated atomically.

For more details, see:

- [Debian specification](https://wiki.debian.org/DebianRepository/Format#Acquire-By-Hash)
- [Ubuntu specification](https://wiki.ubuntu.com/AptByHash)
- [Debian developer blog post](https://www.chiark.greenend.org.uk/~cjwatson/blog/no-more-hash-sum-mismatch-errors.html)

This mitigates the interleaved uploads problem, but not the partial upload problem or the out-of-order upload problem.

### Periodically re-synchronize state

Since the correct S3 state can be completely reconstructed from Postgres state, we can create a handler that synchronizes S3 state from Postgres, and a worker to periodically trigger this logic.

This mitigates the partial upload problem, but not the out-of-order upload problem or the interleaved uploads problem.

In theory, it's actually possible for this approach to not solve the out-of-order problem either. For example, imagine a cron job whose invocation always happens exactly before an index change, and whose upload always happens after the corresponding index change. In this case, the release file would always be in the old state. In practice, this failure mode seems exceedingly unlikely.

### Resynchronize after upload batches

Periodic resynchronization means S3 state may be inconsistent for up to the cron job period. If we want to be more proactive, we can trigger the resynchronization handler after each "batch" of uploads (since concurrent batches are more likely to cause inconsistency).

This doesn't give us any stronger guarantees than periodic state resynchronization, because a new index change could always occur concurrently to the triggered resynchronization.

### Locking mechanism for uploads

The problem with S3 uploads ensuring that concurrent index changes upload to S3 in order. We could use Postgres advisory locks to implement some form of "upload locking", so that each index change needs to wait until a previous upload has finished.

This seems complicated to design correctly, and it doesn't mitigate the partial uploads problem. In fact, it makes the partial uploads problem more complicated, because that problem is caused by handler crashes, and now we also have to handle lock timeouts in case of handler crashes.

### Use a queue

Rather than locking uploads, we could use a queue to ensure that only one index change can occur at a time to a specific repository release. This is a simpler high-level design, but still has significant end-to-end complexity, because each index change requires us to keep clients waiting in order to be ready to sign a generated index, and we would need to handle timeouts and crashes between three components (the CLI, the control plane, and the queue worker).

Here's a sketch for how a queueing design could work:

1. On index change, the CLI requests a job ID and polls for an index to sign.
2. A backend worker (at most one per release) generates the index, and places it in a field in the job row for the control plane to return to the CLI on the next poll. The worker then polls for a signature.
3. The CLI picks up the index, signs it, and uploads a signature, and then polls for job completion. The control plane puts this signature in a field in the job row.
4. The worker picks up the signature, verifies it, commits the index change in the database, uploads the changes to S3, and marks the job complete in the job row.
5. The CLI sees that the job is completed and returns success.

This design requires us to think about various possible crashes and network partitions:
- Dead/unreachable CLI
  - The CLI will time out its polling, and return failure.
  - The worker will notice via timeout that the CLI has not responded in time, and will mark the job as failed.
- Dead/unreachable control plane
  - The CLI will notice via network error, and return failure.
  - The worker will notice that the job fails to progress via timeout (like in the dead CLI case).
- Dead/unreachable worker
  - The CLI will time out its polling, and return failure.

It's possible, but much more complicated end-to-end than transactions and retries, and requires much more coordination.

This design mitigates the interleaved uploads problem and the out-of-order upload problem, but does not mitigate the partial upload problem.

## Recommendation

My recommendation is to implement:

1. `Acquire-By-Hash` retrieval in order to mitigate interleaved uploads.
2. Periodic resynchronization in order to mitigate out-of-order uploads and partial uploads.

I think these two solutions provide the best value-per-complexity and handle all currently known failure modes. If new failure modes arise, we can consider additional solutions.

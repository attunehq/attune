# Publish serialization

## Problem

Users publish to Attune in CD environments that run publishing pipelines in parallel. When users kick off a release run, this often publishes multiple packages to the same repository at the same time. For example, a release run might publish a new OSS and Enterprise edition of a package, or a new package for both the 1.x and 2.x version lines.

When concurrent publishes occur, we need to make sure that they can't clobber each other.

## Background

When a user integrates Attune in CI, it looks something like this:

```bash
attune repo pkg add <package>
attune repo sync
```

In today's design, index generation and index signing steps occur asynchronously. Consider the following sequence:

1. CI job 1 publishes package A.
2. CI job 1 generates index 1 (containing A) for signing.
3. CI job 2 publishes package B.
4. CI job 2 generates index 2 (containing A and B) for signing.
5. CI job 2 signs index 2.
6. CI job 1 signs index 1.

In this scenario, index 1 (which does not contain package B) will be the one published, because the index signing endpoint currently always publishes the latest signature.

Consider another sequence:

1. CI job 1 publishes package A.
2. CI job 2 publishes package B, concurrently to step 3.
3. CI job 1 generates index 1 (containing A) for signing, concurrently to step 2.
4. CI job 2 generates index 2 (containing A and B) for signing.
5. CI job 2 signs index 2.
6. CI job 1 signs index 1.

In this scenario, the index 1 will be the one published, but the problem is that index 1 does not contain package B even though package B was added before index 1 was generated.

In order to get index signing to behave correctly in the face of concurrency, we need to enforce two invariants:

1. The latest signed index is always the one published.
2. Generated indexes always include every package that was added before the index was generated.

## Design

To enforce invariant (1), we'll:

1. Add a timestamp to the generated index.
2. Take the index timestamp as a parameter in the signature submission endpoint.
3. Store the "index timestamp" of the current signature in the database.
4. Only allow signatures with newer index timestamps.

An authed client could theoretically spoof the signature timestamp, but they would only be able to damage their own repository. As future work, we should verify the integrity of indexes and signatures to resist insider attacks.

Postgres should already enforce invariant (2) for us, because the publishing takes place in a transaction that always commits before index generation in the corresponding CI run occurs (because the publish endpoint doesn't return until the transaction commits, and therefore the CLI command for adding a package doesn't complete until the transaction commits, and the index generation can't occur until the CLI command for adding a package completes).

Just to be extra safe, I'm going to make index generation, index signing, and all package updates (add, update, delete) handlers run in transactions at the serializable isolation level. This shouldn't actually be necessary since there can't be read/write cycles in these operations (adding a package doesn't read from the index tables, and creating an index doesn't write to the package tables), but seeing serialization errors will be a good indication that something strange is happening.

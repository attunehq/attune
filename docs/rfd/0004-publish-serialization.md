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

1. Compute a "fingerprint" for the generated index, based on a hash of the index contents.
2. Store the fingerprint of the current generated index in the database.
3. Take the index fingerprint as a parameter in the signature submission endpoint.
4. Only accept signatures with a fingerprint that matches the current fingerprint.

Postgres should already enforce invariant (2) for us, because the publishing takes place in a transaction that always commits before index generation in the corresponding CI run occurs (because the publish endpoint doesn't return until the transaction commits, and therefore the CLI command for adding a package doesn't complete until the transaction commits, and the index generation can't occur until the CLI command for adding a package completes).

Just to be extra safe, I'm going to make index generation, index signing, and all package updates (add, update, delete) handlers run in transactions at the serializable isolation level. This shouldn't actually be necessary for most operations since there can't be read/write cycles in these operations (adding a package only reads and writes to the packages table and users will basically never add the same package concurrently and therefore won't conflict with other adds; creating an index reads from the package tables but doesn't write to them and therefore won't conflict with package updates) except maybe for signing or generating indexes concurrently. On serialization error, the whole handler endpoint will fail, and the CLI will retry the signature from the client side.

## Future work

- Verifying GPG signatures. We don't actually verify that the uploaded GPG signatures are valid. This allows malicious clients to upload signatures that are invalid, and therefore cause end-user package installations to fail.
- Verifying the signed indexes. We don't actually verify that the clearsigned index contents match the generated index. This allows malicious clients to upload a clearsigned index that is validly signed but has modified contents. Content modifications can only cause the end-user to see packages that were uploaded or not yet published, or prevent the end-user from seeing packages that were published.

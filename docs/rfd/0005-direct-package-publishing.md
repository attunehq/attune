# Direct package publishing

## Background

As part of [#44](https://github.com/attunehq/attune/issues/44), we're making a change to the UI semantics to remove the concept of "staged" packages. Instead, adding or removing a package will immediately "publish" the package and make it visible to consumers.

This dramatically simplifies our UX (there is no longer a separate "sync" step), our data plane design (we no longer need a "staging" area), and our control plane design (there is no longer ambiguity about whether index rows represent the "current" or "working" state of the index). However, it also makes other parts of the design more complex. In particular, adding a package now needs a signing key, and adding a package requires us to immediately regenerate the index and sign it.

From a performance perspective, I think this is fairly de-risked:

- Index generations per package aren't actually _that_ slow, because we don't actually need to regenerate every index. Since we know that we are always adding or removing exactly one package, we only need to regenerate exactly two index files: the index of the `(component, arch)` of the newly modified package, and the top-level Release file of the distribution.
- I benchmarked this locally on $REDACTED_CUSTOMER's mirrored production repository, and it took about 300ms.

### The hard part

The real question is how we retain publish serialization in this new workflow. Since index signing _must_ take place locally, every package upload is at least a two-request process: one request to perform the package upload, and one request to sign the index. This makes it impossible to perform transactionally, because:

1. We can't hold a transaction across both requests.
2. Let's say the index starts in containing package `A`, and we concurrently try to add packages `B` and `C`. If two package additions are concurrent, then we may generate two indexes `A+B` and `A+C`. But we can't actually combine these into `A+B+C` in the backend, because only the client can sign the index! We need to somehow get a signature for `A+B+C` from the client.

## Design

We have a couple of conflicting goals we would like to satisfy:

1. We want to make sure that the resulting index is _complete_. It must not be missing packages that were added concurrently, and we must not "clobber" concurrent package additions.
2. We want to minimize the amount of time it takes to publish a package. In particular, this means avoiding redundant uploads from the CLI to the control plane, which is the slowest part of the process.
3. We want to minimize the accessibility of "unsigned" packages to consumers. If a package hasn't been "fully published", ideally users shouldn't be able to download it at all.
4. We want to avoid _accidental signing_. The threat model here is an attacker who has access to an Attune API token but does not have access to the index signing key. Even if the attacker can craft arbitrary requests to the control plane, they should not be able to upload a package that is both network-accessible and appears to be legitimately signed to consumers.

There are a couple of possible designs here that trade off these goals differently.

### A: Full retries

Let's say we finalize a package addition by uploading a signed index. When we do this, we need to detect whether the current state of the index has changed since we signed it (this is the `A+B+C` problem above), so we'll also upload a "fingerprint" (hash) of the starting index and compare whether it has changed compared to the latest index. We currently do this today.

What should we do if the fingerprint _has_ changed? In design A, we throw everything away and start over, including deleting the uploaded package from object storage and re-uploading it. This is for goal 3, to minimize accessibility of "unpublished" packages.

### B: Retry with object storage caching

Uploading packages over and over again is slow. What if instead, we just let the package sit in the object storage bucket during retries, and avoid re-uploading it? This is design B. Here, we are dramatically improving goal 2 (publishing performance) by trading off goal 3 (consumers now have a longer window to download an "unpublished" package that they guess the key of).

### C: Retry with temporary storage caching

We could try avoiding exposing packages in object storage by instead caching them in temporary storage in the control plane container until they are fully published. This is actually really complicated, because it requires sticky sessions (otherwise you might send your signed index to a control plane instance that does not have your package cached) and because the control plane might die and lose the cached package and become inconsistent. I haven't fully fleshed out how we would do this because I think it's just way too complicated.

### D: Retry with staging object storage

Another way to avoid exposing packages is to bring back staging. This would still require a `CopyObject`, but S3 to S3 should be fast. The worst part about this is that it requires a staging area (either a bucket or object key prefix), which I think does not provide sufficient value to justify the additional operational complexity.

### E: Make it transactional

What if we could make the two requests (uploading and signing) somehow transactional? I've looked a little bit into how we could do this using Postgres advisory locks and stuff like that, but this really seems like too much trouble. We would need to figure out how to coordinate lock timeouts across multiple requests. What happens if the CLI process dies between requests? I just really don't think it's worthwhile complexity.

### F: Batch signing

We could just do what we currently do, which is sign whatever latest form of the index contains all the uploaded packages, even if that includes packages that I did not explicitly add during my current invocation. This is the least complex option, but it also violates goal 4 (accidental signing), so I think it's a non-starter.

## Recommendation

I think we should go with B, because I just don't think goal 3 is very important. Why?

1. In the threat model of an authenticated attacker who doesn't have the signing key, the consumer must _already_ resist downloading packages that are accessible but not within a signed index.
2. Packages are _always_ already exposed a little before they are "fully published" because S3 doesn't allow us to atomically upload multiple object keys (and therefore we upload the package files into the pool before we update the Release file).
3. APT clients already specifically ignore packages that are not within a signed index, because other APT server implementations do this.
4. There's no way for APT clients to enumerate accessible packages that are not within a signed index, because there are no directory index packages. To download an unpublished package, a user would have to completely guess the object key of the package file.

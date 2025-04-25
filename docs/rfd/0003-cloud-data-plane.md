# Cloud data plane

The goal of this design is to serve repositories in the cloud. We're optimizing here to minimize hosting cost while keeping good performance for maintainer actions.

## Design

### Storage

For storage, we'll use Cloudflare R2, since they (unbelievably) offer free egress. Since they have an S3-compatible API, and we don't use any endpoints that they don't support (see [list](https://developers.cloudflare.com/r2/api/s3/api/)), we won't need to make major changes to our `PutObject` and `CopyObject` logic. However, we'll need to store different tenant objects either in different buckets or under different prefixes to prevent them from clashing.

For now, we'll serve out of a prefix. In order to determine where to store objects, we'll modify the `debian_repository` table to add two fields:

1. `s3_bucket TEXT NOT NULL`: The name of the bucket where objects are stored. The control plane will provide an application-level default value for this if it's not configured by the user on control plane boot.
2. `s3_prefix TEXT NOT NULL`: The prefix under which objects are stored. This can be set to the empty string to mimic the current behavior of saving objects at root (e.g. if you want to publish to the root of your own S3 bucket). By default, this will be set to the empty string unless the user configures multi-tenant mode when the control plane is booted.

We'll also put a unique index on `(s3_bucket, s3_prefix, distribution)`, so that repositories that are being served at different URIs cannot be stored in the same location.

### Routing

In order to serve custom subdomains, we'll use Cloudflare Workers and [Cloudflare for SaaS](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/)'s [custom hostnames](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/domain-support/) feature. We'll route these subdomains to our fallback domain using the [standard configuration](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/#standard-cloudflare-for-saas-configuration), since most of these will not be apex domains (I expect them to look mostly like `apt.releases.example.com`). For now, setting up `CNAME`s on customer subdomains and doing the setup and validation step will be manual.

When the Worker receives a request, it will examine the request hostname, look up the corresponding repository in the `debian_repository` table, and serve the requested object out of R2 by constructing a key using the request hostname and path.

For users who want to use AWS, there should be an equivalent setup possible using S3, CloudFront, and Lambda@Edge.

### Multi-tenant mode and backwards compatibility

For users that are deploying with Attune in their own bucket, they can set their `s3_prefix` to the empty string to continue publishing to the same location. This design means we won't need separate code paths for serving single-tenant and multi-tenant deployments.

Instead, "multi-tenant mode" will be a flag set on the control plane at startup. When enabled, created repositories will have a non-empty `s3_prefix`. Otherwise, they will be created with an empty `s3_prefix`.

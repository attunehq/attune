# Cloud data plane

The goal of this design is to serve repositories in the cloud. We're optimizing here to minimize hosting cost while keeping good performance for maintainer actions.

## Design

### Object storage

For object storage, we'll use Cloudflare R2, since they (unbelievably) offer free egress. Since they have an S3-compatible API, and we don't use any endpoints that they don't support (see [list](https://developers.cloudflare.com/r2/api/s3/api/)), we won't need to make major changes to our code to use it. However, we'll need to store different tenant objects either in different buckets or under different prefixes to prevent them from clashing.

For now, we'll serve out of a prefix. This means we'll need to add "single tenant mode" and "multi tenant mode" to the server, which will have different object key formats.

### Routing

In order to serve custom subdomains, we'll use Cloudflare Workers and [Cloudflare for SaaS](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/)'s [custom hostnames](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/domain-support/) feature. We'll route these subdomains to our fallback domain using the [standard configuration](https://developers.cloudflare.com/cloudflare-for-platforms/cloudflare-for-saas/#standard-cloudflare-for-saas-configuration), since most of these will not be apex domains (I expect them to look mostly like `apt.releases.example.com`). For now, setting up `CNAME`s on customer subdomains and doing the setup and validation step will be manual.

When the Worker receives a request, it will examine the request hostname, look up the corresponding repository in the `debian_repository` table, and serve the requested object out of R2 by constructing a key using the request hostname and path.

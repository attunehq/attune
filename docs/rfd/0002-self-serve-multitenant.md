# Multi-tenant instances

The goal of this design is to add _multi-tenant_ support to a single Attune instance. This is in preparation for a cloud-hosted Attune instance.

## Overview

We'll spin up a `webui` service that will handle authentication and authorization for multi-tenant instances. We'll power authentication using [Better Auth](https://www.better-auth.com/).

TODO:

- How exactly do we configure Better Auth, including plugins?
- How do we tie users and organizations to repositories?
- If self-serve, how do we implement usage quotas and limits to prevent abuse?

# Avoiding fingerprints

## Background

In our change from staged publishing to direct package publishing, we retained the "fingerprint" mechanism for detecting staleness in signed indexes.

This design has always been a bit insecure. Imagine a malicious user who is authenticated with the control plane and would like to make arbitrary changes to the release index (e.g. to exploit a parsing bug in the APT client). With fingerprinting, any client that can successfully guess the fingerprint index can publish arbitrary release file contents. Before we moved from staging to direct publishing, this was mitigated by the fact that indexes could be in a "staged" state, where their contents were not immediately visible to consumers, and required a bit of work to reconstruct and fingerprint. However, now the indexes are all immediately published, so it's trivial to race to determine the current fingerprint.

## Design

Instead of using fingerprints, we're going to make index generation deterministic, and require clients to submit their expected index diff when they submit index signatures. On the server side, we'll then replay that index generation and check that it matches the submitted signatures.

This will prevent attackers from publishing arbitrary content into indexes, although they will still be able to make arbitrary legal index changes (e.g. adding and removing packages). This also allows us to remove the fingerprint mechanism, which simplifies our design a bit.

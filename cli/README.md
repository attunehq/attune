# Attune CLI

This directory contains the source code for the Attune CLI tool.

## Releasing New Versions

The Attune CLI uses [GoReleaser](https://goreleaser.com/) for building and releasing new versions. Follow these steps to create a new release:

1. **Prerequisites**
   - Install GoReleaser following the [official installation guide](https://goreleaser.com/install/)
   - Create a [fine-grained personal access token](https://github.com/settings/tokens?type=beta) with Read and Write access to Contents
   - Make sure all your changes are committed and pushed

2. **Release Process**
   ```bash
   # 1. Create and push a new tag
   git tag -a v1.x.x -m "Release v1.x.x"
   git push origin v1.x.x

   # 2. Set your GitHub token
   export GITHUB_TOKEN=your_github_token

   # 3. Run GoReleaser
   goreleaser release
   ```

The release process will:
- Build binaries for multiple platforms (Linux, macOS, Windows)
- Create a GitHub release with the binaries
- Generate and upload checksums
- Add release notes from Git commits
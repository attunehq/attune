---
sidebar_position: 1
---

# macOS Notarization

This guide covers the process of code signing and notarizing macOS binaries for the Attune CLI. While code signing and notarization are not technically required, we do this to make it easier for users to adopt the tooling. Without signing and notarization, users would need to either build the binary themselves from source code or deal with the friction of running unsigned binaries (which macOS warns against and makes difficult to execute).

## Prerequisites

- Apple Developer account
- Xcode Command Line Tools (meaning you can only follow this guide on macOS)
- The Attune CLI binary you want to sign and notarize

## Step 1: Create and Download Certificate

### 1.1 Create Certificate

1. Log in to your [Apple Developer account](https://developer.apple.com/account/)
2. Navigate to [Certificates](https://developer.apple.com/account/resources/certificates/list)
3. Click the **+** button to create a new certificate
4. Select **Developer ID Application**
5. Follow the prompts to generate and download the certificate

### 1.2 Import Certificate

Download and double-click the certificate file to import it into your Keychain. Alternatively, you can use the `security` command-line tool:

## Step 2: Code Sign the Binary

Use the `codesign` command to sign your binary. Replace `"XYZ"` with your actual Apple Developer Team ID (which you can find [here](https://developer.apple.com/account#MembershipDetailsCard)):

```bash
codesign --sign "XYZ" \
         --timestamp \
         --options=runtime \
         --force \
         attune
```

### Code Signing Options Explained

- `--sign`: Specifies the signing identity (your Team ID)
- `--timestamp`: Includes a secure timestamp (required for notarization)
- `--options=runtime`: Enables the hardened runtime (required for notarization)
- `--force`: Replaces any existing signature

## Step 3: Create App-Specific Password

For notarization, you'll need an app-specific password:

1. Go to [appleid.apple.com](https://appleid.apple.com/)
2. Sign in with your Apple ID
3. Navigate to **Sign-In and Security** → **App-Specific Passwords**
4. Click the **+** button to create a new app-specific password
5. Enter a label
6. Save the password securely

## Step 4: Set Up Notarization Credentials

Store your credentials securely using the `notarytool`:

```bash
xcrun notarytool store-credentials "notarytool-password" \
        --apple-id "user@example.com" \
        --team-id "XYZ" \
        --password "abcd-efgh-ijkl-mnop"
```

## Step 5: Prepare Binary for Notarization

Create a ZIP archive of your signed binary:

```bash
zip attune.zip attune
```

## Step 6: Submit for Notarization

Submit the ZIP file for notarization:

```bash
xcrun notarytool submit attune.zip \
        --keychain-profile "notarytool-password" \
        --wait \
        --verbose
```

## TODOs

- [ ] Update docs to include CI/CD integration instructions using the Notary API

## References

- [Apple Developer Documentation: Customizing the Notarization Workflow](https://developer.apple.com/documentation/security/customizing-the-notarization-workflow)
- [Code Signing and Notarizing macOS Binaries Outside Apple App Store](https://dennisbabkin.com/blog/?t=how-to-get-certificate-code-sign-notarize-macos-binaries-outside-apple-app-store)
- [Apple Notary API Documentation](https://developer.apple.com/documentation/notaryapi)

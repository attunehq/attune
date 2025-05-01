package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"strings"
	"syscall"

	"github.com/ProtonMail/gopenpgp/v3/crypto"
	"github.com/spf13/cobra"
	"golang.org/x/term"
)

type RepositoryIndexes struct {
	Release string
}

type SyncRepositoryRequest struct {
	Clearsigned string `json:"clearsigned"`
	Detached    string `json:"detached"`
}

func repoSyncCmd() *cobra.Command {
	cmd := &cobra.Command{
		// Other potential names: "commit", "deploy", "update", "push"?
		Use:   "sync",
		Short: "Synchronize unsaved changes to repository",
		Long: `Synchronize unsaved changes to repository.

This command signs and publishes the repository's Release file using GPG. You must
specify exactly one of the following signing methods:

1. --signing-key-file=<path>: Provide a file containing an armored GPG private key.
2. --signing-key-id=<key-id>: Use your local GPG installation with the specified key ID (fingerprint, email, etc.).

When using local GPG (--signing-key-id), the command will invoke the system's gpg
command to sign the Release file. This allows using keys stored in your local
keyring, GPG agent, or hardware tokens.`,
		Run: func(cmd *cobra.Command, args []string) {
			repoID, err := cmd.Flags().GetInt("repo-id")
			if err != nil {
				fmt.Printf("could not read --repo-id: %s\n", err)
				os.Exit(1)
			}

			// Get signing method flags and make sure exactly one method is selected.
			signingKeyFile, err := cmd.Flags().GetString("signing-key-file")
			if err != nil {
				fmt.Printf("could not read --signing-key-file: %s\n", err)
				os.Exit(1)
			}
			signingKeyID, err := cmd.Flags().GetString("signing-key-id")
			if err != nil {
				fmt.Printf("could not read --signing-key-id: %s\n", err)
				os.Exit(1)
			}
			if (signingKeyFile == "") == (signingKeyID == "") {
				fmt.Println("Error: You must specify exactly one signing method:")
				fmt.Println("  --signing-key-file=<path> OR --signing-key-id=<key-id>")
				os.Exit(1)
			}

			// Load release index for signing.
			req, err := http.NewRequest(http.MethodGet, fmt.Sprintf("/api/v0/repositories/%d/indexes", repoID), nil)
			if err != nil {
				fmt.Printf("could not create request to get repository indexes: %s\n", err)
				os.Exit(1)
			}
			res, err := API(req)
			if err != nil {
				fmt.Printf("could not get repository indexes: %s\n", err)
				os.Exit(1)
			}
			defer res.Body.Close()

			if res.StatusCode != http.StatusOK {
				fmt.Printf("could not get repository indexes: %s\n", res.Status)
				os.Exit(1)
			}

			var indexes RepositoryIndexes
			if err := json.NewDecoder(res.Body).Decode(&indexes); err != nil {
				fmt.Printf("could not decode repository indexes: %s\n", err)
				os.Exit(1)
			}

			var syncRequest *SyncRepositoryRequest
			if signingKeyFile != "" {
				// Sign release index using provided key file.
				syncRequest, err = signWithKeyFile(signingKeyFile, indexes.Release)
			} else {
				// Sign release index using local GPG installation.
				syncRequest, err = signWithLocalGPG(signingKeyID, indexes.Release)
			}

			if err != nil {
				fmt.Println(err)
				os.Exit(1)
			}

			// Start synchronization.
			jsonBody, err := json.Marshal(syncRequest)
			if err != nil {
				fmt.Printf("could not marshal SyncRepositoryRequest: %s\n", err)
				os.Exit(1)
			}

			req, err = http.NewRequest(
				http.MethodPost,
				fmt.Sprintf("/api/v0/repositories/%d/sync", repoID),
				bytes.NewReader(jsonBody),
			)
			if err != nil {
				fmt.Printf("could not create request for starting synchronization: %s\n", err)
				os.Exit(1)
			}
			req.Header.Set("Content-Type", "application/json")
			res, err = API(req)
			if err != nil {
				fmt.Printf("could not start synchronization: %s\n", err)
				os.Exit(1)
			}
			defer res.Body.Close()

			if res.StatusCode != http.StatusOK {
				fmt.Printf("could not start synchronization: %s\n", res.Status)
				os.Exit(1)
			}

			fmt.Println("Synchronization completed!")
		},
	}

	cmd.Flags().IntP("repo-id", "r", 0, "ID of the repository")
	cmd.MarkFlagRequired("repo-id")
	cmd.Flags().StringP("signing-key-file", "k", "", "File containing armored GPG private key for signing")
	cmd.Flags().StringP("signing-key-id", "i", "", "GPG key ID, fingerprint, or email to use with local GPG")

	return cmd
}

// Signs the release content using a provided GPG key file.
func signWithKeyFile(keyFilePath, releaseContent string) (*SyncRepositoryRequest, error) {
	keyFd, err := os.Open(keyFilePath)
	if err != nil {
		return nil, fmt.Errorf("could not open key file: %s", err)
	}
	defer keyFd.Close()

	key, err := crypto.NewKeyFromReader(keyFd)
	if err != nil {
		return nil, fmt.Errorf("could not parse key file: %s", err)
	}
	locked, err := key.IsLocked()
	if err != nil {
		return nil, fmt.Errorf("could not determine whether key is locked: %s", err)
	}
	if locked {
		fmt.Printf("Key is locked. Please enter passphrase: ")
		var passphrase []byte
		passphrase, err = term.ReadPassword(int(syscall.Stdin))
		if err != nil {
			return nil, fmt.Errorf("could not read passphrase: %s", err)
		}
		key, err = key.Unlock(passphrase)
		if err != nil {
			return nil, fmt.Errorf("could not unlock key: %s", err)
		}
		fmt.Println()
	}

	pgp := crypto.PGP()
	signer, err := pgp.Sign().SigningKey(key).New()
	if err != nil {
		return nil, fmt.Errorf("could not create signer: %s", err)
	}

	// Notice the trimmed newline. This is apparently a long-standing
	// compatibility bug in GPG cleartext signing. See:
	// - https://lists.gnupg.org/pipermail/gnupg-devel/1999-September/016016.html
	// - https://dev.gnupg.org/T7106
	clearsigned, err := signer.SignCleartext([]byte(strings.TrimSuffix(releaseContent, "\n")))
	if err != nil {
		return nil, fmt.Errorf("could not clearsign release index: %s", err)
	}
	detached, err := signer.Sign([]byte(releaseContent), crypto.Armor)
	if err != nil {
		return nil, fmt.Errorf("could not detached sign release index: %s", err)
	}

	return &SyncRepositoryRequest{
		Clearsigned: string(clearsigned),
		Detached:    string(detached),
	}, nil
}

// Signs the release content using the local GPG installation.
func signWithLocalGPG(keyID, releaseContent string) (*SyncRepositoryRequest, error) {
	fmt.Println("Using local GPG installation for signing")

	gpgClearsignCmd := exec.Command("gpg", "--clearsign", "--local-user", keyID, "--batch", "--yes")
	gpgClearsignCmd.Stdin = strings.NewReader(releaseContent)
	var clearsignedOutput bytes.Buffer
	gpgClearsignCmd.Stdout = &clearsignedOutput
	var clearsignedError bytes.Buffer
	gpgClearsignCmd.Stderr = &clearsignedError

	err := gpgClearsignCmd.Run()
	if err != nil {
		errMsg := fmt.Sprintf("could not clearsign release index: %s", err)
		if clearsignedError.Len() > 0 {
			errMsg += fmt.Sprintf("\nGPG error output: %s", clearsignedError.String())
		}
		return nil, fmt.Errorf("%s", errMsg)
	}
	clearsigned := clearsignedOutput.Bytes()

	gpgDetachCmd := exec.Command("gpg", "--detach-sign", "--armor", "--local-user", keyID, "--batch", "--yes")
	gpgDetachCmd.Stdin = strings.NewReader(releaseContent)
	var detachedOutput bytes.Buffer
	gpgDetachCmd.Stdout = &detachedOutput
	var detachedError bytes.Buffer
	gpgDetachCmd.Stderr = &detachedError

	err = gpgDetachCmd.Run()
	if err != nil {
		errMsg := fmt.Sprintf("could not detached sign release index: %s", err)
		if detachedError.Len() > 0 {
			errMsg += fmt.Sprintf("\nGPG error output: %s", detachedError.String())
		}
		return nil, fmt.Errorf("%s", errMsg)
	}
	detached := detachedOutput.Bytes()

	return &SyncRepositoryRequest{
		Clearsigned: string(clearsigned),
		Detached:    string(detached),
	}, nil
}

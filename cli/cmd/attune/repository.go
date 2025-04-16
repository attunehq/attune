package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"strings"
	"syscall"
	"text/tabwriter"
	"time"

	"github.com/ProtonMail/gopenpgp/v3/crypto"
	"github.com/spf13/cobra"
	"golang.org/x/term"
)

func repoCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "repo",
		Short: "Manage repositories",
		Run: func(cmd *cobra.Command, args []string) {
			cmd.Help()
		},
	}

	createRepositoryCmd.Flags().StringP("uri", "u", "", "URI of the repository")
	createRepositoryCmd.MarkFlagRequired("uri")
	createRepositoryCmd.Flags().StringP("distribution", "d", "", "Distribution of the repository")
	createRepositoryCmd.MarkFlagRequired("distribution")
	createRepositoryCmd.Flags().StringP("origin", "o", "", "Origin of the repository")
	createRepositoryCmd.MarkFlagRequired("origin")
	createRepositoryCmd.Flags().StringP("label", "l", "", "Label of the repository")
	createRepositoryCmd.MarkFlagRequired("label")
	createRepositoryCmd.Flags().StringP("suite", "s", "", "Suite of the repository")
	createRepositoryCmd.MarkFlagRequired("suite")
	createRepositoryCmd.Flags().StringP("codename", "c", "", "Codename of the repository")
	createRepositoryCmd.MarkFlagRequired("codename")
	createRepositoryCmd.Flags().StringP("description", "e", "", "Description of the repository")
	createRepositoryCmd.MarkFlagRequired("description")

	statusRepositoryCmd.Flags().IntP("repo-id", "r", 0, "ID of the repository")
	statusRepositoryCmd.MarkFlagRequired("repo-id")

	syncRepositoryCmd.Flags().IntP("repo-id", "r", 0, "ID of the repository")
	syncRepositoryCmd.MarkFlagRequired("repo-id")
	syncRepositoryCmd.Flags().StringP("signing-key-file", "k", "", "File containing armored GPG private key for signing")
	syncRepositoryCmd.MarkFlagRequired("signing-key-file")

	cmd.AddCommand(createRepositoryCmd, listRepositoriesCmd, statusRepositoryCmd, syncRepositoryCmd, repoPkgCmd())
	return cmd
}

type CreateRepositoryRequest struct {
	URI          string `json:"uri"`
	Distribution string `json:"distribution"`
	Origin       string `json:"origin"`
	Label        string `json:"label"`
	Suite        string `json:"suite"`
	Codename     string `json:"codename"`
	Description  string `json:"description"`
}

var createRepositoryCmd = &cobra.Command{
	Use:   "create",
	Short: "Create a new repository",
	Run: func(cmd *cobra.Command, args []string) {
		reqBody := CreateRepositoryRequest{
			URI:          cmd.Flag("uri").Value.String(),
			Distribution: cmd.Flag("distribution").Value.String(),
			Origin:       cmd.Flag("origin").Value.String(),
			Label:        cmd.Flag("label").Value.String(),
			Suite:        cmd.Flag("suite").Value.String(),
			Codename:     cmd.Flag("codename").Value.String(),
			Description:  cmd.Flag("description").Value.String(),
		}

		jsonBody, err := json.Marshal(reqBody)
		if err != nil {
			fmt.Printf("could not marshal CreateRepositoryRequest: %s\n", err)
			os.Exit(1)
		}

		res, err := http.Post("http://localhost:3000/api/v0/repositories",
			"application/json",
			bytes.NewBuffer(jsonBody))
		if err != nil {
			fmt.Printf("could not create repository: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not create repository: %s\n", res.Status)
			os.Exit(1)
		}

		var repository Repository
		if err := json.NewDecoder(res.Body).Decode(&repository); err != nil {
			fmt.Printf("could not decode repository: %s\n", err)
			os.Exit(1)
		}

		fmt.Println("Created new repository:")
		w := tabwriter.NewWriter(os.Stdout, 0, 8, 1, '\t', 0)
		fmt.Fprint(w, "ID\tURI\tDistribution\n")
		fmt.Fprintf(w, "%d\t%s\t%s\n", repository.ID, repository.URI, repository.Distribution)
		w.Flush()
	},
}

type Repository struct {
	ID           int
	URI          string
	Distribution string
}

var listRepositoriesCmd = &cobra.Command{
	Use:   "list",
	Short: "List repositories",
	Run: func(cmd *cobra.Command, args []string) {
		res, err := http.Get("http://localhost:3000/api/v0/repositories")
		if err != nil {
			fmt.Printf("could not list repositories: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not list repositories: %s\n", res.Status)
			os.Exit(1)
		}

		var repositories []Repository
		if err := json.NewDecoder(res.Body).Decode(&repositories); err != nil {
			fmt.Printf("could not decode repositories: %s\n", err)
			os.Exit(1)
		}

		w := tabwriter.NewWriter(os.Stdout, 0, 8, 1, '\t', 0)
		fmt.Fprint(w, "ID\tURI\tDistribution\n")
		for _, repository := range repositories {
			fmt.Fprintf(w, "%d\t%s\t%s\n", repository.ID, repository.URI, repository.Distribution)
		}
		w.Flush()
	},
}

type RepositoryStatus struct {
	Changes []RepositoryChange
}

type RepositoryChange struct {
	PackageID    int64     `json:"package_id"`
	Component    string    `json:"component"`
	Package      string    `json:"package"`
	Version      string    `json:"version"`
	Architecture string    `json:"architecture"`
	UpdatedAt    time.Time `json:"updated_at"`
	Change       string    `json:"change"`
}

var statusRepositoryCmd = &cobra.Command{
	Use:   "status",
	Short: "Show status of a repository",
	Run: func(cmd *cobra.Command, args []string) {
		repoID, err := cmd.Flags().GetInt("repo-id")
		if err != nil {
			fmt.Printf("could not read --repo-id: %s\n", err)
			os.Exit(1)
		}

		res, err := http.Get(fmt.Sprintf("http://localhost:3000/api/v0/repositories/%d", repoID))
		if err != nil {
			fmt.Printf("could not get repository status: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not get repository status: %s\n", res.Status)
			os.Exit(1)
		}

		var status RepositoryStatus
		if err := json.NewDecoder(res.Body).Decode(&status); err != nil {
			fmt.Printf("could not decode repository: %s\n", err)
			os.Exit(1)
		}

		fmt.Println("Repository status:")
		w := tabwriter.NewWriter(os.Stdout, 0, 8, 1, '\t', 0)
		fmt.Fprint(w, "ID\tAction\tComponent\tPackage\tVersion\tArchitecture\tUpdated At\n")
		for _, change := range status.Changes {
			fmt.Fprintf(
				w,
				"%d\t%s\t%s\t%s\t%s\t%s\t%s\n",
				change.PackageID,
				change.Change,
				change.Component,
				change.Package,
				change.Version,
				change.Architecture,
				change.UpdatedAt,
			)
		}
		w.Flush()
	},
}

type RepositoryIndexes struct {
	Release string
}

type SyncRepositoryRequest struct {
	Clearsigned string `json:"clearsigned"`
	Detached    string `json:"detached"`
}

var syncRepositoryCmd = &cobra.Command{
	// Other potential names: "commit", "deploy", "update", "push"?
	Use:   "sync",
	Short: "Synchronize unsaved changes to repository",
	Run: func(cmd *cobra.Command, args []string) {
		repoID, err := cmd.Flags().GetInt("repo-id")
		if err != nil {
			fmt.Printf("could not read --repo-id: %s\n", err)
			os.Exit(1)
		}
		signingKeyFile, err := cmd.Flags().GetString("signing-key-file")
		if err != nil {
			fmt.Printf("could not read --signing-key-file: %s\n", err)
			os.Exit(1)
		}

		// Load release index for signing.
		res, err := http.Get(fmt.Sprintf("http://localhost:3000/api/v0/repositories/%d/indexes", repoID))
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

		// Sign release index.
		keyFd, err := os.Open(signingKeyFile)
		if err != nil {
			fmt.Printf("could not open key file: %s\n", err)
			os.Exit(1)
		}
		defer keyFd.Close()

		key, err := crypto.NewKeyFromReader(keyFd)
		if err != nil {
			fmt.Printf("could not parse key file: %s\n", err)
			os.Exit(1)
		}
		locked, err := key.IsLocked()
		if err != nil {
			fmt.Printf("could not determine whether key is locked: %s\n", err)
			os.Exit(1)
		}
		if locked {
			fmt.Printf("Key is locked. Please enter passphrase: ")
			var passphrase []byte
			passphrase, err = term.ReadPassword(int(syscall.Stdin))
			if err != nil {
				fmt.Printf("could not read passphrase: %s\n", err)
				os.Exit(1)
			}
			key, err = key.Unlock(passphrase)
			if err != nil {
				fmt.Printf("could not unlock key: %s\n", err)
				os.Exit(1)
			}
			fmt.Println()
		}

		pgp := crypto.PGP()
		signer, err := pgp.Sign().SigningKey(key).New()
		if err != nil {
			fmt.Printf("could not create signer: %s\n", err)
			os.Exit(1)
		}

		// Notice the trimmed newline. This is apparently a long-standing
		// compatibility bug in GPG cleartext signing. See:
		// - https://lists.gnupg.org/pipermail/gnupg-devel/1999-September/016016.html
		// - https://dev.gnupg.org/T7106
		clearsigned, err := signer.SignCleartext([]byte(strings.TrimSuffix(indexes.Release, "\n")))
		if err != nil {
			fmt.Printf("could not clearsign release index: %s\n", err)
			os.Exit(1)
		}
		detached, err := signer.Sign([]byte(indexes.Release), crypto.Armor)
		if err != nil {
			fmt.Printf("could not detached sign release index: %s\n", err)
			os.Exit(1)
		}

		// Start synchronization.
		reqBody := SyncRepositoryRequest{
			Clearsigned: string(clearsigned),
			Detached:    string(detached),
		}

		jsonBody, err := json.Marshal(reqBody)
		if err != nil {
			fmt.Printf("could not marshal SyncRepositoryRequest: %s\n", err)
			os.Exit(1)
		}

		res, err = http.Post(
			fmt.Sprintf("http://localhost:3000/api/v0/repositories/%d/sync", repoID),
			"application/json",
			bytes.NewReader(jsonBody),
		)
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

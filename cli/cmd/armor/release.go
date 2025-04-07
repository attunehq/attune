package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"text/tabwriter"

	"github.com/spf13/cobra"
)

// release create
// release set-active

// hmm, how do we avoid double-saving unchanged files? maybe use content addressing for everything?
// maybe use D1 instead of KV; use transaction for atomic changes; use D1 as k/v lookup of (host, release, route) -> content addressed file?
// allow users to roll back releases by keeping the old rows around? change active release id instead of overwriting routes

func releaseCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "releases",
		Short: "Manage releases in a repository",
		Run: func(cmd *cobra.Command, args []string) {
			cmd.Help()
		},
		TraverseChildren: true,
	}
	cmd.Flags().IntP("repository-id", "r", 0, "ID of repository to operate on")

	createReleaseCmd.Flags().IntP("from", "f", 0, "Copy packages and attributes from an existing release")
	createReleaseCmd.Flags().StringP("set-origin", "o", "", "Value to set in the \"origin\" field for the new release")
	createReleaseCmd.Flags().StringP("set-label", "l", "", "Value to set in the \"label\" field for the new release")
	createReleaseCmd.Flags().StringP("set-suite", "s", "", "Value to set in the \"suite\" field for the new release")
	createReleaseCmd.Flags().StringP("set-codename", "c", "", "Value to set in the \"codename\" field for the new release")
	createReleaseCmd.Flags().StringP("set-description", "d", "", "Value to set in the \"description\" field for the new release")

	cmd.AddCommand(createReleaseCmd, listReleasesCmd, promoteReleaseCmd)
	return cmd
}

type CreateReleaseRequest struct {
	RepositoryID int    `json:"repository_id"`
	From         *int   `json:"from,omitempty"`
	Origin       string `json:"origin,omitempty"`
	Label        string `json:"label,omitempty"`
	Suite        string `json:"suite,omitempty"`
	Codename     string `json:"codename,omitempty"`
	Description  string `json:"description,omitempty"`
}

var createReleaseCmd = &cobra.Command{
	Use:              "create",
	Short:            "Create a new repository release",
	TraverseChildren: true,
	Run: func(cmd *cobra.Command, args []string) {
		// Read flags.
		if !cmd.Parent().Flags().Changed("repository-id") {
			// NOTE: (*cobra.Command).MarkFlagRequired does not work on parent flags.
			fmt.Println("error: --repository-id must be set")
			os.Exit(1)
		}
		repositoryID, err := cmd.Parent().Flags().GetInt("repository-id")
		if err != nil {
			fmt.Printf("could not read --repository-id: %s\n", err)
			os.Exit(1)
		}

		var from *int
		from = nil
		if cmd.Flags().Changed("from") {
			fromInt, err := cmd.Flags().GetInt("from")
			if err != nil {
				fmt.Printf("could not read --from: %s\n", err)
				os.Exit(1)
			}
			from = &fromInt
		}

		origin, err := cmd.Flags().GetString("set-origin")
		if err != nil {
			fmt.Printf("could not read --set-origin: %s\n", err)
			os.Exit(1)
		}
		label, err := cmd.Flags().GetString("set-label")
		if err != nil {
			fmt.Printf("could not read --set-label: %s\n", err)
			os.Exit(1)
		}
		suite, err := cmd.Flags().GetString("set-suite")
		if err != nil {
			fmt.Printf("could not read --set-suite: %s\n", err)
			os.Exit(1)
		}
		codename, err := cmd.Flags().GetString("set-codename")
		if err != nil {
			fmt.Printf("could not read --set-codename: %s\n", err)
			os.Exit(1)
		}
		description, err := cmd.Flags().GetString("set-description")
		if err != nil {
			fmt.Printf("could not read --set-description: %s\n", err)
			os.Exit(1)
		}

		// Check flagset validity. Either `--from` flag must be set, or all the
		// field flags must be set.
		//
		// If `--from` is set and field flags are also set, then the field flags
		// will override values inherited from the source release. This
		// functionality is implemented in the backend.
		allFieldsSet := origin != "" && label != "" && suite != "" && codename != "" && description != ""
		if from == nil && !allFieldsSet {
			fmt.Println("error: --from must be set, or else all field flags must be set")
			os.Exit(1)
		}

		// Make the API call to create the release.
		reqBody, err := json.Marshal(CreateReleaseRequest{
			RepositoryID: repositoryID,
			From:         from,
			Origin:       origin,
			Label:        label,
			Suite:        suite,
			Codename:     codename,
			Description:  description,
		})
		if err != nil {
			fmt.Printf("could not marshal request body: %s\n", err)
			os.Exit(1)
		}
		req, err := http.NewRequest(http.MethodPost, "http://localhost:3000/api/v0/releases", bytes.NewReader(reqBody))
		if err != nil {
			fmt.Printf("could not create request: %s\n", err)
			os.Exit(1)
		}
		req.Header.Set("Content-Type", "application/json")
		client := &http.Client{}
		res, err := client.Do(req)
		if err != nil {
			fmt.Printf("could not make request: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		// Check response.
		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not create release: %s\n", res.Status)
			os.Exit(1)
		}
		body, err := io.ReadAll(res.Body)
		if err != nil {
			fmt.Printf("could not read response body: %s\n", err)
			os.Exit(1)
		}

		// TODO: Print the response in a nicer way.
		fmt.Printf("%s\n", string(body))
	},
}

type Release struct {
	ID          int    `json:"id"`
	Origin      string `json:"origin"`
	Label       string `json:"label"`
	Suite       string `json:"suite"`
	Codename    string `json:"codename"`
	Date        string `json:"date"`
	Description string `json:"description"`
	Active      bool   `json:"active"`
	Signed      bool   `json:"signed"`
	Stale       bool   `json:"stale"`
}

var listReleasesCmd = &cobra.Command{
	Use:   "list",
	Short: "List repository releases",
	Run: func(cmd *cobra.Command, args []string) {
		// Read flags.
		if !cmd.Parent().Flags().Changed("repository-id") {
			// NOTE: (*cobra.Command).MarkFlagRequired does not work on parent flags.
			fmt.Println("error: --repository-id must be set")
			os.Exit(1)
		}
		repositoryID, err := cmd.Parent().Flags().GetInt("repository-id")
		if err != nil {
			fmt.Printf("could not read --repository-id: %s\n", err)
			os.Exit(1)
		}

		// Make the API call to list releases.
		req, err := http.NewRequest(http.MethodGet, "http://localhost:3000/api/v0/releases", nil)
		if err != nil {
			fmt.Printf("could not create request: %s\n", err)
			os.Exit(1)
		}
		req.Header.Set("Content-Type", "application/json")
		q := req.URL.Query()
		q.Set("repository_id", fmt.Sprintf("%d", repositoryID))
		req.URL.RawQuery = q.Encode()

		client := &http.Client{}
		res, err := client.Do(req)
		if err != nil {
			fmt.Printf("could not list releases: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		// Check response.
		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not list releases: %s\n", res.Status)
			os.Exit(1)
		}

		// Decode response.
		var releases []Release
		if err := json.NewDecoder(res.Body).Decode(&releases); err != nil {
			fmt.Printf("could not decode releases: %s\n", err)
			os.Exit(1)
		}

		w := tabwriter.NewWriter(os.Stdout, 0, 8, 1, '\t', 0)
		fmt.Fprint(w, "ID\tDate\tActive\tSigned\tStale\n")
		for _, release := range releases {
			fmt.Fprintf(w, "%d\t%s\t%t\t%t\t%t\n", release.ID, release.Date, release.Active, release.Signed, release.Stale)
		}
		w.Flush()
	},
}

var promoteReleaseCmd = &cobra.Command{
	Use:   "promote",
	Short: "Sign a release and promote it to active",
	Run: func(cmd *cobra.Command, args []string) {
		panic("not implemented")
	},
}

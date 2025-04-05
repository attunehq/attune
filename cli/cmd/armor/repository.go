package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"text/tabwriter"

	"github.com/spf13/cobra"
)

func repositoryCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "repositories",
		Short: "Manage repositories",
		Run: func(cmd *cobra.Command, args []string) {
			cmd.Help()
		},
	}

	cmd.AddCommand(createRepositoryCmd, listRepositoriesCmd)
	return cmd
}

var createRepositoryCmd = &cobra.Command{
	Use:   "create",
	Short: "Create a new repository",
	Run: func(cmd *cobra.Command, args []string) {
		panic("not implemented")
	},
}

type Repository struct {
	ID              int    `json:"id"`
	URI             string `json:"uri"`
	Distribution    string `json:"distribution"`
	ActiveReleaseID *int   `json:"active_release_id"`
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
		fmt.Fprint(w, "ID\tURI\tDistribution\tActive release\n")
		for _, repository := range repositories {
			activeReleaseStr := "(none)"
			if repository.ActiveReleaseID != nil {
				activeReleaseStr = fmt.Sprintf("%d", *repository.ActiveReleaseID)
			}
			fmt.Fprintf(w, "%d\t%s\t%s\t%s\n", repository.ID, repository.URI, repository.Distribution, activeReleaseStr)
		}
		w.Flush()
	},
}

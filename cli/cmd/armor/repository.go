package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"text/tabwriter"

	"github.com/spf13/cobra"
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

	cmd.AddCommand(createRepositoryCmd, listRepositoriesCmd, statusRepositoryCmd, syncRepositoryCmd, repoPkgCmd())
	return cmd
}

var createRepositoryCmd = &cobra.Command{
	Use:   "create",
	Short: "Create a new repository",
	Run: func(cmd *cobra.Command, args []string) {
		reqBody := map[string]string{
			"uri":          cmd.Flag("uri").Value.String(),
			"distribution": cmd.Flag("distribution").Value.String(),
			"origin":       cmd.Flag("origin").Value.String(),
			"label":        cmd.Flag("label").Value.String(),
			"suite":        cmd.Flag("suite").Value.String(),
			"codename":     cmd.Flag("codename").Value.String(),
			"description":  cmd.Flag("description").Value.String(),
		}

		jsonBody, err := json.Marshal(reqBody)
		if err != nil {
			fmt.Printf("could not marshal repository request: %s\n", err)
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

var statusRepositoryCmd = &cobra.Command{
	Use:   "status",
	Short: "Show status of a repository",
	Run: func(cmd *cobra.Command, args []string) {
		panic("not implemented")
	},
}

var syncRepositoryCmd = &cobra.Command{
	// Other names: "commit", "deploy", "update", "push"?
	Use:   "sync",
	Short: "Synchronize unsaved changes to repository",
	Run: func(cmd *cobra.Command, args []string) {
		panic("not implemented")
	},
}

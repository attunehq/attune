package main

import (
	"encoding/json"
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"
	"text/tabwriter"

	"github.com/schollz/progressbar/v3"
	"github.com/spf13/cobra"
)

func repoPkgCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "pkg",
		Short: "Manage packages",
	}
	cmd.PersistentFlags().IntP("repo-id", "r", 0, "ID of repository to change")
	cmd.MarkPersistentFlagRequired("repo-id")

	createPkgsCmd.Flags().StringP("component", "c", "", "Component to add the package to")
	createPkgsCmd.MarkFlagRequired("component")

	cmd.AddCommand(createPkgsCmd)
	return cmd
}

type PackageResponse struct {
	ID           int
	Package      string
	Version      string
	Architecture string
}

var createPkgsCmd = &cobra.Command{
	Use:   "add <filename>",
	Short: "Add a package",
	Args:  cobra.ExactArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		// Read flags.
		repoID, err := cmd.Parent().Flags().GetInt("repo-id")
		if err != nil {
			fmt.Printf("could not read --repo-id: %s\n", err)
			os.Exit(1)
		}

		component, err := cmd.Flags().GetString("component")
		if err != nil {
			fmt.Printf("could not read --component: %s\n", err)
			os.Exit(1)
		}

		// Read package file and prepare for upload.
		deb, err := os.Open(args[0])
		if err != nil {
			fmt.Printf("could not open package file: %s\n", err)
			os.Exit(1)
		}
		defer deb.Close()

		debStat, err := deb.Stat()
		if err != nil {
			fmt.Printf("could not get package file info: %s\n", err)
			os.Exit(1)
		}

		var progress *progressbar.ProgressBar
		r, w := io.Pipe()
		writer := multipart.NewWriter(w)
		go func() {
			defer w.Close()
			defer writer.Close()
			part, err := writer.CreateFormFile("file", filepath.Base(args[0]))
			if err != nil {
				fmt.Printf("could not create form file: %s\n", err)
				os.Exit(1)
			}
			progress = progressbar.DefaultBytes(debStat.Size(), "Uploading package:")
			_, err = io.Copy(io.MultiWriter(part, progress), deb)
			if err != nil {
				fmt.Printf("could not copy package file: %s\n", err)
				os.Exit(1)
			}
			progress = progressbar.NewOptions(
				-1,
				progressbar.OptionSetDescription("Processing package..."),
				progressbar.OptionSetWriter(os.Stderr),
				progressbar.OptionOnCompletion(func() {
					fmt.Fprintf(os.Stderr, "\n")
				}),
				progressbar.OptionSpinnerType(14),
				progressbar.OptionFullWidth(),
				progressbar.OptionSetRenderBlankState(true),
			)
		}()

		req, err := http.NewRequest(http.MethodPost, fmt.Sprintf("/api/v0/repositories/%d/packages", repoID), r)
		if err != nil {
			fmt.Printf("could not create request to add package: %s\n", err)
			os.Exit(1)
		}
		req.Header.Set("Content-Type", writer.FormDataContentType())
		q := req.URL.Query()
		q.Set("component", component)
		req.URL.RawQuery = q.Encode()
		res, err := API(req)
		if err != nil {
			fmt.Printf("could not make request to add package: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

		// Complete progress spinner.
		if progress != nil {
			progress.Finish()
		}

		// Check response.
		if res.StatusCode != http.StatusOK {
			fmt.Printf("could not add package: %s\n", res.Status)
			os.Exit(1)
		}
		body, err := io.ReadAll(res.Body)
		if err != nil {
			fmt.Printf("could not read response body: %s\n", err)
			os.Exit(1)
		}

		var pkg PackageResponse
		if err := json.Unmarshal(body, &pkg); err != nil {
			fmt.Printf("could not decode package: %s\n", err)
			os.Exit(1)
		}

		fmt.Println("Added new package:")
		tw := tabwriter.NewWriter(os.Stdout, 0, 8, 1, '\t', 0)
		fmt.Fprint(tw, "ID\tPackage\tVersion\tArchitecture\n")
		fmt.Fprintf(tw, "%d\t%s\t%s\t%s\n", pkg.ID, pkg.Package, pkg.Version, pkg.Architecture)
		tw.Flush()
	},
}

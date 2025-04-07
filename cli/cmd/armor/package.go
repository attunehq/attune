package main

import (
	"fmt"
	"io"
	"mime/multipart"
	"net/http"
	"os"
	"path/filepath"

	"github.com/spf13/cobra"
)

func pkgsCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "pkgs",
		Short: "Manage packages in a release",
		Run: func(cmd *cobra.Command, args []string) {
			cmd.Help()
		},
	}
	cmd.Flags().IntP("release-id", "r", 0, "ID of release to operate on")

	createPkgsCmd.Flags().StringP("component", "c", "", "Component to add the package to")
	createPkgsCmd.MarkFlagRequired("component")

	cmd.AddCommand(createPkgsCmd, removePkgsCmd, listPkgsCmd)
	return cmd
}

// type AddPackageRequest struct {
// 	ReleaseID int    `json:"release_id"`
// 	Component string `json:"component"`
// 	// This is the raw control file contents, which will be parsed in the backend.
// 	Control   string `json:"control"`
// 	MD5Sum    string `json:"md5sum"`
// 	SHA512Sum string `json:"sha512sum"`
// }

// type AddPackageResponse struct {
// 	UploadURL string `json:"upload_url"`
// }

var createPkgsCmd = &cobra.Command{
	Use:   "add <filename>",
	Short: "Add a package to a release",
	Args:  cobra.ExactArgs(1),
	Run: func(cmd *cobra.Command, args []string) {
		// Read flags.
		if !cmd.Parent().Flags().Changed("release-id") {
			// NOTE: (*cobra.Command).MarkFlagRequired does not work on parent flags.
			fmt.Println("error: --release-id must be set")
			os.Exit(1)
		}
		releaseID, err := cmd.Parent().Flags().GetInt("release-id")
		if err != nil {
			fmt.Printf("could not read --release-id: %s\n", err)
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

		r, w := io.Pipe()
		writer := multipart.NewWriter(w)
		go func() {
			defer writer.Close()
			part, err := writer.CreateFormFile("file", filepath.Base(args[0]))
			if err != nil {
				fmt.Printf("could not create form file: %s\n", err)
				os.Exit(1)
			}
			_, err = io.Copy(part, deb)
			if err != nil {
				fmt.Printf("could not copy package file: %s\n", err)
				os.Exit(1)
			}
		}()

		req, err := http.NewRequest(http.MethodPost, "http://localhost:3000/api/v0/packages", r)
		if err != nil {
			fmt.Printf("could not create request: %s\n", err)
			os.Exit(1)
		}
		req.Header.Set("Content-Type", writer.FormDataContentType())
		q := req.URL.Query()
		q.Set("release_id", fmt.Sprintf("%d", releaseID))
		q.Set("component", component)
		req.URL.RawQuery = q.Encode()
		client := &http.Client{}
		res, err := client.Do(req)
		if err != nil {
			fmt.Printf("could not make request: %s\n", err)
			os.Exit(1)
		}
		defer res.Body.Close()

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

		// TODO: Print the response in a nicer way.
		fmt.Printf("%s\n", string(body))
	},
}

var removePkgsCmd = &cobra.Command{
	Use:   "rm",
	Short: "Remove a package from a release",
	Run: func(cmd *cobra.Command, args []string) {
		cmd.Help()
	},
}

var listPkgsCmd = &cobra.Command{
	Use:   "list",
	Short: "List packages in a release",
	Run: func(cmd *cobra.Command, args []string) {
		cmd.Help()
	},
}

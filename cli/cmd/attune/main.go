package main

import (
	"fmt"
	"net/http"
	"os"

	"github.com/spf13/cobra"
)

func main() {
	root.AddCommand(repoCmd())

	if err := root.Execute(); err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}

var root = &cobra.Command{
	Use:               "attune",
	Short:             "Attune is a secure software delivery system for Linux packages",
	CompletionOptions: cobra.CompletionOptions{DisableDefaultCmd: true},
}

func API(req *http.Request) (*http.Response, error) {
	token, ok := os.LookupEnv("ATTUNE_API_TOKEN")
	if !ok {
		fmt.Println("ATTUNE_API_TOKEN environment variable not set")
		os.Exit(1)
	}
	req.SetBasicAuth("attune", token)

	u := req.URL
	endpoint, ok := os.LookupEnv("ATTUNE_API_ENDPOINT")
	if ok {
		parsed, err := req.URL.Parse(endpoint)
		if err != nil {
			fmt.Printf("Could not parse ATTUNE_API_ENDPOINT: %s\n", err)
			os.Exit(1)
		}
		req.URL.Host = parsed.Host
		req.URL.Scheme = parsed.Scheme
	} else {
		if u.Hostname() == "" {
			req.URL.Host = "localhost:3000"
		}
		if u.Scheme == "" {
			req.URL.Scheme = "http"
		}
	}

	client := &http.Client{}
	return client.Do(req)
}

func GetMaybeString(cmd *cobra.Command, name string) *string {
	value, err := cmd.Flags().GetString(name)
	if err != nil {
		fmt.Printf("could not read --%s: %s\n", name, err)
		os.Exit(1)
	}
	if value == "" {
		return nil
	}
	return &value
}

package main

import (
	"fmt"
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
	Use:               "armor",
	Short:             "ArmorCD is a secure software delivery system for Linux packages",
	CompletionOptions: cobra.CompletionOptions{DisableDefaultCmd: true},
}

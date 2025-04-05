package main

import (
	"fmt"
	"os"

	"github.com/spf13/cobra"
)

func main() {
	root.AddCommand(releaseCmd(), pkgsCmd(), repositoryCmd())

	if err := root.Execute(); err != nil {
		fmt.Println(err)
		os.Exit(1)
	}
}

var root = &cobra.Command{
	Use:              "armor",
	Short:            "ArmorCD is a secure software delivery system for Linux packages",
	TraverseChildren: true,
	Run: func(cmd *cobra.Command, args []string) {
		cmd.Help()
	},
	CompletionOptions: cobra.CompletionOptions{DisableDefaultCmd: true},
}

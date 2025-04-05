package main

import "github.com/spf13/cobra"

func pkgsCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "pkgs",
		Short: "Manage packages in a release",
		Run: func(cmd *cobra.Command, args []string) {
			cmd.Help()
		},
	}
	cmd.AddCommand(createPkgsCmd, removePkgsCmd, listPkgsCmd)
	return cmd
}

var createPkgsCmd = &cobra.Command{
	Use:   "add",
	Short: "Add a package to a release",
	Run: func(cmd *cobra.Command, args []string) {
		cmd.Help()
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

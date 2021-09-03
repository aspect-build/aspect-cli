/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/
package cmd

import (
	"fmt"
	"github.com/spf13/cobra"
	"os"

	"github.com/fatih/color"
	"github.com/mattn/go-isatty"
	homedir "github.com/mitchellh/go-homedir"
	"github.com/spf13/viper"
)

var (
	cfgFile string
	interactive bool

	varInitFncs []func()
	cmdInitFncs []func()	
) 

// rootCmd represents the base command when called without any subcommands
var rootCmd = &cobra.Command{
	Use:   "aspect",
	Short: "Aspect.build bazel wrapper",
	Long:  color.New(color.FgBlue).SprintFunc()(`Aspect CLI`) + ` is a better frontend for running bazel`,
	// Uncomment the following line if your bare application
	// has an action associated with it:
	// Run: func(cmd *cobra.Command, args []string) { },
}

func RegisterCommandVar(c func()) bool {
	varInitFncs = append(varInitFncs, c)

	return true
}

func RegisterCommandInit(c func()) bool {
	cmdInitFncs = append(cmdInitFncs, c)
	return true
}

func Main() error {
	// Setup all variables.
	// Setting up all the variables first will allow px
	// to initialize the init functions in any order
	for _, v := range varInitFncs {
		v()
	}

	// Call all plugin inits
	for _, f := range cmdInitFncs {
		f()
	}
	return rootCmd.Execute()
}

// Execute adds all child commands to the root command and sets flags appropriately.
// This is called by main.main(). It only needs to happen once to the rootCmd.
func Execute() {
	cobra.CheckErr(Main())
}

func init() {
	cobra.OnInitialize(initConfig)

	// Here you will define your flags and configuration settings.
	// Cobra supports persistent flags, which, if defined here,
	// will be global for your application.

	rootCmd.PersistentFlags().StringVar(&cfgFile, "config", "", "config file (default is $HOME/.aspect.yaml)")

	interactive_default := false
	if isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd()) {
		interactive_default = true
	}
	rootCmd.PersistentFlags().BoolVar(&interactive, "interactive", interactive_default, "Interactive mode (e.g. prompts for user input)")
}

// initConfig reads in config file and ENV variables if set.
func initConfig() {
	if cfgFile != "" {
		// Use config file from the flag.
		viper.SetConfigFile(cfgFile)
	} else {
		// Find home directory.
		home, err := homedir.Dir()
		cobra.CheckErr(err)

		// Search config in home directory with name ".aspect" (without extension).
		viper.AddConfigPath(home)
		viper.SetConfigName(".aspect")
	}

	viper.AutomaticEnv() // read in environment variables that match

	// If a config file is found, read it in.
	if err := viper.ReadInConfig(); err == nil {
		fmt.Fprintln(os.Stderr, "Using config file:", viper.ConfigFileUsed())
	}
}

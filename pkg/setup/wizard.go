package setup

import (
	"fmt"
	"os"
	"path"

	"github.com/fatih/color"
	"github.com/manifoldco/promptui"
	homedir "github.com/mitchellh/go-homedir"
	"github.com/spf13/viper"
)

var (
	boldCyan = color.New(color.FgCyan, color.Bold)
	faint    = color.New(color.Faint)
)

func SetupWizard() {
	cache, err := LocateCacheFolder()
	if err != nil {
		fmt.Fprintf(os.Stderr, "ERROR: Cannot read aspect run state. Cache folder not found. %v", err)
		os.Exit(1)
	}
	marker := path.Join(cache, "aspect")
	if _, err := os.Stat(marker); err == nil {
		return
	}

	defer func() {
		os.WriteFile(marker, []byte("ran"), 0644)
	}()

	boldCyan.Printf("It looks like you're running aspect for the first time. Welcome!\n")
	fmt.Printf("Let's take a moment to configure your environment to be the most productive.\n\n")

	home, err := homedir.Dir()
	if err != nil {
		fmt.Printf("No home directory found, unable to write config files")
	} else {
		maybeWriteAspectConfig(home)
		maybeWriteBazelConfig(home)
	}

	configLicenseKey()

	boldCyan.Printf("\nAll done!\n")
	faint.Printf("You can always re-run this wizard by deleting %s as well as the config file, then run aspect again.\n", marker)
	prompt := promptui.Prompt{
		Label: "Press enter to exit the setup",
	}
	_, _ = prompt.Run()
}

func maybeWriteAspectConfig(home string) {
	// Note: we expect the setup wizard is only called when no config file was located by Viper
	// so we don't check here for existence of any config file first.

	fmt.Println("\nWe can create a configuration file for aspect, to make it easy for you to save preferences.")
	prompt := promptui.Prompt{
		Label:     "Create a .aspect.ini config file in your home directory",
		IsConfirm: true,
	}

	if _, err := prompt.Run(); err == nil {
		viper.SetConfigType("ini")
		err = viper.SafeWriteConfig()
		if err != nil {
			fmt.Fprintf(os.Stderr, "ERROR: Failed to safe write config file: %v", err)
		}
	}
}

func maybeWriteBazelConfig(home string) {
	if _, err := os.Stat(path.Join(home, ".bazelrc")); err == nil {
		return
	}
	fmt.Println("\nHaving a bazelrc file makes it easier to customize how Bazel commands work.")
	faint.Println("For more information, see https://docs.bazel.build/versions/main/guide.html#bazelrc-the-bazel-configuration-file")
	prompt := promptui.Prompt{
		Label:     "Create a .bazelrc file in your home directory",
		IsConfirm: true,
	}

	if _, err := prompt.Run(); err == nil {
		err = os.WriteFile(path.Join(home, ".bazelrc"), []byte("# Bazel settings that apply for all executions on this machine\n"), 0644)
		if err != nil {
			fmt.Fprintf(os.Stderr, "ERROR: Failed to write bazelrc file: %v", err)
		}
	}
}

func configLicenseKey() {
	const enterKey = "enter a license key"
	const enterServer = "use a license server"

	fmt.Println("\nSome plugins may require one a license.")
	faint.Println("Check with your organization's developer productivity team if you're unsure.")
	prompt := promptui.Select{
		Label: "Would you like to",
		Items: []string{
			"use unlicensed for free",
			enterKey,
			enterServer,
		},
	}

	_, result, err := prompt.Run()

	if err != nil {
		return
	}

	switch result {
	case enterKey:
		prompt := promptui.Prompt{
			Label: "License Key",
		}

		result, err = prompt.Run()

		if err != nil {
			return
		}

		viper.Set("license.key", result)

	case enterServer:
		prompt := promptui.Prompt{
			Label: "License Server",
			// Validate: TODO check if the server is reachable
		}

		result, err = prompt.Run()

		if err != nil {
			return
		}

		viper.Set("license.server", result)

	default:
		return
	}
	if err := viper.WriteConfig(); err != nil {
		fmt.Fprintf(os.Stderr, "ERROR: Failed to update config file: %v", err)
	} else {
		fmt.Printf("Saved your license info to %s\n", viper.ConfigFileUsed())
	}
}

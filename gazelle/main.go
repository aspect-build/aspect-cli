package main

import (
	"flag"
	"fmt"
	"log"
	"os"
	"path"

	"github.com/aspect-build/aspect-cli/gazelle/common/buildinfo"
	"github.com/aspect-build/aspect-cli/gazelle/common/ibp"
	host "github.com/aspect-build/aspect-cli/gazelle/language/host"
	"github.com/aspect-build/aspect-cli/gazelle/runner"
	"github.com/bazelbuild/bazel-gazelle/language"
	"gopkg.in/yaml.v3"
)

/**
 * A `gazelle_binary`-like binary that runs gazelle following respecting the Aspect CLI config file.
 *
 * Supports additional features such as incremental builds via the Incremental Build Protocol.
 */
func main() {
	// Convenience for local development: under `bazel run <binary target>` respect the
	// users working directory, don't run in the execroot
	if wd, exists := os.LookupEnv("BUILD_WORKING_DIRECTORY"); exists {
		_ = os.Chdir(wd)
	}

	mode, languages, plugins, dirs := parseArgs()

	c := runner.New()

	// Add languages
	fmt.Printf("Languages: %v\n", languages)
	for _, lang := range languages {
		c.AddLanguage(lang)
	}

	// Add additional starlark plugins
	fmt.Printf("Plugins: %v\n", plugins)
	c.AddLanguageFactory(host.GazelleLanguageName, func() language.Language {
		return host.NewLanguage(plugins...)
	})

	fmt.Printf("Mode: %s\n", mode)
	fmt.Printf("Dirs: %v\n", dirs)

	if watchSocket := os.Getenv(ibp.PROTOCOL_SOCKET_ENV); watchSocket != "" {
		err := c.Watch(watchSocket, mode, []string{}, dirs)

		// Handle command errors
		if err != nil {
			log.Fatalf("Error running gazelle watcher: %v", err)
		}
	} else {
		_, err := c.Generate(mode, []string{}, dirs)

		// Handle command errors
		if err != nil {
			log.Fatalf("Error running gazelle: %v", err)
		}
	}
}

// Simple CLI parser for 'configure' args.
// No support for flag variations, shortforms etc.
func parseArgs() (runner.GazelleMode, []string, []string, []string) {
	args := flag.NewFlagSet("Aspect Configure", flag.ExitOnError)

	mode := args.String("mode", runner.Diff, "Configure mode: fix|update|diff")
	help := args.Bool("help", false, "Print help message")
	version := args.Bool("version", false, "Print version")
	config := args.String("config", "", `Aspect CLI config file (yaml). Properties include:

configure:
  languages:
    {lang}: true|false
    ...
  plugins:
    - {pluginPath}
    ...`)

	args.Parse(os.Args[1:])

	if *help {
		args.Usage()
		os.Exit(0)
	}

	if *version {
		fmt.Println(buildinfo.Current().Version())
		os.Exit(0)
	}

	if *config == "" {
		fmt.Println("No --config file specified")
		os.Exit(1)
	}

	languages, plugins, err := readConfigFile(*config)
	if err != nil {
		fmt.Println("Error reading config file:", err)
		os.Exit(1)
	}

	return *mode, languages, plugins, flag.Args()
}

// Simplified Aspect CLI config file parser
func readConfigFile(file string) ([]string, []string, error) {
	wd, err := os.Getwd()
	if err != nil {
		return nil, nil, err
	}

	configYaml, err := os.ReadFile(path.Join(wd, file))
	if err != nil {
		return nil, nil, err
	}

	var c cliConfig
	if err := yaml.Unmarshal(configYaml, &c); err != nil {
		return nil, nil, err
	}

	languages := []string{}
	plugins := []string{}

	// Must manually traverse the yaml.Node to preserve map[lang]bool order
	for i, node := range c.Configure.Languages.Content {
		if node.Tag != "!!str" {
			continue
		}

		lang := node.Value
		enabled := c.Configure.Languages.Content[i+1].Value == "true"

		if enabled {
			languages = append(languages, lang)
		}
	}

	for _, plugin := range c.Configure.Plugins {
		plugins = append(plugins, plugin)
	}

	return languages, plugins, nil
}

// A subset of the Aspect CLI config file format required for 'configure'
type cliConfig struct {
	Configure struct {
		Languages yaml.Node `yaml:"languages"`
		Plugins   []string  `yaml:"plugins"`
	} `yaml:"configure"`
}

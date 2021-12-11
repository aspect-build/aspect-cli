package main

import (
	"log"
	"os"

	"github.com/spf13/cobra/doc"

	"aspect.build/cli/cmd/aspect/root"
	"aspect.build/cli/pkg/ioutils"
	"aspect.build/cli/pkg/plugin/system"
)

func main() {
	if len(os.Args) != 2 {
		log.Fatal("Usage: cmd/docgen /path/to/outdir")
		os.Exit(1)
	}

	pluginSystem := system.NewPluginSystem()
	if err := pluginSystem.Configure(ioutils.DefaultStreams); err != nil {
		log.Fatal(err)
	}
	defer pluginSystem.TearDown()

	err := doc.GenMarkdownTree(root.NewDefaultRootCmd(pluginSystem), os.Args[1])
	if err != nil {
		log.Fatal(err)
	}
}

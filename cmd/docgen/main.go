package main

import (
	"log"
	"os"

	"aspect.build/cli/cmd/aspect/root"
	"github.com/spf13/cobra/doc"
)

func main() {
	if len(os.Args) != 2 {
		log.Fatal("Usage: cmd/docgen /path/to/outdir")
	}

	err := doc.GenMarkdownTree(root.NewDefaultRootCmd(nil), os.Args[1])
	if err != nil {
		log.Fatal(err)
	}
}

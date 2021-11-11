/*
Copyright © 2021 Aspect Build Systems Inc

Not licensed for re-use.
*/

package ioutils

import (
	"io"
	"os"

	"github.com/mattn/go-isatty"
)

type Streams struct {
	Stdin  io.Reader
	Stdout io.Writer
	Stderr io.Writer
}

var DefaultStreams = Streams{
	Stdin:  os.Stdin,
	Stdout: os.Stdout,
	Stderr: os.Stderr,
}

// Check if the CLI can be run in interactive mode
func IsInteractive() bool {
	return isatty.IsTerminal(os.Stdout.Fd()) || isatty.IsCygwinTerminal(os.Stdout.Fd())
}

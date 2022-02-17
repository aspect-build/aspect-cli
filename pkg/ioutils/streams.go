/*
 * Copyright 2022 Aspect Build Systems, Inc.
 *
 * Licensed under the aspect.build Commercial License (the "License");
 * you may not use this file except in compliance with the License.
 * Full License text is in the LICENSE file included in the root of this repository.
 */

package ioutils

import (
	"io"
	"os"
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
